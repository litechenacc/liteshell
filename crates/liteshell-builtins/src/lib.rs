mod color;
mod help;

pub use help::{command_text as command_help_text, overview_text, version_text, VERSION};

use color::{file_color, highlight_lines, highlight_text, span, strong};
use liteshell_core::{
    CommandResult, OutputEvent, OutputSink, SearchKind, SearchProvider, SemanticColor as Color,
    ShellState, StyledSpan, StyledText,
};
use std::{
    fs,
    fs::{FileTimes, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    time::SystemTime,
};

pub const NAMES: &[&str] = &[
    "cd", "pwd", "ls", "mkdir", "rm", "touch", "cat", "tail", "less", "clear", "which", "find",
    "rg", "help", "version", "exit", "quit",
];
pub trait CommandResolver {
    fn resolve(&self, command: &str, cwd: &Path) -> Option<PathBuf>;

    fn describe(&self, command: &str, cwd: &Path) -> Option<String> {
        self.resolve(command, cwd)
            .map(|path| path.display().to_string())
    }
}
pub struct Context<'a> {
    pub shell: &'a mut ShellState,
    /// Standard input for the command. Agent/pipeline execution supplies the
    /// inherited handle or preceding stage's pipe.
    pub input: &'a mut dyn Read,
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
    if args.len() == 2 && matches!(args[1].as_str(), "-h" | "--help") {
        if let Some(text) = help::command_text(&cmd) {
            emit(ctx, text);
            return CommandResult::ok();
        }
    }
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
        "mkdir" => mkdir(args, ctx),
        "rm" => rm(args, ctx),
        "touch" => touch(args, ctx),
        "cat" => cat(args, ctx),
        "tail" => tail(args, ctx),
        "less" => less(args, ctx),
        "clear" => clear(args, ctx),
        "which" => which(args, ctx),
        "find" => search(args, ctx, false),
        "rg" => search(args, ctx, true),
        "help" => help(args, ctx),
        "version" => version(args, ctx),
        "ps" | "kill" => CommandResult::unhandled(),
        _ => CommandResult::unhandled(),
    }
}

pub fn handles(args: &[String]) -> bool {
    args.first().is_some_and(|command| {
        NAMES.iter().any(|name| name.eq_ignore_ascii_case(command))
            || (matches!(command.to_ascii_lowercase().as_str(), "ps" | "kill")
                && args.len() == 2
                && matches!(args[1].as_str(), "-h" | "--help"))
    })
}

fn mkdir(a: &[String], c: &mut Context<'_>) -> CommandResult {
    let mut parents = false;
    let mut paths = Vec::new();
    let mut end = false;
    for value in &a[1..] {
        if !end && value == "--" {
            end = true;
        } else if !end && matches!(value.as_str(), "-p" | "--parents") {
            parents = true;
        } else if !end && value.starts_with('-') {
            return usage(c, "mkdir", &format!("unknown option: {value}"));
        } else {
            paths.push(value);
        }
    }
    if paths.is_empty() {
        return usage(c, "mkdir", "expected at least one directory");
    }

    let mut status = 0;
    for raw in paths {
        let path = c.shell.resolve(raw);
        let result = if parents {
            fs::create_dir_all(&path)
        } else {
            fs::create_dir(&path)
        };
        if let Err(e) = result {
            status = 1;
            error(c, "mkdir", format!("{}: {e}", path.display()), 1);
        }
    }
    CommandResult::status(status)
}

