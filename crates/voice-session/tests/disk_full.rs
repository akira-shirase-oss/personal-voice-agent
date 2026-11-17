//! Failure cases on a full disk, with the real binary on a small disk image (macOS `hdiutil`,
//! no root). A revocation must hold: the volume is filled while capture is on, then capture is
//! revoked; the app must hear the turn close and the revoked state, push-to-talk must be
//! refused, and after a restart — space freed — consent must still be revoked. And "Delete
//! history" must work on a full volume: the deletion must not wait for space it cannot get.
#![cfg(target_os = "macos")]
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

struct Service {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<Value>,
    seen: Vec<Value>,
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
        let mut s = Self {
            child,
            stdin,
            lines,
            seen: Vec::new(),
        };
        s.send(&json!({"type":"hello","protocol":1,"client":"disk-full-test"}));
        s
    }

    fn send(&mut self, v: &Value) {
        let _ = writeln!(self.stdin, "{v}");
        let _ = self.stdin.flush();
    }

    /// The next message of type `ty`, or None after 10 s. Everything received is kept in `seen`.
    fn until(&mut self, ty: &str) -> Option<Value> {
        self.until_any(&[ty])
    }

    /// The next message whose type is one of `types`, or None after 10 s.
    fn until_any(&mut self, types: &[&str]) -> Option<Value> {
        loop {
            let m = self.lines.recv_timeout(Duration::from_secs(10)).ok()?;
            self.seen.push(m.clone());
            if types.iter().any(|t| m["type"] == *t) {
                return Some(m);
            }
        }
    }
}

struct Image {
    mount: PathBuf,
    dmg: PathBuf,
}

impl Image {
    fn new(fs: &str, case: &str) -> Self {
        let base = std::env::temp_dir().join(format!("pva-diskfull-{}-{case}-{fs}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let dmg = base.join("full.dmg");
        let mount = base.join("mnt");
        let ok = Command::new("hdiutil")
            .args([
                "create", "-quiet", "-size", "16m", "-fs", fs, "-volname", "pvafull", "-type", "UDIF",
            ])
            .arg(&dmg)
            .status()
            .unwrap()
            .success();
        assert!(ok, "hdiutil create {fs}");
        let ok = Command::new("hdiutil")
            .args(["attach", "-quiet", "-nobrowse", "-mountpoint"])
            .arg(&mount)
            .arg(&dmg)
            .status()
            .unwrap()
            .success();
        assert!(ok, "hdiutil attach");
        Self { mount, dmg }
    }

    /// Writes filler until the volume refuses more.
    fn fill(&self) -> u64 {
        let mut f = std::fs::File::create(self.mount.join("filler")).unwrap();
        let mut total = 0;
        for size in [1 << 20, 64 << 10, 4 << 10, 512, 1] {
            let chunk = vec![0xA5u8; size];
            while f.write_all(&chunk).is_ok() && f.sync_all().is_ok() {
                total += size as u64;
            }
        }
        total
    }

    fn free(&self) {
        std::fs::remove_file(self.mount.join("filler")).unwrap();
    }
}

impl Drop for Image {
    fn drop(&mut self) {
        let _ = Command::new("hdiutil")
            .args(["detach", "-quiet", "-force"])
            .arg(&self.mount)
            .status();
        let _ = std::fs::remove_dir_all(self.dmg.parent().unwrap());
    }
}

/// A record that fits in the slack of the consent log's last block needs no new space, so a
/// revocation would be written even on a full volume. Grows the log (with repeated grants) until
/// the next record must start a new block, then fills the volume, so it really refuses it.
fn fill_after_the_consent_log_reaches_a_block_boundary(s: &mut Service, image: &Image, data: &Path, fs: &str) -> u64 {
    let log = data.join("consent.log");
    for _ in 0..200 {
        let slack = 4096 - std::fs::metadata(&log).unwrap().len() % 4096;
        if slack < 150 {
            break;
        }
        s.send(&json!({"type":"consent","scope":"capture","grant":true,"rights_review":null}));
        s.until("consent_state").expect("granted again");
    }
    assert!(
        4096 - std::fs::metadata(&log).unwrap().len() % 4096 < 150,
        "{fs}: the log ends near a block boundary"
    );
    image.fill()
}

/// The residual case of the second review: the token cannot be unlinked (the data directory
/// is not writable) and the revocation's record cannot be logged (the volume is full; at most a
/// torn tail lands). The revocation must be on disk in the token itself (mode 0000), not only
/// in whatever the failed append left, and the restart comes back revoked.
#[test]
fn a_revocation_holds_when_the_token_cannot_be_unlinked_and_the_log_cannot_be_written() {
    use std::os::unix::fs::PermissionsExt;
    let image = Image::new("HFS+", "double");
    let data = image.mount.join("data");
    let mut s = Service::start(&data);
    s.until("ready").expect("ready");
    s.send(&json!({"type":"consent","scope":"capture","grant":true,"rights_review":null}));
    assert_eq!(s.until("consent_state").expect("granted")["capture"], true);
    assert!(fill_after_the_consent_log_reaches_a_block_boundary(&mut s, &image, &data, "HFS+") > 0);
    let writable = std::fs::metadata(&data).unwrap().permissions();
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o500)).unwrap();
    assert!(
        std::fs::remove_file(data.join("consent.capture.granted")).is_err(),
        "the scenario must really stop the unlink"
    );
    s.send(&json!({"type":"consent","scope":"capture","grant":false,"rights_review":null}));
    let state = s.until("consent_state");
    assert_eq!(
        state.map(|m| m["capture"].clone()),
        Some(json!(false)),
        "saw {:?}",
        s.seen
    );
    let degraded = s.until("degraded");
    assert_eq!(
        degraded.map(|m| m["stage"].clone()),
        Some(json!("ledger")),
        "the log write must really fail (else this test proves nothing); saw {:?}",
        s.seen
    );
    s.send(&json!({"type":"shutdown"}));
    let _ = s.child.wait();
    let token = std::fs::metadata(data.join("consent.capture.granted")).unwrap();
    assert_eq!(
        token.permissions().mode() & 0o600,
        0,
        "the token itself records the revocation"
    );

