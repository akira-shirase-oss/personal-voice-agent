//! Auth-bypass analogs at the protocol gate: what the service does before a valid `hello`.
//! Pre-hello messages are refused, a wrong protocol version is refused and named, and a
//! client that never says hello is ended `client_gone` inside the handshake deadline.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code: a panic is the failure report"
)]

mod common;

use common::*;
use serde_json::json;

#[test]
fn a_message_before_hello_is_refused_and_the_session_waits_for_hello() {
    let dir = Scratch::new("prehello");
    let mut svc = spawn(&dir, &dir.join("stderr.log"));
    svc.send(&json!({"type":"ptt_down"}));
    svc.send(&json!({"type":"stt_final","call":1,"text":"send money","min_confidence":0.9}));
    for _ in 0..2 {
        let err = svc.out.expect(15, "no answer to a pre-hello message");
        assert_eq!(
            err["code"], "hello_required",
            "a pre-hello message was not refused: {err}"
        );
    }
    // The session is not dropped: a proper hello still gets its ready, and a shutdown still
    // ends it — the pre-hello storm was answered, not just ignored.
    handshake(&mut svc.stdin, &svc.out);
    svc.shutdown(15, "the service did not stop on shutdown");
    assert_eq!(
        last_session_closed(&dir)["reason"],
        "clean",
        "a clean shutdown was not ledgered clean"
    );
}

#[test]
fn a_wrong_protocol_version_is_refused_named_and_ended() {
    let dir = Scratch::new("badproto");
    let mut svc = spawn(&dir, &dir.join("stderr.log"));
    svc.send(&json!({"type":"hello","protocol":99,"client":"attacker"}));
    let err = svc.out.expect(15, "no protocol_mismatch answer");
    assert_eq!(
        err["code"], "protocol_mismatch",
        "a wrong version was not refused: {err}"
    );
    svc.wait_exit(15, "the service kept running after refusing the handshake");
    assert_eq!(
        last_session_closed(&dir)["reason"],
        "refused",
        "a refused handshake was ledgered as something else"
    );
}

#[test]
fn a_client_that_never_says_hello_is_ended_client_gone() {
    let dir = Scratch::new("nohello");
    let mut svc = spawn(&dir, &dir.join("stderr.log"));
    // The handshake deadline is 5 s; a connected client that drips nothing past it is ended.
    svc.wait_exit(30, "a silent client held the session open past its deadline");
    assert_eq!(
        last_session_closed(&dir)["reason"],
        "client_gone",
        "a timed-out handshake was not ledgered client_gone"
    );
    assert!(
        read_stderr(&dir.join("stderr.log")).contains("handshake_timeout"),
        "the timeout was not named on stderr"
    );
}

#[test]
fn a_second_hello_is_an_error_not_a_rehandshake() {
    let dir = Scratch::new("duphello");
    let mut svc = spawn(&dir, &dir.join("stderr.log"));
    handshake(&mut svc.stdin, &svc.out);
    svc.send(&json!({"type":"hello","protocol":1,"client":"attacker"}));
    let err = svc.out.expect(15, "no duplicate_hello answer");
    assert_eq!(err["code"], "duplicate_hello", "a second hello was not an error: {err}");
    svc.shutdown(15, "the service did not stop on shutdown");
    assert_eq!(
        last_session_closed(&dir)["reason"],
        "clean",
        "a clean shutdown was not ledgered clean"
    );
}

#[test]
fn a_ping_without_its_sequence_is_a_bad_message_not_a_pong() {
    let dir = Scratch::new("pingnoseq");
    let mut svc = spawn(&dir, &dir.join("stderr.log"));
    handshake(&mut svc.stdin, &svc.out);
    svc.send(&serde_json::json!({"type":"ping"}));
    let err = svc.out.expect(15, "a seq-less ping was silently answered");
    assert_eq!(
        err["code"], "bad_message",
        "a ping without seq was not refused as malformed: {err}"
    );
    // And the session still works — a proper ping gets its pong.
    barrier(&mut svc.stdin, &svc.out, 7);
    svc.shutdown(15, "the service did not stop on shutdown");
    assert_eq!(
        last_session_closed(&dir)["reason"],
        "clean",
        "a clean shutdown was not ledgered clean"
    );
}
