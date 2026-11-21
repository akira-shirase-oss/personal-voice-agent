//! The completion-claim guard's eighth held-out set (`fixtures/claims/held-out-8.jsonl`):
//! 120 lines, written blind — by someone who never saw the code or the earlier sets — and
//! shuffled. Each line labels whether its question asks for an action (`asks_action`):
//! 90 do not (45 claims, 45 plain replies) and 30 do (15 claims, 15 plain replies). The
//! informational half tests whether the set-7 fixes generalise to new words: unusual
//! done-verbs, got-passives and nominalisations; Indian, Singlish, South African,
//! Australian, AAVE, Caribbean and Welsh English; Scots and Pidgin with new vocabulary;
//! pronouns that bind back to a named party, including the assistant as the pronoun;
//! and hedged, reported and buried claims. The question is driven
//! as heard, so each item takes the path its `asks_action` implies
//! (`request_may_ask_for_action` decides it from the heard words, exactly as in a turn);
//! where that disagrees with the label, the item is reported as misrouted in both
//! directions. It is evaluation-only: the rates are printed for D-028 (`cargo test
//! --test held_out_8 -- --nocapture`), and nothing here pins them — the set is for
//! judging the guard, never for tuning it. The drive is the same path the other held-out
//! sets take: consent, a press, the heard question, `lm_kind`, the reply streamed
//! sentence by sentence, then `lm_done`.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::print_stdout,
    clippy::cast_precision_loss,
    clippy::too_many_lines,
    reason = "test code: a panic is the failure report, the rates are printed for D-028, and one turn is driven step by step"
)]

use voice_domain::consent::{ConsentScope, ConsentState};
use voice_domain::guard::{DRAFT_DISCLAIMER, DRAFT_DISCLOSURE, request_may_ask_for_action};
use voice_domain::proposal::{Proposal, ProposalKind};
use voice_domain::protocol::{ClientMessage as C, ServiceMessage as S};
use voice_domain::record::Record;
use voice_domain::session::split_sentences;
use voice_domain::tools::ToolRegistry;
use voice_domain::{Input, Limits, Output, Session};

struct Item {
    id: String,
    label: String,
    question: String,
    reply: String,
    asks_action: bool,
    note: String,
}

fn held_out_8() -> Vec<Item> {
    let text: &'static str = include_str!("../../../fixtures/claims/held-out-8.jsonl");
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).expect("a jsonl line");
            let get = |k: &str| v[k].as_str().expect("a string field").to_owned();
            Item {
                id: get("id"),
                label: get("label"),
                question: get("question"),
                reply: get("reply"),
                asks_action: v["asks_action"].as_bool().expect("a bool field"),
                note: get("note"),
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

fn run(item: &Item) -> Turn {
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
            text: item.question.clone(),
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
    let sentences = split_sentences(&item.reply);
    for (i, sentence) in sentences.iter().enumerate() {
        step(
            &mut s,
            C::LmSentence {
                call: lm,
                index: u32::try_from(i).unwrap(),
                text: sentence.clone(),
            },
        );
    }
    step(
        &mut s,
        C::LmDone {
            call: lm,
            proposal: Proposal::Reply {
                reply: item.reply.clone(),
            },
            model: "held-out-8".into(),
            instructions: "held-out-8".into(),
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
fn the_blind_set_is_well_formed() {
    let items = held_out_8();
    assert_eq!(items.len(), 120, "the blind set has 120 lines");
    for (asks, claims, oks) in [(false, 45, 45), (true, 15, 15)] {
        let set: Vec<&Item> = items.iter().filter(|i| i.asks_action == asks).collect();
        assert_eq!(
            set.iter().filter(|i| i.label == "claim").count(),
            claims,
            "{claims} claims where asks_action is {asks}"
        );
        assert_eq!(
            set.iter().filter(|i| i.label == "ok").count(),
            oks,
            "{oks} plain replies where asks_action is {asks}"
        );
    }
}

#[test]
fn the_held_out_8_rates_are_printed_for_d028() {
    let items = held_out_8();
    let results: Vec<(&Item, Turn)> = items.iter().map(|i| (i, run(i))).collect();
    for asks in [false, true] {
        let set: Vec<(&Item, &Turn)> = results
            .iter()
            .filter(|(i, _)| i.asks_action == asks)
            .map(|(i, t)| (*i, t))
            .collect();
        let claims: Vec<(&Item, &Turn)> = set.iter().filter(|(i, _)| i.label == "claim").copied().collect();
        let oks: Vec<(&Item, &Turn)> = set.iter().filter(|(i, _)| i.label == "ok").copied().collect();
        let pct = |n: usize, d: usize| if d == 0 { 0.0 } else { 100.0 * n as f64 / d as f64 };
        let undisclosed: Vec<(&Item, &Turn)> = claims.iter().filter(|(_, t)| !t.disclosed).copied().collect();
        let sentence_missed = claims.iter().filter(|(_, t)| !t.flagged).count();
        let replaced: Vec<(&Item, &Turn)> = oks
            .iter()
            .filter(|(_, t)| t.spoken.iter().any(|s| s == DRAFT_DISCLAIMER))
            .copied()
            .collect();
        let disclosed: Vec<(&Item, &Turn)> = oks.iter().filter(|(_, t)| t.disclosed).copied().collect();
        println!(
            "held-out-8 (asks_action={asks}): claims {}: sentence guard missed {} ({:.0}%), \
             spoken without the disclosure {} ({:.0}%); plain {}: replaced {} ({:.0}%), \
             turn-disclosed {} ({:.0}%)",
            claims.len(),
            sentence_missed,
            pct(sentence_missed, claims.len()),
            undisclosed.len(),
            pct(undisclosed.len(), claims.len()),
            oks.len(),
            replaced.len(),
            pct(replaced.len(), oks.len()),
            disclosed.len(),
            pct(disclosed.len(), oks.len()),
        );
        for (i, t) in &undisclosed {
            println!(
                "  undisclosed claim: {} | {} -> {:?} [{}]",
                i.id, i.reply, t.spoken, i.note
            );
        }
        for (i, t) in &replaced {
            println!("  replaced ok: {} | {} -> {:?} [{}]", i.id, i.reply, t.spoken, i.note);
        }
        for (i, _) in &disclosed {
            println!("  turn-disclosed ok: {} | {} [{}]", i.id, i.reply, i.note);
        }
    }
    // Routing in both directions: informational questions called action-asking, and
    // action requests called informational (the dangerous direction — a missed claim
    // there has no unconditional disclosure to catch it).
    let to_action: Vec<&Item> = items
        .iter()
        .filter(|i| !i.asks_action && request_may_ask_for_action(&i.question))
        .collect();
    let to_info: Vec<&Item> = items
        .iter()
        .filter(|i| i.asks_action && !request_may_ask_for_action(&i.question))
        .collect();
    println!(
        "held-out-8 routing: {} of {} informational questions called action-asking; \
         {} of {} action requests called informational",
        to_action.len(),
        items.iter().filter(|i| !i.asks_action).count(),
        to_info.len(),
        items.iter().filter(|i| i.asks_action).count(),
    );
    for i in &to_action {
        println!("  informational -> action: {} | {} [{}]", i.id, i.question, i.note);
    }
    for i in &to_info {
        println!("  action -> informational: {} | {} [{}]", i.id, i.question, i.note);
    }
}
