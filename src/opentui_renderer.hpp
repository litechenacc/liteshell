#pragma once

#include <cstddef>
#include <cstdint>
#include <memory>
#include <string>
#include <string_view>
#include <vector>

namespace liteshell {

struct OpenTuiCandidate {
  std::wstring label;
  std::wstring detail;
};

// Thin, runtime-loaded wrapper around the native OpenTUI C ABI.  Keeping this
// boundary free of OpenTUI headers lets the rest of LiteShell remain a normal
// cl.exe project while keeping the pinned runtime ABI isolated in one adapter.
class OpenTuiRenderer {
 public:
  static std::unique_ptr<OpenTuiRenderer> create(std::uint32_t width,
                                                 std::uint32_t height);

  ~OpenTuiRenderer();

  OpenTuiRenderer(const OpenTuiRenderer&) = delete;
  OpenTuiRenderer& operator=(const OpenTuiRenderer&) = delete;

  bool resize(std::uint32_t width, std::uint32_t height);
  bool renderPrompt(std::wstring_view prompt, std::wstring_view line,
                    std::size_t cursor, std::uint32_t viewportTopRow,
                    const std::vector<OpenTuiCandidate>& candidates,
                    std::size_t selectedCandidate, std::size_t totalCandidates,
                    bool force);

 private:
  struct Api;

  OpenTuiRenderer() = default;
  bool load(std::uint32_t width, std::uint32_t height);
  void unload() noexcept;

  void* library_ = nullptr;
  std::unique_ptr<Api> api_;
  std::uint32_t renderer_ = 0;
  std::uint32_t width_ = 0;
  std::uint32_t height_ = 0;
};

}  // namespace liteshell
