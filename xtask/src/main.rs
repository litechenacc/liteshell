use sha2::{Digest, Sha256};
use std::{
    env,
    ffi::OsStr,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
const DLL_URL:&str="https://github.com/dmtrKovalenko/fff/releases/download/v0.10.0/c-lib-x86_64-pc-windows-msvc.dll";
const LICENSE_URL: &str = "https://raw.githubusercontent.com/dmtrKovalenko/fff/v0.10.0/LICENSE";
const DLL_SHA: &str = "2d643319aee9899980084245e4fd6752c084c7a343e47228449458550ad55966";
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    match env::args().nth(1).as_deref() {
        Some("fetch") => fetch(&root)?,
        Some("package") => package(&root)?,
        Some("install") => install(&root)?,
        _ => return Err("usage: cargo xtask <fetch|package|install>".into()),
    }
    Ok(())
}
fn download(url: &str, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if path.exists() {
        return Ok(());
    }
    if let Some(p) = path.parent() {
        fs::create_dir_all(p)?
    }
    let temporary = path.with_extension("download");
    let status = Command::new("curl.exe")
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--output",
        ])
        .arg(&temporary)
        .arg(url)
        .status()?;
    if !status.success() {
        let _ = fs::remove_file(&temporary);
        return Err(format!("download failed: {url}").into());
    }
    fs::rename(temporary, path)?;
    Ok(())
}
fn fetch(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let dir = root.join("build/deps/fff-0.10.0");
    let dll = dir.join("fff_c.dll");
    download(DLL_URL, &dll)?;
    let mut bytes = Vec::new();
    fs::File::open(&dll)?.read_to_end(&mut bytes)?;
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != DLL_SHA {
        fs::remove_file(&dll)?;
        return Err(format!("fff DLL checksum mismatch: {actual}").into());
    }
    download(LICENSE_URL, &dir.join("LICENSE.fff"))?;
    println!("{}", dll.display());
    Ok(())
}
fn package(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    fetch(root)?;
    let status = Command::new("cargo")
        .args(["build", "--release", "-p", "liteshell"])
        .current_dir(root)
        .status()?;
    if !status.success() {
        return Err("release build failed".into());
    }
    let out = root.join("build/release");
    fs::create_dir_all(&out)?;
    fs::copy(
        root.join("target/release/liteshell.exe"),
        out.join("liteshell.exe"),
    )?;
    fs::copy(
        root.join("build/deps/fff-0.10.0/fff_c.dll"),
        out.join("fff_c.dll"),
    )?;
    fs::copy(
        root.join("build/deps/fff-0.10.0/LICENSE.fff"),
        out.join("LICENSE.fff"),
    )?;
    println!("Packaged {}", out.display());
    Ok(())
}

fn install(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let release_dir = root.join("target/release");
    let payload_source = release_dir.join("liteshell.exe");
    let launcher_source = release_dir.join("liteshell-launcher.exe");
    require_file(&payload_source, "release payload")?;
    require_file(&launcher_source, "release launcher")?;

    let install_dir = match env::var_os("LITESHELL_INSTALL_DIR") {
        Some(path) => PathBuf::from(path),
        None => home_dir()?.join(".local/bin"),
    };
    let state_dir = install_dir.join(".liteshell");
    let versions_dir = state_dir.join("versions");
    fs::create_dir_all(&versions_dir)?;

    let build_id = digest_file(&payload_source)?;
    let version_dir = versions_dir.join(&build_id);
    fs::create_dir_all(&version_dir)?;
    install_immutable(
        &payload_source,
        &version_dir.join("liteshell.exe"),
        &build_id,
    )?;
    atomic_write(
        &state_dir.join("current"),
        format!("{build_id}\n").as_bytes(),
    )?;

    let installed_launcher = install_dir.join("liteshell.exe");
    let migrated = install_launcher(&launcher_source, &installed_launcher, &state_dir)?;
    println!("Installed LiteShell {build_id}");
    println!("Launcher: {}", installed_launcher.display());
    if let Some(previous) = migrated {
        println!(
            "The previous executable is still usable by running instances and will be removed later: {}",
            previous.display()
        );
    }
    Ok(())
}

fn require_file(path: &Path, description: &str) -> Result<(), Box<dyn std::error::Error>> {
    if path.is_file() {
        Ok(())
    } else {
        Err(format!(
            "missing {description}: {}; run the release build first",
            path.display()
        )
        .into())
    }
}

fn home_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .ok_or_else(|| "neither USERPROFILE nor HOME is set".into())
}

fn digest_file(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    io::copy(&mut file, &mut digest)?;
    Ok(format!("{:x}", digest.finalize()))
}

fn install_immutable(source: &Path, destination: &Path, expected: &str) -> io::Result<()> {
    if destination.exists() {
        if digest_file(destination)? == expected {
            return Ok(());
        }
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "versioned executable has unexpected contents: {}",
                destination.display()
            ),
        ));
    }
    let temporary = destination.with_extension(format!("{}.tmp", std::process::id()));
    fs::copy(source, &temporary)?;
    match fs::rename(&temporary, destination) {
        Ok(()) => Ok(()),
        Err(_) if destination.is_file() && digest_file(destination)? == expected => {
            let _ = fs::remove_file(temporary);
            Ok(())
        }
        Err(error) => {
            let _ = fs::remove_file(temporary);
            Err(error)
        }
    }
}

fn install_launcher(
    source: &Path,
    destination: &Path,
    state_dir: &Path,
) -> Result<Option<PathBuf>, Box<dyn std::error::Error>> {
    let expected = digest_file(source)?;
    if destination.is_file() && digest_file(destination)? == expected {
        return Ok(None);
    }

    let temporary = destination.with_extension(format!("launcher.{}.tmp", std::process::id()));
    fs::copy(source, &temporary)?;
    let previous = if destination.exists() {
        let legacy_dir = state_dir.join("legacy");
        fs::create_dir_all(&legacy_dir)?;
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
        let legacy = legacy_dir.join(format!("liteshell-{stamp}-{}.exe", std::process::id()));
        if let Err(error) = fs::rename(destination, &legacy) {
            let _ = fs::remove_file(&temporary);
            return Err(
                format!("cannot move the existing LiteShell aside for migration: {error}").into(),
            );
        }
        Some(legacy)
    } else {
        None
    };

    if let Err(error) = fs::rename(&temporary, destination) {
        let _ = fs::remove_file(&temporary);
        if let Some(previous) = &previous {
            let _ = fs::rename(previous, destination);
        }
        return Err(format!("cannot install LiteShell launcher: {error}").into());
    }
    Ok(previous)
}

fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temporary, contents)?;
    let result = replace_file(&temporary, path);
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }
    let source = wide(source.as_os_str());
    let destination = wide(destination.as_os_str());
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(source, destination)
}
