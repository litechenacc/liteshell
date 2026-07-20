#pragma once

#include "fff_finder.hpp"
#include "history.hpp"
#include "terminal.hpp"

#include <filesystem>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace liteshell {

enum class ExternalKind { executable, command_script, powershell_script };

struct ResolvedCommand {
  std::filesystem::path path;
  ExternalKind kind = ExternalKind::executable;
};

class Shell {
 public:
  int run();

 private:
  std::wstring prompt() const;
  int dispatch(const std::vector<std::wstring>& arguments);
  int builtinCd(const std::vector<std::wstring>& arguments);
  int builtinPwd(const std::vector<std::wstring>& arguments);
  int builtinLs(const std::vector<std::wstring>& arguments);
  int builtinCat(const std::vector<std::wstring>& arguments);
  int builtinTail(const std::vector<std::wstring>& arguments);
  int builtinLess(const std::vector<std::wstring>& arguments);
  int builtinClear(const std::vector<std::wstring>& arguments);
  int builtinWhich(const std::vector<std::wstring>& arguments);
  int builtinFind(const std::vector<std::wstring>& arguments);
  int builtinRg(const std::vector<std::wstring>& arguments);
  int builtinHelp(const std::vector<std::wstring>& arguments);
  int launchExternal(const ResolvedCommand& command,
                     const std::vector<std::wstring>& arguments);
  std::optional<ResolvedCommand> resolveExternal(std::wstring_view command) const;
  CompletionResult complete(std::wstring_view line, std::size_t cursor);
  void loadPathCommands();
  bool isBuiltin(std::wstring_view command) const;
  void error(std::wstring_view command, std::wstring_view message);

  Terminal terminal_;
  History history_;
  FffFinder fff_;
  bool running_ = true;
  bool pathCommandsLoaded_ = false;
  std::vector<std::wstring> pathCommands_;
};

}  // namespace liteshell
