//! What the language model proposes. A proposal is *inferred* data: it is recorded,
//! checked by the policy, and never trusted to decide anything itself.

use crate::ids::{Digest, TurnId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalKind {
    Reply,
    Action,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotKind {
    Amount,
    Datetime,
    Person,
    Recipient,
    Text,
}

impl SlotKind {
    /// Slots whose mistakes cost money, time or the wrong person (D-013).
    pub fn is_sensitive(self) -> bool {
        matches!(self, Self::Amount | Self::Datetime | Self::Person | Self::Recipient)
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Amount => "amount",
            Self::Datetime => "datetime",
            Self::Person => "person",
            Self::Recipient => "recipient",
            Self::Text => "text",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slot {
    pub name: String,
    pub kind: SlotKind,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub tool: String,
    pub slots: Vec<Slot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Proposal {
    Reply {
        reply: String,
    },
    Action {
        reply: String,
        tool: String,
        slots: Vec<Slot>,
    },
}

impl Proposal {
    pub fn kind(&self) -> ProposalKind {
        match self {
            Self::Reply { .. } => ProposalKind::Reply,
            Self::Action { .. } => ProposalKind::Action,
        }
    }
}

impl Action {
    /// The digest a confirmation must carry for this action in this turn.
    pub fn digest(&self, turn: TurnId) -> Digest {
        let turn_bytes = turn.0.to_le_bytes();
        let mut fields: Vec<&[u8]> = vec![&turn_bytes, self.tool.as_bytes()];
        for slot in &self.slots {
            fields.push(slot.name.as_bytes());
            fields.push(slot.kind.as_str().as_bytes());
            fields.push(slot.value.as_bytes());
        }
        Digest::of_fields(fields)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(value: &str) -> Action {
        Action {
            tool: "fixture.transfer".into(),
            slots: vec![Slot {
                name: "amount".into(),
                kind: SlotKind::Amount,
                value: value.into(),
            }],
        }
    }

    #[test]
    fn digest_changes_with_turn_and_any_slot_value() {
        let a = action("42");
        assert_eq!(a.digest(TurnId(1)), action("42").digest(TurnId(1)));
        assert_ne!(
            a.digest(TurnId(1)),
            a.digest(TurnId(2)),
            "a confirmation cannot carry over to another turn"
        );
        assert_ne!(a.digest(TurnId(1)), action("420").digest(TurnId(1)));
    }

    #[test]
    fn proposal_wire_form() {
        let p: Proposal = serde_json::from_str(r#"{"kind":"reply","reply":"hi"}"#).unwrap();
        assert_eq!(p, Proposal::Reply { reply: "hi".into() });
        assert!(serde_json::from_str::<Proposal>(r#"{"kind":"command","reply":"x"}"#).is_err());
    }
}
