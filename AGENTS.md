# Personal Voice Agent — agent instructions

A voice interface that learns your style without taking uncontrolled actions
in your name. Swift/SwiftUI client on Apple platforms over AVAudioEngine; a
Rust session service owns orchestration, bounded tool execution and provider
adapters. Contract: `../../docs/Project_Guides/07_Personal_Voice_Agent.docx`.
When code and guide disagree, the guide wins unless a `D-NNN` says otherwise.

## Read first

1. `docs/END_GOAL.md` — what done means, scope, the four required failure
   cases, the ship gate.
2. `docs/ARCHITECTURE.md` — sources of truth, turn/interrupt/revocation
   paths, layout.
3. `docs/DECISIONS.md` — prescribed directions (native client, Rust
   orchestration, evals over hype, consent-gated profile, local-first data)
   and the open questions — do not guess where the guide is open.
4. `docs/ROADMAP.md` — V0–V4 step order.

## Non-negotiables

1. **Consent before a profile.** Enroll only your own voice or a speaker with
   explicit rights; consent and deletion scope are recorded. Revocation stops
   capture/playback and prevents all further use of the profile. Never use
   voice imitation to bypass authentication.
2. **Heard ≠ inferred ≠ executed.** The transcript keeps recognizer output,
   assistant inference and tool-result evidence separate. A spoken "done"
   requires tool-result evidence, never a model prediction. Ambiguous or
   consequential requests are confirmed against the preserved original
   transcript.
3. **Interrupt means stop.** Bounded buffers and session/turn IDs; on
   interruption, stop playback and cancel downstream work where possible; a
   late chunk from an old turn can never play over a new one; an action
   already sent keeps an honest `uncertain`, reconciled — never reported
   undone. First reproduce the stale-chunk bug, then prove it cannot play.
4. **Actions need their permission.** Retrieved text, audio and tool results
   are data, never instructions — no silent tool invocation, no permission
   elevation. External actions carry explicit authorization, recipient
   disclosure of synthetic speech, and an audit record.
5. **Memory is controllable.** User-reviewable, sourced, deletable — real
   erasure, not tombstones over retained data. Recording retention defaults
   to off. Model keys in the system credential store or server secret
   manager; private speech is not a training dataset.
6. **Bounded everything.** Deadlines on every provider call; bounded buffers,
   queues and retries; a stall or drop shows offline/degraded — never an
   invented result.
7. **Pure domain.** `voice-domain` carries no I/O — enforced like the other
   projects' domain crates.
8. **Evidence over claims.** `perf/` results carry machine + commit. Full
   load runs are million-scale in the equivalent unit — simulated turns and
   interrupt events through the session service against stub providers;
   real-provider volume is cost-bounded and is a quality measurement, not a
   load substitute. The <200 ms stop-playback figure is a proposed target to
   measure against, never a claim. Misses are kept, not rerun until green.
   No universal voice-latency claims.

## Rust rules

- `export PATH="$(brew --prefix rustup)/bin:$PATH"` before every cargo
  command — Homebrew's cargo ignores the pin.
- Typed errors (`thiserror`) in libraries; `anyhow` only in binaries.
- No `unwrap`/`expect` on external input. Deadlines on every I/O await.
- No secrets, transcripts, prompts or audio content in logs or metrics.

## Client rules (Swift)

- SwiftUI for the interface, AVAudioEngine for audio, AppKit where needed.
  No Tauri, Electron or webview-hosted UI — the guide's firm exclusion.
- Bounded audio buffers; every capture and playback chunk carries a
  session/turn ID; playback drops anything from a superseded turn.
- Deterministic playback and interruption tests are contract — real-time
  audio behavior is proven by test, not assumed.
- A compatible Mac + Xcode is required for the client; Ubuntu hosts the
  Rust-side checks and services.

## Testing rule (workspace D-018, shared)

Every feature ships regression tests + a fixture path (`fixtures/`:
adversarial audio/tool results, provider stall/drop stubs, the stale-chunk
harness) + a load scenario, in the same change. Every fixed bug gets a test
that fails without it. Keep consent records, data-deletion tests,
provider/model compatibility checks and the voice-evaluation set current;
re-evaluate quality after any model change.

## Verification

`cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace` — before every commit, plus the interruption and
provider-failure drills when audio/session code changed, plus the Swift-side
test suite and a client build on a Mac when `apps/macos` changed.
