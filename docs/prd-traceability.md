# PRD v0.2 traceability

| Area | Rust implementation | Verification |
|---|---|---|
| Native process | `crates/liteshell/src/main.rs` | workspace release build |
| Ratatui UI | `liteshell-tui::draw`, `TerminalSession` | TUI unit/manual matrix |
| Terminal restoration | `TerminalSession::suspend` and RAII `Drop` | inherited child manual tests |
| Shell-owned cwd | `liteshell-core::ShellState` | builtin and redirected tests |
| Parser | `liteshell-core::parser` | quote/operator/backslash unit tests |
| Unicode editor | `liteshell-core::editor` | grapheme/emoji unit test |
| History | `liteshell-core::history` | capacity/dedup unit test |
| Bounded scrollback | `liteshell-tui::Scrollback` | deterministic state tests |
| Output isolation | `OutputSink` / `OutputEvent` | redirected ANSI assertion |
| Filesystem builtins | `liteshell-builtins` modules/API | `non_interactive.rs` and unit tests |
| Pager | `OutputEvent::Pager`, Ratatui pager mode | manual keys/resize/mouse matrix |
| Search | `SearchProvider`, `liteshell-fff` | find/rg acceptance fixtures |
| External commands | `liteshell-windows` | quoting unit tests and Windows acceptance |
| Plain mode | terminal detection in `liteshell` | `non_interactive.rs` |
| Packaging | `xtask`, `just package` | clean-machine package check |

## Baseline retained from v0.1

The C++ reference measured a 112.31 ms five-process warm startup average, a 5.22
MiB idle working set, and sub-7 ms measured builtin round trips on the target
endpoint. Rust release measurements must be collected on that same endpoint
before declaring performance parity.

## Target-environment acceptance still required

- cold/warm startup, working-set, latency, and distribution-size comparison;
- completion, resize, CJK, combining text, emoji, and scrollback copy behavior;
- real `nvim`, `ssh`, and `codex` child restoration, including Ctrl-C;
- pager keyboard and mouse behavior in the supported Windows Terminal profile;
- panic restoration and clean-machine `just package` verification.
