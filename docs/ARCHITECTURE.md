# Architecture

## Topology (D-015, D-016)

```
┌──────────────────────────── VoiceAgent.app (Swift) ────────────────────────────┐
│ SwiftUI shell · AppKit key monitor (push-to-talk)                              │
│ AVAudioEngine capture ─▶ SpeechAnalyzer/SpeechTranscriber (on-device)          │
│ FoundationModels LanguageModelSession (on-device, guided generation)          │
│ AVSpeechSynthesizer.write ─▶ TurnFencedPlayer (AVAudioPlayerNode)              │
│        ▲ provider requests, cancel, stop_playback           │ results, events  │
│        │                                                    ▼                  │
│   stdin/stdout pipes, JSON lines (docs/PROTOCOL.md) — no socket, no network    │
└────────┼────────────────────────────────────────────────────┼──────────────────┘
         │                                                    │
┌────────┴─────────── voice-session (Rust child process) ─────┴──────────────────┐
│ stdin reader (bounded lines) ─▶ session actor ─▶ stdout writer (bounded)       │
│ session actor = voice-domain reducer + deadline timers + tool tasks            │
│ ledger writer thread: append-only, checksummed, fsync'd (group commit)          │
└────────────────────────────────────────────────────────────────────────────────┘
```

- The app launches `voice-session` as a **child process** and speaks
  versioned JSON lines over its stdin/stdout. There is no listening
  socket, so no other local process or web page can reach the service. The
  service exits when its stdin closes, so its lifetime is tied to the app.
- Apple's Speech, FoundationModels and AVFoundation APIs are Swift-only,
  so the **providers run in the app process**. The Rust service is still
  the orchestrator: it decides when each provider is called, gives every
  call a deadline, cancels calls, and decides what may be spoken or
  executed. A provider reply that arrives after its deadline or after
  cancellation is dropped and recorded as dropped.
- **Audio never crosses the process boundary.** Only text and control
  messages do. Audio lives in bounded buffers in the app and is never
  written to disk.

## Sources of truth

1. **Consent log** — `consent.log`, append-only, versioned, local
   (D-017). The authority on capture consent and on any enrolled voice,
   under what rights and with what deletion scope. Revocation takes effect
   in the same reducer step that records it.
