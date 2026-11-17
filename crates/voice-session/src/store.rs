//! The on-disk ledger and consent log (D-017, ARCHITECTURE §On-disk format).
//!
//! One record per line: `<crc32 as 8 hex digits> <json>\n`, the CRC over the JSON bytes,
//! the JSON an envelope `{"v":1,"seq":N,"at_ms":…,"session":"…","rec":{…}}`. One writer
//! thread per file drains a bounded queue, writes the batch and `sync_data`s once (group
//! commit). On start every unclosed file is verified: a torn tail is truncated and recorded;
//! damage before the tail stops the service rather than guessing.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot, watch};
use voice_domain::consent::{ConsentRecord, ConsentScope, ConsentState};
use voice_domain::protocol::Recovered;
use voice_domain::record::{Record, RecoveryFold};

pub const FORMAT_VERSION: u32 = 1;
/// Largest encoded record. The domain's content bounds keep real records far below it.
pub const MAX_RECORD_BYTES: usize = 64 * 1024;
const MAX_BATCH: usize = 1_024;
const QUEUE: usize = 4_096;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("i/o on {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("{path} is damaged at line {line} ({why}); refusing to guess")]
    Corrupt {
        path: PathBuf,
        line: u64,
        why: &'static str,
    },
    #[error("another session service is using {path}")]
    Locked { path: PathBuf },
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> StoreError + '_ {
    move |source| StoreError::Io {
        path: path.to_owned(),
        source,
    }
}

#[derive(Deserialize)]
struct Envelope<T> {
    v: u32,
    seq: u64,
    rec: T,
}

#[derive(Serialize)]
struct EnvelopeRef<'a, T> {
    v: u32,
    seq: u64,
    at_ms: u64,
    session: &'a str,
    rec: &'a T,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// One encoded line, newline included.
fn encode<T: Serialize>(seq: u64, session: &str, rec: &T) -> Result<Vec<u8>, serde_json::Error> {
    let json = serde_json::to_vec(&EnvelopeRef {
        v: FORMAT_VERSION,
        seq,
        at_ms: now_ms(),
        session,
        rec,
    })?;
    let mut line = Vec::with_capacity(json.len() + 10);
    write!(line, "{:08x} ", crc32fast::hash(&json)).map_err(serde_json::Error::io)?;
    line.extend_from_slice(&json);
    line.push(b'\n');
    Ok(line)
}

fn decode<T: DeserializeOwned>(line: &[u8]) -> Result<Envelope<T>, &'static str> {
    let (crc, json) = line.split_at_checked(9).ok_or("short line")?;
    if crc[8] != b' ' {
        return Err("bad frame");
    }
    let crc = std::str::from_utf8(&crc[..8])
        .ok()
        .and_then(|s| u32::from_str_radix(s, 16).ok())
        .ok_or("bad crc field")?;
    if crc32fast::hash(json) != crc {
        return Err("checksum mismatch");
    }
    let env: Envelope<T> = serde_json::from_slice(json).map_err(|_| "undecodable record")?;
    if env.v != FORMAT_VERSION {
        return Err("unsupported format version");
    }
    Ok(env)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Scan {
    pub records: u64,
    pub last_seq: u64,
    /// Bytes truncated from a torn tail.
    pub torn_bytes: Option<u64>,
}

