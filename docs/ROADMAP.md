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
| **V0 — spine** | Rust workspace; `voice-domain` (reducer, ledger records, consent and authorization, consequential/ambiguous policy, recovery fold); `voice-session` (stdio JSON lines, session actor, deadlines, cancellation, crash-safe ledger); SwiftPM package that builds and tests; `scripts/check.sh`; CI | Every ambiguous or consequential proposal routes to confirmation; every invalid transition is named and refused; the reducer is pure and replayable; the five failure cases pass as automated tests; the domain has no I/O dependency (architecture check) | Planned |
| **V1 — audio** | `apps/macos`: push-to-talk capture (thin live path), bounded capture/playback buffers, on-device recognition from committed fixtures, synthesis to buffers, the turn-fenced player, transcript display | Deterministic playback tests (offline rendering); a stale chunk after interruption is first reproduced with a naive player, then proven unplayable; stop-playback measured against the <200 ms target; fixture recognition error recorded | Planned |
| **V2 — conversation** | The app runs `voice-session` as a child; FoundationModels streaming proposals; synthesis per sentence; interruption client→service; offline/degraded states; restart on service crash | A provider stall or failure hits its deadline and shows degraded — never an invented result; late chunks from an old turn are dropped; a spoken "done" appears only with tool-result evidence; end-of-turn → first audio p50/p95 measured on this Mac; million-scale turn events and ledger appends with p99 recorded | Planned |

**Owed to the maintainer for the ship gate** (a human with a microphone):
the live push-to-talk drill (`docs/DRILLS.md`), 100 consented utterances
quiet/noisy, and three one-hour sessions with stop-playback p95 and zero
stale chunks.

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
