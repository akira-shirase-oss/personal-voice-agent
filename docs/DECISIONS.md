# Decisions

Deliberate choices, newest last. D-001–D-011 seeded the contract's
prescribed directions; later entries amend them explicitly rather than
rewriting them. Where the guide leaves a choice open and no decision has
settled it, it is listed under **Open questions** rather than guessed.

- **D-001 — Native Apple client, no webview.** Swift + SwiftUI with
  AVAudioEngine for audio and AppKit where needed; no Tauri, Electron or
  webview-hosted interface (the guide's firm exclusion). macOS first is the
  guide's *proposed* target — treated as committed for the first release.
  Trade-off: single-platform effort requiring a compatible Mac + Xcode; buys
  real control over capture, buffering and interruption semantics that a
  webview cannot guarantee.
- **D-002 — Rust session service owns orchestration.** Conversation state,
  bounded tool execution and provider adapters live in a Rust service, not
  in the Swift app. Trade-off: an extra process boundary and protocol; buys
  a deterministic, testable core, one enforcement point for permissions and
  deadlines, and a place secrets can live off the client. *Refined by
  D-015/D-016: the providers themselves run in the app; the service owns
  their deadlines, cancellation and every decision.*
- **D-003 — Existing models first, chosen by recorded evaluation.**
  Speech-to-text, language and voice-synthesis models are selected on
  recorded evaluations, usage rights and latency — not on reputation.
  Training is a separate later experiment; the availability of a
  voice-cloning model is not assumed.
- **D-004 — Standard voice before personal voice.** The first useful release
  ships a standard voice. A personal-voice profile is added only after the
  pipeline works *and* the model's usage rights are understood — and only
  under an explicit consent record.
- **D-005 — Drafting ≠ acting.** Private conversation and draft responses are
  the default posture. Speaking or sending on your behalf is a separate,
  explicit authorization boundary; synthetic speech is disclosed to
  recipients when acting externally, and voice imitation is never used to
  bypass authentication.
- **D-006 — Local-first private data.** Preferences and approved memory live
  locally; recordings are encrypted at rest with working deletion controls,
  and recording retention defaults to off. PostgreSQL enters only if
  server-side multi-device state becomes a real requirement — it is not
  pre-built. *Amended by D-018: the first release retains no audio at all,
  and deletion is described with its real scope.*
- **D-007 — Memory is reviewable, sourced, deletable.** Every memory entry
  carries its source; the user can inspect, edit and delete; private speech
  is not a training dataset by default. Context selection draws only on
  approved memory, never raw recordings.
- **D-008 — Heard / inferred / executed are separate records.** The turn
  ledger keeps the recognizer's output, the assistant's inference and
  tool-result evidence apart; a spoken "done" requires the third, never the
  second. The original transcript is preserved so a misheard amount or name
  can be confirmed against what was actually heard.
- **D-009 — Turn IDs make stale audio unplayable.** Bounded buffers plus
  session/turn IDs: on interruption, playback stops, downstream work is
  cancelled, and any late chunk from an older turn is dropped. An action
  already sent keeps an `uncertain` state and is reconciled — cancellation
  is never reported as undo.
- **D-010 — External actions are reviewed and audited.** The first tool is
  reversible and local; external actions require their explicit review and
  write an audit record. Retrieved text, audio and tool results are data,
  never instructions — adversarial input cannot elevate permissions or
  invoke a tool silently.
- **D-011 — Keys in the credential store, not in the app.** Model keys live in
  the system credential store or a server secret manager. Private speech,
  transcripts, prompts and audio content stay out of logs and metrics.
  *Amended by D-014: the first release has no provider keys at all; logs
  carry codes, IDs, counts and durations only, enforced by a log-capture
  test.*

### 2026-11-16 — first build

- **D-012 — The first release is V0–V2.** The guide's first useful release
  is "push-to-talk conversation with interruption, a transcript, clear
  errors and a standard voice". Memory, tools and the personal voice are
  the guide's *full direction*, so they move to V3/V4 as later contracts
  with their own gates. Trade-off: the first release does less; it can be
  finished and proven.
- **D-013 — "Consequential" and "ambiguous" are defined, not judged.** As
  in END_GOAL §Definitions: consequential = external or irreversible
  effect, or any amount/date/person/recipient slot; ambiguous = lowest
  segment confidence below 0.80, no confidence reported, or a slot value
  not literally in the normalized heard text. Either routes to a
  confirmation that shows the heard text verbatim and binds to the
  proposal digest. Trade-off: more confirmations than a model's judgement
  would ask for; in exchange the rule is testable and cannot be talked
  out of by the model.
- **D-014 — Providers: Apple on-device frameworks only.** Recognition with
  Speech's `SpeechAnalyzer` + `SpeechTranscriber` (the newest analyzer API
  on macOS 26+; this Mac runs 27.2.1); speech output with
  `AVSpeechSynthesizer` system voices; language with FoundationModels
  `LanguageModelSession` and guided generation. No cloud providers and no
  API keys. FoundationModels can be unavailable at runtime (Apple
  Intelligence off, device not eligible, model not ready): the app checks
  availability and shows that state as degraded. A deterministic test
  provider sits behind the same protocol for tests and CI; it is labelled
  as such and is never presented as the model. Apple Personal Voice is out
  of the first release (personal, non-commercial terms; V3's rights
  review). Trade-off: on-device model quality and a 4,096-token context
  window, macOS 26+ only; in exchange no audio or transcript leaves the
  Mac, there is no provider cost, and provider-side retention questions
  disappear. Probes on 2026-11-16 on this Mac: file transcription of a
  2.6 s utterance in 0.33 s; synthesis first buffer 0.67–1.06 s; model
  first structured snapshot 1.78 s.
