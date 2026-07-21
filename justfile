# Do not let just's default `sh` prepend Unix coreutils to PATH on Windows.
# Its `link.exe` creates hard links and shadows MSVC's linker.
set windows-shell := ["cmd.exe", "/C"]

default: test

build:
    cargo build -p liteshell

dev: build
    target\debug\liteshell.exe

test:
    cargo test --workspace

release:
    cargo build --release -p liteshell -p liteshell-launcher

install: release
    cargo run --quiet -p xtask -- install

fetch:
    cargo run -p xtask -- fetch

package:
    cargo run -p xtask -- package

fmt:
    cargo fmt --all

check:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
