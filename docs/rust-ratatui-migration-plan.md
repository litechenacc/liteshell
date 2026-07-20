# Rust + Ratatui migration plan

## Status

Implemented as the Rust workspace and Ratatui application. Target-machine
performance measurements and the manual Windows Terminal acceptance matrix remain
to be recorded.

This plan replaces the former C++20/OpenTUI implementation with a Rust application
using Ratatui.

This document intentionally treats the language migration and the UI redesign as
one product transition, but divides implementation into independently testable
stages. The existing C++ executable remains the behavioral reference until the
Rust executable passes the parity and acceptance gates in this document.

## Goals

- Replace the C++20 shell implementation with Rust targeting Windows 10/11.
- Replace the mixed OpenTUI/direct-console UI with one Ratatui-owned interactive
  application surface.
- Preserve the current in-process filesystem commands, parser behavior, history,
  indexed search, completion, and foreground external process support.
- Preserve a plain non-interactive mode for redirected stdin/stdout and automation.
- Keep startup fast, memory usage small, and runtime deployment native.
- Isolate Windows APIs and unsafe third-party FFI from the shell domain model.
- Keep builtins modular without creating one crate per command.

## Non-goals for the initial migration

- Pipes, redirects, jobs, scripting, aliases, or plugins.
- Background processes.
- Running external interactive programs inside Ratatui panes.
- ConPTY terminal emulation, tabs, or multiple shell sessions.
- Changing the pinned `fff` implementation while the shell is being migrated.
- Preserving the OpenTUI C ABI as a long-term rendering fallback.

## Target user experience

### Interactive mode

When stdin and stdout are attached to a terminal, LiteShell enters one coherent
Ratatui application:

```text
+--------------------------------------------------------------+
| output / shell scrollback                                    |
|                                                              |
|                                                              |
+--------------------------------------------------------------+
| completion candidates, when open                             |
+--------------------------------------------------------------+
| D:\work\project> command being edited                        |
+--------------------------------------------------------------+
| status / transient errors                                    |
+--------------------------------------------------------------+
```

Ratatui owns rendering for:

- prompt and editable input;
- builtin output;
- completion candidates and selection;
- status and error messages;
- history-backed shell scrollback;
- the built-in pager.

Builtin commands must not print directly to the console in interactive mode.
They emit output through a shell output abstraction, which updates application
state and is then rendered by Ratatui.

This design deliberately replaces normal terminal scrollback while the shell UI
is active with an application-managed scrollback buffer. Scrollback capacity,
navigation, copy behavior, and memory limits must therefore be explicit and
covered by acceptance testing.

### External foreground programs

The first Rust release preserves direct console inheritance for `ssh`, `nvim`,
`codex`, and other interactive programs:

1. finish the current Ratatui frame;
2. disable raw mode and mouse capture;
3. leave the alternate screen and restore console state;
4. run the child with inherited standard handles and the shell working directory;
5. wait for the child;
6. restore the Ratatui terminal and redraw the complete application;
7. append the child's exit status to shell scrollback.

The child output is visible while the child owns the console, but is not captured
into LiteShell's internal scrollback. Capturing and embedding child output requires
ConPTY and a VT screen model and is a separate future project.

Terminal suspension and restoration must be panic-safe. A child launch failure
must also restore the Ratatui session before displaying the error.

### Non-interactive mode

When stdin or stdout is redirected, LiteShell does not initialize Crossterm or
Ratatui. It retains a plain command-stream mode covered by the Rust redirected-mode integration test:

```text
stdin lines -> parse -> dispatch -> plain stdout/stderr
```

No ANSI control sequences may be emitted in this mode.

## Proposed workspace

```text
Cargo.toml
Cargo.lock
.cargo/
  config.toml
crates/
  liteshell/
    src/
      main.rs
      app.rs
      event_loop.rs

  liteshell-core/
    src/
      lib.rs
      shell.rs
      parser.rs
      history.rs
      completion.rs
      command.rs
      config.rs
      output.rs

  liteshell-builtins/
    src/
      lib.rs
      cd.rs
      pwd.rs
      ls.rs
      cat.rs
      tail.rs
      less.rs
      clear.rs
      which.rs
      find.rs
      rg.rs
      help.rs

  liteshell-tui/
    src/
      lib.rs
      terminal_session.rs
      ui.rs
      input.rs
      scrollback.rs
      completion_view.rs
      pager_view.rs

  liteshell-windows/
    src/
      lib.rs
      process.rs
      command_search.rs
      console.rs
      paths.rs

  liteshell-fff/
    src/
      lib.rs
      ffi.rs
      loader.rs
      search.rs

xtask/
  src/main.rs
```

