#define NOMINMAX
#include <windows.h>

#include "opentui_renderer.hpp"

#include "parser.hpp"

#include <algorithm>
#include <array>
#include <limits>
#include <utility>

namespace liteshell {
namespace {

using NativeHandle = std::uint32_t;
using PackedColor = std::array<std::uint16_t, 4>;

constexpr PackedColor transparent{0, 0, 0, 0};
constexpr PackedColor promptColor{0x6A, 0xD4, 0x8A, 0xFF};
constexpr PackedColor inputColor{0xEA, 0xEA, 0xEA, 0xFF};
constexpr PackedColor headerColor{0x72, 0x9F, 0xCF, 0xFF};
constexpr PackedColor candidateColor{0xC8, 0xC8, 0xC8, 0xFF};
constexpr PackedColor selectedColor{0xFF, 0xD7, 0x5F, 0xFF};
constexpr PackedColor detailColor{0x80, 0x80, 0x80, 0xFF};

std::wstring moduleDirectory() {
  std::wstring path(32768, L'\0');
  const DWORD length = GetModuleFileNameW(nullptr, path.data(),
                                          static_cast<DWORD>(path.size()));
  if (length == 0 || length >= path.size()) return {};
  path.resize(length);
  const std::size_t separator = path.find_last_of(L"\\/");
  return separator == std::wstring::npos ? std::wstring{} : path.substr(0, separator);
}

std::size_t previousCharacter(std::wstring_view value, std::size_t cursor) {
  if (cursor == 0) return 0;
  --cursor;
  if (cursor > 0 && value[cursor] >= 0xDC00 && value[cursor] <= 0xDFFF &&
      value[cursor - 1] >= 0xD800 && value[cursor - 1] <= 0xDBFF) {
    --cursor;
  }
  return cursor;
}

std::size_t displayWidth(std::wstring_view value) {
  std::size_t width = 0;
  for (std::size_t index = 0; index < value.size(); ++index) {
    const wchar_t ch = value[index];
    if (ch >= 0xD800 && ch <= 0xDBFF && index + 1 < value.size() &&
        value[index + 1] >= 0xDC00 && value[index + 1] <= 0xDFFF) {
      width += 2;
      ++index;
      continue;
    }
    WORD type = 0;
    if (GetStringTypeW(CT_CTYPE3, &ch, 1, &type)) {
      if ((type & C3_NONSPACING) != 0) continue;
      if ((type & (C3_FULLWIDTH | C3_IDEOGRAPH | C3_HIRAGANA | C3_KATAKANA)) != 0) {
        width += 2;
        continue;
      }
    }
    ++width;
  }
  return width;
}

struct InputViewport {
  std::wstring text;
  std::uint32_t cursorColumn = 0;
  std::size_t promptCells = 0;
  std::size_t promptCodeUnits = 0;
};

InputViewport visibleInput(std::wstring_view prompt, std::wstring_view line,
                           std::size_t cursor, std::uint32_t width) {
  std::wstring combined(prompt);
  combined.append(line);
  const std::size_t promptCells = displayWidth(prompt);
  const std::size_t cursorCells = promptCells + displayWidth(line.substr(0, cursor));
  const std::size_t safeWidth = std::max<std::size_t>(width, 1);
  const std::size_t desiredStart = cursorCells >= safeWidth ? cursorCells - safeWidth + 1 : 0;

  std::size_t begin = 0;
  std::size_t skippedCells = 0;
  while (begin < combined.size() && skippedCells < desiredStart) {
    const std::size_t next = begin + (combined[begin] >= 0xD800 && combined[begin] <= 0xDBFF &&
                                              begin + 1 < combined.size() &&
                                              combined[begin + 1] >= 0xDC00 &&
                                              combined[begin + 1] <= 0xDFFF
                                          ? 2
                                          : 1);
    const std::size_t cells = displayWidth(std::wstring_view(combined).substr(begin, next - begin));
    if (skippedCells + cells > desiredStart) break;
    skippedCells += cells;
    begin = next;
  }

  std::wstring visible = combined.substr(begin);
  while (!visible.empty() && displayWidth(visible) > safeWidth) {
    visible.resize(previousCharacter(visible, visible.size()));
  }

  InputViewport result;
  result.text = std::move(visible);
  result.cursorColumn = static_cast<std::uint32_t>(
      std::min(cursorCells - skippedCells, safeWidth - 1));
  result.promptCodeUnits = begin < prompt.size()
                               ? std::min(prompt.size() - begin, result.text.size())
                               : 0;
  result.promptCells = displayWidth(
      std::wstring_view(result.text).substr(0, result.promptCodeUnits));
  return result;
}

template <typename Function>
bool importFunction(HMODULE library, const char* name, Function& output) {
  output = reinterpret_cast<Function>(GetProcAddress(library, name));
  return output != nullptr;
}

}  // namespace

struct OpenTuiRenderer::Api {
  using CreateRenderer = NativeHandle(__cdecl*)(std::uint32_t, std::uint32_t,
                                                 std::uint8_t, std::uint8_t, void*);
  using DestroyRenderer = void(__cdecl*)(NativeHandle);
  using SetClearOnShutdown = void(__cdecl*)(NativeHandle, bool);
  using SetRenderOffset = void(__cdecl*)(NativeHandle, std::uint32_t);
  using GetNextBuffer = NativeHandle(__cdecl*)(NativeHandle);
  using BufferClear = void(__cdecl*)(NativeHandle, const std::uint16_t*);
  using BufferDrawText = void(__cdecl*)(NativeHandle, const std::uint8_t*, std::uint32_t,
                                        std::uint32_t, std::uint32_t,
                                        const std::uint16_t*, const std::uint16_t*,
                                        std::uint32_t);
  using SetCursorPosition = void(__cdecl*)(NativeHandle, std::int32_t, std::int32_t, bool);
  using Render = std::uint8_t(__cdecl*)(NativeHandle, bool);
  using ResizeRenderer = void(__cdecl*)(NativeHandle, std::uint32_t, std::uint32_t);

