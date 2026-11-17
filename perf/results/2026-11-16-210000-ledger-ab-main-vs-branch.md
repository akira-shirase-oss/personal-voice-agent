# Ledger A/B: main (c744552) against fix/review-2026-11-16 (d8b78cc) — no regression

- Date: 2026-11-16, about 20:59–21:00 local
- Machine: Apple M2 Pro, 32 GB, 12 cores; macOS 27.2.1 (26B131)
- Builds: `main` at c744552 (git worktree, own target dir) and the branch at d8b78cc, both
  `cargo build --release --locked -p voice-testkit --bin voice-perf` from clean trees
- Load average (1 / 5 / 15 min) at the end: 18.22 26.76 37.12 (shared with other builds; disk busy)

Why: the recorded smokes at d8b78cc (`205704-ledger-50000`, `205803-ledger-sync-2000`) failed
their thresholds (saturated p99 554 ms, 17k appends/s; durable p99 155 ms, 35/s) while
`iostat` showed other processes driving the disk at up to 3,751 transfers/s. The two binaries
were then run alternately, three rounds each, so both met the same conditions.

| Round | Build | One durable append p50 / p99 (ms, 300 appends) | Saturated appends/s, p99 ms (20,000 appends) |
|---|---|---|---|
| 1 | main | 8.8 / 19.3 | 141,390 / 36.0 |
| 1 | branch | 9.2 / 19.3 | 124,288 / 47.6 |
| 2 | main | 5.9 / 8.1 | 173,531 / 32.4 |
| 2 | branch | 6.0 / 7.1 | 143,695 / 35.2 |
| 3 | main | 6.0 / 15.9 | 134,428 / 41.8 |
| 3 | branch | 6.0 / 11.9 | 161,285 / 33.1 |

Reading: the two builds overlap round for round; the failed smokes measured a contended disk,
not a slower writer. The failed records are kept as they are.
