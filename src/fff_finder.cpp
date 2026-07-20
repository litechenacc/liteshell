#define NOMINMAX
#include <windows.h>

#include "fff_finder.hpp"

#include "parser.hpp"

#include <algorithm>
#include <cstdint>
#include <stdexcept>
#include <utility>

namespace liteshell {
namespace {

struct FffResult {
  bool success;
  char* error;
  void* handle;
  std::int64_t intValue;
};

struct FffCreateOptions {
  std::uint32_t version;
  const char* basePath;
  const char* frecencyDatabasePath;
  const char* historyDatabasePath;
  bool enableMmapCache;
  bool enableContentIndexing;
  bool watch;
  bool aiMode;
  const char* logFilePath;
  const char* logLevel;
  std::uint64_t cacheBudgetMaxFiles;
  std::uint64_t cacheBudgetMaxBytes;
  std::uint64_t cacheBudgetMaxFileSize;
  bool enableFileSystemRootScanning;
  bool enableHomeDirectoryScanning;
  bool followSymlinks;
};

struct FffFileItem {
  char* relativePath;
  char* fileName;
  char* gitStatus;
  std::uint64_t size;
  std::uint64_t modified;
  std::int64_t accessFrecencyScore;
  std::int64_t modificationFrecencyScore;
  std::int64_t totalFrecencyScore;
  bool isBinary;
};

struct FffLocation {
  std::uint8_t tag;
  std::int32_t line;
  std::int32_t column;
  std::int32_t endLine;
  std::int32_t endColumn;
};

struct FffSearchResult {
  FffFileItem* items;
  void* scores;
  std::uint32_t count;
  std::uint32_t totalMatched;
  std::uint32_t totalFiles;
  FffLocation location;
};

struct FffDirectoryItem {
  char* relativePath;
  char* directoryName;
  std::int32_t maximumAccessFrecency;
};

struct FffDirectorySearchResult {
  FffDirectoryItem* items;
  void* scores;
  std::uint32_t count;
  std::uint32_t totalMatched;
  std::uint32_t totalDirectories;
};

struct FffMixedItem {
  std::uint8_t itemType;
  char* relativePath;
  char* displayName;
  char* gitStatus;
  std::uint64_t size;
  std::uint64_t modified;
  std::int64_t accessFrecencyScore;
  std::int64_t modificationFrecencyScore;
  std::int64_t totalFrecencyScore;
  bool isBinary;
};

struct FffMixedSearchResult {
  FffMixedItem* items;
  void* scores;
  std::uint32_t count;
  std::uint32_t totalMatched;
  std::uint32_t totalFiles;
  std::uint32_t totalDirectories;
  FffLocation location;
};

struct FffGrepMatch {
  char* relativePath;
  char* fileName;
  char* gitStatus;
  char* lineContent;
  void* matchRanges;
  char** contextBefore;
  char** contextAfter;
  std::uint64_t size;
  std::uint64_t modified;
  std::int64_t totalFrecencyScore;
  std::int64_t accessFrecencyScore;
  std::int64_t modificationFrecencyScore;
  std::uint64_t lineNumber;
  std::uint64_t byteOffset;
  std::uint32_t column;
  std::uint32_t matchRangesCount;
  std::uint32_t contextBeforeCount;
  std::uint32_t contextAfterCount;
  std::uint16_t fuzzyScore;
  bool hasFuzzyScore;
  bool isBinary;
  bool isDefinition;
};

struct FffGrepResult {
  FffGrepMatch* items;
  std::uint32_t count;
  std::uint32_t totalMatched;
  std::uint32_t totalFilesSearched;
  std::uint32_t totalFiles;
  std::uint32_t filteredFileCount;
  std::uint32_t nextFileOffset;
  char* regexFallbackError;
};

std::wstring moduleDirectory() {
  std::wstring path(32768, L'\0');
  const DWORD length = GetModuleFileNameW(nullptr, path.data(),
                                          static_cast<DWORD>(path.size()));
  if (length == 0 || length >= path.size()) return {};
  path.resize(length);
  const std::size_t separator = path.find_last_of(L"\\/");
  return separator == std::wstring::npos ? std::wstring{} : path.substr(0, separator);
}

template <typename Function>
bool importFunction(HMODULE library, const char* name, Function& output) {
  output = reinterpret_cast<Function>(GetProcAddress(library, name));
  return output != nullptr;
}

std::wstring fromUtf8(const char* value) {
  return value ? utf8ToWide(value) : std::wstring{};
}

std::wstring normalizedRelativePath(const char* value) {
  std::wstring path = fromUtf8(value);
  std::replace(path.begin(), path.end(), L'/', L'\\');
  return path;
}

std::wstring sizeDetail(std::uint64_t size, bool directory) {
  if (directory) return L"directory";
  if (size < 1024) return std::to_wstring(size) + L" B";
  if (size < 1024 * 1024) return std::to_wstring(size / 1024) + L" KiB";
  return std::to_wstring(size / (1024 * 1024)) + L" MiB";
}

}  // namespace

struct FffFinder::Api {
  using Create = FffResult*(__cdecl*)(const FffCreateOptions*);
  using Destroy = void(__cdecl*)(void*);
  using Search = FffResult*(__cdecl*)(void*, const char*, const char*, std::uint32_t,
                                      std::uint32_t, std::uint32_t, std::int32_t,
                                      std::uint32_t);
  using SearchDirectories = FffResult*(__cdecl*)(void*, const char*, const char*,
                                                 std::uint32_t, std::uint32_t,
                                                 std::uint32_t);
  using SearchMixed = Search;
  using Grep = FffResult*(__cdecl*)(void*, const char*, std::uint8_t, std::uint64_t,
                                    std::uint32_t, bool, std::uint32_t, std::uint32_t,
                                    std::uint64_t, std::uint32_t, std::uint32_t, bool);
  using WaitForScan = FffResult*(__cdecl*)(void*, std::uint64_t);
  using RestartIndex = FffResult*(__cdecl*)(void*, const char*);
  using FreeResult = void(__cdecl*)(FffResult*);
  using FreeSearchResult = void(__cdecl*)(FffSearchResult*);
  using FreeDirectoryResult = void(__cdecl*)(FffDirectorySearchResult*);
  using FreeMixedResult = void(__cdecl*)(FffMixedSearchResult*);
  using FreeGrepResult = void(__cdecl*)(FffGrepResult*);