  CreateRenderer createRenderer = nullptr;
  DestroyRenderer destroyRenderer = nullptr;
  SetClearOnShutdown setClearOnShutdown = nullptr;
  SetRenderOffset setRenderOffset = nullptr;
  GetNextBuffer getNextBuffer = nullptr;
  BufferClear bufferClear = nullptr;
  BufferDrawText bufferDrawText = nullptr;
  SetCursorPosition setCursorPosition = nullptr;
  Render render = nullptr;
  ResizeRenderer resizeRenderer = nullptr;
};

std::unique_ptr<OpenTuiRenderer> OpenTuiRenderer::create(std::uint32_t width,
                                                         std::uint32_t height) {
  auto renderer = std::unique_ptr<OpenTuiRenderer>(new OpenTuiRenderer());
  return renderer->load(width, height) ? std::move(renderer) : nullptr;
}

OpenTuiRenderer::~OpenTuiRenderer() { unload(); }

bool OpenTuiRenderer::load(std::uint32_t width, std::uint32_t height) {
  const std::wstring directory = moduleDirectory();
  const std::wstring path = directory.empty() ? L"opentui.dll" : directory + L"\\opentui.dll";
  HMODULE library = LoadLibraryW(path.c_str());
  if (!library) return false;
  library_ = library;
  api_ = std::make_unique<Api>();

  const bool complete =
      importFunction(library, "createRenderer", api_->createRenderer) &&
      importFunction(library, "destroyRenderer", api_->destroyRenderer) &&
      importFunction(library, "setClearOnShutdown", api_->setClearOnShutdown) &&
      importFunction(library, "setRenderOffset", api_->setRenderOffset) &&
      importFunction(library, "getNextBuffer", api_->getNextBuffer) &&
      importFunction(library, "bufferClear", api_->bufferClear) &&
      importFunction(library, "bufferDrawText", api_->bufferDrawText) &&
      importFunction(library, "setCursorPosition", api_->setCursorPosition) &&
      importFunction(library, "render", api_->render) &&
      importFunction(library, "resizeRenderer", api_->resizeRenderer);
  if (!complete) {
    unload();
    return false;
  }

  width_ = std::max<std::uint32_t>(width, 1);
  height_ = std::max<std::uint32_t>(height, 1);
  // 0 selects process stdout; 1 declares a local terminal; no span feed is used.
  renderer_ = api_->createRenderer(width_, height_, 0, 1, nullptr);
  if (renderer_ == 0) {
    unload();
    return false;
  }
  // This adapter deliberately does not call setupTerminal: LiteShell continues
  // to own raw input and terminal lifecycle, while OpenTUI owns only frame diffing.
  api_->setClearOnShutdown(renderer_, false);
  return true;
}

void OpenTuiRenderer::unload() noexcept {
  if (renderer_ != 0 && api_ && api_->destroyRenderer) {
    api_->destroyRenderer(renderer_);
    renderer_ = 0;
  }
  api_.reset();
  if (library_) {
    FreeLibrary(static_cast<HMODULE>(library_));
    library_ = nullptr;
  }
}

bool OpenTuiRenderer::resize(std::uint32_t width, std::uint32_t height) {
  if (renderer_ == 0) return false;
  width = std::max<std::uint32_t>(width, 1);
  height = std::max<std::uint32_t>(height, 1);
  if (width == width_ && height == height_) return true;
  api_->resizeRenderer(renderer_, width, height);
  width_ = width;
  height_ = height;
  return true;
}

bool OpenTuiRenderer::renderPrompt(std::wstring_view prompt, std::wstring_view line,
                                    std::size_t cursor, std::uint32_t viewportRow,
                                    const std::vector<OpenTuiCandidate>& candidates,
                                    std::size_t selectedCandidate,
                                    std::size_t totalCandidates, bool force) {
  if (renderer_ == 0 || !api_) return false;
  cursor = std::min(cursor, line.size());
  api_->setRenderOffset(renderer_, viewportRow);
  const NativeHandle buffer = api_->getNextBuffer(renderer_);
  if (buffer == 0) return false;
  api_->bufferClear(buffer, transparent.data());

  const auto draw = [this, buffer](std::wstring_view value, std::uint32_t x,
                                    std::uint32_t y, const PackedColor& color) {
    if (value.empty() || x >= width_ || y >= height_) return;
    const std::string utf8 = wideToUtf8(value);
    api_->bufferDrawText(buffer, reinterpret_cast<const std::uint8_t*>(utf8.data()),
                         static_cast<std::uint32_t>(utf8.size()), x, y,
                         color.data(), nullptr, 0);
  };

  if (height_ > 1) {
    const std::wstring header = L" fff  " + std::to_wstring(totalCandidates) +
                                L" matches  [Up/Down select, Tab/Enter accept, Esc close]";
    draw(header, 0, 0, headerColor);
    const std::size_t rows = std::min<std::size_t>(candidates.size(), height_ - 2);
    for (std::size_t index = 0; index < rows; ++index) {
      const bool selected = index == selectedCandidate;
      const std::wstring prefix = selected ? L"> " : L"  ";
      draw(prefix + candidates[index].label, 0, static_cast<std::uint32_t>(index + 1),
           selected ? selectedColor : candidateColor);
      if (!candidates[index].detail.empty()) {
        const std::size_t labelWidth = displayWidth(prefix + candidates[index].label);
        if (labelWidth + 3 < width_) {
          draw(L" · " + candidates[index].detail,
               static_cast<std::uint32_t>(labelWidth),
               static_cast<std::uint32_t>(index + 1), detailColor);
        }
      }
    }
  }

  const InputViewport viewport = visibleInput(prompt, line, cursor, width_);
  const std::string text = wideToUtf8(viewport.text);
  if (!text.empty()) {
    const std::size_t promptBytes =
        wideToUtf8(std::wstring_view(viewport.text).substr(0, viewport.promptCodeUnits)).size();
    const std::uint32_t safePromptBytes = static_cast<std::uint32_t>(
        std::min<std::size_t>(promptBytes, text.size()));
    if (safePromptBytes > 0) {
      api_->bufferDrawText(buffer, reinterpret_cast<const std::uint8_t*>(text.data()),
                           safePromptBytes, 0, height_ - 1, promptColor.data(), nullptr, 0);
    }
    if (safePromptBytes < text.size()) {
      api_->bufferDrawText(
          buffer, reinterpret_cast<const std::uint8_t*>(text.data() + safePromptBytes),
          static_cast<std::uint32_t>(text.size() - safePromptBytes),
          static_cast<std::uint32_t>(viewport.promptCells), height_ - 1,
          inputColor.data(), nullptr, 0);
    }
  }
  api_->setCursorPosition(renderer_, static_cast<std::int32_t>(viewport.cursorColumn),
                          static_cast<std::int32_t>(height_ - 1), true);
  // Render status 2 is failed; 0 and 1 mean rendered/skipped.
  return api_->render(renderer_, force) != 2;
}

}  // namespace liteshell
