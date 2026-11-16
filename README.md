# Personal Voice Agent

> **Status — planning.** Documents only; no code yet. See `docs/ROADMAP.md`.

A voice interface that learns your style without taking uncontrolled actions
in your name. Push-to-talk conversations on macOS — a Swift/SwiftUI client
over AVAudioEngine, a Rust session service orchestrating speech, language and
voice models behind deadlines. Private conversation and draft responses are
the default; speaking or sending on your behalf is a separate, explicit
authorization boundary with disclosure to the recipient.

- `docs/END_GOAL.md` — what done means, scope, the four required failure
  cases, the ship gate, the evidence plan
- `docs/ARCHITECTURE.md` — sources of truth, turn / interrupt / revocation
  paths, repository layout
- `docs/ROADMAP.md` — V0 model → V1 audio → V2 conversation → V3
  personalization → V4 controlled tools + ship
- `docs/DECISIONS.md` — native client (no webview), Rust orchestration,
  evaluations over model hype, consent-gated voice profile, local-first
  private data
- `evals/` — utterance and voice evaluation sets (planned)
- `perf/` — turn/interrupt load scenarios + results (planned)

**What counts as done:** conversations are interruptible, memory is
controllable, and no external action happens without its required
permission — with the recorded evaluations, deletion tests and consent
records stored as evidence, not intent.
