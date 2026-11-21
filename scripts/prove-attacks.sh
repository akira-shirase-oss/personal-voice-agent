#!/usr/bin/env bash
# Every protection the attack suite relies on is proven by a test that fails without it.
#
# Each protection in the source carries a marker, `// GUARD(name)`, on the line that
# enforces it. For every marker this script removes the protection in a scratch copy —
# one substitution per marked line — and runs the one test that proves it, which must
# then FAIL as a test (a build error is not a proof). A marker without a mutation below,
# or a mutation whose marker is gone, fails the script too, so a new protection cannot
# be added without its proof.
#
#   scripts/prove-attacks.sh [--out FILE]   (FILE: a JSON record; never names the machine)
#
# Builds into target/prove (this copy's own target directory); the scratch source is
# deleted at the end.
set -euo pipefail
case "${1:-}" in -h|--help) sed -n '2,/^set -euo/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'; exit 0 ;; esac
cd "$(dirname "$0")/.."
here="$PWD"
out=""
case "${1:-}" in
  "") ;;
  --out) out="${2:?--out needs a file}" ;;
  *) echo "prove-attacks.sh: unknown argument $1 (see --help)" >&2; exit 2 ;;
esac
export PATH="$(brew --prefix rustup)/bin:$PATH"

work="$(mktemp -d "${TMPDIR:-/tmp}/pva-prove.XXXXXX")"
trap 'rm -rf "$work"' EXIT
tar --exclude ./target --exclude ./.git -cf - . | tar -xf - -C "$work"
export CARGO_TARGET_DIR="$here/target/prove"

# name|file|pattern=>replacement (a Python regex, applied to every marked line)|package|test target|test
mutations=(
  # The wire gate.
  "protocol-version|crates/voice-session/src/lib.rs|protocol != PROTOCOL_VERSION=>false|voice-session|attack_handshake|a_wrong_protocol_version_is_refused_named_and_ended"
  "hello-required|crates/voice-session/src/lib.rs|self\.bad_line\(\"hello_required\", \"send hello first\"\)\.await\.is_err\(\)=>false|voice-session|attack_handshake|a_message_before_hello_is_refused_and_the_session_waits_for_hello"
  "handshake-deadline|crates/voice-session/src/lib.rs|return Err\(EndReason::HandshakeTimeout\);=>continue;|voice-session|attack_handshake|a_client_that_never_says_hello_is_ended_client_gone"
  "dup-hello|crates/voice-session/src/lib.rs|\.error\(\"duplicate_hello\", \"hello was already received\"\)=>.send(ServiceMessage::Pong { seq: 0 })|voice-session|attack_handshake|a_second_hello_is_an_error_not_a_rehandshake"
  # Input bounding.
  "line-cap|crates/voice-session/src/framing.rs|line\.len\(\) \+ take > max=>false|voice-session|attack_flooding|a_line_past_the_cap_is_discarded_and_the_next_line_reads"
  "bad-line-limit|crates/voice-session/src/lib.rs|self\.answered >= BAD_LINE_REPLIES_PER_S=>false|voice-session|attack_flooding|a_malformed_storm_is_rate_limited_and_the_session_survives"
  "malformed-count|crates/voice-session/src/lib.rs|self\.suppressed \+= 1=>self.unanswered += 0|voice-session|attack_flooding|a_malformed_storm_is_rate_limited_and_the_session_survives"
  # Identity: a call id stays bound to the turn that issued it.
  "call-bound|crates/voice-domain/src/session.rs|t\.phase != \(Phase::Transcribing \{ stt: call \}\)=>false|voice-session|attack_forgery|a_stt_result_for_a_foreign_call_is_late_dropped"
  "call-bound|crates/voice-domain/src/session.rs|t\.phase != \(Phase::Transcribing \{ stt: call \}\)=>false|voice-session|attack_replay|a_result_replayed_into_a_later_turn_is_late_dropped_never_heard"
  "late-drop|crates/voice-domain/src/session.rs|out\.push=>let _ =|voice-session|attack_forgery|a_result_for_a_call_never_made_is_dropped_and_ledgered"
  # Confirmation: a digest binds exactly one turn's pending action.
  "digest-match|crates/voice-domain/src/session.rs|pending != digest=>false|voice-testkit|replay|an_old_turns_digest_approves_nothing"
  "confirm-turn|crates/voice-domain/src/session.rs|t\.id != turn=>false|voice-testkit|replay|an_old_turns_digest_approves_nothing"
  "phase-consumes|crates/voice-domain/src/session.rs|t\.phase = Phase::Executing \{ tool: call \};=>let _ = call;|voice-testkit|replay|an_approval_replayed_twice_executes_once"
  "reject-closes|crates/voice-domain/src/session.rs|self\.close\(Outcome::Rejected, out\);=>let _ = out;|voice-testkit|replay|an_approval_replayed_after_a_reject_executes_nothing"
  "expire-window|crates/voice-domain/src/session.rs|self\.close\(Outcome::Expired, out\);=>let _ = out;|voice-testkit|replay|an_approval_replayed_after_expiry_executes_nothing"
  # Model output is data: the guard between it and the synthesizer, and the registry
  # between a proposal and a tool call.
  "claim-guard|crates/voice-domain/src/session.rs|guard::classify\(&spoken\)=>guard::Verdict::Plain|voice-session|attack_injection|a_model_claiming_done_never_speaks_without_evidence"
  "unknown-tool|crates/voice-domain/src/policy.rs|return Decision::Refuse=>return Decision::Execute; let _ = Decision::Refuse|voice-session|attack_forgery|a_forged_action_proposal_is_refused_named_and_never_executed"
  "external-refused|crates/voice-domain/src/policy.rs|spec\.effect == ToolEffect::External=>false|voice-testkit|replay|an_external_action_is_refused_never_confirmed"
  # The store's path defenses.
  "nofollow|crates/voice-session/src/store.rs|libc::O_NOFOLLOW=>0|voice-session|attack_store_paths|a_symlinked_consent_log_is_refused_not_followed"
  "dir-follow|crates/voice-session/src/store.rs|libc::O_NOFOLLOW . if meta\.is_dir\(\) \{ libc::O_DIRECTORY \} else \{ 0 \}=>0|voice-session|attack_store_paths|a_symlinked_ledger_directory_is_refused_not_followed"
  "single-instance|crates/voice-session/src/store.rs|lock\.try_lock\(\)=>Ok::<(), fs::TryLockError>(())|voice-session|attack_store_paths|a_second_service_on_the_same_directory_is_refused"
  # The end is honest: a dead writer and a silent EOF are the client gone, never clean.
  "writer-dead|crates/voice-session/src/lib.rs|\.is_err\(\) \{=>.is_ok() {|voice-session|client_gone|a_dead_reader_ends_the_session_as_client_gone"
  "eof-gone|crates/voice-session/src/lib.rs|return EndReason::Eof=>return EndReason::Shutdown|voice-testkit|service|closing_stdin_mid_turn_closes_the_turn_honestly"
)