- **D-015 — The session service is a child process over stdio.** The app
  spawns `voice-session` and speaks versioned JSON lines (`docs/PROTOCOL.md`)
  over its stdin/stdout. No TCP or Unix socket: no other process or web
  page can drive tools or read transcripts. EOF on stdin ends the service.
  Trade-off: one service per app instance, no long-lived daemon; acceptable
  for single-user, single-device use. This settles the topology question
  for the first release: local process; revisit only with a second device.
- **D-016 — Providers run in the app; the service orchestrates them.**
  Apple's frameworks are Swift-only, so the app hosts the recognizer,
  model and synthesizer behind Swift protocols. The service sends each
  provider request with a deadline, cancels it, drops late replies, and is
  the only place that decides what is spoken or executed. Audio never
  crosses the boundary. Trade-off: the provider boundary is a process
  hop (sub-millisecond on a pipe); the gain is that the enforcement point
  stays in the tested Rust core.
- **D-017 — Consent and ledger records: append-only, versioned, local.**
  One line per record, `<crc32> <json>`, `v: 1`, monotonic `seq`; one
  writer thread with group-commit `sync_data`; torn tails truncated and
  recorded, earlier corruption refused (ARCHITECTURE §On-disk format).
  Consent records carry scope, speaker, the rights-review hash (personal
  voice only) and deletion scope. Trade-off: a hand-rolled format instead
  of SQLite; it is small, inspectable with standard tools, and its crash
  behaviour is fully tested here. *Amended 2026-11-16 (review): consent also
  keeps a token file per granted scope, unlinked before a revocation is
  logged (an unlink needs no free space) and created after a grant is
  logged; effective consent is "logged and token present", a torn consent
  tail counts as revoked, and a grant that cannot be recorded does not
  stand. Tested on full HFS+ and APFS images (`tests/disk_full.rs`). Only the
  HFS+ run forces the revocation's log write to fail; on APFS unlinking the
  token can free enough space for the record to land, so that run checks what
  the user sees, not the failure path.
  Consent granted before this change has no token and must be granted once
  more. If the unlink fails too (the data directory cannot be written), the
  token's mode is set to 0000, a metadata change needing neither free space
  nor a writable directory, and a token without its owner's read and write
  bits counts as revoked; so a restart after both the unlink and the log
  write failed comes back revoked (second review; a `store.rs` test, and a
  full HFS+ image with the directory made read-only).*