    std::fs::set_permissions(&data, writable).unwrap();
    image.free();
    let mut s = Service::start(&data);
    let ready = s.until("ready").expect("ready after restart");
    assert_eq!(
        ready["consent"]["capture"], false,
        "after a restart the revocation still holds, though neither unlink nor log write landed"
    );
    s.send(&json!({"type":"shutdown"}));
    let _ = s.child.wait();
}

/// `write_must_fail`: on HFS+ an unlink frees nothing, so the revocation's log record really
/// cannot be written and the failure path is exercised. On APFS, unlinking the consent token
/// can free enough space for the record to land; the user-visible results must hold either way.
fn revocation_holds_on_a_full(fs: &str, write_must_fail: bool) {
    let image = Image::new(fs, "revoke");
    let data = image.mount.join("data");
    let mut s = Service::start(&data);
    s.until("ready").expect("ready");
    s.send(&json!({"type":"consent","scope":"capture","grant":true,"rights_review":null}));
    assert_eq!(s.until("consent_state").expect("granted")["capture"], true);
    s.send(&json!({"type":"ptt_down"}));
    let turn = s.until("turn_opened").expect("turn")["turn"].as_u64().unwrap();
    s.until("capture").expect("capture on");
    let filled = fill_after_the_consent_log_reaches_a_block_boundary(&mut s, &image, &data, fs);
    assert!(filled > 0, "{fs}: the volume was filled");
    s.send(&json!({"type":"consent","scope":"capture","grant":false,"rights_review":null}));
    let closed = s.until("turn_closed");
    assert_eq!(
        closed.map(|m| m["outcome"].clone()),
        Some(json!("consent_revoked")),
        "{fs}: the turn closes as revoked; saw {:?}",
        s.seen
    );
    let state = s.until("consent_state");
    assert_eq!(
        state.map(|m| m["capture"].clone()),
        Some(json!(false)),
        "{fs}: the app is told consent is revoked; saw {:?}",
        s.seen
    );
    if write_must_fail {
        let degraded = s.until("degraded");
        assert_eq!(
            degraded.map(|m| m["stage"].clone()),
            Some(json!("ledger")),
            "{fs}: the scenario must really fail the write (else this test proves nothing); saw {:?}",
            s.seen
        );
    }
    assert!(
        s.seen
            .iter()
            .any(|m| m["type"] == "capture" && m["on"] == false && m["turn"] == turn),
        "{fs}: capture stopped"
    );
    s.send(&json!({"type":"ptt_down"}));
    let notice = s.until("notice").expect("notice");
    assert_eq!(
        notice["code"], "consent_required",
        "{fs}: capture is refused after the revocation"
    );
    s.send(&json!({"type":"shutdown"}));
    let _ = s.child.wait();

    image.free();
    let mut s = Service::start(&data);
    let ready = s.until("ready").expect("ready after restart");
    assert_eq!(
        ready["consent"]["capture"], false,
        "{fs}: after a restart the revocation still holds"
    );
    s.send(&json!({"type":"shutdown"}));
    let _ = s.child.wait();
}

