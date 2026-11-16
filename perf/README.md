# Load and latency evidence

Every run is recorded by `scripts/perf.sh` under `results/` with machine, OS, commit and load
average before and after, whether it passes or fails. Misses are kept. Laptop numbers on a
shared machine are development observations, not capacity claims.

| Scenario | What it measures | Threshold (fails the run) |
|---|---|---|
| `ledger --records N` | N appends through the ledger writer with the producer never waiting: submit → durable ack under saturation (group commit) | p99 ≤ 50 ms |
| `ledger-sync --records N` | One durable append at a time on an idle writer: what a tool call or consent change pays before it proceeds | p99 ≤ 20 ms |
| `turns --events N --interrupt-every K --stall-every M` | Scripted turns through the real actor and protocol with stub providers: reply turns, interrupted turns (late output must be dropped), stalled turns (must degrade at a 50 ms model deadline) | reply and interrupted turn p99 ≤ 10 ms; every late result dropped; no degraded turn other than the scripted stalls |

Full runs (D-022) are 1,000,000 ledger appends and 1,000,000 actor inputs. They run only when
`uptime` shows a load average below about 6 (12 cores shared with other builds); otherwise a
smoke run is recorded and the full run is listed as owed in `docs/ROADMAP.md`.
