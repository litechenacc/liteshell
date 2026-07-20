#include "history.hpp"

#include "config.h"
#include "parser.hpp"

#include <fstream>
#include <sstream>

namespace fs = std::filesystem;

namespace liteshell {

History::History() {
  if (const auto localAppData = environmentVariable(L"LOCALAPPDATA")) {
    path_ = fs::path(*localAppData) / config::history_relative_path;
  } else {
    path_ = homeDirectory() / L".liteshell_history";
  }
}

void History::load() {
  std::ifstream input(path_, std::ios::binary);
  if (!input) {
    return;
  }
  std::ostringstream bytes;
  bytes << input.rdbuf();
  bool valid = false;
  std::wstring content = utf8ToWide(bytes.str(), &valid);
  if (!valid) {
    return;
  }
  if (!content.empty() && content.front() == 0xFEFF) {
    content.erase(content.begin());
  }

  std::size_t begin = 0;
  while (begin <= content.size()) {
    const std::size_t end = content.find(L'\n', begin);
    std::wstring line = content.substr(begin, end == std::wstring::npos
                                                 ? std::wstring::npos
                                                 : end - begin);
    if (!line.empty() && line.back() == L'\r') {
      line.pop_back();
    }
    if (!line.empty() && (entries_.empty() || entries_.back() != line)) {
      entries_.push_back(std::move(line));
    }
    if (end == std::wstring::npos) {
      break;
    }
    begin = end + 1;
  }
  if (entries_.size() > config::history_size) {
    entries_.erase(entries_.begin(), entries_.end() -
                                          static_cast<std::ptrdiff_t>(config::history_size));
  }
}

void History::add(const std::wstring& line) {
  if (line.empty() || (!entries_.empty() && entries_.back() == line)) {
    return;
  }
  entries_.push_back(line);
  if (entries_.size() > config::history_size) {
    entries_.erase(entries_.begin(), entries_.begin() +
                                         static_cast<std::ptrdiff_t>(entries_.size() -
                                                                     config::history_size));
  }
}

void History::save() const {
  std::error_code error;
  fs::create_directories(path_.parent_path(), error);
  if (error) {
    return;
  }
  std::ofstream output(path_, std::ios::binary | std::ios::trunc);
  if (!output) {
    return;
  }
  for (const std::wstring& entry : entries_) {
    output << wideToUtf8(entry) << '\n';
  }
}

}  // namespace liteshell
