use liteshell_builtins::CommandResolver;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Default)]
pub struct WindowsCommandResolver;
const EXTS: &[&str] = &["", "exe", "com", "cmd", "bat", "ps1"];
impl CommandResolver for WindowsCommandResolver {
    fn resolve(&self, command: &str, cwd: &Path) -> Option<PathBuf> {
        resolve(command, cwd)
    }
}
pub fn resolve(command: &str, cwd: &Path) -> Option<PathBuf> {
    let p = Path::new(command);
    let explicit = p.components().count() > 1 || p.is_absolute();
    if explicit {
        return candidates(&cwd.join(p))
            .into_iter()
            .find(|p| p.is_file())
            .and_then(|p| p.canonicalize().ok());
    }
    let mut dirs = vec![cwd.to_owned()];
    dirs.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    for d in dirs {
        if let Some(p) = candidates(&d.join(p)).into_iter().find(|p| p.is_file()) {
            return p.canonicalize().ok().or(Some(p));
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
    let status = c
        .current_dir(cwd)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;
    Ok(status.code().unwrap_or(1))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quoting() {
        assert_eq!(quote_windows_argument(OsStr::new("a b")), "\"a b\"");
        assert_eq!(quote_windows_argument(OsStr::new("")), "\"\"");
    }
}
