# Roadmap

Step order follows the contract: model, audio, conversation — the first
release (D-012) — then personalization and controlled tools as later
contracts. Each step has its proof before the next starts.

**Status rule.** A cell may say **Done** only when it names the tests and
the evidence path that prove it. Anything not yet proven is listed as
**owed**, with who or what it needs.

## First release

| Step | Content | Proof gate | Status |
|---|---|---|---|
| **V0 — spine** | Rust workspace; `voice-domain` (reducer, ledger records, consent and authorization, consequential/ambiguous policy, completion-claim rule, recovery fold); `voice-session` (stdio JSON lines, session actor, deadlines, cancellation, crash-safe ledger, one instance per data directory, fail-closed consent, deletion and retention); SwiftPM package; `scripts/check.sh`; CI | Every ambiguous or consequential proposal routes to confirmation; every invalid transition is named and refused; the reducer is pure and replayable; the failure cases pass as automated tests; the domain has no I/O dependency | **Done** — `crates/voice-domain/tests/session.rs` (randomized invariants incl. no capture or speech without consent, replay), `tests/claims.rs` (334-phrasing corpus, every sentence verdict pinned), `tests/held_out.rs` (92 held-out phrasings never used for tuning: every claim answering a request ends with the turn-level disclosure; the sentence classifier alone misses 83%, D-027), `tests/protocol.rs`; `crates/voice-testkit/tests/service.rs` (stall, interrupt, revocation, ambiguous expiry, adversarial input, hung tool, content-free logs, claims never spoken bare (replaced or disclosed), spoken text ledgered, oversized proposal, deletion, retention); `crates/voice-session/tests/crash_restart.rs` (SIGKILL + torn tail), `disk_full.rs` (revocation and "Delete history" on full HFS+ and APFS images; macOS only, so it runs locally, not in the Linux CI job), `single_instance.rs`; store tests (torn tails, zero-filled holes, batch bounds, consent tokens, directory sync). Load: smokes only, and **both ledger smokes at `d8b78cc` failed their thresholds**: `perf/results/2026-11-16-205704-ledger-50000.md` (saturated append-to-durable p99 554 ms against 50 ms) and `…205803-ledger-sync-2000.md` (one durable append p99 155 ms against 20 ms), at 1-minute load 26–38 with another process driving the disk; run alternately with the old build under the same conditions (`…210000-ledger-ab-main-vs-branch.md`) both builds measured 7–19 ms durable p99, which reads as a contended disk rather than a slower writer, but that is a reading, not a pass. The turns smoke (`…205809-turns-100000.md`) passed. **Full 1,000,000-unit runs owed** (1-minute load 15–65 all session; rule: below ~6) |
| **V1 — audio** | `apps/macos`: push-to-talk capture (thin live path), bounded buffers, on-device recognition from committed fixtures, synthesis to buffers, the turn-fenced player | Deterministic playback tests; a stale chunk after interruption first reproduced, then proven unplayable; stop-playback measured against the <200 ms target; fixture recognition error recorded | **Done for everything automatable** — `PlayerTests.swift` (naive reproduction, fenced proof, session-epoch fence, the lock rule tested by a hook that lands a stop between the fence check and the schedule (it caught the scheduling-outside-the-lock mutation, but only when the stop thread runs within the test's 50 ms window, which a loaded machine makes likely, not certain), offline stop 0 samples after the call, bounds); `SpeechTests.swift`; `evals/recognition/2026-11-16-210114.md` (quiet WER 0.042, 10 dB 0.021, synthetic speech, clean tree); interrupt storm `perf/results/2026-11-16-210104-player-storm-cycles100000.md`. **Owed:** the live stop figure on the output device (Drill 2) |
| **V2 — conversation** | The app runs `voice-session` as a child; FoundationModels streaming proposals; synthesis per sentence; interruption client→service; offline/degraded states; restart on service crash | A provider stall or failure hits its deadline and shows degraded; late chunks from an old turn are dropped; a spoken "done" appears only with tool-result evidence; end-of-turn → first audio p50/p95 measured; million-scale runs recorded | **Done for everything automatable** — `ConversationTests.swift` against the real binary: reply turn; stalled model degraded with nothing played; interrupt mid-utterance; consent revoked mid-turn; SIGKILL mid-turn, restart, recovery **and the next turn completes and is heard**; quick tap never opens the microphone; local capture cap; hung service (SIGSTOP) detected during capture with capture stopped, between turns with no key pressed, and on a key press before the microphone opens; a second copy is refused, waits with backoff and takes over when the first quits (the running copy is never killed); quitting ends even a hung service before the app exits; a stale orphan holding the lock is stopped and replaced; recognizer released on interrupt; a restart that cannot launch (binary missing or not executable) is reported; lost events reported; `SessionProcessTests.swift`: 50 crashes while sending, the app survives every one; overflow ends the session. On this Mac: the on-device path end to end with long deadlines (fails if the model path is broken). Latency `perf/results/2026-11-16-210413-e2e-latency-rounds3.md` (clean tree): end → first audio **p50 1,549 / p95 2,036 ms — misses the proposed 2,000 ms p95 budget** (`docs/LATENCY.md`; the model). **Owed:** live drills, full load runs, a long soak (bench memory 41.8 → 48.9 MB over three rounds; service steady at ~3 MB) |

**Owed to the maintainer for the ship gate** (a human with a microphone): the live drills
(`docs/DRILLS.md`: push-to-talk, the live stop figure, barge-in echo, revocation, crash with the
next turn spoken, model unavailable, the fastest tap, a hung service while holding the key,
delete history), 100 consented utterances quiet/noisy, and three one-hour sessions with
stop-playback p95 and zero stale chunks.

**Known residual (D-027):** a claim made inside an answer to a plain informational question is
caught only by the sentence classifier (5 of 8 such held-out claims got through).

**Owed in code:** one shared audio engine for capture and playback, so echo cancellation can
reference the assistant's output (D-023 limitation); the model's first-sentence latency
(`docs/LATENCY.md`).

**Owed to a dispatched CI run** (`.github/workflows/load.yml`, ubuntu; its Linux fsync
numbers are that platform's, not the Mac's): `scripts/perf.sh ledger --records 1000000`
and `scripts/perf.sh turns --events 1000000 --interrupt-every 10 --stall-every 100`.
**Still owed to this Mac when quiet:** a long `scripts/bench-e2e.sh` soak to settle the
memory growth seen over three rounds.

## Later contracts

| Step | Content | Proof gate | Status |
|---|---|---|---|
| **V3 — memory and personal voice** | `voice-memory`: reviewable, sourced, deletable entries with per-item keys in the Keychain (crypto-shredding); the usage-rights review artifact (D-020); a personal-voice profile gated on consent + that review | Revocation blocks the profile for all later turns; deleting an item destroys its key and the item is unrecoverable (tested); no automatic external actions; the voice-evaluation set is recorded | Not started |
| **V4 — controlled tools** | One reversible local tool, then reviewed external actions on channels the V4 contract names, with recipient disclosure and an audit record | An external action without its required permission is proven refused, not merely absent from the UI; the audit record is complete | Not started |

## Later extensions (explicit contracts required)

Training a custom voice/language model, the iPhone companion, server-side
multi-device state (PostgreSQL). Each gets its own consent, data and
ownership contract before work starts — the availability of a
voice-cloning model is never assumed.