The initial workspace should not create a crate for every builtin. Modules are
sufficient for command-level organization. Crate boundaries are reserved for
platform APIs, unsafe FFI, UI ownership, and domain logic.

### Dependency direction

```text
liteshell
  +-- liteshell-core
  +-- liteshell-builtins
  +-- liteshell-tui
  +-- liteshell-windows
  `-- liteshell-fff

liteshell-builtins --> liteshell-core
liteshell-tui      --> liteshell-core
liteshell-windows  --> liteshell-core types only where necessary
liteshell-fff      --> liteshell-core search interfaces only
```

`liteshell-core` must not depend on Ratatui, Crossterm, Win32, OpenTUI, or the
`fff` ABI. This allows parser, history, command dispatch, and editor-state tests
to run without a real terminal.

## Principal design changes

### Application state and event loop

The current `Terminal::readLine` blocks until a complete line is returned. The
Ratatui implementation instead uses an application event loop:

```text
read terminal event
  -> update App state
  -> optionally execute command
  -> render one frame
  -> repeat
```

The application state should explicitly represent modes:

```rust
pub enum AppMode {
    Editing,
    Completion,
    Pager,
    RunningChild,
    Exiting,
}
```

The exact Rust API may evolve, but mode transitions must remain explicit and
unit-testable. Rendering reads state and must not perform filesystem searches,
launch processes, or mutate shell state.

### Shell-owned working directory

The Rust shell should keep its working directory in shell state rather than
using the process-global current directory as its primary source of truth:

```rust
pub struct ShellState {
    pub cwd: PathBuf,
    pub running: bool,
    // history, last status, and other domain state
}
```

Builtin path resolution is relative to this value. External processes receive it
as their working directory. This prevents tests and future sessions from sharing
a mutable process-global directory.

### Command API and output

Builtins should execute through a shared context and output sink. A representative
shape is:

```rust
pub struct CommandContext<'a> {
    pub cwd: &'a mut PathBuf,
    pub output: &'a mut dyn OutputSink,
    pub search: &'a mut dyn SearchProvider,
    pub terminal: Option<&'a mut dyn InteractiveServices>,
}

pub trait OutputSink {
    fn emit(&mut self, event: OutputEvent);
}
```

`OutputEvent` should distinguish normal text, errors, and optional styled or
structured rows without exposing Ratatui types from `liteshell-core`. Interactive
mode converts events into scrollback entries. Non-interactive mode writes them to
stdout/stderr.

Commands such as `ls` may initially emit plain text lines. Semantic styling can
be introduced after behavioral parity without changing command dispatch.

### Text and paths

- Editable command lines and display text use `String`.
- Filesystem paths and child arguments use `PathBuf` and `OsString`.
- Lossy path conversion is allowed only at a documented display boundary.
- Open files as bytes and explicitly validate or decode UTF-8 where current
  behavior requires UTF-8 text.
- Use `unicode-width` for terminal cell width and `unicode-segmentation` where
  cursor movement must respect grapheme clusters.

The new editor must improve on the current UTF-16 surrogate-pair handling by
covering combining marks and emoji sequences in tests.

### Terminal ownership

`liteshell-tui::TerminalSession` owns the interactive terminal lifecycle:

- enter/leave alternate screen;
- enable/disable raw mode;
- enable/disable mouse capture when needed;
- construct and restore `ratatui::Terminal`;
- recover terminal state from errors and panics where possible;
- suspend and resume around child processes.

There must be one owner for terminal lifecycle. Builtins and the renderer must not
independently toggle console modes.

## Rust libraries

The initial implementation should evaluate and lock compatible releases of:

- `ratatui` for layout, buffers, widgets, and frame rendering;
- `crossterm` for the Ratatui backend, terminal events, raw mode, and alternate
  screen control;
- `unicode-width` and `unicode-segmentation` for editor/display correctness;
- `windows` for Windows process and console APIs not adequately represented by
  the Rust standard library or Crossterm;
- `libloading` for the pinned `fff_c.dll` API;
- `thiserror` for library error types;
- `tempfile` as a development dependency for filesystem tests.

Dependency versions are selected when the workspace is bootstrapped and committed
in `Cargo.lock`. Avoid broad version ranges and unnecessary async runtimes.
Tokio is not required for the initial synchronous shell.

## OpenTUI removal

OpenTUI is removed rather than retained as a fallback.

The following files are replaced by `liteshell-tui`:

- `src/opentui_renderer.cpp`
- `src/opentui_renderer.hpp`
- the OpenTUI-specific portions of `src/terminal.cpp`
- `scripts/fetch-opentui.ps1`

The distribution no longer contains:

- `opentui.dll`
- `LICENSE.opentui`

Removal occurs only after the Ratatui UI passes interactive acceptance tests.
During development, the C++ reference build continues to use OpenTUI and its
existing fetch script.

## fff migration policy

The first Rust release continues to load the pinned official `fff_c.dll` 0.10.0
ABI at runtime. Its unsafe declarations and ownership rules live only in
`liteshell-fff::ffi`.

The Rust wrapper must preserve:

- lazy DLL and finder initialization;
- search rooted at the shell working directory;
- index restart after `cd`;
- the current initial scan timeout behavior;
- result-specific free functions;
- the existing short-process teardown workaround until proven unnecessary.

Directly depending on an upstream Rust `fff` crate is considered only after the
main migration, and only if upstream provides a suitable stable library API.

## Build and packaging

### Developer commands

Cargo becomes the compiler and test driver:

```powershell
cargo build --workspace
cargo test --workspace
cargo build --release -p liteshell
```

The supported target is initially:

```text
x86_64-pc-windows-msvc
```

The release profile should enable LTO and use an explicit panic policy. Static
MSVC CRT linkage should be evaluated against binary size, startup performance,
and deployment requirements before being made permanent.

### Distribution command

Use `just package` as the user-facing entry point. It delegates to Cargo and
`xtask`:

```text
verify toolchain
  -> fetch and verify fff_c.dll
  -> cargo build --release
  -> assemble build/release
  -> copy licenses
