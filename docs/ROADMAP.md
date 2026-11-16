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
| **V0 — spine** | Rust workspace; `voice-domain` (reducer, ledger records, consent and authorization, consequential/ambiguous policy, recovery fold); `voice-session` (stdio JSON lines, session actor, deadlines, cancellation, crash-safe ledger); SwiftPM package that builds and tests; `scripts/check.sh`; CI | Every ambiguous or consequential proposal routes to confirmation; every invalid transition is named and refused; the reducer is pure and replayable; the five failure cases pass as automated tests; the domain has no I/O dependency (architecture check) | **Done** — `crates/voice-domain/tests/session.rs` (21 tests incl. 100,000 randomized steps checking invariants and replay), `tests/protocol.rs`, 22 unit tests; `crates/voice-testkit/tests/service.rs` (11: stall, interrupt, consent revoked, ambiguous expires, adversarial, hung tool, content-free logs, protocol garbage); `crates/voice-session/tests/crash_restart.rs` (SIGKILL + torn tail + restart, file modes); `scripts/check-architecture.sh` (negative-tested); CI green on ubuntu and macos-26. Load: smoke runs in `perf/results/` — **full 1,000,000-unit runs owed** (load average 25–160 all session; rule: below ~6) |
| **V1 — audio** | `apps/macos`: push-to-talk capture (thin live path), bounded capture/playback buffers, on-device recognition from committed fixtures, synthesis to buffers, the turn-fenced player | Deterministic playback tests (offline rendering); a stale audio chunk after interruption is first reproduced with a naive player, then proven unplayable; stop-playback measured against the <200 ms target; fixture recognition error recorded | **Done for everything automatable** — `Tests/VoiceCoreTests/PlayerTests.swift` (8: reproduction, proof, monotonic fence, offline stop = 0 samples after the call, stop-call timing, 100 rounds × 8 threads stress, bounds, played callbacks), `SpeechTests.swift` (recognition on fixtures, round trip, overrun, cancel; synthesis, cancel); `evals/recognition/2026-11-16-171708.md` (quiet WER 0.042, 10 dB noise 0.021, synthetic speech). Load: interrupt storm `perf/results/2026-11-16-174339-player-storm-cycles100000.md` — 100,000 interrupts, 0 late chunks accepted, silent afterwards, memory flat, stop() p50 8 µs / p99 1.9 ms at load 50. **Owed:** the live stop figure on the real output device (`docs/DRILLS.md` Drill 2 — needs a person) |
| **V2 — conversation** | The app runs `voice-session` as a child; FoundationModels streaming proposals; synthesis per sentence; interruption client→service; offline/degraded states; restart on service crash | A provider stall or failure hits its deadline and surfaces degraded — never an invented result; late chunks from an old turn are dropped; a spoken "done" appears only with tool-result evidence; end-of-turn → first audio p50/p95 measured on this Mac; million-scale turn events and ledger appends with p99 recorded | **Done for everything automatable** — `ConversationTests.swift` against the real binary (reply turn, stalled model degraded at its deadline with nothing played, interrupt mid-utterance, consent revoked mid-turn, SIGKILL + restart + recovery, action refused) and `ModelTests` on this Mac (FoundationModels stream order; the whole on-device path end to end); "done" only with evidence is proven in the domain and service tests (no tools exist in this release); latency `perf/results/2026-11-16-173730-e2e-latency-rounds3.md`: end → first audio **p50 1,877 ms / p95 2,203 ms — misses the proposed 2,000 ms p95 budget** (`docs/LATENCY.md`; the model's first sentence is the miss). **Owed:** live drills 1–6, full load runs |

**Owed to the maintainer for the ship gate** (a human with a microphone):
the live drills (`docs/DRILLS.md`: push-to-talk, the live stop figure, barge-in, revocation,
crash, model unavailable), 100 consented utterances quiet/noisy, and three one-hour
sessions with stop-playback p95 and zero stale chunks.

**Owed to a quiet machine:** `scripts/perf.sh ledger --records 1000000` and
`scripts/perf.sh turns --events 1000000 --interrupt-every 10 --stall-every 100`.

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
