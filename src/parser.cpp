#define NOMINMAX
#include <windows.h>

#include "parser.hpp"

#include <algorithm>
#include <cwctype>

namespace fs = std::filesystem;

namespace liteshell {
namespace {

enum class QuoteMode { none, single, double_quote };

bool isVariableStart(wchar_t ch) {
  return ch == L'_' || std::iswalpha(ch) != 0;
}

bool isVariableCharacter(wchar_t ch) {
  return ch == L'_' || std::iswalnum(ch) != 0;
}

}  // namespace

std::wstring lowerCase(std::wstring value) {
  std::transform(value.begin(), value.end(), value.begin(), [](wchar_t ch) {
    return static_cast<wchar_t>(std::towlower(ch));
  });
  return value;
}

bool startsWithInsensitive(std::wstring_view value, std::wstring_view prefix) {
  if (prefix.size() > value.size()) {
    return false;
  }
  for (std::size_t index = 0; index < prefix.size(); ++index) {
    if (std::towlower(value[index]) != std::towlower(prefix[index])) {
      return false;
    }
  }
  return true;
}

std::optional<std::wstring> environmentVariable(std::wstring_view name) {
  const std::wstring terminated(name);
  const DWORD required = GetEnvironmentVariableW(terminated.c_str(), nullptr, 0);
  if (required == 0) {
    if (_wcsicmp(terminated.c_str(), L"HOME") == 0) {
      return environmentVariable(L"USERPROFILE");
    }
    return std::nullopt;
  }
  std::wstring value(required, L'\0');
  const DWORD written = GetEnvironmentVariableW(terminated.c_str(), value.data(), required);
  if (written == 0 || written >= required) {
    return std::nullopt;
  }
  value.resize(written);
  return value;
}

fs::path homeDirectory() {
  if (const auto profile = environmentVariable(L"USERPROFILE")) {
    return *profile;
  }
  const auto drive = environmentVariable(L"HOMEDRIVE");
  const auto path = environmentVariable(L"HOMEPATH");
  if (drive && path) {
    return *drive + *path;
  }
  std::error_code error;
  return fs::current_path(error);
}

fs::path expandHome(std::wstring_view rawPath) {
  if (rawPath == L"~") {
    return homeDirectory();
  }
  if (rawPath.size() > 1 && rawPath.front() == L'~' &&
      (rawPath[1] == L'\\' || rawPath[1] == L'/')) {
    return homeDirectory() / rawPath.substr(2);
  }
  return fs::path(rawPath);
}

ParsedLine parseCommandLine(std::wstring_view line) {
  ParsedLine result;
  std::wstring current;
  QuoteMode quote = QuoteMode::none;
  bool tokenStarted = false;

  const auto appendEnvironment = [&](std::wstring_view name) {
    if (const auto value = environmentVariable(name)) {
      current += *value;
    }
  };

  for (std::size_t index = 0; index < line.size(); ++index) {
    const wchar_t ch = line[index];
    if (quote == QuoteMode::none && std::iswspace(ch)) {
      if (tokenStarted) {
        result.arguments.push_back(std::move(current));
        current.clear();
        tokenStarted = false;
      }
      continue;
    }

    if (quote == QuoteMode::none &&
        (ch == L'|' || ch == L'>' || ch == L'<' || ch == L';' || ch == L'&')) {
      result.error = L"unsupported shell operator";
      return result;
    }

    if (ch == L'\'' && quote != QuoteMode::double_quote) {
      quote = quote == QuoteMode::single ? QuoteMode::none : QuoteMode::single;
      tokenStarted = true;
      continue;
    }
    if (ch == L'"' && quote != QuoteMode::single) {
      quote = quote == QuoteMode::double_quote ? QuoteMode::none : QuoteMode::double_quote;
      tokenStarted = true;
      continue;
    }

    if (ch == L'\\' && index + 1 < line.size()) {
      const wchar_t next = line[index + 1];
      const bool escapesQuote =
          (quote == QuoteMode::double_quote && next == L'"') ||
          (quote == QuoteMode::none && (next == L'"' || next == L'\''));
      if (escapesQuote) {
        current.push_back(next);
        ++index;
        tokenStarted = true;
        continue;
      }
    }

    if (quote != QuoteMode::single && ch == L'$' && index + 1 < line.size()) {
      std::size_t begin = index + 1;
      std::size_t end = begin;
      if (line[begin] == L'{') {
        begin++;
        end = line.find(L'}', begin);
        if (end == std::wstring_view::npos) {
          result.error = L"unclosed environment variable";
          return result;
        }
        appendEnvironment(line.substr(begin, end - begin));
        index = end;
        tokenStarted = true;
        continue;
      }
      if (isVariableStart(line[begin])) {
        end = begin + 1;
        while (end < line.size() && isVariableCharacter(line[end])) {
          ++end;
        }
        appendEnvironment(line.substr(begin, end - begin));
        index = end - 1;
        tokenStarted = true;
        continue;
      }
    }

    if (quote != QuoteMode::single && ch == L'%') {
      const std::size_t end = line.find(L'%', index + 1);
      if (end != std::wstring_view::npos && end > index + 1) {
        appendEnvironment(line.substr(index + 1, end - index - 1));
        index = end;
        tokenStarted = true;
        continue;
      }
    }

    current.push_back(ch);
    tokenStarted = true;
  }

  if (quote != QuoteMode::none) {
    result.error = L"unclosed quote";
    return result;
  }
  if (tokenStarted) {
    result.arguments.push_back(std::move(current));
  }
  for (std::wstring& argument : result.arguments) {
    argument = expandHome(argument).wstring();
  }
  return result;
}

std::wstring utf8ToWide(std::string_view value, bool* valid) {
  if (valid) {
    *valid = true;
  }
  if (value.empty()) {
    return {};
  }
  const int required = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, value.data(),
                                            static_cast<int>(value.size()), nullptr, 0);
  if (required <= 0) {
    if (valid) {
      *valid = false;
    }
    return {};
  }
  std::wstring output(static_cast<std::size_t>(required), L'\0');
  MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, value.data(),
                      static_cast<int>(value.size()), output.data(), required);
  return output;
}

