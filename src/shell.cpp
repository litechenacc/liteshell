#define NOMINMAX
#include <windows.h>

#include "shell.hpp"

#include "config.h"
#include "parser.hpp"

#include <algorithm>
#include <array>
#include <chrono>
#include <cstdint>
#include <cwctype>
#include <fstream>
#include <iomanip>
#include <iterator>
#include <optional>
#include <set>
#include <sstream>
#include <string>
#include <system_error>
#include <unordered_set>
#include <utility>

namespace fs = std::filesystem;

namespace liteshell {
namespace {

constexpr std::array<std::wstring_view, 13> builtins{
    L"cd",   L"pwd",  L"ls",    L"cat",  L"tail", L"less",
    L"clear", L"which", L"find", L"rg", L"help", L"exit", L"quit",
};

struct ListedEntry {
  fs::path path;
  std::wstring name;
  bool directory = false;
  bool hidden = false;
  std::uintmax_t size = 0;
  fs::file_time_type modified{};
};

struct TextFile {
  std::wstring text;
  bool binary = false;
  std::wstring error;
};

std::wstring replaceAll(std::wstring value, std::wstring_view token,
                        std::wstring_view replacement) {
  std::size_t position = 0;
  while ((position = value.find(token, position)) != std::wstring::npos) {
    value.replace(position, token.size(), replacement);
    position += replacement.size();
  }
  return value;
}

bool hasExecutableExtension(const fs::path& path) {
  const std::wstring extension = lowerCase(path.extension().wstring());
  return extension == L".exe" || extension == L".com" || extension == L".cmd" ||
         extension == L".bat" || extension == L".ps1";
}

bool hiddenPath(const fs::path& path, std::wstring_view name) {
  if (!name.empty() && name.front() == L'.') {
    return true;
  }
  const DWORD attributes = GetFileAttributesW(path.c_str());
  return attributes != INVALID_FILE_ATTRIBUTES &&
         (attributes & FILE_ATTRIBUTE_HIDDEN) != 0;
}

std::wstring formattedTime(const fs::file_time_type& fileTime) {
  const auto systemTime = std::chrono::time_point_cast<std::chrono::system_clock::duration>(
      fileTime - fs::file_time_type::clock::now() + std::chrono::system_clock::now());
  const std::time_t value = std::chrono::system_clock::to_time_t(systemTime);
  std::tm local{};
  if (localtime_s(&local, &value) != 0) {
    return L"?";
  }
  wchar_t buffer[32]{};
  return std::wcsftime(buffer, std::size(buffer), L"%Y-%m-%d %H:%M", &local) != 0
             ? std::wstring(buffer)
             : L"?";
}

bool isProbablyBinary(std::string_view bytes) {
  const std::size_t length = std::min(bytes.size(), config::binary_probe_bytes);
  for (std::size_t index = 0; index < length; ++index) {
    const unsigned char value = static_cast<unsigned char>(bytes[index]);
    if (value == 0) {
      return true;
    }
    if (value < 8 || (value > 13 && value < 32)) {
      return true;
    }
  }
  return false;
}

TextFile readTextFile(const fs::path& path) {
  std::ifstream input(path, std::ios::binary);
  if (!input) {
    return {.error = L"cannot open file"};
  }
  std::ostringstream stream;
  stream << input.rdbuf();
  std::string bytes = stream.str();
  if (isProbablyBinary(bytes)) {
    return {.binary = true};
  }

  TextFile result;
  if (bytes.size() >= 3 && static_cast<unsigned char>(bytes[0]) == 0xEF &&
      static_cast<unsigned char>(bytes[1]) == 0xBB &&
      static_cast<unsigned char>(bytes[2]) == 0xBF) {
    bytes.erase(0, 3);
  }
  bool valid = false;
  result.text = utf8ToWide(bytes, &valid);
  if (!valid) {
    result.error = L"file is not valid UTF-8";
  }
  return result;
}

std::vector<std::wstring> splitLines(std::wstring_view text) {
  std::vector<std::wstring> lines;
  std::size_t begin = 0;
  while (begin <= text.size()) {
    const std::size_t end = text.find(L'\n', begin);
    std::wstring line(text.substr(begin, end == std::wstring_view::npos
                                            ? std::wstring_view::npos
                                            : end - begin));
    if (!line.empty() && line.back() == L'\r') {
      line.pop_back();
    }
    lines.push_back(std::move(line));
    if (end == std::wstring_view::npos) {
      break;
    }
    begin = end + 1;
  }
  return lines;
}

std::optional<fs::path> searchCommand(std::wstring_view command) {
  const std::wstring commandString(command);
  const fs::path inputPath(commandString);
  const bool explicitPath = command.find_first_of(L"\\/") != std::wstring_view::npos ||
                            (command.size() >= 2 && command[1] == L':');
  const std::array<std::wstring_view, 6> extensions{
      L"", L".exe", L".com", L".cmd", L".bat", L".ps1"};

  if (explicitPath) {
    for (const std::wstring_view extension : extensions) {
      fs::path candidate = inputPath;
      if (!extension.empty() && inputPath.has_extension()) {
        continue;
      }
      if (!extension.empty()) {
        candidate += extension;
      }
      std::error_code error;
      if (fs::is_regular_file(candidate, error) && !error) {
        return fs::absolute(candidate, error);
      }
    }
    return std::nullopt;
  }

  for (const std::wstring_view extension : extensions) {
    if (!extension.empty() && inputPath.has_extension()) {
      continue;
    }
    const std::wstring extensionString(extension);
    const wchar_t* extensionPointer = extension.empty() ? nullptr : extensionString.c_str();
    const DWORD required = SearchPathW(nullptr, commandString.c_str(), extensionPointer, 0,
                                       nullptr, nullptr);
    if (required == 0) {
      continue;
    }
    std::wstring resolved(static_cast<std::size_t>(required) + 1, L'\0');
    const DWORD written = SearchPathW(nullptr, commandString.c_str(), extensionPointer,
                                      static_cast<DWORD>(resolved.size()), resolved.data(), nullptr);
    if (written > 0 && written < resolved.size()) {
      resolved.resize(written);
      return fs::path(resolved);
    }
  }
  return std::nullopt;
}

ExternalKind kindForPath(const fs::path& path) {
  const std::wstring extension = lowerCase(path.extension().wstring());
  if (extension == L".cmd" || extension == L".bat") {
    return ExternalKind::command_script;
  }
  if (extension == L".ps1") {
    return ExternalKind::powershell_script;
  }
  return ExternalKind::executable;
}

std::wstring buildArgumentTail(const std::vector<std::wstring>& arguments,
                               std::size_t begin = 1) {
  std::wstring result;
  for (std::size_t index = begin; index < arguments.size(); ++index) {
    result.push_back(L' ');
    result += quoteWindowsArgument(arguments[index]);
  }
  return result;
}

BOOL WINAPI ignoreShellControl(DWORD controlType) {
  return controlType == CTRL_C_EVENT || controlType == CTRL_BREAK_EVENT;
}

std::wstring commonPrefixInsensitive(const std::vector<std::wstring>& values) {
  if (values.empty()) {
    return {};
  }
  std::size_t length = values.front().size();
  for (std::size_t item = 1; item < values.size(); ++item) {
    length = std::min(length, values[item].size());
    std::size_t index = 0;
    while (index < length &&
           std::towlower(values.front()[index]) == std::towlower(values[item][index])) {
      ++index;
    }
    length = index;
  }
  return values.front().substr(0, length);
}

std::wstring joinedArguments(const std::vector<std::wstring>& arguments,
                             std::size_t begin = 1) {
  std::wstring result;
  for (std::size_t index = begin; index < arguments.size(); ++index) {
    if (!result.empty()) result.push_back(L' ');
    result += arguments[index];
  }
  return result;
}

std::wstring completionValue(std::wstring value, bool directory,
                             bool appendSpace = true) {
  if (value.find_first_of(L" \t") != std::wstring::npos) {
    value = L"\"" + value + L"\"";
  }
  if (appendSpace && !directory) value.push_back(L' ');
  return value;
}

}  // namespace

void Shell::error(std::wstring_view command, std::wstring_view message) {
  terminal_.writeError(std::wstring(command) + L": " + std::wstring(message) + L"\n");
}

bool Shell::isBuiltin(std::wstring_view command) const {
  const std::wstring lowered = lowerCase(std::wstring(command));
  return std::find(builtins.begin(), builtins.end(), lowered) != builtins.end();
}

std::wstring Shell::prompt() const {
  std::error_code errorCode;
  const fs::path cwdPath = fs::current_path(errorCode);
  const std::wstring full = errorCode ? L"?" : cwdPath.wstring();
  std::wstring leaf = errorCode ? L"?" : cwdPath.filename().wstring();
  if (leaf.empty()) {
    leaf = full;
  }
  const std::wstring drive = errorCode ? L"" : cwdPath.root_name().wstring();

  std::wstring styledCwd;
  if constexpr (config::prompt_style == config::PromptStyle::full_path) {
    styledCwd = full;
  } else if constexpr (config::prompt_style == config::PromptStyle::leaf) {
    styledCwd = leaf;
  } else {
    styledCwd = drive + leaf;
  }

  std::wstring result(config::prompt_format);
  result = replaceAll(std::move(result), L"{cwd}", styledCwd);
  result = replaceAll(std::move(result), L"{leaf}", leaf);
  result = replaceAll(std::move(result), L"{drive}", drive);
  result = replaceAll(std::move(result), L"{user}",
                      environmentVariable(L"USERNAME").value_or(L"user"));
  return result;
}

int Shell::run() {
  history_.load();
  while (running_) {
    const auto line = terminal_.readLine(
        prompt(), history_.entries(),
        [this](std::wstring_view value, std::size_t cursor) {
          return complete(value, cursor);
        });
    if (!line) {
      if (!terminal_.interactive()) {
        terminal_.write(L"\n");
      }
      break;
    }
    const ParsedLine parsed = parseCommandLine(*line);
    if (parsed.error) {
      error(L"parse", *parsed.error);
      continue;
    }
    if (parsed.arguments.empty()) {
      continue;
    }
    history_.add(*line);
    dispatch(parsed.arguments);
  }
  history_.save();
  return 0;
}

int Shell::dispatch(const std::vector<std::wstring>& arguments) {
  const std::wstring command = lowerCase(arguments.front());
  if (command == L"exit" || command == L"quit") {
    if (arguments.size() != 1) {
      error(command, L"expected no arguments");
      return 2;
    }
    running_ = false;
    return 0;
  }
  if (command == L"cd") return builtinCd(arguments);
  if (command == L"pwd") return builtinPwd(arguments);
  if (command == L"ls") return builtinLs(arguments);
  if (command == L"cat") return builtinCat(arguments);
  if (command == L"tail") return builtinTail(arguments);
  if (command == L"less") return builtinLess(arguments);
  if (command == L"clear") return builtinClear(arguments);
  if (command == L"which") return builtinWhich(arguments);
  if (command == L"find") return builtinFind(arguments);
  if (command == L"rg") return builtinRg(arguments);
  if (command == L"help") return builtinHelp(arguments);

  const auto resolved = resolveExternal(arguments.front());
  if (!resolved) {
    error(arguments.front(), L"command not found");
    return 127;
  }
  return launchExternal(*resolved, arguments);
}

int Shell::builtinCd(const std::vector<std::wstring>& arguments) {
  if (arguments.size() > 2) {
    error(L"cd", L"expected zero or one path");
    return 2;
  }
  const fs::path target = arguments.size() == 1 ? homeDirectory() : fs::path(arguments[1]);
  std::error_code code;
  if (!fs::exists(target, code) || code) {
    error(L"cd", L"directory not found: " + target.wstring());
    return 1;
  }
  if (!fs::is_directory(target, code) || code) {
    error(L"cd", L"not a directory: " + target.wstring());
    return 1;
  }
  fs::current_path(target, code);
  if (code) {
    error(L"cd", L"cannot open directory: " + target.wstring());
    return 1;
  }
  return 0;
}

int Shell::builtinPwd(const std::vector<std::wstring>& arguments) {
  if (arguments.size() != 1) {
    error(L"pwd", L"expected no arguments");
    return 2;
  }
  std::error_code code;
  const fs::path cwd = fs::current_path(code);
  if (code) {
    error(L"pwd", L"cannot read current directory");
    return 1;
  }
  terminal_.write(cwd.wstring() + L"\n");
  return 0;
}

int Shell::builtinLs(const std::vector<std::wstring>& arguments) {
  bool showAll = false;
  bool longFormat = false;
  bool optionsEnded = false;
  std::optional<fs::path> requested;
  for (std::size_t index = 1; index < arguments.size(); ++index) {
    const std::wstring& argument = arguments[index];
    if (!optionsEnded && argument == L"--") {
      optionsEnded = true;
      continue;
    }
    if (!optionsEnded && argument.size() > 1 && argument.front() == L'-') {
      for (std::size_t option = 1; option < argument.size(); ++option) {
        if (argument[option] == L'a') showAll = true;
        else if (argument[option] == L'l') longFormat = true;
        else {
          error(L"ls", L"unknown option: -" + std::wstring(1, argument[option]));
          return 2;
        }
      }
    } else if (requested) {
      error(L"ls", L"expected at most one path");
      return 2;
    } else {
      requested = argument;
    }
  }

  std::error_code code;
  const fs::path target = requested.value_or(fs::current_path(code));
  if (code || !fs::exists(target, code)) {
    error(L"ls", L"path not found: " + target.wstring());
    return 1;
  }

  const auto readEntry = [](const fs::directory_entry& source,
                            std::error_code& entryCode) -> std::optional<ListedEntry> {
    ListedEntry entry;
    entry.path = source.path();
    entry.name = entry.path.filename().wstring();
    if (entry.name.empty()) entry.name = entry.path.wstring();
    entry.directory = source.is_directory(entryCode);
    if (entryCode) return std::nullopt;
    entry.hidden = hiddenPath(entry.path, entry.name);
    if (!entry.directory) {
      entry.size = source.file_size(entryCode);
      if (entryCode) return std::nullopt;
    }
    entry.modified = source.last_write_time(entryCode);
    if (entryCode) return std::nullopt;
    return entry;
  };

  std::vector<ListedEntry> entries;
  if (fs::is_directory(target, code) && !code) {
    fs::directory_iterator iterator(target, fs::directory_options::skip_permission_denied, code);
    const fs::directory_iterator end;
    while (!code && iterator != end) {
      std::error_code entryCode;
      auto entry = readEntry(*iterator, entryCode);
      if (entry && (showAll || !entry->hidden)) entries.push_back(std::move(*entry));
      iterator.increment(code);
    }
  } else if (!code) {
    fs::directory_entry source(target, code);
    if (!code) {
      auto entry = readEntry(source, code);
      if (entry && (showAll || !entry->hidden)) entries.push_back(std::move(*entry));
    }
  }
  if (code) {
    error(L"ls", L"cannot read: " + target.wstring());
    return 1;
  }

  std::sort(entries.begin(), entries.end(), [](const ListedEntry& left,
                                                const ListedEntry& right) {
    if (left.directory != right.directory) return left.directory;
    return _wcsicmp(left.name.c_str(), right.name.c_str()) < 0;
  });
  for (const ListedEntry& entry : entries) {
    if (longFormat) {
      std::wostringstream prefix;
      prefix << formattedTime(entry.modified) << L"  "
             << (entry.directory ? L"dir " : L"file") << L"  " << std::setw(12)
             << (entry.directory ? L"-" : std::to_wstring(entry.size)) << L"  ";
      terminal_.write(prefix.str());
    }
    terminal_.write(entry.name);
    if (entry.directory && config::append_slash_to_directories) terminal_.write(L"\\");
    terminal_.write(L"\n");
  }
  return 0;
}

int Shell::builtinCat(const std::vector<std::wstring>& arguments) {
  if (arguments.size() < 2) {
    error(L"cat", L"expected at least one file");
    return 2;
  }
  int result = 0;
  for (std::size_t index = 1; index < arguments.size(); ++index) {
    const fs::path path(arguments[index]);
    std::error_code code;
    if (!fs::exists(path, code) || code) {
      error(L"cat", L"file not found: " + path.wstring());
      result = 1;
      continue;
    }
    if (!fs::is_regular_file(path, code) || code) {
      error(L"cat", L"not a regular file: " + path.wstring());
      result = 1;
      continue;
    }
    const TextFile file = readTextFile(path);
    if (file.binary) {
      error(L"cat", L"refusing to print binary file: " + path.wstring());
      result = 1;
    } else if (!file.error.empty()) {
      error(L"cat", file.error + L": " + path.wstring());
      result = 1;
    } else {
      terminal_.write(file.text);
    }
  }
  return result;
}

int Shell::builtinTail(const std::vector<std::wstring>& arguments) {
  std::size_t lineCount = config::default_tail_lines;
  std::optional<fs::path> requested;
  for (std::size_t index = 1; index < arguments.size(); ++index) {
    if (arguments[index] == L"-n") {
      if (++index >= arguments.size()) {
        error(L"tail", L"-n requires a line count");
        return 2;
      }
      try {
        std::size_t used = 0;
        const unsigned long long parsed = std::stoull(arguments[index], &used);
        if (used != arguments[index].size()) throw std::invalid_argument("line count");
        lineCount = static_cast<std::size_t>(parsed);
      } catch (...) {
        error(L"tail", L"invalid line count: " + arguments[index]);
        return 2;
      }
    } else if (requested) {
      error(L"tail", L"expected one file");
      return 2;
    } else {
      requested = arguments[index];
    }
  }
  if (!requested) {
    error(L"tail", L"expected one file");
    return 2;
  }

  std::ifstream input(*requested, std::ios::binary);
  if (!input) {
    error(L"tail", L"file not found: " + requested->wstring());
    return 1;
  }
  input.seekg(0, std::ios::end);
  const std::streamoff end = input.tellg();
  if (end < 0) {
    error(L"tail", L"cannot read file: " + requested->wstring());
    return 1;
  }
  constexpr std::streamoff chunkSize = 64 * 1024;
  std::streamoff position = end;
  std::string bytes;
  std::size_t newlines = 0;
  while (position > 0 && newlines <= lineCount) {
    const std::streamoff amount = std::min(position, chunkSize);
    position -= amount;
    std::string chunk(static_cast<std::size_t>(amount), '\0');
    input.seekg(position);
    input.read(chunk.data(), amount);
    bytes.insert(0, chunk);
    newlines += static_cast<std::size_t>(std::count(chunk.begin(), chunk.end(), '\n'));
  }
  if (isProbablyBinary(bytes)) {
    error(L"tail", L"refusing to print binary file: " + requested->wstring());
    return 1;
  }
  std::size_t start = bytes.size();
  std::size_t found = 0;
  if (lineCount > 0) {
    if (start > 0 && bytes[start - 1] == '\n') --start;
    while (start > 0) {
      --start;
      if (bytes[start] == '\n' && ++found == lineCount) {
        ++start;
        break;
      }
    }
    if (found < lineCount) start = 0;
  }
  bool valid = false;
  const std::wstring output = utf8ToWide(std::string_view(bytes).substr(start), &valid);
  if (!valid) {
    error(L"tail", L"file is not valid UTF-8: " + requested->wstring());
    return 1;
  }
  terminal_.write(output);
  return 0;
}

int Shell::builtinLess(const std::vector<std::wstring>& arguments) {
  if (arguments.size() != 2) {
    error(L"less", L"expected one file");
    return 2;
  }
  const fs::path path(arguments[1]);
  const TextFile file = readTextFile(path);
  if (file.binary) {
    error(L"less", L"refusing to open binary file: " + path.wstring());
    return 1;
  }
  if (!file.error.empty()) {
    error(L"less", file.error + L": " + path.wstring());
    return 1;
  }
  if (!terminal_.interactive()) {
    terminal_.write(file.text);
    return 0;
  }

  const std::vector<std::wstring> lines = splitLines(file.text);
  std::size_t top = 0;
  terminal_.enterAlternateScreen();
  terminal_.beginRawInput(true);
  bool viewing = true;
  while (viewing) {
    const auto [width, height] = terminal_.size();
    const std::size_t page = static_cast<std::size_t>(std::max(1, height - 1));
    const std::size_t maximumTop = lines.size() > page ? lines.size() - page : 0;
    top = std::min(top, maximumTop);
    terminal_.clear();
    for (std::size_t row = 0; row < page && top + row < lines.size(); ++row) {
      const std::wstring_view line = lines[top + row];
      terminal_.write(line.substr(0, static_cast<std::size_t>(width)));
      terminal_.write(L"\r\n");
    }
    const std::size_t last = std::min(lines.size(), top + page);
    const int percent = lines.empty() ? 100 : static_cast<int>((last * 100) / lines.size());
    std::wstring status = L" " + path.filename().wstring() + L"  " +
                          std::to_wstring(last) + L"/" + std::to_wstring(lines.size()) +
                          L"  " + std::to_wstring(percent) + L"%  (q to quit) ";
    if (status.size() < static_cast<std::size_t>(width)) {
      status.append(static_cast<std::size_t>(width) - status.size(), L' ');
    } else {
      status.resize(static_cast<std::size_t>(width));
    }
    terminal_.write(status, true);

    const KeyPress key = terminal_.readKey();
    if (key.kind == KeyKind::escape ||
        (key.kind == KeyKind::character && (key.character == L'q' || key.character == L'Q'))) {
      viewing = false;
    } else if (key.kind == KeyKind::down || key.kind == KeyKind::scroll_down ||
               (key.kind == KeyKind::character && key.character == L'j')) {
      if (top < maximumTop) ++top;
    } else if (key.kind == KeyKind::up || key.kind == KeyKind::scroll_up ||
               (key.kind == KeyKind::character && key.character == L'k')) {
      if (top > 0) --top;
    } else if (key.kind == KeyKind::page_down ||
               (key.kind == KeyKind::character && key.character == L' ')) {
      top = std::min(maximumTop, top + page);
    } else if (key.kind == KeyKind::page_up) {
      top = top > page ? top - page : 0;
    } else if (key.kind == KeyKind::character && key.character == L'g') {
      top = 0;
    } else if (key.kind == KeyKind::character && key.character == L'G') {
      top = maximumTop;
    }
  }
  terminal_.endRawInput();
  terminal_.leaveAlternateScreen();
  return 0;
}

int Shell::builtinClear(const std::vector<std::wstring>& arguments) {
  if (arguments.size() != 1) {
    error(L"clear", L"expected no arguments");
    return 2;
  }
  terminal_.clear();
  return 0;
}

int Shell::builtinWhich(const std::vector<std::wstring>& arguments) {
  if (arguments.size() < 2) {
    error(L"which", L"expected at least one command");
    return 2;
  }
  int result = 0;
  for (std::size_t index = 1; index < arguments.size(); ++index) {
    if (isBuiltin(arguments[index])) {
      terminal_.write(arguments[index] + L": builtin\n");
    } else if (const auto resolved = resolveExternal(arguments[index])) {
      terminal_.write(arguments[index] + L": " + resolved->path.wstring() + L"\n");
    } else {
      error(L"which", L"command not found: " + arguments[index]);
      result = 1;
    }
  }
  return result;
}

int Shell::builtinFind(const std::vector<std::wstring>& arguments) {
  std::error_code code;
  const fs::path root = fs::current_path(code);
  if (code) {
    error(L"find", L"cannot read current directory");
    return 1;
  }
  const FffSearchResponse response =
      fff_.search(FffSearchKind::files, joinedArguments(arguments), root, 100);
  if (!response.error.empty()) {
    error(L"find", response.error);
    return 1;
  }
  for (const FffCandidate& item : response.items) {
    terminal_.write(item.label + L"\n");
  }
  return 0;
}

int Shell::builtinRg(const std::vector<std::wstring>& arguments) {
  if (arguments.size() < 2) {
    error(L"rg", L"expected a search query");
    return 2;
  }
  std::error_code code;
  const fs::path root = fs::current_path(code);
  if (code) {
    error(L"rg", L"cannot read current directory");
    return 1;
  }
  const FffSearchResponse response =
      fff_.search(FffSearchKind::grep, joinedArguments(arguments), root, 100);
  if (!response.error.empty()) {
    error(L"rg", response.error);
    return 1;
  }
  for (const FffCandidate& item : response.items) {
    terminal_.write(item.label + L": " + item.detail + L"\n");
  }
  return 0;
}

int Shell::builtinHelp(const std::vector<std::wstring>& arguments) {
  if (arguments.size() != 1) {
    error(L"help", L"expected no arguments");
    return 2;
  }
  terminal_.write(
      L"LiteShell commands:\n"
      L"  cd [path]              change directory (no path uses home)\n"
      L"  pwd                    print current directory\n"
      L"  ls [-a] [-l] [path]    list directory contents\n"
      L"  cat file...            print UTF-8 text files\n"
      L"  tail [-n count] file   print the last lines\n"
      L"  less file              open the built-in pager\n"
      L"  clear                  clear the terminal\n"
      L"  which command...       resolve builtins and external commands\n"
      L"  find [query]           fff fuzzy file search\n"
      L"  rg query               fff indexed content search\n"
      L"  exit                   leave LiteShell\n"
      L"External .exe/.com/.cmd/.bat/.ps1 commands run in the foreground.\n");
  return 0;
}

std::optional<ResolvedCommand> Shell::resolveExternal(std::wstring_view command) const {
  const auto path = searchCommand(command);
  if (!path) return std::nullopt;
  return ResolvedCommand{*path, kindForPath(*path)};
}

int Shell::launchExternal(const ResolvedCommand& command,
                          const std::vector<std::wstring>& arguments) {
  fs::path application = command.path;
  std::wstring commandLine;
  if (command.kind == ExternalKind::executable) {
    commandLine = quoteWindowsArgument(application.wstring()) + buildArgumentTail(arguments);
  } else if (command.kind == ExternalKind::command_script) {
    const auto commandInterpreter = searchCommand(L"cmd.exe");
    if (!commandInterpreter) {
      error(arguments.front(), L"cmd.exe not found");
      return 127;
    }
    application = *commandInterpreter;
    const std::wstring inner = quoteWindowsArgument(command.path.wstring()) +
                               buildArgumentTail(arguments);
    commandLine = quoteWindowsArgument(application.wstring()) + L" /d /s /c \"" + inner + L"\"";
  } else {
    auto powerShell = searchCommand(L"pwsh.exe");
    if (!powerShell) powerShell = searchCommand(L"powershell.exe");
    if (!powerShell) {
      error(arguments.front(), L"pwsh.exe not found");
      return 127;
    }
    application = *powerShell;
    commandLine = quoteWindowsArgument(application.wstring()) +
                  L" -NoLogo -NoProfile -File " +
                  quoteWindowsArgument(command.path.wstring()) + buildArgumentTail(arguments);
  }

  std::vector<wchar_t> mutableCommand(commandLine.begin(), commandLine.end());
  mutableCommand.push_back(L'\0');
  STARTUPINFOW startup{};
  startup.cb = sizeof(startup);
  PROCESS_INFORMATION process{};
  SetConsoleCtrlHandler(ignoreShellControl, TRUE);
  const BOOL created = CreateProcessW(application.c_str(), mutableCommand.data(), nullptr, nullptr,
                                      TRUE, 0, nullptr, nullptr, &startup, &process);
  if (!created) {
    const DWORD code = GetLastError();
    SetConsoleCtrlHandler(ignoreShellControl, FALSE);
    error(arguments.front(), L"cannot start: " + windowsError(code));
    return 126;
  }
  CloseHandle(process.hThread);
  WaitForSingleObject(process.hProcess, INFINITE);
  DWORD exitCode = 1;
  GetExitCodeProcess(process.hProcess, &exitCode);
  CloseHandle(process.hProcess);
  SetConsoleCtrlHandler(ignoreShellControl, FALSE);
  terminal_.resetAfterChild();
  return static_cast<int>(exitCode);
}

void Shell::loadPathCommands() {
  if (pathCommandsLoaded_) return;
  pathCommandsLoaded_ = true;
  std::unordered_set<std::wstring> seen;
  for (const std::wstring_view builtin : builtins) {
    seen.insert(lowerCase(std::wstring(builtin)));
    pathCommands_.emplace_back(builtin);
  }
  const auto pathValue = environmentVariable(L"PATH");
  if (!pathValue) return;
  std::size_t begin = 0;
  while (begin <= pathValue->size()) {
    const std::size_t end = pathValue->find(L';', begin);
    std::wstring directory = pathValue->substr(begin, end == std::wstring::npos
                                                          ? std::wstring::npos
                                                          : end - begin);
    if (directory.size() >= 2 && directory.front() == L'"' && directory.back() == L'"') {
      directory = directory.substr(1, directory.size() - 2);
    }
    std::error_code code;
    fs::directory_iterator iterator(directory, fs::directory_options::skip_permission_denied, code);
    const fs::directory_iterator iteratorEnd;
    while (!code && iterator != iteratorEnd) {
      if (iterator->is_regular_file(code) && !code && hasExecutableExtension(iterator->path())) {
        const std::wstring name = iterator->path().stem().wstring();
        const std::wstring lowered = lowerCase(name);
        if (seen.insert(lowered).second) pathCommands_.push_back(name);
      }
      code.clear();
      iterator.increment(code);
    }
    if (end == std::wstring::npos) break;
    begin = end + 1;
  }
  std::sort(pathCommands_.begin(), pathCommands_.end(), [](const std::wstring& left,
                                                            const std::wstring& right) {
    return _wcsicmp(left.c_str(), right.c_str()) < 0;
  });
}

CompletionResult Shell::complete(std::wstring_view line, std::size_t cursor) {
  CompletionResult result{std::wstring(line), cursor};
  cursor = std::min(cursor, line.size());
  std::size_t tokenStart = 0;
  wchar_t quote = 0;
  for (std::size_t index = 0; index < cursor; ++index) {
    const wchar_t ch = line[index];
    if ((ch == L'\'' || ch == L'"') && (quote == 0 || quote == ch)) {
      quote = quote == 0 ? ch : 0;
    } else if (quote == 0 && std::iswspace(ch)) {
      tokenStart = index + 1;
    }
  }
  std::wstring rawToken(line.substr(tokenStart, cursor - tokenStart));
  const wchar_t openingQuote = !rawToken.empty() &&
                                       (rawToken.front() == L'\'' || rawToken.front() == L'"')
                                   ? rawToken.front()
                                   : 0;
  std::wstring token = openingQuote ? rawToken.substr(1) : rawToken;
  result.replacementStart = tokenStart;
  result.replacementLength = cursor - tokenStart;

  std::size_t commandStart = 0;
  while (commandStart < cursor && std::iswspace(line[commandStart])) ++commandStart;
  std::size_t commandEnd = commandStart;
  while (commandEnd < cursor && !std::iswspace(line[commandEnd])) ++commandEnd;
  const std::wstring command = lowerCase(std::wstring(
      line.substr(commandStart, commandEnd - commandStart)));

  std::optional<FffSearchKind> fffKind;
  if (tokenStart > commandEnd) {
    if (command == L"cd") fffKind = FffSearchKind::directories;
    else if (command == L"ls") fffKind = FffSearchKind::mixed;
    else if (command == L"find") fffKind = FffSearchKind::files;
    else if (command == L"rg") fffKind = FffSearchKind::grep;
  }

  if (fffKind) {
    std::wstring query = token;
    if (command == L"find" || command == L"rg") {
      std::size_t queryStart = commandEnd;
      while (queryStart < cursor && std::iswspace(line[queryStart])) ++queryStart;
      query = std::wstring(line.substr(queryStart, cursor - queryStart));
      result.replacementStart = 0;
      result.replacementLength = line.size();
    }
    std::error_code code;
    const fs::path root = fs::current_path(code);
    if (!code) {
      const FffSearchResponse response = fff_.search(*fffKind, query, root, 24);
      result.totalAlternatives = response.total;
      for (const FffCandidate& item : response.items) {
        std::wstring replacement;
        if (command == L"find" || command == L"rg") {
          replacement = L"less " + completionValue(item.value, false, false);
        } else {
          replacement = completionValue(item.value, item.directory);
        }
        result.alternatives.push_back(
            {item.label, item.detail, std::move(replacement)});
      }
      if (result.alternatives.size() == 1) {
        const std::wstring replacement = result.alternatives.front().replacement;
        result.line.replace(result.replacementStart, result.replacementLength, replacement);
        result.cursor = result.replacementStart + replacement.size();
        result.replacementLength = replacement.size();
        result.changed = true;
      }
      if (!result.alternatives.empty() || command == L"find" || command == L"rg") {
        return result;
      }
    }
  }

  std::vector<std::wstring> candidates;
  bool pathCompletion = tokenStart != 0;
  if (!pathCompletion) {
    loadPathCommands();
    for (const std::wstring& pathCommand : pathCommands_) {
      if (startsWithInsensitive(pathCommand, token)) candidates.push_back(pathCommand);
    }
  } else {
    const std::size_t separator = token.find_last_of(L"\\/");
    const std::wstring directoryPart = separator == std::wstring::npos
                                           ? L""
                                           : token.substr(0, separator + 1);
    const std::wstring prefix = separator == std::wstring::npos
                                    ? token
                                    : token.substr(separator + 1);
    const fs::path directory = directoryPart.empty() ? fs::path(L".")
                                                      : expandHome(directoryPart);
    std::error_code code;
    fs::directory_iterator iterator(directory, fs::directory_options::skip_permission_denied, code);
    const fs::directory_iterator end;
    while (!code && iterator != end) {
      const std::wstring name = iterator->path().filename().wstring();
      if (startsWithInsensitive(name, prefix)) {
        std::wstring candidate = directoryPart + name;
        if (iterator->is_directory(code) && !code) candidate.push_back(L'\\');
        candidates.push_back(std::move(candidate));
      }
      code.clear();
      iterator.increment(code);
    }
    std::sort(candidates.begin(), candidates.end(), [](const std::wstring& left,
                                                        const std::wstring& right) {
      return _wcsicmp(left.c_str(), right.c_str()) < 0;
    });
  }

  if (candidates.empty()) return result;
  result.totalAlternatives = candidates.size();
  for (const std::wstring& candidate : candidates) {
    const bool candidateDirectory = pathCompletion && !candidate.empty() &&
                                    (candidate.back() == L'\\' || candidate.back() == L'/');
    result.alternatives.push_back(
        {candidate, candidateDirectory ? L"directory" : L"",
         completionValue(candidate, candidateDirectory)});
  }
  std::wstring replacement = candidates.size() == 1 ? candidates.front()
                                                      : commonPrefixInsensitive(candidates);
  if (replacement.size() <= token.size() && candidates.size() > 1) return result;

  const bool directory = pathCompletion && !replacement.empty() &&
                         (replacement.back() == L'\\' || replacement.back() == L'/');
  replacement = completionValue(std::move(replacement), directory,
                                candidates.size() == 1);
  result.line.replace(tokenStart, cursor - tokenStart, replacement);
  result.cursor = tokenStart + replacement.size();
  result.replacementLength = replacement.size();
  result.changed = true;
  return result;
}

}  // namespace liteshell
