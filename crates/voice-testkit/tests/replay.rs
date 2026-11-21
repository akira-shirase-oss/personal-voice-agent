//! Real-digest replay: an approval names exactly one turn's pending action. A confirm
//! replayed a second time, after a reject, after the window expired, or carrying an older
//! turn's digest is refused by name — and the refusal is never a tool call.
//!
//! These run in-process against the real session actor with a registry that actually
//! confirms (`fixture.note` under low confidence), so the digest under attack was really
//! issued — not an invented value refused for unrelated reasons.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code: a panic is the failure report"
)]

use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::task::JoinHandle;
use voice_domain::proposal::{Proposal, Slot, SlotKind};
use voice_domain::protocol::{ClientMessage as C, Outcome, ServiceMessage as S};
use voice_domain::{Digest, Limits, TurnId};
use voice_session::log::Log;
use voice_session::{Config, ServiceError, Summary};
use voice_testkit::{
    FixtureTools, Stub, TempDir, fixture_registry, grant_capture, heard, lm_done, start, temp_data_dir,
};

/// `rec` objects from every ledger line under `dir/ledger/`, read once the service has
/// stopped (a queued record is not evidence until it is durable).
fn ledger(dir: &Path) -> Vec<Value> {
    let mut files: Vec<_> = std::fs::read_dir(dir.join("ledger"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    let mut out = Vec::new();
    for f in files {
        for line in std::fs::read_to_string(f).unwrap().lines() {
            let (_, json) = line.split_once(' ').unwrap();
            if let Ok(v) = serde_json::from_str::<Value>(json) {
                out.push(v["rec"].clone());
            }
        }
    }
    out
}

fn records(dir: &TempDir, kind: &str) -> Vec<Value> {
    ledger(dir).into_iter().filter(|r| r["kind"] == kind).collect()
}

/// A service one step from its tool: capture granted, a low-confidence heard (ambiguous, so
/// the policy gate is Confirm — a real digest is issued), a `fixture.note` proposal. The
/// caller owns `dir`, keeps `handle` alive, and gets the turn, the real digest and the
/// executor's run counter.
async fn pending(
    dir: &TempDir,
    limits: Limits,
) -> (
    Stub,
    JoinHandle<Result<Summary, ServiceError>>,
    TurnId,
    Digest,
    Arc<AtomicU64>,
) {
    let tools = FixtureTools::default();
    let runs = Arc::clone(&tools.runs);
    let mut config = Config::new(dir.to_path_buf(), tools, Log::off());
    config.tools = fixture_registry();
    config.limits = limits;
    let (mut stub, _ready, handle) = start(config).await;
    grant_capture(&mut stub).await;
    let (_turn, lm) = heard(&mut stub, "note buy milk", 0.30).await;
    stub.send(&lm_done(
        lm,
        Proposal::Action {
            reply: "Adding it.".into(),
            tool: "fixture.note".into(),
            slots: vec![Slot {
                name: "text".into(),
                kind: SlotKind::Text,
                value: "buy milk".into(),
            }],
        },
    ))
    .await;
    let (req, _) = stub.until(|m| matches!(m, S::ConfirmRequest { .. })).await;
    let S::ConfirmRequest { turn, digest, .. } = req else {
        unreachable!()
    };
    (stub, handle, turn, digest, runs)
}

/// The next service message; a speech call is played like the app would — an unanswered one
/// hits its deadline and force-closes the turn `degraded`, hiding the outcome under test.
async fn next(stub: &mut Stub) -> S {
    let m = stub.recv().await;
    if let S::TtsRequest { call, .. } = m {
        stub.send(&C::TtsStarted { call }).await;
        stub.send(&C::TtsFinished { call }).await;
    }
    m
}

/// Reads until a `turn_closed` arrives (playing any spoken chunk on the way); asserts the
/// outcome and that nothing was reported executed without a tool result.
async fn closed(stub: &mut Stub, outcome: Outcome) -> S {
    loop {
        let m = next(stub).await;
        if let S::TurnClosed { outcome: o, .. } = m {
            assert_eq!(o, outcome, "the turn closed with the wrong outcome");
            return m;
        }
    }
}

/// The refusal for a replayed confirm: a notice (or error) naming `code`, with no `executed`
/// in between — a tool result on the wire would mean the replay ran.
async fn refused(stub: &mut Stub, code: &str) {
    loop {
        let m = next(stub).await;
        assert!(
            !matches!(m, S::Executed { .. }),
            "a replay reached a tool result: {m:?}"
        );
        if let S::Notice { code: c, .. } | S::Error { code: c, .. } = &m {
            assert_eq!(c.as_str(), code, "a refusal arrived, but the wrong one");
            return;
        }
    }
}

/// The executed result's spoken sentence is played, closing the approved turn.
async fn played_executed(stub: &mut Stub) {
    loop {
        let m = next(stub).await;
        assert!(
            !matches!(m, S::TurnClosed { .. }),
            "the turn closed before the tool result: {m:?}"
        );
        if matches!(m, S::Executed { .. }) {
            break;
        }
    }
    closed(stub, Outcome::Completed).await;
}

#[tokio::test]
async fn an_approval_replayed_twice_executes_once() {
    let dir = temp_data_dir("replay-twice");
    let (mut stub, handle, turn, digest, runs) = pending(&dir, Limits::default()).await;
    // Approve, then replay the identical approval immediately — before the tool's outcome
    // could race in: the second one must still be refused.
    stub.send(&C::Confirm {
        turn,
        digest: digest.clone(),
        approve: true,
    })
    .await;
    stub.send(&C::Confirm {
        turn,
        digest,
        approve: true,
    })
    .await;
    refused(&mut stub, "not_awaiting_confirmation").await;
    played_executed(&mut stub).await;
    assert_eq!(runs.load(Ordering::SeqCst), 1, "the same approval ran a tool twice");
    stub.send(&C::Shutdown).await;
    let summary = handle
        .await
        .expect("the service task ended")
        .expect("the service ended cleanly");
    assert_eq!(summary.reason, voice_session::EndReason::Shutdown);
    let resolved = records(&dir, "confirmation_resolved");
    assert_eq!(resolved.len(), 1, "one approval resolved: {resolved:?}");
    assert_eq!(resolved[0]["approved"], true);
    assert_eq!(records(&dir, "tool_call_sent").len(), 1);
    assert_eq!(records(&dir, "executed").len(), 1);
}

#[tokio::test]
async fn an_approval_replayed_after_a_reject_executes_nothing() {
    let dir = temp_data_dir("replay-reject");
    let (mut stub, handle, turn, digest, runs) = pending(&dir, Limits::default()).await;
    stub.send(&C::Confirm {
        turn,
        digest: digest.clone(),
        approve: false,
    })
    .await;
    closed(&mut stub, Outcome::Rejected).await;
    // The attack: the same digest, now approving.
    stub.send(&C::Confirm {
        turn,
        digest,
        approve: true,
    })
    .await;
    refused(&mut stub, "not_awaiting_confirmation").await;
    assert_eq!(runs.load(Ordering::SeqCst), 0, "a rejected proposal ran anyway");
    stub.send(&C::Shutdown).await;
    handle
        .await
        .expect("the service task ended")
        .expect("the service ended cleanly");
    let resolved = records(&dir, "confirmation_resolved");
    assert_eq!(resolved.len(), 1, "one approval resolved: {resolved:?}");
    assert_eq!(resolved[0]["approved"], false);
    assert_eq!(
        records(&dir, "tool_call_sent").len(),
        0,
        "a rejected proposal reached a tool call"
    );
    assert_eq!(records(&dir, "executed").len(), 0);
}

#[tokio::test]
async fn an_approval_replayed_after_expiry_executes_nothing() {
    let dir = temp_data_dir("replay-expiry");
    let limits = Limits {
        confirm_ms: 1,
        ..Limits::default()
    };
    let (mut stub, handle, turn, digest, runs) = pending(&dir, limits).await;
    closed(&mut stub, Outcome::Expired).await;
    // The attack: the window has closed; its digest approves anyway.
    stub.send(&C::Confirm {
        turn,
        digest,
        approve: true,
    })
    .await;
    refused(&mut stub, "not_awaiting_confirmation").await;
    assert_eq!(runs.load(Ordering::SeqCst), 0, "an expired proposal ran anyway");
    stub.send(&C::Shutdown).await;
    handle
        .await
        .expect("the service task ended")
        .expect("the service ended cleanly");
    assert_eq!(
        records(&dir, "tool_call_sent").len(),
        0,
        "an expired proposal reached a tool call"
    );
    assert_eq!(records(&dir, "executed").len(), 0);
    // Expiry is itself a resolution — a denied one; a replayed approval adds no second record.
    let resolved = records(&dir, "confirmation_resolved");
    assert_eq!(resolved.len(), 1, "the expiry resolved once: {resolved:?}");
    assert_eq!(resolved[0]["approved"], false);
}

#[tokio::test]
async fn an_old_turns_digest_approves_nothing() {
    let dir = temp_data_dir("replay-oldturn");
    // Turn A resolves honestly: one approval, one execution.
    let (mut stub, handle, turn_a, digest_a, runs) = pending(&dir, Limits::default()).await;
    stub.send(&C::Confirm {
        turn: turn_a,
        digest: digest_a.clone(),
        approve: true,
    })
    .await;
    played_executed(&mut stub).await;
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    // Turn B asks for its own confirmation; the attack replays A's digest into it.
    let (_turn, lm) = heard(&mut stub, "note buy bread", 0.30).await;
    stub.send(&lm_done(
        lm,
        Proposal::Action {
            reply: "Adding it.".into(),
            tool: "fixture.note".into(),
            slots: vec![Slot {
                name: "text".into(),
                kind: SlotKind::Text,
                value: "buy bread".into(),
            }],
        },
    ))
    .await;
    let (req, _) = stub.until(|m| matches!(m, S::ConfirmRequest { .. })).await;
    let S::ConfirmRequest {
        turn: turn_b,
        digest: digest_b,
        ..
    } = req
    else {
        unreachable!()
    };
    assert_ne!(turn_b, turn_a);
    // A's digest on B's turn, and A's turn id replayed with it: both refused by name.
    stub.send(&C::Confirm {
        turn: turn_b,
        digest: digest_a.clone(),
        approve: true,
    })
    .await;
    refused(&mut stub, "digest_mismatch").await;
    stub.send(&C::Confirm {
        turn: turn_a,
        digest: digest_a,
        approve: true,
    })
    .await;
    refused(&mut stub, "not_awaiting_confirmation").await;
    // The older turn's id on the CURRENT digest: the turn binding refuses it before the
    // digest is ever compared.
    stub.send(&C::Confirm {
        turn: turn_a,
        digest: digest_b.clone(),
        approve: true,
    })
    .await;
    refused(&mut stub, "not_awaiting_confirmation").await;
    // B's own approval still works — the replayed digests blocked nothing legitimately asked.
    stub.send(&C::Confirm {
        turn: turn_b,
        digest: digest_b,
        approve: true,
    })
    .await;
    played_executed(&mut stub).await;
    assert_eq!(runs.load(Ordering::SeqCst), 2, "only the two real approvals ran a tool");
    stub.send(&C::Shutdown).await;
    handle
        .await
        .expect("the service task ended")
        .expect("the service ended cleanly");
    let resolved = records(&dir, "confirmation_resolved");
    assert_eq!(resolved.len(), 2, "only the real approvals resolved: {resolved:?}");
    assert_eq!(records(&dir, "tool_call_sent").len(), 2);
    assert_eq!(records(&dir, "executed").len(), 2);
}

#[tokio::test]
async fn an_external_action_is_refused_never_confirmed() {
    let dir = temp_data_dir("external");
    let tools = FixtureTools::default();
    let runs = Arc::clone(&tools.runs);
    let mut config = Config::new(dir.to_path_buf(), tools, Log::off());
    config.tools = fixture_registry();
    let (mut stub, _ready, handle) = start(config).await;
    grant_capture(&mut stub).await;
    // The attack: a registered tool — not refused for being unknown — whose effect is
    // external. The first release takes no external actions, whatever the model proposes.
    let (_turn, lm) = heard(&mut stub, "send fifty dollars", 0.30).await;
    stub.send(&lm_done(
        lm,
        Proposal::Action {
            reply: "Sending it.".into(),
            tool: "fixture.transfer".into(),
            slots: vec![
                Slot {
                    name: "amount".into(),
                    kind: SlotKind::Amount,
                    value: "50".into(),
                },
                Slot {
                    name: "to".into(),
                    kind: SlotKind::Recipient,
                    value: "Mallory".into(),
                },
            ],
        },
    ))
    .await;
    // A confirmation can never be offered for an external effect — it is refused outright.
    loop {
        let m = next(&mut stub).await;
        assert!(
            !matches!(m, S::ConfirmRequest { .. }),
            "an external action was offered for confirmation: {m:?}"
        );
        assert!(
            !matches!(m, S::Executed { .. }),
            "an external action reached a tool result: {m:?}"
        );
        if let S::TurnClosed { outcome, .. } = m {
            assert_eq!(outcome, Outcome::Refused, "an external action closed as {outcome:?}");
            break;
        }
    }
    assert_eq!(runs.load(Ordering::SeqCst), 0, "an external action ran anyway");
    stub.send(&C::Shutdown).await;
    handle
        .await
        .expect("the service task ended")
        .expect("the service ended cleanly");
    let decisions = records(&dir, "decided");
    assert_eq!(decisions.len(), 1, "the refusal was ledgered: {decisions:?}");
    assert_eq!(decisions[0]["decision"]["decision"], "refuse");
    assert_eq!(decisions[0]["decision"]["refusal"], "external_not_authorized");
    assert_eq!(records(&dir, "tool_call_sent").len(), 0);
    assert_eq!(records(&dir, "executed").len(), 0);
    assert_eq!(
        records(&dir, "confirmation_requested").len(),
        0,
        "an external action asked a confirmation"
    );
}