#[test]
fn a_revocation_holds_on_a_full_hfs_volume() {
    revocation_holds_on_a_full("HFS+", true);
}

#[test]
fn a_revocation_holds_on_a_full_apfs_volume() {
    revocation_holds_on_a_full("APFS", false);
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

fn files_holding(dir: &Path, needle: &str) -> Vec<PathBuf> {
    walk(dir)
        .into_iter()
        .filter(|p| {
            std::fs::read(p)
                .unwrap()
                .windows(needle.len())
                .any(|w| w == needle.as_bytes())
        })
        .collect()
}

/// Deletes history on a filled volume. The old order created the fresh log first, so a full
/// disk could stop the deletion when it is most wanted; an unlink needs no free space.
fn history_is_deleted_on_a_full(fs: &str) {
    const SECRET: &str = "QUINCE-LANTERN-2093";
    let image = Image::new(fs, "forget");
    let data = image.mount.join("data");
    let mut s = Service::start(&data);
    s.until("ready").expect("ready");
    s.send(&json!({"type":"consent","scope":"capture","grant":true,"rights_review":null}));
    s.until("consent_state").expect("granted");
    s.send(&json!({"type":"ptt_down"}));
    let turn = s.until("turn_opened").expect("turn")["turn"].as_u64().unwrap();
    s.send(&json!({"type":"ptt_up","turn":turn}));
    let call = s.until("stt_request").expect("stt request")["call"].as_u64().unwrap();
    s.send(&json!({"type":"stt_final","call":call,"text":format!("remember {SECRET}"),"min_confidence":0.99}));
    s.until("lm_request").expect("lm request");
    s.send(&json!({"type":"interrupt"}));
    s.until("turn_closed").expect("turn closed");
    s.send(&json!({"type":"flush"}));
    s.until("flushed").expect("durable");
    assert!(
        !files_holding(&data, SECRET).is_empty(),
        "{fs}: the transcript is on disk first"
    );

    assert!(image.fill() > 0, "{fs}: the volume was filled");
    s.send(&json!({"type":"forget","scope":"capture"}));
    let answer = s.until_any(&["forgotten", "error"]);
    assert_eq!(
        answer.as_ref().map(|m| m["type"].clone()),
        Some(json!("forgotten")),
        "{fs}: history is deleted on a full volume; saw {:?}",
        s.seen
    );
    assert!(
        answer.unwrap()["files"].as_u64().unwrap() >= 1,
        "{fs}: the log was counted"
    );
    assert!(
        files_holding(&data, SECRET).is_empty(),
        "{fs}: the transcript is gone from the live file system"
    );
    s.send(&json!({"type":"shutdown"}));
    let _ = s.child.wait();

    image.free();
    let mut s = Service::start(&data);
    s.until("ready").expect("ready after restart");
    s.send(&json!({"type":"shutdown"}));
    let _ = s.child.wait();
    assert!(files_holding(&data, SECRET).is_empty(), "{fs}: nothing came back");
}

#[test]
fn history_is_deleted_on_a_full_hfs_volume() {
    history_is_deleted_on_a_full("HFS+");
}

#[test]
fn history_is_deleted_on_a_full_apfs_volume() {
    history_is_deleted_on_a_full("APFS");
}
