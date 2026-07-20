use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    pub history_capacity: usize,
    pub scrollback_lines: usize,
    pub scrollback_bytes: usize,
    pub default_tail_lines: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            history_capacity: 5_000,
            scrollback_lines: 10_000,
            scrollback_bytes: 4 * 1024 * 1024,
            default_tail_lines: 10,
        }
    }
}

pub fn history_path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
        })
        .join("LiteShell")
        .join("history")
}
