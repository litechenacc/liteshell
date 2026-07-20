#define NOMINMAX
#include <windows.h>

#include "terminal.hpp"

#include "opentui_renderer.hpp"

#include <algorithm>
#include <cstdio>
#include <fcntl.h>
#include <io.h>
#include <iostream>
#include <stdexcept>

namespace liteshell {
namespace {

std::size_t previousCharacter(std::wstring_view value, std::size_t cursor) {
  if (cursor == 0) return 0;
  --cursor;
  if (cursor > 0 && value[cursor] >= 0xDC00 && value[cursor] <= 0xDFFF &&
      value[cursor - 1] >= 0xD800 && value[cursor - 1] <= 0xDBFF) {
    --cursor;
  }
  return cursor;
}

std::size_t nextCharacter(std::wstring_view value, std::size_t cursor) {
  if (cursor >= value.size()) return value.size();
  if (value[cursor] >= 0xD800 && value[cursor] <= 0xDBFF &&
      cursor + 1 < value.size() && value[cursor + 1] >= 0xDC00 &&
      value[cursor + 1] <= 0xDFFF) {
    return cursor + 2;
  }
  return cursor + 1;
}

void setUtf8StreamMode(FILE* stream) {
  if (_setmode(_fileno(stream), _O_U8TEXT) == -1) {
    // Keep the CRT's existing mode if this particular standard handle does not
    // support translation (for example, an unusual embedded host).
    clearerr(stream);
  }
}

}  // namespace

Terminal::Terminal() {
  setUtf8StreamMode(stdin);
  setUtf8StreamMode(stdout);
  setUtf8StreamMode(stderr);
  SetConsoleCP(CP_UTF8);
  SetConsoleOutputCP(CP_UTF8);

  input_ = GetStdHandle(STD_INPUT_HANDLE);
  output_ = GetStdHandle(STD_OUTPUT_HANDLE);
  inputIsConsole_ = input_ != INVALID_HANDLE_VALUE &&
                    GetConsoleMode(input_, &originalInputMode_) != FALSE;
  outputIsConsole_ = output_ != INVALID_HANDLE_VALUE &&
                     GetConsoleMode(output_, &originalOutputMode_) != FALSE;
  interactive_ = inputIsConsole_ && outputIsConsole_;

  if (outputIsConsole_) {
    const DWORD desired = originalOutputMode_ | ENABLE_PROCESSED_OUTPUT |
                          ENABLE_VIRTUAL_TERMINAL_PROCESSING;
    vtEnabled_ = SetConsoleMode(output_, desired) != FALSE;
  }

  if (interactive_ && !vtEnabled_) {
    throw std::runtime_error("OpenTUI requires virtual terminal output support");
  }
  if (interactive_) {
    const auto [width, unusedHeight] = size();
    (void)unusedHeight;
    openTui_ = OpenTuiRenderer::create(static_cast<std::uint32_t>(width), 1);
    if (!openTui_) {
      throw std::runtime_error("OpenTUI renderer initialization failed");
    }
  }
}

Terminal::~Terminal() {
  openTui_.reset();
  if (rawInput_) {
    setRawInput(false);
  }
  if (inputIsConsole_) {
    SetConsoleMode(input_, originalInputMode_);
  }
  if (outputIsConsole_) {
    SetConsoleMode(output_, originalOutputMode_);
  }
}

void Terminal::write(std::wstring_view value, bool flush) {
  std::wcout.write(value.data(), static_cast<std::streamsize>(value.size()));
  if (flush) {
    std::wcout.flush();
  }
}

void Terminal::writeError(std::wstring_view value) {
  std::wcerr << value;
  std::wcerr.flush();
}

void Terminal::clear() {
  if (vtEnabled_) {
    write(L"\x1b[2J\x1b[H", true);
    return;
  }
  if (!outputIsConsole_) {
    return;
  }
  CONSOLE_SCREEN_BUFFER_INFO info{};
  if (!GetConsoleScreenBufferInfo(output_, &info)) {
    return;
  }
  const DWORD cells = static_cast<DWORD>(info.dwSize.X) * static_cast<DWORD>(info.dwSize.Y);
  DWORD written = 0;
  const COORD origin{0, 0};
  FillConsoleOutputCharacterW(output_, L' ', cells, origin, &written);
  FillConsoleOutputAttribute(output_, info.wAttributes, cells, origin, &written);
  SetConsoleCursorPosition(output_, origin);
}

std::uint32_t Terminal::viewportCursorRow() const {
  CONSOLE_SCREEN_BUFFER_INFO info{};
  if (outputIsConsole_ && GetConsoleScreenBufferInfo(output_, &info)) {
    const int row = std::max(0, static_cast<int>(info.dwCursorPosition.Y) -
                                    static_cast<int>(info.srWindow.Top));
    const int maximum = std::max(0, static_cast<int>(info.srWindow.Bottom) -
                                        static_cast<int>(info.srWindow.Top));
    return static_cast<std::uint32_t>(std::min(row, maximum));
  }
  return 0;
}

std::pair<int, int> Terminal::size() const {
  CONSOLE_SCREEN_BUFFER_INFO info{};
  if (outputIsConsole_ && GetConsoleScreenBufferInfo(output_, &info)) {
    return {std::max(1, static_cast<int>(info.srWindow.Right - info.srWindow.Left + 1)),
            std::max(2, static_cast<int>(info.srWindow.Bottom - info.srWindow.Top + 1))};
  }
  return {80, 24};
}

void Terminal::setRawInput(bool enabled, bool mouse) {
  if (!inputIsConsole_ || (rawInput_ == enabled && (!enabled || mouseInput_ == mouse))) {
    return;
  }
  if (enabled) {
    DWORD mode = originalInputMode_;
    mode &= ~(ENABLE_ECHO_INPUT | ENABLE_LINE_INPUT | ENABLE_PROCESSED_INPUT);
    mode |= ENABLE_EXTENDED_FLAGS | ENABLE_WINDOW_INPUT;
    if (mouse) {
      mode &= ~ENABLE_QUICK_EDIT_MODE;
      mode |= ENABLE_MOUSE_INPUT;
    } else {
      // Leave selection/scrollback behavior with the terminal host while the
      // prompt is idle. Mouse events are captured only by the pager.
      mode &= ~ENABLE_MOUSE_INPUT;
      if ((originalInputMode_ & ENABLE_QUICK_EDIT_MODE) != 0) {
        mode |= ENABLE_QUICK_EDIT_MODE;
      }
    }
    if (SetConsoleMode(input_, mode)) {
      rawInput_ = true;
      mouseInput_ = mouse;
    }
  } else {
    SetConsoleMode(input_, originalInputMode_);
    rawInput_ = false;
    mouseInput_ = false;
  }
}

KeyPress Terminal::readConsoleKey() {
  INPUT_RECORD record{};
  DWORD read = 0;
  while (ReadConsoleInputW(input_, &record, 1, &read)) {
    if (record.EventType == MOUSE_EVENT &&
        record.Event.MouseEvent.dwEventFlags == MOUSE_WHEELED) {
      const SHORT delta = static_cast<SHORT>(
          HIWORD(record.Event.MouseEvent.dwButtonState));
      return {delta > 0 ? KeyKind::scroll_up : KeyKind::scroll_down, 0};
    }
    if (record.EventType != KEY_EVENT || !record.Event.KeyEvent.bKeyDown) {
      continue;
    }
    const KEY_EVENT_RECORD& key = record.Event.KeyEvent;
    const bool control = (key.dwControlKeyState & (LEFT_CTRL_PRESSED | RIGHT_CTRL_PRESSED)) != 0;
    if ((control && key.wVirtualKeyCode == 'C') || key.uChar.UnicodeChar == 3) {
      return {KeyKind::ctrl_c, 0};
    }
    if ((control && key.wVirtualKeyCode == 'L') || key.uChar.UnicodeChar == 12) {
      return {KeyKind::ctrl_l, 0};
    }
    if (key.uChar.UnicodeChar == L'\r' || key.uChar.UnicodeChar == L'\n') {
      return {KeyKind::enter, 0};
    }
    if (key.uChar.UnicodeChar == L'\b' || key.uChar.UnicodeChar == 0x7F) {
      return {KeyKind::backspace, 0};
    }
    if (key.uChar.UnicodeChar == L'\t') {
      return {KeyKind::tab, 0};
    }
    if (key.uChar.UnicodeChar == 0x1B) {
      return {KeyKind::escape, 0};
    }
    switch (key.wVirtualKeyCode) {
      case VK_RETURN:
        return {KeyKind::enter, 0};
      case VK_ESCAPE:
        return {KeyKind::escape, 0};
      case VK_BACK:
        return {KeyKind::backspace, 0};
      case VK_DELETE:
        return {KeyKind::delete_key, 0};
      case VK_LEFT:
        return {KeyKind::left, 0};
      case VK_RIGHT:
        return {KeyKind::right, 0};
      case VK_HOME:
        return {KeyKind::home, 0};
      case VK_END:
        return {KeyKind::end, 0};
      case VK_UP:
        return {KeyKind::up, 0};
      case VK_DOWN:
        return {KeyKind::down, 0};
      case VK_PRIOR:
        return {KeyKind::page_up, 0};
      case VK_NEXT:
        return {KeyKind::page_down, 0};
      case VK_TAB:
        return {KeyKind::tab, 0};
      default:
        if (key.uChar.UnicodeChar >= L' ') {
          return {KeyKind::character, key.uChar.UnicodeChar};
        }
        break;
    }
  }
  return {};
}

KeyPress Terminal::readKey() {
  if (!interactive_) {
    wchar_t ch = 0;
    if (!std::wcin.get(ch)) {
      return {KeyKind::escape, 0};
    }
    return {KeyKind::character, ch};
  }
  const bool wasRaw = rawInput_;
  setRawInput(true);
  const KeyPress result = readConsoleKey();
  if (!wasRaw) {
    setRawInput(false);
  }
  return result;
}

void Terminal::redrawLine(std::wstring_view prompt, std::wstring_view line,
                          std::size_t cursor,
                          const std::vector<CompletionResult::Candidate>& candidates,
                          std::size_t selectedCandidate, std::size_t totalCandidates,
                          bool relocate) {
  if (!openTui_) {
    throw std::runtime_error("OpenTUI renderer is not initialized");
  }

  constexpr std::size_t maximumCandidateRows = 6;
  const std::size_t firstCandidate =
      selectedCandidate >= maximumCandidateRows
          ? selectedCandidate - maximumCandidateRows + 1
          : 0;
  const std::size_t visibleCount = candidates.empty()
                                       ? 0
                                       : std::min(maximumCandidateRows,
                                                  candidates.size() - firstCandidate);
  const std::uint32_t desiredHeight = visibleCount == 0
                                          ? 1
                                          : static_cast<std::uint32_t>(visibleCount + 2);
  if (desiredHeight > promptSurfaceHeight_) {
    for (std::uint32_t row = promptSurfaceHeight_; row < desiredHeight; ++row) {
      std::wcout << L"\r\n";
    }
    std::wcout.flush();
    promptSurfaceHeight_ = desiredHeight;
    relocate = true;
  }

  std::vector<OpenTuiCandidate> visible;
  visible.reserve(visibleCount);
  for (std::size_t index = 0; index < visibleCount; ++index) {
    const auto& candidate = candidates[firstCandidate + index];
    visible.push_back({candidate.label, candidate.detail});
  }
  const std::size_t visibleSelection =
      selectedCandidate >= firstCandidate ? selectedCandidate - firstCandidate : 0;
  const auto [width, unusedHeight] = size();
  (void)unusedHeight;
  openTui_->resize(static_cast<std::uint32_t>(width), promptSurfaceHeight_);
  const std::uint32_t cursorRow = viewportCursorRow();
  const std::uint32_t topRow = cursorRow >= promptSurfaceHeight_ - 1
                                   ? cursorRow - promptSurfaceHeight_ + 1
                                   : 0;
  if (!openTui_->renderPrompt(prompt, line, cursor, topRow, visible,
                              visibleSelection, totalCandidates, relocate)) {
    throw std::runtime_error("OpenTUI render failed");
  }
}

std::optional<std::wstring> Terminal::readLine(
    std::wstring_view prompt, const std::vector<std::wstring>& history,
    const CompletionFunction& complete) {
  if (!interactive_) {
    write(prompt, true);
    std::wstring line;
    if (!std::getline(std::wcin, line)) {
      return std::nullopt;
    }
    return line;
  }

  setRawInput(true);
  std::wstring line;
  std::wstring scratch;
  std::size_t cursor = 0;
  std::size_t historyIndex = history.size();
  std::vector<CompletionResult::Candidate> menuCandidates;
  std::size_t selectedCandidate = 0;
  std::size_t totalCandidates = 0;
  std::size_t menuReplacementStart = 0;
  std::size_t menuReplacementLength = 0;
  promptSurfaceHeight_ = 1;
  redrawLine(prompt, line, cursor, menuCandidates, selectedCandidate,
             totalCandidates, true);

  const auto closeMenu = [&]() {
    menuCandidates.clear();
    selectedCandidate = 0;
    totalCandidates = 0;
    menuReplacementStart = 0;
    menuReplacementLength = 0;
  };
  const auto acceptCandidate = [&]() {
    if (menuCandidates.empty()) return;
    const auto& candidate = menuCandidates[std::min(selectedCandidate,
                                                     menuCandidates.size() - 1)];
    const std::size_t start = std::min(menuReplacementStart, line.size());
    const std::size_t length = std::min(menuReplacementLength, line.size() - start);
    line.replace(start, length, candidate.replacement);
    cursor = start + candidate.replacement.size();
    closeMenu();
  };

  while (true) {
    const KeyPress key = readConsoleKey();
    bool repaint = true;
    switch (key.kind) {
      case KeyKind::character:
        closeMenu();
        line.insert(line.begin() + static_cast<std::ptrdiff_t>(cursor), key.character);
        ++cursor;
        break;
      case KeyKind::backspace:
        closeMenu();
        if (cursor > 0) {
          const std::size_t previous = previousCharacter(line, cursor);
          line.erase(previous, cursor - previous);
          cursor = previous;
        }
        break;
      case KeyKind::delete_key:
        closeMenu();
        if (cursor < line.size()) {
          line.erase(cursor, nextCharacter(line, cursor) - cursor);
        }
        break;
      case KeyKind::left:
        closeMenu();
        if (cursor > 0) {
          cursor = previousCharacter(line, cursor);
        }
        break;
      case KeyKind::right:
        closeMenu();
        if (cursor < line.size()) {
          cursor = nextCharacter(line, cursor);
        }
        break;
      case KeyKind::home:
        closeMenu();
        cursor = 0;
        break;
      case KeyKind::end:
        closeMenu();
        cursor = line.size();
        break;
      case KeyKind::up:
        if (!menuCandidates.empty()) {
          selectedCandidate = selectedCandidate == 0
                                  ? menuCandidates.size() - 1
                                  : selectedCandidate - 1;
        } else if (!history.empty() && historyIndex > 0) {
          if (historyIndex == history.size()) {
            scratch = line;
          }
          --historyIndex;
          line = history[historyIndex];
          cursor = line.size();
        }
        break;
      case KeyKind::down:
        if (!menuCandidates.empty()) {
          selectedCandidate = (selectedCandidate + 1) % menuCandidates.size();
        } else if (historyIndex < history.size()) {
          ++historyIndex;
          line = historyIndex == history.size() ? scratch : history[historyIndex];
          cursor = line.size();
        }
        break;
      case KeyKind::tab: {
        if (!menuCandidates.empty()) {
          acceptCandidate();
          break;
        }
        CompletionResult result = complete(line, cursor);
        if (result.changed) {
          line = std::move(result.line);
          cursor = std::min(result.cursor, line.size());
        }
        if (result.alternatives.size() > 1) {
          menuCandidates = std::move(result.alternatives);
          selectedCandidate = 0;
          totalCandidates = result.totalAlternatives == 0
                                ? menuCandidates.size()
                                : result.totalAlternatives;
          menuReplacementStart = result.replacementStart;
          menuReplacementLength = result.replacementLength;
        }
        break;
      }
      case KeyKind::ctrl_l:
        closeMenu();
        clear();
        promptSurfaceHeight_ = 1;
        redrawLine(prompt, line, cursor, menuCandidates, selectedCandidate,
                   totalCandidates, true);
        repaint = false;
        break;
      case KeyKind::ctrl_c:
        write(L"^C\r\n", true);
        setRawInput(false);
        return std::wstring{};
      case KeyKind::enter:
        if (!menuCandidates.empty()) {
          acceptCandidate();
          break;
        }
        write(L"\r\n", true);
        setRawInput(false);
        return line;
      case KeyKind::escape:
        if (!menuCandidates.empty()) {
          closeMenu();
          break;
        }
        repaint = false;
        break;
      case KeyKind::page_up:
      case KeyKind::page_down:
      case KeyKind::scroll_up:
      case KeyKind::scroll_down:
      case KeyKind::unknown:
        repaint = false;
        break;
    }
    if (repaint) {
      redrawLine(prompt, line, cursor, menuCandidates, selectedCandidate,
                 totalCandidates);
    }
  }
}

void Terminal::enterAlternateScreen() {
  if (vtEnabled_) {
    write(L"\x1b[?1049h\x1b[?25l", true);
  } else {
    clear();
  }
}

void Terminal::leaveAlternateScreen() {
  if (vtEnabled_) {
    write(L"\x1b[?25h\x1b[?1049l", true);
  } else {
    clear();
  }
}

void Terminal::resetAfterChild() {
  if (inputIsConsole_) {
    SetConsoleMode(input_, originalInputMode_);
  }
  if (outputIsConsole_) {
    const DWORD desired = originalOutputMode_ | ENABLE_PROCESSED_OUTPUT |
                          ENABLE_VIRTUAL_TERMINAL_PROCESSING;
    vtEnabled_ = SetConsoleMode(output_, desired) != FALSE;
  }
  rawInput_ = false;
  mouseInput_ = false;
}

}  // namespace liteshell