/// Streams every record of `path` into `visit` in order, truncating a torn tail. A damaged
/// line anywhere but the end is an error: the log is append-only, so only the last write
/// can be torn.
pub fn scan_and_repair<T: DeserializeOwned>(path: &Path, mut visit: impl FnMut(&T)) -> Result<Scan, StoreError> {
    let file = match OpenOptions::new().read(true).write(true).open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Scan::default()),
        Err(e) => return Err(io_err(path)(e)),
    };
    let len = file.metadata().map_err(io_err(path))?.len();
    let mut reader = BufReader::with_capacity(256 * 1024, &file);
    let mut scan = Scan::default();
    let mut offset: u64 = 0;
    let mut line_no: u64 = 0;
    let mut buf = Vec::with_capacity(4_096);
    let limit = (MAX_RECORD_BYTES + 16) as u64;
    loop {
        buf.clear();
        let n = io::Read::take(&mut reader, limit)
            .read_until(b'\n', &mut buf)
            .map_err(io_err(path))?;
        if n == 0 {
            break;
        }
        line_no += 1;
        let start = offset;
        offset += n as u64;
        let complete = buf.last() == Some(&b'\n');
        let problem = if complete {
            match decode::<T>(&buf[..buf.len() - 1]) {
                Ok(env) if env.seq == scan.last_seq + 1 || scan.records == 0 => {
                    scan.records += 1;
                    scan.last_seq = env.seq;
                    visit(&env.rec);
                    None
                }
                Ok(_) => Some("sequence gap"),
                Err(why) => Some(why),
            }
        } else if n as u64 == limit {
            Some("line too long")
        } else {
            Some("torn line")
        };
        if let Some(why) = problem {
            if offset < len {
                return Err(StoreError::Corrupt {
                    path: path.to_owned(),
                    line: line_no,
                    why,
                });
            }
            drop(reader);
            file.set_len(start).map_err(io_err(path))?;
            file.sync_all().map_err(io_err(path))?;
            scan.torn_bytes = Some(len - start);
            break;
        }
    }
    Ok(scan)
}

fn create_dir(path: &Path) -> Result<(), StoreError> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(io_err(path))
}

fn open_append(path: &Path, create_new: bool) -> Result<File, StoreError> {
    let mut o = OpenOptions::new();
    o.append(true).mode(0o600);
    if create_new {
        o.create_new(true);
    } else {
        o.create(true);
    }
    o.open(path).map_err(io_err(path))
}

/// Appends `records` synchronously and makes them durable (recovery only).
fn append_now<T: Serialize>(path: &Path, session: &str, mut seq: u64, records: &[T]) -> Result<u64, StoreError> {
    let mut file = open_append(path, false)?;
    let mut buf = Vec::new();
    for rec in records {
        seq += 1;
        let line = encode(seq, session, rec).map_err(|e| io_err(path)(io::Error::other(e)))?;
        buf.extend_from_slice(&line);
    }
    file.write_all(&buf).map_err(io_err(path))?;
    file.sync_data().map_err(io_err(path))?;
    Ok(seq)
}

/// Consent tokens: `consent.<scope>.granted` exists only while a grant stands. A revocation
/// unlinks it before the log record is appended, and an unlink needs no free space, so a
/// revocation holds on disk even when the log cannot be written. Effective consent at start
/// is "the log says granted and the token exists" (D-017, amended).
#[derive(Clone, Debug)]
pub struct ConsentTokens {
    dir: PathBuf,
}

impl ConsentTokens {
    pub fn new(dir: &Path) -> Self {
        Self { dir: dir.to_owned() }
    }

    fn path(&self, scope: ConsentScope) -> PathBuf {
        let name = match scope {
            ConsentScope::Capture => "capture",
            ConsentScope::PersonalVoice => "personal_voice",
        };
        self.dir.join(format!("consent.{name}.granted"))
    }

    pub fn granted(&self, scope: ConsentScope) -> bool {
        self.path(scope).exists()
    }

