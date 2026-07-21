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
```

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

Interactive mode uses a plain-text, bounded transcript with a divider between
command/response groups. The first prompt begins at the top and each subsequent
prompt follows the preceding response, matching a conventional shell's continuous
flow. The two-line prompt follows Starship's default style, completion opens as a
fuzzy-search overlay, Ctrl-R provides fuzzy history search, and a persistent
statusline occupies the bottom row. Page Up/Down and the mouse wheel navigate the
transcript.
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

## Commands

`cd`, `pwd`, `ls`, `cat`, `tail`, `less`, `clear`, `which`, `find`, `rg`, `help`,
`exit`, and foreground `.exe`, `.com`, `.cmd`, `.bat`, and `.ps1` commands.

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
[`docs/prd-traceability.md`](docs/prd-traceability.md).
