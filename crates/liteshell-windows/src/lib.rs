use liteshell_builtins::CommandResolver;
use std::{
    ffi::OsStr,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};
use thiserror::Error;
use windows_sys::Win32::{
    Foundation::{CloseHandle, GlobalFree, HANDLE},
    System::{
        DataExchange::{
            CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable,
            OpenClipboard, SetClipboardData,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
        Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE, GMEM_ZEROINIT},
    },
};

const CF_UNICODETEXT: u32 = 13;

fn decode_clipboard_units(contents: &[u16]) -> String {
    let end = contents
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(contents.len());
    String::from_utf16_lossy(&contents[..end])
}

struct ClipboardGuard;

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
        }
    }
}

fn open_clipboard() -> std::io::Result<ClipboardGuard> {
    let opened = (0..5).any(|attempt| {
        if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
            true
        } else {
            if attempt < 4 {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            false
        }
    });
    if opened {
        Ok(ClipboardGuard)
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Put UTF-8 text on the Windows clipboard as CF_UNICODETEXT. The movable
/// allocation becomes clipboard-owned after SetClipboardData succeeds.
pub fn set_clipboard_text(text: &str) -> std::io::Result<()> {
    let mut contents: Vec<u16> = text.encode_utf16().collect();
    contents.push(0);

    let bytes = contents.len() * std::mem::size_of::<u16>();
    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes) };
    if memory.is_null() {
        return Err(std::io::Error::last_os_error());
    }
    let destination = unsafe { GlobalLock(memory) }.cast::<u16>();
    if destination.is_null() {
        let error = std::io::Error::last_os_error();
        unsafe {
            GlobalFree(memory);
        }
        return Err(error);
    }
    unsafe {
        std::ptr::copy_nonoverlapping(contents.as_ptr(), destination, contents.len());
        GlobalUnlock(memory);
    }

    let _guard = match open_clipboard() {
        Ok(guard) => guard,
        Err(error) => {
            unsafe {
                GlobalFree(memory);
            }
            return Err(error);
        }
    };

    if unsafe { EmptyClipboard() } == 0 {
        let error = std::io::Error::last_os_error();
        unsafe {
            GlobalFree(memory);
        }
        return Err(error);
    }
    if unsafe { SetClipboardData(CF_UNICODETEXT, memory) }.is_null() {
        let error = std::io::Error::last_os_error();
        unsafe {
            GlobalFree(memory);
        }
        return Err(error);
    }
    Ok(())
}

/// Read CF_UNICODETEXT from the Windows clipboard as UTF-8.
pub fn get_clipboard_text() -> std::io::Result<Option<String>> {
    if unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT) } == 0 {
        return Ok(None);
    }
    let _guard = open_clipboard()?;
    let memory = unsafe { GetClipboardData(CF_UNICODETEXT) };
    if memory.is_null() {
        return Err(std::io::Error::last_os_error());
    }
    let units = unsafe { GlobalSize(memory) } / std::mem::size_of::<u16>();
    let source = unsafe { GlobalLock(memory) }.cast::<u16>();
    if source.is_null() {
        return Err(std::io::Error::last_os_error());
    }
    let contents = unsafe { std::slice::from_raw_parts(source, units) };
    let text = decode_clipboard_units(contents);
    unsafe {
        GlobalUnlock(memory);
    }
    Ok(Some(text))
}

/// A Windows Job Object that terminates attached process trees when LiteShell
/// exits or is killed by a terminal-tool timeout.
pub struct ProcessJob {
    handle: HANDLE,
}

