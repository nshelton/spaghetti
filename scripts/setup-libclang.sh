#!/usr/bin/env bash
# Ensures libclang is available for building frontend-clang.
# Idempotent — safe to run multiple times.
set -euo pipefail

already_available() {
  # Check if clang-sys can find libclang
  if [ -n "${LIBCLANG_PATH:-}" ] && [ -d "$LIBCLANG_PATH" ]; then
    return 0
  fi
  # Check common locations
  if ldconfig -p 2>/dev/null | grep -q libclang; then
    return 0
  fi
  if [ -f /usr/lib/llvm-*/lib/libclang.so ] 2>/dev/null; then
    return 0
  fi
  if [ -f /usr/lib/libclang.so ] 2>/dev/null; then
    return 0
  fi
  return 1
}

if already_available; then
  echo "libclang already available"
  exit 0
fi

echo "Installing libclang..."

if [ "$(uname)" = "Linux" ]; then
  if command -v apt-get &>/dev/null; then
    sudo apt-get update -qq
    sudo apt-get install -y -qq libclang-dev
  elif command -v dnf &>/dev/null; then
    sudo dnf install -y clang-devel
  elif command -v pacman &>/dev/null; then
    sudo pacman -S --noconfirm clang
  else
    echo "ERROR: unsupported Linux package manager" >&2
    exit 1
  fi
elif [ "$(uname)" = "Darwin" ]; then
  if command -v brew &>/dev/null; then
    brew install llvm
    echo "Set LIBCLANG_PATH=\$(brew --prefix llvm)/lib"
  else
    echo "ERROR: install Homebrew first, then: brew install llvm" >&2
    exit 1
  fi
else
  echo "ERROR: unsupported OS — install LLVM manually and set LIBCLANG_PATH" >&2
  exit 1
fi

echo "libclang setup complete"
