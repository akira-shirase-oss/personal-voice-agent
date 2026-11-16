# Personal Voice Agent

> **Status — building the first release (V0–V2).** See `docs/ROADMAP.md`
> for what is done, with its evidence, and what is owed.

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

## Requirements

macOS 26 or later (Speech `SpeechAnalyzer`, FoundationModels), Xcode with
Swift 6.2+, Rust 1.99.1 via rustup. FoundationModels needs Apple
Intelligence enabled; without it the app shows a degraded state rather
than pretending.

## License

MIT — see `LICENSE`.
