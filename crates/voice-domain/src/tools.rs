//! Tools the session may run, and what a run returned. The first release registers no
//! tools in the app; tests and load runs register fixture tools.

use crate::proposal::SlotKind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolEffect {
    ReadOnly,
    LocalReversible,
    LocalIrreversible,
    External,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolSpec {
    pub name: String,
    pub effect: ToolEffect,
    /// The slots the tool takes, by name. A proposal must supply exactly these.
    pub slots: Vec<(String, SlotKind)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolRegistry {
    tools: BTreeMap<String, ToolSpec>,
}

impl ToolRegistry {
    pub fn new(specs: impl IntoIterator<Item = ToolSpec>) -> Self {
        Self {
            tools: specs.into_iter().map(|s| (s.name.clone(), s)).collect(),
        }
    }

    pub fn get(&self, name: &str) -> Option<&ToolSpec> {
        self.tools.get(name)
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

/// What a tool call produced. `Succeeded` must carry evidence: the only path to a spoken
/// "done" (invariant 2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ToolOutcome {
    Succeeded {
        evidence: String,
    },
    Failed {
        code: String,
    },
    /// The call was sent but no result is known: interrupted, timed out, or the service
    /// restarted. Never reported as done or undone.
    Uncertain {
        reason: UncertainReason,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UncertainReason {
    Deadline,
    Interrupted,
    ConsentRevoked,
    ServiceRestart,
}
