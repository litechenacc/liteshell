mod color;

use color::{file_color, highlight_lines, highlight_text, span, strong};
use liteshell_core::{
    CommandResult, OutputEvent, OutputSink, SearchKind, SearchProvider, SemanticColor as Color,
    ShellState, StyledSpan, StyledText,
};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::SystemTime,
};

pub const NAMES: &[&str] = &[
    "cd", "pwd", "ls", "cat", "tail", "less", "clear", "which", "find", "rg", "help", "exit",
    "quit",
];
pub trait CommandResolver {
    fn resolve(&self, command: &str, cwd: &Path) -> Option<PathBuf>;
}
pub struct Context<'a> {
    pub shell: &'a mut ShellState,
    pub output: &'a mut dyn OutputSink,
    pub search: &'a mut dyn SearchProvider,
    pub resolver: &'a dyn CommandResolver,
    pub interactive: bool,
}
fn emit(ctx: &mut Context<'_>, text: impl Into<String>) {
    ctx.output.emit(OutputEvent::Text(text.into()));
}
fn emit_styled(ctx: &mut Context<'_>, spans: Vec<StyledSpan>) {
    ctx.output.emit(OutputEvent::Styled(StyledText::new(spans)));
}
fn error(ctx: &mut Context<'_>, cmd: &str, text: impl AsRef<str>, status: i32) -> CommandResult {
    ctx.output
        .emit(OutputEvent::Error(format!("{cmd}: {}\n", text.as_ref())));
    CommandResult::status(status)
}
fn usage(ctx: &mut Context<'_>, cmd: &str, text: &str) -> CommandResult {
    error(ctx, cmd, text, 2)
}

