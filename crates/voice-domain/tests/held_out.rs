//! The completion-claim guard on phrasings it was never tuned on (`fixtures/claims/held-out.tsv`),
//! through the real reducer: the heard request, the reply streamed sentence by sentence, then
//! `lm_done`. The rates are printed for D-027 (`cargo test --test held_out -- --nocapture`);
//! the one pin is that every claim answering its request ends with the turn-level disclosure.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::print_stdout,
    clippy::cast_precision_loss,
    clippy::too_many_lines,
    reason = "test code: a panic is the failure report, the rates are printed for D-027, and one turn is driven step by step"
)]

use voice_domain::consent::{ConsentScope, ConsentState};
use voice_domain::guard::{DRAFT_DISCLAIMER, DRAFT_DISCLOSURE};
use voice_domain::proposal::{Proposal, ProposalKind};
use voice_domain::protocol::{ClientMessage as C, ServiceMessage as S};
use voice_domain::record::Record;
use voice_domain::tools::ToolRegistry;
use voice_domain::{Input, Limits, Output, Session};

struct Line {
    truth: &'static str,
    group: &'static str,
    request: &'static str,
    sentences: Vec<&'static str>,
}

fn held_out() -> Vec<Line> {
    let text: &'static str = include_str!("../../../fixtures/claims/held-out.tsv");
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let cols: Vec<&str> = l.splitn(4, '\t').collect();
            assert_eq!(cols.len(), 4, "four columns: {l}");
            assert!(matches!(cols[0], "claim" | "ok"), "bad truth: {l}");
            Line {
                truth: cols[0],
                group: cols[1],
                request: cols[2],
                sentences: cols[3].split(" / ").collect(),
            }
        })
        .collect()
}

/// What the turn spoke: each chunk's text, and whether the turn ended disclosed.
struct Turn {
    spoken: Vec<String>,
    disclosed: bool,
    flagged: bool,
}

fn run(line: &Line) -> Turn {
    let mut s = Session::new(Limits::default(), ToolRegistry::default(), ConsentState::default());
    let mut all = Vec::new();
    let mut step = |s: &mut Session, msg: C| {
        let out = s.step(Input::Client(msg)).expect("accepted");
        all.extend(out.clone());
        out
    };
    let find = |out: &[Output], f: &dyn Fn(&S) -> Option<u64>| {
        out.iter()
            .find_map(|o| if let Output::Send(m) = o { f(m) } else { None })
            .expect("message")
    };
    step(
        &mut s,
        C::Consent {
            scope: ConsentScope::Capture,
            grant: true,
            rights_review: None,
        },
    );
    let out = step(&mut s, C::PttDown);
    let turn = find(&out, &|m| {
        if let S::TurnOpened { turn } = m {
            Some(turn.0)
        } else {
            None
        }
    });
    let out = step(
        &mut s,
        C::PttUp {
            turn: voice_domain::TurnId(turn),
        },
    );
    let stt = find(&out, &|m| {
        if let S::SttRequest { call, .. } = m {
            Some(call.0)
        } else {
            None
        }
    });
    let out = step(
        &mut s,
        C::SttFinal {
            call: voice_domain::CallId(stt),
            text: line.request.into(),
            min_confidence: Some(0.95),
        },
    );
    let lm = voice_domain::CallId(find(&out, &|m| {
        if let S::LmRequest { call, .. } = m {
            Some(call.0)
        } else {
            None
        }
    }));
    step(
        &mut s,
        C::LmKind {
            call: lm,
            kind: ProposalKind::Reply,
        },
    );
    for (i, sentence) in line.sentences.iter().enumerate() {
        step(
            &mut s,
            C::LmSentence {
                call: lm,
                index: u32::try_from(i).unwrap(),
                text: (*sentence).into(),
            },
        );
    }
    step(
        &mut s,
        C::LmDone {
            call: lm,
            proposal: Proposal::Reply {
                reply: line.sentences.join(" "),
            },
            model: "held-out".into(),
            instructions: "held-out".into(),
        },
    );
    let spoken: Vec<String> = all
        .iter()
        .filter_map(|o| {
            if let Output::Send(S::TtsRequest { text, .. }) = o {
                Some(text.clone())
            } else {
                None
            }
        })
        .collect();
    let disclosed = all
        .iter()
        .any(|o| matches!(o, Output::Record(Record::TurnDisclosed { .. })));
    let flagged = all.iter().any(|o| {
        matches!(
            o,
            Output::Record(Record::ClaimSuppressed { .. } | Record::ClaimDisclosed { .. })
        )
    });
    assert_eq!(
        disclosed,
        spoken.last().is_some_and(|t| t == DRAFT_DISCLOSURE),
        "a disclosed turn ends with the spoken disclosure: {spoken:?}"
    );
    Turn {
        spoken,
        disclosed,
        flagged,
    }
}

#[test]
fn every_held_out_claim_answering_its_request_ends_with_the_disclosure() {
    let lines = held_out();
    assert!(
        lines.len() >= 80,
        "the held-out set has at least 80 lines ({})",
        lines.len()
    );
    let undisclosed: Vec<String> = lines
        .iter()
        .filter(|l| l.truth == "claim" && l.group != "unprompted")
        .filter_map(|l| {
            let t = run(l);
            (!t.disclosed).then(|| format!("{} -> {:?}", l.request, t.spoken))
        })
        .collect();
    assert!(
        undisclosed.is_empty(),
        "claims spoken without the turn-level disclosure:\n{}",
        undisclosed.join("\n")
    );
}

#[test]
fn the_held_out_rates_are_printed_for_d027() {
    let lines = held_out();
    let pct = |n: usize, d: usize| if d == 0 { 0.0 } else { 100.0 * n as f64 / d as f64 };
    for (name, groups) in [
        ("review", &["review-b1", "review-b2", "review-split"][..]),
        ("own", &["own"][..]),
        ("unprompted", &["unprompted"][..]),
        (
            "all",
            &["review-b1", "review-b2", "review-split", "own", "unprompted"][..],
        ),
    ] {
        let set: Vec<&Line> = lines.iter().filter(|l| groups.contains(&l.group)).collect();
        let claims: Vec<(&Line, Turn)> = set
            .iter()
            .filter(|l| l.truth == "claim")
            .map(|l| (*l, run(l)))
            .collect();
        let oks: Vec<(&Line, Turn)> = set.iter().filter(|l| l.truth == "ok").map(|l| (*l, run(l))).collect();
        let sentence_missed = claims.iter().filter(|(_, t)| !t.flagged).count();
        let turn_missed = claims.iter().filter(|(_, t)| !t.disclosed).count();
        let replaced = oks
            .iter()
            .filter(|(_, t)| t.spoken.iter().any(|s| s == DRAFT_DISCLAIMER))
            .count();
        let disclosed = oks.iter().filter(|(_, t)| t.disclosed).count();
        println!(
            "held-out {name}: claims {}: sentence guard missed {sentence_missed} ({:.0}%), \
             spoken without any disclosure {turn_missed} ({:.0}%); plain {}: replaced {replaced} ({:.0}%), \
             given the turn disclosure {disclosed} ({:.0}%)",
            claims.len(),
            pct(sentence_missed, claims.len()),
            pct(turn_missed, claims.len()),
            oks.len(),
            pct(replaced, oks.len()),
            pct(disclosed, oks.len()),
        );
        for (l, t) in claims.iter().filter(|(_, t)| !t.disclosed) {
            println!("  undisclosed claim: {} -> {:?}", l.request, t.spoken);
        }
    }
}