```

Network downloads must not run from a Cargo `build.rs`. Dependency acquisition
and integrity verification belong in `xtask` or explicit PowerShell scripts.

Expected initial Rust distribution:

```text
build/release/
  liteshell.exe
  fff_c.dll
  LICENSE.fff
```

## Test strategy

### Unit tests

`liteshell-core`:

- quoting and environment expansion;
- unsupported operators and malformed input;
- history capacity and adjacent deduplication;
- completion replacement ranges;
- editor cursor movement over ASCII, CJK, combining text, and emoji;
- application mode transitions.

`liteshell-builtins`:

- path resolution against an explicit shell cwd;
- `cd`, `pwd`, and `ls` behavior;
- Unicode filenames;
- UTF-8 BOM handling and invalid UTF-8 rejection;
- binary file detection;
- chunked `tail` behavior;
- command usage errors and exit statuses.

`liteshell-windows`:

- Windows argument quoting;
- `.exe`, `.com`, `.cmd`, `.bat`, and `.ps1` resolution;
- command-line construction;
- console restoration error paths where they can be simulated.

`liteshell-tui`:

- deterministic frame snapshots with Ratatui's test backend;
- narrow and wide terminal layouts;
- completion popup selection and scrolling;
- shell scrollback navigation;
- pager mode;
- resize behavior;
- status and error rendering.

### Existing integration tests

`crates/liteshell/tests/non_interactive.rs` is the non-interactive compatibility
gate.

Ratatui test-backend tests cover deterministic layout behavior. Windows console
integration and manual tests cover the real full-screen event loop without a C++
test driver or `cl.exe`.

### Manual acceptance matrix

Test in the supported Windows Terminal profile:

- startup and clean exit;
- typing, deletion, Home/End, and history navigation;
- CJK paths, emoji, combining marks, and paths containing spaces;
- completion open/select/accept/cancel;
- completion and input under terminal resize;
- shell scrollback navigation and copying text;
- `less` keyboard and mouse navigation;
- Ctrl-C and Ctrl-L behavior;
- `cmd`, PowerShell scripts, and failing child launches;
- full-screen `nvim`;
- interactive `ssh`;
- `codex` or another inherited-console TUI;
- terminal restoration after normal child exit, Ctrl-C, and shell panic.

## Performance and compatibility gates

Before replacing the C++ executable, collect equivalent measurements for both
implementations on the target endpoint:

- warm and cold startup time;
- idle working set;
- `pwd`, `ls`, `cat`, and `tail` round-trip latency;
- first and repeated `fff` search latency;
- input-to-frame latency during rapid typing;
- release executable and distribution size.

The Rust implementation must continue to satisfy the product targets documented
in `docs/prd-traceability.md`. Any intentional regression requires an explicit
product decision and an update to that traceability document.

Behavioral gates:

- all applicable core and builtin unit tests pass;
- the Rust redirected-mode integration test passes and emits no ANSI escapes;
- the adapted hidden-console integration test passes;
- manual child-process restoration tests pass;
- release output has no OpenTUI runtime dependency;
- no ANSI escapes appear in redirected output.

## Migration stages

### Stage 0: Baseline and decisions

- Record current startup, memory, builtin latency, and binary size.
- Capture screenshots or screen recordings of interactive edge cases.
- Add parser and builtin characterization cases missing from the smoke suite.
- Confirm the full-screen alternate-screen design and internal scrollback as the
  intended replacement for normal terminal scrollback.

Exit criterion: the current implementation has a reproducible behavioral and
performance baseline.

### Stage 1: Workspace and pure core

- Create the Cargo workspace and release profile.
- Implement config, parser, history, shell state, command result, and output types.
- Keep the core independent of Ratatui and Windows APIs.
- Port parser/history tests before porting interactive UI.

Exit criterion: core unit tests cover current parsing and history behavior.

### Stage 2: Non-interactive shell and builtins

- Implement explicit-cwd path resolution.
- Port `cd`, `pwd`, `ls`, `cat`, `tail`, `which`, and `help`.
- Implement plain stdout/stderr output sinks.
- Port external command resolution and foreground process launch.
- Add `clear` as a semantic output/control event rather than a direct ANSI write.

Exit criterion: the Rust binary passes the applicable smoke tests without
initializing Ratatui.

### Stage 3: Ratatui application shell

- Implement `TerminalSession`, `App`, and the Crossterm event loop.
- Implement unified output, input, status, and completion layout.
- Implement internal scrollback with a configured memory/capacity limit.
- Implement editor behavior and history navigation.
- Use Ratatui's test backend for deterministic UI tests.

Exit criterion: builtins execute inside the Ratatui surface without writing
around or underneath it.

### Stage 4: Completion and fff

- Port command and filesystem completion.
- Implement the `SearchProvider` interface.
- Port the pinned `fff_c.dll` loader and safe wrapper.
- Restore `find`, `rg`, and their completion flows.
- Verify index restart against the shell-owned cwd.

Exit criterion: completion, `find`, and `rg` satisfy smoke and interactive tests.

### Stage 5: Pager and child suspension

- Reimplement `less` as a Ratatui application mode.
- Add keyboard, page, top/bottom, resize, and mouse-wheel behavior.
- Implement panic-safe terminal suspension around foreground children.
- Validate console restoration with interactive third-party programs.

Exit criterion: pager and child-process acceptance matrices pass.

### Stage 6: Build cutover

- Add a `justfile` and `xtask`; do not retain PowerShell build scripts.
- Stop packaging `opentui.dll`.
- Update README, configuration documentation, and PRD traceability.
- Replace the C++ interactive test harness with Rust.
- Make the Rust executable the default release artifact.

Exit criterion: a clean machine can build, test, and package the documented Rust
release without a C++ compiler or OpenTUI.

### Stage 7: Remove the reference implementation

- Keep the final C++ revision available in version control history or a release
  tag.
- Remove C++ source, MSVC object build logic, and OpenTUI fetch logic.
- Audit licenses and runtime dependencies.
- Re-run target-environment startup, memory, and manual TUI acceptance tests.

Exit criterion: the repository has one supported Rust/Ratatui implementation.

## Risks and mitigations

### Alternate screen removes normal terminal scrollback

Mitigation: provide bounded internal scrollback, keyboard navigation, clear
copy-selection behavior, and explicit acceptance testing before cutover.

### External child transitions corrupt terminal state

Mitigation: centralize lifecycle in `TerminalSession`, use RAII guards, restore on
all error paths, and test real inherited-console applications.

### Crossterm Windows events differ from current Win32 input

Mitigation: characterize current key behavior first. Keep a narrow Windows event
adapter option behind `liteshell-tui` if Crossterm cannot represent a required
event, without allowing two components to own terminal mode simultaneously.

### UI migration hides shell behavior regressions

Mitigation: finish pure core and non-interactive parity before building the
Ratatui event loop. Keep output sinks independent from UI widgets.

### Large output consumes excessive memory

Mitigation: use a bounded scrollback model with measured byte and line limits.
Stream command output into the sink rather than assembling unlimited strings.

### Unsafe fff ABI remains fragile

Mitigation: isolate all declarations in one crate, preserve pinned hashes, validate
required symbols at load time, and expose only safe owned Rust values to callers.

### Ratatui dependency increases startup or binary size

Mitigation: avoid unnecessary features and async runtimes, inspect the release
dependency tree, and compare measured release performance with the current PRD
baseline.

## Completion definition

The migration is complete when:

- the supported executable is implemented in Rust;
- Ratatui owns every interactive shell frame and builtin UI;
- OpenTUI and its DLL have been removed;
- non-interactive behavior remains automation-safe;
- builtins use shared command/output interfaces rather than direct terminal writes;
- foreground child applications suspend and restore the UI reliably;
- `fff` unsafe code is isolated behind a safe Rust API;
- automated and manual acceptance gates pass;
- build, configuration, and traceability documentation describe only the new
  supported architecture.