- **D-018 — Deletion scope is stated honestly.** Deleting a file on APFS
  does not erase its blocks from local snapshots or earlier backups, and
  SQLite-style freelists keep bytes too. So: the data directory is
  excluded from Time Machine when created; docs say "removed from the live
  file system" and name snapshots and backups as residual copies; no
  third-party provider holds audio or transcripts (D-014). Real erasure for
  memory and any voice data comes in V3 by crypto-shredding: per-item keys
  in the Keychain, deletion destroys the key, and a test proves the item
  is unrecoverable afterwards. *Amended 2026-11-16 (review): heard
  transcripts now have controls. "Delete history" (`forget` with scope
  `capture`, acting on that consent's deletion scope) deletes every session
  log, the live one included (rotated into a fresh log that records the
  deletion), and closed logs older than 30 days are deleted at start and
  every hour while the session stays open (`--retention-days`). Both are tested: a sentinel transcript is found
  nowhere under the data directory afterwards. Copies in snapshots or
  backups made before the exclusion are not reached. "Delete history" and a
  capture revocation also clear the model's in-memory recap (D-025), so deleted
  speech never reaches the next prompt; a turn still finishing when history is
  deleted cannot add its text back (tested, second review N3). The deletion
  comes before the fresh log is created, so a full disk cannot stop it (it did:
  `tests/disk_full.rs` on full HFS+ and APFS images), and a deletion that fails
  is shown to the user (N4).*
- **D-019 — Per-stage deadlines (initial values).** Recognizer finalize
  after release 3 s; model first output 6 s; model complete 20 s;
  synthesis first audio 3 s; synthesis chunk playback 30 s; tool 5 s;
  confirmation 60 s; capture held at most 60 s. Set from the D-014 probes
  with headroom for a cold model; the V2 latency run re-evaluates them and
  any change gets a new entry with the run that justified it.
- **D-020 — The usage-rights artifact is required before any personal-voice
  feature.** `evals/rights/<provider>-<model>-<date>.md` covers voice
  rights *and* data use; the consent record references it by hash, so a
  model change forces a new review. Until one exists, the personal-voice
  scope cannot be granted (enforced in the domain), and a personal-voice grant
  found in the consent log does not fold to granted in a build without the
  voice or without a valid review hash; revoking it mid-turn stops playback
  like a capture revocation.
- **D-021 — "Stop" is defined for measurement.** Stop-playback latency is
  from the interrupt event's timestamp to the last non-silent sample
  rendered by the output node. Tests measure it by offline rendering
  (frames after the stop call); the live figure comes from the manual
  drill. The app stops its player locally on the interrupt key before the
  service round trip, and the service's fence makes stale chunks
  unplayable regardless of ordering.
- **D-022 — The load unit is turn events and ledger appends.** Full runs are
  1,000,000 turn events through the session service against the stub
  client and 1,000,000 ledger appends with append-to-durable p99. They are
  cheap regressions; for a single-user local app the evidence that matters
  is the audio soak and interrupt storms, owed to the live sessions.
- **D-023 — Barge-in stops playback before capture.** Push-to-talk during
  playback interrupts first, so the microphone does not hear the
  assistant; live capture enables voice processing (echo cancellation)
  where the device supports it. Audio from people other than an enrolled
  speaker is never retained or used for a profile — no audio is retained
  at all in the first release. *Limitation (2026-11-16): capture and playback
  run on separate AVAudioEngines, so voice processing on the capture engine
  has no reference to the assistant's own output and cannot cancel it; the
  stop-before-capture ordering is the mitigation that is actually in place
  and tested. Sharing one engine for both is owed, with Drill 3 as its check.*

- **D-024 — Warm, reused speech output.** One long-lived `AVSpeechSynthesizer`, one
  utterance at a time, warmed with a short phrase at start. Probed on this Mac: a fresh
  synthesizer took ~520 ms to its first buffer, a warm reused one ~15 ms; in the end-to-end
  bench synthesis first audio fell from ~1.2 s to p50 19 ms. Trade-off: a cancelled utterance
  stops the shared synthesizer, so utterances are serialized (they already are, per turn).
- **D-025 — A fresh model session per turn with a bounded recap.** Each turn gets a new,
  prewarmed `LanguageModelSession` whose prompt carries only the last two exchanges, each side
  cut to 300 characters, instead of one session growing with the whole conversation. The
  first bench (one growing session) showed model latency rising across rounds and 6 of 21
  turns missing the 6 s first-output deadline. Trade-off: the model forgets anything older
  than two exchanges; long-term memory is V3's reviewed store, not the model's context. The
  recap is cleared on "Delete history" and on a capture revocation (D-018).
- **D-026 — Offline rendering is the deterministic playback test bed.** Playback tests and the
  bench render the player with AVAudioEngine manual offline rendering, so nothing plays on the
  speakers and every sample is inspectable. Two behaviours found by probing are part of the
  contract of those tests: `prepare()` before an offline start makes the first quantum
  silent, and a buffer scheduled on a running node can start a few quanta late — so tests
  render until sound appears and prove silence over windows longer than the audio in question.

- **D-027 — The completion-claim guard fails closed (2026-11-16, second review).** No tool can
  run in V0–V2, so a model sentence that might say an action happened is never spoken bare. The
  classifier gives one of three verdicts per sentence while the turn has no executed evidence.
  *Replace* (spoken as the draft disclaimer): a past, perfect, passive or headline form of an
  action verb whose subject is the assistant, a task, a person or a pronoun ("Priya has been
  notified", "$50 sent to Priya", "I'd already sent it"), a completion phrase ("went out",
  "consider it done"), or "I'm sending it now". *Disclose* (spoken with "Nothing has been done."
  after it, so a false positive costs a clause, not the sentence): a less certain form — a
  general verb with a pronoun subject ("It's set in Paris"), a third party's action, a past
  passive about something other than a task, a claim inside a quoted draft. *Plain*: the rest.
  Questions, negation, modals, conditionals, habits, the user's own actions, historical markers
  and attributive adjectives exempt a verb only where they govern it, so "I paid the bill after
  you confirmed", "No problem I've sent it" and the claim before a tag question ("…, anything
  else?") are caught. Measured on `fixtures/claims/corpus.tsv`, 334 phrasings: the earlier 136,
  the second review's 32 listed claims (two as the two-sentence replies it describes) and the 7
  plain sentences it listed (it did not publish its other 8 claims and 10 plain sentences), and
  159 added here (tenses, passives, numbers and currency, quoted drafts, multi-clause replies,
  story, recipe and history prose). Of 169 claims, 161 are replaced, 7 disclosed and **1 is
  missed (0.6%)**: "Priya should have it by now." Of 165 plain sentences, 149 are spoken intact,
  **15 get the disclosure (9.1%)** and **1 is replaced (0.6%)**: "Booked solid, the restaurant
  had no tables." The guard before this change, on the same corpus: 60 of 169 claims spoken
  bare (35.5%, including all 32 of the review's) and 21 of 165 plain sentences replaced
  (12.7%). These are rates on a corpus the classifier was tuned against, not on the model's
  real output, so they are an upper bound on how well it does, not a guarantee. It reads
  English, plus a short list of other languages' participles of send/pay/book/transfer (any of
  them counts as a claim; their negation is not parsed). `tests/claims.rs` pins every verdict
  and fails if a rate rises.
  *Amended (third review): the sentence classifier is not reliable on new text, so the guard
  now fails closed at the turn level.* No tool can run in V0–V2, so a reply turn ends with one
  spoken "Nothing has been done." (session-authored, ledgered as `turn_disclosed`) whenever its
  heard request may ask for an action, or any of its sentences was replaced or flagged. The
  request test fails closed too: only a question or an informational request ("what…",
  "tell me…", "explain…") naming no action verb in any form and no task noun, and not phrased
  as a request ("could you…", "will you…", "please…"), counts as purely informational; every
  other request, statements included, gets the disclosure. A qualifying turn cut off before the
  reply ends (interrupt, model failure, deadline, revocation) gets it too: recorded, and shown
  by the app with the turn (`notice nothing_done`), since it can no longer be spoken. Per-sentence
  `Replace` stays on top for clear claims; a flagged (`Disclose`) sentence is now spoken as
  written, since the turn carries the disclosure. Measured on `fixtures/claims/held-out.tsv`
  (83 lines, written before the run and never used for tuning: the 39 phrasings a third review
  published, which were mostly its misses, plus 44 new): the **sentence classifier alone missed
  51 of 61 claims (84%)** and replaced 6 of 22 plain sentences (27%, all from the review's
  published misses); **with the turn-level disclosure, 1 of 61 claims was spoken with no
  disclosure at all (2%) and 0 of the 57 that answer a request asking for an action**. The one
  miss is an unprompted claim in reply to an informational question ("What's two plus two?" →
  "Four, and your reminder is in."), which only the sentence classifier can catch; 1 of the 4
  such held-out claims got through. The cost: 12 of 22 plain replies also got the disclosure
  (55%; 5 of the 15 written for this file). The code before this amendment spoke 51 of the 61
  claims with no disclosure. `tests/held_out.rs` pins only that every claim answering its
  request ends disclosed; the rates are printed, not pinned. Trade-off: "Nothing has been
  done." after some replies that never needed it; a fabricated "done" sounds true.
  *Recomputed after a final check* (polite-question requests such as "could you sort out the
  plumber?" were treated as plain questions; 9 of its turns added, 92 lines): the sentence
  classifier alone misses 58 of 70 claims (83%); with the turn-level disclosure **5 of 70 get
  no disclosure (7%), all of them claims inside answers to plain questions (5 of 8 such), and
  0 of the 62 that answer a request**. Plain replies unchanged: 6 of 22 replaced, 12 of 22
  disclosed.

## Open questions

- **Channels for external actions** — V4's contract must name them (email?
  messages?) and how recipients are told speech is synthetic. Outbound
  calls stay excluded.
- **Personal-voice provider** — evaluate Apple Personal Voice first in V3
  (OS-managed consent, personal non-commercial terms) against the
  rights-review artifact; nothing is assumed.
