//! Shared helpers for the attack lab: run the real `voice-session` binary over pipes and read
//! its ledger. The binary path follows the Swift suites' convention — `PVA_SESSION_BINARY`,
//! then `$CARGO_TARGET_DIR/debug`, then `target/debug` — and a missing binary fails loudly.

#![allow(dead_code, reason = "shared helpers: not every attack binary uses every helper")]

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

/// A scratch data directory, removed with its contents when dropped (also on a failed
/// assertion): data directories hold heard transcripts.
pub(crate) struct Scratch(pub(crate) PathBuf);

impl Scratch {
    pub(crate) fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("pva-attacks-{name}-{}", std::process::id()));
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

/// The session binary, in the order the Swift suites search it. Fails the test with the
/// reason, never a skip.
pub(crate) fn session_binary() -> PathBuf {
    if let Some(p) = option_env!("PVA_SESSION_BINARY")
        .map(PathBuf::from)
        .or_else(|| std::env::var("PVA_SESSION_BINARY").ok().map(PathBuf::from))
    {
        assert!(p.is_file(), "PVA_SESSION_BINARY does not exist: {}", p.display());
        return p;
    }
    let target = std::env::var("CARGO_TARGET_DIR").map_or_else(
        |_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"),
        PathBuf::from,
    );
    let bin = target.join("debug/voice-session");
    assert!(
        bin.is_file(),
        "the voice-session binary is missing at {} — run `cargo build --bin voice-session`",
        bin.display()
    );
    bin
}

/// The service's stdout on a reader thread behind a bounded channel: an attack test must
/// never hang waiting on a service that stopped answering. When the service exits the pipe
/// hits EOF, the thread ends, and `next` returns `None`.
pub(crate) struct Lines(std::sync::mpsc::Receiver<Result<Value, String>>);

impl Lines {
    /// The next service line within `secs`, or `None` on EOF. Panics on a malformed line —
    /// the service speaks JSON or the test is already broken.
    pub(crate) fn next(&self, secs: u64) -> Option<Value> {
        match self.0.recv_timeout(Duration::from_secs(secs)) {
            Ok(Ok(v)) => Some(v),
            Ok(Err(e)) => panic!("a service line was not JSON: {e}"),
            Err(_) => None,
        }
    }
}

/// Spawns the service on `dir` with stderr in `stderr_log`; returns the child, its stdin and
/// a bounded line reader on its stdout.
pub(crate) fn spawn(dir: &Path, stderr_log: &Path) -> (Child, ChildStdin, Lines) {
    let stderr = std::fs::File::create(stderr_log).unwrap();
    let mut child = Command::new(session_binary())
        .arg("--data-dir")
        .arg(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(stderr))
        .spawn()
        .unwrap();
    let stdin = child.stdin.take().unwrap();
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut out = stdout;
        let mut line = String::new();
        while out.read_line(&mut line).unwrap_or(0) > 0 {
            let r = serde_json::from_str::<Value>(&line).map_err(|_| line.clone());
            if tx.send(r).is_err() {
                return;
            }
            line.clear();
        }
    });
    (child, stdin, Lines(rx))
}

pub(crate) fn send(stdin: &mut ChildStdin, v: &Value) {
    writeln!(stdin, "{v}").unwrap();
    stdin.flush().unwrap();
}

pub(crate) fn handshake(stdin: &mut ChildStdin, out: &Lines) {
    send(stdin, &json!({"type":"hello","protocol":1,"client":"attacks"}));
    let ready = out.next(15).expect("the service did not answer hello");
    assert_eq!(ready["type"], "ready", "handshake failed: {ready}");
}

/// A full push-to-talk press: `ptt_down`, then `ptt_up` naming the turn the service opened —
/// `ptt_up` carries the turn id, so a press must learn it from the wire, not guess it. Reads
/// until the turn id arrives (`turn_opened` or `capture`) and returns it.
pub(crate) fn press_and_release(stdin: &mut ChildStdin, out: &Lines) -> u64 {
    send(stdin, &json!({"type":"ptt_down"}));
    let turn = loop {
        let m = out.next(15).expect("no turn_opened after ptt_down");
        if m["type"] == "turn_opened" || m["type"] == "capture" {
            break m["turn"].as_u64().expect("the turn message carried no id");
        }
    };
    send(stdin, &json!({"type":"ptt_up","turn":turn}));
    turn
}

pub(crate) fn wait_exit(child: &mut Child, secs: u64, why: &str) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "{why}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// `rec` objects from every ledger line under `dir/ledger/`.
pub(crate) fn ledger(dir: &Path) -> Vec<Value> {
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

pub(crate) fn last_session_closed(dir: &Path) -> Value {
    ledger(dir)
        .into_iter()
        .rev()
        .find(|r| r["kind"] == "session_closed")
        .expect("no session_closed record")
}

pub(crate) fn read_stderr(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}
