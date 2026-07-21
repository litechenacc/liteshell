use std::{
    env, fs, io,
    path::{Component, Path, PathBuf},
    process::Command,
};

const INSTALL_STATE_DIR: &str = ".liteshell";

fn main() {
    match run() {
        Ok(0) => {}
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("liteshell launcher: {error}");
            std::process::exit(1);
        }
    }
}

fn run() -> Result<i32, Box<dyn std::error::Error>> {
    let launcher = env::current_exe()?;
    let install_dir = launcher.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "launcher has no parent directory",
        )
    })?;
    cleanup_legacy(&install_dir.join(INSTALL_STATE_DIR).join("legacy"));
    let payload = resolve_payload(install_dir)?;
    let status = Command::new(&payload)
        .args(env::args_os().skip(1))
        .status()?;
    Ok(status.code().unwrap_or(1))
}

fn cleanup_legacy(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_file()) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn resolve_payload(install_dir: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let state_dir = install_dir.join(INSTALL_STATE_DIR);
    let build_id = fs::read_to_string(state_dir.join("current"))?;
    let build_id = build_id.trim();
    if !valid_build_id(build_id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid current version: {build_id:?}"),
        )
        .into());
    }
    let payload = state_dir
        .join("versions")
        .join(build_id)
        .join("liteshell.exe");
    if !payload.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("installed version is missing: {}", payload.display()),
        )
        .into());
    }
    Ok(payload)
}

fn valid_build_id(value: &str) -> bool {
    value.len() == 64
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        && Path::new(value)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_id_is_a_single_sha256_component() {
        assert!(valid_build_id(&"a".repeat(64)));
        assert!(!valid_build_id("abc"));
        assert!(!valid_build_id(&format!("{}\\x", "a".repeat(64))));
        assert!(!valid_build_id(&"g".repeat(64)));
    }
}
