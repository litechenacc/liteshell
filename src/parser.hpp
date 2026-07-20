#pragma once

#include <filesystem>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace liteshell {

struct ParsedLine {
  std::vector<std::wstring> arguments;
  std::optional<std::wstring> error;
};

ParsedLine parseCommandLine(std::wstring_view line);
std::wstring lowerCase(std::wstring value);
bool startsWithInsensitive(std::wstring_view value, std::wstring_view prefix);
std::optional<std::wstring> environmentVariable(std::wstring_view name);
std::filesystem::path homeDirectory();
std::filesystem::path expandHome(std::wstring_view rawPath);
std::wstring utf8ToWide(std::string_view value, bool* valid = nullptr);
std::string wideToUtf8(std::wstring_view value);
std::wstring windowsError(unsigned long error);
std::wstring quoteWindowsArgument(std::wstring_view argument);

}  // namespace liteshell
