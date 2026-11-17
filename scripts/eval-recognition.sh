#!/usr/bin/env bash
# Runs the recognition evaluation on the committed fixtures with the on-device recognizer
# and records it under evals/recognition/ with machine, OS, commit and model locale.
# The fixtures are synthetic (macOS `say`), so this is a regression measurement of the
# pipeline, not the guide's 100 consented human utterances (owed: docs/ROADMAP.md).
set -euo pipefail
cd "$(dirname "$0")/.."
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
tmp="$(mktemp -d -t voice-agent-eval)"
trap 'rm -rf "$tmp"' EXIT
out="$tmp/eval.json"
if [ -n "$(git status --porcelain -- apps fixtures)" ] && [ "${PVA_ALLOW_DIRTY:-}" != 1 ]; then
  echo "refusing to record from a dirty tree: commit first, so the result names the code it measured" >&2
  echo "(PVA_ALLOW_DIRTY=1 records anyway, marked -dirty)" >&2
  exit 2
fi
commit="$(git rev-parse --short HEAD)$([ -z "$(git status --porcelain -- apps fixtures)" ] || echo '-dirty')"
PVA_EVAL_OUT="$out" swift test --package-path apps/macos --filter "RecognitionTests/fixtures" >/dev/null
file="evals/recognition/$(date +%Y-%m-%d-%H%M%S).md"
python3 - "$out" "$file" "$commit" <<'PY'
import json, subprocess, sys
data = json.load(open(sys.argv[1]))
sh = lambda c: subprocess.run(c, shell=True, capture_output=True, text=True).stdout.strip()
lines = [
    "# Recognition evaluation — committed synthetic fixtures",
    "",
    f"- Date: {sh('date -u +%Y-%m-%dT%H:%M:%SZ')}",
    f"- Machine: {sh('sysctl -n machdep.cpu.brand_string')}, {int(sh('sysctl -n hw.memsize'))//2**30} GB",
    f"- OS: macOS {sh('sw_vers -productVersion')} ({sh('sw_vers -buildVersion')})",
    f"- Commit: {sys.argv[3]}",
    f"- Load average (1 / 5 / 15 min): {sh('uptime').split('load average')[-1].lstrip('s: ').replace(',', '')}",
    "- Recognizer: Speech SpeechAnalyzer + SpeechTranscriber, en-US, on-device",
    "- Audio: fixtures/audio/*.wav (macOS say, voice Samantha, 16 kHz); noisy = white noise at 10 dB SNR, seeded",
    "",
    f"**Word error rate: quiet {data['quiet_wer']:.3f}, noisy (10 dB) {data['noisy_10db_wer']:.3f}** over {int(data['words'])} words.",
    "WER is computed on normalized tokens (lowercase, punctuation dropped, $N → N dollars, number words → digits).",
    "",
    "| Fixture | Quiet WER | Quiet min conf. | Noisy WER | Noisy min conf. | Heard (quiet) |",
    "|---|---|---|---|---|---|",
]
for u in data["utterances"]:
    lines.append(f"| {u['name']} | {u['quiet_wer']:.3f} | {u['quiet_min_confidence']:.3f} | {u['noisy_wer']:.3f} | {u['noisy_min_confidence']:.3f} | {u['quiet']} |")
lines += ["", "Synthetic speech is far easier than a person in a room; these numbers say the pipeline works, not how well it hears you."]
open(sys.argv[2], "w").write("\n".join(lines) + "\n")
print(sys.argv[2])
PY
