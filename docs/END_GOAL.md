# Personal Voice Agent — end goal

**A voice interface that learns your style without taking uncontrolled
actions in your name.**

A personal assistant on macOS: push-to-talk conversations with
interruption, a transcript that separates what was heard from what was
inferred from what executed, and — in later, separately contracted
releases — reviewed memory, bounded tool actions and a consented voice
profile. The default posture is private conversation and draft
responses; speaking or sending on your behalf is a separate, explicit
authorization boundary with disclosure to the recipient.

All speech recognition, language and speech synthesis run **on the Mac**
through Apple's frameworks (D-014). No audio, transcript or prompt
leaves the machine, and there are no provider keys.

## The first useful release (V0–V2)

The guide's first release, and nothing more (D-012):

- Push-to-talk conversation with interruption: bounded audio capture,
  streaming responses, a live transcript, clear errors and a
  **standard system voice**.
- A turn ledger separating what the recognizer heard, what the assistant
  inferred and what actually executed. A spoken "done" requires
  tool-result evidence, never a model prediction.
- Offline/degraded states when an on-device model is unavailable,
  stalls or fails — never an invented answer.
- The consent and authorization model the later releases depend on:
  capture consent, the personal-voice gate (closed in this release) and
  the external-action boundary (closed in this release).

## Later contracts (not in the first release)

Each starts only after the first release passes its gate, with its own
proof gate in `ROADMAP.md`:

- **V3 — memory and personal voice.** Editable long-term memory
  (user-reviewable, sourced, deletable) and, only after a recorded
  usage-rights review (D-020), a consented personal-voice profile.
- **V4 — controlled tools.** One reversible local tool, then reviewed
  external actions with an audit record. The V4 contract must name the
  external channels (email, messages, …) before any is built; outbound
  calls are not in scope.

## Out of scope (exclusions are contract, not backlog)

- Covert recording, voice authentication, unapproved outbound calls, a
  fully autonomous identity substitute — **excluded by the guide; never
  features.**
- Training a custom voice or language model — a separate later
  experiment. The availability of a voice-cloning model is **not
  assumed**.
- Cloud speech, language or voice providers (D-014).
- Apple Personal Voice in the first release: its terms limit it to
  personal, non-commercial use, and it belongs to V3's rights review.
- iPhone companion — deferred; macOS is the first target.
- Server-side multi-device state — PostgreSQL enters only if that
  requirement becomes real.

## Definitions the gates depend on (D-013)

- **Consequential** — a proposed action whose tool has an external
  effect or an irreversible local effect, **or** that carries any slot of
  type amount, date/time, person or recipient.
- **Ambiguous** — the recognizer's lowest per-segment confidence for the
  turn is below the threshold (0.80 initially), **or** the recognizer
  reported no confidence, **or** any slot value is not literally present
  in the heard transcript after normalization (case, punctuation,
  currency symbols, digit grouping, English number words → digits).
- **Rule** — every consequential or ambiguous action proposal goes to
  confirmation, and the confirmation shows the preserved heard
  transcript verbatim. A confirmation binds to the proposal's digest; a
  confirmation for any other digest or turn is refused. An unconfirmed
  proposal expires; it never executes.
- **External actions** are refused in the first release even when
  confirmed: no external authorization can be granted before V4.

## Non-negotiable invariants

1. **Consent before capture and before any profile.** Capture happens only
   while push-to-talk is held and capture consent is granted, and the app
   enforces it itself: the key coming up stops the microphone at once
   (before any reply from the service), a capture that would start after
   the key is up never starts, capture has its own 60 s cap, and a service
   that stops answering is detected and replaced with the microphone off.
   Enroll only your own voice or a speaker with explicit rights; consent and
   deletion scope are recorded. Revocation stops capture/playback
   immediately and prevents any further use of the revoked scope, also when
   the disk is full (it fails closed), with only one service per data
   directory, and when the service never reads it (hung or dying): the app
   removes the consent token itself before sending the revocation, refuses
   push-to-talk while listening is off, and revokes again if a replacement
   service reports capture granted.
