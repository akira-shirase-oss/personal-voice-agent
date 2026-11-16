#!/usr/bin/env bash
# Runs the end-to-end latency bench (apps/macos VoiceBench) and records it under
# perf/results/ with machine, OS, commit and load average, pass or fail. Needs macOS with
# Apple Intelligence on and the en-US speech assets installed. Nothing plays on the speakers.
#
#   scripts/bench-e2e.sh --rounds 3          # warm (prewarmed model, warmed voice)
#   scripts/bench-e2e.sh --rounds 1 --cold
set -euo pipefail
cd "$(dirname "$0")/.."
# Homebrew cargo ignores rust-toolchain.toml; use rustup's proxies when present.
if command -v brew >/dev/null && [ -d "$(brew --prefix rustup 2>/dev/null)/bin" ]; then export PATH="$(brew --prefix rustup)/bin:$PATH"; fi
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
commit="$(git rev-parse --short HEAD)$([ -z "$(git status --porcelain -- apps crates Cargo.lock)" ] || echo '-dirty')"
cargo build --quiet --release --locked -p voice-session
swift build --quiet -c release --package-path apps/macos --product VoiceBench
load_before="$(uptime | sed 's/.*load average[s]*: //')"
set +e
out="$(PVA_SESSION_BINARY="$PWD/target/release/voice-session" apps/macos/.build/release/VoiceBench "$@")"
status=$?
set -e
load_after="$(uptime | sed 's/.*load average[s]*: //')"
label="$(echo "$*" | tr -d ' -' )"
file="perf/results/$(date +%Y-%m-%d-%H%M%S)-e2e-latency-${label:-default}.md"
result=PASS
[ "$status" -eq 0 ] || result=FAIL
cat > "$file" <<MD
# End-to-end latency on this Mac — ${result}

- Date: $(date -u +%Y-%m-%dT%H:%M:%SZ)
- Machine: $(sysctl -n machdep.cpu.brand_string), $(($(sysctl -n hw.memsize) / 1073741824)) GB, $(sysctl -n hw.ncpu) cores
- OS: macOS $(sw_vers -productVersion) ($(sw_vers -buildVersion))
- Commit: ${commit}
- Load average before: ${load_before}
- Load average after: ${load_after}
- Command: \`scripts/bench-e2e.sh $*\`
- Providers: SpeechAnalyzer/SpeechTranscriber, FoundationModels, AVSpeechSynthesizer (all on-device)
- Audio: committed synthetic fixtures fed at real time in 100 ms slices; playback rendered offline
  at real time. Device output latency is not included (live figure: docs/DRILLS.md).

"End to first audio" is push-to-talk release → first synthesized buffer scheduled on the
player; the first turn of a run is reported separately (cold model). A shared, busy laptop:
development observations, not a latency claim.

\`\`\`json
${out}
\`\`\`
MD
echo "$out" | python3 -c 'import json,sys; d=json.load(sys.stdin); print(json.dumps({k: d[k] for k in ("first_turn_ms","steady_state","outcomes","interrupt_drill")}, indent=1))' || true
echo "recorded: $file ($result)"
exit "$status"