    pub fn grant(&self, scope: ConsentScope) -> io::Result<()> {
        let path = self.path(scope);
        let f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&path)?;
        f.sync_all()?;
        File::open(&self.dir)?.sync_all()
    }

    pub fn revoke(&self, scope: ConsentScope) -> io::Result<()> {
        match fs::remove_file(self.path(scope)) {
            Ok(()) => File::open(&self.dir)?.sync_all(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
}

pub struct Opened {
    /// Held for the life of the process: one service per data directory.
    pub lock: File,
    pub tokens: ConsentTokens,
    pub session: String,
    pub ledger_path: PathBuf,
    pub ledger: LogWriter<Record>,
    pub consent: LogWriter<ConsentRecord>,
    pub consent_state: ConsentState,
    pub recovered: Recovered,
}

/// One service per data directory, taken before anything is read: a second instance would
/// otherwise "recover" the first one's live session and write the consent log with its own
/// sequence counter. The lock lives as long as the returned file (the process).
fn lock_dir(dir: &Path) -> Result<File, StoreError> {
    let path = dir.join("lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(&path)
        .map_err(io_err(&path))?;
    match lock.try_lock() {
        Ok(()) => Ok(lock),
        Err(fs::TryLockError::WouldBlock) => Err(StoreError::Locked { path: dir.to_owned() }),
        Err(fs::TryLockError::Error(e)) => Err(io_err(&path)(e)),
    }
}

/// Opens the data directory: repairs and folds the consent log, closes every session a crash
/// left open, and starts a new session log. Blocking: call from `spawn_blocking`.
pub fn open(dir: &Path, max_records: u64) -> Result<Opened, StoreError> {
    create_dir(dir)?;
    let lock = lock_dir(dir)?;
    let ledger_dir = dir.join("ledger");
    create_dir(&ledger_dir)?;
    let mut recovered = Recovered::default();

    let consent_path = dir.join("consent.log");
    let mut consent_state = ConsentState::default();
    let consent_scan = scan_and_repair::<ConsentRecord>(&consent_path, |r| consent_state.apply(r))?;
    let tokens = ConsentTokens::new(dir);
    if consent_scan.torn_bytes.is_some() {
        // A torn consent record may have been a revocation: fail closed until re-granted.
        recovered.torn_tails += 1;
        consent_state = ConsentState::default();
    }
    for scope in [ConsentScope::Capture, ConsentScope::PersonalVoice] {
        let logged = match scope {
            ConsentScope::Capture => &mut consent_state.capture,
            ConsentScope::PersonalVoice => &mut consent_state.personal_voice,
        };
        *logged = *logged && tokens.granted(scope);
        if !*logged {
            tokens.revoke(scope).map_err(io_err(dir))?;
        }
    }

    let mut unclosed: Vec<PathBuf> = fs::read_dir(&ledger_dir)
        .map_err(io_err(&ledger_dir))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "log") && !p.to_string_lossy().ends_with(".closed.log"))
        .collect();
    unclosed.sort();
    for path in unclosed {
        let session = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut fold = RecoveryFold::default();
        let scan = scan_and_repair::<Record>(&path, |r| fold.observe(r))?;
        let mut repair = Vec::new();
        if let Some(bytes) = scan.torn_bytes {
            repair.push(Record::RecoveredTornTail { bytes });
            recovered.torn_tails += 1;
        }
        let recovery = fold.finish();
        if !recovery.records.is_empty() {
            recovered.sessions += 1;
            recovered.abandoned_turns += recovery.abandoned_turns;
            recovered.uncertain_calls += recovery.uncertain_calls;
        }
        repair.extend(recovery.records);
        if !repair.is_empty() {
            append_now(&path, &session, scan.last_seq, &repair)?;
        }
        mark_closed(&path)?;
    }

    let session = uuid::Uuid::now_v7().to_string();
    let ledger_path = ledger_dir.join(format!("{session}.log"));
    let ledger = LogWriter::start(open_append(&ledger_path, true)?, session.clone(), 0, max_records)?;
    let consent = LogWriter::start(
        open_append(&consent_path, false)?,
        session.clone(),
        consent_scan.last_seq,
        u64::MAX,
    )?;
    Ok(Opened {
        lock,
        tokens,
        session,
        ledger_path,
        ledger,
        consent,
        consent_state,
        recovered,
    })
}

/// Renames `<id>.log` to `<id>.closed.log` once its `session_closed` is durable, so later
/// starts only ever scan sessions a crash left open.
pub fn mark_closed(path: &Path) -> Result<(), StoreError> {
    let closed = path.with_extension("closed.log");
    fs::rename(path, &closed).map_err(io_err(path))?;
    if let Some(parent) = path.parent()
        && let Ok(d) = File::open(parent)
    {
        let _ = d.sync_all();
    }
    Ok(())
}

// ---- the writer thread ------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WriteError {
    #[error("the ledger did not accept the record in time")]
    Stalled,
    #[error("the ledger could not be written")]
    Io,
    #[error("the ledger reached its record limit")]
    Full,
    #[error("a record exceeded the size limit")]
    TooLarge,
    #[error("the ledger is closed")]
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    Io,
    Full,
}

