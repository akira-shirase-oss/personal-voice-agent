//! Failure case: the client stops reading stdout while stdin stays open — a dead reader, the
//! real-EPIPE shape (observed driving the docs site's snippet suite: the writer task dies, the
//! session ends `client_unresponsive`). The end must be bounded and the ledger must name it
//! `client_gone`, so the lock frees and a restarted app is not refused `already_running`.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code: a panic is the failure report"
)]

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

/// A scratch data directory, removed with its contents when dropped (also on a failed assertion):
/// data directories hold heard transcripts.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("pva-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl std::ops::Deref for Scratch {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The service's `session_end` stderr events, captured for the reason check (stderr carries
/// codes and counts only, never content).
fn read_stderr(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

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
            out.push(serde_json::from_str::<Value>(json).unwrap()["rec"].clone());
        }
    }
    out
}

fn spawn(dir: &Path, stderr_log: &Path) -> (Child, ChildStdin, BufReader<std::process::ChildStdout>) {
    let stderr = std::fs::File::create(stderr_log).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_voice-session"))
        .arg("--data-dir")
        .arg(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(stderr))
        .spawn()
        .unwrap();
    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    (child, stdin, BufReader::new(stdout))
}

fn send(stdin: &mut ChildStdin, v: &Value) {
    writeln!(stdin, "{v}").unwrap();
    stdin.flush().unwrap();
}

#[test]
fn a_dead_reader_ends_the_session_as_client_gone() {
    let dir = Scratch::new("gone");
    let stderr_log = dir.join("stderr.log");
    let (mut child, mut stdin, mut out) = spawn(&dir, &stderr_log);
    send(&mut stdin, &json!({"type":"hello","protocol":1,"client":"gone-test"}));
    let mut ready = String::new();
    out.read_line(&mut ready).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&ready).unwrap()["type"], "ready");

    // The app's read side is dead (reader thread gone, app stopped): its stdout pipe has no
    // reader from here on, while its stdin keeps accepting lines. Keep pinging: the writer task
    // dies on the first flush it attempts (EPIPE), and the *next* send meets the dead channel —
    // but any pings already queued ahead of that first flush are just dropped with it, so the
    // session only ends once a send lands after the writer is gone.
    drop(out);
    let deadline = Instant::now() + Duration::from_secs(60);
    while child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "the service never ended for a dead reader");
        if writeln!(stdin, "{}", json!({"type":"ping"})).is_err() {
            break;
        }
        let _ = stdin.flush();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        child.try_wait().unwrap().is_some(),
        "stdin died before the service noticed the dead reader"
    );

    let stderr = read_stderr(&stderr_log);
    assert!(
        stderr.contains("reason=client_unresponsive"),
        "the end is named on stderr too, got: {stderr}"
    );
    let recs = ledger(&dir);
    let closed = recs.iter().rev().find(|r| r["kind"] == "session_closed").unwrap();
    assert_eq!(
        closed["reason"], "client_gone",
        "the ledger names an unreachable client, not 'clean'"
    );

    // And the lock is really free: a new service opens the same directory instead of exiting
    // `already_running` — the headless-session alternative would leave the app refused forever.
    let (mut again, mut stdin2, mut out2) = spawn(&dir, &dir.join("stderr2.log"));
    std::thread::sleep(Duration::from_secs(1));
    assert!(
        again.try_wait().unwrap().is_none(),
        "a second service was refused: {}",
        read_stderr(&dir.join("stderr2.log"))
    );
    send(&mut stdin2, &json!({"type":"hello","protocol":1,"client":"gone-test"}));
    let mut ready2 = String::new();
    out2.read_line(&mut ready2).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&ready2).unwrap()["type"], "ready");
    send(&mut stdin2, &json!({"type":"shutdown"}));
    drop(out2);
    let wait = Instant::now() + Duration::from_secs(15);
    while again.try_wait().unwrap().is_none() {
        assert!(Instant::now() < wait, "the restarted service did not stop on shutdown");
        std::thread::sleep(Duration::from_millis(50));
    }
}
