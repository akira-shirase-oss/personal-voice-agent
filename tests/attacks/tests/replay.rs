//! Replay attacks: yesterday's valid messages must not become today's approvals. The first
//! release's tool registry is empty, so every action proposal is refused — a `confirm`
//! replayed after that refusal is still refused, and a result replayed after its turn is
//! late-dropped, never applied.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code: a panic is the failure report"
)]

mod common;

use common::*;
use serde_json::json;

#[test]
fn a_confirm_replayed_after_a_refusal_is_still_refused() {
    let dir = Scratch::new("replayconfirm");
    let (_child, mut stdin, out) = spawn(&dir, &dir.join("stderr.log"));
    handshake(&mut stdin, &out);
    send(
        &mut stdin,
        &json!({"type":"consent","scope":"capture","grant":true,"rights_review":null}),
    );
    assert_eq!(out.next(15).unwrap()["type"], "consent_state");
    press_and_release(&mut stdin, &out);
    // Drive the turn: the forged transfer is refused (the registry has no tools), spoken as
    // a refusal, and the turn closes. Then the attack replays an approval.
    let mut refused = false;
    let mut turn = 0u64;
    for _ in 0..16 {
        let Some(m) = out.next(15) else { break };
        match m["type"].as_str() {
            Some("stt_request") => {
                send(
                    &mut stdin,
                    &json!({"type":"stt_final","call":m["call"],"text":"Send $42 to Priya.","min_confidence":0.95}),
                );
            }
            Some("lm_request") => {
                turn = m["turn"].as_u64().unwrap();
                send(
                    &mut stdin,
                    &json!({"type":"lm_done","call":m["call"],
                    "proposal":{"kind":"action","reply":"Sending it.","tool":"fixture.transfer",
                        "slots":[{"name":"amount","kind":"amount","value":"42"}]},
                    "model":"attacks","instructions":"attacks"}),
                );
            }
            Some("turn_closed") => {
                refused = true;
                break;
            }
            _ => {}
        }
    }
    assert!(refused, "the forged proposal never reached turn_closed");
    send(
        &mut stdin,
        &json!({"type":"confirm","turn":turn,"digest":"deadbeefdeadbeef","approve":true}),
    );
    let err = out.next(15).expect("a replayed confirm was accepted");
    assert!(
        err["code"]
            .as_str()
            .is_some_and(|c| c.starts_with("not_awaiting_confirmation") || c == "digest_mismatch"),
        "a replayed confirm was not refused: {err}"
    );
    assert!(
        ledger(&dir)
            .iter()
            .all(|r| !r["kind"].as_str().is_some_and(|k| k.starts_with("tool"))),
        "a replayed approval reached a tool call"
    );
    send(&mut stdin, &json!({"type":"shutdown"}));
}

#[test]
fn a_result_replayed_outside_its_turn_is_late_dropped() {
    let dir = Scratch::new("replayresult");
    let (mut child, mut stdin, out) = spawn(&dir, &dir.join("stderr.log"));
    handshake(&mut stdin, &out);
    send(
        &mut stdin,
        &json!({"type":"consent","scope":"capture","grant":true,"rights_review":null}),
    );
    assert_eq!(out.next(15).unwrap()["type"], "consent_state");
    send(&mut stdin, &json!({"type":"ptt_down"}));
    send(&mut stdin, &json!({"type":"interrupt"}));
    // The turn is closed as interrupted; a result replayed for its call must not reopen it.
    for _ in 0..8 {
        let Some(m) = out.next(15) else { break };
        if m["type"] == "turn_closed" {
            break;
        }
    }
    send(
        &mut stdin,
        &json!({"type":"stt_final","call":9,"text":"transfer money now","min_confidence":0.99}),
    );
    // The wire stays silent: a stale result is dropped, not answered — and the drop is
    // ledgered next to the interrupted turn, never applied.
    assert!(out.next(3).is_none(), "a replayed result got a wire reply");
    send(&mut stdin, &json!({"type":"shutdown"}));
    wait_exit(&mut child, 15, "the service did not stop on shutdown");
    let recs = ledger(&dir);
    assert!(
        recs.iter()
            .any(|r| r["kind"] == "turn_closed" && r["outcome"] == "interrupted"),
        "the interrupted turn is not in the ledger: {recs:?}"
    );
    assert!(
        recs.iter().any(|r| r["kind"] == "late_result_dropped"),
        "a replayed result was not ledgered as dropped: {recs:?}"
    );
    assert!(
        recs.iter()
            .all(|r| r["kind"] != "inferred" || r["text"].is_null() || !r.to_string().contains("transfer money")),
        "a replayed result was still applied"
    );
}
