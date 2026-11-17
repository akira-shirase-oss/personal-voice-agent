//! The completion-claim rule (`docs/END_GOAL.md`, invariant 2). A reply is *drafted text*: a sentence
//! that says an action happened — sent, paid, booked, set, saved, moved, done — is spoken
//! only when this turn's ledger holds executed evidence (the session checks that); otherwise
//! it is replaced by the draft disclaimer.
//!
//! The classifier is structural, not a phrase list. A clause claims an action when a past or
//! participle form of an action verb (or a completion word) appears
//! - in a first-person statement: "I've added milk", "I went ahead and ordered it";
//! - as the predicate of a task, pronoun or "your …" subject: "the invoice has been paid",
//!   "your transfer is complete", "it's done";
//! - as a headline after a task noun: "Payment sent", "Alarm set for 6";
//! - on its own: "Sent!", "Done.", "Successfully sent.";
//! - in a completion phrase: "on its way", "went through", "all set", "taken care of".
//!
//! Questions, negated clauses ("nothing was sent"), modal or future forms ("it will be
//! sent"), conditionals ("once you approve it") and habitual statements ("messages are
//! usually delivered") are not claims. It errs towards catching: a false positive replaces one
//! sentence; a false negative would speak a fabricated "done". It is a classifier, so an
//! unusual phrasing can still slip through; what was spoken is recorded in the ledger so a
//! slip is visible. `fixtures/claims/corpus.tsv` pins its behaviour.

const ACTED: &[&str] = &[
    "sent",
    "paid",
    "booked",
    "scheduled",
    "rescheduled",
    "texted",
    "emailed",
    "messaged",
    "called",
    "dialed",
    "dialled",
    "saved",
    "added",
    "set",
    "created",
    "deleted",
    "removed",
    "moved",
    "transferred",
    "ordered",
    "notified",
    "submitted",
    "posted",
    "updated",
    "cancelled",
    "canceled",
    "completed",
    "finished",
    "bought",
    "purchased",
    "reserved",
    "shared",
    "forwarded",
    "replied",
    "invited",
    "reminded",
    "noted",
    "logged",
    "recorded",
    "changed",
    "turned",
    "opened",
    "closed",
    "placed",
    "confirmed",
    "approved",
    "signed",
    "uploaded",
    "downloaded",
    "installed",
    "delivered",
    "wrote",
    "written",
    "filed",
    "archived",
    "started",
    "stopped",
    "done",
    "renewed",
    "refunded",
    "returned",
    "paused",
    "unsubscribed",
    "subscribed",
    "printed",
    "copied",
    "cleared",
    "locked",
    "unlocked",
    "enabled",
    "disabled",
    "muted",
    "blocked",
    "unblocked",
    "accepted",
    "declined",
    "answered",
    "assigned",
    "arranged",
    "organized",
    "organised",
    "handled",
];
/// Words that close a clause about a task as finished: "transfer complete".
const COMPLETE: &[&str] = &[
    "complete",
    "completed",
    "done",
    "finished",
    "sent",
    "booked",
    "paid",
    "confirmed",
    "delivered",
];
const NEGATORS: &[&str] = &[
    "not",
    "never",
    "nothing",
    "no",
    "none",
    "haven't",
    "hasn't",
    "hadn't",
    "didn't",
    "don't",
    "doesn't",
    "can't",
    "cannot",
    "won't",
    "wouldn't",
    "couldn't",
    "shouldn't",
    "isn't",
    "aren't",
    "wasn't",
    "weren't",
    "unable",
    "without",
    "neither",
    "nor",
];
const MODALS: &[&str] = &[
    "will", "would", "can", "could", "should", "may", "might", "must", "shall", "going", "gonna", "to", "want",
    "wants", "need", "needs", "let", "i'll", "we'll", "you'll", "it'll", "be",
];
const CONDITIONAL: &[&str] = &[
    "if", "once", "when", "whenever", "after", "before", "until", "unless", "whether",
];
const HABITUAL: &[&str] = &[
    "usually",
    "typically",
    "normally",
    "generally",
    "often",
    "always",
    "sometimes",
    "rarely",
];
const FIRST_PERSON: &[&str] = &["i", "i've", "we", "we've"];
/// Between a first-person subject and its verb: "I have just", "I went ahead and".
const FIRST_FILLER: &[&str] = &[
    "have",
    "had",
    "just",
    "already",
    "also",
    "then",
    "now",
    "successfully",
    "went",
    "gone",
    "ahead",
    "and",
    "quickly",
    "finally",
    "actually",
];
const AUX: &[&str] = &[
    "is",
    "are",
    "was",
    "were",
    "has",
    "have",
    "had",
    "been",
    "got",
    "gets",
    "it's",
    "that's",
    "everything's",
];
/// Auxiliaries that carry their own task subject.
const AUX_WITH_SUBJECT: &[&str] = &["it's", "that's", "everything's"];
const PASSIVE_FILLER: &[&str] = &[
    "been",
    "just",
    "already",
    "now",
    "all",
    "successfully",
    "officially",
    "finally",
    "also",
    "fully",
];
/// Before a bare participle: "Successfully sent", "All done".
const BARE_FILLER: &[&str] = &[
    "successfully",
    "all",
    "just",
    "okay",
    "ok",
    "sure",
    "yes",
    "great",
    "alright",
    "and",
];
const PRONOUNS: &[&str] = &["it", "that", "this", "everything", "your", "yours"];
const TASK_NOUNS: &[&str] = &[
    "message",
    "messages",
    "text",
    "texts",
    "email",
    "emails",
    "mail",
    "payment",
    "payments",
    "money",
    "transfer",
    "invoice",
    "bill",
    "bills",
    "reminder",
    "reminders",
    "alarm",
    "alarms",
    "note",
    "notes",
    "list",
    "meeting",
    "meetings",
    "call",
    "appointment",
    "booking",
    "reservation",
    "table",
    "room",
    "ticket",
    "tickets",
    "order",
    "event",
    "calendar",
    "invite",
    "invitation",
    "file",
    "files",
    "document",
    "photo",
    "photos",
    "form",
    "subscription",
    "lights",
    "light",
    "request",
    "expense",
    "report",
    "reply",
    "refund",
    "task",
    "tasks",
    "delivery",
    "transaction",
    "deposit",
    "timer",
    "contact",
];
const LEADING: &[&str] = &[
    "okay", "ok", "sure", "great", "alright", "yes", "so", "the", "your", "my", "a", "an", "our",
];
const PHRASES: &[&[&str]] = &[
    &["all", "set"],
    &["taken", "care", "of"],
    &["on", "its", "way"],
    &["on", "the", "way"],
    &["went", "through"],
    &["gone", "through"],
];