2. **The turn ledger** — `ledger/<session>.log`, the session service's
   per-turn record, keeping three things separate: **heard** (the
   recognizer's text, verbatim, with its confidence), **inferred** (the
   model's proposal, with model and instructions version), **executed**
   (tool calls with their result evidence, or `uncertain`). The ledger,
   not the model's narration, is what happened.
3. **The approved-memory store** (V3) — the only future source of
   personalization: local, sourced, reviewable, editable, deletable.
4. **Audio is ephemeral** — bounded buffers only; session/turn IDs exist
   so stale chunks are identifiable and droppable.

### On-disk format (ledger and consent log)

One record per line: `<crc32 as 8 hex digits> <json>\n`, where the CRC
covers the JSON bytes exactly and the JSON is
`{"v":1,"seq":N,"at_ms":<unix ms>,"session":"…","rec":{…}}`.

- **Appends** go through one writer thread that drains its bounded
  queue, writes the batch, then `sync_data`s once (group commit) and acks.
  Records that must survive a crash before the next step (a tool call
  being sent, a consent change, an explicit `flush`) wait for that ack.
- **Recovery** on start: every record is verified. A torn final line
  (missing newline or bad CRC at the tail only) is truncated and recorded
  as `recovered_torn_tail`. So is a zero-filled hole followed only by lines
  of the same unsynced batch (at most 1 MiB, every sequence number after the
  last good one): what a power loss can leave, never acknowledged. Any other
  damage stops the service with an error instead of guessing. A session log without
  `session_closed` gets `turn_abandoned` for each open turn, `uncertain`
  for each tool call sent without a result, then `session_closed`.
- **Bounds:** a record is at most 64 KiB (the domain's content limits keep real records far smaller); a session log holds at most
  4,000,000 records, after which new turns are refused as degraded
  (`ledger_full`) rather than dropped silently.
- Files are created with mode 0600 inside a directory the app marks as
  excluded from backup. A data or ledger directory that already exists
  with group/other access is tightened to 0700 at open; a stricter mode
  is never loosened.

## Components

| Component | Role |
|---|---|
| `crates/voice-domain` | Pure model, no I/O (enforced by `scripts/check-architecture.sh`): the turn state machine as a reducer `step(input) → outputs`, ledger record types, consent and authorization states, the consequential/ambiguous policy, transcript normalization, recovery fold, limits. No clock: deadlines are outputs (`arm_deadline`) and come back as inputs. |
| `crates/voice-session` | The I/O shell and binary: stdio framing with bounded lines, the session actor, deadline timers, tracked tool tasks, the ledger/consent writer and recovery, the stub client used by tests and load runs. |
| `apps/macos` | SwiftPM package: `VoiceCore` (protocol types, session-process client, providers behind protocols with Apple and deterministic implementations, the turn-fenced player) and `VoiceAgent` (the SwiftUI app). |
| `voice-consent`, `voice-memory` | V3 crates (planned); consent stays in the domain + log until then. |

## Paths

**Turn:** push-to-talk down → service opens turn N and turns capture on →
app captures into a bounded buffer and streams it to the on-device
recognizer → push-to-talk up → service sends `stt_request` with a
deadline → app finalizes and returns the heard text and confidence
(**heard**) → service sends `lm_request` (heard text as quoted data, never
raw audio) → the model streams a structured proposal: its `kind` first,
then reply sentences → for a reply, each sentence goes through the
completion-claim guard (replaced, or spoken as written) and becomes a
`tts_request` tagged with turn N; a turn whose request may ask for an
action, or with a flagged sentence, ends with one spoken "Nothing has been
done." (D-027)
→ for an action, the policy decides: refuse, confirm (heard text shown
verbatim) or execute → a tool result is recorded (**executed**) → only a
successful result with evidence lets the service say "done".

**Interrupt** (a key press while a turn is active, or push-to-talk
during playback): the app stops its player locally first, then the
service records the interrupt, sends `stop_playback` with a fence of
N+1, cancels every pending call, and closes turn N as interrupted. The
player drops any chunk whose turn is below the fence. A tool call already
sent is marked `uncertain`; a later result for it is recorded as
reconciliation, never spoken.

**Consent revocation:** `consent` (revoke) → recorded durably → if a turn
is active: capture off, playback stopped, calls cancelled, turn closed as
`consent_revoked` → further push-to-talk is refused until consent is
granted again. The personal-voice scope cannot be granted without a
usage-rights review hash and is not usable in the first release.

**Provider failure:** every call has a deadline → on expiry, or on an
explicit failure from the provider, the turn closes as `degraded` with
the stage and reason, the app shows it, and the ledger records the
failure, not an answer.

**Service failure:** the app watches the child process; on an unexpected
exit it shows degraded, restarts it with bounded backoff, and the new
process runs recovery before it reports `ready`. A restart that cannot
launch says why (`binary_missing` at once, `launch_failed` once the restart
budget is spent) instead of showing "restarting" forever. A service that stops
answering without exiting is found by `ping`: once it is ready, every
second while a turn is open or the key is held, every five seconds between
turns, and on every key press; a ping unanswered for 3 s (30 s behind a
`forget`) stops the microphone and playback and replaces the service. Before
`ready` a service answers no ping, so one that has not said `ready` 30 s
after its launch (hung in startup or recovery) is replaced too.

## Consistency

Single-device: consent, ledger and (later) memory live on the Mac; there
is no server authority. The ledger records *that* a deletion or
revocation happened; the scope of deletion is described in END_GOAL
invariant 5 and D-018. PostgreSQL and a server enter only if multi-device
state becomes a real requirement.

## Layout

```
crates/voice-domain     pure model (no I/O dependencies)
crates/voice-session    stdio service, ledger, stub client, load runner
apps/macos/             SwiftPM package: VoiceCore + VoiceAgent app
docs/                   END_GOAL, ARCHITECTURE, PROTOCOL, ROADMAP, DECISIONS, DRILLS
evals/                  recognition fixtures' expected text; rights reviews (V3)
fixtures/               committed audio utterances, protocol golden lines
perf/                   load/latency runners' results with machine + commit
scripts/                check.sh, check-toolchain.sh, check-architecture.sh, fixture generation
```
