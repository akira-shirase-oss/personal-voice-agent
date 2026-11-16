#!/usr/bin/env bash
# Runs a load scenario (release build) and records it under perf/results/ with the machine,
# OS, commit and load average before and after. The file is written whether the run passes
# or fails: a miss is evidence, not something to rerun until green.
#
#   scripts/perf.sh ledger --records 1000000
#   scripts/perf.sh turns --events 1000000 --interrupt-every 10 --stall-every 100
set -euo pipefail
cd "$(dirname "$0")/.."
scenario="${1:?usage: scripts/perf.sh ledger|turns [args]}"
shift

commit="$(git rev-parse --short HEAD)$([ -z "$(git status --porcelain -- crates Cargo.toml Cargo.lock)" ] || echo '-dirty')"
load_before="$(uptime | sed 's/.*load average[s]*: //')"
cargo build --quiet --release --locked -p voice-testkit --bin voice-perf
set +e
out="$(target/release/voice-perf "$scenario" "$@")"
status=$?
set -e
load_after="$(uptime | sed 's/.*load average[s]*: //')"

if [ "$(uname -s)" = "Darwin" ]; then
  machine="$(sysctl -n machdep.cpu.brand_string), $(($(sysctl -n hw.memsize) / 1073741824)) GB, $(sysctl -n hw.ncpu) cores"
  os="macOS $(sw_vers -productVersion) ($(sw_vers -buildVersion))"
else
  machine="$(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2 | xargs), $(nproc) cores"
  os="$(uname -sr)"
fi
size="$(echo "$@" | grep -oE '[0-9]+' | head -1 || true)"
file="perf/results/$(date +%Y-%m-%d-%H%M%S)-${scenario}-${size:-default}.md"
result=PASS
[ "$status" -eq 0 ] || result=FAIL
cat > "$file" <<MD
# ${scenario} — ${result}

- Date: $(date -u +%Y-%m-%dT%H:%M:%SZ)
- Machine: ${machine}
- OS: ${os}
- Commit: ${commit}
- Load average before: ${load_before}
- Load average after: ${load_after}
- Command: \`scripts/perf.sh ${scenario} $*\`

Laptop numbers are development observations on a shared machine, not capacity claims.

\`\`\`json
${out}
\`\`\`
MD
echo "$out"
echo "recorded: $file ($result)"
exit "$status"
