# Compile-time configuration

LiteShell uses [`src/config.h`](../src/config.h) as its zero-runtime-cost
configuration surface. Change the `inline constexpr` values and rebuild with
`.\build.ps1`.

Because this is a header rather than a startup script, configuration cannot launch
programs, scan plugins, or add work to every prompt render.

## Prompt

Choose how `{cwd}` is abbreviated:

```cpp
inline constexpr PromptStyle prompt_style = PromptStyle::full_path;
// Other choices: PromptStyle::leaf, PromptStyle::compact
```

Set the prompt template:

```cpp
inline constexpr std::wstring_view prompt_format = L"[{user}] {cwd} $ ";
```

Available placeholders:

- `{cwd}`: path formatted according to `prompt_style`;
- `{leaf}`: final directory name;
- `{drive}`: Windows drive/root name;
- `{user}`: `%USERNAME%`.

Prompt rendering performs no Git lookup or external command.

## Rendering

Prompt and candidate colors are owned by the OpenTUI adapter. Builtin and
redirected output is plain text; there is no legacy ANSI theme branch.

## History and command defaults

```cpp
inline constexpr std::size_t history_size = 5000;
inline constexpr std::wstring_view history_relative_path = L"LiteShell\\history";
inline constexpr std::size_t default_tail_lines = 10;
inline constexpr bool append_slash_to_directories = true;
```

`history_relative_path` is resolved beneath `%LOCALAPPDATA%`. If that environment
variable is unavailable, LiteShell uses `~/.liteshell_history`.
