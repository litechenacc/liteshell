# MVP validation record

Validation environment: Windows Terminal, MSVC 19.51 x64, and active Sophos
Network Threat Protection, Endpoint Defense, File Scanner, MCS Agent, and MCS
Client services. Measurements below therefore include the endpoint stack active on
this host; cache-evicted cold startup and subjective daily-workflow comparison still
require deployment-time observation.

## OpenTUI + fff C ABI integration (2026-07-18)

- OpenTUI native package: `@opentui/core-win32-x64` 0.4.5.
- The published SHA-512 integrity value is checked before extraction.
- `fff` C library: official Windows MSVC x64 release 0.10.0; SHA-256 is checked
  before it is copied beside the executable.
- `link.exe /dump /exports` confirmed the imported renderer, buffer, cursor, and
  render-offset symbols in the packaged DLL.
- Release x64 built successfully with `cl.exe` 19.51 and the static CRT.
- Release artifacts: `liteshell.exe` 431,104 bytes; `opentui.dll` 3,786,120 bytes;
  `fff_c.dll` 13,640,704 bytes.
- `tests/smoke.ps1` passed with the OpenTUI-enabled Release build.
- `tests/interactive.ps1` passed against the OpenTUI-enabled Release build.
- A short redirected `find`/exit regression passed with exit code 0, covering the
  Rust index lifetime and process teardown boundary.
- Legacy rendering and its build option were removed; missing OpenTUI is now a
  startup error for an interactive console.

## Automated integration coverage

Run:

```powershell
.\build.ps1 -Configuration Release
.\tests\smoke.ps1
.\tests\interactive.ps1
```

The suite verifies:

- current-directory state and quoted Chinese/space-containing paths;
- `ls -la`, UTF-8 `cat`, backward-reading `tail`, and redirected `less`;
- `$HOME` expansion and persistent UTF-8 history;
- builtin/external `which` resolution;
- `.exe`, `.com`, `.cmd`, `.bat`, and `.ps1` foreground execution;
- child output followed by another usable shell prompt;
- probable binary-file refusal;
- `fff` fuzzy path lookup and indexed content lookup through `find` and `rg`.

The hidden-console suite starts the actual Release executable with real console
handles and injects Win32 key events for history, Left/Delete, Home/End, Backspace,
Tab, Ctrl-L, Ctrl-C, and all documented pager navigation keys. It asserts clean
exit, valid edited/completed commands, selectable fff candidates, pager traversal,
and Ctrl-C handling.

Prompt and candidate redraw is emitted as one diffed frame. Normal prompt input
leaves mouse input with the terminal host so scrollback remains available; the
pager switches modes locally and accepts `MOUSE_WHEELED` events for line scrolling.

`ssh -V`, `nvim --version`, and `codex --version` were additionally launched from
LiteShell on the development machine, followed by `pwd`; all returned control to
the shell.

## Historical pre-index performance snapshot

These figures predate the `fff` integration and are retained only as a baseline;
current startup/working-set acceptance should be remeasured on the target machine.
Five fresh Release processes were timed from `Process.Start()` until the redirected
prompt became readable:

```text
run 1: 112.95 ms, 5.22 MiB working set
run 2: 107.45 ms, 5.22 MiB working set
run 3: 109.46 ms, 5.22 MiB working set
run 4: 106.48 ms, 5.22 MiB working set
run 5: 125.23 ms, 5.22 MiB working set
average: 112.31 ms
```

This is below the PRD's 300 ms warm-start target and 30 MiB idle working-set
target in this environment. `dumpbin /dependents` reports only `KERNEL32.dll` for
the roughly 388 KiB statically linked Release executable.

One long-lived redirected shell was also timed from submitting each builtin until
the next prompt became readable (including pipe rendering overhead):

```text
pwd:                         6.09 ms
ls:                          0.79 ms
cat tests\fixtures\com.txt:  1.06 ms
tail tests\fixtures\tail.txt: 0.77 ms
```

These round trips are below the PRD's local-directory latency targets on this host.

## Target-machine manual acceptance

The following checks require a real interactive Windows Terminal and, for final
performance sign-off, the target endpoint-protection environment:

1. Exercise the line-editor and pager keys once in the target terminal to catch
   host-specific rendering differences.
2. Open and exit real `nvim`, `ssh`, and `codex` interactive sessions; confirm the
   prompt, cursor, and keyboard modes are restored.
3. Measure cold startup after reboot/cache eviction and builtins in large local
   directories.
4. Compare the daily workflow against short-lived PowerShell/executable commands
   under Sophos.
