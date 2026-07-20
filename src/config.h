#pragma once

#include <cstddef>
#include <string_view>

// LiteShell's deliberately static configuration surface. Editing this file and
// rebuilding cannot execute startup scripts, scan plugins, or add prompt latency.
namespace liteshell::config {

enum class PromptStyle {
  full_path,  // D:\work\project>
  leaf,       // project>
  compact,    // D:project>
};

inline constexpr PromptStyle prompt_style = PromptStyle::full_path;

// Available placeholders: {cwd}, {leaf}, {drive}, {user}.
inline constexpr std::wstring_view prompt_format = L"{cwd}> ";

inline constexpr std::size_t history_size = 5000;
inline constexpr std::wstring_view history_relative_path = L"LiteShell\\history";
inline constexpr std::size_t default_tail_lines = 10;
inline constexpr std::size_t binary_probe_bytes = 4096;
inline constexpr bool append_slash_to_directories = true;

}  // namespace liteshell::config
