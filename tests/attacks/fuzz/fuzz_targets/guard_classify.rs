//! The completion-claim guard under arbitrary text: sentence splitting, clause
//! splitting, subject attribution and `classify`, the `uninvited_action` screen,
//! the `second_pass` seam, and the `request_may_ask_for_action` router — the
//! never-panics minimum for code that sits on every spoken sentence. The guard
//! answers `Verdict` for anything; a panic here is a red-first bug (the first
//! one found was an inherited-subject index out of bounds on a fragment clause).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = voice_domain::guard::classify(&text);
    let _ = voice_domain::guard::uninvited_action(&text);
    let _ = voice_domain::guard::request_may_ask_for_action(&text);
    let _ = voice_domain::guard::second_pass(&text, voice_domain::guard::claims_completion);
});
