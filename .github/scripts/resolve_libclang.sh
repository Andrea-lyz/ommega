#!/usr/bin/env bash
set -euo pipefail

# Ubuntu 24.04 provides LLVM 18; hosted runners may expose its versioned command.
find_libclang() {
  local llvm_config libclang_path
  for llvm_config in llvm-config llvm-config-18; do
    if libclang_path="$("$llvm_config" --libdir 2>/dev/null)" &&
      { [[ -f "$libclang_path/libclang.so" ]] || [[ -f "$libclang_path/libclang.so.1" ]]; }; then
      printf 'libclang-path=%s\n' "$libclang_path" >> "$GITHUB_OUTPUT"
      return 0
    fi
  done
  return 1
}

if ! find_libclang; then
  sudo apt-get update
  sudo apt-get install -y --no-install-recommends clang libclang-dev llvm-dev
  find_libclang
fi