impl ProcessJob {
    pub fn kill_on_close() -> std::io::Result<Self> {
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                std::ptr::addr_of!(limits).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        };
        if configured == 0 {
            let error = std::io::Error::last_os_error();
            unsafe {
                CloseHandle(handle);
            }
            return Err(error);
        }
        Ok(Self { handle })
    }

    pub fn assign(&self, child: &Child) -> std::io::Result<()> {
        use std::os::windows::io::AsRawHandle;
        let assigned =
            unsafe { AssignProcessToJobObject(self.handle, child.as_raw_handle() as HANDLE) };
        if assigned == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

impl Drop for ProcessJob {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

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

pub fn is_supported_executable(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(OsStr::to_str)
            .map(|extension| {
                EXTS[..EXTS.len() - 1]
                    .iter()
                    .any(|candidate| extension.eq_ignore_ascii_case(candidate))
            })
            .unwrap_or(true)
}

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
            let pattern = if query.ends_with('*') {
                query.to_owned()
            } else {
                format!("{query}*")
            };
            if pattern[..pattern.len() - 1].contains('*') {
                return translation_usage(
                    "ps",
                    "wildcards are only supported at the end of a name",
                );
            }
            format!("IMAGENAME eq {pattern}")
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
    let status = build_command(path, args, cwd)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;
    Ok(status.code().unwrap_or(1))
}

pub fn spawn_captured(path: &Path, args: &[String], cwd: &Path) -> std::io::Result<Child> {
    build_command(path, args, cwd)
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

/// Construct a process command using LiteShell's normal Windows routing rules.
/// Callers may attach custom stdio before spawning it, which is required by the
/// pipeline executor.
pub fn build_command(path: &Path, args: &[String], cwd: &Path) -> Command {
    let ext = path
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut c = if matches!(ext.as_str(), "cmd" | "bat") {
        // `cmd /c` does not accept the script path and its arguments as an
        // argv-style tail. In particular, passing a path such as
        // `C:\Program Files\...\code.cmd` as a separate argument makes cmd
        // try to execute `C:\Program`. Build one command string and surround
        // the whole string with the extra quotes required by `/s /c`.
        let mut command_line = quote_windows_argument(path.as_os_str());
        for argument in args {
            command_line.push(' ');
            command_line.push_str(&quote_windows_argument(OsStr::new(argument)));
        }
        let mut c = Command::new("cmd.exe");
        c.args(["/d", "/s", "/c"])
            .raw_arg(format!("\"{command_line}\""));
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
    fn clipboard_unicode_text_stops_at_the_nul_terminator() {
        let mut contents: Vec<u16> = "中文 👩‍💻".encode_utf16().collect();
        contents.extend([0, b'x' as u16]);
        assert_eq!(decode_clipboard_units(&contents), "中文 👩‍💻");
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
        assert!(
            output.status.success(),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("out"));
        assert!(String::from_utf8_lossy(&output.stderr).contains("err"));
    }

    #[test]
    fn batch_file_with_spaces_in_its_path_is_invoked_as_one_command() {
        let directory = std::env::temp_dir().join(format!(
            "liteshell batch path {} {}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let script = directory.join("print argument.cmd");
        std::fs::write(&script, "@echo off\r\necho [%~1]\r\n").unwrap();

        let output = spawn_captured(
            &script,
            &["hello world".to_owned()],
            &std::env::current_dir().unwrap(),
        )
        .unwrap()
        .wait_with_output()
        .unwrap();

        let _ = std::fs::remove_dir_all(&directory);
        assert!(
            output.status.success(),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("[hello world]"));
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
            ["/fo", "table", "/v", "/fi", "IMAGENAME eq code*"]
        );
        assert_eq!(command.path.file_name().unwrap(), "tasklist.exe");
    }

    #[test]
    fn ps_executable_name_uses_a_tasklist_compatible_prefix_filter() {
        let command = translate_ps(
            &["chrome.exe".to_owned()],
            &std::env::current_dir().unwrap(),
        )
        .unwrap();
        assert_eq!(
            command.args,
            ["/fo", "table", "/fi", "IMAGENAME eq chrome.exe*"]
        );
    }

    #[test]
    fn translated_ps_name_filter_is_accepted_by_tasklist() {
        let cwd = std::env::current_dir().unwrap();
        let command = translate_ps(&["liteshell-no-such-process".to_owned()], &cwd).unwrap();
        let output = spawn_captured(&command.path, &command.args, &cwd)
            .unwrap()
            .wait_with_output()
            .unwrap();
        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!String::from_utf8_lossy(&output.stderr).contains("search filter"));
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
