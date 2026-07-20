use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct ShellState {
    pub cwd: PathBuf,
    pub running: bool,
    pub last_status: i32,
}

impl ShellState {
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            cwd,
            running: true,
            last_status: 0,
        }
    }
    pub fn resolve(&self, path: impl AsRef<Path>) -> PathBuf {
        let path = path.as_ref();
        if path.is_absolute() {
            path.to_owned()
        } else {
            self.cwd.join(path)
        }
    }
    pub fn display_cwd(&self) -> String {
        display_path(&self.cwd)
    }

    pub fn prompt(&self) -> String {
        let cwd = self.display_cwd();
        let displayed = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .map(|home| display_path(&home))
            .and_then(|home| {
                let cwd_lower = cwd.to_lowercase();
                let home_lower = home.to_lowercase();
                if cwd_lower == home_lower {
                    Some("~".to_owned())
                } else if cwd_lower.starts_with(&format!("{home_lower}\\")) {
                    Some(format!("~\\{}", &cwd[home.len() + 1..]))
                } else {
                    None
                }
            })
            .unwrap_or(cwd);
        format!("{displayed}\n❯ ")
    }
}

/// Convert a Windows verbatim path to its conventional display form while
/// retaining the original path in shell state for filesystem operations.
fn display_path(path: &Path) -> String {
    let displayed = path.to_string_lossy();
    if let Some(rest) = displayed.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = displayed.strip_prefix(r"\\?\") {
        rest.to_owned()
    } else {
        displayed.into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_windows_verbatim_prefix() {
        assert_eq!(display_path(Path::new(r"\\?\D:\work")), r"D:\work");
        assert_eq!(
            display_path(Path::new(r"\\?\UNC\server\share")),
            r"\\server\share"
        );
    }
}
