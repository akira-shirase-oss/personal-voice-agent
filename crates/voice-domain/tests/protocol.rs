//! The golden protocol lines in `fixtures/protocol/` decode, and re-encode to the same JSON.
//! The Swift test suite decodes the same files, so the two sides cannot drift apart.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code: a panic is the failure report"
)]

use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::BTreeSet;
use voice_domain::protocol::{ClientMessage, ServiceMessage};

fn golden(name: &str) -> Vec<(String, serde_json::Value)> {
    let path = format!("{}/../../fixtures/protocol/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| (l.to_owned(), serde_json::from_str(l).unwrap()))
        .collect()
}

fn round_trip<T: DeserializeOwned + Serialize>(name: &str) -> BTreeSet<String> {
    let mut types = BTreeSet::new();
    for (line, value) in golden(name) {
        let msg: T = serde_json::from_str(&line).unwrap_or_else(|e| panic!("{name}: {e}: {line}"));
        // Through text, as on the wire (to_value would widen f32 confidences to f64).
        let again: serde_json::Value = serde_json::from_str(&serde_json::to_string(&msg).unwrap()).unwrap();
        assert_eq!(again, value, "{name}: re-encoding changed the line: {line}");
        types.insert(value["type"].as_str().unwrap().to_owned());
    }
    types
}

#[test]
fn every_client_message_type_has_a_golden_line() {
    let types = round_trip::<ClientMessage>("client.jsonl");
    let expected = [
        "hello",
        "ptt_down",
        "ptt_up",
        "interrupt",
        "stt_final",
        "lm_kind",
        "lm_sentence",
        "lm_done",
        "tts_started",
        "tts_finished",
        "stt_failed",
        "lm_failed",
        "tts_failed",
        "confirm",
        "consent",
        "flush",
        "shutdown",
    ];
    assert_eq!(types, expected.iter().map(|s| (*s).to_owned()).collect());
}

#[test]
fn every_service_message_type_has_a_golden_line() {
    let types = round_trip::<ServiceMessage>("service.jsonl");
    let expected = [
        "ready",
        "turn_opened",
        "capture",
        "stt_request",
        "lm_request",
        "tts_request",
        "cancel",
        "stop_playback",
        "confirm_request",
        "turn_closed",
        "degraded",
        "consent_state",
        "notice",
        "flushed",
        "error",
    ];
    assert_eq!(types, expected.iter().map(|s| (*s).to_owned()).collect());
}
