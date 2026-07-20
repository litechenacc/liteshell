# PRD v0.1 traceability

This matrix maps the MVP's named requirements to implementation and verification
evidence. Product targets that inherently depend on the corporate endpoint and a
human-operated TUI remain called out as target-machine acceptance items rather than
being inferred from development-machine tests.

| PRD area | Implementation evidence | Verification evidence |
|---|---|---|
| Single native process | `main.cpp`, `Shell::run`; no daemon/IPC/runtime | Release dependency audit: `KERNEL32.dll` only |
| Current-directory prompt | `Shell::prompt`, configurable format/style | smoke prompt/cwd transitions; hidden-console screen |
| Line editing | `Terminal::readLine`, Win32 `ReadConsoleInputW` | `tests/interactive.ps1`: Backspace, Delete, Left/Right, Home/End, Enter, Ctrl-C/L |
| History | `History`, Up/Down navigation, UTF-8 bounded file | smoke persistence/capacity/dedup checks; hidden-console Up |
| Tab completion | `Shell::complete`, `FffFinder`, lazy PATH command cache | hidden-console builtin, PATH executable, file/folder completion, selectable candidate surface |
| Indexed find/grep | `FffFinder` C ABI adapter, `builtinFind`, `builtinRg` | smoke path and unique-content probes; clean short-session teardown |
| Unicode/drive paths | wide Win32 and `std::filesystem` APIs | smoke Chinese path with spaces and UTF-8 filename |
| Parser | `parseCommandLine` quotes, quote escapes, `$VAR`, `${VAR}`, `%VAR%`, `~` | smoke quoted paths and `$HOME`; integration commands exercise Windows backslashes |
| `cd` / `pwd` | `builtinCd`, `builtinPwd` | smoke cwd persistence; hidden-console edited/completed `pwd` |
| `ls` | `builtinLs`, `-a`, `-l`, metadata, hidden attributes, sorting | smoke `ls -la` on Unicode fixture |
| `cat` | multi-file UTF-8 decode and binary probe | smoke UTF-8 output and binary refusal |
| `tail` | configurable default, `-n`, reverse chunk reads | smoke last-two-line assertion |
| `less` | alternate-screen pager and non-interactive output | hidden-console all navigation keys plus q/Esc; smoke redirected mode |
| `clear` | VT/Win32 clear | hidden-console Ctrl-L uses the same clear path |
| `which` | builtin marker and external resolver | smoke resolves `ls` and `cmd` |
| `.exe` / `.com` | direct `CreateProcessW` | smoke `cmd` and `more.com` |
| `.cmd` / `.bat` | `cmd.exe /d /s /c` | smoke quoted argument through both fixtures |
| `.ps1` | `pwsh -NoLogo -NoProfile -File` (Windows PowerShell fallback) | smoke quoted argument through fixture |
| Foreground child | inherited handles, wait, exit code, mode reset | smoke child-output sequence; local `ssh`, `nvim`, `codex` version launches followed by `pwd` |
| Static configuration | `src/config.h` | Release/Debug rebuilds include prompt/history/display constants |
| Startup target | static CRT, LTO, no startup discovery | five-process average 112.31 ms with Sophos services active |
| Builtin latency | in-process dispatch and chunked tail | pwd 6.09 ms; ls 0.79 ms; cat 1.06 ms; tail 0.77 ms round trips |
| Idle memory target | native static executable | 5.22 MiB working set with Sophos services active |

## Target-environment acceptance still required

- Cold-start measurement after reboot/cache eviction (Sophos was active for all
  current measurements, but disk cache state was not forcibly reset).
- Subjective workflow comparison against PowerShell and multiple short-lived tools.
- Manual full-screen sessions for `nvim`, `ssh`, and `codex` in the user's actual
  Windows Terminal profile. Version commands and generic foreground restoration are
  automated, but they do not prove every third-party TUI's rendering behavior.
