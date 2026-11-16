//! The Personal Voice Agent's pure model. No I/O, no clock, no runtime: the session is a
//! reducer from inputs to outputs, so every behaviour is replayable and testable.
//!
//! - [`session`]: the turn state machine.
//! - [`policy`]: D-013's consequential/ambiguous rule for action proposals.
//! - [`guard`]: the completion-claim guard for spoken drafts.
//! - [`consent`]: consent records and the state folded from them.
//! - [`record`]: turn-ledger records and crash recovery.
//! - [`protocol`]: the app ↔ service message vocabulary.

pub mod consent;
pub mod guard;
pub mod ids;
pub mod limits;
pub mod normalize;
pub mod policy;
pub mod proposal;
pub mod protocol;
pub mod record;
pub mod session;
pub mod tools;

pub use ids::{CallId, Digest, TurnId};
pub use limits::Limits;
pub use session::{Input, Output, Refused, Session, Timer, TimerKind};
