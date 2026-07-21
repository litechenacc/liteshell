use std::{
    io::Write,
    process::{Command, Stdio},
};

fn agent_command(cwd: &std::path::Path, line: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_liteshell"))
        .current_dir(cwd)
        .env("HOME", cwd)
        .env("USERPROFILE", cwd)
        .args(["-c", line])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap()
}

#[test]
fn cli_and_builtin_help_share_versioned_tabular_output() {
    let temp = tempfile::tempdir().unwrap();
    let version = Command::new(env!("CARGO_BIN_EXE_liteshell"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8(version.stdout).unwrap(),
        format!("LiteShell {}\n", env!("CARGO_PKG_VERSION"))
    );

    let overview = Command::new(env!("CARGO_BIN_EXE_liteshell"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(overview.status.success());
    let overview = String::from_utf8(overview.stdout).unwrap();
    assert!(overview.contains("Options:\n  -c, --command <COMMAND>"));
    assert!(overview.contains("Commands:\n  cd [DIRECTORY]"));

    for command in ["ls", "ps", "kill", "exit"] {
        let result = agent_command(temp.path(), &format!("{command} --help"));
        assert!(result.status.success(), "help failed for {command}");
        assert!(result.stderr.is_empty(), "help wrote stderr for {command}");
        let output = String::from_utf8(result.stdout).unwrap();
        assert!(output.contains("Usage:"), "missing usage for {command}");
        assert!(output.contains("Options:"), "missing options for {command}");
    }
}

#[test]
fn redirected_mode_is_plain_and_preserves_cwd() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("hello.txt"), "alpha\nbeta\ngamma\n").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_liteshell"))
        .current_dir(temp.path())
        .env("HOME", temp.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"pwd\nls\ncat hello.txt\ntail -n 2 hello.txt\nexit\n")
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success());
    let stdout = String::from_utf8(result.stdout).unwrap();
    assert!(stdout.contains("hello.txt"));
    assert!(stdout.contains("alpha\nbeta\ngamma"));
    assert!(stdout.contains("beta\ngamma"));
    assert!(!stdout.contains('\u{1b}'));
}

#[test]
fn startup_aliases_expand_in_redirected_mode() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join(".hidden"), "secret\n").unwrap();
    std::fs::write(
        temp.path().join(".liteshellrc"),
        "alias l='ll'\nalias ll='ls -la'\n",
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_liteshell"))
        .current_dir(temp.path())
        .env("HOME", temp.path())
        .env("USERPROFILE", temp.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"l\nexit\n").unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success());
    let stdout = String::from_utf8(result.stdout).unwrap();
    assert!(stdout.contains(".hidden"));
    assert!(stdout.contains("file"));
}

#[test]
fn agent_mode_streams_large_builtin_pipeline_without_ui_output() {
    let temp = tempfile::tempdir().unwrap();
    let contents = "pipeline-data\n".repeat(32 * 1024);
    std::fs::write(temp.path().join("large.txt"), &contents).unwrap();

    let result = agent_command(temp.path(), "cat large.txt | cat");
    assert!(result.status.success());
    assert_eq!(String::from_utf8(result.stdout).unwrap(), contents);
    assert!(result.stderr.is_empty());
}

#[test]
fn agent_mode_preserves_stdin_for_the_first_stage() {
    let temp = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_liteshell"))
        .current_dir(temp.path())
        .env("HOME", temp.path())
        .env("USERPROFILE", temp.path())
        .args(["-c", "cat | cat"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let input: Vec<u8> = (0u8..=255).cycle().take(1024 * 1024).collect();
    let expected = input.clone();
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || stdin.write_all(&input).unwrap());
    let result = child.wait_with_output().unwrap();
    writer.join().unwrap();
    assert!(result.status.success());
    assert_eq!(result.stdout, expected);
}

#[test]
fn agent_mode_supports_redirection_and_conditional_lists() {
    let temp = tempfile::tempdir().unwrap();
    let result = agent_command(
        temp.path(),
        "pwd > current.txt && cat current.txt; ls missing || pwd",
    );
    assert!(result.status.success());
    let stdout = String::from_utf8(result.stdout).unwrap();
    let cwd = temp.path().display().to_string();
    assert_eq!(stdout.matches(&cwd).count(), 2);
    assert_eq!(
        std::fs::read_to_string(temp.path().join("current.txt")).unwrap(),
        format!("{cwd}\n")
    );
    assert!(String::from_utf8(result.stderr)
        .unwrap()
        .contains("ls: path not found"));
}

#[test]
fn agent_mode_pipefail_is_on_by_default_and_can_be_disabled() {
    let temp = tempfile::tempdir().unwrap();
    let failed = agent_command(temp.path(), "ls missing 2>&1 | cat");
    assert!(!failed.status.success());
    assert!(String::from_utf8(failed.stdout)
        .unwrap()
        .contains("ls: path not found"));
    assert!(failed.stderr.is_empty());

    let succeeded = Command::new(env!("CARGO_BIN_EXE_liteshell"))
        .current_dir(temp.path())
        .env("HOME", temp.path())
        .env("USERPROFILE", temp.path())
        .args(["-c", "ls missing 2>&1 | cat", "--no-pipefail"])
        .output()
        .unwrap();
    assert!(succeeded.status.success());
}

#[test]
fn agent_mode_can_mix_external_commands_and_builtins() {
    let temp = tempfile::tempdir().unwrap();
    let result = agent_command(temp.path(), "cmd.exe /d /c \"echo external\" | cat");
    assert!(result.status.success());
    assert_eq!(String::from_utf8(result.stdout).unwrap().trim(), "external");
}

#[test]
fn agent_mode_loads_the_same_startup_path_environment_and_aliases() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    std::fs::write(bin.join("shared-tool.cmd"), "@echo shared-tool-visible\r\n").unwrap();
    std::fs::write(
        temp.path().join(".liteshellrc"),
        format!(
            "PATH={};%PATH%\nLITESHELL_AGENT_VALUE=shared-environment\nalias whereami='pwd'\n",
            bin.display()
        ),
    )
    .unwrap();

    let result = agent_command(
        temp.path(),
        "shared-tool | cat; cmd.exe /d /c \"echo %LITESHELL_AGENT_VALUE%\" | cat; whereami | cat",
    );
    assert!(result.status.success());
    let stdout = String::from_utf8(result.stdout).unwrap();
    assert!(stdout.contains("shared-tool-visible"));
    assert!(stdout.contains("shared-environment"));
    assert!(stdout.contains(&temp.path().display().to_string()));
}

#[test]
fn filesystem_builtins_create_touch_and_remove_paths() {
    let temp = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_liteshell"))
        .current_dir(temp.path())
        .env("HOME", temp.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            b"mkdir -p one/two\ntouch one/two/a.txt one/two/b.txt\nrm one/two/a.txt\nrm -rf one\nexit\n",
        )
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success());
    assert!(
        result.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!temp.path().join("one").exists());
}

#[test]
fn rm_requires_recursive_mode_for_directories() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("kept")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_liteshell"))
        .current_dir(temp.path())
        .env("HOME", temp.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"rm kept\nexit\n")
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(temp.path().join("kept").is_dir());
    assert!(String::from_utf8_lossy(&result.stderr).contains("use -r"));
}
