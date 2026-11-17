//! The session service: the I/O shell around `voice-domain`.
//!
//! One actor task owns the [`Session`] reducer and is the only writer of its state. It
//! receives app messages from a bounded stdin reader, tool results from tracked tool tasks,
//! deadline expiries from its own timer set, and faults from the ledger writer; it applies
//! the reducer's outputs in order — sending to the app through a bounded stdout writer,
//! appending to the ledger, waiting for durability where the domain asks for it.

pub mod framing;
pub mod log;
pub mod store;
pub mod timers;
pub mod tools;

use log::Log;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use store::{Fault, LogWriter, StoreError, WriteError};
use timers::Timers;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufWriter};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;
use tokio::time::{Instant, sleep_until, timeout};
use tools::ToolExecutor;
use voice_domain::protocol::{ClientMessage, FailureReason, Outcome, PROTOCOL_VERSION, ServiceMessage, Stage};
use voice_domain::record::{CloseReason, Record};
use voice_domain::tools::{ToolOutcome, ToolRegistry};
use voice_domain::{CallId, Input, Limits, Output, Session};

const IN_QUEUE: usize = 256;
const OUT_QUEUE: usize = 256;
const TOOL_QUEUE: usize = 64;

pub struct Config<E> {
    pub data_dir: PathBuf,
    pub limits: Limits,
    pub tools: ToolRegistry,
    pub executor: Arc<E>,
    /// Records one session log may hold before new turns are refused (D-017).
    pub max_records: u64,
    /// How long the app has to send `hello`.
    pub handshake: Duration,
    /// Longest wait for the app to accept a message or the ledger to accept a record.
    pub io_timeout: Duration,
    pub log: Log,
    /// Closed session logs older than this are deleted at start (D-018); `None` keeps them.
    pub retention: Option<Duration>,
}

