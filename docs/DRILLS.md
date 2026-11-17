# Manual drills

Some behaviour needs a person and real hardware: macOS permission prompts cannot be granted
by an agent, and live microphone and speaker timing is not reproducible in CI. These drills
cover exactly the thin live paths the automated tests cannot reach — microphone capture,
the output device, key events — and record the live stop-playback figure (D-021).

Record each run as `perf/results/<date>-live-drill.md` with the date, machine, macOS version,
commit, `uptime` load average, output device (built-in speakers, headphones, Bluetooth), and
the numbers asked for below. A failed step is recorded as failed, with what happened.

## Setup

1. `scripts/bundle-app.sh`, then `open build/VoiceAgent.app`.
2. System Settings → Apple Intelligence & Siri: Apple Intelligence on (the app's status line
   shows "On-device model ready"; otherwise it shows why and turns degrade honestly).
3. If the status line offers **Download speech model**, click it once (an on-device download).
4. Click **Allow microphone** and accept the system prompt.
5. Turn on **Listen while I hold Space**.

## Drill 1 — a turn, end to end

Hold Space, say "What is the capital of France?", release.

- Expect: **Heard** shows your words verbatim; **Draft** fills in as the reply streams; the
  system voice speaks; the turn shows `completed` and `first audio N ms`.
- Record: `first audio` for 10 turns of your own choosing (p50, max).

## Drill 2 — interrupt mid-utterance (the stop-playback figure)

Say "Tell me a long story about a lighthouse." While it speaks, press **Escape**.

- Expect: speech stops at once; the turn shows `interrupted`; the talk bar shows
  `last stop X ms rendered + Y ms device`.
- `X` is from the Escape key event's timestamp to the last non-silent sample the mixer
  rendered; `Y` is the output device's reported presentation latency. The proposed target
  is X + Y under 200 ms — a target, not a claim.
- Record: 20 interrupts (p50, p95, max of X and of X + Y), and whether any speech from the
  interrupted turn was heard afterwards (it must not be).

## Drill 3 — barge-in with push-to-talk

While a reply is playing, hold Space and say something new.

- Expect: the reply stops before the microphone opens; the new turn's **Heard** contains only
  your words, none of the assistant's.
- Record: 5 tries; any echo of the assistant in **Heard** is a failure.

## Drill 4 — revoke consent mid-turn

While a reply is playing, switch **Listen while I hold Space** off.

- Expect: speech stops, the turn shows `consent revoked`, and holding Space is refused with
  the banner asking you to turn listening on.

## Drill 5 — the service crashes

Ask something, and while the reply is being prepared run
`pkill -9 -f 'VoiceAgent.app/Contents/MacOS/voice-session'` in Terminal.

- Expect: the app stays up (it must never die with the service), the status line shows the
  session restarting, then ready, with a banner saying an unfinished turn was closed, and the
  next turn is spoken.

## Drill 6 — the model is unavailable

Turn Apple Intelligence off, then ask something.

- Expect: the status line says why the model is unavailable, the turn shows a degraded
  reason, and nothing is spoken as if it were an answer.

## Drill 8 — the fastest tap

Tap Space as quickly as you can, ten times.

- Expect: the menu-bar microphone indicator goes off at once every time (it may not come on at
  all); no turn shows text you did not say; nothing is recorded after the key is up.

## Drill 9 — hold, then a hung service

Hold Space and, while holding, run `pkill -STOP -f 'VoiceAgent.app/Contents/MacOS/voice-session'`.

- Expect: within about 3 seconds the microphone indicator goes off although Space is still
  held, the app says the session stopped responding, and a new session becomes ready (the
  stopped service is killed by the app; no `pkill -CONT` is needed).
- Then, between turns: run the same `pkill -STOP` with nothing held and wait. Within about
  8 seconds the app says the session stopped responding and restarts it. Repeat, and this time
  press Space within a second of the `pkill`: the talk bar shows "Waiting for the session…",
  the microphone indicator never comes on, and within about 3 seconds the same banner appears.

## Drill 10 — delete history

Talk for a few turns, then click **Delete history**.

- Expect: a banner says how many session logs were removed; `grep -r` for a phrase you said
  finds nothing under `~/Library/Application Support/PersonalVoiceAgent`.

## Drill 7 — the 100 consented utterances and three one-hour sessions (ship gate)

The guide's evidence plan, owed to a person: 100 utterances of your own in quiet and noisy
conditions (recorded only with your consent, and kept only as the evaluation set), and three
one-hour sessions with Drill 2 repeated throughout. Record end-of-turn → first-audio p50/p95,
transcription error against what you said, the stop figure from Drill 2, and any stale
speech heard after an interrupt (the gate needs zero).
