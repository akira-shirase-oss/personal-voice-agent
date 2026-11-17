#!/usr/bin/env bash
# CI's Swift step. Picks the newest Xcode on the runner and builds and tests apps/macos only
# when its macOS SDK is 26 or newer (SpeechAnalyzer, FoundationModels need it). Otherwise it
# says so and exits 0: a skip, written to the job summary as a skip, never as a pass.
# Tests that need an on-device model or speech assets skip themselves with an "on-device:"
# reason; any other skip fails the step (scripts/swift-skips.sh).
set -euo pipefail
cd "$(dirname "$0")/.."

note() {
  echo "$1"
  if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then echo "$1" >> "$GITHUB_STEP_SUMMARY"; fi
}

xcode="$(ls -d /Applications/Xcode*.app 2>/dev/null | sort -V | tail -1 || true)"
if [ -z "$xcode" ]; then
  note "SKIPPED: no Xcode on this runner; apps/macos was not built or tested."
  exit 0
fi
export DEVELOPER_DIR="$xcode/Contents/Developer"
sdk="$(xcrun --sdk macosx --show-sdk-version)"
if [ "${sdk%%.*}" -lt 26 ]; then
  note "SKIPPED: $(basename "$xcode") has the macOS $sdk SDK; apps/macos needs 26 or newer. Not built or tested."
  exit 0
fi
note "Swift: $(basename "$xcode"), macOS SDK $sdk, $(sw_vers -productName) $(sw_vers -productVersion)"
target_dir="$(cargo metadata --format-version 1 --no-deps --locked | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
export PVA_SESSION_BINARY="$target_dir/debug/voice-session"
swift build --package-path apps/macos
swift test --package-path apps/macos 2>&1 | tee swift-test.log
if ! summary="$(scripts/swift-skips.sh swift-test.log)"; then
  note "$summary"
  exit 1
fi
note "Swift tests passed; $summary"