impl<E> Config<E> {
    pub fn new(data_dir: PathBuf, executor: E, log: Log) -> Self {
        Self {
            data_dir,
            limits: Limits::default(),
            tools: ToolRegistry::default(),
            executor: Arc::new(executor),
            max_records: 4_000_000,
            handshake: Duration::from_secs(5),
            io_timeout: Duration::from_secs(2),
            log,
            retention: Some(Duration::from_hours(30 * 24)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndReason {
    Shutdown,
    Eof,
    ClientUnresponsive,
    ReadError,
    HandshakeTimeout,
    ProtocolMismatch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub session: String,
    pub reason: EndReason,
    /// Inputs the reducer processed (app messages, timers, tool results, faults).
    pub inputs: u64,
    pub turns_closed: u64,
    pub refused: u64,
    pub late_dropped: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("a blocking task failed")]
    Join,
}

/// Why the actor stopped applying outputs.
enum Stop {
    ClientGone,
}

struct Actor<E> {
    session: Session,
    ledger: LogWriter<Record>,
    consent: LogWriter<voice_domain::consent::ConsentRecord>,
    out: mpsc::Sender<ServiceMessage>,
    timers: Timers,
    tasks: JoinSet<()>,
    tool_tx: mpsc::Sender<(CallId, ToolOutcome)>,
    executor: Arc<E>,
    limits: Limits,
    io_timeout: Duration,
    log: Log,
    ledger_down: bool,
    /// A ledger failure noticed while applying outputs, fed to the reducer next.
    pending_fault: Option<FailureReason>,
    /// A consent change that could not be made durable, fed to the reducer next.
    pending_consent: Option<(voice_domain::consent::ConsentScope, bool)>,
    tokens: store::ConsentTokens,
    /// The data directory, record cap and live log: `forget` rotates to a fresh log.
    data_dir: PathBuf,
    max_records: u64,
    ledger_path: PathBuf,
    ledger_faults: watch::Receiver<Option<Fault>>,
    summary: Summary,
}

/// Opens the store off the async thread; on failure tells the app why (a code, no paths).
async fn open_store(
    data_dir: PathBuf,
    max_records: u64,
    out: &mpsc::Sender<ServiceMessage>,
    log: &Log,
) -> Result<store::Opened, ServiceError> {
    let opened = tokio::task::spawn_blocking(move || store::open(&data_dir, max_records))
        .await
        .map_err(|_| ServiceError::Join)?;
    if let Ok(o) = &opened {
        let r = o.recovered;
        log.event(
            "store_opened",
            &[
                ("recovered_sessions", r.sessions.into()),
                ("abandoned_turns", r.abandoned_turns.into()),
                ("uncertain_calls", r.uncertain_calls.into()),
                ("torn_tails", r.torn_tails.into()),
            ],
        );
    }
    opened.map_err(|e| {
        let code = match e {
            StoreError::Corrupt { .. } => "store_corrupt",
            StoreError::Locked { .. } => "already_running",
            StoreError::Io { .. } => "store_io",
        };
        log.event("store_failed", &[("code", code.into())]);
        let _ = out.try_send(ServiceMessage::Error {
            code: code.into(),
            detail: "the ledger could not be opened".into(),
        });
        ServiceError::Store(e)
    })
}

pub async fn run<R, W, E>(reader: R, writer: W, config: Config<E>) -> Result<Summary, ServiceError>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
    E: ToolExecutor,
{
    let Config {
        data_dir,
        limits,
        tools,
        executor,
        max_records,
        handshake,
        io_timeout,
        log,
        retention,
    } = config;
    let (out_tx, out_rx) = mpsc::channel::<ServiceMessage>(OUT_QUEUE);
    let writer_task = tokio::spawn(write_loop(writer, out_rx));

    let opened = match open_store(data_dir.clone(), max_records, &out_tx, &log).await {
        Ok(o) => o,
        Err(e) => {
            drop(out_tx);
            let _ = timeout(io_timeout, writer_task).await;
            return Err(e);
        }
    };
    let consent_state = opened.consent_state;
    let recovered = opened.recovered;
    let session_id = opened.session.clone();
    let (in_tx, mut in_rx) = mpsc::channel(IN_QUEUE);
    let reader_task = tokio::spawn(framing::read_loop(reader, in_tx, framing::MAX_LINE));
    let (tool_tx, mut tool_rx) = mpsc::channel(TOOL_QUEUE);
    let mut consent_faults = opened.consent.faults();
    let parts = Parts {
        tools,
        executor,
        limits,
        io_timeout,
        log,
        data_dir,
        max_records,
        out: out_tx,
        tool_tx,
    };
    let (mut actor, _lock) = Actor::new(opened, parts);

    let reason = match actor.handshake(&mut in_rx, handshake).await {
        Err(reason) => reason,
        Ok(client) => {
            actor
                .record(Record::SessionOpened {
                    protocol: PROTOCOL_VERSION,
                    client,
                })
                .await;
            actor.expire(retention).await;
            let ready = ServiceMessage::Ready {
                protocol: PROTOCOL_VERSION,
                session: session_id.clone(),
                consent: consent_state,
                recovered,
            };
            if actor.send(ready).await.is_err() {
                EndReason::ClientUnresponsive
            } else {
                actor.log.event("session_ready", &[]);
                actor.serve(&mut in_rx, &mut tool_rx, &mut consent_faults).await
            }
        }
    };
    actor.summary.reason = reason;
    let summary = actor.finish().await;
    reader_task.abort();
    let _ = timeout(io_timeout, writer_task).await;
    Ok(summary)
}

/// What the actor needs besides the opened store.
struct Parts<E> {
    tools: ToolRegistry,
    executor: Arc<E>,
    limits: Limits,
    io_timeout: Duration,
    log: Log,
    data_dir: PathBuf,
    max_records: u64,
    out: mpsc::Sender<ServiceMessage>,
    tool_tx: mpsc::Sender<(CallId, ToolOutcome)>,
}

impl<E: ToolExecutor> Actor<E> {
    /// Builds the actor; also returns the data-directory lock, which the caller keeps alive.
    fn new(opened: store::Opened, p: Parts<E>) -> (Self, std::fs::File) {
        let store::Opened {
            lock,
            tokens,
            session,
            ledger_path,
            ledger,
            consent,
            consent_state,
            recovered: _,
        } = opened;
        let ledger_faults = ledger.faults();
        let actor = Self {
            session: Session::new(p.limits.clone(), p.tools, consent_state),
            ledger,
            consent,
            out: p.out,
            timers: Timers::default(),
            tasks: JoinSet::new(),
            tool_tx: p.tool_tx,
            executor: p.executor,
            limits: p.limits,
            io_timeout: p.io_timeout,
            log: p.log,
            ledger_down: false,
            pending_fault: None,
            pending_consent: None,
            tokens,
            data_dir: p.data_dir,
            max_records: p.max_records,
            ledger_path,
            ledger_faults,
            summary: Summary {
                session,
                reason: EndReason::Eof,
                inputs: 0,
                turns_closed: 0,
                refused: 0,
                late_dropped: 0,
            },
        };
        (actor, lock)
    }

    /// Retention (D-018): deletes closed sessions past `retention` and records how many.
    async fn expire(&mut self, retention: Option<Duration>) {
        let Some(older_than) = retention else { return };
        let dir = self.data_dir.clone();
        let Ok(Ok((files, bytes))) = tokio::task::spawn_blocking(move || store::expire_history(&dir, older_than)).await
        else {
            self.log.event("retention_failed", &[]);
            return;
        };
        if files > 0 {
            let retention_days = u32::try_from(older_than.as_secs() / 86_400).unwrap_or(u32::MAX);
            self.record(Record::HistoryExpired {
                files,
                bytes,
                retention_days,
            })
            .await;
            self.log.event("history_expired", &[("files", files.into())]);
        }
    }

    /// Carries out a consent scope's deletion scope now (D-018). For capture: every heard
    /// transcript, closed or live. The live log is rotated: a fresh one is opened first, the old
    /// writer is closed, then every other log is deleted.
    async fn forget(&mut self, scope: voice_domain::consent::ConsentScope) -> Result<(), Stop> {
        if self.session.active_turn().is_some() {
            let turn = self.session.active_turn();
            return self
                .send(ServiceMessage::Notice {
                    code: "forget_refused_turn_active".into(),
                    turn,
                })
                .await;
        }
        if scope == voice_domain::consent::ConsentScope::PersonalVoice {
            // No personal-voice data can exist in the first release.
            return self
                .send(ServiceMessage::Forgotten {
                    scope,
                    files: 0,
                    bytes: 0,
                })
                .await;
        }
        let (dir, max) = (self.data_dir.clone(), self.max_records);
        let Ok(Ok((session, path, writer))) =
            tokio::task::spawn_blocking(move || store::new_session_log(&dir, max)).await
        else {
            return self.error("forget_failed", "a new log could not be started").await;
        };
        let old = std::mem::replace(&mut self.ledger, writer);
        self.ledger_faults = self.ledger.faults();
        self.ledger_path.clone_from(&path);
        self.summary.session = session;
        let _ = old.close(self.io_timeout).await;
        let dir = self.data_dir.clone();
        let deleted = tokio::task::spawn_blocking(move || store::delete_history(&dir, &path)).await;
        let Ok(Ok((files, bytes))) = deleted else {
            return self.error("forget_failed", "some history could not be deleted").await;
        };
        self.record(Record::SessionOpened {
            protocol: PROTOCOL_VERSION,
            client: "after-forget".into(),
        })
        .await;
        self.record(Record::HistoryDeleted { scope, files, bytes }).await;
        self.log.event("history_deleted", &[("files", files.into())]);
        self.send(ServiceMessage::Forgotten { scope, files, bytes }).await
    }

    async fn handshake(
        &mut self,
        in_rx: &mut mpsc::Receiver<framing::Inbound>,
        wait: Duration,
    ) -> Result<String, EndReason> {
        let deadline = Instant::now() + wait;
        loop {
            let Ok(next) = tokio::time::timeout_at(deadline, in_rx.recv()).await else {
                self.log.event("handshake_timeout", &[]);
                return Err(EndReason::HandshakeTimeout);
            };
            match next {
                Some(framing::Inbound::Message(ClientMessage::Hello { protocol, client })) => {
                    if protocol != PROTOCOL_VERSION {
                        let _ = self.error("protocol_mismatch", "this service speaks protocol 1").await;
                        return Err(EndReason::ProtocolMismatch);
                    }
                    return Ok(client.chars().take(64).collect());
                }
                Some(framing::Inbound::Message(_)) => {
                    let _ = self.error("hello_required", "send hello first").await;
                }
                Some(framing::Inbound::Bad(code)) => {
                    let _ = self.error(code, "malformed line").await;
                }
                Some(framing::Inbound::ReadError) => return Err(EndReason::ReadError),
                Some(framing::Inbound::Eof) | None => return Err(EndReason::Eof),
            }
        }
    }

    async fn serve(
        &mut self,
        in_rx: &mut mpsc::Receiver<framing::Inbound>,
        tool_rx: &mut mpsc::Receiver<(CallId, ToolOutcome)>,
        consent_faults: &mut watch::Receiver<Option<Fault>>,
    ) -> EndReason {
        loop {
            let next_deadline = self.timers.next();
            let result = tokio::select! {
                biased;
                inbound = in_rx.recv() => match inbound {
                    Some(framing::Inbound::Message(msg)) => self.message(msg).await,
                    Some(framing::Inbound::Bad(code)) => self.error(code, "malformed line").await.map(|()| None),
                    Some(framing::Inbound::ReadError) => return EndReason::ReadError,
                    Some(framing::Inbound::Eof) | None => return EndReason::Eof,
                },
                Some((call, outcome)) = tool_rx.recv() => self.input(Input::ToolFinished { call, outcome }).await.map(|()| None),
                Ok(()) = self.ledger_faults.changed() => {
                    let fault = *self.ledger_faults.borrow();
                    self.ledger_fault(fault).await.map(|()| None)
                }
                Ok(()) = consent_faults.changed() => {
                    let fault = *consent_faults.borrow();
                    self.ledger_fault(fault).await.map(|()| None)
                }
                () = sleep_until(next_deadline.unwrap_or_else(Instant::now)), if next_deadline.is_some() => {
                    self.fire_due().await.map(|()| None)
                }
                Some(joined) = self.tasks.join_next(), if !self.tasks.is_empty() => {
                    if joined.is_err() {
                        self.log.event("tool_task_failed", &[]);
                    }
                    Ok(None)
                }
            };
            match result {
                Ok(Some(end)) => return end,
                Ok(None) => {}
                Err(Stop::ClientGone) => return EndReason::ClientUnresponsive,
            }
        }
    }

    async fn message(&mut self, msg: ClientMessage) -> Result<Option<EndReason>, Stop> {
        match msg {
            ClientMessage::Shutdown => Ok(Some(EndReason::Shutdown)),
            ClientMessage::Hello { .. } => self
                .error("duplicate_hello", "hello was already received")
                .await
                .map(|()| None),
            ClientMessage::Ping { seq } => self.send(ServiceMessage::Pong { seq }).await.map(|()| None),
            ClientMessage::Forget { scope } => self.forget(scope).await.map(|()| None),
            ClientMessage::Flush => {
                match self.ledger.flush(self.io_timeout).await {
                    Ok(seq) => self.send(ServiceMessage::Flushed { seq }).await?,
                    Err(e) => {
                        self.ledger_error(e);
                        self.drain_faults().await?;
                    }
                }
                Ok(None)
            }
            other => self.input(Input::Client(other)).await.map(|()| None),
        }
    }

    async fn fire_due(&mut self) -> Result<(), Stop> {
        for timer in self.timers.pop_due(Instant::now()) {
            self.input(Input::TimerFired(timer)).await?;
        }
        Ok(())
    }

    async fn ledger_fault(&mut self, fault: Option<Fault>) -> Result<(), Stop> {
        match fault {
            Some(Fault::Full) => self.ledger_down(FailureReason::LedgerFull),
            Some(Fault::Io) => self.ledger_down(FailureReason::LedgerIo),
            None => {}
        }
        self.drain_faults().await
    }

    fn ledger_error(&mut self, e: WriteError) {
        self.ledger_down(if e == WriteError::Full {
            FailureReason::LedgerFull
        } else {
            FailureReason::LedgerIo
        });
    }

    /// Notes the first ledger failure; the reducer hears about it once, after the current step.
    fn ledger_down(&mut self, reason: FailureReason) {
        if self.ledger_down {
            return;
        }
        self.ledger_down = true;
        self.pending_fault = Some(reason);
        self.log
            .event("ledger_unavailable", &[("reason", reason_name(reason).into())]);
    }

    async fn drain_faults(&mut self) -> Result<(), Stop> {
        match self.pending_fault.take() {
            Some(reason) => self.input(Input::LedgerUnavailable { reason }).await,
            None => Ok(()),
        }
    }

    /// Feeds one input through the reducer and applies its outputs in order; a ledger failure
    /// found while applying becomes the next input.
    async fn input(&mut self, input: Input) -> Result<(), Stop> {
        let mut next = Some(input);
        while let Some(input) = next.take() {
            self.summary.inputs += 1;
            match self.session.step(input) {
                Ok(outputs) => self.apply(outputs).await?,
                Err(refused) => {
                    self.summary.refused += 1;
                    self.log.event("refused", &[("code", refused.code().into())]);
                    let notice = ServiceMessage::Notice {
                        code: refused.code().into(),
                        turn: self.session.active_turn(),
                    };
                    self.send(notice).await?;
                }
            }
            next = match self.pending_consent.take() {
                Some((scope, granted)) => Some(Input::ConsentUnavailable { scope, granted }),
                None => self
                    .pending_fault
                    .take()
                    .map(|reason| Input::LedgerUnavailable { reason }),
            };
        }
        Ok(())
    }

    async fn apply(&mut self, outputs: Vec<Output>) -> Result<(), Stop> {
        let mut outputs = outputs.into_iter();
        while let Some(output) = outputs.next() {
            match output {
                Output::Send(msg) => {
                    self.observe(&msg);
                    self.send(msg).await?;
                }
                Output::Record(rec) => {
                    if matches!(rec, Record::LateResultDropped { .. }) {
                        self.summary.late_dropped += 1;
                    }
                    self.record(rec).await;
                }
                Output::RecordDurable(rec) => {
                    if let Err(e) = self.ledger.append_durable(rec, self.io_timeout).await {
                        // Not durable means not safe to continue: a tool must never run
                        // without its record. Drop the rest of this step's outputs.
                        drop(outputs);
                        self.ledger_error(e);
                        return Ok(());
                    }
                }
                Output::Consent(rec) => {
                    let (scope, granted) = (rec.scope, rec.granted);
                    if !granted {
                        // Revoke on disk first: unlinking the token needs no free space.
                        let tokens = self.tokens.clone();
                        if !matches!(
                            tokio::task::spawn_blocking(move || tokens.revoke(scope)).await,
                            Ok(Ok(()))
                        ) {
                            self.log.event("consent_token_unlink_failed", &[]);
                        }
                    }
                    let mut durable = self.consent.append_durable(rec, self.io_timeout).await.is_ok();
                    if durable && granted {
                        let tokens = self.tokens.clone();
                        durable = matches!(
                            tokio::task::spawn_blocking(move || tokens.grant(scope)).await,
                            Ok(Ok(()))
                        );
                    }
                    if !durable {
                        self.log.event("consent_unrecorded", &[("granted", granted.into())]);
                        self.pending_consent = Some((scope, granted));
                        if granted {
                            // A grant that is not durable does not stand: send nothing of it.
                            drop(outputs);
                            return Ok(());
                        }
                        // A revocation stands anyway: finish the step, so the app hears the
                        // turn close and the revoked state.
                    }
                }
                Output::Arm { timer, after_ms } => {
                    self.timers.arm(timer, Instant::now() + Duration::from_millis(after_ms));
                }
                Output::Disarm { timer } => self.timers.disarm(timer),
                Output::Execute { call, action } => {
                    let executor = Arc::clone(&self.executor);
                    let tx = self.tool_tx.clone();
                    // The domain's tool deadline marks the call uncertain; this backstop only
                    // keeps a hung tool from living forever. Its silence stays uncertain.
                    let backstop = Duration::from_millis(self.limits.tool_ms.saturating_mul(4));
                    self.tasks.spawn(async move {
                        if let Ok(outcome) = timeout(backstop, executor.execute(action)).await {
                            let _ = tx.send((call, outcome)).await;
                        }
                    });
                }
            }
        }
        Ok(())
    }

    fn observe(&mut self, msg: &ServiceMessage) {
        match msg {
            ServiceMessage::TurnClosed { turn, outcome } => {
                self.summary.turns_closed += 1;
                self.log.event(
                    "turn_closed",
                    &[("turn", turn.0.into()), ("outcome", outcome_name(*outcome).into())],
                );
            }
            ServiceMessage::Degraded { stage, reason, .. } => {
                self.log.event(
                    "degraded",
                    &[
                        ("stage", stage_name(*stage).into()),
                        ("reason", reason_name(*reason).into()),
                    ],
                );
            }
            _ => {}
        }
    }

    async fn record(&mut self, rec: Record) {
        if self.ledger_down {
            return;
        }
        if let Err(e) = self.ledger.append(rec, self.io_timeout).await {
            self.ledger_error(e);
        }
    }

    async fn send(&self, msg: ServiceMessage) -> Result<(), Stop> {
        self.out
            .send_timeout(msg, self.io_timeout)
            .await
            .map_err(|_| Stop::ClientGone)
    }

    async fn error(&self, code: &str, detail: &str) -> Result<(), Stop> {
        self.send(ServiceMessage::Error {
            code: code.into(),
            detail: detail.into(),
        })
        .await
    }

    /// Ends the session honestly: an active turn is interrupted (a running tool becomes
    /// uncertain), `session_closed` is made durable, and the log is marked closed.
    async fn finish(mut self) -> Summary {
        if self.session.active_turn().is_some()
            && let Ok(outputs) = self.session.step(Input::Client(ClientMessage::Interrupt))
        {
            let _ = self.apply(outputs).await;
        }
        self.tasks.shutdown().await;
        let reason = self.summary.reason;
        let closed = Record::SessionClosed {
            reason: CloseReason::Clean,
        };
        let durable = self.ledger.append_durable(closed, self.io_timeout).await.is_ok();
        let _ = self.ledger.close(self.io_timeout).await;
        let _ = self.consent.close(self.io_timeout).await;
        if durable {
            let path = self.ledger_path.clone();
            let _ = tokio::task::spawn_blocking(move || store::mark_closed(&path)).await;
        }
        self.log.event(
            "session_end",
            &[
                ("reason", end_name(reason).into()),
                ("inputs", self.summary.inputs.into()),
                ("turns", self.summary.turns_closed.into()),
                ("refused", self.summary.refused.into()),
                ("late_dropped", self.summary.late_dropped.into()),
                ("log_dropped", self.log.dropped().into()),
            ],
        );
        self.summary
    }
}

/// Writes service messages as JSON lines, flushing whenever the queue is momentarily empty.
async fn write_loop<W: AsyncWrite + Unpin>(writer: W, mut rx: mpsc::Receiver<ServiceMessage>) {
    let mut w = BufWriter::with_capacity(64 * 1024, writer);
    let mut line = Vec::with_capacity(1_024);
    while let Some(msg) = rx.recv().await {
        line.clear();
        if serde_json::to_writer(&mut line, &msg).is_err() {
            continue;
        }
        line.push(b'\n');
        if w.write_all(&line).await.is_err() {
            return;
        }
        if rx.is_empty() && w.flush().await.is_err() {
            return;
        }
    }
    let _ = w.flush().await;
}

fn outcome_name(o: Outcome) -> &'static str {
    match o {
        Outcome::Completed => "completed",
        Outcome::Interrupted => "interrupted",
        Outcome::Degraded => "degraded",
        Outcome::Refused => "refused",
        Outcome::Rejected => "rejected",
        Outcome::Expired => "expired",
        Outcome::ConsentRevoked => "consent_revoked",
        Outcome::Empty => "empty",
        Outcome::Abandoned => "abandoned",
    }
}

fn stage_name(s: Stage) -> &'static str {
    match s {
        Stage::Stt => "stt",
        Stage::Lm => "lm",
        Stage::Tts => "tts",
        Stage::Tool => "tool",
        Stage::Ledger => "ledger",
    }
}

fn reason_name(r: FailureReason) -> &'static str {
    match r {
        FailureReason::Deadline => "deadline",
        FailureReason::Unavailable => "unavailable",
        FailureReason::ModelNotReady => "model_not_ready",
        FailureReason::AssetsMissing => "assets_missing",
        FailureReason::ContextWindow => "context_window",
        FailureReason::Guardrail => "guardrail",
        FailureReason::AudioFormat => "audio_format",
        FailureReason::Cancelled => "cancelled",
        FailureReason::TooLong => "too_long",
        FailureReason::Protocol => "protocol",
        FailureReason::LedgerFull => "ledger_full",
        FailureReason::LedgerIo => "ledger_io",
        FailureReason::Error => "error",
    }
}

fn end_name(r: EndReason) -> &'static str {
    match r {
        EndReason::Shutdown => "shutdown",
        EndReason::Eof => "eof",
        EndReason::ClientUnresponsive => "client_unresponsive",
        EndReason::ReadError => "read_error",
        EndReason::HandshakeTimeout => "handshake_timeout",
        EndReason::ProtocolMismatch => "protocol_mismatch",
    }
}