  Create create = nullptr;
  Destroy destroy = nullptr;
  Search search = nullptr;
  SearchDirectories searchDirectories = nullptr;
  SearchMixed searchMixed = nullptr;
  Grep grep = nullptr;
  WaitForScan waitForScan = nullptr;
  RestartIndex restartIndex = nullptr;
  FreeResult freeResult = nullptr;
  FreeSearchResult freeSearchResult = nullptr;
  FreeDirectoryResult freeDirectoryResult = nullptr;
  FreeMixedResult freeMixedResult = nullptr;
  FreeGrepResult freeGrepResult = nullptr;
};

FffFinder::FffFinder() = default;

FffFinder::~FffFinder() { unload(); }

void FffFinder::load() {
  const std::wstring directory = moduleDirectory();
  const std::wstring path = directory.empty() ? L"fff_c.dll" : directory + L"\\fff_c.dll";
  HMODULE library = LoadLibraryW(path.c_str());
  if (!library) throw std::runtime_error("cannot load fff_c.dll");
  library_ = library;
  api_ = std::make_unique<Api>();
  const bool complete =
      importFunction(library, "fff_create_instance_with", api_->create) &&
      importFunction(library, "fff_destroy", api_->destroy) &&
      importFunction(library, "fff_search", api_->search) &&
      importFunction(library, "fff_search_directories", api_->searchDirectories) &&
      importFunction(library, "fff_search_mixed", api_->searchMixed) &&
      importFunction(library, "fff_live_grep", api_->grep) &&
      importFunction(library, "fff_wait_for_scan", api_->waitForScan) &&
      importFunction(library, "fff_restart_index", api_->restartIndex) &&
      importFunction(library, "fff_free_result", api_->freeResult) &&
      importFunction(library, "fff_free_search_result", api_->freeSearchResult) &&
      importFunction(library, "fff_free_dir_search_result", api_->freeDirectoryResult) &&
      importFunction(library, "fff_free_mixed_search_result", api_->freeMixedResult) &&
      importFunction(library, "fff_free_grep_result", api_->freeGrepResult);
  if (!complete) {
    unload();
    throw std::runtime_error("fff_c.dll does not expose the expected v0.10.0 C ABI");
  }
}

void FffFinder::unload() noexcept {
  const bool hadInstance = finder_ != nullptr;
  if (finder_ && api_ && api_->destroy) api_->destroy(finder_);
  finder_ = nullptr;
  api_.reset();
  // Keep the Rust DLL mapped until normal process teardown. The instance joins
  // its own workers, but FreeLibrary can still race Rust runtime/TLS cleanup in
  // a short process that exits immediately after its first search.
  if (library_ && !hadInstance) FreeLibrary(static_cast<HMODULE>(library_));
  library_ = nullptr;
}

bool FffFinder::ensureRoot(const std::filesystem::path& requestedRoot,
                           std::wstring& error) {
  if (!api_) {
    try {
      load();
    } catch (const std::exception& exception) {
      error = utf8ToWide(exception.what());
      return false;
    }
  }
  std::error_code code;
  const std::filesystem::path absolute = std::filesystem::absolute(requestedRoot, code);
  if (code) {
    error = L"cannot resolve search root";
    return false;
  }
  const std::string rootUtf8 = wideToUtf8(absolute.wstring());

  FffResult* operation = nullptr;
  if (!finder_) {
    FffCreateOptions options{};
    options.version = 2;
    options.basePath = rootUtf8.c_str();
    options.enableMmapCache = true;
    options.enableContentIndexing = true;
    // LiteShell explicitly restarts the index after cd. Avoid a background
    // watcher in this first embedding so DLL teardown is deterministic.
    options.watch = false;
    options.enableHomeDirectoryScanning = true;
    operation = api_->create(&options);
  } else if (_wcsicmp(root_.c_str(), absolute.c_str()) != 0) {
    operation = api_->restartIndex(finder_, rootUtf8.c_str());
  } else {
    return true;
  }

  if (!operation) {
    error = L"fff returned no result";
    return false;
  }
  if (!operation->success) {
    error = fromUtf8(operation->error);
    api_->freeResult(operation);
    return false;
  }
  if (!finder_) finder_ = operation->handle;
  api_->freeResult(operation);
  root_ = absolute;

  // Keep first interaction responsive. A timed-out wait still leaves a valid
  // live index and searches return the portion scanned so far.
  FffResult* waited = api_->waitForScan(finder_, 250);
  if (waited) api_->freeResult(waited);
  return true;
}

FffSearchResponse FffFinder::search(FffSearchKind kind, std::wstring_view query,
                                    const std::filesystem::path& root,
                                    std::size_t limit) {
  FffSearchResponse response;
  if (!ensureRoot(root, response.error)) return response;
  const std::string queryUtf8 = wideToUtf8(query);
  const std::uint32_t pageSize = static_cast<std::uint32_t>(
      std::clamp<std::size_t>(limit, 1, 100));

  FffResult* envelope = nullptr;
  if (kind == FffSearchKind::directories) {
    envelope = api_->searchDirectories(finder_, queryUtf8.c_str(), nullptr, 0, 0, pageSize);
  } else if (kind == FffSearchKind::mixed) {
    envelope = api_->searchMixed(finder_, queryUtf8.c_str(), nullptr, 0, 0, pageSize, 0, 0);
  } else if (kind == FffSearchKind::files) {
    envelope = api_->search(finder_, queryUtf8.c_str(), nullptr, 0, 0, pageSize, 0, 0);
  } else {
    envelope = api_->grep(finder_, queryUtf8.c_str(), 0, 10ULL * 1024 * 1024, 20,
                          true, 0, pageSize, 100, 0, 0, true);
  }

  if (!envelope) {
    response.error = L"fff returned no result";
    return response;
  }
  if (!envelope->success || !envelope->handle) {
    response.error = fromUtf8(envelope->error);
    api_->freeResult(envelope);
    return response;
  }

  if (kind == FffSearchKind::directories) {
    auto* result = static_cast<FffDirectorySearchResult*>(envelope->handle);
    response.total = result->totalMatched;
    for (std::uint32_t index = 0; index < result->count; ++index) {
      std::wstring path = normalizedRelativePath(result->items[index].relativePath);
      response.items.push_back({path, L"directory", path + L"\\", true});
    }
    api_->freeDirectoryResult(result);
  } else if (kind == FffSearchKind::mixed) {
    auto* result = static_cast<FffMixedSearchResult*>(envelope->handle);
    response.total = result->totalMatched;
    for (std::uint32_t index = 0; index < result->count; ++index) {
      const FffMixedItem& item = result->items[index];
      const bool directory = item.itemType == 1;
      std::wstring path = normalizedRelativePath(item.relativePath);
      response.items.push_back(
          {path, sizeDetail(item.size, directory), path + (directory ? L"\\" : L""), directory});
    }
    api_->freeMixedResult(result);
  } else if (kind == FffSearchKind::files) {
    auto* result = static_cast<FffSearchResult*>(envelope->handle);
    response.total = result->totalMatched;
    for (std::uint32_t index = 0; index < result->count; ++index) {
      const FffFileItem& item = result->items[index];
      std::wstring path = normalizedRelativePath(item.relativePath);
      response.items.push_back({path, sizeDetail(item.size, false), path, false});
    }
    api_->freeSearchResult(result);
  } else {
    auto* result = static_cast<FffGrepResult*>(envelope->handle);
    response.total = result->totalMatched;
    for (std::uint32_t index = 0; index < result->count; ++index) {
      const FffGrepMatch& match = result->items[index];
      const std::wstring path = normalizedRelativePath(match.relativePath);
      const std::wstring location = path + L":" + std::to_wstring(match.lineNumber) + L":" +
                                    std::to_wstring(match.column + 1);
      response.items.push_back({location, fromUtf8(match.lineContent), path, false});
    }
    api_->freeGrepResult(result);
  }
  api_->freeResult(envelope);
  return response;
}

}  // namespace liteshell