2. **Heard ≠ inferred ≠ executed.** Separate ledger records for the
   recognizer's output, the assistant's inference and tool-result
   evidence. The ledger is what happened; the model's narration is not. A
   reply is drafted text, and while a turn has no executed evidence the
   guard fails closed at the turn level: a reply turn whose request may ask
   for an action, or with any sentence that may claim one, ends with one
   spoken "Nothing has been done.", and a sentence that clearly says an
   action happened is also replaced by a draft disclaimer. The sentence
   classifier alone misses most claims it was not tuned on (83% — 58 of
   70 claims on the held-out set, D-027); with the turn-level disclosure, every held-out
   claim answering a request ended disclosed (a reply cut off early is
   disclosed in the record and the window), and 5 of 8 claims made inside
   answers to plain questions were not. What was spoken is recorded too.
3. **Interrupt means stop.** Bounded buffers and session/turn IDs; on
   interruption playback stops and downstream work is cancelled; a late
   text/audio chunk from an old turn can never play over a new one; an
   action already sent keeps an honest `uncertain` state and is
   reconciled from later evidence — never reported done or undone.
4. **Actions need their permission.** Transcripts, model output and tool
   results are data, never instructions: nothing they contain can elevate
   permissions or invoke a tool without the policy's decision.
5. **Data stays local and its deletion is described honestly (D-018).**
   No audio is retained. The ledger and consent log are local files
   (mode 0600, excluded from Time Machine). Heard transcripts are kept 30
   days by default, and "Delete history" removes every one at once, and
   clears the model's in-memory recap of the last two exchanges with them. Either
   deletion removes them from the live file system; copies can survive in APFS local snapshots
   or backups made before the exclusion, until those expire — the docs
   never claim more. V3 adds per-item encryption with keys in the
   Keychain so that deleting an item destroys its key ("unrecoverable
   after key destruction", a claim a test can check). No third-party
   provider holds audio or transcripts in this design.
6. **Bounded everything.** Every provider call has a deadline (D-019);
   every queue, line and buffer has a bound; a stall or failure surfaces
   an offline/degraded state — the system stops waiting and says so.

## Required failure cases (automated tests, not intentions)

| Trigger | Required result |
|---|---|
| Recognition mistakes an amount/name | Confirmation before a consequential action, heard transcript shown verbatim; the original transcript preserved in the ledger |
| Provider stalls or connection drops | Stop waiting at the deadline, show offline/degraded state, never invent an execution result |
| User interrupts or revokes consent mid-turn | Discard stale turn output, stop capture/playback, prevent further use of the revoked scope |
| Adversarial audio/tool result | Retrieved text/audio cannot elevate permissions or silently invoke tools |
| Session service crashes | Restart without losing durable ledger records; open turns are closed as abandoned; sent tool calls become `uncertain` |

## Ship gate (first release)

Ship V0–V2 for personal use when **conversations are interruptible**
(stop-playback p95 measured, zero stale chunks played across the soak
sessions), **every failure case above has a passing automated test**,
and **no action executes without its required permission** — with the
evidence stored under `perf/results/` and `evals/`.

## Evidence plan

- **Stop-playback**, defined for measurement (D-021): from the interrupt
  event's timestamp to the last non-silent sample rendered by the output
  node. Proposed target **under 200 ms** — a target to measure against,
  never a claim. Measured deterministically by offline rendering in tests
  and on the live device in the manual drill.
- **End-of-turn → first audio** p50/p95, split by stage (recognizer
  finalize, model first sentence, synthesis first buffer), on this Mac.
- **Transcription error** on committed fixture utterances now, and on
  **100 consented utterances** (quiet and noisy) plus **three one-hour
  sessions** recorded by the maintainer — owed to a human with a
  microphone.
- **Load:** the session service runs million-scale simulated turn events
  and ledger appends against stub providers (cheap regressions with p99
  recorded). For a single-user local app the evidence that matters is the
  audio soak and interrupt storms; the million-scale runs do not replace
  them.
- Results commit with machine + commit. Misses are kept, not rerun until
  green. Laptop numbers are development observations; no universal
  voice-latency claim is made. There is no provider cost: every model is
  on-device.
