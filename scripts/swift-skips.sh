#!/usr/bin/env bash
# Reads a `swift test` log and fails on any skip that is not an on-device one. A test may skip
# only when this Mac lacks the on-device model, speech assets or a system voice (its reason
# starts with "on-device:"); any other skip — say, a service-backed suite that could not find
# voice-session — would let the gate pass without running the proofs, so it fails instead.
set -euo pipefail
log="${1:?usage: scripts/swift-skips.sh <swift test log>}"
skips="$(grep -E '^[^ ]+ (Test|Suite) .* skipped: ' "$log" || true)"
expected="$(printf '%s\n' "$skips" | grep -c 'skipped: "on-device: ' || true)"
unexpected="$(printf '%s\n' "$skips" | grep -v 'skipped: "on-device: ' | grep -v '^$' || true)"
echo "swift: ${expected} on-device skip(s) (the model, speech assets or a system voice are not present)"
if [ -n "$unexpected" ]; then
  echo "FAIL: tests skipped for a reason other than the on-device model or assets:"
  printf '%s\n' "$unexpected"
  exit 1
fi
