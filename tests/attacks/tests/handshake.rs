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
    let (_child, mut stdin, out) = spawn(&dir, &dir.join("stderr.log"));
    send(&mut stdin, &json!({"type":"ptt_down"}));
    send(
        &mut stdin,
        &json!({"type":"stt_final","call":1,"text":"send money","min_confidence":0.9}),
    );
    for _ in 0..2 {
        let err = out.next(15).expect("no answer to a pre-hello message");
        assert_eq!(
            err["code"], "hello_required",
            "a pre-hello message was not refused: {err}"
        );
    }
    // The session is not dropped: a proper hello still gets its ready.
    handshake(&mut stdin, &out);
}

#[test]
fn a_wrong_protocol_version_is_refused_named_and_ended() {
    let dir = Scratch::new("badproto");
    let (mut child, mut stdin, out) = spawn(&dir, &dir.join("stderr.log"));
    send(&mut stdin, &json!({"type":"hello","protocol":99,"client":"attacker"}));
    let err = out.next(15).expect("no protocol_mismatch answer");
    assert_eq!(
        err["code"], "protocol_mismatch",
        "a wrong version was not refused: {err}"
    );
    wait_exit(&mut child, 15, "the service kept running after refusing the handshake");
    assert_eq!(
        last_session_closed(&dir)["reason"],
        "refused",
        "a refused handshake was ledgered as something else"
    );
}

#[test]
fn a_client_that_never_says_hello_is_ended_client_gone() {
    let dir = Scratch::new("nohello");
    let (mut child, _stdin, _out) = spawn(&dir, &dir.join("stderr.log"));
    // The handshake deadline is 5 s; a connected client that drips nothing past it is ended.
    wait_exit(
        &mut child,
        30,
        "a silent client held the session open past its deadline",
    );
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
    let (_child, mut stdin, out) = spawn(&dir, &dir.join("stderr.log"));
    handshake(&mut stdin, &out);
    send(&mut stdin, &json!({"type":"hello","protocol":1,"client":"attacker"}));
    let err = out.next(15).expect("no duplicate_hello answer");
    assert_eq!(err["code"], "duplicate_hello", "a second hello was not an error: {err}");
    send(&mut stdin, &json!({"type":"shutdown"}));
}
