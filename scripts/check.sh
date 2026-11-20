#!/usr/bin/env bash
# Runs what CI runs: the toolchain pin, Rust gates, the architecture rules, then the Swift
# package's build and tests on macOS. Run it before every commit.
set -euo pipefail
cd "$(dirname "$0")/.."

# Tests make data directories (named pva-*) that hold heard transcripts. They run in a private
# temp directory, and the gate fails if any is left behind there or in the user's temp directory.
user_tmp="$(getconf DARWIN_USER_TEMP_DIR 2>/dev/null || echo "${TMPDIR:-/tmp}")"
before="$(find "$user_tmp" -mindepth 1 -maxdepth 1 -name 'pva-*' -print 2>/dev/null | sort || true)"
scratch="$(mktemp -d -t voice-agent-check)"
trap 'rm -rf "$scratch"' EXIT
export TMPDIR="$scratch/"

scripts/check-toolchain.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
# voice-attacks drives the real binary over pipes, so it must exist before the tests run.
cargo build --locked -p voice-session
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
    log="$scratch/swift-test.log"
    swift test --package-path apps/macos 2>&1 | tee "$log"
    scripts/swift-skips.sh "$log"
  else
    echo "skip: apps/macos needs macOS + Xcode (Swift build and tests not run here)"
  fi
fi
left="$(find "$scratch" -mindepth 1 -maxdepth 1 -name 'pva-*' -print | sed "s|^$scratch/||")"
after="$(find "$user_tmp" -mindepth 1 -maxdepth 1 -name 'pva-*' -print 2>/dev/null | sort || true)"
escaped="$(comm -13 <(printf '%s\n' "$before") <(printf '%s\n' "$after") | grep -v '^$' || true)"
if [ -n "$left$escaped" ]; then
  echo "FAIL: tests left data directories behind (they can hold transcripts):"
  [ -n "$left" ] && printf '  %s (removed now)\n' $left
  [ -n "$escaped" ] && printf '  %s (outside the private temp directory)\n' $escaped
  exit 1
fi
echo "check ok"
