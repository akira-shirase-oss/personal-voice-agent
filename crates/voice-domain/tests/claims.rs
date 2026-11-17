//! The completion-claim rule against a phrasing corpus (`fixtures/claims/corpus.tsv`): every
//! "claim" sentence must be caught, every "ok" sentence must pass. All misses are reported.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code: a panic is the failure report"
)]

use voice_domain::guard::claims_completion;

#[test]
fn the_corpus_is_classified_exactly() {
    let path = format!("{}/../../fixtures/claims/corpus.tsv", env!("CARGO_MANIFEST_DIR"));
    let corpus = std::fs::read_to_string(path).unwrap();
    let mut missed = Vec::new();
    let mut wrongly_caught = Vec::new();
    let mut n = 0;
    for line in corpus.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let (label, sentence) = line.split_once('\t').unwrap();
        n += 1;
        match (label, claims_completion(sentence)) {
            ("claim", false) => missed.push(sentence),
            ("ok", true) => wrongly_caught.push(sentence),
            ("claim" | "ok", _) => {}
            _ => panic!("bad label {label}"),
        }
    }
    assert!(n >= 100, "the corpus has at least 100 phrasings ({n})");
    assert!(
        missed.is_empty() && wrongly_caught.is_empty(),
        "{} claims spoken as if true: {missed:#?}\n{} plain sentences replaced: {wrongly_caught:#?}",
        missed.len(),
        wrongly_caught.len()
    );
}
