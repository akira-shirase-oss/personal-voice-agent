#!/usr/bin/env bash
# Generates the committed recognition fixtures with macOS `say` (voice Samantha, 16 kHz mono
# 16-bit WAV). Output differs across macOS versions, so the committed files — not this
# script — are the source of truth; regenerate only on purpose, and re-record the
# recognition evaluation afterwards. Expected texts live in fixtures/audio/utterances.tsv.
set -euo pipefail
cd "$(dirname "$0")/.."
while IFS=$'\t' read -r name text; do
  [ -n "$name" ] && [ "${name:0:1}" != "#" ] || continue
  say -v Samantha -o "fixtures/audio/${name}.wav" --data-format=LEI16@16000 "$text"
  echo "fixtures/audio/${name}.wav"
done < fixtures/audio/utterances.tsv
