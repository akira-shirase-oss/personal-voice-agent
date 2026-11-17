//! The completion-claim rule against its phrasing corpus (`fixtures/claims/corpus.tsv`). Every
//! line's verdict is pinned, every mismatch is reported, and the miss and false-positive rates
//! recorded in D-027 are recomputed and printed (`cargo test --test claims -- --nocapture`).
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::print_stdout,
    clippy::cast_precision_loss,
    reason = "test code: a panic is the failure report, and the rates are printed for D-027"
)]

use voice_domain::guard::{DRAFT_DISCLOSURE, Verdict, classify, disclosed};

struct Line {
    truth: &'static str,
    verdict: Verdict,
    source: &'static str,
    sentence: &'static str,
}

fn corpus() -> Vec<Line> {
    let text: &'static str = include_str!("../../../fixtures/claims/corpus.tsv");
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let mut cols = l.splitn(4, '\t');
            let (truth, verdict, source, sentence) = (
                cols.next().unwrap(),
                cols.next().unwrap(),
                cols.next().unwrap(),
                cols.next().unwrap_or_else(|| panic!("four columns: {l}")),
            );
            assert!(matches!(truth, "claim" | "ok"), "bad truth {truth}");
            let verdict = match verdict {
                "replace" => Verdict::Replace,
                "disclose" => Verdict::Disclose,
                "plain" => Verdict::Plain,
                v => panic!("bad verdict {v}"),
            };
            Line {
                truth,
                verdict,
                source,
                sentence,
            }
        })
        .collect()
}

#[test]
fn every_verdict_in_the_corpus_is_pinned() {
    let lines = corpus();
    assert!(
        lines.len() >= 300,
        "the corpus has at least 300 phrasings ({})",
        lines.len()
    );
    let wrong: Vec<String> = lines
        .iter()
        .filter_map(|l| {
            let got = classify(l.sentence);
            (got != l.verdict).then(|| format!("{:?} (pinned {:?}): {}", got, l.verdict, l.sentence))
        })
        .collect();
    assert!(
        wrong.is_empty(),
        "{} verdicts differ from the corpus:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// The phrasings both reviews found spoken as if true are never spoken bare, and none of the
/// second review's plain sentences loses its words to the disclaimer.
#[test]
fn the_reviews_phrasings_are_never_spoken_bare() {
    let lines = corpus();
    let spoken: Vec<&str> = lines
        .iter()
        .filter(|l| matches!(l.source, "r1" | "r2") && l.truth == "claim" && classify(l.sentence) == Verdict::Plain)
        .map(|l| l.sentence)
        .collect();
    assert!(spoken.is_empty(), "claims spoken as if true: {spoken:#?}");
    let replaced: Vec<&str> = lines
        .iter()
        .filter(|l| l.source == "r2" && l.truth == "ok" && classify(l.sentence) == Verdict::Replace)
        .map(|l| l.sentence)
        .collect();
    assert!(replaced.is_empty(), "plain sentences replaced: {replaced:#?}");
    assert!(
        lines.iter().filter(|l| l.source == "r2").count() >= 39,
        "the second review's phrasings are all in the corpus"
    );
}

#[test]
fn the_rates_recorded_in_d027_hold() {
    let lines = corpus();
    let count = |truth: &str, verdict: Verdict| {
        lines
            .iter()
            .filter(|l| l.truth == truth && classify(l.sentence) == verdict)
            .count()
    };
    let claims = lines.iter().filter(|l| l.truth == "claim").count();
    let oks = lines.iter().filter(|l| l.truth == "ok").count();
    let missed = count("claim", Verdict::Plain);
    let (fp_replaced, fp_disclosed) = (count("ok", Verdict::Replace), count("ok", Verdict::Disclose));
    let pct = |n: usize, d: usize| 100.0 * n as f64 / d as f64;
    println!(
        "claims {claims}: replaced {}, disclosed {}, missed {missed} ({:.1}%); \
         plain {oks}: intact {}, disclosed {fp_disclosed} ({:.1}%), replaced {fp_replaced} ({:.1}%)",
        count("claim", Verdict::Replace),
        count("claim", Verdict::Disclose),
        pct(missed, claims),
        count("ok", Verdict::Plain),
        pct(fp_disclosed, oks),
        pct(fp_replaced, oks),
    );
    // D-027's bounds: a rise in either is a regression to decide on, not to re-pin silently.
    assert!(missed <= 1, "claims missed: {missed}");
    assert!(fp_replaced <= 1, "plain sentences replaced: {fp_replaced}");
    assert!(fp_disclosed <= 15, "plain sentences disclosed: {fp_disclosed}");
}

#[test]
fn a_disclosed_sentence_keeps_its_words_and_ends_with_the_disclosure() {
    for l in corpus().iter().filter(|l| l.verdict == Verdict::Disclose) {
        let said = disclosed(l.sentence);
        assert!(said.starts_with(l.sentence.trim_end()), "{said}");
        assert!(said.ends_with(DRAFT_DISCLOSURE), "{said}");
    }
}
