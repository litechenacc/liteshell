# Configuration

LiteShell's static defaults are in `crates/liteshell-core/src/config.rs`.
`Config::default()` controls:

- history capacity (5,000 commands);
- internal scrollback line and byte limits;
- default `tail` line count.

History is stored as UTF-8 at `%LOCALAPPDATA%\LiteShell\history`. The prompt uses
the shell-owned working directory and is produced by `ShellState::prompt`.

At startup LiteShell loads environment assignments from `~/.liteshellrc`, when
the file exists. `~` uses `HOME` first and falls back to `USERPROFILE`. Supported
forms are `NAME=value`, `export NAME=value`, and `set NAME=value`. Values can
reference inherited variables or earlier assignments with `%NAME%`, `$NAME`, or
`${NAME}`. Single-quoted values are literal; double-quoted and unquoted values
are expanded.

The file is deliberately data-only. LiteShell does not execute startup commands,
profiles, or plugins from it.
