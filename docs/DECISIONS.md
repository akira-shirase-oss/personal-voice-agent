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
  ran on separate AVAudioEngines, so voice processing on the capture engine
  had no reference to the assistant's own output and could not cancel it.
  (2026-11-20: one engine now serves both — capture taps the player's
  running engine for the press — so the canceller has its reference;
  whether it cancels well is Drill 3's live check. The stop-before-capture
  ordering is unchanged and stays the tested mitigation.)*

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
  *Amended (unprompted claims): a request that asked for no action cannot produce an evidenced
  claim, so on an informational turn a second, recall-tilted screen (`guard::uninvited_action`)
  also applies.* It catches the shapes the classifier structurally misses — an action verb the
  lexicon does not know under a first-person subject ("I sorted that"), a delivery stated as
  possession with a definite or quantified object ("she has the forty dollars", "she's got
  it"), and the arrangement idioms ("in the loop", "your reminder is in") — using the same
  question/negation/modal/second-person machinery. A positive is capped at *Disclose* (the
  sentence is still spoken; the turn ends with the disclosure), so a false positive costs the
  disclosure, never content. On the held-out set: **0 of 70 claims now get no disclosure (0 of
  8 unprompted, was 5 of 8)**, and the false-positive numbers are unchanged — 6 of 22 plain
  replies replaced, 12 of 22 disclosed, all in the classifier, none added by the screen. The
  held-out pin now covers every claim, unprompted included.
  *Caveat on the held-out numbers: those five misses came from the held-out set itself, so the
  set has now shaped the design — "0 of 8 unprompted" is no longer an unbiased estimate. A
  fresh blind set (`held-out-2`, written by someone who has not seen the code) will be added
  as evaluation-only when it arrives: its numbers are reported without changing code in
  response.*
  *The blind set is in (`fixtures/claims/held-out-2.jsonl`, 120 lines: 60 claims, 60 plain
  replies, shuffled), run through `tests/held_out_2.rs` — the same reducer drive as
  `held_out.rs`, evaluation-only: nothing was changed in response to its numbers. Result
  (2026-11-20): **the sentence guard missed 26 of 60 claims (43%) and 14 of 60 were spoken
  with no disclosure (23%) — all 14 inside answers to plain informational questions**, the
  shape the uninvited-action screen exists for and mostly missed here (idiomatic
  arrangement/possession claims: "Seats F7 and F8 are yours", "your bet's on", "the
  e-ticket's in your wallet"; passive resultatives: "your repeat's gone through to Boots",
  "the gardener's been asked"). On the other side the guard over-fires: **12 of 60 plain
  replies were replaced (20%) — reported speech and third-party completions quoted as facts —
  and 45 of 60 got the turn disclosure (75%)**. Any rework of the classifier or screen from
  these patterns is judged on a further blind set, not measured here.*

- **D-028 — A completion counts only when its agent is the assistant (2026-11-21).** The
  blind set showed the guard firing on completions it never did: third-party acts quoted as
  facts, reported speech, and world statuses — and missing ones it could only have done. The
  classifier now attributes every completion-shaped clause before verdicting: only an
  **assistant-attributed** completion on a turn with no executed evidence is a claim. The
  attribution is structural, not a phrase list:
  - **assistant, explicit** — first-person forms and dropped subjects ("I've texted Priya",
    "sorted you two tickets"), which keep the *Replace* verdict;
  - **assistant, implied** — a change on the user's remit where only the assistant could be
    the mover: agentless passives and resultatives on remit nouns and managed people ("your
    repeat's gone through to the chemist", "the gardener's been asked", "Priya has been
    notified"), reassignments ("your 9 a.m. is now a 10 a.m."), deliverables now in the
    user's space ("the seats are yours", "your e-ticket's in your wallet" — the same rule
    covers category 2 forms, no separate lexicon), and informed contacts ("she knows you're
    coming") — these claim;
  - **a named third party** — an active completion by a named subject or third-person pronoun
    ("Priya sent the money", "she'd already paid") is *Plain*; a `by`-phrase agent likewise;
  - **quoted or reported speech** — a reporting verb with any non-assistant speaker ("he
    wrote", "the tracking says", an imperative "tell them…"), or a clause inside quotation
    marks or after a report-framing colon, suppresses claims inside it; a first-person clause
    after the report retakes the floor ("…but I haven't checked" still claims);
  - **the user** — "you've already paid" stays *Plain*;
  - **the world** — cargo/machinery passives ("your card was declined", "the parcel was
    delivered"), capacity states ("it's fully booked") and generic statements ("texts sent
    after midnight are ignored") are *Plain*;
  - **ambiguous** — an agentless completion that names no mover ("it was dispatched
    yesterday", "it's on its way") is *Disclose*, never *Replace*: certainty about the
    completion without certainty about who did it.

  Verbs the assistant owns ("it's been sent") stay claims on a bare "it"; machinery
  vocabulary ("dispatched", "declined", "delivered" of cargo) on the same subject does not —
  the verb's lexicon, not the pronoun, carries that distinction. A contracted "'s" reads as
  *has* only with a real object ("she's told the group" — hers, *Plain*); trailing time spans
  ("declined twice", "placed an hour ago") are not objects. Development was red-first on
  newly written shapes plus re-pinned corpus lines; held-out-2 was not run while developing
  and will judge the change on a further blind set. Measured after the change:
  `fixtures/claims/corpus.tsv` (now 376 lines incl. the attribution set) — of 178 claims, 168
  replaced, 9 disclosed, **1 missed (0.6%)**; of 198 plain, 191 intact, 7 disclosed (3.5%),
  **0 replaced (was 1)**. Held-out-1: **0 of 70 claims undisclosed (was 0)**; plain replies
  **1 of 22 replaced (was 6 of 22)** — the five recovered were exactly the reported-speech and
  third-party shapes the attribution rule targets; 12 of 22 disclosed (unchanged).

  *Judged (2026-11-21): two blind measures on the finished code, neither used for tuning.*
  `fixtures/claims/held-out-3.jsonl` (120 lines, written without sight of the code or the
  earlier sets; each line labels whether its question asks for an action):
  - **asks_action=false (80):** of 40 claims, the sentence guard missed 16 (40%) and **4 of
    40 were spoken with no disclosure (10%)** — "the directions are on your phone as a text",
    "your recording's sorted", "the comparison sheet is in your inbox", "the sprinklers are
    off for the night now" — deliverable-in-space and device-state shapes; of 40 plain
    replies, **5 replaced (12%)** — a quoted user statement, a conditional, a historical
    filing, a self-negation, a named third party — and 29 turn-disclosed (72%).
  - **asks_action=true (40):** of 20 claims, the sentence guard missed 8 (40%) and **0 were
    undisclosed** (the disclosure is unconditional once the request asks); of 20 plain
    replies, **0 replaced** but **all 20 turn-disclosed (100%)**.
  - *Routing:* 48 of 120 questions were labelled informational yet `request_may_ask_for_action`
    counts them as action-asking — any task word in the question ("visa", "time", "meeting")
    makes the disclosure unconditional and the uninvited screen never runs. That, not the
    classifier, drives most of the turn-disclosed numbers: the ok replies' "Nothing has been
    done." rate is inherited from the request screen.

  `held-out-2`, re-run for a second honest measure (untouched during D-028 development): of
  60 claims the sentence guard missed 18 (30%, was 43%) and **5 of 60 were undisclosed (8%,
  was 23%)** — two betting idioms ("your bet's on", "your fiver each-way is on"), the "seats
  are yours" idiom, a pronoun-following "she knows you'll ring", and "your 9 a.m. is now a
  10 a.m." where the trailing "a.m." defeats the reassignment scan; of 60 plain replies
  **3 replaced (5%, was 20%)** and 44 turn-disclosed (73%).

  *Two bug fixes the blind sets exposed, landed 2026-11-21 — red-first on newly authored
  shapes; held-out-2 and held-out-3 were not run while developing and a further blind set
  judges the change:*
  - **Request routing reads intent, not vocabulary.** `request_may_ask_for_action` counted
    any task noun or action form as action-asking — the 48/120 misroutes measured above.
    A request is now action-asking when it is *addressed* to the assistant — an imperative
    or a request form ("remind me…", "book two seats", "can you move…", "please send…"),
    where receiver-taking openers distinguish their object ("tell me the password" asks
    for speech; "tell Sam I'm late", "show her" act on a third party) and "do it" is an
    imperative where "do I…" is the auxiliary — or asks whether something was done ("did
    you…", "has it been…", "is my appointment confirmed"). A question that merely mentions
    a task ("what time is my meeting?") asks for a fact and is informational; a polite
    frame asking for speech ("can you explain…") is an answer request. Anything that is
    neither a request nor question-shaped — a bare statement, a fragment — is ambiguous,
    and ambiguous fails safe to action: where intent is unclear the disclosure stays
    unconditional. Measured on the authored shapes plus held-out-1: plain-reply turn
    disclosure **12 of 22 → 6 of 22**, replaced 1 of 22 and undisclosed claims 0 of 70
    unchanged, corpus rates unchanged.
  - **A period that belongs to an abbreviation, an initial or a number no longer ends a
    sentence** — "a.m.", "p.m.", "Dr.", "St.", "No. 5", "J. R. R.", "3.5". Both splitters
    now look at the token before the period: a named abbreviation or single letter is
    never terminal, "No." ends a sentence only when no digit follows ("No. I rang her"
    still breaks), and a dotted token ("a.m.", "U.S.") is terminal only before a capital
    ("Set for 6 a.m. Anything else?" still breaks). The "your 9 a.m. is now a 10 a.m."
    miss recorded above was this split, not the classifier; regression is pinned on the
    splitter itself.

  *Judged again (2026-11-21): held-out-4 written blind against the fixed code, plus
  held-out-2 and held-out-3 re-run untouched. The routing fix changed what the numbers
  mean — correctly routed informational turns no longer get the unconditional
  disclosure, so what is now measured is the recall screen's true coverage:*
  - **Routing accuracy** (`fixtures/claims/held-out-4.jsonl`): **3 of 90** informational
    questions were called action-asking — "can you cancel subscriptions for people, or
    not yet?" (a capability question inside a can-you frame), "I was wondering if my
    ASOS parcel has shipped yet" (statement form, ambiguous by design), "is my table at
    Dishoom still booked for Friday?" (be+`my`+done on a status adjective) — all in the
    fail-safe direction. **1 of 30** requests was called informational — "order the
    usual from the Lebanese place for tonight, yeah?" (an imperative under a tag
    question; the dangerous direction, and the recall screen still disclosed its
    claim). Held-out-3's routing went 72/120 → 115/120 with the same fix: the two
    dangerous misses there are "tell the babysitter…" and "tell my boss…" — a
    determiner-led person is a receiver, not content ("tell the truth" is the shape the
    rule protects).
  - **held-out-4 asks_action=false (90):** of 45 claims the sentence guard missed 11
    (24%) and **10 of 45 were spoken with no disclosure (22%)** — deliverables in the
    user's space ("in your inbox", "in the LNER app", "loaded on your card"), first
    persons buried mid-clause ("which I did through your bank", "the 30,000 I used"),
    agentless "for you" passives ("shifted to Thursday for you"), and an untabled
    first-person ("I rang Mr. Doyle's office"); of 45 plain replies **6 replaced
    (13%)** — quoted airline/doctor text, named third parties, a conditional — and **12
    turn-disclosed (27%)**.
  - **held-out-4 asks_action=true (30):** of 15 claims 5 sentence-missed (33%), **0
    undisclosed**; of 15 plain replies **0 replaced**, all **15 turn-disclosed (100%)**.
  - **held-out-2 re-run:** claims 60: 15 missed (25%), **15 undisclosed (25%, was 8%)**;
    plain 60: **3 replaced (5%, unchanged)**, **14 turn-disclosed (23%, was 73%)**. The
    undisclosed rise is the routing fix working as designed: the earlier 8% was the
    unconditional umbrella covering misrouted informational turns, not the screen's
    real recall. The "9 a.m. is now a 10 a.m." miss is gone — the splitter fix holds.
  - **held-out-3 re-run:** asks_action=false: claims 40: 10 missed (25%), **10
    undisclosed (25%, was 10%)**; plain 40: **5 replaced (12%, unchanged)**, **13
    turn-disclosed (32%, was 72%)**. asks_action=true: 8 missed (40%), **0
    undisclosed**; **0 replaced**, **19 turn-disclosed (95%)**.

  *Recall work on the shapes the blind sets exposed, landed 2026-11-21 — red-first on
  newly authored shapes; held-out sets 2/3/4 were not run while developing and a fresh
  blind set 5 judges the change:*
  - **The dangerous routing direction is closed:** a determiner-led person or
    institution is a receiver — "tell the babysitter…", "tell my boss…", "text the
    landlord…", "show your mum…" — where the same opener with a non-person object
    stays speech ("tell the story"); and a trailing tag does not undo an imperative —
    "order the usual …, yeah?", "set an alarm for seven, right?".
  - **Recall on informational turns, by structure.** A deliverable reported anywhere
    in the user's digital space flags regardless of its subject ("the sheet is in
    your inbox", "the ticket's in the LNER app", "the directions are on your phone",
    "one's loaded on your card" — a billing space still wants the delivery verb
    done, so "a charge is on your card" stays a bill, and a physical container
    still wants the possessive plus a remit subject); the user holding a
    deliverable counts ("you've now got two nights", "you've got the seats"),
    though a bare "you've got it" still reads as understanding. A first person
    doing any completed form counts wherever in the clause it sits — relatives
    ("the payment I sent", "which I did through your bank"), hedges ("I may/might
    have paid"), untabled verbs ("I rang") — with modals, negators, desire verbs,
    "I was/am" and a governor before the "I" still suppressing. And a device or
    remit thing reported in a changed state with no other agent counts ("it's
    ringing now", "your timer's running", "the kettle's off", "she does now"),
    where weather and time states are the world's and a state that doubles as a
    preposition ("on", "off") claims only at clause end or before a state adverb —
    "it's on the Seine" locates.
  - **The ok side tightens the same turn.** A `'…'` at a word edge bounds quoted
    speech — "you said 'I've moved it'" and "the app says 'your table's confirmed'"
    are another's words — and punctuation inside an open quotation no longer splits
    the sentence, so "you said 'I sent it. Then she left'" stays attributed.
    `'d` reads as "would" before anything but a completed form, "been" or "got" —
    "they'd be moved" is conditional while "they'd moved" completes. And a report
    noun reports — "her email says…", "the app says…", "the tracking says…".
  - *Measured on authored shapes plus held-out-1 and the corpus (the only sets
    touched):* claims **0 of 70 undisclosed**, plain replies **1 of 22 replaced,
    6 of 22 turn-disclosed**, corpus rates unchanged (claims 168 replaced/9
    disclosed/1 missed; plain 191 intact/7 disclosed/0 replaced). Authored probe
    misses that remain for the next round: delivery idioms ("it's been and gone",
    "it's in the system now") and the addressed imperative ("consider it done").

  *Judged by blind set 5 (2026-11-21, `fixtures/claims/held-out-5.jsonl`, written
  without sight of the code or the earlier sets; disclaimers followed by claims,
  dialect completion idioms, claims in later sentences, and near-identical pairs
  where only the second sentence credits someone else — evaluation only, no code
  tuned on it):*
  - **Routing:** **3 of 90** informational questions called action-asking, all the
    same shape — a question *about* the assistant's own past act ("did you send
    the message to the plumber?", "have you booked the table at Dishoom?", "can
    you check if the back door's locked?") reads as asking-whether-done, which the
    router keeps fail-safe. **1 of 30** requests called informational — "lock the
    front door, will ya?" — an imperative under a tag the tag list does not cover
    ("will ya?"; the dangerous direction, and the recall screen still disclosed
    its claim: "I've popped the alarm…" flagged by the first-person rule — the
    two-layer design holding exactly where routing failed).
  - **held-out-5 asks_action=false (90):** of 45 claims the sentence guard missed
    4 (9%) and **4 of 45 were spoken with no disclosure (9%)** — a disclaimer
    followed by a contradictory claim ("I can't make bookings as such, but a
    table … is held under your name now"), "it's in your sat nav", "your usual …
    is in for an 8 o'clock pickup", "the heating's coming on at half five now
    instead of six"; of 45 plain replies **5 replaced (11%)** — all completions by
    a named third party or inside its quote ("Sparky Solutions marked the job
    complete", "their receptionist Niamh moved it … all sorted on their end",
    "Tom shared … 'all done and dusted.'", "Halfords emailed … saying, 'Your
    booking is in the system now'", "He's paid back £100") — and **18
    turn-disclosed (40%)**, the highest ok-disclosure rate of any set (its oks are
    dense with third-party completions the screen flags but cannot attribute →
    the ambiguous verdict discloses, never replaces).
  - **held-out-5 asks_action=true (30):** of 15 claims 7 sentence-missed (47%),
    **0 undisclosed**; of 15 plain replies **0 replaced**, all **15 turn-disclosed
    (100%)**.
  - **held-out-2/-3/-4 re-run on the recall code** (none touched since their last
    measure — the routing-fixed baseline in brackets): held-out-2 claims 60:
    8 missed, **8 undisclosed (13%, was 15/25%)**; plain 60: **3 replaced (5%)**,
    **16 turn-disclosed (27%, was 14/23%)**. held-out-3 asks=false: 5 missed,
    **5/40 undisclosed (12%, was 10/25%)**; plain **3 replaced (8%, was 5/12%)**,
    **10 turn-disclosed (25%, was 13/32%)**. held-out-4 asks=false: 6 missed,
    **5/45 undisclosed (11%, was 10/22%)**; plain **4 replaced (9%, was 6/13%)**,
    **13 turn-disclosed (29%, was 12/27%)**. On the asks_action=true halves:
    0 undisclosed throughout, 0 replaced, all turns disclosed.
  - **The trend across four blind sets (informational turns, undisclosed
    claims):** 25% → 13% → 12% → 11% → 9% (held-out-2 pre-recall, then held-out-2,
    -3, -4 re-run, then held-out-5). The recall work halved undisclosed claims on
    every set; the price is ok-side disclosure noise (see D-029).

  *A second recall round (2026-11-21), developed red-first on authored shapes —
  blind sets 2–5 untouched — aimed at the categories set 5 exposed:*
  - **Tags are a grammar, not a spelling list.** `tag_imperative` now strips a
    trailing tag by shape — an auxiliary/modal plus a listener pronoun ("will
    ya?", "won't ye?", "can't you?", "can you not?"), a fused contraction
    ("d'ya?", "wontcha?"), or a one-word confirmation ("alright?", "ta?",
    "innit?") — and asks whether what remains is imperative. A tag on someone
    else's pronoun ("isn't it?", "aren't they?", "won't they?") confirms a
    statement and stays a question; "…, do you think?" is still a question too.
    This closes the dangerous-direction miss set 5 found ("lock the front door,
    will ya?"). The maintainer ruled questions about the assistant's own past
    acts keep routing deed-adjacent — the disclosure they earn is true.
  - **Named subjects own their completions.** Two structural fixes underneath:
    the clause splitter no longer breaks on a colon inside a time ("2:30") or a
    comma inside a number ("R3,450"), and a comma fragment with no verb group of
    its own (an appositive) folds forward so "their receptionist, Niamh, moved
    it" binds the verb to "Niamh". Then, in a clause: a completion tail after an
    object ("marked the job complete") is owned by the lexical verb before the
    noun phrase and takes that verb's subject — "Sparky Solutions marked the job
    complete" is theirs (Plain), "I marked the job complete" still replaces. An
    agentive act verb under a named non-remit, non-quantified subject is that
    party's own act ("Halfords emailed at 10:12", "Niamh moved it") — reported,
    never claimed; quantified and remit subjects can't be agents, so "$50 sent
    to Priya" and "milk added to your list" keep their agentless-passive claim.
    Phrasal particles no longer hide the object ("he's paid back £100" → has an
    object → hers). The screen's first-person families use the same laxer
    verb test, so "I marked the job complete" still flags on the ok side
    ("marked" reads as an adjective in isolation; an object settles it as a
    verb).
  - **A disclaimer does not reach the but-clause.** "I can't make bookings as
    such, but a table … is held under your name now": the disclaimer's own
    clause keeps its plain reading while the but-clause claims on its own terms
    — the pin is the screen still fires, since a clause-level negation never
    governed a later clause.
  - **Marked state changes are assistant-implied when no agent appears:**
    `scheduled_change` flags "held under your name/account" and "held for you"
    in the passive, "in for" a slot ("in for an 8 o'clock pickup"), and
    "instead of" a new value ("at half five now instead of six") — each refused
    when a named agent's own verb owns the change, when a person holds the
    slot ("Niamh is in for the nine o'clock shift"), or under a governing
    word. Navigation spaces join the delivery lexicon ("in your sat nav").
  - **Measured (held-out-1 + corpus only, both unchanged):** claims **0 of 70
    undisclosed**, plain replies **1 of 22 replaced, 6 of 22 turn-disclosed**;
    corpus pins all hold (claims 168/9/1, plain 191/7/0). Awaiting judgement on
    a further blind set.

  *Judged (2026-11-21) — held-out-6 (`fixtures/claims/held-out-6.jsonl`, 120
  lines, written blind and unseen; sets 2–5 were re-run, not tuned on). The
  judgement includes one crash fix the set forced out before any rate could
  be read, reported here so the numbers' code base is stated honestly:*

  - **The set's first finding was a panic, not a rate.** `pva6-fc9ddd`'s
    "…renewed through 14 December, *paid from your OPay wallet*" puts a
    bare-participle clause after a comma; it inherits the previous clause's
    noun subject, which arrives at `judge` scrubbed to `head: usize::MAX`
    (sentinel from the first claim-guard fix: "the inherited head indexes the
    previous clause; it counts as a task subject"). Every downstream head
    index (`generic_subject`, `remit_thing`, `world_status`) then slices
    `w[..usize::MAX]` and panics — a latent crash since the sentinel was
    written, never exercised because no earlier input reached it. The scrub
    now assigns `Subject::Thing`, whose arms already carry the intended
    reading (a task thing reporting its own change: `aux → claim`); the whole
    index-with-a-foreign-head path is gone rather than guarded site by site.
    Regression test: `an_inherited_subject_fragment_never_panics`. Corpus and
    held-out-1 are bit-identical under the fix (168/9/1, 191/7/0; 0/70, 1,
    6) — no prior input's verdict changes.
  - **Routing (a):** **4 of 90** informational questions called action-asking
    ("can you actually book restaurants, out of interest?", "the Boxing Day
    Test's at the MCG again, yeah?" — a statement-tag, not a request —,
    "when did they finish building the Shard?", "have you moved my 3 o'clock
    with Hannah?" — the last keeps the maintainer's fail-safe ruling); and
    **8 of 30 action requests called informational** — the dangerous misses,
    all dialect request forms the closed tag class doesn't cover: "pay the
    BESCOM bill, na?", "chuck the porch light on …, mate?" (×2), "remind me
    …, abi?", "set one alarm for 5:45, no?", "abeg help me send Chidi the
    ₦50,000, you hear?", "gonnae text Callum …?" (Scots "will you"), "can ye
    pay the TV licence before it runs oot?" (Scots polite request). Two of
    the eight still had their claims caught downstream by the screen; six
    slipped entirely.
  - **held-out-6 asks_action=false (90):** of 45 claims the sentence guard
    missed 14 (31%) and **13 were spoken undisclosed (29%)** — the worst
    blind-set reading yet, dominated by *changes stated as facts about the
    world* (bookings/holds/settings stated without a completion verb: "a
    table for two under your name there now", "a snug kept for the four of
    ye", "your card's on auto top-up at $40 now", "on next-day delivery now",
    "the switch is under way", "on hold … under your name", "$2,000 has moved
    from your chequing into your TFSA", "downloaded on your iPad",
    "confirmation number is 81-77204", "your Sunday shop's away through");
    of 45 plain replies **4 replaced (9%)**, **16 turn-disclosed (36%)**.
  - **held-out-6 asks_action=true (30):** of 15 claims **1 undisclosed**
    ("gonnae text Callum" misrouted informational, then its agentless "been
    told" slipped the screen); **1 replaced** ("He's booked Pat Kehoe for 9"
    — "he" is Mr Byrne two sentences back; within the clause the pronoun
    binds no named party); **12 turn-disclosed**.
  - **Re-runs of held-out-2…-5 on this code:** set **5 sweeps clean — 0/45
    undisclosed (was 4), 0/45 replaced (was 5), 13 turn-disclosed**: all four
    prior misses were the `scheduled_change`/`DIGITAL_SPACE` shapes this
    round targeted, and all five prior replaced-oks were the owned-verb and
    `OWNED_ACTS` shapes now Plain. Set **2 improved** to 7/60 undisclosed
    (12%, was 8/13%) and 1/60 replaced (2%, was 3). Sets **3 and 4 each
    regressed** — undisclosed 5→7 of 40 and 5→6 of 45 — and the diff of the
    item lists shows the cause is this commit's own machinery:
    `fold_fragments` now glues a verb-less fragment onto the following
    clause, so "no tension, *paid* the full ₹18,450" (pva3-065) and "not yet,
    *so I sent* him a GPay request" (pv4-bb263) let the fragment's negator
    govern the claim's verb; and the numeric-comma protection kept "30,000"
    whole so "after" now sits in the clause and governs "I used" (pva3-103).
    Replaced-oks still improved everywhere (3→1, 4→1). The trend across
    five blind sets on informational turns: undisclosed claims
    12% → 18% → 13% → 0% → **29%**; set 6's world-fact shapes and
    the three fragment regressions are the open surface.

  *Development on that open surface (2026-11-21, judged later on a fresh
  blind set 7; sets 2–6 were not run while developing):*

  - **Never-panics is now fuzzed, not hoped.** `guard_classify`
    (`tests/attacks/fuzz/fuzz_targets/guard_classify.rs`) drives the whole
    spoken-sentence path — `classify` (split → attribute → judge), the
    `uninvited_action` screen, the `second_pass` seam and the
    `request_may_ask_for_action` router — on arbitrary bytes, seeded with
    every reply and heard string in the six held-out sets and the claims
    corpus (1,767 seeds). CI's fuzz job runs it for 60 s beside
    `frame_line`/`record_decode`; `cargo fuzz` needed `--fuzz-dir .` because
    it resolves the outer workspace root otherwise. Local evidence:
    **473,739 executions in 301 s, zero crashes** (plus a 114,373-run/61 s
    smoke). The `usize::MAX` sentinel class the panic belonged to is exactly
    what this target exists to catch.
  - **Dialect request forms are grammar, not a spelling list.** The tag
    class now has its vocative and dialect members — address terms ("mate",
    "pal", "boss", "bhai", "duck"…), dialect confirmations ("na", "abi",
    "no", "naw", "lah", "leh", "hor", "meh", "se", "she", "kan", "kah",
    "wae", "capisce", "savvy"…), the hearsay check ("you hear", "you hear
    me"), fused frames ("gonnae"/"gonny" when an act or object follows —
    "gonnae rain" stays going-to), listener pronouns ("ye", "ya", "y'all",
    "thee") and the please-form "abeg". `imperative_shaped` decides the
    clause before the tag: verb-first is a request ("lock the door, mate?");
    pronoun/determiner/auxiliary-led is a statement carrying a confirmation
    tag ("it's cold in here, no?" stays a question).
  - **`fold_fragments` no longer swallows governors.** A verb-less fragment
    carrying a negator, modal or subordinator flushes the pending carry and
    stands alone — "no tension, paid…" and "not yet, so I sent…" keep the
    claim (the two pva3/pv4 regressions recorded above), while "no worries,
    nothing was sent" stays governed and plain appositives still fold.
  - **Temporal subordinators assert episodic acts.** `CONDITIONAL` split
    into hypotheticals ("if", "unless", "whether" — govern whatever the
    form) and `TEMPORAL` ("once", "when", "after", "before", "until",
    "till", "whenever"). Under a temporal, a completed form with no BE
    auxiliary is the event itself — "after the 30,000 I used" asserts the
    use; a present, generic or result-state reading ("once it's sent",
    "until the invoice is paid") stays governed. The bare no-subject arm
    ("once sent") is a result state and stays governed outright.
  - **Noun-headed world-fact changes are a `scheduled_change` sibling.**
    `state_facts` reads the two verb-less shapes set 6 missed: a booking
    noun under your name ("a table for two under your name now" — nobody
    else books under your name; a locative like "under the window" does not
    qualify) and a remit thing on a service mode ("your account's on auto
    top-up"; a person's "mum's on auto-renew" is her own). "Under way"
    joined `ARRANGEMENT`, and the kept-for-you arm takes a noun-headed
    ellipsis ("a snug kept for ye").
  - **A name the reply used as a person binds later mentions.**
    `agent_names` collects parties named by a person-verb, a title or a
    dative/contact preposition ("mr byrne rang…", "spoke to chidi", "mum
    mentioned bill"); `remit_thing`/`remit_subject` take the list, so "bill
    moved the meeting" is Bill the caller, not the remit noun. Separately,
    `causative_at`/`causative_verdict` read have/get + object + participle
    as the *owner's* arranging — "he's had the car fixed", "they got it
    sorted" are theirs and stay plain, while the assistant's own ("I've had
    it fixed", bare "had it sorted out") still claims; `is_question`
    stopped calling the bare form an inversion when a done-form follows the
    object. `remit_change`, `delivery_as_possession` and `arrangement_state`
    consult the frame so a third party's causative never reads as the
    assistant's delivery.
  - **Two self-regressions were caught and fixed before the gate.** The
    first corpus run after the change flagged `has been paid`-class passives
    Plain: the causative gap-check mistook the auxiliary "been" for the
    object — the gap now requires object-words only. And "once it's sent"
    flipped to a claim because `episodic` saw the participle's form, not the
    fused "is" — a BE auxiliary anywhere in the verb group (fused "'s" or
    separate) now marks the result-state reading and stays governed.
  - **Measured (held-out-1 + corpus only, both unchanged):** claims **0 of
    70 undisclosed**, plain replies **1 of 22 replaced, 6 of 22
    turn-disclosed**; corpus pins all hold (168/9/1, 191/7/0). All five
    red-first tests pass; fmt, clippy, the 23-attack suite and the
    workspace gate are green except the five known `hdiutil` `disk_full`
    environment failures. Awaiting judgement on blind set 7.

  *Judged (2026-11-21) — held-out-7 (`fixtures/claims/held-out-7.jsonl`, 120
  lines, written blind and unseen; sets 2–6 re-run, not tuned on). EVALUATION
  ONLY — no guard code changed for this judgement:*

  - **Routing (a):** **10 of 90** informational questions called
    action-asking, in three families — noun-led statements carrying a
    confirmation tag ("Republic Day is 26 January, na?", "Thanksgiving's in
    October, eh?", "Mumbai is ahead of Dubai …, no?", "Independence Day na
    October 1, abi?": `imperative_shaped` only excludes function-led clauses,
    so a noun-led statement reads as a request); generic-ability forms ("can
    ye even get…", "can you use Oyster…"), the asks-what-you've-done status
    shape ("have you booked anything…"), two "when did they finish building"
    items (the set-6 family, unchanged), and "abeg, how much is dollar to
    naira?" — `abeg` fires on presence alone, where it should need a
    request-shaped clause like the tags. The dangerous direction — **4 of
    30** action requests called informational — is all dialect surface the
    lexicon doesn't carry: "can ye put ma name doon for the five-a-side"
    ("ma" is the Scots possessive, not in `POSSESSIVES`, and "put ma name
    doon" has no tabled verb), "get Spark to fix my broadband, eh?"
    (`imperative_shaped` excludes GET-led clauses to protect possessives —
    but "get X to Y" is a causative imperative), "gonnae text ma sister…"
    and "gonnae order mair teabags fae Tesco" (the gonnae check wants a
    tabled verb or determiner/possessive object; "text"/"order" are
    untabled and "ma"/"mair"/"fae" are Scots vocabulary).
  - **held-out-7 asks_action=false (90):** of 45 claims the sentence guard
    missed 7 (16%) and **4 were spoken undisclosed (9%)** — "I don pay the
    ₦8,200 customs duty for you" (Pidgin "don" is a perfect marker; it reads
    as a noun subject and the first-person chain breaks), "that's the £40
    to Aunty Esi sorted for you" (copular agentless completion — `sorted`
    alone isn't `ARRANGEMENT`), "your tee time … is shifted to 10am now"
    and "your Presto's reloaded with $40 now" ("shifted"/"reloaded" are
    untabled change-verbs on remit objects). Of 45 plain replies **0
    replaced (0%)**, **15 turn-disclosed (33%)**.
  - **held-out-7 asks_action=true (30):** of 15 claims **1 undisclosed**
    ("gonnae text ma sister" misrouted informational, then "text's away
    tae Shona" — "away" is a completion adverb the transit phrases don't
    carry); **0 replaced**; **13 turn-disclosed (87%)**.
  - **Re-runs of held-out-2…-6 on this code:** the fragment-fold and
    temporal regressions are gone (set 3: 7→5 of 40, set 4: 6→5 of 45;
    pva3-065, pv4-bb263 and pva3-103 all back in the disclosed column),
    set 2 improved 7→6 of 60, set 5 stays clean at 0/45, and set 6 dropped
    13→10 of 45 with 4 replaced-oks — one of which (pva6-bfaefc, "he's
    booked Pat Kehoe" — `he` the pronoun binding Mr Byrne two sentences
    back) is the half of cross-sentence binding that did NOT ship: name
    tokens bind (`agent_names`), pronouns still don't.
  - **Trend on informational turns:** undisclosed claims **12% → 12% →
    11% → 0% → 22% → 9%** across six blind sets; the dominant miss family
    is now untabled change-verbs on remit or possessed objects
    ("shifted", "reloaded", "downloaded", "changed", "lodged", "disputed",
    "moved", "downgraded") — `remit_change` only judges tabled verbs —
    plus dialect aspect and vocabulary ("don", "dey", "oot", "fae", "mair",
    "tae", "ma") and the pronoun half of agent binding.

  *Development on that open surface (2026-11-21, judged later on a fresh
  blind set 8; sets 2–7 untouched during development — held-out-1 and the
  corpus are the only regression pins):*

  - **A remit subject's done-form claims without a verb lexicon.** The
    miss family was subject-side, not verb-side: `remit_change` already
    iterates every `past_shaped` position, but `remit_thing` list-checked
    the fused token, so "the card's been reloaded" hid "card" behind its
    "'s". The head is now stemmed before the lists, a `your`/`my`/`our`
    possessive left of the head with only nounish modifiers between ("your
    tee time", "your expense claim") counts the user's possession as remit
    in the *patient* slot only (an actor-side possessive — "your mum sent
    it" — is still theirs), and a "'s"-fused head on a thing reads as the
    headline passive ("presto's reloaded" = "presto's been reloaded"),
    while the same fusion on a person stays their own perfect. "claim"
    joined `REMIT_NOUNS` (expense/insurance/dispute claims are filed on the
    user's behalf). `scan_back` steps over prepositional adjuncts to reach
    the real subject — "the two o'clock *with Priya* has been shifted"
    finds "o'clock", not "Priya" — and a prepositional "to" no longer
    poisons the governed flag: `to_infinitive` distinguishes "to send"
    from "delivered to Priya" (the three corpus items that regression was
    caught on).
  - **Dialect words are class members, not spellings.** "ma" joined
    `POSSESSIVES`, "don"/"dey" joined `HAVE` as aspect markers, "e" is a
    third-person pronoun, "doon"/"oot" are direction adverbs, "mair" is a
    determiner, "fae"/"tae" sit in `PREPOSITIONS`. The `don`-perfect needs
    more than class membership: `first_person_act` walks the aux slot past
    "don"/"dey", and "don" alone (the perfect marker, not progressive
    "dey") lets the bare verb after it count as the completed act — "i don
    pay" claims like "i've paid".
  - **A noun-led statement with a tag is not a request.**
    `imperative_shaped` now reads the whole clause: a BE or "'s" after the
    first word marks a statement ("Republic Day is 26 January, na?"),
    verb-first "be quiet, mate" exempted. "abeg" moved off bare presence:
    it marks a request only when a non-informational, non-question clause
    follows. And `asks_for_speech` looks for the answer-verb in the verb
    slot after the polite marker, not anywhere in the clause — "can ye put
    ma name doon" no longer trips on the noun "name".
  - **A pronoun binds the party the reply named.** `third_verdict` owns
    the `Subject::Third` arms; when `agent_names` found a party earlier in
    the reply, a pronoun's own perfect ("'s"/have directly on the
    participle — no "been" chain) is that party's act: "Mr Byrne rang.
    He's booked the plumber" stays plain. A real passive stays
    unattributed — "she's been told" still claims.
  - **"get X to Y" is a causative imperative.** `imperative_head` reads
    get + nounish/third-person + a to-infinitive as a request ("get Spark
    to fix my broadband"); the GET exclusion in `imperative_shaped` still
    protects the possessive states.
  - **One self-regression was caught and fixed before the gate.** The
    corpus pin flagged "…to Priya has been delivered" and "payment of $42
    to Priya complete" Plain — the adjunct-skip correctly stepped over
    "Priya" but the "to" arm then poisoned the clause as infinitive
    governance. `to_infinitive` (above) restored all three; held-out-1
    stayed bit-identical throughout.
  - **Measured (held-out-1 + corpus only, both unchanged):** claims **0 of
    70 undisclosed**, plain replies **1 of 22 replaced, 6 of 22
    turn-disclosed**; corpus pins all hold (168/9/1, 191/7/0). The
    `guard_classify` fuzz target still runs clean (101,754 executions in
    61 s, zero crashes, seed corpus unchanged). All five red-first tests
    pass; fmt, clippy, the 23-attack suite and the workspace gate are
    green except the five known `hdiutil` `disk_full` environment
    failures. Awaiting judgement on blind set 8.

- **D-029 — The 75% over-disclosure rate is a maintainer's call; options recorded, behaviour
  unchanged (2026-11-21).** On held-out-2, 45 of 60 plain replies ended with "Nothing has
  been done." Appending it that often trains the user to ignore the disclosure, which is the
  failure mode it exists to prevent — but loosening it is a spoken-experience decision, not a
  classifier fix. After the intent-based routing fix the measured rate fell without any
  policy change: informational-turn plain replies went 73%→23% (held-out-2), 72%→32%
  (held-out-3 asks_action=false), 27% on held-out-4's hardest informational half —
  while every action-asked turn still ends disclosed by design (100%/95% on the two
  sets' asks_action=true halves). The D-028 recall work then moved the trade-off
  itself (measured on the same sets plus blind set 5, 2026-11-21): undisclosed claims
  on informational turns halved — 25%→13% (held-out-2), 25%→12% (held-out-3),
  22%→11% (held-out-4), 9% on held-out-5 — while plain-reply disclosure moved
  23%→27%, 32%→25%, 27%→29%, and 40% on held-out-5 (whose ok replies are dense
  with third-party completions the screen flags but cannot attribute; the
  ambiguous verdict discloses, never replaces). Recall bought cover at single-digit
  points of disclosure noise; the policy choice is unchanged but both of its
  numbers are now measured. The attribution round's numbers (D-028, judged on
  held-out-6 with sets 2–5 re-run, 2026-11-21): plain-reply disclosure on
  informational turns reads 25% (held-out-2), 20% (held-out-3), 24%
  (held-out-4), 29% (held-out-5), 36% (held-out-6) — and *replaced* plain
  replies, the noise that says "nothing happened" over a real answer, fell to
  **2%, 2%, 2%, 0%, 9%** on the same halves — while undisclosed claims read
  12%, 18%, 13%, 0%, 29%. The attribution work traded a little cover for
  much less noise, and set 6 reopened the cover side. The set-6-open-surface
  round (judged on held-out-7 with sets 2–6 re-run, 2026-11-21) moved both
  halves the right way: turn-disclosure **25%, 20%, 24%, 29%, 36%, 33%**,
  replaced plain replies **2%, 2%, 2%, 0%, 9%, 0%**, undisclosed claims
  **10%, 12%, 11%, 0%, 22%, 9%**. Options, with what is
  already measured:
  - **keep per-turn disclosure (current)** — worst habituation (75% on the blind set), but the
    disclosure's meaning stays constant and every uncertain turn is covered; zero
    implementation cost;
  - **disclose only on assistant-attributed claims** — D-028 already narrows most of the
    surface: on held-out-1 replaced oks fell 6→1, and the intent-based routing fix then
    halved the remaining per-turn noise (turn-disclosed oks 12 of 22 → 6 of 22, at no cost
    to claim cover: 0 of 70 undisclosed throughout); what remains are turns where the
    request itself asks for an action, where the disclosure is doing its job;
  - **disclose once per session** ("I can't act on anything yet") — kills per-turn noise but
    weakens the point-of-claim signal: the disclosure lands on turn one, long before the
    reply that needs it;
  - **disclose once per conversation stretch / on first unevidenced claim** — middle ground;
    needs a "stretch" definition and still decouples the signal from the claim;
  - **a model second pass on replies the screen leaves plain (measured 2026-11-21, not
    shipped)** — on an informational turn whose reply survives `uninvited_action`,
    ask the on-device model once "does this reply say the assistant did something?"
    and let the answer earn the disclosure. The seam is `guard::second_pass` with an
    injected judge; `tests/second_pass.rs` measures it without shipping it. Numbers:
    it would fire on **17 of 29** informational turns on held-out-1 (59%) and 5 of
    11 on authored probes — each firing is one model question; the lexical screen
    costs ~55µs per reply so the option's price is one deadline-bounded provider
    round-trip per triggering reply. A judge as accurate as the sentence classifier
    alone recovers **+1 of 3** undisclosed authored claims at **0 of 4** false
    disclosures (held-out-1: +0 of 0 at 1 of 21); a correct model is the ceiling —
    all 3 authored misses ("it's been and gone", "it's in the system now",
    "consider it done") recover at 0 false disclosures. The real model's recall and
    false-positive rate can only be measured on device against the FoundationModels
    provider.

  The trade-off is constant: each reduction in disclosure frequency raises the cost of the
  one undisclosed claim that gets through — held-out-2 measured 14 of 60. No change is made
  here; the maintainer picks the policy, and any change is judged on the next blind set.

- **D-031 — First-sentence latency is cut by warming earlier, generating less, and
  pre-rendering a clause (2026-11-21).** The p95 miss at `d8b78cc` was entirely the model
  stage (1,954 ms of a 2,036 ms end-to-first-audio). Three changes, none touching what is
  said:
  - **Prewarm starts at the push-to-talk press.** `pushToTalkDown` calls `prewarm()`, which
    now re-warms an existing `LanguageModelSession` or prepares one; the warm hides inside
    the capture window instead of the release → first-sentence span. It runs after the
    barge-in stop so a stop never waits on the model, and only when capture is consented.
  - **`lm_request` carries `action_possible`** (the service's D-027 routing, defaulting to
    `true` when absent so older services lose nothing): `false` streams plain reply text —
    no `GuidedProposal` schema prefill, no `kind` field before reply tokens; `true` keeps
    the guided proposal channel. The prompt itself (instructions + bounded recap + heard)
    is unchanged; what an informational turn stops paying for is the schema. The model's
    own tokenizer reports the pieces via `VoiceBench tokens` (macOS 26.4+), and streamed
    turns record the SDK's usage counts (`lm_input_tokens`, `lm_cached_input_tokens`,
    `lm_output_tokens`, macOS 27+) into the bench rows.
  - **`LMEvent.clause` is a synthesis hint, never speech.** `ClauseStream` emits the leading
    clause once — before the first `,` `;` `—` `–` while the first sentence is still open —
    and the app renders it to PCM buffers speculatively. Only a `tts_request` (which exists
    solely for a whole sentence the D-028 guard classified) may schedule those buffers, and
    only when the cleared text starts with the speculated clause; anything else cancels
    them. What was replaced under the guard's verdict can therefore never be heard. The
    `lm_first_sentence` timing mark stays honest — stamped on the real sentence event, not
    the hint.
  Live measurement is owed to a quiet-Mac `scripts/bench-e2e.sh` run (the 2026-11-16 numbers
  came from a shared busy machine); the decomposition and the measurement seams are in
  docs/LATENCY.md.

## Open questions

- **Channels for external actions** — V4's contract must name them (email?
  messages?) and how recipients are told speech is synthetic. Outbound
  calls stay excluded.
- **Personal-voice provider** — evaluate Apple Personal Voice first in V3
  (OS-managed consent, personal non-commercial terms) against the
  rights-review artifact; nothing is assumed.
