use liteshell_builtins::CommandResolver;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};
use thiserror::Error;

pub const TRANSLATED_NAMES: &[&str] = &["ps", "kill"];

#[derive(Debug, Eq, PartialEq)]
pub struct TranslatedCommand {
    pub path: PathBuf,
    pub args: Vec<String>,
}

#[derive(Debug, Error, Eq, PartialEq)]
#[error("{command}: {message}")]
pub struct TranslationError {
    pub command: &'static str,
    pub message: String,
}

#[derive(Default)]
pub struct WindowsCommandResolver;
// Prefer Windows-native command formats over extensionless Unix shims. Scoop,
// for example, installs `scoop`, `scoop.cmd`, and `scoop.ps1` side by side.
const EXTS: &[&str] = &["exe", "com", "cmd", "bat", "ps1", ""];
impl CommandResolver for WindowsCommandResolver {
    fn resolve(&self, command: &str, cwd: &Path) -> Option<PathBuf> {
        resolve(translated_target(command).unwrap_or(command), cwd)
    }

    fn describe(&self, command: &str, cwd: &Path) -> Option<String> {
        let target = translated_target(command);
        let path = resolve(target.unwrap_or(command), cwd)?;
        Some(match target {
            Some(target) => format!("windows translation -> {target} ({})", path.display()),
            None => path.display().to_string(),
        })
    }
}

fn translated_target(command: &str) -> Option<&'static str> {
    match command.to_ascii_lowercase().as_str() {
        "ps" => Some("tasklist.exe"),
        "kill" => Some("taskkill.exe"),
        _ => None,
    }
}

pub fn translate(
    command: &str,
    args: &[String],
    cwd: &Path,
) -> Option<Result<TranslatedCommand, TranslationError>> {
    match command.to_ascii_lowercase().as_str() {
        "ps" => Some(translate_ps(args, cwd)),
        "kill" => Some(translate_kill(args, cwd)),
        _ => None,
    }
}

fn translated_command(
    command: &'static str,
    target: &'static str,
    args: Vec<String>,
    cwd: &Path,
) -> Result<TranslatedCommand, TranslationError> {
    let path = resolve(target, cwd).ok_or_else(|| TranslationError {
        command,
        message: format!("Windows command not found: {target}"),
    })?;
    Ok(TranslatedCommand { path, args })
}

fn translate_ps(args: &[String], cwd: &Path) -> Result<TranslatedCommand, TranslationError> {
    let mut verbose = false;
    let mut query = None;
    let mut end = false;
    for value in args {
        if !end && value == "--" {
            end = true;
        } else if !end && value == "aux" {
            verbose = true;
        } else if !end && value.starts_with('-') && value.len() > 1 {
            for option in value[1..].chars() {
                match option {
                    'a' | 'x' => {}
                    'u' | 'v' => verbose = true,
                    _ => return translation_usage("ps", format!("unknown option: -{option}")),
                }
            }
        } else if query.replace(value).is_some() {
            return translation_usage("ps", "expected at most one name or PID");
        }
    }

    let mut translated = vec!["/fo".to_owned(), "table".to_owned()];
    if verbose {
        translated.push("/v".to_owned());
    }
    if let Some(query) = query {
        if query.contains('"') {
            return translation_usage("ps", "query cannot contain a double quote");
        }
        translated.push("/fi".to_owned());
        translated.push(if query.parse::<u32>().is_ok() {
            format!("PID eq {query}")
        } else {
            format!("IMAGENAME eq *{query}*")
        });
    }
    translated_command("ps", "tasklist.exe", translated, cwd)
}

