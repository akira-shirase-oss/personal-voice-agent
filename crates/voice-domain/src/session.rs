//! The turn state machine as a pure reducer: `step(input) → outputs`. No clock, no I/O.
//! Deadlines are outputs (`Arm`) that come back as inputs (`TimerFired`), so every run is
//! replayable from its inputs alone.
//!
//! A refused input returns `Err(Refused)` and leaves the session exactly as it was.

use crate::consent::{self, ConsentRecord, ConsentScope, ConsentState, PERSONAL_VOICE_AVAILABLE, Speaker};
use crate::guard;
use crate::ids::{CallId, Digest, TurnId};
use crate::limits::Limits;
use crate::policy::{self, Decision, Heard, Refusal};
use crate::proposal::{Action, Proposal, ProposalKind, Slot};
use crate::protocol::{ClientMessage, ExecutionStatus, FailureReason, Outcome, ServiceMessage, Stage};
use crate::record::Record;
use crate::tools::{ToolOutcome, ToolRegistry, UncertainReason};
use std::collections::BTreeMap;

/// Tool calls kept for reconciliation after being marked uncertain.
const MAX_UNRESOLVED_TOOLS: usize = 1_024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TimerKind {
    /// First output of a call (model's first stream message, synthesis's first audio).
    First,
    /// The whole call.
    Total,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timer {
    pub call: CallId,
    pub kind: TimerKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Client(ClientMessage),
    TimerFired(Timer),
    ToolFinished {
        call: CallId,
        outcome: ToolOutcome,
    },
    /// The shell could not write the ledger (I/O error or record cap).
    LedgerUnavailable {
        reason: FailureReason,
    },
    /// The shell could not make a consent change durable. A failed grant is undone (fail
    /// closed); a failed revocation stays in force; no further grant is accepted.
    ConsentUnavailable {
        scope: ConsentScope,
        granted: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Output {
    Send(ServiceMessage),
    /// Append to the turn ledger.
    Record(Record),
    /// Append to the turn ledger and wait until it is durable before the next output.
    RecordDurable(Record),
    /// Append to the consent log and wait until it is durable before the next output.
    Consent(ConsentRecord),
    Arm {
        timer: Timer,
        after_ms: u64,
    },
    Disarm {
        timer: Timer,
    },
    /// Run a tool. Always preceded in the same step by `RecordDurable(ToolCallSent)`.
    Execute {
        call: CallId,
        action: Action,
    },
}

/// A refused input, by name. The session is unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Refused {
    #[error("capture consent is required before push-to-talk")]
    ConsentRequired,
    #[error("the ledger is unavailable; no new turns")]
    LedgerUnavailable,
    #[error("push-to-talk is already held")]
    AlreadyCapturing,
    #[error("push-to-talk release does not match a capturing turn")]
    StalePushToTalk,
    #[error("there is no active turn")]
    NoActiveTurn,
    #[error("no confirmation is pending for that turn")]
    NotAwaitingConfirmation,
    #[error("the confirmation does not match the pending proposal")]
    DigestMismatch,
    #[error("personal voice needs a usage-rights review reference")]
    RightsReviewRequired,
    #[error("personal voice is not available in this release")]
    PersonalVoiceUnavailable,
    #[error("handled by the service shell, not the session")]
    ShellMessage,
    #[error("consent cannot be recorded, so it cannot be granted")]
    ConsentLogUnavailable,
}

impl Refused {
    pub fn code(self) -> &'static str {
        match self {
            Self::ConsentRequired => "consent_required",
            Self::LedgerUnavailable => "ledger_unavailable",
            Self::AlreadyCapturing => "already_capturing",
            Self::StalePushToTalk => "stale_push_to_talk",
            Self::NoActiveTurn => "no_active_turn",
            Self::NotAwaitingConfirmation => "not_awaiting_confirmation",
            Self::DigestMismatch => "digest_mismatch",
            Self::RightsReviewRequired => "rights_review_required",
            Self::PersonalVoiceUnavailable => "personal_voice_unavailable",
            Self::ShellMessage => "shell_message",
            Self::ConsentLogUnavailable => "consent_log_unavailable",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    limits: Limits,
    tools: ToolRegistry,
    consent: ConsentState,
    next_turn: u64,
    next_call: u64,
    turn: Option<Turn>,
    /// Chunks of turns below the fence never play.
    fence: TurnId,
    /// Tool calls marked uncertain whose real result may still arrive.
    unresolved: BTreeMap<CallId, TurnId>,
    ledger_ok: bool,
    /// False once a consent change could not be recorded: grants are refused from then on.
    consent_writable: bool,
}

#[derive(Clone, Debug, PartialEq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent facts about one turn, each read on its own"
)]
struct Turn {
    id: TurnId,
    heard: Option<Heard>,
    /// A tool returned success with evidence in this turn.
    executed: bool,
    /// The heard request may ask for an action (D-027).
    asks_action: bool,
    /// A reply sentence of this turn was replaced or flagged as a possible claim.
    flagged: bool,
    /// A tool call was sent: its result may be uncertain, so "nothing has been done" would not
    /// be known to be true.
    tool_sent: bool,
    /// The turn-level disclosure was given.
    disclosed: bool,
    phase: Phase,
}

#[derive(Clone, Debug, PartialEq)]
enum Phase {
    Capturing {
        capture: CallId,
    },
    Transcribing {
        stt: CallId,
    },
    /// Waiting for the model and/or speaking: covers "thinking", streaming and announcements.
    Responding(Responding),
    AwaitingConfirmation {
        window: CallId,
        action: Action,
        digest: Digest,
    },
    Executing {
        tool: CallId,
    },
}

#[derive(Clone, Debug, PartialEq)]
struct Responding {
    lm: Option<Lm>,
    chunks: BTreeMap<CallId, bool>,
    next_seq: u32,
    /// The outcome once the model is done and every chunk has played.
    outcome: Outcome,
}

#[derive(Clone, Debug, PartialEq)]
struct Lm {
    call: CallId,
    kind: Option<ProposalKind>,
    sentences: u32,
    seen_output: bool,
}

impl Session {
    pub fn new(limits: Limits, tools: ToolRegistry, consent: ConsentState) -> Self {
        Self {
            limits,
            tools,
            consent,
            next_turn: 1,
            next_call: 1,
            turn: None,
            fence: TurnId(0),
            unresolved: BTreeMap::new(),
            ledger_ok: true,
            consent_writable: true,
        }
    }

    pub fn consent(&self) -> ConsentState {
        self.consent
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    pub fn active_turn(&self) -> Option<TurnId> {
        self.turn.as_ref().map(|t| t.id)
    }

    pub fn fence(&self) -> TurnId {
        self.fence
    }

    /// Human-readable phase of the active turn, for tests and diagnostics.
    pub fn phase_name(&self) -> &'static str {
        match self.turn.as_ref().map(|t| &t.phase) {
            None => "idle",
            Some(Phase::Capturing { .. }) => "capturing",
            Some(Phase::Transcribing { .. }) => "transcribing",
            Some(Phase::Responding(r)) if r.lm.is_some() && r.chunks.is_empty() => "thinking",
            Some(Phase::Responding(_)) => "speaking",
            Some(Phase::AwaitingConfirmation { .. }) => "awaiting_confirmation",
            Some(Phase::Executing { .. }) => "executing",
        }
    }

    pub fn step(&mut self, input: Input) -> Result<Vec<Output>, Refused> {
        let mut out = Vec::new();
        match input {
            Input::Client(msg) => self.client(msg, &mut out)?,
            Input::TimerFired(timer) => self.timer(timer, &mut out),
            Input::ToolFinished { call, outcome } => self.tool_finished(call, outcome, &mut out),
            Input::ConsentUnavailable { scope, granted } => self.consent_unavailable(scope, granted, &mut out),
            Input::LedgerUnavailable { reason } => {
                self.ledger_ok = false;
                if self.turn.is_some() {
                    self.fail(Stage::Ledger, None, reason, &mut out);
                } else {
                    out.push(Output::Send(ServiceMessage::Degraded {
                        turn: None,
                        stage: Stage::Ledger,
                        reason,
                    }));
                }
            }
        }
        Ok(out)
    }

    fn alloc_call(&mut self) -> CallId {
        let id = CallId(self.next_call);
        self.next_call += 1;
        id
    }

    fn client(&mut self, msg: ClientMessage, out: &mut Vec<Output>) -> Result<(), Refused> {
        match msg {
            ClientMessage::Hello { .. }
            | ClientMessage::Flush
            | ClientMessage::Ping { .. }
            | ClientMessage::Forget { .. }
            | ClientMessage::Shutdown => {
                return Err(Refused::ShellMessage);
            }
            ClientMessage::PttDown => self.ptt_down(out)?,
            ClientMessage::PttUp { turn } => {
                match &self.turn {
                    Some(t) if t.id == turn && matches!(t.phase, Phase::Capturing { .. }) => {}
                    _ => return Err(Refused::StalePushToTalk),
                }
                self.end_capture(out);
            }
            ClientMessage::Interrupt => {
                if self.turn.is_none() {
                    return Err(Refused::NoActiveTurn);
                }
                self.interrupt(out);
            }
            ClientMessage::Confirm { turn, digest, approve } => self.confirm(turn, &digest, approve, out)?,
            ClientMessage::Consent {
                scope,
                grant,
                rights_review,
            } => self.consent_change(scope, grant, rights_review, out)?,
            ClientMessage::SttFinal {
                call,
                text,
                min_confidence,
            } => self.stt_final(call, text, min_confidence, out),
            ClientMessage::LmKind { call, kind } => self.lm_kind(call, kind, out),
            ClientMessage::LmSentence { call, index, text } => self.lm_sentence(call, index, &text, out),
            ClientMessage::LmDone {
                call,
                proposal,
                model,
                instructions,
            } => self.lm_done(call, proposal, &model, &instructions, out),
            ClientMessage::TtsStarted { call } => self.tts_started(call, out),
            ClientMessage::TtsFinished { call } => self.tts_finished(call, out),
            ClientMessage::SttFailed { call, reason } => {
                self.provider_failed(call, Stage::Stt, reason, "stt_failed", out);
            }
            ClientMessage::LmFailed { call, reason } => self.provider_failed(call, Stage::Lm, reason, "lm_failed", out),
            ClientMessage::TtsFailed { call, reason } => {
                self.provider_failed(call, Stage::Tts, reason, "tts_failed", out);
            }
        }
        Ok(())
    }

    // ---- turn lifecycle -------------------------------------------------------------

    fn ptt_down(&mut self, out: &mut Vec<Output>) -> Result<(), Refused> {
        if !self.consent.capture {
            return Err(Refused::ConsentRequired);
        }
        if !self.ledger_ok {
            return Err(Refused::LedgerUnavailable);
        }
        if let Some(t) = &self.turn {
            if matches!(t.phase, Phase::Capturing { .. }) {
                return Err(Refused::AlreadyCapturing);
            }
            // Barge-in: stop the assistant before the microphone opens (D-023).
            self.interrupt(out);
        }
        let id = TurnId(self.next_turn);
        self.next_turn += 1;
        let capture = self.alloc_call();
        self.turn = Some(Turn {
            id,
            heard: None,
            executed: false,
            asks_action: false,
            flagged: false,
            tool_sent: false,
            disclosed: false,
            phase: Phase::Capturing { capture },
        });
        out.push(Output::Record(Record::TurnOpened { turn: id }));
        out.push(Output::Send(ServiceMessage::TurnOpened { turn: id }));
        out.push(Output::Send(ServiceMessage::Capture { turn: id, on: true }));
        out.push(arm(capture, TimerKind::Total, self.limits.capture_max_ms));
        Ok(())
    }

    fn end_capture(&mut self, out: &mut Vec<Output>) {
        let stt = self.alloc_call();
        let finalize_ms = self.limits.stt_finalize_ms;
        let Some(t) = self.turn.as_mut() else { return };
        let Phase::Capturing { capture } = t.phase else {
            return;
        };
        out.push(Output::Disarm {
            timer: Timer {
                call: capture,
                kind: TimerKind::Total,
            },
        });
        out.push(Output::Send(ServiceMessage::Capture { turn: t.id, on: false }));
        out.push(Output::Send(ServiceMessage::SttRequest {
            call: stt,
            turn: t.id,
            deadline_ms: finalize_ms,
        }));
        out.push(arm(stt, TimerKind::Total, finalize_ms));
        t.phase = Phase::Transcribing { stt };
    }

    /// Every call the active turn is waiting on, and whether it is a provider call the app
    /// should be told to cancel.
    fn pending_calls(phase: &Phase) -> Vec<(CallId, bool)> {
        match phase {
            Phase::Capturing { capture } => vec![(*capture, false)],
            Phase::Transcribing { stt } => vec![(*stt, true)],
            Phase::Responding(r) => {
                let mut calls: Vec<(CallId, bool)> = r.lm.iter().map(|lm| (lm.call, true)).collect();
                calls.extend(r.chunks.keys().map(|c| (*c, true)));
                calls
            }
            Phase::AwaitingConfirmation { window, .. } => vec![(*window, false)],
            Phase::Executing { tool } => vec![(*tool, false)],
        }
    }

    /// Ends the active turn abnormally: playback stops first, every pending call is cancelled,
    /// and a sent tool becomes uncertain. The caller records why and closes the turn.
    fn abort(&mut self, uncertain: UncertainReason, out: &mut Vec<Output>) -> Vec<CallId> {
        let Some(t) = self.turn.take() else {
            return Vec::new();
        };
        self.fence = self.fence.max(TurnId(t.id.0 + 1));
        out.push(Output::Send(ServiceMessage::StopPlayback { fence: self.fence }));
        if matches!(t.phase, Phase::Capturing { .. }) {
            out.push(Output::Send(ServiceMessage::Capture { turn: t.id, on: false }));
        }
        let mut cancelled = Vec::new();
        for (call, provider) in Self::pending_calls(&t.phase) {
            disarm_all(call, out);
            if provider {
                out.push(Output::Send(ServiceMessage::Cancel { call }));
                cancelled.push(call);
            }
        }
        if let Phase::Executing { tool } = t.phase {
            out.push(Output::Record(Record::Executed {
                turn: t.id,
                call: tool,
                outcome: ToolOutcome::Uncertain { reason: uncertain },
                reconciled: false,
            }));
            out.push(executed(t.id, tool, ExecutionStatus::Uncertain));
            self.remember_unresolved(tool, t.id);
        }
        self.turn = Some(t);
        cancelled
    }

    fn interrupt(&mut self, out: &mut Vec<Output>) {
        let Some(turn) = self.active_turn() else {
            return;
        };
        let cancelled = self.abort(UncertainReason::Interrupted, out);
        out.push(Output::Record(Record::Interrupted { turn, cancelled }));
        self.close(Outcome::Interrupted, out);
    }

    /// Closes a provider-, tool- or ledger-failed turn as degraded. Nothing is invented: the
    /// ledger gets the failure and the app gets the stage and reason.
    fn fail(&mut self, stage: Stage, call: Option<CallId>, reason: FailureReason, out: &mut Vec<Output>) {
        let Some(turn) = self.active_turn() else {
            return;
        };
        if let Some(call) = call {
            out.push(Output::Record(Record::ProviderFailed {
                turn,
                call,
                stage,
                reason,
            }));
        }
        // A tool still running is uncertain for the reason the turn failed, not always "deadline".
        let uncertain = match (stage, reason) {
            (Stage::Ledger, _) => UncertainReason::LedgerUnavailable,
            (_, FailureReason::Deadline) => UncertainReason::Deadline,
            _ => UncertainReason::ProviderFailed,
        };
        self.abort(uncertain, out);
        out.push(Output::Send(ServiceMessage::Degraded {
            turn: Some(turn),
            stage,
            reason,
        }));
        self.close(Outcome::Degraded, out);
    }

    fn close(&mut self, outcome: Outcome, out: &mut Vec<Output>) {
        let Some(t) = self.turn.take() else { return };
        // A turn that qualified for the disclosure but ended another way (a model failure or
        // deadline, an interrupt, a revocation, a refusal): it cannot be spoken any more, so it
        // is recorded and the app shows it with the turn (D-027).
        // A disclosure that was being spoken when the turn was cut off may not have been heard,
        // so the app is told either way; the record is written once.
        let qualified = (t.asks_action || t.flagged) && !t.executed && !t.tool_sent;
        if qualified && outcome != Outcome::Completed {
            if !t.disclosed {
                out.push(Output::Record(Record::TurnDisclosed {
                    turn: t.id,
                    request: t.asks_action,
                    flagged: t.flagged,
                }));
            }
            out.push(Output::Send(ServiceMessage::Notice {
                code: "nothing_done".into(),
                turn: Some(t.id),
            }));
        }
        out.push(Output::Record(Record::TurnClosed { turn: t.id, outcome }));
        out.push(Output::Send(ServiceMessage::TurnClosed { turn: t.id, outcome }));
    }

    fn remember_unresolved(&mut self, call: CallId, turn: TurnId) {
        if self.unresolved.len() >= MAX_UNRESOLVED_TOOLS
            && let Some(oldest) = self.unresolved.keys().next().copied()
        {
            self.unresolved.remove(&oldest);
        }
        self.unresolved.insert(call, turn);
    }

    // ---- consent --------------------------------------------------------------------

    fn consent_change(
        &mut self,
        scope: ConsentScope,
        grant: bool,
        rights_review: Option<String>,
        out: &mut Vec<Output>,
    ) -> Result<(), Refused> {
        if grant && !self.consent_writable {
            return Err(Refused::ConsentLogUnavailable);
        }
        if scope == ConsentScope::PersonalVoice && grant {
            if !rights_review.as_deref().is_some_and(consent::is_rights_review_hash) {
                return Err(Refused::RightsReviewRequired);
            }
            if !PERSONAL_VOICE_AVAILABLE {
                return Err(Refused::PersonalVoiceUnavailable);
            }
        }
        let record = ConsentRecord {
            v: 1,
            scope,
            granted: grant,
            speaker: Speaker::Owner,
            rights_review: if grant { rights_review } else { None },
            deletion_scope: consent::deletion_scope(scope),
        };
        let revoking_capture = scope == ConsentScope::Capture && !grant;
        // A revoked personal voice must also silence what was already requested in it (V3).
        let revoking_voice = scope == ConsentScope::PersonalVoice && !grant && self.consent.personal_voice;
        let mut cancelled = None;
        let active = self.active_turn();
        if (revoking_capture || revoking_voice) && active.is_some() {
            // Stop first, record second: a crash between them leaves capture stopped.
            cancelled = Some(self.abort(UncertainReason::ConsentRevoked, out));
        }
        self.consent.apply(&record);
        out.push(Output::Consent(record));
        if let (Some(turn), Some(cancelled)) = (active, cancelled) {
            out.push(Output::Record(Record::Interrupted { turn, cancelled }));
            self.close(Outcome::ConsentRevoked, out);
        }
        out.push(Output::Send(ServiceMessage::ConsentState(self.consent)));
        Ok(())
    }

    fn consent_unavailable(&mut self, scope: ConsentScope, granted: bool, out: &mut Vec<Output>) {
        self.consent_writable = false;
        let active = self.active_turn();
        if granted {
            // The grant is not durable, so it does not stand: back to revoked.
            let undo = ConsentRecord {
                v: 1,
                scope,
                granted: false,
                speaker: Speaker::Owner,
                rights_review: None,
                deletion_scope: consent::deletion_scope(scope),
            };
            self.consent.apply(&undo);
            if scope == ConsentScope::Capture
                && let Some(turn) = active
            {
                let cancelled = self.abort(UncertainReason::ConsentRevoked, out);
                out.push(Output::Record(Record::Interrupted { turn, cancelled }));
                self.close(Outcome::ConsentRevoked, out);
            }
        }
        out.push(Output::Send(ServiceMessage::Degraded {
            turn: self.active_turn(),
            stage: Stage::Ledger,
            reason: FailureReason::LedgerIo,
        }));
        out.push(Output::Send(ServiceMessage::ConsentState(self.consent)));
    }

    // ---- recognizer -----------------------------------------------------------------

    fn stt_final(&mut self, call: CallId, text: String, min_confidence: Option<f32>, out: &mut Vec<Output>) {
        let lm = self.alloc_call();
        let (first_ms, total_ms, max_heard) = (
            self.limits.lm_first_ms,
            self.limits.lm_total_ms,
            self.limits.max_heard_chars,
        );
        let Some(t) = self.turn.as_mut() else {
            return late(call, "stt_final", out);
        };
        if t.phase != (Phase::Transcribing { stt: call }) {
            // GUARD(call-bound)
            return late(call, "stt_final", out);
        }
        out.push(Output::Disarm {
            timer: Timer {
                call,
                kind: TimerKind::Total,
            },
        });
        if text.chars().count() > max_heard {
            return self.fail(Stage::Stt, Some(call), FailureReason::TooLong, out);
        }
        let min_confidence = min_confidence.filter(|c| c.is_finite() && (0.0..=1.0).contains(c));
        out.push(Output::Record(Record::Heard {
            turn: t.id,
            call,
            text: text.clone(),
            min_confidence,
        }));
        if text.trim().is_empty() {
            t.heard = Some(Heard { text, min_confidence });
            return self.close(Outcome::Empty, out);
        }
        t.asks_action = guard::request_may_ask_for_action(&text);
        out.push(Output::Send(ServiceMessage::LmRequest {
            call: lm,
            turn: t.id,
            deadline_ms: total_ms,
            heard: text.clone(),
        }));
        out.push(arm(lm, TimerKind::First, first_ms));
        out.push(arm(lm, TimerKind::Total, total_ms));
        t.heard = Some(Heard { text, min_confidence });
        t.phase = Phase::Responding(Responding {
            lm: Some(Lm {
                call: lm,
                kind: None,
                sentences: 0,
                seen_output: false,
            }),
            chunks: BTreeMap::new(),
            next_seq: 0,
            outcome: Outcome::Completed,
        });
    }

    // ---- language model -------------------------------------------------------------

    /// The active model call if `call` is it; marks its first output seen.
    fn lm_mut(&mut self, call: CallId, out: &mut Vec<Output>) -> Option<&mut Lm> {
        let Some(Turn {
            phase: Phase::Responding(Responding { lm: Some(lm), .. }),
            ..
        }) = self.turn.as_mut()
        else {
            return None;
        };
        if lm.call != call {
            return None;
        }
        if !lm.seen_output {
            lm.seen_output = true;
            out.push(Output::Disarm {
                timer: Timer {
                    call,
                    kind: TimerKind::First,
                },
            });
        }
        Some(lm)
    }

    fn lm_kind(&mut self, call: CallId, kind: ProposalKind, out: &mut Vec<Output>) {
        let Some(lm) = self.lm_mut(call, out) else {
            return late(call, "lm_kind", out);
        };
        match lm.kind {
            None => lm.kind = Some(kind),
            Some(k) if k == kind => {}
            Some(_) => self.fail(Stage::Lm, Some(call), FailureReason::Protocol, out),
        }
    }

    fn lm_sentence(&mut self, call: CallId, index: u32, text: &str, out: &mut Vec<Output>) {
        let max_sentences = self.limits.max_sentences;
        let Some(lm) = self.lm_mut(call, out) else {
            return late(call, "lm_sentence", out);
        };
        if lm.kind != Some(ProposalKind::Reply) || index != lm.sentences || lm.sentences >= max_sentences {
            return late(call, "lm_sentence", out);
        }
        lm.sentences += 1;
        self.speak(text, Some(index), out);
    }

    /// Sends one chunk to synthesis for the active, responding turn. Model sentences go
    /// through the completion-claim guard (replaced, disclosed or spoken as written); `index`
    /// is `None` for session-authored text.
    fn speak(&mut self, text: &str, index: Option<u32>, out: &mut Vec<Output>) {
        let call = self.alloc_call();
        let voice = self.consent.voice();
        let (max_chars, first_ms, chunk_ms) = (
            self.limits.max_sentence_chars,
            self.limits.tts_first_audio_ms,
            self.limits.tts_chunk_ms,
        );
        let Some(t) = self.turn.as_mut() else { return };
        let Phase::Responding(r) = &mut t.phase else {
            return;
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        let mut spoken: String = trimmed.chars().take(max_chars).collect();
        let mut model_text = None;
        if let Some(index) = index
            && !t.executed
        {
            // A request that asked for no action cannot produce an evidenced claim, so on an
            // informational turn the recall-tilted uninvited-assertion screen also applies:
            // a possible claim is still spoken, but its turn ends disclosed (D-027).
            let verdict = guard::classify(&spoken).max(if !t.asks_action && guard::uninvited_action(&spoken) {
                // GUARD(claim-guard)
                guard::Verdict::Disclose
            } else {
                guard::Verdict::Plain
            });
            match verdict {
                guard::Verdict::Plain => {}
                guard::Verdict::Replace => {
                    t.flagged = true;
                    out.push(Output::Record(Record::ClaimSuppressed { turn: t.id, index }));
                    model_text = Some(std::mem::replace(&mut spoken, guard::DRAFT_DISCLAIMER.to_owned()));
                }
                guard::Verdict::Disclose => {
                    // Spoken as written; the turn ends with the disclosure.
                    t.flagged = true;
                    out.push(Output::Record(Record::ClaimDisclosed { turn: t.id, index }));
                }
            }
        }
        let seq = r.next_seq;
        r.next_seq += 1;
        r.chunks.insert(call, false);
        out.push(Output::Record(Record::Spoken {
            turn: t.id,
            call,
            seq,
            text: spoken.clone(),
            index,
            model_text,
        }));
        out.push(Output::Send(ServiceMessage::TtsRequest {
            call,
            turn: t.id,
            seq,
            text: spoken,
            voice,
            deadline_ms: first_ms,
        }));
        out.push(arm(call, TimerKind::First, first_ms));
        out.push(arm(call, TimerKind::Total, chunk_ms));
    }

    fn lm_done(&mut self, call: CallId, proposal: Proposal, model: &str, instructions: &str, out: &mut Vec<Output>) {
        let (max_name, max_reply) = (self.limits.max_name_chars, self.limits.max_reply_chars);
        let Some(lm) = self.lm_mut(call, out) else {
            return late(call, "lm_done", out);
        };
        let announced = lm.kind;
        let sentences = lm.sentences;
        out.push(Output::Disarm {
            timer: Timer {
                call,
                kind: TimerKind::Total,
            },
        });
        let reply_len = match &proposal {
            Proposal::Reply { reply } | Proposal::Action { reply, .. } => reply.chars().count(),
        };
        if reply_len > max_reply {
            return self.fail(Stage::Lm, Some(call), FailureReason::TooLong, out);
        }
        if announced.is_some_and(|k| k != proposal.kind()) {
            return self.fail(Stage::Lm, Some(call), FailureReason::Protocol, out);
        }
        let Some(turn) = self.active_turn() else {
            return;
        };
        out.push(Output::Record(Record::Inferred {
            turn,
            call,
            // Bounded for the record; the policy below still judges the proposal as received.
            proposal: bounded(&proposal, &self.limits),
            model: model.chars().take(max_name).collect(),
            instructions: instructions.chars().take(max_name).collect(),
        }));
        if let Some(Turn {
            phase: Phase::Responding(r),
            ..
        }) = self.turn.as_mut()
        {
            r.lm = None;
        }
        match proposal {
            Proposal::Reply { reply } => {
                if sentences == 0 {
                    // A provider that does not stream: speak the whole reply, sentence by sentence.
                    for (i, sentence) in split_sentences(&reply)
                        .iter()
                        .enumerate()
                        .take(self.limits.max_sentences as usize)
                    {
                        self.speak(sentence, Some(u32::try_from(i).unwrap_or(u32::MAX)), out);
                    }
                }
                self.disclose_turn(out);
                self.maybe_finish(out);
            }
            Proposal::Action { tool, slots, .. } => {
                let action = Action { tool, slots };
                let heard = self.turn.as_ref().and_then(|t| t.heard.clone()).unwrap_or(Heard {
                    text: String::new(),
                    min_confidence: None,
                });
                let decision = policy::evaluate(&action, &heard, &self.tools, &self.limits);
                out.push(Output::Record(Record::Decided {
                    turn,
                    decision: decision.clone(),
                }));
                match decision {
                    Decision::Refuse { refusal } => self.announce(refusal_text(refusal), Outcome::Refused, out),
                    Decision::Execute => self.start_tool(action, out),
                    Decision::Confirm { reasons } => {
                        let window = self.alloc_call();
                        let digest = action.digest(turn);
                        out.push(Output::Record(Record::ConfirmationRequested {
                            turn,
                            digest: digest.clone(),
                        }));
                        out.push(Output::Send(ServiceMessage::ConfirmRequest {
                            turn,
                            digest: digest.clone(),
                            heard: heard.text,
                            tool: action.tool.clone(),
                            slots: action.slots.clone(),
                            reasons,
                        }));
                        out.push(arm(window, TimerKind::Total, self.limits.confirm_ms));
                        if let Some(t) = self.turn.as_mut() {
                            t.phase = Phase::AwaitingConfirmation { window, action, digest };
                        }
                    }
                }
            }
        }
    }

    /// Fails closed for the whole turn (D-027). No tool can run in this release, and a sentence
    /// classifier cannot catch every way of saying something was done ("Is it booked? It is."),
    /// so a reply turn whose request may ask for an action, or with any sentence replaced or
    /// flagged, ends with one spoken "Nothing has been done.", recorded as `turn_disclosed`.
    fn disclose_turn(&mut self, out: &mut Vec<Output>) {
        let Some(t) = self.turn.as_mut() else { return };
        if t.executed || t.tool_sent || !(t.asks_action || t.flagged) {
            return;
        }
        t.disclosed = true;
        out.push(Output::Record(Record::TurnDisclosed {
            turn: t.id,
            request: t.asks_action,
            flagged: t.flagged,
        }));
        self.speak(guard::DRAFT_DISCLOSURE, None, out);
    }

    /// Speaks session-authored text, then closes the turn with `outcome`.
    fn announce(&mut self, text: &str, outcome: Outcome, out: &mut Vec<Output>) {
        if let Some(t) = self.turn.as_mut() {
            t.phase = Phase::Responding(Responding {
                lm: None,
                chunks: BTreeMap::new(),
                next_seq: 0,
                outcome,
            });
        }
        self.speak(text, None, out);
        self.maybe_finish(out);
    }

    fn maybe_finish(&mut self, out: &mut Vec<Output>) {
        let outcome = match self.turn.as_ref().map(|t| &t.phase) {
            Some(Phase::Responding(r)) if r.lm.is_none() && r.chunks.is_empty() => r.outcome,
            _ => return,
        };
        self.close(outcome, out);
    }

    // ---- confirmation and tools ---------------------------------------------------------

    fn confirm(&mut self, turn: TurnId, digest: &Digest, approve: bool, out: &mut Vec<Output>) -> Result<(), Refused> {
        let Some(t) = self.turn.as_ref() else {
            return Err(Refused::NotAwaitingConfirmation);
        };
        let Phase::AwaitingConfirmation {
            window,
            action,
            digest: pending,
        } = &t.phase
        else {
            return Err(Refused::NotAwaitingConfirmation);
        };
        if t.id != turn {
            // GUARD(confirm-turn)
            return Err(Refused::NotAwaitingConfirmation);
        }
        if pending != digest {
            // GUARD(digest-match)
            return Err(Refused::DigestMismatch);
        }
        let (window, action, digest) = (*window, action.clone(), pending.clone());
        disarm_all(window, out);
        out.push(Output::Record(Record::ConfirmationResolved {
            turn,
            digest,
            approved: approve,
        }));
        if approve {
            self.start_tool(action, out);
        } else {
            self.close(Outcome::Rejected, out); // GUARD(reject-closes)
        }
        Ok(())
    }

    fn start_tool(&mut self, action: Action, out: &mut Vec<Output>) {
        let call = self.alloc_call();
        let Some(t) = self.turn.as_mut() else { return };
        let digest = action.digest(t.id);
        out.push(Output::RecordDurable(Record::ToolCallSent {
            turn: t.id,
            call,
            tool: action.tool.clone(),
            digest,
        }));
        out.push(Output::Execute { call, action });
        out.push(arm(call, TimerKind::Total, self.limits.tool_ms));
        t.tool_sent = true;
        t.phase = Phase::Executing { tool: call }; // GUARD(phase-consumes)
    }

    fn tool_finished(&mut self, call: CallId, outcome: ToolOutcome, out: &mut Vec<Output>) {
        let outcome = bound_outcome(outcome, self.limits.max_evidence_chars);
        let current = match &self.turn {
            Some(t) if t.phase == (Phase::Executing { tool: call }) => Some(t.id),
            _ => None,
        };
        let Some(turn) = current else {
            if let Some(turn) = self.unresolved.remove(&call) {
                // The real result for a call already recorded as uncertain: keep it as evidence,
                // never speak it (the turn is over).
                out.push(Output::Record(Record::Executed {
                    turn,
                    call,
                    outcome,
                    reconciled: true,
                }));
                out.push(Output::Send(ServiceMessage::Notice {
                    code: "tool_reconciled".into(),
                    turn: Some(turn),
                }));
            } else {
                late(call, "tool_finished", out);
            }
            return;
        };
        disarm_all(call, out);
        out.push(Output::Record(Record::Executed {
            turn,
            call,
            outcome: outcome.clone(),
            reconciled: false,
        }));
        out.push(executed(turn, call, ExecutionStatus::of(&outcome)));
        let text = match &outcome {
            ToolOutcome::Succeeded { evidence } => {
                if let Some(t) = self.turn.as_mut() {
                    t.executed = true;
                }
                format!("Done. The tool returned: {evidence}")
            }
            ToolOutcome::Failed { .. } => FAILED_TEXT.to_owned(),
            ToolOutcome::Uncertain { .. } => UNCERTAIN_TEXT.to_owned(),
        };
        self.announce(&text, Outcome::Completed, out);
    }

    // ---- synthesis --------------------------------------------------------------------

    fn chunk_mut(&mut self, call: CallId) -> Option<&mut Responding> {
        match self.turn.as_mut().map(|t| &mut t.phase) {
            Some(Phase::Responding(r)) if r.chunks.contains_key(&call) => Some(r),
            _ => None,
        }
    }

    fn tts_started(&mut self, call: CallId, out: &mut Vec<Output>) {
        let Some(r) = self.chunk_mut(call) else {
            return late(call, "tts_started", out);
        };
        r.chunks.insert(call, true);
        out.push(Output::Disarm {
            timer: Timer {
                call,
                kind: TimerKind::First,
            },
        });
    }

    fn tts_finished(&mut self, call: CallId, out: &mut Vec<Output>) {
        let Some(r) = self.chunk_mut(call) else {
            return late(call, "tts_finished", out);
        };
        r.chunks.remove(&call);
        disarm_all(call, out);
        self.maybe_finish(out);
    }

    // ---- failures and deadlines ---------------------------------------------------------

    /// Whether `timer` still guards something: its call is pending and, for a `First` timer,
    /// the first output has not arrived yet.
    fn is_live(&self, timer: Timer) -> bool {
        let Some(t) = self.turn.as_ref() else {
            return false;
        };
        if !Self::pending_calls(&t.phase).iter().any(|(c, _)| *c == timer.call) {
            return false;
        }
        if timer.kind == TimerKind::Total {
            return true;
        }
        match &t.phase {
            Phase::Responding(r) => match &r.lm {
                Some(lm) if lm.call == timer.call => !lm.seen_output,
                _ => r.chunks.get(&timer.call).is_some_and(|started| !started),
            },
            _ => false,
        }
    }

    fn provider_failed(
        &mut self,
        call: CallId,
        stage: Stage,
        reason: FailureReason,
        name: &str,
        out: &mut Vec<Output>,
    ) {
        let expected = match self.turn.as_ref().map(|t| &t.phase) {
            Some(Phase::Transcribing { stt }) => stage == Stage::Stt && *stt == call,
            Some(Phase::Responding(r)) => match stage {
                Stage::Lm => r.lm.as_ref().is_some_and(|lm| lm.call == call),
                Stage::Tts => r.chunks.contains_key(&call),
                _ => false,
            },
            _ => false,
        };
        if !expected {
            return late(call, name, out);
        }
        self.fail(stage, Some(call), reason, out);
    }

    fn timer(&mut self, timer: Timer, out: &mut Vec<Output>) {
        if !self.is_live(timer) {
            return; // A timer that lost the race with its call's reply.
        }
        let Some(t) = self.turn.as_ref() else { return };
        let (turn, call) = (t.id, timer.call);
        match &t.phase {
            Phase::Capturing { .. } => {
                out.push(Output::Send(ServiceMessage::Notice {
                    code: "capture_limit".into(),
                    turn: Some(turn),
                }));
                self.end_capture(out);
            }
            Phase::Transcribing { .. } => self.fail(Stage::Stt, Some(call), FailureReason::Deadline, out),
            Phase::Responding(r) => {
                let stage = if r.lm.as_ref().is_some_and(|lm| lm.call == call) {
                    Stage::Lm
                } else {
                    Stage::Tts
                };
                self.fail(stage, Some(call), FailureReason::Deadline, out);
            }
            Phase::AwaitingConfirmation { digest, .. } => {
                let digest = digest.clone();
                out.push(Output::Record(Record::ConfirmationResolved {
                    turn,
                    digest,
                    approved: false,
                }));
                self.close(Outcome::Expired, out); // GUARD(expire-window)
            }
            Phase::Executing { .. } => {
                out.push(Output::Record(Record::Executed {
                    turn,
                    call,
                    outcome: ToolOutcome::Uncertain {
                        reason: UncertainReason::Deadline,
                    },
                    reconciled: false,
                }));
                out.push(executed(turn, call, ExecutionStatus::Uncertain));
                self.remember_unresolved(call, turn);
                out.push(Output::Send(ServiceMessage::Degraded {
                    turn: Some(turn),
                    stage: Stage::Tool,
                    reason: FailureReason::Deadline,
                }));
                self.announce(UNCERTAIN_TEXT, Outcome::Completed, out);
            }
        }
    }
}

const UNCERTAIN_TEXT: &str = "I don't know whether that finished. I've marked it uncertain.";
const FAILED_TEXT: &str = "That didn't work, so nothing changed.";

fn refusal_text(refusal: Refusal) -> &'static str {
    match refusal {
        Refusal::ExternalNotAuthorized => "I can't act outside this Mac in this version; nothing left this Mac.",
        Refusal::UnknownTool => "I can't do that here, so nothing happened.",
        Refusal::SlotMismatch | Refusal::TooManySlots | Refusal::SlotTooLong | Refusal::EmptySlot => {
            "I couldn't tell exactly what to do, so I did nothing."
        }
    }
}

fn executed(turn: TurnId, call: CallId, status: ExecutionStatus) -> Output {
    Output::Send(ServiceMessage::Executed { turn, call, status })
}

fn arm(call: CallId, kind: TimerKind, after_ms: u64) -> Output {
    Output::Arm {
        timer: Timer { call, kind },
        after_ms,
    }
}

fn disarm_all(call: CallId, out: &mut Vec<Output>) {
    out.push(Output::Disarm {
        timer: Timer {
            call,
            kind: TimerKind::First,
        },
    });
    out.push(Output::Disarm {
        timer: Timer {
            call,
            kind: TimerKind::Total,
        },
    });
}

fn late(call: CallId, message: &str, out: &mut Vec<Output>) {
    out.push(Output::Record(Record::LateResultDropped {
        // GUARD(late-drop)
        call,
        message: message.to_owned(),
    }));
}

/// A proposal cut to the limits, for the ledger: at most `max_slots` slots, each name and value
/// cut to its limit, the tool name cut to `max_name_chars`. Replies are already bounded.
fn bounded(p: &Proposal, limits: &Limits) -> Proposal {
    let cut = |s: &str, n: usize| s.chars().take(n).collect::<String>();
    match p {
        Proposal::Reply { reply } => Proposal::Reply { reply: reply.clone() },
        Proposal::Action { reply, tool, slots } => Proposal::Action {
            reply: reply.clone(),
            tool: cut(tool, limits.max_name_chars),
            slots: slots
                .iter()
                .take(limits.max_slots)
                .map(|s| Slot {
                    name: cut(&s.name, limits.max_name_chars),
                    kind: s.kind,
                    value: cut(&s.value, limits.max_slot_value_chars),
                })
                .collect(),
        },
    }
}

fn bound_outcome(outcome: ToolOutcome, max: usize) -> ToolOutcome {
    match outcome {
        ToolOutcome::Succeeded { evidence } => ToolOutcome::Succeeded {
            evidence: evidence.chars().take(max).collect(),
        },
        ToolOutcome::Failed { code } => ToolOutcome::Failed {
            code: code.chars().take(64).collect(),
        },
        u @ ToolOutcome::Uncertain { .. } => u,
    }
}

/// Splits text after `.`, `!` or `?` followed by whitespace. Used only when a provider did
/// not stream sentences itself.
pub fn split_sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        current.push(c);
        if matches!(c, '.' | '!' | '?') && chars.peek().is_none_or(|n| n.is_whitespace()) {
            let s = current.trim();
            if !s.is_empty() {
                out.push(s.to_owned());
            }
            current.clear();
        }
    }
    let s = current.trim();
    if !s.is_empty() {
        out.push(s.to_owned());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Session-authored speech other than the evidence-backed "Done." must never itself look
    /// like a completion claim (the randomized invariant test found the first disclaimer did).
    #[test]
    fn session_authored_text_passes_the_completion_guard() {
        let refusals = [
            Refusal::UnknownTool,
            Refusal::ExternalNotAuthorized,
            Refusal::SlotMismatch,
            Refusal::TooManySlots,
            Refusal::SlotTooLong,
            Refusal::EmptySlot,
        ];
        let texts =
            refusals
                .iter()
                .map(|r| refusal_text(*r))
                .chain([UNCERTAIN_TEXT, FAILED_TEXT, guard::DRAFT_DISCLAIMER]);
        for text in texts {
            assert!(!guard::claims_completion(text), "{text}");
        }
    }

    #[test]
    fn sentences_split_on_terminal_punctuation() {
        assert_eq!(
            split_sentences("Hi. It is 3.5 km! Ok? end"),
            ["Hi.", "It is 3.5 km!", "Ok?", "end"]
        );
        assert_eq!(split_sentences("   "), Vec::<String>::new());
    }
}
