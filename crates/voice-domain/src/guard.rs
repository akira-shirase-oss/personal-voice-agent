//! The completion-claim guard. A reply is a *draft*: model text that claims something was
//! done, sent, paid or booked is not spoken unless this turn has a successful tool result.
//! The check is lexical and deliberately broad — a false positive costs one replaced
//! sentence; a false negative would speak a fabricated "done".

const CLAIMS: &[&str] = &[
    "done",
    "i've sent",
    "i have sent",
    "i sent",
    "sent it",
    "it's sent",
    "has been sent",
    "have been sent",
    "was sent",
    "i've paid",
    "i have paid",
    "i paid",
    "been paid",
    "transferred",
    "i've booked",
    "i have booked",
    "booked it",
    "been booked",
    "i've scheduled",
    "i have scheduled",
    "been scheduled",
    "i've called",
    "i have called",
    "i've emailed",
    "i have emailed",
    "i've messaged",
    "i have messaged",
    "i've ordered",
    "i have ordered",
    "i've bought",
    "i have bought",
    "purchased",
    "i've deleted",
    "i have deleted",
    "been deleted",
    "i've cancelled",
    "i have cancelled",
    "i've canceled",
    "i have canceled",
    "completed",
    "taken care of",
    "all set",
];

/// What the session speaks instead of a sentence that claims completion.
pub const DRAFT_DISCLAIMER: &str = "That was only a draft; nothing has happened.";

pub fn claims_completion(sentence: &str) -> bool {
    let lowered = sentence.to_lowercase().replace('\u{2019}', "'");
    let words: String = lowered
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '\'' { c } else { ' ' })
        .collect();
    let padded = format!(" {} ", words.split_whitespace().collect::<Vec<_>>().join(" "));
    CLAIMS.iter().any(|claim| padded.contains(&format!(" {claim} ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_claims_are_caught() {
        for s in [
            "Done!",
            "I've sent it to Priya.",
            "I’ve sent the email.",
            "Your payment has been sent.",
            "All set, it's booked. Taken care of.",
            "I paid the invoice",
            "The meeting has been scheduled for 3pm.",
        ] {
            assert!(claims_completion(s), "{s}");
        }
    }

    #[test]
    fn drafts_and_plain_answers_pass() {
        for s in [
            "Here is a draft reply to Priya.",
            "I can't send messages yet.",
            "Paris is the capital of France.",
            "Would you like me to draft it?",
            "Abandoned buildings are common there.",
        ] {
            assert!(!claims_completion(s), "{s}");
        }
    }
}