type Ack = oneshot::Sender<Result<u64, WriteError>>;

enum Req<T> {
    Append(T, Option<Ack>),
    Flush(Ack),
    Close(Ack),
}

pub struct LogWriter<T> {
    tx: mpsc::Sender<Req<T>>,
    fault: watch::Receiver<Option<Fault>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl<T: Serialize + Send + 'static> LogWriter<T> {
    fn start(file: File, session: String, seq: u64, max_records: u64) -> Result<Self, StoreError> {
        let (tx, rx) = mpsc::channel(QUEUE);
        let (fault_tx, fault) = watch::channel(None);
        let thread = std::thread::Builder::new()
            .name("ledger-writer".into())
            .spawn(move || writer_thread(file, &session, seq, max_records, rx, &fault_tx))
            .map_err(|e| StoreError::Io {
                path: PathBuf::from("ledger-writer"),
                source: e,
            })?;
        Ok(Self {
            tx,
            fault,
            thread: Some(thread),
        })
    }

    /// Queues a record. Waits only for queue space, at most `wait`.
    pub async fn append(&self, rec: T, wait: Duration) -> Result<(), WriteError> {
        self.tx
            .send_timeout(Req::Append(rec, None), wait)
            .await
            .map_err(|e| match e {
                mpsc::error::SendTimeoutError::Timeout(_) => WriteError::Stalled,
                mpsc::error::SendTimeoutError::Closed(_) => WriteError::Closed,
            })
    }

    /// Queues a record and returns a receiver that resolves once it is durable.
    pub async fn submit(
        &self,
        rec: T,
        wait: Duration,
    ) -> Result<oneshot::Receiver<Result<u64, WriteError>>, WriteError> {
        let (ack, rx) = oneshot::channel();
        self.tx
            .send_timeout(Req::Append(rec, Some(ack)), wait)
            .await
            .map_err(|e| match e {
                mpsc::error::SendTimeoutError::Timeout(_) => WriteError::Stalled,
                mpsc::error::SendTimeoutError::Closed(_) => WriteError::Closed,
            })?;
        Ok(rx)
    }

    /// Appends a record and waits until it is durable, at most `wait` in total.
    pub async fn append_durable(&self, rec: T, wait: Duration) -> Result<u64, WriteError> {
        let deadline = tokio::time::Instant::now() + wait;
        let rx = self.submit(rec, wait).await?;
        match tokio::time::timeout_at(deadline, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(WriteError::Closed),
            Err(_) => Err(WriteError::Stalled),
        }
    }

    /// Waits until every record queued before this call is durable; returns the last seq.
    pub async fn flush(&self, wait: Duration) -> Result<u64, WriteError> {
        let deadline = tokio::time::Instant::now() + wait;
        let (ack, rx) = oneshot::channel();
        self.tx
            .send_timeout(Req::Flush(ack), wait)
            .await
            .map_err(|_| WriteError::Stalled)?;
        match tokio::time::timeout_at(deadline, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(WriteError::Closed),
            Err(_) => Err(WriteError::Stalled),
        }
    }

    /// Makes everything durable and stops the writer thread.
    pub async fn close(mut self, wait: Duration) -> Result<u64, WriteError> {
        let deadline = tokio::time::Instant::now() + wait;
        let (ack, rx) = oneshot::channel();
        self.tx
            .send_timeout(Req::Close(ack), wait)
            .await
            .map_err(|_| WriteError::Stalled)?;
        let result = match tokio::time::timeout_at(deadline, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(WriteError::Closed),
            Err(_) => Err(WriteError::Stalled),
        };
        if let Some(thread) = self.thread.take() {
            let _ = tokio::task::spawn_blocking(move || thread.join()).await;
        }
        result
    }

