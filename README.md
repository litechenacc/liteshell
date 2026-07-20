# LiteShell

LiteShell is a small, long-lived native Windows shell. Filesystem commands run
inside the shell process, avoiding the repeated process startup and endpoint
inspection cost of short-lived command-line tools.

The MVP targets Windows 10/11 and is built directly with Microsoft `cl.exe`.
It has no PowerShell profile, plugin scan, daemon, managed runtime, or startup
script dependency.

## Features

- OpenTUI native C ABI renderer for the prompt and completion surface; the legacy
  repaint renderer has been removed.
- Native line editor: Backspace, Delete, arrows, Home/End, history,
  Ctrl-C, Ctrl-L, and selectable Tab completion.
- `fff` 0.10.0 C ABI indexing: directory candidates for `cd`, mixed candidates
  for `ls`, file search through `find`, and indexed content search through `rg`.
- Completion candidates render above the prompt; Up/Down selects, Tab/Enter
  accepts, and Esc closes the surface.
- Flicker-reduced single-frame prompt redraw while preserving terminal scrollback.
- Persistent UTF-8 history under `%LOCALAPPDATA%\LiteShell\history`.
- In-process `cd`, `pwd`, `ls`, `cat`, `tail`, `less`, `clear`, and `which`.
- Keyboard and mouse-wheel navigation in the built-in `less` pager.
- Single/double quotes, simple quote escaping, `$VAR`, `${VAR}`, `%VAR%`, and `~`.
- Unicode and space-containing Windows paths.
- Foreground `.exe`, `.com`, `.cmd`, `.bat`, and `.ps1` launching.
- Direct terminal inheritance for interactive tools such as `ssh`, `nvim`, and
  `codex`, followed by console-mode restoration.
- Compile-time prompt, history, and display configuration in `config.h`.

Pipes, redirects, jobs, scripting, aliases, plugins, ConPTY sessions, and
background processes are intentionally outside this MVP.

## Build

Open an x64 Visual Studio Developer PowerShell and run:

```powershell
.\build.ps1
```

The executable is written to `build\release\liteshell.exe`. Debug builds are
available with:

```powershell
.\build.ps1 -Configuration Debug
```

Release builds use the static CRT, whole-program optimization, and link-time code
generation. The build downloads the pinned official OpenTUI 0.4.5 and `fff`
0.10.0 Windows x64 DLLs once, verifies their published hashes, and copies them
beside LiteShell. OpenTUI is required for interactive startup; `fff_c.dll` is
loaded lazily by the first indexed completion/search. Bun, Node, npm, Cargo, and
Zig are not required.

## Run

```powershell
.\build\release\liteshell.exe
```

Examples:

```text
ls -la
cd "D:\path with spaces"
find renderer
rg OpenTUI
cat README.md
tail -n 50 build.log
less report.txt
nvim .
ssh host
codex
exit
```

## Configure

Edit [`src/config.h`](src/config.h), then rebuild. It controls:

- prompt style and `{cwd}`, `{leaf}`, `{drive}`, `{user}` formatting;
- history capacity and location;
- default `tail` line count and other small static limits.

See [`docs/configuration.md`](docs/configuration.md) for examples.

## Test

```powershell
.\build.ps1
.\tests\smoke.ps1
.\tests\interactive.ps1
```

The smoke suite covers Unicode/quoted paths, `fff` path/content search, filesystem
builtins, history persistence, binary detection, and all five external file dispatch
types. The hidden-console integration test drives the real Win32 line editor,
selectable completion surface, and pager with key events. A final manual check is
still recommended in the target Windows Terminal/Sophos environment.

Project design and requirement evidence are documented under [`docs/`](docs/),
including the [`PRD traceability matrix`](docs/prd-traceability.md) and
[`OpenTUI C ABI MVP design`](docs/opentui-c-abi-mvp.md).
