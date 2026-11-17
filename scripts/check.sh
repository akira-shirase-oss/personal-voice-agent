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
    # The service-backed suites drive the binary cargo built, wherever CARGO_TARGET_DIR put it;
    # without it they fail, and any skip other than an on-device one fails the gate.
    target_dir="$(cargo metadata --format-version 1 --no-deps --locked | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
    cargo build --locked -p voice-session
    export PVA_SESSION_BINARY="$target_dir/debug/voice-session"
    swift build --package-path apps/macos
    log="$(mktemp -t voice-agent-swift-test)"
    trap 'rm -f "$log"' EXIT
    swift test --package-path apps/macos 2>&1 | tee "$log"
    scripts/swift-skips.sh "$log"
  else
    echo "skip: apps/macos needs macOS + Xcode (Swift build and tests not run here)"
  fi
fi
echo "check ok"
