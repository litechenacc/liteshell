#include "shell.hpp"

#include <exception>
#include <iostream>

int wmain() {
  try {
    liteshell::Shell shell;
    return shell.run();
  } catch (const std::exception& exception) {
    std::cerr << "liteshell: " << exception.what() << '\n';
    return 1;
  }
}
