//! Every bound the session enforces, in one place. Deadline values are D-019's initial
//! numbers; the V2 latency run re-evaluates them.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Limits {
    /// Recognizer finalize after push-to-talk release.
    pub stt_finalize_ms: u64,
    /// Language model: first streamed output (`lm_kind`, a sentence or `lm_done`).
    pub lm_first_ms: u64,
    /// Language model: complete proposal.
    pub lm_total_ms: u64,
    /// Synthesis: first buffer of a chunk scheduled on the player.
    pub tts_first_audio_ms: u64,
    /// Synthesis: a chunk finishes playing.
    pub tts_chunk_ms: u64,
    /// A tool call returns a result.
    pub tool_ms: u64,
    /// The user answers a confirmation.
    pub confirm_ms: u64,
    /// Push-to-talk held at most this long; capture then stops on its own.
    pub capture_max_ms: u64,
    /// Lowest per-segment recognizer confidence that is not ambiguous (D-013).
    pub min_confidence: f32,
    /// Bounds on content the session accepts and stores (characters).
    pub max_heard_chars: usize,
    pub max_reply_chars: usize,
    pub max_sentence_chars: usize,
    pub max_sentences: u32,
    pub max_slots: usize,
    pub max_slot_value_chars: usize,
    pub max_name_chars: usize,
    pub max_evidence_chars: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            stt_finalize_ms: 3_000,
            lm_first_ms: 6_000,
            lm_total_ms: 20_000,
            tts_first_audio_ms: 3_000,
            tts_chunk_ms: 30_000,
            tool_ms: 5_000,
            confirm_ms: 60_000,
            capture_max_ms: 60_000,
            min_confidence: 0.80,
            max_heard_chars: 4_000,
            max_reply_chars: 4_000,
            max_sentence_chars: 1_000,
            max_sentences: 64,
            max_slots: 8,
            max_slot_value_chars: 200,
            max_name_chars: 64,
            max_evidence_chars: 500,
        }
    }
}
