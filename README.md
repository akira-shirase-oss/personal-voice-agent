# Personal Voice Agent

> **Status — first release (V0–V2) built and tested on this Mac; not yet
> shippable.** Four review rounds' findings are fixed with tests,
> except those `docs/ROADMAP.md` lists as owed. The live microphone drills,
> the human evaluation set, the million-scale load runs and a long soak are
> owed. Two measured misses stand: the on-device model misses the latency
> budget at p95 (2,036 ms against 2,000), and both ledger smokes recorded at
> `d8b78cc` failed their thresholds (saturated append p99 554 ms against
> 50 ms; one durable append p99 155 ms against 20 ms) on a disk another
> process was driving. The completion-claim guard's sentence classifier
> misses most claims it was not tuned on (83% of a held-out set), so every
> reply turn that may claim an action ends with "Nothing has been done.";
> on the held-out set 5 of 70 claims still got through, all inside answers
> to plain questions. `docs/ROADMAP.md` lists what is done, with its evidence, and
> what is owed.

A voice interface that learns your style without taking uncontrolled
actions in your name. Push-to-talk conversations on macOS: a Swift/SwiftUI
app hosting Apple's on-device speech recognition, language model and
speech synthesis, and a Rust session service — launched as the app's child
process — that owns orchestration, deadlines and a turn ledger separating
what was heard, inferred and executed. Private conversation and draft
responses are the default; speaking or sending on your behalf is a
separate, explicit authorization boundary, closed in the first release.

Nothing leaves the Mac: no cloud providers, no API keys, no retained
audio.

- `docs/END_GOAL.md` — the first release, definitions, failure cases, ship
  gate, evidence plan
- `docs/ARCHITECTURE.md` — topology, sources of truth, on-disk format,
  turn / interrupt / revocation paths
- `docs/PROTOCOL.md` — the app ↔ service stdio protocol
- `docs/ROADMAP.md` — V0 spine → V1 audio → V2 conversation (first
  release); V3 memory + personal voice, V4 controlled tools (later)
- `docs/DECISIONS.md` — native client, on-device Apple providers, child
  process over stdio, append-only local records, honest deletion scope

## Try it

```
scripts/check.sh                 # Rust + Swift gates (what CI runs)
scripts/bundle-app.sh            # build/VoiceAgent.app with the service inside
open build/VoiceAgent.app        # hold Space to talk, Esc to stop
scripts/bench-e2e.sh --rounds 3  # end-to-end latency on this Mac, recorded
scripts/perf.sh turns --events 1000000   # service load run, recorded
```

`docs/DRILLS.md` walks through the live checks a person has to do.

## Requirements

macOS 26 or later (Speech `SpeechAnalyzer`, FoundationModels), Xcode with
Swift 6.2+, Rust 1.99.1 via rustup. FoundationModels needs Apple
Intelligence enabled; without it the app shows a degraded state rather
than pretending.

## License

MIT — see `LICENSE`.
