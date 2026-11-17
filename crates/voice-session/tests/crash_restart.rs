//! Failure case: the service crashes and restarts without losing its ledger. Runs the real
//! binary, kills it with SIGKILL mid-turn, tears the log's tail, and restarts it.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code: a panic is the failure report"
)]

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

/// A scratch data directory, removed with its contents when dropped (also on a failed assertion):
/// data directories hold heard transcripts.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("pva-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
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

struct Service {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<Value>,
}

impl Service {
    fn start(dir: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_voice-session"))
            .arg("--data-dir")
            .arg(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(serde_json::from_str::<Value>(&line).unwrap()).is_err() {
                    return;
                }
            }
        });
        let mut s = Self { child, stdin, lines };
        s.send(&json!({"type":"hello","protocol":1,"client":"crash-test"}));
        s
    }

    fn send(&mut self, v: &Value) {
        writeln!(self.stdin, "{v}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn until(&self, ty: &str) -> Value {
        loop {
            let m = self
                .lines
                .recv_timeout(Duration::from_secs(10))
                .expect("service message in time");
            if m["type"] == ty {
                return m;
            }
        }
    }

    /// One turn through recognition; returns (turn, lm call).
    fn heard(&mut self, text: &str) -> (u64, u64) {
        self.send(&json!({"type":"ptt_down"}));
        let turn = self.until("turn_opened")["turn"].as_u64().unwrap();
        self.send(&json!({"type":"ptt_up","turn":turn}));
        let stt = self.until("stt_request")["call"].as_u64().unwrap();
        self.send(&json!({"type":"stt_final","call":stt,"text":text,"min_confidence":0.95}));
        let lm = self.until("lm_request")["call"].as_u64().unwrap();
        (turn, lm)
    }
}

fn records(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<Value>(l.split_once(' ').unwrap().1).unwrap())
        .collect()
}

fn session_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(dir.join("ledger"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    v.sort();
    v
}

#[test]
fn a_killed_service_restarts_with_its_ledger_intact_and_open_turns_closed() {
    let dir = Scratch::new("crash");

    let mut s = Service::start(&dir);
    assert_eq!(s.until("ready")["recovered"]["sessions"], 0);
    s.send(&json!({"type":"consent","scope":"capture","grant":true,"rights_review":null}));
    s.until("consent_state");
    // Turn 1 completes.
    let (t1, lm) = s.heard("first turn");
    s.send(&json!({"type":"lm_kind","call":lm,"kind":"reply"}));
    s.send(&json!({"type":"lm_sentence","call":lm,"index":0,"text":"Okay."}));
    let tts = s.until("tts_request")["call"].as_u64().unwrap();
    s.send(&json!({"type":"lm_done","call":lm,"proposal":{"kind":"reply","reply":"Okay."},"model":"stub","instructions":"stub-1"}));
    s.send(&json!({"type":"tts_started","call":tts}));
    s.send(&json!({"type":"tts_finished","call":tts}));
    assert_eq!(s.until("turn_closed")["outcome"], "completed");
    // Turn 2 is mid-flight (waiting on the model) when the process dies.
    let (t2, _) = s.heard("second turn");
    s.send(&json!({"type":"flush"}));
    let durable = s.until("flushed")["seq"].as_u64().unwrap();
    s.child.kill().unwrap(); // SIGKILL: no shutdown path runs
    s.child.wait().unwrap();

    let files = session_files(&dir);
    assert_eq!(files.len(), 1);
    let crashed = files[0].clone();
    assert!(
        !crashed.to_string_lossy().ends_with(".closed.log"),
        "a killed session is left open"
    );
    let before = records(&crashed);
    assert_eq!(
        before.len() as u64,
        durable,
        "every record acknowledged by flush is on disk"
    );
    // The kill also tore a write in half.
    std::fs::OpenOptions::new()
        .append(true)
        .open(&crashed)
        .unwrap()
        .write_all(b"5f3a9c01 {\"v\":1,\"seq\":")
        .unwrap();

    let mut s = Service::start(&dir);
    let ready = s.until("ready");
    assert_eq!(
        ready["recovered"],
        json!({"sessions":1,"abandoned_turns":1,"uncertain_calls":0,"torn_tails":1})
    );
    assert_eq!(ready["consent"]["capture"], true, "consent survives the crash");
    // The restarted service works.
    s.send(&json!({"type":"ptt_down"}));
    let t3 = s.until("turn_opened")["turn"].as_u64().unwrap();
    assert_eq!(t3, 1, "turn IDs are per session");
    s.send(&json!({"type":"shutdown"}));
    assert!(s.child.wait().unwrap().success());

    let closed = crashed.with_extension("closed.log");
    let after = records(&closed);
    assert_eq!(
        &after[..before.len()],
        &before[..],
        "nothing durable was lost or rewritten"
    );
    let tail: Vec<&str> = after[before.len()..]
        .iter()
        .map(|r| r["rec"]["kind"].as_str().unwrap())
        .collect();
    assert_eq!(tail, ["recovered_torn_tail", "turn_abandoned", "session_closed"]);
    assert_eq!(after[before.len() + 1]["rec"]["turn"], t2);
    assert_eq!(after[before.len() + 2]["rec"]["reason"], "crashed");
    let seqs: Vec<u64> = after.iter().map(|r| r["seq"].as_u64().unwrap()).collect();
    assert_eq!(
        seqs,
        (1..=after.len() as u64).collect::<Vec<_>>(),
        "sequence numbers continue without a gap"
    );
    assert!(
        before
            .iter()
            .any(|r| r["rec"]["kind"] == "turn_closed" && r["rec"]["turn"] == t1)
    );
    // The second session closed cleanly.
    let files = session_files(&dir);
    assert_eq!(files.len(), 2);
    assert!(files.iter().all(|f| f.to_string_lossy().ends_with(".closed.log")));
}

#[test]
fn files_are_private_to_the_user() {
    use std::os::unix::fs::PermissionsExt;
    let dir = Scratch::new("perms");
    let mut s = Service::start(&dir);
    s.until("ready");
    s.send(&json!({"type":"shutdown"}));
    s.child.wait().unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(mode(&dir.join("ledger")), 0o700);
    assert_eq!(mode(&dir.join("consent.log")), 0o600);
    for f in session_files(&dir) {
        assert_eq!(mode(&f), 0o600);
    }
}
