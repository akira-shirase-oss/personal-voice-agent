#!/usr/bin/env bash
# Runs what CI runs: the toolchain pin, Rust gates, the architecture rules, then the Swift
# package's build and tests on macOS. Run it before every commit.
set -euo pipefail
cd "$(dirname "$0")/.."

scripts/check-toolchain.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
scripts/check-architecture.sh

if [ -f apps/macos/Package.swift ]; then
  if [ "$(uname -s)" = "Darwin" ]; then
    export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
    swift build --package-path apps/macos
    swift test --package-path apps/macos
  else
    echo "skip: apps/macos needs macOS + Xcode (Swift build and tests not run here)"
  fi
fi
echo "check ok"
