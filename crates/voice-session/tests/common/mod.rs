//! Shared helpers for the attack lab: run the real `voice-session` binary over pipes and read
//! its ledger. The binary is Cargo's own build of this package's `voice-session` bin — never
//! a path that could be stale; `PVA_SESSION_BINARY` overrides it like the Swift suites.

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

/// The session binary: Cargo's just-built artifact for this package's bin — never a path
/// that could be stale. `PVA_SESSION_BINARY` overrides (the Swift suites' convention).
pub(crate) fn session_binary() -> PathBuf {
    if let Some(p) = option_env!("PVA_SESSION_BINARY")
        .map(PathBuf::from)
        .or_else(|| std::env::var("PVA_SESSION_BINARY").ok().map(PathBuf::from))
    {
        assert!(p.is_file(), "PVA_SESSION_BINARY does not exist: {}", p.display());
        return p;
    }
    PathBuf::from(env!("CARGO_BIN_EXE_voice-session"))
}

/// What `Lines::next` found. Silence and EOF are different outcomes: a dead service must not
/// read as a quiet one — a test that "expects silence" would otherwise pass on a hang.
pub(crate) enum Next {
    Msg(Value),
    Timeout,
    Eof,
}

/// The service's stdout on a reader thread behind a bounded channel: an attack test must
/// never hang waiting on a service that stopped answering. When the service exits the pipe
/// hits EOF, the thread ends, and `next` reports `Eof`.
pub(crate) struct Lines(std::sync::mpsc::Receiver<Result<Value, String>>);

impl Lines {
    /// The next service line within `secs`. Panics on a malformed line — the service speaks
    /// JSON or the test is already broken.
    pub(crate) fn next(&self, secs: u64) -> Next {
        use std::sync::mpsc::RecvTimeoutError::{Disconnected, Timeout};
        match self.0.recv_timeout(Duration::from_secs(secs)) {
            Ok(Ok(v)) => Next::Msg(v),
            Ok(Err(e)) => panic!("a service line was not JSON: {e}"),
            Err(Timeout) => Next::Timeout,
            Err(Disconnected) => Next::Eof,
        }
    }

    /// The next line within `secs`; silence or a dead service is a failed test, not a
    /// passed one.
    pub(crate) fn expect(&self, secs: u64, why: &str) -> Value {
        match self.next(secs) {
            Next::Msg(v) => v,
            Next::Timeout => panic!("no service line within {secs}s: {why}"),
            Next::Eof => panic!("the service's stdout closed: {why}"),
        }
    }

    /// Lines until `f` matches (each within `secs`); returns the match and the lines skipped
    /// before it, so a test can assert what did NOT happen on the way.
    pub(crate) fn until(&self, secs: u64, f: impl Fn(&Value) -> bool) -> (Value, Vec<Value>) {
        let mut skipped = Vec::new();
        loop {
            let m = self.expect(secs, "the service stopped answering");
            if f(&m) {
                return (m, skipped);
            }
            skipped.push(m);
        }
    }
}

/// The spawned service. Its drop is a guard: a test that ends without its own shutdown —
/// passed or failed — still leaves no service running: `shutdown` is tried, then SIGKILL
/// after a bounded wait, and the child is always reaped. A service left running would hold
/// its data-directory lock and poison later tests.
pub(crate) struct Service {
    child: Child,
    pub(crate) stdin: ChildStdin,
    pub(crate) out: Lines,
}

impl Service {
    /// One client message on stdin.
    pub(crate) fn send(&mut self, v: &Value) {
        send(&mut self.stdin, v);
    }

    /// `shutdown`, then the service must actually exit — a service that ignores it is a
    /// failed test, not a resource leak.
    pub(crate) fn shutdown(&mut self, secs: u64, why: &str) {
        self.send(&json!({"type":"shutdown"}));
        wait_exit(&mut self.child, secs, why);
    }

