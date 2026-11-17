//! Consent records (D-017) and the state folded from them. The log is append-only; the
//! current state is always the fold of every record, so a revocation can never be lost by
//! overwriting.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsentScope {
    /// Capture audio while push-to-talk is held. Separate from the OS microphone permission.
    Capture,
    /// Speak in an enrolled personal voice. Closed in the first release (D-012, D-020).
    PersonalVoice,
}

/// Data derived under a consent, which that consent's deletion scope covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Artifact {
    /// Heard text in the turn ledger. No audio is ever retained in the first release.
    LedgerTranscripts,
    VoiceProfile,
    SynthesizedPersonalAudio,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Speaker {
    /// The Mac's owner. No other speaker can be enrolled in the first release.
    Owner,
}

/// One consent-log entry, format version 1.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentRecord {
    pub v: u32,
    pub scope: ConsentScope,
    pub granted: bool,
    pub speaker: Speaker,
    /// SHA-256 (hex) of the usage-rights review artifact; required for personal voice.
    pub rights_review: Option<String>,
    pub deletion_scope: Vec<Artifact>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentState {
    pub capture: bool,
    pub personal_voice: bool,
}

/// The only voice the first release can speak in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Voice {
    Standard,
}

/// Whether this build can use a personal voice at all. False for the first release: the
/// gate exists and is tested, but nothing can open it before V3's rights review.
pub const PERSONAL_VOICE_AVAILABLE: bool = false;

impl ConsentState {
    pub fn fold<'a>(records: impl IntoIterator<Item = &'a ConsentRecord>) -> Self {
        let mut state = Self::default();
        for record in records {
            state.apply(record);
        }
        state
    }

    /// A personal-voice grant counts only in a build that has personal voice, and only with a
    /// valid usage-rights review reference (D-020): a grant written into the log by hand or by
    /// a later build cannot open the gate here.
    pub fn apply(&mut self, record: &ConsentRecord) {
        match record.scope {
            ConsentScope::Capture => self.capture = record.granted,
            ConsentScope::PersonalVoice => {
                self.personal_voice = record.granted
                    && PERSONAL_VOICE_AVAILABLE
                    && record.rights_review.as_deref().is_some_and(is_rights_review_hash);
            }
        }
    }

    pub fn voice(self) -> Voice {
        Voice::Standard
    }
}

pub(crate) fn deletion_scope(scope: ConsentScope) -> Vec<Artifact> {
    match scope {
        ConsentScope::Capture => vec![Artifact::LedgerTranscripts],
        ConsentScope::PersonalVoice => {
            vec![Artifact::VoiceProfile, Artifact::SynthesizedPersonalAudio]
        }
    }
}

/// A rights-review reference is the artifact's SHA-256 as 64 lowercase hex digits.
pub(crate) fn is_rights_review_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(scope: ConsentScope, granted: bool) -> ConsentRecord {
        ConsentRecord {
            v: 1,
            scope,
            granted,
            speaker: Speaker::Owner,
            rights_review: None,
            deletion_scope: deletion_scope(scope),
        }
    }

    #[test]
    fn the_last_record_for_a_scope_wins() {
        let log = [rec(ConsentScope::Capture, true), rec(ConsentScope::Capture, false)];
        assert_eq!(
            ConsentState::fold(&log),
            ConsentState {
                capture: false,
                personal_voice: false
            }
        );
        let log = [rec(ConsentScope::Capture, false), rec(ConsentScope::Capture, true)];
        assert!(ConsentState::fold(&log).capture);
    }

    #[test]
    fn the_first_release_only_speaks_the_standard_voice() {
        let s = ConsentState {
            capture: true,
            personal_voice: true,
        };
        assert_eq!(s.voice(), Voice::Standard);
    }

    /// V3 hazard (second review, LOW20): a personal-voice grant in the log must not fold to
    /// granted in a build without personal voice, even with a valid review reference.
    #[test]
    fn a_logged_personal_voice_grant_does_not_open_the_gate_in_this_release() {
        let grant = ConsentRecord {
            rights_review: Some("a".repeat(64)),
            ..rec(ConsentScope::PersonalVoice, true)
        };
        assert!(!ConsentState::fold([&grant]).personal_voice);
        let unreviewed = rec(ConsentScope::PersonalVoice, true);
        assert!(!ConsentState::fold([&unreviewed]).personal_voice);
    }

    #[test]
    fn rights_review_reference_is_a_sha256_hex() {
        assert!(is_rights_review_hash(&"a".repeat(64)));
        assert!(!is_rights_review_hash(&"A".repeat(64)));
        assert!(!is_rights_review_hash("abc"));
    }
}