fn rm(a: &[String], c: &mut Context<'_>) -> CommandResult {
    let mut force = false;
    let mut recursive = false;
    let mut paths = Vec::new();
    let mut end = false;
    for value in &a[1..] {
        if !end && value == "--" {
            end = true;
        } else if !end && value == "--force" {
            force = true;
        } else if !end && value == "--recursive" {
            recursive = true;
        } else if !end && value.starts_with('-') && value.len() > 1 {
            for option in value[1..].chars() {
                match option {
                    'f' => force = true,
                    'r' | 'R' => recursive = true,
                    _ => return usage(c, "rm", &format!("unknown option: -{option}")),
                }
            }
        } else {
            paths.push(value);
        }
    }
    if paths.is_empty() {
        return usage(c, "rm", "expected at least one path");
    }

    let mut status = 0;
    for raw in paths {
        let path = c.shell.resolve(raw);
        if recursive && unsafe_recursive_target(Path::new(raw), &path) {
            status = 1;
            error(
                c,
                "rm",
                format!("refusing to recursively remove: {}", path.display()),
                1,
            );
            continue;
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(e) if force && e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                status = 1;
                error(c, "rm", format!("{}: {e}", path.display()), 1);
                continue;
            }
        };
        let file_type = metadata.file_type();
        let result = if directory_link(&file_type) {
            fs::remove_dir(&path)
        } else if metadata.is_dir() {
            if recursive {
                fs::remove_dir_all(&path)
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::IsADirectory,
                    "is a directory (use -r to remove recursively)",
                ))
            }
        } else {
            fs::remove_file(&path)
        };
        if let Err(e) = result {
            status = 1;
            error(c, "rm", format!("{}: {e}", path.display()), 1);
        }
    }
    CommandResult::status(status)
}

fn unsafe_recursive_target(raw: &Path, resolved: &Path) -> bool {
    use std::path::Component;
    matches!(
        raw.components().next_back(),
        Some(Component::CurDir | Component::ParentDir)
    ) || resolved.parent().is_none()
        || fs::canonicalize(resolved).is_ok_and(|path| path.parent().is_none())
}

fn directory_link(file_type: &fs::FileType) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTypeExt;
        file_type.is_symlink_dir()
    }
    #[cfg(not(windows))]
    {
        file_type.is_symlink()
    }
}

fn touch(a: &[String], c: &mut Context<'_>) -> CommandResult {
    let mut paths = Vec::new();
    let mut end = false;
    for value in &a[1..] {
        if !end && value == "--" {
            end = true;
        } else if !end && value.starts_with('-') {
            return usage(c, "touch", &format!("unknown option: {value}"));
        } else {
            paths.push(value);
        }
    }
    if paths.is_empty() {
        return usage(c, "touch", "expected at least one file");
    }

    let now = SystemTime::now();
    let mut status = 0;
    for raw in paths {
        let path = c.shell.resolve(raw);
        let result = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|file| file.set_times(FileTimes::new().set_modified(now)));
        if let Err(e) = result {
            status = 1;
            error(c, "touch", format!("{}: {e}", path.display()), 1);
        }
    }
    CommandResult::status(status)
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
        } else if !end && x == "--all" {
            all = true
        } else if !end && x == "--long" {
            long = true
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
    if a.len() == 1 {
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = match c.input.read(&mut buffer) {
                Ok(0) => return CommandResult::ok(),
                Ok(count) => count,
                Err(failure) => return error(c, "cat", failure.to_string(), 1),
            };
            if let Err(failure) = c.output.write_stdout(&buffer[..count]) {
                return error(c, "cat", failure.to_string(), 1);
            }
        }
    }
    let mut status = 0;
    for p in &a[1..] {
        let path = c.shell.resolve(p);
        match read_text(&path) {
            Ok(s) => c
                .output
                .emit(OutputEvent::Styled(highlight_text(&path, &s))),
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
        } else if let Some(description) = c.resolver.describe(x, &c.shell.cwd) {
            emit(c, format!("{x}: {description}\n"))
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
    let text = match a {
        [_] => help::overview_text(),
        [_, command] => match help::command_text(command) {
            Some(text) => text,
            None => return usage(c, "help", &format!("unknown command: {command}")),
        },
        _ => return usage(c, "help", "expected zero or one command"),
    };
    emit(c, text);
    CommandResult::ok()
}

fn version(a: &[String], c: &mut Context<'_>) -> CommandResult {
    if a.len() != 1 {
        return usage(c, "version", "expected no arguments");
    }
    emit(c, help::version_text());
    CommandResult::ok()
}
