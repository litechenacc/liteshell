# Command implementation policy

LiteShell exposes familiar Unix-like command names on Windows, but every command
must be classified as either a **Rust builtin** or a **Windows command
translation**. The distinction is part of the user-facing command model: command
completion and `which` identify translated commands instead of presenting them as
builtins.

## Build: Rust builtin

Build a command in Rust when it is a frequent, latency-sensitive operation; it
must read or mutate LiteShell-owned state such as the current directory; or its
portable filesystem behavior is small enough to implement and test precisely.
Builtins run in-process through `liteshell-builtins` and write through the shared
output sink.

Current examples are `cd`, `ls`, `mkdir`, `rm`, and `touch`. In particular,
starting PowerShell to create or remove a path adds process startup and quoting
cost without providing semantics that Rust's filesystem APIs lack.

## Translate: Windows command translation

Translate a command when Windows already ships a dedicated executable that owns
the platform-specific behavior, reproducing that behavior through Windows APIs
would be materially more complex, and child-process latency is acceptable for the
workflow. Translation parses LiteShell's Unix-like interface into an executable
path and an argument vector in `liteshell-windows`.

Translations must launch the target executable directly. They must not construct
a command string for `cmd.exe` or PowerShell, because doing so changes quoting and
introduces shell-injection risk. Interactive translations use the same captured,
cancellable external-command worker as other non-terminal child processes.

Current translations are:

| LiteShell command | Windows executable | Reason |
| --- | --- | --- |
| `ps` | `tasklist.exe` | Windows owns process enumeration, access rules, and process metadata. |
| `kill` | `taskkill.exe` | Windows owns termination permissions, forced termination, and process-tree handling. |

## Decision rule

Use **build** when responsiveness or LiteShell state ownership dominates and the
behavior has a small testable surface. Use **translate** when Windows-specific
semantics dominate and a dedicated inbox executable provides the operation. Do
not translate through PowerShell merely because it has a similarly named alias.
If neither side clearly wins, first define the Unix-like interface and error
semantics, then choose the implementation with the smaller platform-specific
surface.

## Supported Unix-like interfaces

The goal is useful muscle-memory compatibility, not complete GNU or POSIX option
parity. Unsupported options fail with status 2 instead of being silently ignored.

### Filesystem builtins

```text
mkdir [-p|--parents] directory...
rm [-f] [-r|-R] path...
touch file...
```

- `mkdir -p` creates missing parents and succeeds for an existing directory.
- `rm` does not remove directories unless `-r` or `-R` is supplied. `-f` ignores
  missing paths. Short flags may be combined, as in `rm -rf build`.
- Recursive `rm` refuses `.`, `..`, and filesystem roots. Directory links and
  junctions are removed as links rather than traversed.
- `touch` creates missing files and updates the modification time of existing
  files.

### Process translations

```text
ps [aux|-aux] [name|pid]
ps [-a] [-u] [-x] [-v] [name|pid]
kill [-s TERM|KILL] [--tree] pid...
kill [-TERM|-KILL|-15|-9] [--tree] pid...
```

- `ps` lists processes. `a` and `x` are accepted for Unix muscle memory;
  `u`/`v` request verbose `tasklist` output. A name adds an image-name substring
  filter and a numeric argument adds an exact PID filter. Thus the pipeline
  workflow `ps aux | grep code` is expressed as `ps aux code`.
- `kill` defaults to `TERM`, translated to `taskkill /PID`. `KILL` or signal 9
  adds `/F`; `--tree` adds `/T`. Windows cannot represent general Unix signals,
  so other signal names are rejected.

LiteShell deliberately does not add a general pipeline parser for process
filtering. Pipelines require separate syntax, stream wiring, cancellation, and
multi-process status semantics; the direct `ps` filter covers this workflow
without expanding the shell language.
