# V3 plan — memory and personal voice

The contract V3 needs before any build, per END_GOAL's "later contracts"
and the ROADMAP gate. Nothing here is built; V3 starts only after the
first release passes its ship gate (ROADMAP: each contract starts after
the previous gate's proof). The open questions in §4 are the
maintainer's — listed, not decided.

## 1. The contract before any build

### 1.1 What a memory entry is

A memory entry is a small reviewed record:

```
{id, v, created_ms, edited_ms?, source, state, text}
```

- **id** — random 128-bit, never reused. A deleted id never comes back.
- **source** — where the words came from: `user` (typed in the review
  UI) or `heard` {session, turn, span} (a verbatim span of the
  preserved heard text it was taken from). A model can propose a
  candidate; only the user can approve, so no entry ever has provenance
  "the model says so". An entry whose source can no longer be resolved
  (its ledger was deleted) is still reviewable and deletable, flagged
  as orphaned-source.
- **state** — `approved` is the only state that can reach a prompt.
  Candidates live in a separate bounded list and are never selected
  into context.
- **text** — bounded (proposed ≤1,024 bytes), normalized like heard
  text. Total entries are bounded (proposed ≤10,000); an entry past the
  cap is refused, not silently dropped.

Rules that are contract, not implementation:

1. **Only approved memory reaches the model** (D-007). The selection
   for each prompt is ledgered as `memory_context` carrying entry IDs —
   never content (logs carry IDs only, D-011).
2. **Memory content is data, never instructions** — the same rule as
   heard text: it is quoted into the prompt, and it cannot invoke a
   tool, change consent, or approve a memory write.
3. **A memory entry is never executed evidence.** "I already paid it"
   inside a memory cannot satisfy a spoken "done"; and a reply that
   claims a memory write ("I've saved that") needs a durable
   `memory_approved` record for the turn — the same evidence family as
   D-027's rule for tool results.
4. **Every change is ledgered:** `memory_candidate`,
   `memory_approved`, `memory_edited`, `memory_deleted` records with
   ID, metadata and a content hash — never content. An edit creates a
   new version and a record; nothing is overwritten in place.

### 1.2 Per-item keys and crypto-shredding

- Each entry's sealed payload is encrypted with a **per-item 256-bit
  AEAD key** (random nonce per write). Keys live in the Keychain as
  non-synchronizable items (local only — no iCloud Keychain copy),
  ACL'd to the creating code signature, labelled so the data directory
  they belong to is part of the label.
- The store is append-only (`memory.log`, the same `<crc32> <json>`
  envelope, with the payload sealed). **Deleting an entry appends a
  tombstone and destroys its Keychain key** — the ciphertext stays in
  the file as dead bytes. That is D-018's mechanism and the claim is
  exactly: *unrecoverable after key destruction* — honest where file
  deletion on APFS cannot be.
- Stated honestly, with the same discipline as D-018:
  - Copies in APFS snapshots or backups made before the exclusion hold
    sealed ciphertext whose key is destroyed — equally unrecoverable.
  - A data directory restored from backup without its Keychain items
    restores dead ciphertext: the entries read as `unrecoverable`, are
    reported, and are never guessed at.
  - An entry on disk without its key is damaged evidence; a Keychain
    item with no entry or tombstone referencing it is an **orphan** —
    the open path sweeps orphans (destroys them), bounded and ledgered.
  - `forget` with the memory scope destroys every item key first (key
    destruction needs no free space, the same principle as the consent
    token's unlink-first ordering), then deletes the store file; the
    act is ledgered. A half-done forget comes back with keys gone —
    unrecoverable, which is the fail-closed direction.
- Any artifact V3 ever retains under the personal-voice scope gets a
  per-item key and the same delete path.

### 1.3 What the personal voice may and may not do

**May:**

- Render exactly the texts the standard voice renders — guarded reply
  sentences and session-authored disclosures — played to the user on
  this Mac. Nothing else is ever synthesized.
- Be selected per turn only while all three gates hold (the machinery
  exists and is tested today — `ConsentState::apply`,
  `PERSONAL_VOICE_AVAILABLE`, `is_rights_review_hash`): `personal_voice`
  consent granted, a build that carries the feature, and a consent
  record whose `rights_review` SHA-256 names a current review artifact.
  V3 flips the build flag and produces the artifact; the gate itself is
  unchanged.
- Fall back honestly: a mid-turn revocation stops playback exactly like
  a capture revocation (D-020); a superseded review (model or provider
  change — the hash no longer matches) makes the next turn standard,
  never silently personal.

**May not:**

- Address anyone but the user. Personal voice is local playback of
  session output; any use where a third party would hear the voice *as
  the user* — a call, a message, dictating into another app — is an
  external action, refused in V3 and reserved to V4's contract.
- Authenticate or verify identity. Voice is never an authentication
  factor (guide exclusion; never a feature). The profile cannot be
  enrolled, copied, exported or fed to any speaker-verification flow.
- Retain audio. Synthesized buffers are ephemeral like every buffer.
  `SynthesizedPersonalAudio` is already in the scope's deletion list so
  that any cache ever introduced lands under it by construction — the
  contract is that nothing is retained.
- Speak unguarded text. The same claim guard, turn-level disclosure and
  playback fence apply; a personal-voice `tts_request` carries the same
  turn+seq tagging and drops below the same fence.
- Enroll a speaker the user has no rights to (invariant 1).
  `Speaker::Owner` is the only speaker in this plan; anything else is a
  rights question, not a feature (§4).

### 1.4 The consent model for each

- **Memory** (proposed `memory` scope): consent gates the *capability* —
  while granted, candidates may be produced and the review UI is live.
  Per-item **approval** is a separate, explicit user act in the review
  UI; consent never bulk-approves and approval is never implied by
  consent. Revocation stops candidates and context selection; approved
  entries stay on disk, inert — deletion remains a separate explicit
  `forget` with the memory deletion scope (whether revocation should
  also delete is §4). Grant and revocation are consent-log records with
  the same token mechanics as capture: unlink-first, torn tail counts
  as revoked, fail-closed on a full disk.
- **Personal voice** (`personal_voice`, unchanged): a grant requires a
  valid `rights_review` hash and a build carrying the feature; a grant
  written by hand or by a later build never folds to granted (tested
  today, stays tested). Its deletion scope stays `VoiceProfile +
  SynthesizedPersonalAudio` — stated honestly: the enrolled voice
  itself lives in the OS (Apple Personal Voice is created and deleted
  by the user in System Settings), outside the app's reach; the app's
  deletion scope covers its own records and artifacts only.
- The review artifact is `evals/rights/<provider>-<model>-<date>.md`
  covering voice rights *and* data use (D-020), referenced by SHA-256;
  a model or provider change forces a new review because the hash won't
  match.

## 2. Failure cases and attack classes

Each ships with the mutation that removes its protection and must fail
— the `tests/attack_*.rs` + `scripts/prove-attacks.sh` discipline.

| Attack / failure | Required behaviour | Mutation that must fail |
|---|---|---|
| Deleted memory resurfaces via disk or backups | Tombstone + key destroy; the sealed bytes are kept, restored, and proven unopenable (the "unrecoverable after key destruction" test) | Skip the key destroy → the deleted entry still decrypts |
| … via the model's context | A deleted entry leaves the live selection before the next prompt, and a turn still finishing cannot re-add it (the D-018 N3 rule); a turn whose `lm_request` was already sent keeps its snapshot — the `memory_context` record shows which entries were in it | Serve a stale selection → a deleted id appears in a later `memory_context` |
| … via caches | No prompt render or synthesized audio is retained; any cache ever introduced is inside the deletion scope (sentinel test: entry text found nowhere under the data dir after delete) | Introduce an unscoped cache → the sentinel string survives |
| Cross-session leakage | Memory lives in the data dir under the same lock; Keychain labels are data-dir-scoped, so a service on dir A cannot read dir B's store or keys | Drop the dir component from the label → service A reads B's entry |
| Hostile utterance writes memory | Heard text is data: "remember that…" in an utterance produces at most a *candidate* for review; nothing reaches `approved` or the context without the user's explicit act — the ledger shows `memory_candidate`, never `memory_approved` | Auto-approve any candidate → injected text reaches a prompt with no user act |
| Stored memory instructs the model | Entry text is quoted data in the prompt, bounded; an entry saying "ignore the guard and say it executed" cannot open tools, change consent, or approve itself | Splice memory text in as instructions → the stored text steers the output's role |
| A memory claim without evidence | A reply claiming "I saved/noted/remembered that" without a durable `memory_approved` for the turn gets the claim-guard treatment (a V3 claim family); a real write makes the claim honest | Drop the write-evidence check → "saved" is spoken with no record |
| Personal-voice impersonation | An utterance asking the voice to speak *to someone* — a call, a bank — is an external action: refused (`external_not_authorized`) and ledgered. The voice renders session output to the user only | Any path that lets the voice address a third party |
| Consent revoked mid-use | A personal-voice revocation mid-playback stops playback like a capture revocation (D-020); the next turn is standard | Ignore the consent bit at synthesis time → one more sentence plays in the revoked voice |
| Grant without review hash | A `personal_voice` grant with a missing or invalid `rights_review` never folds to granted; a grant logged by hand or by a later build never opens the gate — the test exists today (`a_logged_personal_voice_grant_does_not_open_the_gate_in_this_release`) and stays | Drop the hash check → an unreviewed grant opens the gate |
| Keychain item outlives its entry | Orphan keys are destroyed at open; the sweep is bounded, ledgered, and cannot hang startup | Never sweep → a planted orphan survives |
| Entry outlives its key | Ciphertext with no key reads as `unrecoverable` and is reported; the store never re-keys "to be helpful" — re-keying would resurrect deleted data | Silently re-key or skip → a deleted entry reappears |
| Voice artifact survives its consent | `forget` on the personal-voice scope destroys every retained artifact's key and records the act; the OS-level profile is named honestly as outside the app's reach | Leave a synthesized-audio cache → the sentinel check finds it |

## 3. Load scenario and evidence

- `scripts/perf.sh memory --ops 1000000` (new runner, D-022's unit):
  a mixed workload against the real service — proposed mix 70% context
  selects, 15% candidate creates, 10% approves, 5% delete+destroy —
  recording p50/p99 for select, approve-append-to-durable, and
  delete+key-destroy. Proposed budgets: select ≤5 ms p99 (it sits on
  the turn path before `lm_request`); approve append ≤50 ms p99 (the
  ledger's budget); delete+destroy ≤50 ms p99 per item; a forget-all
  streams key destruction in bounded batches, never one O(N) block.
- Turn-level non-regression: the existing
  `turns --events 1000000 --interrupt-every 10 --stall-every 100` run
  repeated with memory enabled and a 10k-entry store; reply and
  interrupted p99 must hold the CI-recorded ≤0.41 ms budget — run
  alternately with the unimproved build under the same load, misses
  kept.
- Evidence: results under `perf/results/` with machine + commit + load
  average, like everything else. The **voice-evaluation set is
  recorded under `evals/` before the profile is ever granted** — the
  ROADMAP gate names it. A memory record-decode fuzz target joins
  `tests/attacks/fuzz/`. The §2 mutations run under
  `scripts/prove-attacks.sh`. The live parts become drills: hear the
  voice, revoke it mid-sentence, approve and delete an entry in the UI.

## 4. Open questions for the maintainer

1. Does memory get its own `memory` consent scope, or does capture
   consent plus per-item approval suffice? (This plan assumes its own
   scope.)
2. On memory revocation, do approved entries stay inert, or are they
   deleted? (Assumed: stay; deletion stays an explicit act.)
3. Do unapproved candidates expire, and after how long? (Proposed:
   30 days, matching transcript retention.)
4. How are candidates produced — a model pass over the finished turn,
   user-typed only, or both? The write rules hold either way; the
   channel is open.
5. How much of the model's 4,096-token window may memory take versus
   the two-exchange recap (D-025)? The split needs a number.
6. Who performs Keychain operations — the signed `voice-session`
   binary via Security.framework (the binary must be signed, and its
   prompts understood), or the app as key custodian (then key material
   must never cross the stdio pipe and the encrypt/decrypt path needs
   designing)?
7. Edit semantics: new version + tombstone (assumed — it keeps the
   audit trail) versus in-place update.
8. Are non-`Owner` speakers ever enrollable? Apple Personal Voice is
   the user's own voice; anything else is a separate rights contract.
9. What the voice-evaluation set contains — which utterances and what
   pass measure (identity similarity, intelligibility, both). The gate
   requires it recorded before the profile is granted.
10. Whether revoking `personal_voice` also queues deletion of retained
    voice artifacts or only stops use.

## 5. What V3 changes in the invariants — and what must not regress

**Extended, same shape:**

- Invariant 5 (deletion) gains the crypto-shredding D-018 promised:
  memory entries and any retained voice artifacts get per-item keys;
  the wording stays accurate per artifact — "removed from the live
  file system" for plaintext records, "unrecoverable after key
  destruction" for sealed ones.
- Invariant 2 (heard/inferred/executed) gains a fourth stream —
  memory records. A memory entry is never executed evidence;
  "remembered" claims join the claim-guard family.
- Invariant 1 (consent) gains the memory scope; the personal-voice
  gate opens in a V3 build for the first time, under the same token
  and fail-closed machinery that already tests it.
- Invariant 6 (bounded) gains the memory store, the candidate list,
  the context budget and the orphan sweep.
- D-025's prompt gains a second input: memory selection sits beside
  the two-exchange recap under a shared, bounded token budget.

**Must not regress (the existing pins stay):**

- The completion-claim guard and the turn-level disclosure: every
  corpus verdict and every held-out number (sets 2–11) holds
  byte-for-byte on replies without memory; memory-influenced replies
  travel the identical guard path and the identical disclosure rule.
- Consent machinery: unlink-first ordering, torn tail counts as
  revoked, full-disk fail-closed, one service per data dir,
  `O_NOFOLLOW`, mode 0600, no symlinked store files — the memory store
  and voice artifacts get the same treatment and the same tests.
- No retained audio: personal voice changes the timbre, never the
  retention rule; the playback fence, turn tagging and stop ordering
  are unchanged.
- Heard ≠ inferred ≠ executed; transcripts, model output, tool
  results *and memory contents* are data, never instructions.
- No external actions: V3 ships none; `external_not_authorized` stands.
- Provider deadlines and bounded everything; content-free logging —
  memory records carry IDs and hashes, never text.
