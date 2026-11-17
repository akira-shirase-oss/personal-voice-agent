//! Load scenarios for the session service (D-022). Prints one JSON object per run.
//!
//! - `ledger --records N`: N appends through the ledger writer, each acknowledged when
//!   durable; latency is submit → durable ack, under saturation (the producer never waits).
//! - `turns --events N [--interrupt-every K] [--stall-every M]`: scripted turns through the
//!   real session actor and protocol on an in-memory pipe, with stub providers, until the
//!   actor has processed N inputs. Every K-th turn is interrupted mid-speech; every M-th turn's
//!   model stalls and hits its (shortened) deadline.
//!
//! Thresholds fail the run (exit 1); the JSON is printed either way so a miss is evidence.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "load runner: a panic is the failure report; casts are on sample counts"
)]

use serde_json::json;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use voice_domain::TurnId;
use voice_domain::proposal::{Proposal, ProposalKind};
use voice_domain::protocol::{ClientMessage as C, Outcome, ServiceMessage as S};
use voice_domain::record::Record;
use voice_session::Config;
use voice_session::log::Log;
use voice_session::tools::NoTools;
use voice_testkit::{grant_capture, heard, lm_done, reply_turn, start};

/// Regression guards, recorded with every run.
const LEDGER_P99_MAX_MS: f64 = 50.0;
/// One durable append on an idle writer: the cost a tool call or consent change pays.
const SYNC_P99_MAX_MS: f64 = 20.0;
const TURN_P99_MAX_MS: f64 = 10.0;

type Pending = (
    Instant,
    tokio::sync::oneshot::Receiver<Result<u64, voice_session::store::WriteError>>,
);

fn percentile(sorted: &[Duration], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    // Nearest-rank on the sorted samples, in integer arithmetic (p is given in permille).
    let permille = (p * 1_000.0).round().clamp(0.0, 1_000.0) as usize;
    let idx = (sorted.len() - 1) * permille / 1_000;
    sorted[idx].as_secs_f64() * 1_000.0
}

fn stats(mut v: Vec<Duration>) -> serde_json::Value {
    v.sort_unstable();
    json!({
        "count": v.len(),
        "p50_ms": percentile(&v, 0.50),
        "p95_ms": percentile(&v, 0.95),
        "p99_ms": percentile(&v, 0.99),
        "p999_ms": percentile(&v, 0.999),
        "max_ms": v.last().map_or(0.0, |d| d.as_secs_f64() * 1_000.0),
    })
}

