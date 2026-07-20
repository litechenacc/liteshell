# Configuration

LiteShell's static defaults are in `crates/liteshell-core/src/config.rs`.
`Config::default()` controls:

- history capacity (5,000 commands);
- internal scrollback line and byte limits;
- default `tail` line count.

History is stored as UTF-8 at `%LOCALAPPDATA%\LiteShell\history`. The prompt uses
the shell-owned working directory and is produced by `ShellState::prompt`.

Configuration is deliberately compile-time. LiteShell does not evaluate startup
scripts, profiles, or plugins.
