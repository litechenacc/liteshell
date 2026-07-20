# LiteShell MVP implementation plan

## Product intent

LiteShell keeps frequent, low-cost filesystem work in one native Windows process.
This avoids paying process creation, runtime initialization, and endpoint scanning
cost for every `ls`, `cat`, or `tail` invocation.

The implementation follows `liteshell_mvp_prd.md` v0.1, uses C++20, and is built
directly with Microsoft `cl.exe`.

## Completed architecture

```text
Windows Terminal / PowerShell
└── liteshell.exe
    ├── Terminal          Win32 input events, output, pager keys
    ├── OpenTUI adapter   C ABI prompt buffer, cell diff, cursor rendering
    ├── fff adapter       C ABI path/content index and fuzzy search
    ├── Parser            quoting, escapes, environment and home expansion
    ├── History           bounded UTF-8 file persistence
    ├── Shell             prompt, dispatcher, completion state
    ├── Builtins          cd/pwd/ls/find/rg/cat/tail/less/clear/which
    └── Process launcher  PATH resolution, CreateProcessW, foreground wait
```

The executable remains a single process while idle. Core commands never launch a
child process. External programs inherit the active console and standard handles;
LiteShell waits for them and restores its console modes afterward.

## Delivered milestones

### Milestone 0: startup and prompt

- Native executable with current-directory prompt.
- Compile-time prompt template and path style.
- Clean `exit`, `quit`, and end-of-input behavior.

### Milestone 1: filesystem builtins

- `cd`, `pwd`, `ls`, `cat`, `tail`, `clear`, and `which` are in-process.
- Unicode paths, quoted paths, useful errors, directory-first sorting, `ls -a`,
  and `ls -l` metadata are implemented.
- Text readers validate UTF-8 and refuse probable binary files.
- `tail` seeks backward in chunks rather than loading the entire log.

### Milestone 2: external foreground processes

- PATH/current-directory resolution for `.exe`, `.com`, `.cmd`, `.bat`, `.ps1`.
- Direct `CreateProcessW` for native executables.
- `/d /s /c` routing for batch scripts and profile-free PowerShell script routing.
- Console inheritance, foreground wait, exit-code retrieval, and mode restoration.

### Milestone 3: line editor and history

- Win32 console event handling for cursor keys, Home/End, Backspace/Delete,
  Ctrl-C, Ctrl-L, and history navigation.
- Prefix completion for builtins, PATH executables, and filesystem entries.
- Bounded persistent history without adjacent duplicates.

### Milestone 4: built-in pager

- Alternate-screen pager with `j`/Down, `k`/Up, Space/PageDown, PageUp, `g`, `G`,
  `q`, and Esc.
- File position and percentage status line.
- Non-interactive `less` prints its decoded content without entering pager mode.

### Milestone 5: performance and verification

- Release uses `/O2 /GL /LTCG /MT /OPT:REF /OPT:ICF`.
- Automated smoke coverage includes all external dispatch formats and the core
  Unicode/text flows.
- Local redirected-prompt measurement is recorded in `validation.md`.
- The development/target host had active Sophos endpoint services during build,
  integration, startup, memory, and builtin round-trip measurements.
- A reboot/cache-evicted cold-start run remains a deployment-time acceptance check.

### Milestone 6: indexed completion surface

- Removed the legacy interactive renderer; OpenTUI is the single prompt and
  candidate-surface renderer.
- Added the pinned `fff` 0.10.0 C ABI library with lazy per-working-directory
  indexing and no filesystem watcher.
- `cd` completion searches directories, `ls` searches files and directories,
  `find` searches indexed paths, and `rg` searches indexed content.
- Multiple Tab candidates appear above the prompt and support Up/Down,
  Tab/Enter, and Esc without writing candidate lists into scrollback.

## Deliberate constraints

The MVP has no daemon, IPC, ConPTY, session manager, plugin/module discovery,
startup scripts, pipelines, redirects, aliases, job control, continuously running
filesystem watchers, or full shell language. The `fff` index is created lazily on
first completion/search and restarted when the working directory changes.