# Every marker in the source has exactly one mutation, and every mutation a marker.
found="$(grep -rhoE 'GUARD\([a-z-]+\)' crates/*/src | sed 's/GUARD(\(.*\))/\1/' | sort -u)"
defined="$(printf '%s\n' "${mutations[@]}" | cut -d'|' -f1 | sort -u)"
if [ "$found" != "$defined" ]; then
  echo "error: markers and mutations differ:" >&2
  diff <(echo "$found") <(echo "$defined") >&2 || true
  exit 1
fi

mutate() { # file marker pattern replacement
  python3 -I - "$work/$1" "$2" "$3" "$4" <<'PY'
import re, sys
path, marker, pattern, replacement = sys.argv[1:]
lines = open(path).read().split("\n")
tag = f"GUARD({marker})"
changed = 0
for i, line in enumerate(lines):
    if tag not in line:
        continue
    # The marker sits on its target's line, or on its own line between the `if`/`match`
    # header and the body (rustfmt moves trailing comments off `{`-openers). The target
    # is whichever of those lines the pattern actually changes.
    for target in (i, i - 1, i + 1, i - 2, i + 2, i - 3, i + 3):
        if not (0 <= target < len(lines)):
            continue
        new = re.sub(pattern, replacement, lines[target], count=1)
        if new != lines[target]:
            lines[target] = new
            changed += 1
            break
if not changed:
    sys.exit(f"the mutation for {marker} changed nothing")
open(path, "w").write("\n".join(lines))
PY
}

failures=0
results=()
for row in "${mutations[@]}"; do
  IFS='|' read -r name file subst package target test <<<"$row"
  pattern="${subst%%=>*}"
  replacement="${subst#*=>}"
  cp "$here/$file" "$work/$file"
  mutate "$file" "$name" "$pattern" "$replacement"
  if [ "$target" = lib ]; then selector=(--lib); else selector=(--test "$target"); fi
  started=$(date +%s)
  set +e
  output="$(cd "$work" && cargo test --locked -q -p "$package" "${selector[@]}" -- --exact "$test" 2>&1)"
  status=$?
  set -e
  seconds=$(( $(date +%s) - started ))
  cp "$here/$file" "$work/$file"
  if [ "$status" -ne 0 ] && grep -q 'test result: FAILED' <<<"$output"; then
    verdict=proven
    printf '  proven   %-22s %s fails without it (%ss)\n' "$name" "$test" "$seconds"
  elif [ "$status" -ne 0 ]; then
    verdict=broken
    failures=$((failures + 1))
    printf '  BROKEN   %-22s the mutation did not build or the test did not run:\n%s\n' "$name" "$(tail -20 <<<"$output")"
  else
    verdict=unproven
    failures=$((failures + 1))
    printf '  UNPROVEN %-22s %s passes without the protection\n' "$name" "$test"
  fi
  results+=("{\"attack\":\"$name\",\"test\":\"$test\",\"verdict\":\"$verdict\",\"seconds\":$seconds}")
done

if [ -n "$out" ]; then
  commit="$(git rev-parse HEAD 2>/dev/null || echo unknown)"
  dirty=false; git diff --quiet HEAD -- . 2>/dev/null || dirty=true
  load="$(uptime | sed 's/.*load averages*: *//')"
  printf '{"proof":"attacks","commit":"%s","tree_dirty":%s,"load_average":"%s","os":"%s","results":[%s]}\n' \
    "$commit" "$dirty" "$load" "$(uname -sr)" "$(IFS=,; echo "${results[*]}")" >"$out"
  echo "record: $out"
fi
if (( failures )); then
  echo "error: $failures of ${#mutations[@]} protections are not proven" >&2
  exit 1
fi
echo "attacks ok: all ${#mutations[@]} protections are proven by a test that fails without them"
