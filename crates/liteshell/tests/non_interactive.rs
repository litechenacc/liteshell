use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn redirected_mode_is_plain_and_preserves_cwd() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("hello.txt"), "alpha\nbeta\ngamma\n").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_liteshell"))
        .current_dir(temp.path())
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