/// What the session speaks instead of a sentence that claims completion.
pub const DRAFT_DISCLAIMER: &str = "That was only a draft; nothing has happened.";

fn is(word: &str, set: &[&str]) -> bool {
    set.contains(&word)
}

fn clauses(lowered: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for part in lowered.split([',', ';', ':', '—', '–']) {
        for clause in part.split(" but ") {
            let words: Vec<String> = clause
                .split(|c: char| !(c.is_alphanumeric() || c == '\''))
                .map(|w| w.trim_matches('\''))
                .filter(|w| !w.is_empty())
                .map(str::to_owned)
                .collect();
            if !words.is_empty() {
                out.push(words);
            }
        }
    }
    out
}

fn has_phrase(w: &[&str], phrase: &[&str]) -> bool {
    w.windows(phrase.len()).any(|win| win == phrase)
}

fn clause_claims(w: &[&str]) -> bool {
    if w.iter().any(|t| is(t, CONDITIONAL) || is(t, HABITUAL)) {
        return false;
    }
    let negated_at = |i: usize| w[..i].iter().any(|t| is(t, NEGATORS));
    if PHRASES.iter().any(|p| has_phrase(w, p)) && !negated_at(w.len()) {
        return true;
    }
    let has_task_noun = w.iter().any(|t| is(t, TASK_NOUNS));
    if let Some(last) = w.last()
        && is(last, COMPLETE)
        && has_task_noun
        && !negated_at(w.len() - 1)
        // "to be complete" is not a claim; a bare "to" here is a preposition ("… to Priya complete").
        && !w[w.len().saturating_sub(3)..w.len() - 1].iter().any(|t| is(t, MODALS) && *t != "to")
    {
        return true;
    }
    for (i, word) in w.iter().enumerate() {
        if !(is(word, ACTED) || *word == "complete") || negated_at(i) {
            continue;
        }
        if w[i.saturating_sub(2)..i].iter().any(|t| is(t, MODALS)) {
            continue;
        }
        // On its own: "Sent!", "Successfully sent", "All done".
        if w[..i].iter().all(|t| is(t, BARE_FILLER)) {
            return true;
        }
        // First person: "I've added", "I just sent", "I went ahead and ordered".
        if let Some(j) = w[..i].iter().rposition(|t| is(t, FIRST_PERSON))
            && i - j <= 5
            && w[j + 1..i].iter().all(|t| is(t, FIRST_FILLER))
        {
            return true;
        }
        // Predicate of a task: "the invoice has been paid", "your transfer is complete".
        if let Some(k) = w[..i].iter().rposition(|t| is(t, AUX))
            && w[k + 1..i].iter().all(|t| is(t, PASSIVE_FILLER))
            && (is(w[k], AUX_WITH_SUBJECT) || w[..k].iter().any(|t| is(t, TASK_NOUNS) || is(t, PRONOUNS)))
        {
            return true;
        }
        // Headline after a task noun: "Payment sent", "Alarm set for 6".
        let start = w.iter().position(|t| !is(t, LEADING)).unwrap_or(0);
        if start < i && i - start <= 2 && is(w[start], TASK_NOUNS) {
            return true;
        }
    }
    false
}

pub fn claims_completion(sentence: &str) -> bool {
    let lowered = sentence.to_lowercase().replace(['\u{2019}', '\u{2018}'], "'");
    if lowered.trim_end().ends_with('?') {
        return false;
    }
    clauses(&lowered)
        .iter()
        .any(|c| clause_claims(&c.iter().map(String::as_str).collect::<Vec<_>>()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_negations_and_futures_are_not_claims() {
        for s in [
            "Have you sent it?",
            "Nothing was sent.",
            "It will be sent once you approve.",
            "I didn't pay it.",
            "The payment needs to be complete.",
        ] {
            assert!(!claims_completion(s), "{s}");
        }
    }

    #[test]
    fn the_disclaimer_itself_is_not_a_claim() {
        assert!(!claims_completion(DRAFT_DISCLAIMER));
    }
}