fn translate_kill(args: &[String], cwd: &Path) -> Result<TranslatedCommand, TranslationError> {
    let mut force = false;
    let mut tree = false;
    let mut pids = Vec::new();
    let mut end = false;
    let mut index = 0;
    while index < args.len() {
        let value = &args[index];
        if !end && value == "--" {
            end = true;
        } else if !end && value == "--tree" {
            tree = true;
        } else if !end && matches!(value.as_str(), "-s" | "--signal") {
            index += 1;
            let signal = args.get(index).ok_or_else(|| TranslationError {
                command: "kill",
                message: format!("{value} requires TERM or KILL"),
            })?;
            force = parse_signal(signal)?;
        } else if !end && value.starts_with("--signal=") {
            force = parse_signal(value.trim_start_matches("--signal="))?;
        } else if !end && value.starts_with('-') {
            force = parse_signal(&value[1..])?;
        } else {
            let pid = value.parse::<u32>().map_err(|_| TranslationError {
                command: "kill",
                message: format!("invalid PID: {value}"),
            })?;
            if pid == 0 {
                return translation_usage("kill", "PID must be greater than zero");
            }
            pids.push(pid);
        }
        index += 1;
    }
    if pids.is_empty() {
        return translation_usage("kill", "expected at least one PID");
    }

    let mut translated = Vec::new();
    for pid in pids {
        translated.extend(["/pid".to_owned(), pid.to_string()]);
    }
    if force {
        translated.push("/f".to_owned());
    }
    if tree {
        translated.push("/t".to_owned());
    }
    translated_command("kill", "taskkill.exe", translated, cwd)
}

fn parse_signal(value: &str) -> Result<bool, TranslationError> {
    match value.to_ascii_uppercase().trim_start_matches("SIG") {
        "TERM" | "15" => Ok(false),
        "KILL" | "9" => Ok(true),
        _ => translation_usage(
            "kill",
            format!("unsupported signal: {value}; use TERM or KILL"),
        ),
    }
}

fn translation_usage<T>(
    command: &'static str,
    message: impl Into<String>,
) -> Result<T, TranslationError> {
    Err(TranslationError {
        command,
        message: message.into(),
    })
}
pub fn resolve(command: &str, cwd: &Path) -> Option<PathBuf> {
    let p = Path::new(command);
    let explicit = p.components().count() > 1 || p.is_absolute();
    if explicit {
        return candidates(&cwd.join(p)).into_iter().find(|p| p.is_file());
    }
    let mut dirs = vec![cwd.to_owned()];
    dirs.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    for d in dirs {
        if let Some(p) = candidates(&d.join(p)).into_iter().find(|p| p.is_file()) {
            return Some(p);
        }
    }
    None
}
fn candidates(p: &Path) -> Vec<PathBuf> {
    if p.extension().is_some() {
        return vec![p.to_owned()];
    }
    EXTS.iter()
        .map(|e| {
            if e.is_empty() {
                p.to_owned()
            } else {
                p.with_extension(e)
            }
        })
        .collect()
}
pub fn quote_windows_argument(value: &OsStr) -> String {
    let s = value.to_string_lossy();
    if s.is_empty() {
        return "\"\"".into();
    }
    if !s.chars().any(|c| c.is_whitespace() || c == '\"') {
        return s.into_owned();
    }
    let mut out = String::from("\"");
    let mut slashes = 0;
    for ch in s.chars() {
        if ch == '\\' {
            slashes += 1
        } else if ch == '\"' {
            out.push_str(&"\\".repeat(slashes * 2 + 1));
            out.push('\"');
            slashes = 0
        } else {
            out.push_str(&"\\".repeat(slashes));
            slashes = 0;
            out.push(ch)
        }
    }
    out.push_str(&"\\".repeat(slashes * 2));
    out.push('\"');
    out
}
pub fn launch(path: &Path, args: &[String], cwd: &Path) -> std::io::Result<i32> {
    let status = command(path, args, cwd)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;
    Ok(status.code().unwrap_or(1))
}

