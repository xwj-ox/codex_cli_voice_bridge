#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "[CHECK] project root: ${PROJECT_ROOT}"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "[ERROR] This script must run on macOS." >&2
  exit 1
fi

failures=0

check_cmd() {
  local name="$1"
  if command -v "$name" >/dev/null 2>&1; then
    echo "[OK] ${name}: $(command -v "$name")"
  else
    echo "[MISSING] ${name}"
    failures=$((failures + 1))
  fi
}

check_cmd xcode-select
check_cmd clang
check_cmd cargo
check_cmd rustup
check_cmd osascript
check_cmd pbcopy

if command -v xcode-select >/dev/null 2>&1; then
  if xcode-select -p >/dev/null 2>&1; then
    echo "[OK] Xcode Command Line Tools: $(xcode-select -p)"
  else
    echo "[MISSING] Xcode Command Line Tools"
    failures=$((failures + 1))
  fi
fi

if command -v rustup >/dev/null 2>&1; then
  if rustup target list --installed | grep -Eq 'aarch64-apple-darwin|x86_64-apple-darwin'; then
    echo "[OK] apple-darwin Rust target is installed"
  else
    echo "[MISSING] apple-darwin Rust target"
    echo "         Run: rustup target add aarch64-apple-darwin"
    failures=$((failures + 1))
  fi
fi

echo
echo "[INFO] Runtime permissions are checked by:"
echo "       cargo run --bin voice_bridge_doctor"
echo
echo "[INFO] Release build command:"
echo "       ./packaging/build_macos_release.sh"

if [[ ${failures} -gt 0 ]]; then
  echo
  echo "[FAIL] macOS environment is not ready. Missing checks: ${failures}" >&2
  exit 1
fi

echo
echo "[OK] macOS environment looks ready for the first source build."
