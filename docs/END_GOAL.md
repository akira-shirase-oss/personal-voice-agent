# Personal Voice Agent — end goal

**A voice interface that learns your style without taking uncontrolled
actions in your name.**

A personal assistant on Apple platforms: push-to-talk conversations with
interruption, a transcript that separates what was heard from what was
inferred from what executed, bounded tool actions — and, only under explicit
recorded consent, a voice profile that speaks like you. The default posture
is private conversation and draft responses; speaking or sending on your
behalf is a separate, explicit authorization boundary with disclosure to the
recipient.

## In scope (the first useful release)

- Push-to-talk conversation with interruption: bounded audio capture,
  streaming responses, a live transcript, clear errors, a **standard voice**.
- A turn ledger separating what the recognizer heard, what the assistant
  inferred, and what actually executed — a spoken "done" requires tool-result
  evidence, never a model prediction.
- Editable long-term memory: user-reviewable, sourced, deletable; no
  recording retention unless explicitly enabled.
- One reversible local tool; then reviewed external actions with an audit
  record.
- Consent records with deletion scope for any enrolled voice; an explicitly
  authorized **personal-voice profile** added only after the pipeline works
  and the model's usage rights are understood.

## Out of scope (exclusions are contract, not backlog)

- Covert recording, voice authentication, unapproved outbound calls, a fully
  autonomous identity substitute — **excluded by the guide; never features.**
- Training a custom voice or language model — a separate later experiment.
  The availability of a voice-cloning model is **not assumed**; existing
  speech-to-text, language and voice-synthesis models come first, chosen by
  recorded evaluations, rights and latency.
- iPhone companion — deferred; macOS is the proposed first target.
- Server-side multi-device state — PostgreSQL enters only if that requirement
  becomes real.

## Non-negotiable invariants

1. **Consent before a profile.** Enroll only your own voice or a speaker with
   explicit rights; consent and deletion scope are recorded. Revocation stops
   capture/playback and prevents any further use of the revoked profile.
2. **Heard ≠ inferred ≠ executed.** The transcript keeps the recognizer's
   output, the assistant's inference and tool-result evidence separate.
   Ambiguous or consequential requests are confirmed against the preserved
   original transcript.
3. **Interrupt means stop.** Bounded buffers and session/turn IDs; on
   interruption playback stops and downstream work is cancelled where
   possible; a late text/audio chunk from an old turn can never play over a
   new one; actions already sent keep an honest `uncertain` state — never a
   fabricated result.
4. **Actions need their permission.** Retrieved text, audio and tool results
   are data, never instructions: adversarial input cannot elevate permissions
   or invoke a tool silently. External actions carry their explicit
   authorization plus recipient disclosure of synthetic speech, and produce
   an audit record.
5. **Memory is controllable.** User-reviewable, sourced and deletable;
   recordings encrypted at rest with working deletion controls; model keys in
   the system credential store or server secret manager; private speech is
   not a training dataset by default.
6. **Bounded everything.** Every provider call has a deadline; a stall or
   dropped connection surfaces an offline/degraded state — the system stops
   waiting and says so, it never invents an execution result.

## Required failure cases

| Trigger | Required result |
|---|---|
| Recognition mistakes an amount/name | Request confirmation before a consequential action; preserve the original transcript |
| Provider stalls or connection drops | Stop waiting at the deadline, show offline/degraded state, never invent an execution result |
| User interrupts or revokes consent | Discard stale turn output, stop capture/playback, prevent further use of the revoked profile |
| Adversarial audio/tool result | Retrieved text/audio cannot elevate permissions or silently invoke tools |

## Ship gate

Ship for personal use when **conversations are interruptible, memory is
controllable, and no external action happens without its required
permission** — and the recorded evaluations, data-deletion tests and consent
records exist as stored evidence.

## Evidence plan

Per the guide: **100 consented utterances** in quiet and noisy conditions and
**three one-hour sessions**. Measure end-of-turn→first-audio p50/p95,
transcription error, interruption delay and total provider cost. Proposed
local stop-playback target: **under 200 ms** after an interrupt event —
network/model latency is reported separately, and no universal voice-latency
claim is made. CPU, audio glitches and memory are profiled on the actual
target Mac.

Per the workspace load standard, full runs are million-scale in the
equivalent unit — **turns and interrupt events through the session service**
— executed against stub providers so the run is deterministic and
cost-bounded. Real-provider evaluation volume is bounded by cost and is a
quality measurement, not a load substitute. Results commit with machine +
commit; misses are kept, not rerun until green. Laptop numbers are
development observations, never production capacity.
