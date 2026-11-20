//! Path and identity attacks on the data directory: store files that are links are refused
//! rather than followed, a planted directory is refused rather than read, and a second
//! service on the same directory is refused `already_running` instead of racing the lock.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code: a panic is the failure report"
)]

mod common;

use common::*;
use serde_json::json;
use std::os::unix::fs::symlink;

#[test]
fn a_symlinked_consent_log_is_refused_not_followed() {
    let dir = Scratch::new("pathlink");
    let outside = dir.join("outside.log");
    std::fs::write(&outside, b"attacker bytes\n").unwrap();
    symlink(&outside, dir.join("consent.log")).unwrap();
    let (mut child, _stdin, out) = spawn(&dir, &dir.join("stderr.log"));
    // The service says why and exits (store error code on the wire, non-zero exit).
    let err = out.next(15).expect("a symlinked consent.log opened silently");
    assert!(
        err["code"].as_str().is_some_and(|c| c.starts_with("store_")),
        "a symlinked store file was not refused: {err}"
    );
    wait_exit(&mut child, 15, "the service stayed up on a symlinked store file");
    assert_eq!(
        std::fs::read_to_string(&outside).unwrap(),
        "attacker bytes\n",
        "the service wrote through the link"
    );
}

#[test]
fn a_directory_planted_where_a_store_file_belongs_is_refused() {
    let dir = Scratch::new("pathdir");
    // `consent.log` must be a file; a directory planted in its place is not opened.
    std::fs::create_dir_all(dir.join("consent.log")).unwrap();
    let (mut child, _stdin, out) = spawn(&dir, &dir.join("stderr.log"));
    let err = out.next(15).expect("a planted ledger directory opened silently");
    assert!(
        err["code"].as_str().is_some_and(|c| c.starts_with("store_")),
        "a planted directory was not refused: {err}"
    );
    wait_exit(&mut child, 15, "the service stayed up on a planted directory");
}

#[test]
fn a_second_service_on_the_same_directory_is_refused() {
    let dir = Scratch::new("pathlock");
    let (mut first, mut stdin, out) = spawn(&dir, &dir.join("stderr.log"));
    handshake(&mut stdin, &out);
    let (mut second, _s2, out2) = spawn(&dir, &dir.join("stderr2.log"));
    let err = out2.next(15).expect("a second instance was not refused");
    assert_eq!(
        err["code"], "already_running",
        "the second instance was not refused: {err}"
    );
    wait_exit(&mut second, 15, "the refused instance did not exit");
    // The holder is unaffected and ends cleanly on shutdown.
    send(&mut stdin, &json!({"type":"shutdown"}));
    wait_exit(&mut first, 15, "the first service did not stop on shutdown");
    assert_eq!(last_session_closed(&dir)["reason"], "clean");
}