    /// Resolves when the writer reports a fault (an I/O error or the record cap).
    pub fn faults(&self) -> watch::Receiver<Option<Fault>> {
        self.fault.clone()
    }
}

fn writer_thread<T: Serialize>(
    mut file: File,
    session: &str,
    mut seq: u64,
    max_records: u64,
    mut rx: mpsc::Receiver<Req<T>>,
    fault: &watch::Sender<Option<Fault>>,
) {
    let mut buf: Vec<u8> = Vec::with_capacity(256 * 1024);
    let mut acks: Vec<(Ack, Result<u64, WriteError>)> = Vec::new();
    let mut failed: Option<WriteError> = None;
    let mut written: u64 = 0;
    while let Some(first) = rx.blocking_recv() {
        let mut closing: Option<Ack> = None;
        let mut pending = vec![first];
        while pending.len() < MAX_BATCH
            && let Ok(req) = rx.try_recv()
        {
            pending.push(req);
        }
        let mut batch_acks: Vec<(Ack, u64)> = Vec::new();
        for req in pending {
            match req {
                Req::Append(rec, ack) => {
                    if let Some(e) = failed {
                        if let Some(ack) = ack {
                            acks.push((ack, Err(e)));
                        }
                        continue;
                    }
                    if written >= max_records {
                        failed = Some(WriteError::Full);
                        let _ = fault.send(Some(Fault::Full));
                        if let Some(ack) = ack {
                            acks.push((ack, Err(WriteError::Full)));
                        }
                        continue;
                    }
                    match encode(seq + 1, session, &rec) {
                        Ok(line) if line.len() <= MAX_RECORD_BYTES => {
                            seq += 1;
                            written += 1;
                            buf.extend_from_slice(&line);
                            if let Some(ack) = ack {
                                batch_acks.push((ack, seq));
                            }
                        }
                        _ => {
                            if let Some(ack) = ack {
                                acks.push((ack, Err(WriteError::TooLarge)));
                            }
                        }
                    }
                }
                Req::Flush(ack) => batch_acks.push((ack, seq)),
                Req::Close(ack) => closing = Some(ack),
            }
        }
        if failed.is_none() || !buf.is_empty() {
            let result = file.write_all(&buf).and_then(|()| file.sync_data());
            buf.clear();
            // An I/O failure overrides any earlier state: nothing in this batch is durable.
            if result.is_err() && failed != Some(WriteError::Io) {
                failed = Some(WriteError::Io);
                let _ = fault.send(Some(Fault::Io));
            }
        }
        for (ack, s) in batch_acks {
            acks.push((ack, failed.filter(|e| *e == WriteError::Io).map_or(Ok(s), Err)));
        }
        for (ack, result) in acks.drain(..) {
            let _ = ack.send(result);
        }
        if let Some(ack) = closing {
            let _ = ack.send(failed.map_or(Ok(seq), Err));
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use voice_domain::TurnId;
    use voice_domain::protocol::Outcome;
    use voice_domain::record::CloseReason;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pva-store-{name}-{}", uuid::Uuid::now_v7()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_lines(path: &Path, records: &[Record]) {
        let mut out = Vec::new();
        for (i, r) in records.iter().enumerate() {
            out.extend(encode(i as u64 + 1, "s", r).unwrap());
        }
        fs::write(path, out).unwrap();
    }

    fn read_all(path: &Path) -> Vec<Record> {
        let mut v = Vec::new();
        scan_and_repair::<Record>(path, |r| v.push(r.clone())).unwrap();
        v
    }

    #[test]
    fn the_crc_catches_a_flipped_byte() {
        let line = encode(1, "s", &Record::TurnOpened { turn: TurnId(1) }).unwrap();
        assert!(decode::<Record>(&line[..line.len() - 1]).is_ok());
        let mut bad = line.clone();
        let i = bad.len() - 5;
        bad[i] ^= 0x01;
        assert_eq!(decode::<Record>(&bad[..bad.len() - 1]).err(), Some("checksum mismatch"));
    }

    #[test]
    fn a_torn_tail_is_truncated_and_reported() {
        let dir = temp_dir("torn");
        let path = dir.join("a.log");
        write_lines(
            &path,
            &[
                Record::TurnOpened { turn: TurnId(1) },
                Record::TurnOpened { turn: TurnId(2) },
            ],
        );
        let good_len = fs::metadata(&path).unwrap().len();
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"1234abcd {\"v\":1,\"seq\":3,\"at_").unwrap();
        drop(f);
        let scan = scan_and_repair::<Record>(&path, |_| {}).unwrap();
        assert_eq!(scan.records, 2);
        assert_eq!(scan.torn_bytes, Some(28));
        assert_eq!(fs::metadata(&path).unwrap().len(), good_len, "the torn bytes are gone");
        // A complete final line with a bad checksum is also a torn write.
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"00000000 {}\n").unwrap();
        drop(f);
        assert!(scan_and_repair::<Record>(&path, |_| {}).unwrap().torn_bytes.is_some());
    }

    #[test]
    fn damage_before_the_tail_refuses_to_start() {
        let dir = temp_dir("corrupt");
        let path = dir.join("a.log");
        write_lines(
            &path,
            &[
                Record::TurnOpened { turn: TurnId(1) },
                Record::TurnOpened { turn: TurnId(2) },
            ],
        );
        let mut bytes = fs::read(&path).unwrap();
        bytes[20] ^= 0x01;
        fs::write(&path, &bytes).unwrap();
        let err = scan_and_repair::<Record>(&path, |_| {}).unwrap_err();
        assert!(matches!(err, StoreError::Corrupt { line: 1, .. }), "{err}");
        assert_eq!(
            fs::read(&path).unwrap(),
            bytes,
            "nothing is truncated when the damage is not at the tail"
        );
    }

    #[test]
    fn opening_closes_a_crashed_session_and_continues_its_sequence() {
        let dir = temp_dir("recover");
        fs::create_dir_all(dir.join("ledger")).unwrap();
        let crashed = dir.join("ledger").join("0001.log");
        write_lines(&crashed, &[Record::TurnOpened { turn: TurnId(1) }]);
        let opened = open(&dir, 100).unwrap();
        assert_eq!(
            opened.recovered,
            Recovered {
                sessions: 1,
                abandoned_turns: 1,
                uncertain_calls: 0,
                torn_tails: 0
            }
        );
        let closed = dir.join("ledger").join("0001.closed.log");
        assert!(!crashed.exists() && closed.exists());
        assert_eq!(
            read_all(&closed),
            vec![
                Record::TurnOpened { turn: TurnId(1) },
                Record::TurnAbandoned { turn: TurnId(1) },
                Record::SessionClosed {
                    reason: CloseReason::Crashed
                },
            ]
        );
        // The first open's own session was dropped without closing — a crash before any
        // turn — so the second start closes that one, and leaves 0001 alone.
        let before = fs::read(&closed).unwrap();
        drop(opened);
        let again = open(&dir, 100).unwrap();
        assert_eq!(
            again.recovered,
            Recovered {
                sessions: 1,
                abandoned_turns: 0,
                uncertain_calls: 0,
                torn_tails: 0
            }
        );
        assert_eq!(fs::read(&closed).unwrap(), before);
        let _ = Outcome::Completed;
    }

    #[test]
    fn a_tool_call_sent_before_a_crash_becomes_uncertain() {
        use voice_domain::tools::{ToolOutcome, UncertainReason};
        use voice_domain::{CallId, Digest};
        let dir = temp_dir("tool-crash");
        fs::create_dir_all(dir.join("ledger")).unwrap();
        let crashed = dir.join("ledger").join("0001.log");
        let sent = Record::ToolCallSent {
            turn: TurnId(1),
            call: CallId(4),
            tool: "fixture.note".into(),
            digest: Digest("0".repeat(16)),
        };
        write_lines(&crashed, &[Record::TurnOpened { turn: TurnId(1) }, sent]);
        let opened = open(&dir, 100).unwrap();
        assert_eq!(opened.recovered.uncertain_calls, 1);
        let recs = read_all(&dir.join("ledger").join("0001.closed.log"));
        assert_eq!(
            recs[2],
            Record::Executed {
                turn: TurnId(1),
                call: CallId(4),
                outcome: ToolOutcome::Uncertain {
                    reason: UncertainReason::ServiceRestart
                },
                reconciled: false,
            }
        );
    }

    fn consent(granted: bool) -> ConsentRecord {
        ConsentRecord {
            v: 1,
            scope: ConsentScope::Capture,
            granted,
            speaker: voice_domain::consent::Speaker::Owner,
            rights_review: None,
            deletion_scope: vec![],
        }
    }

    fn write_consent(dir: &Path, records: &[ConsentRecord]) {
        let mut out = Vec::new();
        for (i, r) in records.iter().enumerate() {
            out.extend(encode(i as u64 + 1, "s", r).unwrap());
        }
        fs::write(dir.join("consent.log"), out).unwrap();
    }

    #[test]
    fn consent_stands_only_with_its_token_and_a_torn_tail_fails_closed() {
        // Logged and token present: granted.
        let dir = temp_dir("token-ok");
        write_consent(&dir, &[consent(true)]);
        ConsentTokens::new(&dir).grant(ConsentScope::Capture).unwrap();
        assert!(open(&dir, 100).unwrap().consent_state.capture);
        // Logged, but the token is gone (a revocation whose record never landed): revoked.
        let dir = temp_dir("token-missing");
        write_consent(&dir, &[consent(true)]);
        assert!(!open(&dir, 100).unwrap().consent_state.capture);
        // A torn record at the tail may have been a revocation: revoked, and the token removed.
        let dir = temp_dir("token-torn");
        write_consent(&dir, &[consent(true)]);
        ConsentTokens::new(&dir).grant(ConsentScope::Capture).unwrap();
        let mut f = OpenOptions::new().append(true).open(dir.join("consent.log")).unwrap();
        f.write_all(b"0badc0de {\"v\":1,\"seq\":2,\"rec\":{\"v\":1,\"scope\":\"cap")
            .unwrap();
        drop(f);
        let opened = open(&dir, 100).unwrap();
        assert!(!opened.consent_state.capture);
        assert_eq!(opened.recovered.torn_tails, 1);
        assert!(!ConsentTokens::new(&dir).granted(ConsentScope::Capture));
    }

    #[tokio::test]
    async fn the_writer_acks_durable_records_and_reports_the_cap() {
        let dir = temp_dir("writer");
        let opened = open(&dir, 3).unwrap();
        let w = opened.ledger;
        let mut faults = w.faults();
        let wait = Duration::from_secs(5);
        assert_eq!(
            w.append_durable(Record::TurnOpened { turn: TurnId(1) }, wait).await,
            Ok(1)
        );
        w.append(Record::TurnOpened { turn: TurnId(2) }, wait).await.unwrap();
        assert_eq!(w.flush(wait).await, Ok(2));
        assert_eq!(
            w.append_durable(Record::TurnOpened { turn: TurnId(3) }, wait).await,
            Ok(3)
        );
        assert_eq!(
            w.append_durable(Record::TurnOpened { turn: TurnId(4) }, wait).await,
            Err(WriteError::Full)
        );
        faults.changed().await.unwrap();
        assert_eq!(*faults.borrow(), Some(Fault::Full));
        let _ = w.close(wait).await;
        assert_eq!(read_all(&opened.ledger_path).len(), 3);
    }
}
