# Architecture

## Sources of truth

1. **Consent records** — the authority on whose voice is enrolled, under what
   rights, with what deletion scope. A revoked consent takes effect
   immediately: no later turn may use the profile, and deletion scope is
   executed against its recordings and derived data.
2. **The turn ledger** — the session service's per-turn record, keeping three
   things separate: **heard** (the recognizer's output; the original
   transcript is preserved), **inferred** (the assistant's interpretation and
   proposals), **executed** (tool calls with their result evidence). The
   ledger, not the model's narration, is what happened.
3. **The approved-memory store** — the only source of personalization:
   local, sourced, user-reviewable, editable and deletable. Raw recordings
   are encrypted, retention is off by default, and no recording becomes
   memory without review.
4. **Audio is ephemeral** — bounded buffers only. Nothing in audio memory is
   authoritative; session/turn IDs exist precisely so stale chunks are
   identifiable and droppable.

## Components

| Component | Role |
|---|---|
| `voice-domain` | Pure model: turn/session state machine, heard/inferred/executed records, consent and permission states, limits. No I/O — enforced. |
| `voice-session` | Rust orchestration service: conversation state, provider adapters behind traits (speech-to-text, language, voice synthesis), per-call deadlines, cancellation propagation, bounded tool execution, audit record. |
| `voice-consent` | Consent records, voice-profile enrollment, revocation and deletion-scope execution. |
| `voice-memory` | Local approved-memory store: sourced, reviewable, editable, deletable entries; retention policy enforcement. |
| `apps/macos` | Swift + SwiftUI client: AVAudioEngine capture/playback, push-to-talk, interruption, transcript and confirmation UI, memory review, consent flows. AppKit where needed. |

## Paths

**Turn:** push-to-talk → new turn ID → bounded capture → speech-to-text →
transcript (**heard**) → context selection (transcript + approved memory —
never raw audio, never unreviewed data) → language model → proposal: response
text or tool action → if consequential or ambiguous, confirmation against the
preserved original transcript → approved tool executes under a deadline →
result evidence (**executed**) → voice synthesis → playback chunks tagged
with the turn ID → optional, reviewed memory write.

**Interrupt:** interrupt event → playback stops (proposed local target
<200 ms, measured separately from network/model latency) → in-flight
provider and tool work cancelled where the API allows → any later chunk
carrying an older turn ID is dropped, never played → a tool call already
sent is marked `uncertain` and reconciled — never reported done or undone
without evidence.

**Consent and revocation:** enroll your own voice or a speaker with explicit
rights → record consent + deletion scope → the personal-voice profile is
enabled only under that record, and only after the pipeline works and the
model's usage rights are understood → on revocation: capture/playback stop,
the profile is blocked for all future turns, and the deletion scope executes
against its recordings and derived data.

**Provider failure:** every adapter call carries a deadline → stall or
dropped connection → the turn is marked offline/degraded → the UI shows it →
the ledger records the failure, not a synthesized answer.

## Consistency

Single-device first: consent, memory and ledger live on the Mac; there is no
server authority to disagree with. The ledger is an ordered record per
session — it records *that* a deletion or revocation happened, while the
deleted bytes themselves are gone: deletion controls are real erasure, not
tombstones over retained data. PostgreSQL and a server-side store enter only
if multi-device state becomes a real requirement; that decision, and where
the session service deploys (local process vs. a home server), are open —
see `DECISIONS.md`.

## Failure cases

See `END_GOAL.md` — recognition mistakes, provider stall/drop, interrupt or
consent revocation, adversarial audio/tool result. These are contract, not
options.

## Layout

```
crates/        voice-domain, voice-session, voice-consent, voice-memory
apps/macos/    Swift + SwiftUI client (requires a compatible Mac + Xcode;
               Ubuntu hosts the Rust-side checks and services)
docs/          END_GOAL, ARCHITECTURE, ROADMAP, DECISIONS
evals/         consented utterance set (quiet/noisy), voice evaluation set,
               recorded provider evaluations
fixtures/      adversarial audio/tool-result fixtures, provider stall/drop
               stubs, stale-chunk reproduction harness
perf/          turn + interrupt-storm scenarios vs stub providers, results
               with machine + commit
```
