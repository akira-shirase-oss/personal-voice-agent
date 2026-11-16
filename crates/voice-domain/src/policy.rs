//! D-013 as code: what happens to an action proposal. The policy reads the proposal, the
//! heard transcript and the registry — never the model's reply text — so nothing a model,
//! transcript or tool result says can change the decision.

use crate::limits::Limits;
use crate::normalize;
use crate::proposal::Action;
use crate::tools::{ToolEffect, ToolRegistry};
use serde::{Deserialize, Serialize};

/// The recognizer's output for a turn, verbatim.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Heard {
    pub text: String,
    /// Lowest per-segment confidence, or `None` when the recognizer gave none.
    pub min_confidence: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    UnknownTool,
    /// External actions need an authorization that cannot be granted before V4.
    ExternalNotAuthorized,
    SlotMismatch,
    TooManySlots,
    SlotTooLong,
    EmptySlot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmReason {
    IrreversibleEffect,
    SensitiveSlot,
    LowConfidence,
    NoConfidence,
    SlotNotHeard,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    Refuse { refusal: Refusal },
    Confirm { reasons: Vec<ConfirmReason> },
    Execute,
}

pub fn evaluate(action: &Action, heard: &Heard, registry: &ToolRegistry, limits: &Limits) -> Decision {
    let Some(spec) = registry.get(&action.tool) else {
        return Decision::Refuse {
            refusal: Refusal::UnknownTool,
        };
    };
    if spec.effect == ToolEffect::External {
        return Decision::Refuse {
            refusal: Refusal::ExternalNotAuthorized,
        };
    }
    if action.slots.len() > limits.max_slots {
        return Decision::Refuse {
            refusal: Refusal::TooManySlots,
        };
    }
    if action.slots.len() != spec.slots.len()
        || !spec.slots.iter().all(|(name, kind)| {
            action
                .slots
                .iter()
                .filter(|s| &s.name == name && s.kind == *kind)
                .count()
                == 1
        })
    {
        return Decision::Refuse {
            refusal: Refusal::SlotMismatch,
        };
    }
    if action.slots.iter().any(|s| s.value.trim().is_empty()) {
        return Decision::Refuse {
            refusal: Refusal::EmptySlot,
        };
    }
    if action
        .slots
        .iter()
        .any(|s| s.value.chars().count() > limits.max_slot_value_chars)
    {
        return Decision::Refuse {
            refusal: Refusal::SlotTooLong,
        };
    }

    let mut reasons = Vec::new();
    if spec.effect == ToolEffect::LocalIrreversible {
        reasons.push(ConfirmReason::IrreversibleEffect);
    }
    if action.slots.iter().any(|s| s.kind.is_sensitive()) {
        reasons.push(ConfirmReason::SensitiveSlot);
    }
    match heard.min_confidence {
        None => reasons.push(ConfirmReason::NoConfidence),
        // A NaN never compares as at-or-above the threshold, so it counts as low.
        Some(c)
            if c.partial_cmp(&limits.min_confidence)
                .is_none_or(std::cmp::Ordering::is_lt) =>
        {
            reasons.push(ConfirmReason::LowConfidence);
        }
        Some(_) => {}
    }
    let heard_tokens = normalize::tokens(&heard.text);
    if action
        .slots
        .iter()
        .any(|s| !normalize::contains_literal(&heard_tokens, &s.value))
    {
        reasons.push(ConfirmReason::SlotNotHeard);
    }
    if reasons.is_empty() {
        Decision::Execute
    } else {
        Decision::Confirm { reasons }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proposal::{Slot, SlotKind};
    use crate::tools::ToolSpec;

    fn registry() -> ToolRegistry {
        ToolRegistry::new([
            ToolSpec {
                name: "note".into(),
                effect: ToolEffect::LocalReversible,
                slots: vec![("text".into(), SlotKind::Text)],
            },
            ToolSpec {
                name: "wipe".into(),
                effect: ToolEffect::LocalIrreversible,
                slots: vec![("text".into(), SlotKind::Text)],
            },
            ToolSpec {
                name: "pay".into(),
                effect: ToolEffect::External,
                slots: vec![("amount".into(), SlotKind::Amount), ("to".into(), SlotKind::Recipient)],
            },
            ToolSpec {
                name: "remind".into(),
                effect: ToolEffect::LocalReversible,
                slots: vec![("when".into(), SlotKind::Datetime)],
            },
        ])
    }

    fn heard(text: &str, c: Option<f32>) -> Heard {
        Heard {
            text: text.into(),
            min_confidence: c,
        }
    }

    fn act(tool: &str, slots: &[(&str, SlotKind, &str)]) -> Action {
        Action {
            tool: tool.into(),
            slots: slots
                .iter()
                .map(|(n, k, v)| Slot {
                    name: (*n).into(),
                    kind: *k,
                    value: (*v).into(),
                })
                .collect(),
        }
    }

    fn decide(a: &Action, h: &Heard) -> Decision {
        evaluate(a, h, &registry(), &Limits::default())
    }

    #[test]
    fn plain_reversible_action_heard_clearly_executes() {
        let d = decide(
            &act("note", &[("text", SlotKind::Text, "milk")]),
            &heard("note milk", Some(0.95)),
        );
        assert_eq!(d, Decision::Execute);
    }

    #[test]
    fn misheard_amount_goes_to_confirmation() {
        // Recognizer heard 40; the model "corrected" it to 42.
        let d = decide(
            &act("remind", &[("when", SlotKind::Datetime, "42")]),
            &heard("remind me at 40", Some(0.99)),
        );
        assert_eq!(
            d,
            Decision::Confirm {
                reasons: vec![ConfirmReason::SensitiveSlot, ConfirmReason::SlotNotHeard]
            }
        );
    }

    #[test]
    fn low_or_missing_confidence_is_ambiguous() {
        let a = act("note", &[("text", SlotKind::Text, "milk")]);
        assert_eq!(
            decide(&a, &heard("note milk", Some(0.5))),
            Decision::Confirm {
                reasons: vec![ConfirmReason::LowConfidence]
            }
        );
        assert_eq!(
            decide(&a, &heard("note milk", None)),
            Decision::Confirm {
                reasons: vec![ConfirmReason::NoConfidence]
            }
        );
        assert_eq!(
            decide(&a, &heard("note milk", Some(f32::NAN))),
            Decision::Confirm {
                reasons: vec![ConfirmReason::LowConfidence]
            },
            "a NaN confidence must not pass a >= comparison by accident"
        );
    }

    #[test]
    fn irreversible_effect_confirms_even_when_heard_clearly() {
        let d = decide(
            &act("wipe", &[("text", SlotKind::Text, "milk")]),
            &heard("wipe milk", Some(0.99)),
        );
        assert_eq!(
            d,
            Decision::Confirm {
                reasons: vec![ConfirmReason::IrreversibleEffect]
            }
        );
    }

    #[test]
    fn external_effects_are_refused_before_any_confirmation() {
        let a = act(
            "pay",
            &[("amount", SlotKind::Amount, "42"), ("to", SlotKind::Recipient, "Priya")],
        );
        let d = decide(&a, &heard("pay $42 to Priya", Some(0.99)));
        assert_eq!(
            d,
            Decision::Refuse {
                refusal: Refusal::ExternalNotAuthorized
            }
        );
    }

    #[test]
    fn unknown_tools_and_malformed_slots_are_refused() {
        let h = heard("note milk", Some(0.99));
        assert_eq!(
            decide(&act("grant_permission", &[]), &h),
            Decision::Refuse {
                refusal: Refusal::UnknownTool
            }
        );
        assert_eq!(
            decide(&act("note", &[]), &h),
            Decision::Refuse {
                refusal: Refusal::SlotMismatch
            }
        );
        assert_eq!(
            decide(&act("note", &[("text", SlotKind::Amount, "milk")]), &h),
            Decision::Refuse {
                refusal: Refusal::SlotMismatch
            },
            "the slot kind is part of the contract"
        );
        assert_eq!(
            decide(
                &act(
                    "note",
                    &[("text", SlotKind::Text, "milk"), ("text", SlotKind::Text, "eggs")]
                ),
                &h
            ),
            Decision::Refuse {
                refusal: Refusal::SlotMismatch
            }
        );
        assert_eq!(
            decide(&act("note", &[("text", SlotKind::Text, "  ")]), &h),
            Decision::Refuse {
                refusal: Refusal::EmptySlot
            }
        );
    }
}