pub fn dispatch(args: &[String], ctx: &mut Context<'_>) -> CommandResult {
    if args.is_empty() {
        return CommandResult::ok();
    }
    let cmd = args[0].to_ascii_lowercase();
    match cmd.as_str() {
        "exit" | "quit" => {
            if args.len() != 1 {
                return usage(ctx, &cmd, "expected no arguments");
            }
            ctx.shell.running = false;
            CommandResult::ok()
        }
        "cd" => cd(args, ctx),
        "pwd" => pwd(args, ctx),
        "ls" => ls(args, ctx),
        "cat" => cat(args, ctx),
        "tail" => tail(args, ctx),
        "less" => less(args, ctx),
        "clear" => clear(args, ctx),
        "which" => which(args, ctx),
        "find" => search(args, ctx, false),
        "rg" => search(args, ctx, true),
        "help" => help(args, ctx),
        _ => CommandResult::unhandled(),
    }
}
fn cd(a: &[String], c: &mut Context<'_>) -> CommandResult {
    if a.len() > 2 {
        return usage(c, "cd", "expected zero or one path");
    }
    let raw = a
        .get(1)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| c.shell.cwd.clone());
    let p = c.shell.resolve(raw);
    if !p.is_dir() {
        return error(c, "cd", format!("directory not found: {}", p.display()), 1);
    }
    match fs::canonicalize(&p) {
        Ok(p) => {
            c.shell.cwd = p;
            CommandResult::ok()
        }
        Err(e) => error(c, "cd", e.to_string(), 1),
    }
}
fn pwd(a: &[String], c: &mut Context<'_>) -> CommandResult {
    if a.len() != 1 {
        return usage(c, "pwd", "expected no arguments");
    }
    let text = format!("{}\n", c.shell.display_cwd());
    emit(c, text);
    CommandResult::ok()
}
fn ls(a: &[String], c: &mut Context<'_>) -> CommandResult {
    let mut all = false;
    let mut long = false;
    let mut path = None;
    let mut end = false;
    for x in &a[1..] {
        if !end && x == "--" {
            end = true
        } else if !end && x.starts_with('-') && x.len() > 1 {
            for o in x[1..].chars() {
                match o {
                    'a' => all = true,
                    'l' => long = true,
                    _ => return usage(c, "ls", &format!("unknown option: -{o}")),
                }
            }
        } else if path.replace(x).is_some() {
            return usage(c, "ls", "expected at most one path");
        }
    }
    let target = path
        .map(|p| c.shell.resolve(p))
        .unwrap_or_else(|| c.shell.cwd.clone());
    let meta = match fs::metadata(&target) {
        Ok(m) => m,
        Err(_) => return error(c, "ls", format!("path not found: {}", target.display()), 1),
    };
    let mut entries: Vec<(PathBuf, fs::Metadata)> = if meta.is_dir() {
        match fs::read_dir(&target) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .filter_map(|e| fs::symlink_metadata(e.path()).ok().map(|m| (e.path(), m)))
                .collect(),
            Err(e) => return error(c, "ls", e.to_string(), 1),
        }
    } else {
        vec![(target, meta)]
    };
    entries.retain(|(p, _)| {
        all || !p
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .starts_with('.')
    });
    entries.sort_by(|a, b| {
        (
            !a.1.is_dir(),
            a.0.file_name().map(|v| v.to_string_lossy().to_lowercase()),
        )
            .cmp(&(
                !b.1.is_dir(),
                b.0.file_name().map(|v| v.to_string_lossy().to_lowercase()),
            ))
    });
    for (p, m) in entries {
        let n = p.file_name().unwrap_or_default().to_string_lossy();
        let prefix = if long {
            let secs = m
                .modified()
                .ok()
                .and_then(|v| v.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|v| v.as_secs())
                .unwrap_or(0);
            format!(
                "{secs}  {}  {:>12}  ",
                if m.is_dir() { "dir " } else { "file" },
                if m.is_dir() {
                    "-".into()
                } else {
                    m.len().to_string()
                }
            )
        } else {
            String::new()
        };
        let suffix = if m.is_dir() { "\\" } else { "" };
        let mut spans = Vec::new();
        if !prefix.is_empty() {
            spans.push(span(prefix, Color::Metadata));
        }
        spans.push(strong(format!("{n}{suffix}"), file_color(&p, &m)));
        spans.push(StyledSpan::plain("\n"));
        emit_styled(c, spans);
    }
    CommandResult::ok()
}
fn read_text(path: &Path) -> Result<String, String> {
    let mut b = Vec::new();
    fs::File::open(path)
        .map_err(|_| "cannot open file".to_string())?
        .read_to_end(&mut b)
        .map_err(|e| e.to_string())?;
    if b.iter()
        .take(4096)
        .any(|v| *v == 0 || *v < 8 || (*v > 13 && *v < 32))
    {
        return Err("binary".into());
    }
    if b.starts_with(&[0xef, 0xbb, 0xbf]) {
        b.drain(..3);
    }
    String::from_utf8(b).map_err(|_| "file is not valid UTF-8".into())
}
fn cat(a: &[String], c: &mut Context<'_>) -> CommandResult {
    if a.len() < 2 {
        return usage(c, "cat", "expected at least one file");
    }
    let mut status = 0;
    for p in &a[1..] {
        let path = c.shell.resolve(p);
        match read_text(&path) {
            Ok(s) => c.output.emit(OutputEvent::Styled(highlight_text(&path, &s))),
            Err(e) => {
                status = 1;
                let msg = if e == "binary" {
                    format!("refusing to print binary file: {}", path.display())
                } else {
                    format!("{e}: {}", path.display())
                };
                error(c, "cat", msg, 1);
            }
        }
    }
    CommandResult::status(status)
}
fn tail(a: &[String], c: &mut Context<'_>) -> CommandResult {
    let mut count = 10usize;
    let mut path = None;
    let mut i = 1;
    while i < a.len() {
        if a[i] == "-n" {
            i += 1;
            if i >= a.len() {
                return usage(c, "tail", "-n requires a line count");
            }
            count = match a[i].parse() {
                Ok(v) => v,
                Err(_) => return usage(c, "tail", &format!("invalid line count: {}", a[i])),
            };
        } else if path.replace(&a[i]).is_some() {
            return usage(c, "tail", "expected one file");
        }
        i += 1
    }
    let Some(p) = path else {
        return usage(c, "tail", "expected one file");
    };
    let path = c.shell.resolve(p);
    match read_text(&path) {
        Ok(s) => {
            let lines: Vec<_> = s.split_inclusive('\n').collect();
            let selected = lines[lines.len().saturating_sub(count)..].concat();
            c.output
                .emit(OutputEvent::Styled(highlight_text(&path, &selected)));
            CommandResult::ok()
        }
        Err(e) => error(
            c,
            "tail",
            if e == "binary" {
                format!("refusing to print binary file: {}", path.display())
            } else {
                format!("{e}: {}", path.display())
            },
            1,
        ),
    }
}
fn less(a: &[String], c: &mut Context<'_>) -> CommandResult {
    if a.len() != 2 {
        return usage(c, "less", "expected one file");
    }
    let p = c.shell.resolve(&a[1]);
    match read_text(&p) {
        Ok(s) => {
            if c.interactive {
                c.output.emit(OutputEvent::Pager {
                    title: p.file_name().unwrap_or_default().to_string_lossy().into(),
                    lines: highlight_lines(&p, s.lines()),
                })
            } else {
                c.output.emit(OutputEvent::Styled(highlight_text(&p, &s)))
            }
            CommandResult::ok()
        }
        Err(e) => error(
            c,
            "less",
            if e == "binary" {
                format!("refusing to open binary file: {}", p.display())
            } else {
                format!("{e}: {}", p.display())
            },
            1,
        ),
    }
}
fn clear(a: &[String], c: &mut Context<'_>) -> CommandResult {
    if a.len() != 1 {
        return usage(c, "clear", "expected no arguments");
    }
    c.output.emit(OutputEvent::Clear);
    CommandResult::ok()
}
fn which(a: &[String], c: &mut Context<'_>) -> CommandResult {
    if a.len() < 2 {
        return usage(c, "which", "expected at least one command");
    }
    let mut s = 0;
    for x in &a[1..] {
        if NAMES.iter().any(|n| n.eq_ignore_ascii_case(x)) {
            emit(c, format!("{x}: builtin\n"))
        } else if let Some(p) = c.resolver.resolve(x, &c.shell.cwd) {
            emit(c, format!("{x}: {}\n", p.display()))
        } else {
            s = 1;
            error(c, "which", format!("command not found: {x}"), 1);
        }
    }
    CommandResult::status(s)
}
fn search(a: &[String], c: &mut Context<'_>, grep: bool) -> CommandResult {
    if grep && a.len() < 2 {
        return usage(c, "rg", "expected a search query");
    }
    let q = a[1..].join(" ");
    match c.search.search(
        if grep {
            SearchKind::Grep
        } else {
            SearchKind::Files
        },
        &q,
        &c.shell.cwd,
        100,
    ) {
        Ok(v) => {
            for x in v {
                emit(
                    c,
                    if grep {
                        format!("{}: {}\n", x.label, x.detail)
                    } else {
                        format!("{}\n", x.label)
                    },
                )
            }
            CommandResult::ok()
        }
        Err(e) => error(c, if grep { "rg" } else { "find" }, e, 1),
    }
}
fn help(a: &[String], c: &mut Context<'_>) -> CommandResult {
    if a.len() != 1 {
        return usage(c, "help", "expected no arguments");
    }
    let commands = [
        ("cd", " [path]", "change directory (no path uses home)"),
        ("pwd", "", "print current directory"),
        ("ls", " [-a] [-l] [path]", "list directory contents"),
        ("cat", " file...", "print UTF-8 text files"),
        ("tail", " [-n count] file", "print the last lines"),
        ("less", " file", "open the built-in pager"),
        ("clear", "", "clear the terminal"),
        ("which", " command...", "resolve commands"),
        ("find", " [query]", "fuzzy file search"),
        ("rg", " query", "indexed content search"),
        ("exit", "", "leave LiteShell"),
    ];
    let mut spans = vec![
        strong("LiteShell commands", Color::Heading),
        StyledSpan::plain(":\n"),
    ];
    for (command, arguments, description) in commands {
        spans.push(StyledSpan::plain("  "));
        spans.push(strong(command, Color::Command));
        spans.push(span(format!("{arguments:<20}"), Color::Option));
        spans.push(StyledSpan::plain(description));
        spans.push(StyledSpan::plain("\n"));
    }
    emit_styled(c, spans);
    CommandResult::ok()
}
