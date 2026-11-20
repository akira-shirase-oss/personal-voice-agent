//! Flooding: oversized lines are bounded and discarded, and a malformed-line storm is
//! refused line by line without stalling the session — bounded everything holds against a
//! hostile writer, not just a slow one.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code: a panic is the failure report"
)]

mod common;

use common::*;
use serde_json::json;
use std::io::Write;

#[test]
fn a_line_past_the_cap_is_discarded_and_the_next_line_reads() {
    let dir = Scratch::new("bigline");
    let (_child, mut stdin, out) = spawn(&dir, &dir.join("stderr.log"));
    handshake(&mut stdin, &out);
    // 100 KB on one line — over the 64 KB cap — then a valid ping right behind it.
    stdin.write_all(&vec![b'x'; 100 * 1024]).unwrap();
    stdin.write_all(b"\n").unwrap();
    send(&mut stdin, &json!({"type":"ping","seq":1}));
    let mut too_long = false;
    let mut ponged = false;
    for _ in 0..6 {
        let Some(m) = out.next(15) else { break };
        too_long |= m["code"] == "line_too_long";
        ponged |= m["type"] == "pong";
    }
    assert!(too_long, "an oversized line was not reported discarded");
    assert!(ponged, "the service did not recover after the oversized line");
    send(&mut stdin, &json!({"type":"shutdown"}));
}

#[test]
fn a_malformed_storm_is_rate_limited_and_the_session_survives() {
    let dir = Scratch::new("storm");
    let (_child, mut stdin, out) = spawn(&dir, &dir.join("stderr.log"));
    handshake(&mut stdin, &out);
    // 300 malformed lines in one burst, then a valid ping. The service answers at most
    // BAD_LINE_REPLIES_PER_S (8) refusals per second — bounded, content-free — and the
    // session must still answer the ping once the flood drains.
    let mut flood = Vec::new();
    for _ in 0..300 {
        flood.extend_from_slice(b"not json\n");
    }
    stdin.write_all(&flood).unwrap();
    stdin.flush().unwrap();
    send(&mut stdin, &json!({"type":"ping","seq":2}));
    let mut errors = 0u32;
    let mut ponged = false;
    for _ in 0..500 {
        let Some(m) = out.next(30) else { break };
        if m["type"] == "pong" {
            ponged = true;
            break;
        }
        assert_eq!(
            m["code"], "bad_message",
            "a flood line produced something other than a bounded refusal: {m}"
        );
        errors += 1;
    }
    assert!(ponged, "the session never answered a valid line after the storm");
    // At most a window's worth of refusals per second; the flood is consumed in one or two.
    assert!(errors <= 16, "the storm got {errors} replies — not rate-limited");
    assert!(errors > 0, "malformed lines were silently swallowed");
    // The suppressed count is honest: a bad line in the next window reports how many earlier
    // ones went unanswered rather than pretending each was refused to its face.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    stdin.write_all(b"still not json\n").unwrap();
    stdin.flush().unwrap();
    let reported = loop {
        let m = out.next(15).expect("no answer to the follow-up bad line");
        if m["type"] == "error" {
            break m;
        }
    };
    assert!(
        reported["detail"]
            .as_str()
            .is_some_and(|d| d.contains("earlier ones were not answered")),
        "the suppressed-count report never arrived: {reported}"
    );
    send(&mut stdin, &json!({"type":"shutdown"}));
}
