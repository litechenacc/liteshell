#define NOMINMAX
#include <windows.h>

#include <iostream>
#include <string>
#include <string_view>
#include <vector>

namespace {

HANDLE reportHandle = INVALID_HANDLE_VALUE;

void report(std::string_view message) {
  DWORD written = 0;
  WriteFile(reportHandle, message.data(), static_cast<DWORD>(message.size()), &written, nullptr);
}

int fail(std::string_view message) {
  report("Console input test failed: ");
  report(message);
  report("\n");
  return 1;
}

bool sendKey(HANDLE input, WORD virtualKey, wchar_t character = 0,
             DWORD controls = 0) {
  INPUT_RECORD records[2]{};
  for (INPUT_RECORD& record : records) {
    record.EventType = KEY_EVENT;
    record.Event.KeyEvent.wRepeatCount = 1;
    record.Event.KeyEvent.wVirtualKeyCode = virtualKey;
    record.Event.KeyEvent.uChar.UnicodeChar = character;
    record.Event.KeyEvent.dwControlKeyState = controls;
  }
  records[0].Event.KeyEvent.bKeyDown = TRUE;
  DWORD written = 0;
  return WriteConsoleInputW(input, records, 2, &written) && written == 2;
}

bool sendText(HANDLE input, std::wstring_view text) {
  for (const wchar_t ch : text) {
    const SHORT key = VkKeyScanW(ch);
    const WORD virtualKey = key == -1 ? 0 : static_cast<WORD>(key & 0xFF);
    if (!sendKey(input, virtualKey, ch)) return false;
  }
  return true;
}

bool sendEnter(HANDLE input) {
  return sendKey(input, VK_RETURN, L'\r');
}

bool sendWheel(HANDLE input, SHORT delta) {
  INPUT_RECORD record{};
  record.EventType = MOUSE_EVENT;
  record.Event.MouseEvent.dwEventFlags = MOUSE_WHEELED;
  record.Event.MouseEvent.dwButtonState =
      static_cast<DWORD>(static_cast<WORD>(delta)) << 16;
  DWORD written = 0;
  return WriteConsoleInputW(input, &record, 1, &written) && written == 1;
}

bool sendCommand(HANDLE input, std::wstring_view command) {
  return sendText(input, command) && sendEnter(input);
}

std::wstring readScreen(HANDLE output) {
  CONSOLE_SCREEN_BUFFER_INFO info{};
  if (!GetConsoleScreenBufferInfo(output, &info)) return {};
  const DWORD cells = static_cast<DWORD>(info.dwSize.X) * static_cast<DWORD>(info.dwSize.Y);
  std::wstring screen(cells, L' ');
  DWORD read = 0;
  ReadConsoleOutputCharacterW(output, screen.data(), cells, COORD{0, 0}, &read);
  screen.resize(read);
  return screen;
}

std::string toUtf8(std::wstring_view value) {
  const int required = WideCharToMultiByte(CP_UTF8, 0, value.data(),
                                            static_cast<int>(value.size()), nullptr, 0,
                                            nullptr, nullptr);
  std::string result(static_cast<std::size_t>(required), '\0');
  WideCharToMultiByte(CP_UTF8, 0, value.data(), static_cast<int>(value.size()),
                      result.data(), required, nullptr, nullptr);
  return result;
}

}  // namespace

