#!/usr/bin/env bash
# Assembles build/VoiceAgent.app: the SwiftUI app, the session service next to it (the app
# finds it there), and an Info.plist with the microphone usage text. Ad-hoc signed, so the
# microphone permission sticks to this bundle. For personal use on this Mac only.
set -euo pipefail
cd "$(dirname "$0")/.."
if command -v brew >/dev/null && [ -d "$(brew --prefix rustup 2>/dev/null)/bin" ]; then export PATH="$(brew --prefix rustup)/bin:$PATH"; fi
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
cargo build --quiet --release --locked -p voice-session
swift build --quiet -c release --package-path apps/macos --product VoiceAgent
app=build/VoiceAgent.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp apps/macos/.build/release/VoiceAgent "$app/Contents/MacOS/VoiceAgent"
cp target/release/voice-session "$app/Contents/MacOS/voice-session"
cp apps/macos/Bundle/Info.plist "$app/Contents/Info.plist"
codesign --force --sign - "$app/Contents/MacOS/voice-session"
codesign --force --sign - "$app"
echo "built $app — open it with: open $app"
