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
use std::os::unix::fs::symlink;

#[test]
fn a_symlinked_consent_log_is_refused_not_followed() {
    let dir = Scratch::new("pathlink");
    let outside = dir.join("outside.log");
    std::fs::write(&outside, b"attacker bytes\n").unwrap();
    symlink(&outside, dir.join("consent.log")).unwrap();
    let mut svc = spawn(&dir, &dir.join("stderr.log"));
    // The service says why and exits (store error code on the wire, non-zero exit).
    let err = svc.out.expect(15, "a symlinked consent.log opened silently");
    assert!(
        err["code"].as_str().is_some_and(|c| c.starts_with("store_")),
        "a symlinked store file was not refused: {err}"
    );
    svc.wait_exit(15, "the service stayed up on a symlinked store file");
    assert_eq!(
        std::fs::read_to_string(&outside).unwrap(),
        "attacker bytes\n",
        "the service wrote through the link"
    );
}

#[test]
fn a_symlinked_ledger_directory_is_refused_not_followed() {
    let dir = Scratch::new("pathledger");
    // A `ledger/` that is a link would put every heard transcript outside the private data
    // directory — the same attack as a symlinked store file, one level up. The target must
    // be a genuinely separate directory: one planted inside the data dir is refused as an
    // unowned directory before the link is ever followed, which would mask the hole.
    let outside = Scratch::new("pathoutside");
    symlink(&outside.0, dir.join("ledger")).unwrap();
    let mut svc = spawn(&dir, &dir.join("stderr.log"));
    let err = svc.out.expect(15, "a symlinked ledger/ opened silently");
    assert!(
        err["code"].as_str().is_some_and(|c| c.starts_with("store_")),
        "a symlinked ledger directory was not refused: {err}"
    );
    svc.wait_exit(15, "the service stayed up on a symlinked ledger directory");
    assert!(
        std::fs::read_dir(&outside.0).unwrap().next().is_none(),
        "the service wrote through the link"
    );
}

#[test]
fn a_directory_planted_where_a_store_file_belongs_is_refused() {
    let dir = Scratch::new("pathdir");
    // `consent.log` must be a file; a directory planted in its place is not opened.
    std::fs::create_dir_all(dir.join("consent.log")).unwrap();
    let mut svc = spawn(&dir, &dir.join("stderr.log"));
    let err = svc.out.expect(15, "a planted ledger directory opened silently");
    assert!(
        err["code"].as_str().is_some_and(|c| c.starts_with("store_")),
        "a planted directory was not refused: {err}"
    );
    svc.wait_exit(15, "the service stayed up on a planted directory");
}

#[test]
fn a_second_service_on_the_same_directory_is_refused() {
    let dir = Scratch::new("pathlock");
    let mut first = spawn(&dir, &dir.join("stderr.log"));
    handshake(&mut first.stdin, &first.out);
    let mut second = spawn(&dir, &dir.join("stderr2.log"));
    let err = second.out.expect(15, "a second instance was not refused");
    assert_eq!(
        err["code"], "already_running",
        "the second instance was not refused: {err}"
    );
    second.wait_exit(15, "the refused instance did not exit");
    // The holder is unaffected and ends cleanly on shutdown.
    first.shutdown(15, "the first service did not stop on shutdown");
    assert_eq!(last_session_closed(&dir)["reason"], "clean");
}
