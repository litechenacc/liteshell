#pragma once

#include <cstddef>
#include <filesystem>
#include <memory>
#include <string>
#include <string_view>
#include <vector>

namespace liteshell {

enum class FffSearchKind { directories, mixed, files, grep };

struct FffCandidate {
  std::wstring label;
  std::wstring detail;
  std::wstring value;
  bool directory = false;
};

struct FffSearchResponse {
  std::vector<FffCandidate> items;
  std::size_t total = 0;
  std::wstring error;
};

class FffFinder {
 public:
  FffFinder();
  ~FffFinder();

  FffFinder(const FffFinder&) = delete;
  FffFinder& operator=(const FffFinder&) = delete;

  FffSearchResponse search(FffSearchKind kind, std::wstring_view query,
                           const std::filesystem::path& root,
                           std::size_t limit = 12);

 private:
  struct Api;

  void load();
  void unload() noexcept;
  bool ensureRoot(const std::filesystem::path& root, std::wstring& error);

  void* library_ = nullptr;
  std::unique_ptr<Api> api_;
  void* finder_ = nullptr;
  std::filesystem::path root_;
};

}  // namespace liteshell
