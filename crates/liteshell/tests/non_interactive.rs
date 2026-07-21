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