std::string wideToUtf8(std::wstring_view value) {
  if (value.empty()) {
    return {};
  }
  const int required = WideCharToMultiByte(CP_UTF8, 0, value.data(),
                                            static_cast<int>(value.size()), nullptr, 0,
                                            nullptr, nullptr);
  std::string output(static_cast<std::size_t>(required), '\0');
  WideCharToMultiByte(CP_UTF8, 0, value.data(), static_cast<int>(value.size()),
                      output.data(), required, nullptr, nullptr);
  return output;
}

std::wstring windowsError(unsigned long error) {
  wchar_t* message = nullptr;
  const DWORD length = FormatMessageW(
      FORMAT_MESSAGE_ALLOCATE_BUFFER | FORMAT_MESSAGE_FROM_SYSTEM |
          FORMAT_MESSAGE_IGNORE_INSERTS,
      nullptr, error, 0, reinterpret_cast<wchar_t*>(&message), 0, nullptr);
  std::wstring result = length && message ? std::wstring(message, length) : L"Windows error";
  if (message) {
    LocalFree(message);
  }
  while (!result.empty() && (result.back() == L'\r' || result.back() == L'\n' ||
                             std::iswspace(result.back()))) {
    result.pop_back();
  }
  return result;
}

std::wstring quoteWindowsArgument(std::wstring_view argument) {
  if (argument.empty()) {
    return L"\"\"";
  }
  const bool requiresQuotes = argument.find_first_of(L" \t\n\v\"") != std::wstring_view::npos;
  if (!requiresQuotes) {
    return std::wstring(argument);
  }

  std::wstring output(1, L'"');
  std::size_t backslashes = 0;
  for (const wchar_t ch : argument) {
    if (ch == L'\\') {
      ++backslashes;
    } else if (ch == L'"') {
      output.append(backslashes * 2 + 1, L'\\');
      output.push_back(L'"');
      backslashes = 0;
    } else {
      output.append(backslashes, L'\\');
      backslashes = 0;
      output.push_back(ch);
    }
  }
  output.append(backslashes * 2, L'\\');
  output.push_back(L'"');
  return output;
}

}  // namespace liteshell
