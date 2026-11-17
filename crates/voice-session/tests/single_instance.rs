//! One service per data directory: a second instance must refuse to start, not "recover" the
//! first one's live session (renaming its log, writing the consent log with its own counter).
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code: a panic is the failure report"
)]

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

fn start(dir: &Path) -> (Child, ChildStdin, Receiver<Value>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_voice-session"))
        .arg("--data-dir")
        .arg(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(serde_json::from_str::<Value>(&line).unwrap()).is_err() {
                return;
            }
        }
    });
    writeln!(stdin, "{}", json!({"type":"hello","protocol":1,"client":"lock-test"})).unwrap();
    (child, stdin, rx)
}

fn next(rx: &Receiver<Value>) -> Value {
    rx.recv_timeout(Duration::from_secs(10)).expect("a message in time")
}

#[test]
fn a_second_instance_on_the_same_data_directory_refuses_to_start() {
    let dir = std::env::temp_dir().join(format!("pva-lock-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (mut a, mut a_in, a_rx) = start(&dir);
    assert_eq!(next(&a_rx)["type"], "ready");
    assert_eq!(
        std::fs::read_to_string(dir.join("lock")).unwrap().trim(),
        a.id().to_string(),
        "the lock names its holder, so a relaunched app can tell a stale orphan from another copy"
    );
    let live: Vec<_> = std::fs::read_dir(dir.join("ledger"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();

    let (mut b, _b_in, b_rx) = start(&dir);
    let first = next(&b_rx);
    assert_eq!(
        first["type"], "error",
        "the second instance must not become ready: {first}"
    );
    assert_eq!(first["code"], "already_running");
    assert_eq!(
        std::fs::read_to_string(dir.join("lock")).unwrap().trim(),
        a.id().to_string(),
        "the refused instance did not overwrite the holder's process ID"
    );
    assert_eq!(
        b.wait().unwrap().code(),
        Some(6),
        "a distinct exit code for 'already running'"
    );

    let still: Vec<_> = std::fs::read_dir(dir.join("ledger"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(still, live, "the live session's log was not touched");
    writeln!(
        a_in,
        "{}",
        json!({"type":"consent","scope":"capture","grant":true,"rights_review":null})
    )
    .unwrap();
    assert_eq!(next(&a_rx)["type"], "consent_state", "the first instance keeps working");
    writeln!(a_in, "{}", json!({"type":"shutdown"})).unwrap();
    assert!(a.wait().unwrap().success());
    // With the first one gone, a new instance starts normally.
    let (mut c, mut c_in, c_rx) = start(&dir);
    assert_eq!(next(&c_rx)["type"], "ready");
    writeln!(c_in, "{}", json!({"type":"shutdown"})).unwrap();
    assert!(c.wait().unwrap().success());
    let _ = std::fs::remove_dir_all(&dir);
}