fn arg(args: &[String], name: &str, default: u64) -> u64 {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

async fn ledger(records: u64) -> (serde_json::Value, bool) {
    let dir = voice_testkit::temp_data_dir("perf-ledger");
    let opened = tokio::task::spawn_blocking({
        let dir = dir.to_path_buf();
        move || voice_session::store::open(&dir, records + 16)
    })
    .await
    .unwrap()
    .unwrap();
    let writer = opened.ledger;
    let (tx, mut rx) = mpsc::channel::<Pending>(8_192);
    let collector = tokio::spawn(async move {
        let mut lat = Vec::with_capacity(usize::try_from(records).unwrap_or(0));
        while let Some((t0, ack)) = rx.recv().await {
            ack.await.unwrap().unwrap();
            lat.push(Instant::now().duration_since(t0));
        }
        lat
    });
    let started = Instant::now();
    for i in 0..records {
        // A realistic record: heard text of a short utterance.
        let rec = Record::Heard {
            turn: TurnId(i / 8 + 1),
            call: voice_domain::CallId(i + 1),
            text: "remind me to call the dentist tomorrow at nine".into(),
            min_confidence: Some(0.93),
        };
        let t0 = Instant::now();
        let ack = writer.submit(rec, Duration::from_secs(10)).await.unwrap();
        tx.send((t0, ack)).await.unwrap();
    }
    drop(tx);
    let lat = collector.await.unwrap();
    let elapsed = started.elapsed();
    let _ = writer.close(Duration::from_secs(10)).await;
    let bytes = std::fs::metadata(&opened.ledger_path).map_or(0, |m| m.len());
    let _ = std::fs::remove_dir_all(&dir);
    let s = stats(lat);
    let pass = s["p99_ms"].as_f64().unwrap_or(f64::MAX) <= LEDGER_P99_MAX_MS;
    let out = json!({
        "scenario": "ledger",
        "records": records,
        "elapsed_s": elapsed.as_secs_f64(),
        "appends_per_s": records as f64 / elapsed.as_secs_f64(),
        "bytes": bytes,
        "append_to_durable": s,
        "threshold": { "p99_ms_max": LEDGER_P99_MAX_MS },
        "pass": pass,
    });
    (out, pass)
}

/// Sequential durable appends: each waits for its own sync before the next is sent.
async fn ledger_sync(records: u64) -> (serde_json::Value, bool) {
    let dir = voice_testkit::temp_data_dir("perf-sync");
    let opened = tokio::task::spawn_blocking({
        let dir = dir.to_path_buf();
        move || voice_session::store::open(&dir, records + 16)
    })
    .await
    .unwrap()
    .unwrap();
    let writer = opened.ledger;
    let mut lat = Vec::with_capacity(usize::try_from(records).unwrap_or(0));
    let started = Instant::now();
    for i in 0..records {
        let t0 = Instant::now();
        writer
            .append_durable(Record::TurnOpened { turn: TurnId(i + 1) }, Duration::from_secs(10))
            .await
            .unwrap();
        lat.push(t0.elapsed());
    }
    let elapsed = started.elapsed();
    let _ = writer.close(Duration::from_secs(10)).await;
    let _ = std::fs::remove_dir_all(&dir);
    let s = stats(lat);
    let pass = s["p99_ms"].as_f64().unwrap_or(f64::MAX) <= SYNC_P99_MAX_MS;
    let out = json!({
        "scenario": "ledger-sync",
        "records": records,
        "elapsed_s": elapsed.as_secs_f64(),
        "durable_appends_per_s": records as f64 / elapsed.as_secs_f64(),
        "append_to_durable": s,
        "threshold": { "p99_ms_max": SYNC_P99_MAX_MS },
        "pass": pass,
    });
    (out, pass)
}

/// What one scripted turn did, and how long the client waited for its close.
enum Kind {
    Reply,
    Interrupted,
    Stall,
}

async fn interrupted_turn(stub: &mut voice_testkit::Stub, late_expected: &mut u64) -> S {
    let (_, lm) = heard(stub, "tell me a long story", 0.95).await;
    stub.send(&C::LmKind {
        call: lm,
        kind: ProposalKind::Reply,
    })
    .await;
    stub.send(&C::LmSentence {
        call: lm,
        index: 0,
        text: "Once upon a time.".into(),
    })
    .await;
    let call = match stub
        .until(|m| matches!(m, S::TtsRequest { .. } | S::TurnClosed { .. }))
        .await
        .0
    {
        S::TtsRequest { call, .. } => call,
        closed => return closed,
    };
    stub.send(&C::TtsStarted { call }).await;
    stub.send(&C::Interrupt).await;
    let closed = stub.until(|m| matches!(m, S::TurnClosed { .. })).await.0;
    // The old turn's providers keep going for a moment: all of it must be dropped.
    stub.send(&C::LmSentence {
        call: lm,
        index: 1,
        text: "There was a dragon.".into(),
    })
    .await;
    stub.send(&lm_done(
        lm,
        Proposal::Reply {
            reply: "Once upon a time. There was a dragon.".into(),
        },
    ))
    .await;
    stub.send(&C::TtsFinished { call }).await;
    *late_expected += 3;
    closed
}

async fn turns(events: u64, interrupt_every: u64, stall_every: u64) -> (serde_json::Value, bool) {
    const LM_DEADLINE_MS: u64 = 50;
    let dir = voice_testkit::temp_data_dir("perf-turns");
    let mut config = Config::new(dir.to_path_buf(), NoTools, Log::off());
    // Stalled turns hit a shortened model deadline. A turn that was not meant to stall but
    // degrades anyway is a deadline missed under load, and fails the run.
    config.limits.lm_first_ms = LM_DEADLINE_MS;
    config.max_records = u64::MAX;
    let dir = config.data_dir.clone();
    let (mut stub, _, handle) = start(config).await;
    grant_capture(&mut stub).await;
    let (mut reply_lat, mut interrupt_lat, mut stall_lat) = (Vec::new(), Vec::new(), Vec::new());
    let mut outcomes = std::collections::BTreeMap::<String, u64>::new();
    let (mut n, mut stalls, mut late_expected) = (0_u64, 0_u64, 0_u64);
    let started = Instant::now();
    // A scripted turn is about 11 actor inputs; stop once the actor has processed `events`.
    while n * 11 < events {
        n += 1;
        let kind = if stall_every > 0 && n.is_multiple_of(stall_every) {
            Kind::Stall
        } else if interrupt_every > 0 && n.is_multiple_of(interrupt_every) {
            Kind::Interrupted
        } else {
            Kind::Reply
        };
        let t0 = Instant::now();
        let closed = match kind {
            Kind::Stall => {
                stalls += 1;
                heard(&mut stub, "are you there", 0.95).await;
                stub.until(|m| matches!(m, S::TurnClosed { .. })).await.0
            }
            Kind::Interrupted => interrupted_turn(&mut stub, &mut late_expected).await,
            Kind::Reply => {
                reply_turn(
                    &mut stub,
                    "what time is it in Tokyo",
                    &["It is late evening.", "About nine."],
                )
                .await
            }
        };
        let took = t0.elapsed();
        match kind {
            Kind::Reply => reply_lat.push(took),
            Kind::Interrupted => interrupt_lat.push(took),
            Kind::Stall => stall_lat.push(took),
        }
        if let S::TurnClosed { outcome, .. } = closed {
            *outcomes.entry(format!("{outcome:?}").to_lowercase()).or_default() += 1;
        }
    }
    stub.send(&C::Shutdown).await;
    let summary = handle.await.unwrap().unwrap();
    let elapsed = started.elapsed();
    stub.close().await;
    let _ = std::fs::remove_dir_all(&dir);
    let (reply, interrupt, stall) = (stats(reply_lat), stats(interrupt_lat), stats(stall_lat));
    let degraded = outcomes.get("degraded").copied().unwrap_or(0);
    let unexpected_degraded = degraded.saturating_sub(stalls);
    let p99 = |v: &serde_json::Value| v["p99_ms"].as_f64().unwrap_or(f64::MAX);
    let pass = p99(&reply) <= TURN_P99_MAX_MS
        && p99(&interrupt) <= TURN_P99_MAX_MS
        && summary.late_dropped >= late_expected
        && unexpected_degraded == 0
        && outcomes.get("completed").copied().unwrap_or(0) > 0;
    let _ = Outcome::Completed;
    let out = json!({
        "scenario": "turns",
        "turns": n,
        "actor_inputs": summary.inputs,
        "elapsed_s": elapsed.as_secs_f64(),
        "inputs_per_s": summary.inputs as f64 / elapsed.as_secs_f64(),
        "outcomes": outcomes,
        "stalls_scripted": stalls,
        "unexpected_degraded": unexpected_degraded,
        "late_results_dropped": summary.late_dropped,
        "refused": summary.refused,
        "reply_turn_ms": reply,
        "interrupted_turn_ms": interrupt,
        "stalled_turn_ms": stall,
        "stall_deadline_ms": LM_DEADLINE_MS,
        "threshold": {
            "reply_and_interrupted_p99_ms_max": TURN_P99_MAX_MS,
            "late_dropped_min": late_expected,
            "unexpected_degraded_max": 0
        },
        "pass": pass,
    });
    (out, pass)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (out, pass) = match args.get(1).map(String::as_str) {
        Some("ledger") => runtime.block_on(ledger(arg(&args, "--records", 1_000_000))),
        Some("ledger-sync") => runtime.block_on(ledger_sync(arg(&args, "--records", 10_000))),
        Some("turns") => runtime.block_on(turns(
            arg(&args, "--events", 1_000_000),
            arg(&args, "--interrupt-every", 10),
            arg(&args, "--stall-every", 100),
        )),
        _ => {
            eprintln!(
                "usage: voice-perf ledger [--records N] | ledger-sync [--records N] | turns [--events N] [--interrupt-every K] [--stall-every M]"
            );
            std::process::exit(2);
        }
    };
    println!("{}", serde_json::to_string_pretty(&out).unwrap());
    if !pass {
        std::process::exit(1);
    }
}
