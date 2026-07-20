#pragma once

#define NOMINMAX
#include <windows.h>

#include <cstdint>
#include <functional>
#include <memory>
#include <optional>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

namespace liteshell {

class OpenTuiRenderer;

enum class KeyKind {
  character,
  enter,
  escape,
  backspace,
  delete_key,
  left,
  right,
  home,
  end,
  up,
  down,
  page_up,
  page_down,
  scroll_up,
  scroll_down,
  tab,
  ctrl_c,
  ctrl_l,
  unknown,
};

struct KeyPress {
  KeyKind kind = KeyKind::unknown;
  wchar_t character = L'\0';
};

struct CompletionResult {
  std::wstring line;
  std::size_t cursor = 0;
  struct Candidate {
    std::wstring label;
    std::wstring detail;
    std::wstring replacement;
  };
  std::vector<Candidate> alternatives;
  std::size_t totalAlternatives = 0;
  std::size_t replacementStart = 0;
  std::size_t replacementLength = 0;
  bool changed = false;
};

using CompletionFunction =
    std::function<CompletionResult(std::wstring_view line, std::size_t cursor)>;

class Terminal {
 public:
  Terminal();
  ~Terminal();

  Terminal(const Terminal&) = delete;
  Terminal& operator=(const Terminal&) = delete;

  bool interactive() const noexcept { return interactive_; }
  void write(std::wstring_view value, bool flush = false);
  void writeError(std::wstring_view value);
  void clear();
  std::pair<int, int> size() const;
  std::optional<std::wstring> readLine(std::wstring_view prompt,
                                       const std::vector<std::wstring>& history,
                                       const CompletionFunction& complete);
  void beginRawInput(bool mouse = false) { setRawInput(true, mouse); }
  void endRawInput() { setRawInput(false); }
  KeyPress readKey();
  void enterAlternateScreen();
  void leaveAlternateScreen();
  void resetAfterChild();

 private:
  KeyPress readConsoleKey();
  void redrawLine(std::wstring_view prompt, std::wstring_view line,
                  std::size_t cursor,
                  const std::vector<CompletionResult::Candidate>& candidates,
                  std::size_t selectedCandidate, std::size_t totalCandidates,
                  bool relocate = false);
  void setRawInput(bool enabled, bool mouse = false);
  std::uint32_t viewportCursorRow() const;

  HANDLE input_ = INVALID_HANDLE_VALUE;
  HANDLE output_ = INVALID_HANDLE_VALUE;
  DWORD originalInputMode_ = 0;
  DWORD originalOutputMode_ = 0;
  bool inputIsConsole_ = false;
  bool outputIsConsole_ = false;
  bool interactive_ = false;
  bool vtEnabled_ = false;
  bool rawInput_ = false;
  bool mouseInput_ = false;
  std::uint32_t promptSurfaceHeight_ = 1;
  std::unique_ptr<OpenTuiRenderer> openTui_;
};

}  // namespace liteshell
