# Latency budget: push-to-talk release → first audio

**Proposed budget (2026-11-16): p95 ≤ 2,000 ms on this Mac**, end of turn (the key is
released) to the first synthesized buffer scheduled on the player. It comes from the
product's need for a reply to start within about two seconds of letting go, and it is
split by stage from the D-014 probes. It is a target, not a claim; misses stay recorded.
Device output latency is measured separately by the live drill (docs/DRILLS.md, Drill 2).

| Stage | Budget p95 | Measured p50 / p95 | |
|---|---|---|---|
| Recognizer finalize (release → heard text sent) | 250 ms | 67 / 103 ms | within |
| Model first sentence (heard → first reply sentence) | 1,500 ms | 1,778 / 2,073 ms | **miss** |
| Synthesis first audio (request → first buffer scheduled) | 150 ms | 17 / 31 ms | within |
| Service and app hops | 100 ms | included in the stages above | — |
| **End of turn → first audio** | **2,000 ms** | **1,877 / 2,203 ms** | **miss at p95** |

Measured: `perf/results/2026-11-16-173730-e2e-latency-rounds3.md` — 17 steady-state turns
over the six committed fixtures, audio fed at real time, warm providers, load average 25–46
(a shared, busy machine). The cold first turn took 1,551 ms.

## What moved the numbers (same bench, same day)

| Change | End → first audio p50 / p95 | Turns degraded |
|---|---|---|
| Before: fresh synthesizer per chunk, one model session for the whole conversation (`perf/results/2026-11-16-173254-e2e-latency-rounds3.md`) | 3,873 / 5,042 ms | 4 of 18 (model first-output deadline) |
| Warm reused synthesizer (D-024) and a fresh prewarmed model session per turn with a bounded recap (D-025) | 1,877 / 2,203 ms | 0 of 18 |

## Where the remaining miss is

The model. Its first sentence needs the guided-generation preamble plus a whole sentence.
Next levers, each to be measured with `scripts/bench-e2e.sh` before it is kept: stream a
reply as plain text when the request is conversational (guided generation only where an
action is possible), shorter instructions, and a shorter first sentence. Recognizer and
synthesis are well inside their budgets.
