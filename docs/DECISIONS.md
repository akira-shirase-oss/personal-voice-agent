# Decisions

Deliberate choices, newest last. D-NNN numbering continues the workspace
convention; these seed the contract's prescribed directions. Where the guide
leaves a choice open, it is listed under **Open questions** rather than
guessed.

- **D-001 — Native Apple client, no webview.** Swift + SwiftUI with
  AVAudioEngine for audio and AppKit where needed; no Tauri, Electron or
  webview-hosted interface (the guide's firm exclusion). macOS first is the
  guide's *proposed* target — treated as committed for the first release.
  Trade-off: single-platform effort requiring a compatible Mac + Xcode; buys
  real control over capture, buffering and interruption semantics that a
  webview cannot guarantee.
- **D-002 — Rust session service owns orchestration.** Conversation state,
  bounded tool execution and provider adapters live in a Rust service, not in
  the Swift app. Trade-off: an extra process boundary and protocol; buys a
  deterministic, testable core, one enforcement point for permissions and
  deadlines, and a place secrets can live off the client.
- **D-003 — Existing models first, chosen by recorded evaluation.**
  Speech-to-text, language and voice-synthesis models are selected on
  recorded evaluations, usage rights and latency — not on reputation.
  Training is a separate later experiment; the availability of a
  voice-cloning model is not assumed.
- **D-004 — Standard voice before personal voice.** The first useful release
  ships a standard voice. A personal-voice profile is added only after the
  pipeline works *and* the model's usage rights are understood — and only
  under an explicit consent record. Trade-off: the headline feature arrives
  late; the alternative is an uncontrolled impersonation surface, which the
  guide excludes.
- **D-005 — Drafting ≠ acting.** Private conversation and draft responses are
  the default posture. Speaking or sending on your behalf is a separate,
  explicit authorization boundary; synthetic speech is disclosed to
  recipients when acting externally, and voice imitation is never used to
  bypass authentication.
- **D-006 — Local-first private data.** Preferences and approved memory live
  locally; recordings are encrypted at rest with working deletion controls,
  and recording retention defaults to off. PostgreSQL enters only if
  server-side multi-device state becomes a real requirement — it is not
  pre-built.
- **D-007 — Memory is reviewable, sourced, deletable.** Every memory entry
  carries its source; the user can inspect, edit and delete; private speech
  is not a training dataset by default. Trade-off: no silent "the agent just
  knows me" personalization — context selection draws only on approved
  memory, never raw recordings.
- **D-008 — Heard / inferred / executed are separate records.** The turn
  ledger keeps the recognizer's output, the assistant's inference and
  tool-result evidence apart; a spoken "done" requires the third, never the
  second. The original transcript is preserved so a misheard amount or name
  can be confirmed against what was actually heard.
- **D-009 — Turn IDs make stale audio unplayable.** Bounded buffers plus
  session/turn IDs: on interruption, playback stops, downstream work is
  cancelled where the API allows, and any late chunk from an older turn is
  dropped. An action already sent keeps an `uncertain` state and is
  reconciled — cancellation is never reported as undo.
- **D-010 — External actions are reviewed and audited.** The first tool is
  reversible and local; external actions require their explicit review and
  write an audit record. Retrieved text, audio and tool results are data,
  never instructions — adversarial input cannot elevate permissions or
  invoke a tool silently.
- **D-011 — Keys in the credential store, not in the app.** Model keys live in
  the system credential store or a server secret manager. Private speech,
  transcripts, prompts and audio content stay out of logs and metrics.

## Open questions (the guide leaves these open — do not guess)

- **Provider/model selection** — decided by recorded evaluations in V2/V3,
  not chosen up front.
- **Session-service topology** — the guide implies local-first ("PostgreSQL
  only if server-side multi-device state is required") but also names a
  "server secret manager" and keeps Ubuntu useful for Rust services. Working
  assumption: a local process on the Mac; revisit when a second device or a
  server deployment becomes real.
- **Consent-record format and storage** — the contract requires recording
  consent and deletion scope; the representation is an implementation choice
  owed to V3.
- **Usage-rights review artifact** — the guide requires that a personal-voice
  profile waits until "the model's usage rights are understood"; where that
  review is recorded (with the consent record vs. in `evals/`) is open.
