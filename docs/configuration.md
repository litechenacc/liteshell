# Configuration

LiteShell's static defaults are in `crates/liteshell-core/src/config.rs`.
`Config::default()` controls:

- history capacity (5,000 commands);
- internal scrollback line and byte limits;
- default `tail` line count.

History is stored as UTF-8 at `%LOCALAPPDATA%\LiteShell\history`. The prompt uses
the shell-owned working directory and is produced by `ShellState::prompt`.

Visited directories are stored at `%LOCALAPPDATA%\LiteShell\directories.db` and
ranked by frequency and recency for `cd` completion. The in-memory database is
appended to disk every five minutes while dirty and is flushed on exit.

At startup LiteShell loads environment assignments from `~/.liteshellrc`, when
the file exists. `~` uses `HOME` first and falls back to `USERPROFILE`. Supported
forms are `NAME=value`, `export NAME=value`, and `set NAME=value`. Values can
reference inherited variables or earlier assignments with `%NAME%`, `$NAME`, or
`${NAME}`. Single-quoted values are literal; double-quoted and unquoted values
are expanded.

The same file accepts bash-style command aliases. Alias expansion applies only
to the command name, supports chained aliases, and appends arguments entered at
the prompt to the alias value:

```text
alias l='ls'
alias ll='ls -l'
alias la='ls -la'
alias lg='lazygit'
alias vi='nvim'
```

Alias names are case-sensitive. Alias values use LiteShell's normal quoting and
environment-variable expansion rules when invoked. An alias may refer to its
own command name (for example, `alias ls='ls -a'`) without expanding forever.

The file is deliberately data-only. LiteShell reads assignments and alias
definitions but does not execute startup commands, profiles, completion scripts,
or plugins from it.

## Native command completion

Command-name completion automatically includes aliases, builtins, Windows
translations, current-directory executables, and executables found on `PATH`.
The `PATH` catalog is cached at startup; current-directory executables are read
when completion opens.

`just` dynamic completion is enabled by default. Other commands using clap's
environment-activated dynamic completion can be registered in `.liteshellrc`:

```text
complete my-cli clap-env COMPLETE
```

The first value is the command name and the last is the environment variable
used by that command's `clap_complete::CompleteEnv` setup. For example, the
built-in `just` registration is equivalent to:

```text
complete just clap-env JUST_COMPLETE
```

LiteShell invokes a registered binary directly with the `powershell` clap
completion protocol, parses its value-and-description output, and cancels the
request when the input changes. Calls run in the background with a 500 ms
timeout. Generated PowerShell completion scripts are never evaluated.

`LITESHELL_DEEP_SEARCH_EXCLUDE_DIRS` controls directory basenames pruned from
recursive `cd *query` searches. Values are separated by semicolons and matching
is case-insensitive on Windows. The default is:

```text
LITESHELL_DEEP_SEARCH_EXCLUDE_DIRS=.git;node_modules;__pycache__
```

Setting it to an empty value disables pruning. This setting affects recursive
filesystem traversal (`cd *query`, `find`, and `rg`); direct completion,
explicit paths, and the visited-directory database remain available for
excluded directories.
