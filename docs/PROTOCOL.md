# App ↔ session service protocol (v1)

The app launches `voice-session --data-dir <dir>` and exchanges **JSON
lines** over the child's stdin (app → service) and stdout (service →
app). stderr carries the service's own logs: codes, IDs, counts and
durations only, never content (D-011).

- One JSON object per line, UTF-8, terminated by `\n`, with a `"type"`.
- A line longer than **65,536 bytes** is rejected (`error`
  `line_too_long`) and discarded up to its newline.
- Unknown `type` or malformed JSON → `error` `bad_message`; the session
  continues. At most 8 malformed lines a second are answered; the rest are
  counted, and the next answer's `detail` says how many went unanswered. A
  flood of them never delays a deadline: a due timer is served before the
  next inbound line.
- The first app message must be `hello`; anything else first is
  `error` `hello_required`.
- stdin EOF ends the session as `client_gone` (`session_closed` `clean` is reserved for an
  app that sent `shutdown` first — a stream that just stops is the app gone).
- A client the service cannot reach ends the session from the service side: a message that cannot
  be delivered inside the I/O deadline (the app's reader dead or stalled — a dead stdout reader
  EPIPEs the writer task), a stdin read error, or no `hello` in time all end the session as
  `client_unresponsive`, and `session_closed` records `client_gone` — never `clean`, which is
  reserved for "the app chose to end". A refused handshake records `refused`; a service that died
  before it could close is closed `crashed` by the next start's recovery. The session does not
  linger headless: it holds the data-dir lock, so staying alive would refuse a restarted app
  forever. When the service exits, the app's `.exited` event arrives, the session is restarted,
  and any turn in flight is abandoned (recovered at the next open).
- A service that cannot open its data directory sends one `error` before exiting:
  `already_running` (another service holds the directory's lock; exit code 6),
  `store_corrupt` (exit 3) or `store_io` (exit 4). The app does not restart it for the last
  two. The lock holder writes its process ID into `<data-dir>/lock`; on `already_running` the
  app reads it, stops a stale orphan of its own (the same `voice-session` binary whose parent
  app has died, so its parent is launchd: SIGTERM, then SIGKILL) and starts again, and
  otherwise waits for the other copy, retrying after 250 ms, doubling to at most 5 s, for as
  long as it takes.
- The app keeps the user's capture choice itself. Before it sends a revocation it removes the
  service's consent token (`<data-dir>/consent.capture.granted`: unlinked, or set to 0000), so
  a service that never reads the message is replaced by one that starts revoked. While the
  choice is "off" the app sends no `ptt_down` and opens no microphone, and a `ready` or
  `consent_state` that says capture is granted is answered with the revocation again.
- On quit the app sends `shutdown` and closes stdin, then sends SIGTERM if the service has
  not exited within 1 s and SIGKILL 0.5 s later, and quits only once it has exited.
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
| `forget` | `scope` | Carry out that consent scope's deletion scope now (capture: every heard transcript); refused while a turn is open. Every log is deleted before the fresh one is created (no free space needed); answered by `forgotten`, or by `error` `forget_failed` if anything could not be deleted |
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
| `notice` | `code`, `turn` (or null) | A refused request (e.g. `consent_required`, `invalid_transition`), or `nothing_done`: the turn may have claimed an action and ended (interrupted, degraded, refused) before it could say nothing was done; the app shows it with the turn |
| `flushed` | `seq` | Every record up to `seq` is durable |
| `executed` | `turn`, `call`, `status` (`succeeded`/`failed`/`uncertain`) | A tool call of this turn has a result on record (the ledger's `executed`); what the app shows as executed |
| `pong` | `seq` | Answer to `ping` with the same `seq` |
| `forgotten` | `scope`, `files`, `bytes` | A `forget` was carried out |
| `error` | `code`, `detail` | Protocol-level error; `detail` never contains content |

Outcomes: `completed`, `interrupted`, `degraded`, `refused`, `rejected`,
`expired`, `consent_revoked`, `empty`, `abandoned`.

Confirmation `reasons`: `irreversible_effect`, `sensitive_slot`,
`low_confidence`, `no_confidence`, `slot_not_heard`. There is no external-effect
reason: an external tool is refused before any confirmation (`external_not_authorized`),
because no external action can be authorized in the first release.

## Ordering guarantees

- For a turn, the service sends `capture on` only after `turn_opened`, and
  every provider request for turn N before `turn_closed` for N.
- `stop_playback` for an interrupted turn is sent before the
  `turn_closed` that records the interruption, and before `turn_opened`
  of a following turn.
- After `cancel`, replies for that `call` are dropped (and recorded as
  dropped); they never change the turn.