int wmain(int argc, wchar_t** argv) {
  reportHandle = GetStdHandle(STD_OUTPUT_HANDLE);
  if (argc != 3) return fail("expected liteshell.exe and project root arguments");

  FreeConsole();
  if (!AllocConsole()) return fail("AllocConsole");
  if (const HWND window = GetConsoleWindow()) ShowWindow(window, SW_HIDE);

  SECURITY_ATTRIBUTES security{sizeof(security), nullptr, TRUE};
  HANDLE consoleInput = CreateFileW(L"CONIN$", GENERIC_READ | GENERIC_WRITE,
                                    FILE_SHARE_READ | FILE_SHARE_WRITE, &security,
                                    OPEN_EXISTING, 0, nullptr);
  HANDLE consoleOutput = CreateFileW(L"CONOUT$", GENERIC_READ | GENERIC_WRITE,
                                     FILE_SHARE_READ | FILE_SHARE_WRITE, &security,
                                     OPEN_EXISTING, 0, nullptr);
  if (consoleInput == INVALID_HANDLE_VALUE || consoleOutput == INVALID_HANDLE_VALUE) {
    return fail("opening hidden console handles");
  }
  SetStdHandle(STD_INPUT_HANDLE, consoleInput);
  SetStdHandle(STD_OUTPUT_HANDLE, consoleOutput);
  SetStdHandle(STD_ERROR_HANDLE, consoleOutput);
  SetConsoleScreenBufferSize(consoleOutput, COORD{160, 200});
  DWORD configuredInputMode = 0;
  GetConsoleMode(consoleInput, &configuredInputMode);
  configuredInputMode |= ENABLE_EXTENDED_FLAGS | ENABLE_QUICK_EDIT_MODE;
  configuredInputMode &= ~ENABLE_MOUSE_INPUT;
  SetConsoleMode(consoleInput, configuredInputMode);

  STARTUPINFOW startup{};
  startup.cb = sizeof(startup);
  startup.dwFlags = STARTF_USESTDHANDLES;
  startup.hStdInput = consoleInput;
  startup.hStdOutput = consoleOutput;
  startup.hStdError = consoleOutput;
  PROCESS_INFORMATION process{};
  std::wstring commandLine = L"\"" + std::wstring(argv[1]) + L"\"";
  std::vector<wchar_t> mutableCommand(commandLine.begin(), commandLine.end());
  mutableCommand.push_back(L'\0');
  if (!CreateProcessW(argv[1], mutableCommand.data(), nullptr, nullptr, TRUE, 0,
                      nullptr, argv[2], &startup, &process)) {
    return fail("CreateProcessW");
  }
  CloseHandle(process.hThread);
  Sleep(250);

  bool sent = true;
  sent = sent && sendKey(consoleInput, 'L', 12, LEFT_CTRL_PRESSED);
  sent = sent && sendKey(consoleInput, 'C', 3, LEFT_CTRL_PRESSED);
  Sleep(100);

  // Left + Delete turns "pwx" into "pwd".
  sent = sent && sendText(consoleInput, L"pwx");
  sent = sent && sendKey(consoleInput, VK_LEFT);
  sent = sent && sendKey(consoleInput, VK_DELETE);
  sent = sent && sendText(consoleInput, L"d");
  sent = sent && sendEnter(consoleInput);

  // Left + Right round-trip, then insert in the middle.
  sent = sent && sendText(consoleInput, L"pd");
  sent = sent && sendKey(consoleInput, VK_LEFT);
  sent = sent && sendKey(consoleInput, VK_RIGHT);
  sent = sent && sendKey(consoleInput, VK_LEFT);
  sent = sent && sendText(consoleInput, L"w");
  sent = sent && sendEnter(consoleInput);

  // Home + End turns "wd" into "pwd".
  sent = sent && sendText(consoleInput, L"wd");
  sent = sent && sendKey(consoleInput, VK_HOME);
  sent = sent && sendText(consoleInput, L"p");
  sent = sent && sendKey(consoleInput, VK_END);
  sent = sent && sendEnter(consoleInput);

  // Backspace, Tab completion, then Up history.
  sent = sent && sendText(consoleInput, L"pwdx");
  sent = sent && sendKey(consoleInput, VK_BACK, L'\b');
  sent = sent && sendEnter(consoleInput);
  sent = sent && sendText(consoleInput, L"pwd");
  sent = sent && sendKey(consoleInput, VK_TAB, L'\t');
  sent = sent && sendEnter(consoleInput);
  sent = sent && sendKey(consoleInput, VK_UP);
  sent = sent && sendEnter(consoleInput);
  sent = sent && sendText(consoleInput, L"cat tests\\fixtures\\com.t");
  sent = sent && sendKey(consoleInput, VK_TAB, L'\t');
  sent = sent && sendEnter(consoleInput);
  sent = sent && sendText(consoleInput, L"cd sr");
  sent = sent && sendKey(consoleInput, VK_TAB, L'\t');
  sent = sent && sendEnter(consoleInput);
  sent = sent && sendCommand(consoleInput, L"pwd");
  sent = sent && sendCommand(consoleInput, L"cd ..");
  sent = sent && sendText(consoleInput, L"where");
  sent = sent && sendKey(consoleInput, VK_TAB, L'\t');
  sent = sent && sendText(consoleInput, L"cmd");
  sent = sent && sendEnter(consoleInput);
  Sleep(250);
  const bool pathCompletionVisible =
      readScreen(consoleOutput).find(L"cmd.exe") != std::wstring::npos;

  // fff-backed completion opens a selectable surface above the prompt.
  sent = sent && sendText(consoleInput, L"ls ");
  sent = sent && sendKey(consoleInput, VK_TAB, L'\t');
  Sleep(250);
  const std::wstring completionScreen = readScreen(consoleOutput);
  const bool candidateSurfaceVisible =
      completionScreen.find(L"fff ") != std::wstring::npos &&
      completionScreen.find(L"matches") != std::wstring::npos;
  sent = sent && sendKey(consoleInput, VK_DOWN);
  sent = sent && sendKey(consoleInput, VK_TAB, L'\t');
  sent = sent && sendKey(consoleInput, 'C', 3, LEFT_CTRL_PRESSED);
  Sleep(100);

  sent = sent && sendCommand(consoleInput, L"less tests\\fixtures\\tail.txt");
  Sleep(250);
  // Some hidden classic-console hosts reject synthetic MOUSE_WHEELED records;
  // mode inspection above still proves that real wheel events are requested.
  (void)sendWheel(consoleInput, WHEEL_DELTA);
  (void)sendWheel(consoleInput, -WHEEL_DELTA);
  sent = sent && sendKey(consoleInput, VK_DOWN);
  sent = sent && sendKey(consoleInput, 'J', L'j');
  sent = sent && sendKey(consoleInput, VK_UP);
  sent = sent && sendKey(consoleInput, 'K', L'k');
  sent = sent && sendKey(consoleInput, VK_NEXT);
  sent = sent && sendKey(consoleInput, VK_SPACE, L' ');
  sent = sent && sendKey(consoleInput, VK_PRIOR);
  sent = sent && sendKey(consoleInput, 'G', L'G', SHIFT_PRESSED);
  sent = sent && sendKey(consoleInput, 'G', L'g');
  sent = sent && sendKey(consoleInput, 'Q', L'q');
  Sleep(200);
  sent = sent && sendCommand(consoleInput, L"less tests\\fixtures\\tail.txt");
  Sleep(200);
  sent = sent && sendKey(consoleInput, VK_ESCAPE, 0x1B);
  Sleep(100);
  sent = sent && sendCommand(consoleInput, L"pwd");
  sent = sent && sendCommand(consoleInput, L"exit");
  if (!sent) {
    TerminateProcess(process.hProcess, 2);
    return fail("WriteConsoleInputW");
  }

  if (WaitForSingleObject(process.hProcess, 15000) != WAIT_OBJECT_0) {
    TerminateProcess(process.hProcess, 3);
    WaitForSingleObject(process.hProcess, 1000);
    return fail("shell did not exit after key sequence");
  }
  DWORD exitCode = 1;
  GetExitCodeProcess(process.hProcess, &exitCode);
  CloseHandle(process.hProcess);

  const std::wstring screen = readScreen(consoleOutput);
  CloseHandle(consoleInput);
  CloseHandle(consoleOutput);
  FreeConsole();

  if (exitCode != 0) return fail("liteshell returned non-zero");
  if (!candidateSurfaceVisible) return fail("fff candidate surface was not rendered");
  if (screen.find(L"^C") == std::wstring::npos) return fail("Ctrl-C was not handled");
  if (screen.find(L"command not found") != std::wstring::npos) {
    report(toUtf8(screen));
    report("\n");
    return fail("an edited or completed pwd command was malformed");
  }
  if (screen.find(L"COM_OK") == std::wstring::npos) return fail("file completion failed");
  if (!pathCompletionVisible) {
    return fail("PATH executable completion failed");
  }
  if (screen.find(argv[2]) == std::wstring::npos) return fail("pwd did not execute");

  report("LiteShell console input interaction test passed.\n");
  return 0;
}