pub fn spawn_captured(path: &Path, args: &[String], cwd: &Path) -> std::io::Result<Child> {
    command(path, args, cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

pub fn terminate_process_tree(child: &mut Child) -> std::io::Result<()> {
    let pid = child.id().to_string();
    let status = Command::new("taskkill.exe")
        .args(["/pid", &pid, "/t", "/f"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match status {
        Ok(status) if status.success() => Ok(()),
        _ => child.kill(),
    }
}

fn command(path: &Path, args: &[String], cwd: &Path) -> Command {
    let ext = path
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut c = if matches!(ext.as_str(), "cmd" | "bat") {
        let mut c = Command::new("cmd.exe");
        c.args(["/d", "/s", "/c"]).arg(path).args(args);
        c
    } else if ext == "ps1" {
        let shell = resolve("pwsh.exe", cwd)
            .or_else(|| resolve("powershell.exe", cwd))
            .unwrap_or_else(|| "powershell.exe".into());
        let mut c = Command::new(shell);
        c.args(["-NoLogo", "-NoProfile", "-File"])
            .arg(path)
            .args(args);
        c
    } else {
        let mut c = Command::new(path);
        c.args(args);
        c
    };
    c.current_dir(cwd);
    c
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quoting() {
        assert_eq!(quote_windows_argument(OsStr::new("a b")), "\"a b\"");
        assert_eq!(quote_windows_argument(OsStr::new("")), "\"\"");
    }

    #[test]
    fn windows_commands_are_preferred_over_extensionless_shims() {
        let names: Vec<_> = candidates(Path::new("scoop"))
            .into_iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "scoop.exe",
                "scoop.com",
                "scoop.cmd",
                "scoop.bat",
                "scoop.ps1",
                "scoop",
            ]
        );
    }

    #[test]
    fn resolved_paths_do_not_use_the_verbatim_prefix() {
        let executable = std::env::current_exe().unwrap();
        let resolved = resolve(
            executable.to_str().unwrap(),
            &std::env::current_dir().unwrap(),
        )
        .unwrap();
        assert!(!resolved.to_string_lossy().starts_with(r"\\?\"));
    }

    #[test]
    fn captured_child_returns_stdout_and_stderr() {
        let args = vec![
            "/d".to_owned(),
            "/c".to_owned(),
            "echo out & echo err 1>&2".to_owned(),
        ];
        let output = spawn_captured(
            Path::new("cmd.exe"),
            &args,
            &std::env::current_dir().unwrap(),
        )
        .unwrap()
        .wait_with_output()
        .unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("out"));
        assert!(String::from_utf8_lossy(&output.stderr).contains("err"));
    }

    #[test]
    fn ps_aux_query_translates_to_verbose_tasklist_filter() {
        let command = translate_ps(
            &["aux".to_owned(), "code".to_owned()],
            &std::env::current_dir().unwrap(),
        )
        .unwrap();
        assert_eq!(
            command.args,
            ["/fo", "table", "/v", "/fi", "IMAGENAME eq *code*"]
        );
        assert_eq!(command.path.file_name().unwrap(), "tasklist.exe");
    }

    #[test]
    fn numeric_ps_query_uses_pid_filter() {
        let command =
            translate_ps(&["1234".to_owned()], &std::env::current_dir().unwrap()).unwrap();
        assert_eq!(command.args, ["/fo", "table", "/fi", "PID eq 1234"]);
    }

    #[test]
    fn kill_supports_unix_signal_spelling_and_process_trees() {
        let command = translate_kill(
            &[
                "-KILL".to_owned(),
                "--tree".to_owned(),
                "1234".to_owned(),
                "5678".to_owned(),
            ],
            &std::env::current_dir().unwrap(),
        )
        .unwrap();
        assert_eq!(command.args, ["/pid", "1234", "/pid", "5678", "/f", "/t"]);
        assert_eq!(command.path.file_name().unwrap(), "taskkill.exe");
    }

    #[test]
    fn kill_rejects_signals_windows_cannot_represent() {
        let error = translate_kill(
            &["-HUP".to_owned(), "1234".to_owned()],
            &std::env::current_dir().unwrap(),
        )
        .unwrap_err();
        assert!(error.message.contains("unsupported signal"));
    }
}
