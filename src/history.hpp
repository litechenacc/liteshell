#pragma once

#include <filesystem>
#include <string>
#include <vector>

namespace liteshell {

class History {
 public:
  History();

  void load();
  void add(const std::wstring& line);
  void save() const;
  const std::vector<std::wstring>& entries() const noexcept { return entries_; }
  const std::filesystem::path& path() const noexcept { return path_; }

 private:
  std::filesystem::path path_;
  std::vector<std::wstring> entries_;
};

}  // namespace liteshell
