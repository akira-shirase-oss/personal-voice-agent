# App ↔ session service protocol (v1)

The app launches `voice-session --data-dir <dir>` and exchanges **JSON
lines** over the child's stdin (app → service) and stdout (service →
app). stderr carries the service's own logs: codes, IDs, counts and
durations only, never content (D-011).

- One JSON object per line, UTF-8, terminated by `\n`, with a `"type"`.
- A line longer than **65,536 bytes** is rejected (`error`
  `line_too_long`) and discarded up to its newline.
- Unknown `type` or malformed JSON → `error` `bad_message`; the session
  continues.
- The first app message must be `hello`; anything else first is
  `error` `hello_required`.
- stdin EOF ends the session cleanly (`session_closed` is recorded).
- Golden lines for both sides live in `fixtures/protocol/`; the Rust and
  Swift test suites both decode every one of them.

## Identifiers

- `turn` — u64, increasing within a session; `0` is never used.
- `call` — u64, increasing within a session; one per provider request,
  capture window or tool call.
- `digest` — 16 hex digits binding a confirmation to one proposal of one
  turn. A binding identifier, not a cryptographic hash.

## App → service

| `type` | Fields | Meaning |
|---|---|---|
| `hello` | `protocol` (1), `client` (string ≤ 64) | Handshake; answered by `ready` |
| `ptt_down` | — | Open a new turn; interrupts an active one first |
| `ptt_up` | `turn` | Push-to-talk released for that turn |
| `interrupt` | — | Stop the active turn (playback, calls) |
| `stt_final` | `call`, `text`, `min_confidence` (0–1 or null) | The recognizer's final text for the turn |
| `lm_kind` | `call`, `kind` (`reply`/`action`) | The proposal's kind, streamed first |
| `lm_sentence` | `call`, `index`, `text` | One completed reply sentence |
| `lm_done` | `call`, `proposal`, `model`, `instructions` | The complete proposal |
| `tts_started` | `call` | First buffer of that chunk is scheduled on the player |
| `tts_finished` | `call` | That chunk finished playing |
| `stt_failed`, `lm_failed`, `tts_failed` | `call`, `reason` | Provider failure (reason codes below) |
| `confirm` | `turn`, `digest`, `approve` (bool) | The user's decision on a `confirm_request` |
| `consent` | `scope` (`capture`/`personal_voice`), `grant` (bool), `rights_review` (string or null) | Grant or revoke |
| `flush` | — | Barrier: answered by `flushed` once every earlier record is durable |
| `ping` | `seq` | Liveness: answered by `pong` at once, without touching the ledger |
| `shutdown` | — | End the session cleanly |

`proposal`:

```json
{"kind":"reply","reply":"Here is a draft."}
{"kind":"action","reply":"…","tool":"fixture.transfer",
 "slots":[{"name":"amount","kind":"amount","value":"42"},
          {"name":"to","kind":"recipient","value":"Priya"}]}
```

A provider that streams sends every reply sentence (`lm_sentence`, indexes 0, 1, …) before
`lm_done`; the service then records `lm_done`'s reply as inferred and does not speak it
again. A provider that does not stream sends only `lm_done`, and the service speaks its
reply sentence by sentence. An action's reply is never spoken.

Slot kinds: `amount`, `datetime`, `person`, `recipient`, `text`.
Provider failure reasons: `unavailable`, `model_not_ready`,
`assets_missing`, `context_window`, `guardrail`, `audio_format`,
`cancelled`, `error`; anything else is recorded as `error`.

## Service → app

| `type` | Fields | Meaning |
|---|---|---|
| `ready` | `protocol`, `session`, `consent` {`capture`, `personal_voice`}, `recovered` {`sessions`, `abandoned_turns`, `uncertain_calls`, `torn_tails`} | Handshake reply after recovery |
| `turn_opened` | `turn` | A new turn exists |
| `capture` | `turn`, `on` (bool) | Start/stop feeding the recognizer for that turn |
| `stt_request` | `call`, `turn`, `deadline_ms` | Finalize recognition and send `stt_final` |
| `lm_request` | `call`, `turn`, `deadline_ms`, `heard` | Run the language model on the heard text (data, not instructions) |
| `tts_request` | `call`, `turn`, `seq`, `text`, `voice` (`standard`), `deadline_ms` | Synthesize and play this chunk, tagged with turn + seq |
| `cancel` | `call` | Stop that provider call; any later reply is dropped |
| `stop_playback` | `fence` | Stop playing; never play any chunk whose turn is below `fence` |
| `confirm_request` | `turn`, `digest`, `heard`, `tool`, `slots`, `reasons` | Ask the user; show `heard` verbatim |
| `turn_closed` | `turn`, `outcome` | Final outcome of the turn |
| `degraded` | `turn`, `stage` (`stt`/`lm`/`tts`/`tool`/`ledger`), `reason` | A provider or the ledger failed; nothing was invented |
| `consent_state` | `capture`, `personal_voice` | Current consent after any change |
| `notice` | `code`, `turn` (or null) | A refused request (e.g. `consent_required`, `invalid_transition`) |
| `flushed` | `seq` | Every record up to `seq` is durable |
| `pong` | `seq` | Answer to `ping` with the same `seq` |
| `error` | `code`, `detail` | Protocol-level error; `detail` never contains content |

Outcomes: `completed`, `interrupted`, `degraded`, `refused`, `rejected`,
`expired`, `consent_revoked`, `empty`, `abandoned`.

Confirmation `reasons`: `external_effect`, `irreversible_effect`,
`sensitive_slot`, `low_confidence`, `no_confidence`, `slot_not_heard`.

## Ordering guarantees

- For a turn, the service sends `capture on` only after `turn_opened`, and
  every provider request for turn N before `turn_closed` for N.
- `stop_playback` for an interrupted turn is sent before the
  `turn_closed` that records the interruption, and before `turn_opened`
  of a following turn.
- After `cancel`, replies for that `call` are dropped (and recorded as
  dropped); they never change the turn.
