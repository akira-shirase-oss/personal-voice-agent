//! The app ↔ service vocabulary (docs/PROTOCOL.md, v1). The domain's inputs and outputs
//! are these messages, so the protocol cannot drift from what the reducer understands.

use crate::consent::{ConsentScope, ConsentState, Voice};
use crate::ids::{CallId, Digest, TurnId};
use crate::policy::ConfirmReason;
use crate::proposal::{Proposal, ProposalKind, Slot};
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

/// App → service.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Hello {
        protocol: u32,
        client: String,
    },
    PttDown,
    PttUp {
        turn: TurnId,
    },
    Interrupt,
    SttFinal {
        call: CallId,
        text: String,
        min_confidence: Option<f32>,
    },
    LmKind {
        call: CallId,
        kind: ProposalKind,
    },
    LmSentence {
        call: CallId,
        index: u32,
        text: String,
    },
    LmDone {
        call: CallId,
        proposal: Proposal,
        model: String,
        instructions: String,
    },
    TtsStarted {
        call: CallId,
    },
    TtsFinished {
        call: CallId,
    },
    SttFailed {
        call: CallId,
        reason: FailureReason,
    },
    LmFailed {
        call: CallId,
        reason: FailureReason,
    },
    TtsFailed {
        call: CallId,
        reason: FailureReason,
    },
    Confirm {
        turn: TurnId,
        digest: Digest,
        approve: bool,
    },
    Consent {
        scope: ConsentScope,
        grant: bool,
        rights_review: Option<String>,
    },
    Flush,
    /// Liveness: answered by `pong` from the service shell, without touching the ledger.
    Ping {
        seq: u64,
    },
    /// Carry out a consent scope's deletion scope now (D-018): for `capture`, delete every
    /// heard transcript. Refused while a turn is open.
    Forget {
        scope: ConsentScope,
    },
    Shutdown,
}

impl ClientMessage {
    /// The message's wire name, for logs and dropped-result records (never its content).
    pub fn name(&self) -> &'static str {
        match self {
            Self::Hello { .. } => "hello",
            Self::PttDown => "ptt_down",
            Self::PttUp { .. } => "ptt_up",
            Self::Interrupt => "interrupt",
            Self::SttFinal { .. } => "stt_final",
            Self::LmKind { .. } => "lm_kind",
            Self::LmSentence { .. } => "lm_sentence",
            Self::LmDone { .. } => "lm_done",
            Self::TtsStarted { .. } => "tts_started",
            Self::TtsFinished { .. } => "tts_finished",
            Self::SttFailed { .. } => "stt_failed",
            Self::LmFailed { .. } => "lm_failed",
            Self::TtsFailed { .. } => "tts_failed",
            Self::Confirm { .. } => "confirm",
            Self::Consent { .. } => "consent",
            Self::Flush => "flush",
            Self::Ping { .. } => "ping",
            Self::Forget { .. } => "forget",
            Self::Shutdown => "shutdown",
        }
    }
}

/// Service → app.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServiceMessage {
    Ready {
        protocol: u32,
        session: String,
        consent: ConsentState,
        recovered: Recovered,
    },
    TurnOpened {
        turn: TurnId,
    },
    Capture {
        turn: TurnId,
        on: bool,
    },
    SttRequest {
        call: CallId,
        turn: TurnId,
        deadline_ms: u64,
    },
    LmRequest {
        call: CallId,
        turn: TurnId,
        deadline_ms: u64,
        heard: String,
    },
    TtsRequest {
        call: CallId,
        turn: TurnId,
        seq: u32,
        text: String,
        voice: Voice,
        deadline_ms: u64,
    },
    Cancel {
        call: CallId,
    },
    StopPlayback {
        fence: TurnId,
    },
    ConfirmRequest {
        turn: TurnId,
        digest: Digest,
        heard: String,
        tool: String,
        slots: Vec<Slot>,
        reasons: Vec<ConfirmReason>,
    },
    TurnClosed {
        turn: TurnId,
        outcome: Outcome,
    },
    Degraded {
        turn: Option<TurnId>,
        stage: Stage,
        reason: FailureReason,
    },
    ConsentState(ConsentState),
    Notice {
        code: String,
        turn: Option<TurnId>,
    },
    Flushed {
        seq: u64,
    },
    Pong {
        seq: u64,
    },
    /// A tool call of this turn has a result on record (the ledger's `executed`): what the app
    /// shows as executed, instead of assuming nothing ran.
    Executed {
        turn: TurnId,
        call: CallId,
        status: ExecutionStatus,
    },
    /// A `forget` was carried out: how many files and bytes were deleted.
    Forgotten {
        scope: ConsentScope,
        files: u32,
        bytes: u64,
    },
    Error {
        code: String,
        detail: String,
    },
}

/// How a tool call ended, as the app is told: never more than the ledger's record says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Succeeded,
    Failed,
    Uncertain,
}

impl ExecutionStatus {
    pub fn of(outcome: &crate::tools::ToolOutcome) -> Self {
        match outcome {
            crate::tools::ToolOutcome::Succeeded { .. } => Self::Succeeded,
            crate::tools::ToolOutcome::Failed { .. } => Self::Failed,
            crate::tools::ToolOutcome::Uncertain { .. } => Self::Uncertain,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recovered {
    pub sessions: u32,
    pub abandoned_turns: u32,
    pub uncertain_calls: u32,
    pub torn_tails: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Completed,
    Interrupted,
    Degraded,
    Refused,
    Rejected,
    Expired,
    ConsentRevoked,
    Empty,
    Abandoned,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Stt,
    Lm,
    Tts,
    Tool,
    Ledger,
}

/// Why a provider, tool or the ledger failed. Unknown codes from the app become `error`,
/// so no free text from a provider can reach the ledger through this field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    Deadline,
    Unavailable,
    ModelNotReady,
    AssetsMissing,
    ContextWindow,
    Guardrail,
    AudioFormat,
    Cancelled,
    TooLong,
    Protocol,
    LedgerFull,
    LedgerIo,
    #[serde(other)]
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_failure_reasons_collapse_to_error() {
        let m: ClientMessage =
            serde_json::from_str(r#"{"type":"lm_failed","call":3,"reason":"my secret prompt"}"#).unwrap();
        assert_eq!(
            m,
            ClientMessage::LmFailed {
                call: CallId(3),
                reason: FailureReason::Error
            }
        );
    }

    #[test]
    fn missing_confidence_is_none() {
        let m: ClientMessage = serde_json::from_str(r#"{"type":"stt_final","call":1,"text":"hi"}"#).unwrap();
        assert_eq!(
            m,
            ClientMessage::SttFinal {
                call: CallId(1),
                text: "hi".into(),
                min_confidence: None
            }
        );
    }
}
