//! Turn-ledger records (D-008, D-017) and the pure recovery fold that closes a session
//! interrupted by a crash.

use crate::ids::{CallId, Digest, TurnId};
use crate::policy::Decision;
use crate::proposal::Proposal;
use crate::protocol::{FailureReason, Outcome, Stage};
use crate::tools::{ToolOutcome, UncertainReason};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Record {
    SessionOpened {
        protocol: u32,
        client: String,
    },
    TurnOpened {
        turn: TurnId,
    },
    /// What the recognizer heard, verbatim.
    Heard {
        turn: TurnId,
        call: CallId,
        text: String,
        min_confidence: Option<f32>,
    },
    /// What the model proposed: inferred, never evidence of anything done.
    Inferred {
        turn: TurnId,
        call: CallId,
        proposal: Proposal,
        model: String,
        instructions: String,
    },
    /// A reply sentence claiming completion was replaced by the draft disclaimer.
    ClaimSuppressed {
        turn: TurnId,
        index: u32,
    },
    Decided {
        turn: TurnId,
        decision: Decision,
    },
    ConfirmationRequested {
        turn: TurnId,
        digest: Digest,
    },
    ConfirmationResolved {
        turn: TurnId,
        digest: Digest,
        approved: bool,
    },
    /// Written durably before the tool runs, so a crash leaves an honest `uncertain`.
    ToolCallSent {
        turn: TurnId,
        call: CallId,
        tool: String,
        digest: Digest,
    },
    /// What executed, with its evidence. `reconciled` marks a result that arrived after the
    /// turn had already recorded the call as uncertain.
    Executed {
        turn: TurnId,
        call: CallId,
        outcome: ToolOutcome,
        reconciled: bool,
    },
    ProviderFailed {
        turn: TurnId,
        call: CallId,
        stage: Stage,
        reason: FailureReason,
    },
    /// A reply for a call that was cancelled, timed out or never issued. No content kept.
    LateResultDropped {
        call: CallId,
        message: String,
    },
    Interrupted {
        turn: TurnId,
        cancelled: Vec<CallId>,
    },
    TurnClosed {
        turn: TurnId,
        outcome: Outcome,
    },
    TurnAbandoned {
        turn: TurnId,
    },
    SessionClosed {
        reason: CloseReason,
    },
    RecoveredTornTail {
        bytes: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseReason {
    /// The app said shutdown or closed stdin.
    Clean,
    /// The service exited without closing; closed by the next start's recovery.
    Crashed,
}

/// What recovery found in one unclosed session.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recovery {
    pub records: Vec<Record>,
    pub abandoned_turns: u32,
    pub uncertain_calls: u32,
}

/// Streaming form of [`recover`]: observe each record of one session's log in order, then
/// `finish`. Memory is bounded by the open turns and unresolved tool calls, not the log size.
#[derive(Clone, Debug, Default)]
pub struct RecoveryFold {
    closed: bool,
    open_turns: BTreeSet<TurnId>,
    sent: BTreeMap<CallId, TurnId>,
}

impl RecoveryFold {
    pub fn observe(&mut self, record: &Record) {
        match record {
            Record::SessionClosed { .. } => self.closed = true,
            Record::TurnOpened { turn } => {
                self.open_turns.insert(*turn);
            }
            Record::TurnClosed { turn, .. } | Record::TurnAbandoned { turn } => {
                self.open_turns.remove(turn);
            }
            Record::ToolCallSent { turn, call, .. } => {
                self.sent.insert(*call, *turn);
            }
            Record::Executed { call, .. } => {
                self.sent.remove(call);
            }
            _ => {}
        }
    }

    /// Records that close the session: an `uncertain` result for every tool call sent without
    /// one, `turn_abandoned` for every open turn, then `session_closed` (crashed). Empty when
    /// the session was already closed.
    pub fn finish(self) -> Recovery {
        if self.closed {
            return Recovery::default();
        }
        let mut out = Recovery::default();
        for (call, turn) in self.sent {
            out.records.push(Record::Executed {
                turn,
                call,
                outcome: ToolOutcome::Uncertain {
                    reason: UncertainReason::ServiceRestart,
                },
                reconciled: false,
            });
            out.uncertain_calls += 1;
        }
        for turn in self.open_turns {
            out.records.push(Record::TurnAbandoned { turn });
            out.abandoned_turns += 1;
        }
        out.records.push(Record::SessionClosed {
            reason: CloseReason::Crashed,
        });
        out
    }
}

/// Records that close a session whose log has no `session_closed` (see [`RecoveryFold`]).
pub fn recover<'a>(records: impl IntoIterator<Item = &'a Record>) -> Recovery {
    let mut fold = RecoveryFold::default();
    for record in records {
        fold.observe(record);
    }
    fold.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_closed_session_needs_nothing() {
        let log = [
            Record::TurnOpened { turn: TurnId(1) },
            Record::SessionClosed {
                reason: CloseReason::Clean,
            },
        ];
        assert_eq!(recover(&log), Recovery::default());
    }

    #[test]
    fn open_turns_are_abandoned_and_sent_tools_become_uncertain() {
        let d = Digest("0".repeat(16));
        let log = [
            Record::TurnOpened { turn: TurnId(1) },
            Record::TurnClosed {
                turn: TurnId(1),
                outcome: Outcome::Completed,
            },
            Record::TurnOpened { turn: TurnId(2) },
            Record::ToolCallSent {
                turn: TurnId(2),
                call: CallId(9),
                tool: "t".into(),
                digest: d.clone(),
            },
            Record::ToolCallSent {
                turn: TurnId(2),
                call: CallId(7),
                tool: "t".into(),
                digest: d,
            },
            Record::Executed {
                turn: TurnId(2),
                call: CallId(7),
                outcome: ToolOutcome::Succeeded { evidence: "e".into() },
                reconciled: false,
            },
        ];
        let r = recover(&log);
        assert_eq!(r.abandoned_turns, 1);
        assert_eq!(r.uncertain_calls, 1);
        assert_eq!(
            r.records,
            vec![
                Record::Executed {
                    turn: TurnId(2),
                    call: CallId(9),
                    outcome: ToolOutcome::Uncertain {
                        reason: UncertainReason::ServiceRestart
                    },
                    reconciled: false,
                },
                Record::TurnAbandoned { turn: TurnId(2) },
                Record::SessionClosed {
                    reason: CloseReason::Crashed
                },
            ]
        );
    }
}
