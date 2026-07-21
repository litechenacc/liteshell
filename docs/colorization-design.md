# Colorization system design

## Status

Implemented. This document records the architecture and delivery plan for
semantic color output in `ls`, `cat`, `tail`, `less`, and `help`.

## Goals

- Add useful, consistent color to interactive builtin output.
- Keep redirected output byte-oriented plain text with no ANSI escape sequences.
- Keep Ratatui and terminal color types out of `liteshell-core` and builtins.
- Share source-text highlighting between `cat`, `tail`, and `less`.
- Preserve styles through bounded scrollback and the pager.
- Make color choices centralized, testable, and suitable for later theming.

## Non-goals

- Parsing every programming language into a full syntax tree.
- Interpreting ANSI sequences embedded in files.
- Emitting ANSI color in redirected mode.
- User-defined themes in this iteration. Semantic roles deliberately make that a
  compatible future addition.

## System modules

### 1. Core semantic output model (`liteshell-core::output`)

The core owns renderer-independent values:

- `SemanticColor`: named roles such as directory, executable, heading, command,
  option, string, number, keyword, comment, and punctuation.
- `TextStyle`: foreground role plus bold/dim/italic modifiers.
- `StyledSpan`: text and its semantic style.
- `StyledLine`: spans for one pager row.
- `StyledText`: an ordered styled stream which may contain newlines.
- `OutputEvent::Styled` and styled `OutputEvent::Pager` content.

Styles describe meaning rather than RGB values. No Ratatui or Crossterm type
crosses the core boundary.

### 2. Builtin color policy (`liteshell-builtins::color`)

This module owns command-facing classification and highlighting:

- filesystem classification for `ls` (directory, symlink, executable, regular
  file, metadata columns);
- extension-aware lightweight source highlighting;
- reusable text-to-styled-lines conversion for the pager;
- semantic constructors used by `help`.

The highlighter is deterministic and streaming by line. It recognizes comments,
quoted strings, numbers, punctuation, common language keywords, Markdown
headings, and diff additions/removals. Unknown text remains the terminal default
rather than receiving misleading color.

### 3. Command integration (`liteshell-builtins`)

- `ls`: metadata remains subdued; directories, links, executable files, and
  ordinary names use distinct semantic roles.
- `cat`: each file is highlighted using its extension.
- `tail`: selected lines are highlighted with the same path/language context as
  `cat`.
- `less`: pager rows carry styled spans instead of plain strings.
- `help`: title, command names, options, metavariables, and descriptions are
  emitted as structured spans.

Commands never decide whether a terminal supports color. They always emit
semantic output; the selected sink decides how to present it.

### 4. Scrollback style storage (`liteshell-tui::scrollback`)

Scrollback splits styled streams at line boundaries while retaining spans. Its
line/byte bounds continue to count textual payload only. Errors and dividers
remain semantic line attributes. Plain `push_text` remains available for command
echoes, search results, and diagnostics.

### 5. Ratatui palette adapter (`liteshell-tui::ui`)

A single mapping converts each `SemanticColor` and modifier into a Ratatui
`Style`. Both transcript and pager use this adapter. This prevents commands from
hard-coding terminal colors and provides one future theme insertion point.

The initial dark-terminal palette uses blue for directories, green for
executables, cyan for links/commands, yellow for options/strings, magenta for
keywords/headings, and dark gray for comments/metadata.

### 6. Plain output adapter (`liteshell::PlainOutput`)

The non-interactive sink flattens styled spans to their text. It never serializes
style as ANSI. Pager lines are flattened in the same way. This preserves the
existing automation contract.

## Data flow

```text
file/command semantics
  -> StyledText / StyledLine (core roles)
  -> OutputSink
     -> PlainOutput: concatenate text only
     `-> EventBuffer -> TuiState -> styled Scrollback/Pager -> Ratatui palette
```

## Behavior and compatibility

- Existing `Text` and `Error` events remain supported.
- Binary and invalid UTF-8 checks happen before highlighting and retain their
  current errors.
- Highlighting does not inspect or execute file contents beyond lexical scanning.
- ANSI control bytes in a file are rejected by the existing binary/control-byte
  check and are never interpreted as trusted terminal sequences.
- `tail` colors only the selected suffix and retains source extension context.
- A final line without a newline remains representable in `StyledText`; plain
  output concatenates spans exactly.

## Test plan

- Core: styled text flattening and styled-line flattening.
- Builtins: source tokens receive expected semantic roles; `ls` classifies
  directory/executable names; help has command and heading roles; highlighted
  text flattens to the original bytes.
- TUI: styled scrollback splitting preserves spans; palette rendering produces
  expected Ratatui foreground colors; styled pager renders without flattening.
- Application: redirected command output contains expected text and no ESC byte.
- Manual Windows Terminal matrix: run all five commands on narrow/wide windows,
  light/dark profiles, Unicode filenames, extensionless text, source files, and
  large pager input.

## Delivery sequence

1. Add semantic output types and flattening helpers to core.
2. Add builtin classification/highlighting and integrate all five commands.
3. Retain styles in TUI scrollback and pager state.
4. Add the centralized Ratatui palette and render styled spans.
5. Flatten semantic events in non-interactive mode.
6. Add unit tests and run the workspace test suite plus redirected-output checks.

## Future extensions

- Configurable palettes and `NO_COLOR` policy for an optional ANSI-capable sink.
- Search-match spans for `find`/`rg` and completion fuzzy matches.
- Incremental highlighting for very large files.
- Richer file attributes (hidden/read-only/archive) and user-configured extension
  classes.
