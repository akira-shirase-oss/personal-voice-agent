# Latency budget: push-to-talk release → first audio

**Proposed budget (2026-11-16): p95 ≤ 2,000 ms on this Mac**, end of turn (the key is
released) to the first synthesized buffer scheduled on the player. It comes from the
product's need for a reply to start within about two seconds of letting go, and it is
split by stage from the D-014 probes. It is a target, not a claim; misses stay recorded.
Device output latency is measured separately by the live drill (docs/DRILLS.md, Drill 2).

| Stage | Budget p95 | Measured p50 / p95 | |
|---|---|---|---|
| Recognizer finalize (release → heard text sent) | 250 ms | 68 / 106 ms | within |
| Model first sentence (heard → first reply sentence) | 1,500 ms | 1,476 / 1,954 ms | **miss** at p95 |
| Synthesis first audio (request → first buffer scheduled) | 150 ms | 14 / 30 ms | within |
| Service and app hops | 100 ms | included in the stages above | — |
| **End of turn → first audio** | **2,000 ms** | **1,549 / 2,036 ms** | **miss at p95** |

Measured: `perf/results/2026-11-16-210413-e2e-latency-rounds3.md`, from a clean tree at
`d8b78cc` — 17 steady-state turns over the six committed fixtures, audio fed at real time,
warm providers, 1-minute load average 19 → 15 (a shared, busy machine). The cold first turn
took 1,409 ms. The earlier run (`…173730…`, from a dirty tree, load 25–46) measured p50 1,877 /
p95 2,203 ms. In both, the model's first sentence is the whole miss.

## Not voice latency, but missed too: the ledger smokes

What a consent change or a tool call waits for before it proceeds is a durable ledger append.
Both smokes recorded at `d8b78cc` failed their thresholds: saturated append-to-durable p99
554 ms against 50 ms (`perf/results/2026-11-16-205704-ledger-50000.md`) and one durable append
p99 155 ms against 20 ms (`…205803-ledger-sync-2000.md`), at 1-minute load 26–38 while another
process drove the disk. Alternating the old and new builds under the same conditions
(`…210000-ledger-ab-main-vs-branch.md`) measured both at 7–19 ms durable p99, so the misses read
as a contended disk, not a slower writer; they stay recorded as misses until a quiet-machine
run says otherwise.

## What moved the numbers (same bench, same day)

| Change | End → first audio p50 / p95 | Turns degraded |
|---|---|---|
| Before: fresh synthesizer per chunk, one model session for the whole conversation (`perf/results/2026-11-16-173254-e2e-latency-rounds3.md`) | 3,873 / 5,042 ms | 4 of 18 (model first-output deadline) |
| Warm reused synthesizer (D-024) and a fresh prewarmed model session per turn with a bounded recap (D-025) | 1,877 / 2,203 ms | 0 of 18 |

## Where the remaining miss is — and what D-031 changed

The model. `lm_first_sentence` (heard → the service's first `lm_sentence`) decomposes, in
order, into: prompt assembly (µs-scale string building: instructions + a ≤2-exchange recap
+ «heard»), session acquisition (a fresh `LanguageModelSession` per turn — free when
`prewarm()` has already prepared one, paid in the turn when the model was unloaded during
idle), prompt prefill plus time to first token, the tokens from first token to the first
sentence boundary, and the splitter's own wait — a sentence is emitted only when `.`/`!`/`?`
is seen followed by whitespace, i.e. it always arrives one boundary token late. D-031 moved
three of those parts:

- **Session warm starts at the press**, not inside the turn (`Conversation.pushToTalkDown`
  → `prewarm()` prepares or re-warms a session while the user is still talking; the
  previous turn's end-of-response prewarm covers the back-to-back case).
- **Conversational turns skip guided generation.** `lm_request` carries `action_possible`
  (the D-027 routing); `false` streams plain reply text — no schema prefill and no `kind`
  field before reply tokens — while `true` keeps the `GuidedProposal` channel.
  `swift run -c release VoiceBench tokens` reports the model tokenizer's own counts:
  instructions + schema + prompt (guided) vs instructions + prompt (plain), per fixture.
- **The first clause pre-renders.** `LMEvent.clause` hands the leading clause (before the
  first `,` `;` `—` `–` while the first sentence is still open) to the synthesizer at once;
  its buffers sit inert until the service's `tts_request` for the whole classified sentence
  arrives — the D-028 verdict stays the gate, and a replaced or mismatched sentence
  discards them. This shortens time-to-first-audio below the sentence, not the model's own
  `lm_first_sentence` mark (that stays honest: it is stamped on the first real sentence
  event, not on the clause hint).

Whether these bring the p95 back inside 2,000 ms is a live question: the numbers above
were a busy shared machine, and the next bench rows also carry `lm_input_tokens`,
`lm_cached_input_tokens` and `lm_output_tokens` from the stream's usage reports
(macOS 27+). Recognizer and synthesis remain well inside their budgets.
