# Roadmap

Step order follows the contract: model, audio, conversation, personalization,
controlled tools — each with its proof before the next starts. The guide's
four implementation steps map to V1–V4; the ship gate is part of V4.

| Step | Content | Proof gate | Status |
|---|---|---|---|
| **V0 — scaffold + model** | Workspace, docs, `voice-domain`: the turn/session state machine, heard/inferred/executed records, consent and permission states, limits; deterministic tests under a fake clock | Every ambiguous or consequential proposal routes to confirmation; every invalid transition is named and refused; the reducer is pure and replayable | Planned |
| **V1 — audio foundation** | `apps/macos`: push-to-talk capture, microphone permission handling, bounded capture/playback buffers, transcript display, deterministic playback tests | Deterministic playback tests pass; a stale audio chunk after interruption is first reproduced, then proven unplayable over a new turn; stop-playback delay measured against the proposed <200 ms target | Planned |
| **V2 — conversation** | `voice-session`: speech-to-text / language / voice adapters behind traits with per-call deadlines, streaming responses, interruption propagated client→service, the turn ledger, offline/degraded states | A provider stall or drop hits its deadline and surfaces degraded state — never an invented result; late chunks from an old turn are dropped; a spoken "done" appears only with tool-result evidence | Planned |
| **V3 — personalization** | `voice-consent` records with deletion scope, `voice-memory` reviewable/sourced/deletable entries, authorized voice evaluation; the personal-voice profile gated on consent + a usage-rights review | Revocation blocks the profile for all later turns and executes its deletion scope; memory edits and deletes are real erasure; no automatic external actions; the voice-evaluation set is recorded | Planned |
| **V4 — controlled tools + ship** | One reversible local tool, then reviewed external actions with an audit record; the evidence plan (100 consented utterances quiet/noisy, three one-hour sessions, p50/p95 + transcription error + provider cost; million-scale simulated turns and interrupt storms vs stubs) | An external action without its required permission is proven refused, not merely absent from the UI; the audit record is complete; the ship gate — interruptible conversations, controllable memory, no unpermissioned external action — is met with stored evidence | Planned |

## Later extensions (explicit contracts required)

Training a custom voice/language model, the iPhone companion, server-side
multi-device state (PostgreSQL). Each gets its own consent, data and
ownership contract before work starts — the availability of a voice-cloning
model is never assumed.
