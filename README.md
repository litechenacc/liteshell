# LiteShell

LiteShell is a small native Windows shell implemented in Rust. Ratatui owns the
interactive UI, including output scrollback, editing, completion, status, and the
built-in pager. Redirected input/output uses a plain, ANSI-free command stream.

## Requirements

- Rust 1.82 or newer
- `x86_64-pc-windows-msvc` toolchain
- [`just`](https://github.com/casey/just) (optional command runner)

No C++ compiler, OpenTUI, PowerShell build script, Node, Bun, or async runtime is
required.

## Build and test

```text
just build
just test
just release
just install
```

`just install` builds the release executable and a small stable launcher. The
launcher is installed as `~/.local/bin/liteshell.exe`; immutable shell builds
are stored below `~/.local/bin/.liteshell/versions/<sha256>/`. Installation
switches a small `current` file atomically, so existing instances keep running
their old build while the next invocation immediately uses the new one.

On the first install over an older standalone `liteshell.exe`, the installer
moves the running executable into `.liteshell/legacy` before placing the
launcher. Windows lets the existing process continue from its renamed file.
The launcher removes obsolete migration files once they are no longer in use.

Equivalent Cargo commands are:

```text
cargo build -p liteshell
cargo test --workspace
cargo build --release -p liteshell
```

Create `build/release` with the pinned `fff_c.dll` and license:

```text
just package
```

Dependency downloads happen only in the explicit `xtask` command, never in a
Cargo build script.

## Run

```text
cargo run -p liteshell
```

Use `liteshell --version` for build version information and `liteshell --help`
for the tabular command overview. Every builtin and Windows-translated command
has detailed help through `<command> --help` or `help <command>`.

Interactive mode uses a plain-text, bounded transcript with a divider between
command/response groups. The first prompt begins at the top and each subsequent
prompt follows the preceding response, matching a conventional shell's continuous
flow. The two-line prompt follows Starship's default style, completion opens as a
fuzzy-search overlay, Ctrl-R provides fuzzy history search, and a persistent
statusline occupies the bottom row. Page Up/Down navigate LiteShell's bounded
transcript. Mouse input remains owned by the terminal, so normal text selection
and the terminal's native wheel-driven scrollback continue to work.
`cd` completion combines immediate child directories with a frequency-and-recency
ranked database of previously visited directories. Include `*` in the path token
to explicitly request a cancellable recursive directory search, for example
`cd ~\*lite`; every destination remains visible and user-selected in the popup.
`less` supports arrows, `j`/`k`, pages, `g`/`G`, and `q`. Output from external
batch commands such as `just`, `cargo`, and `git` streams into LiteShell's
scrollback. Terminal applications such as `ssh`, `nvim`, and `codex` temporarily
own the console; LiteShell restores Ratatui when they exit.

When either standard input or output is redirected, lines are parsed and run
without terminal initialization or ANSI escapes:

```text
(echo pwd& echo exit) | target\release\liteshell.exe
```

For terminal tools and agents, command mode executes one command string while
leaving standard input available to the command. It loads the same
`~/.liteshellrc`, environment, aliases, working-directory rules, builtins, and
Windows command resolver as the interactive shell:

```text
liteshell.exe -c "rg TODO crates | cat"
```

Command mode never emits a prompt, status line, divider, ANSI escape, or TUI
frame. Pipelines use concurrent anonymous OS pipes and may mix builtins with
external programs. Supported operators are `|`, `<`, `>`, `>>`, `2>`, `2>>`,
`2>&1`, `&&`, `||`, and `;`. Pipeline failures use `pipefail` semantics by
default so an earlier failed stage is visible to automation; pass
`--no-pipefail` for last-stage-only status. External stages are attached to a
kill-on-close Windows Job Object so terminal-tool timeouts do not leave process
trees behind.

The interactive status line defaults to `auto`/on and can be hidden with
`--status-line=off`. It is always absent in command and redirected modes.

## Commands

Rust builtins: `cd`, `pwd`, `ls`, `mkdir`, `rm`, `touch`, `cat`, `tail`, `less`,
`clear`, `which`, `find`, `rg`, `help`, and `exit`.

Windows command translations: `ps` (to `tasklist.exe`) and `kill` (to
`taskkill.exe`). Foreground `.exe`, `.com`, `.cmd`, `.bat`, and `.ps1` commands
remain supported.

Command aliases can be declared in `~/.liteshellrc`, for example
`alias ll='ls -l'`, `alias lg='lazygit'`, and `alias vi='nvim'`. See
[`docs/configuration.md`](docs/configuration.md) for details.

## Architecture

- `liteshell-core`: parser, editor, history, shell state, and output/search traits
- `liteshell-builtins`: modular in-process commands
- `liteshell-tui`: Ratatui rendering and terminal lifecycle
- `liteshell-windows`: command resolution and foreground process launch
- `liteshell-fff`: isolated optional DLL loading and safe search facade
- `liteshell`: interactive/non-interactive application
- `xtask`: verified dependency acquisition and packaging

See [`docs/rust-ratatui-migration-plan.md`](docs/rust-ratatui-migration-plan.md),
[`docs/colorization-design.md`](docs/colorization-design.md), and
[`docs/prd-traceability.md`](docs/prd-traceability.md). The command implementation
rule and supported Unix-like interfaces are documented in
[`docs/command-implementation-policy.md`](docs/command-implementation-policy.md).