    /// Wait for the service to exit on its own (a refused launch, a handshake timeout).
    pub(crate) fn wait_exit(&mut self, secs: u64, why: &str) {
        wait_exit(&mut self.child, secs, why);
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        match self.child.try_wait() {
            Ok(Some(_)) | Err(_) => return,
            Ok(None) => {}
        }
        let _ = writeln!(self.stdin, r#"{{"type":"shutdown"}}"#);
        let _ = self.stdin.flush();
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.child.try_wait().unwrap().is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        if self.child.try_wait().unwrap().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

/// Spawns the service on `dir` with stderr in `stderr_log`.
pub(crate) fn spawn(dir: &Path, stderr_log: &Path) -> Service {
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
    Service {
        child,
        stdin,
        out: Lines(rx),
    }
}

pub(crate) fn send(stdin: &mut ChildStdin, v: &Value) {
    writeln!(stdin, "{v}").unwrap();
    stdin.flush().unwrap();
}

pub(crate) fn handshake(stdin: &mut ChildStdin, out: &Lines) {
    send(stdin, &json!({"type":"hello","protocol":1,"client":"attacks"}));
    let ready = out.expect(15, "the service did not answer hello");
    assert_eq!(ready["type"], "ready", "handshake failed: {ready}");
}

/// ping→pong: proves the service is alive and has processed everything sent before the ping
/// — the honest replacement for "wait for silence". Returns the lines skipped on the way, so
/// a test can assert what did NOT happen in between.
pub(crate) fn barrier(stdin: &mut ChildStdin, out: &Lines, seq: u64) -> Vec<Value> {
    send(stdin, &json!({"type":"ping","seq":seq}));
    let (pong, skipped) = out.until(15, |m| m["type"] == "pong");
    assert_eq!(pong["seq"], seq, "a pong for an older ping arrived: {skipped:?}");
    skipped
}

/// flush→flushed: every record queued before the flush is durable — the only honest point
/// for a mid-run ledger read (a post-exit read is stronger still).
pub(crate) fn flush(stdin: &mut ChildStdin, out: &Lines) -> Vec<Value> {
    send(stdin, &json!({"type":"flush"}));
    let (_, skipped) = out.until(15, |m| m["type"] == "flushed");
    skipped
}

/// A full push-to-talk press: `ptt_down`, then `ptt_up` naming the turn the service opened —
/// `ptt_up` carries the turn id, so a press must learn it from the wire, not guess it. Reads
/// until the turn id arrives (`turn_opened` or `capture`) and returns it.
pub(crate) fn press_and_release(stdin: &mut ChildStdin, out: &Lines) -> u64 {
    send(stdin, &json!({"type":"ptt_down"}));
    let (turn, _) = out.until(15, |m| m["type"] == "turn_opened" || m["type"] == "capture");
    let turn = turn["turn"].as_u64().expect("the turn message carried no id");
    send(stdin, &json!({"type":"ptt_up","turn":turn}));
    turn
}

/// Drives a turn to its close the way the app does: every `tts_request` is answered
/// `tts_started` + `tts_finished` (an unanswered speech call hits its deadline and the turn
/// is force-closed `degraded` — which would hide the outcome a test is trying to prove).
/// Returns every message seen, the `turn_closed` last. A turn that never closes is a failed
/// test, not a passed one.
pub(crate) fn drive_until_closed(svc: &mut Service, secs: u64) -> Vec<Value> {
    let mut seen = Vec::new();
    loop {
        let m = svc.out.expect(secs, "the turn never closed");
        if m["type"] == "tts_request" {
            let call = m["call"].clone();
            svc.send(&json!({"type":"tts_started","call":call}));
            svc.send(&json!({"type":"tts_finished","call":call}));
        }
        let closed = m["type"] == "turn_closed";
        seen.push(m);
        if closed {
            return seen;
        }
    }
}

pub(crate) fn wait_exit(child: &mut Child, secs: u64, why: &str) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "{why}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// `rec` objects from every ledger line under `dir/ledger/`. Call only after the service has
/// exited or answered a `flush` — before that, queued records are invisible and the final
/// line may still be in flight.
pub(crate) fn ledger(dir: &Path) -> Vec<Value> {
    let mut files: Vec<_> = std::fs::read_dir(dir.join("ledger"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    let mut out = Vec::new();
    for f in files {
        for line in std::fs::read_to_string(f).unwrap().lines() {
            let (_, json) = line
                .split_once(' ')
                .unwrap_or_else(|| panic!("a ledger line is torn: {line:?}"));
            let v = serde_json::from_str::<Value>(json)
                .unwrap_or_else(|e| panic!("a ledger record is undecodable ({e}): {json:?}"));
            out.push(v["rec"].clone());
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
