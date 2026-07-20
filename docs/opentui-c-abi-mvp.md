# OpenTUI C ABI renderer MVP

## Decision

LiteShell keeps its existing C++ shell engine and Win32 input path.  The first
OpenTUI integration owns prompt and completion-surface generation and diffing:

```text
Shell / builtins / history / completion
                 |
             Terminal
        +--------+---------+
        |                  |
 Win32 key input     OpenTUI C ABI
 command output      candidates + prompt buffer
 child processes     cell diff + cursor
 scrollback
```

This is deliberately an inline, main-screen adapter rather than a full-screen
application. It reserves rows only while a completion menu is open, keeps the
prompt on the bottom row, and preserves normal terminal scrollback, the built-in
pager, and inherited-console external programs.

## Native ABI boundary

`src/opentui_renderer.cpp` loads `opentui.dll` next to `liteshell.exe` with
`LoadLibraryW`.  It imports only this pinned OpenTUI 0.4.5 surface:

- `createRenderer` / `destroyRenderer`
- `setClearOnShutdown` / `setRenderOffset` / `resizeRenderer`
- `getNextBuffer` / `bufferClear` / `bufferDrawText`
- `setCursorPosition` / `render`

The adapter does not call OpenTUI `setupTerminal`.  LiteShell remains the terminal
lifecycle and input owner, avoiding native capability queries being mixed into the
existing `ReadConsoleInputW` event stream.  OpenTUI owns buffer construction,
Unicode cell handling, frame comparison, ANSI output, and cursor placement.

The binary comes from the official `@opentui/core-win32-x64` npm package.  The
fetch script pins version 0.4.5 and verifies its published SHA-512 integrity value.
No Bun, Node, npm, Zig compiler, or OpenTUI TypeScript binding is required at
runtime or build time.

## Build and runtime requirement

The build copies OpenTUI beside the executable:

```powershell
.\build.ps1
```

There is no legacy renderer or runtime fallback. Interactive startup fails with a
clear error if VT output, `opentui.dll`, or the expected pinned ABI is unavailable.
Redirected stdin/stdout remains a plain non-interactive command stream for scripts
and tests; it is not an interactive renderer.

## fff-backed completion surface

`src/fff_finder.cpp` lazily loads the official `fff` 0.10.0 C library. The DLL and
instance are first touched by an indexed operation and rooted at the current
working directory. `cd` uses directory search, `ls` uses mixed search,
`find` uses file search, and `rg` uses live grep. The watcher is disabled; changing
directories restarts the index explicitly.

The completion surface uses at most six candidate rows plus a header above the
prompt. Up/Down changes selection, Tab or Enter inserts it, and Esc closes the
surface. `find`/`rg` selection inserts `less <path>` so a chosen result immediately
becomes a useful preview command.

## MVP limitations and next increments

- The prompt remains one editable row. Long input scrolls horizontally; multiline
  editing is not part of this increment.
- Candidate detail is a compact one-line preview. A dedicated preview pane is not
  part of this increment.
- Colors are currently applied to prompt and input by the adapter. Builtin output
  stays on the existing output path.
- The C ABI is pinned because upstream exposes functions but does not publish a
  stable C header/version-negotiation contract for this direct integration.

Before building tabs and panes, the adapter should grow into a render-surface
interface. True terminal panes additionally require ConPTY session management and
a VT screen model; OpenTUI remains the compositor rather than the terminal emulator.
