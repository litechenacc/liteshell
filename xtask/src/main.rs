use sha2::{Digest, Sha256};
use std::{
    env, fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
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
        _ => return Err("usage: cargo xtask <fetch|package>".into()),
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
