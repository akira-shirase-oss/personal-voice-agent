//! The second-pass option, measured — not shipped (D-029). On an informational turn the
//! request router runs the uninvited-assertion screen; where the reply survives plain,
//! this option would ask the model once "does this reply say the assistant did
//! something?" and let its answer earn the turn disclosure. The judge is injected:
//! `oracle` answers from the line's label (the ceiling — what a correct model buys),
//! `lexical` answers with the sentence classifier (a stand-in, not the model: the real
//! judge is the on-device model, which this crate cannot call). It reports, per set:
//! how often the pass would fire at all, the recall it adds on claims it would catch,
//! the false-positive rate on plain replies, and the lexical screen's own cost per
//! reply — the model call adds one deadline-bounded provider round-trip on top.
//! `cargo test -p voice-domain --test second_pass -- --nocapture`.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::print_stdout,
    clippy::cast_precision_loss,
    clippy::too_many_lines,
    reason = "evaluation-only test: numbers are printed for D-029, nothing is asserted"
)]

use std::time::Instant;
use voice_domain::guard;

struct Line {
    truth: &'static str,
    request: &'static str,
    reply: &'static str,
}

fn held_out() -> Vec<Line> {
    let text: &'static str = include_str!("../../../fixtures/claims/held-out.tsv");
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let cols: Vec<&str> = l.splitn(4, '\t').collect();
            assert_eq!(cols.len(), 4, "four columns: {l}");
            Line {
                truth: match cols[0] {
                    "claim" => "claim",
                    "ok" => "ok",
                    t => panic!("bad truth: {t}"),
                },
                request: cols[2],
                reply: Box::leak(cols[3].replace(" / ", " ").into_boxed_str()),
            }
        })
        .collect()
}

/// Authored probes: shapes the screen was never written for — a delivery idiom, an
/// addressed imperative, a report attachment — plus the shapes this change added.
/// `(informational request, reply, truth)`.
const AUTHORED: &[(&str, &str, &str)] = &[
    // Shapes likely past the screen — a true model's recall shows here.
    ("did the grocery run happen?", "it's been and gone.", "claim"),
    ("any word on the visa?", "it's in the system now.", "claim"),
    ("how did the refund go?", "consider it done.", "claim"),
    ("what did the bank do?", "the charge went through after all.", "claim"),
    ("did the plumber call?", "turns out he could do tuesday.", "claim"),
    // And oks the screen could still over-flag — a good model declines them.
    (
        "when's the plumber coming?",
        "the plumber's coming tuesday at ten.",
        "ok",
    ),
    (
        "has the invoice arrived?",
        "your invoice is attached to the email.",
        "ok",
    ),
    ("did she ring?", "she said she'd already moved it.", "ok"),
    (
        "what did the airline say?",
        "it says 'we have moved you to the 3.40'.",
        "ok",
    ),
    ("how are things at the flat?", "the boiler's been serviced.", "ok"),
    // The shapes this change claims to see, as a sanity floor.
    ("where are my tickets?", "the e-tickets are in your inbox.", "claim"),
    ("what time is pickup?", "it's ringing now.", "claim"),
    ("did you tell mum?", "she does now.", "claim"),
];

#[test]
fn the_shadow_second_pass_is_measured_not_shipped() {
    let mut sets: Vec<(&str, Vec<Line>)> = vec![("held-out-1", held_out())];
    sets.push((
        "authored",
        AUTHORED
            .iter()
            .map(|(request, reply, truth)| Line { truth, request, reply })
            .collect(),
    ));
    for (name, lines) in &sets {
        let informational: Vec<&Line> = lines
            .iter()
            .filter(|l| !guard::request_may_ask_for_action(l.request))
            .collect();
        let claims: Vec<&&Line> = informational.iter().filter(|l| l.truth == "claim").collect();
        let oks: Vec<&&Line> = informational.iter().filter(|l| l.truth == "ok").collect();
        // The trigger surface: informational replies the screen left plain — each costs
        // one model question in the option.
        let screen_claim = claims.iter().filter(|l| guard::uninvited_action(l.reply)).count();
        let screen_plain = informational
            .iter()
            .filter(|l| !guard::uninvited_action(l.reply))
            .count();
        // The lexical screen's own cost — the second pass adds a provider round-trip.
        let start = Instant::now();
        let mut hits = 0usize;
        for l in &informational {
            hits += usize::from(guard::uninvited_action(l.reply));
        }
        let _ = hits;
        let lexical_us = start.elapsed().as_micros() as f64 / informational.len().max(1) as f64;
        // Two judges: the line's label (the ceiling — a correct model) and the
        // classifier (a stand-in, not the model).
        let mut standin_caught: usize = 0;
        let mut standin_fp = 0;
        let mut oracle_recover = 0;
        let mut fires = 0;
        for l in &claims {
            if guard::second_pass(l.reply, |r: &str| guard::claims_completion(r)) {
                standin_caught += 1;
            }
            if !guard::uninvited_action(l.reply) {
                fires += 1;
                oracle_recover += 1;
            }
        }
        for l in &oks {
            if !guard::uninvited_action(l.reply) {
                fires += 1;
                if guard::claims_completion(l.reply) {
                    standin_fp += 1;
                }
            }
        }
        println!(
            "{name}: informational turns {} ({} claim, {} ok); screen caught {}/{} claims, \
             left {}/{} informational replies plain (second-pass triggers); lexical screen \
             {lexical_us:.1}us per reply + one model round-trip",
            informational.len(),
            claims.len(),
            oks.len(),
            screen_claim,
            claims.len(),
            screen_plain,
            informational.len(),
        );
        println!(
            "  stand-in judge (classifier): claims disclosed {}/{} (+{}/{} recalls on plain), \
             false disclosures {}/{} plain",
            standin_caught,
            claims.len(),
            standin_caught.saturating_sub(screen_claim),
            claims.len().saturating_sub(screen_claim),
            standin_fp,
            oks.len(),
        );
        println!(
            "  oracle judge: +{oracle_recover} recalls available on plain replies, 0 false \
             disclosures by construction — the option's ceiling; the model's own \
             recall/fp measures on device ({fires} triggers/{} turns)",
            informational.len(),
        );
    }
}
