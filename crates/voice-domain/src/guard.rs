//! The completion-claim rule (`docs/END_GOAL.md`, invariant 2). A reply is *drafted text*: a
//! sentence that says an action happened — sent, paid, booked, notified, scheduled, cancelled,
//! ordered, done — is spoken as it is only when this turn's ledger holds executed evidence (the
//! session checks that). No tool can run in the first release, so the rule fails closed: every
//! sentence that might assert a completed action gets a disclosure, and only sentences that
//! assert nothing of the kind are spoken untouched.
//!
//! [`classify`] gives one of three verdicts per sentence:
//! - [`Verdict::Replace`] — it says an action happened: a past, perfect, passive or headline
//!   form of an action verb with the assistant, a task, a person or a pronoun as its subject
//!   ("I've texted Priya", "Priya has been notified", "$50 sent to Priya", "Sent!"), a
//!   completion phrase ("on its way", "went out", "consider it done"), or "I'm sending it now".
//!   The session speaks [`DRAFT_DISCLAIMER`] instead.
//! - [`Verdict::Disclose`] — it might say so: a less certain form, such as a general verb with
//!   a pronoun subject ("It's set for 9"), a third party's action ("Priya sent the money"), or
//!   a claim inside a quoted draft. The session speaks it as written, and its turn ends with
//!   [`DRAFT_DISCLOSURE`].
//! - [`Verdict::Plain`] — no completed action is asserted: spoken as written.
//!
//! A verb is not a claim when its own clause makes it one of: a question ("Have you sent
//! it?"), negated ("nothing was sent", "I haven't paid it"), modal or future ("it will be
//! sent", "I'd have booked it"), conditional ("once it's paid"), habitual ("messages are
//! usually delivered"), the user's action ("you told me you sent it"), historical ("completed
//! in 1889") or an attributive adjective ("the paid version"). Each of these exempts a verb
//! only when it governs that verb: "I paid the bill after you confirmed" and "No problem I've
//! sent it" are still claims, and so is the first clause of "I've booked it, anything else?".
//!
//! It is a classifier for English, with a short list of other languages' participles, and on
//! phrasings it was never tuned on it misses most claims (D-027: `fixtures/claims/held-out.tsv`).
//! So it is not the safety net: the turn is. A reply turn whose request may ask for an action
//! ([`request_may_ask_for_action`]), or with any sentence flagged here, ends with one spoken
//! [`DRAFT_DISCLOSURE`]. What was spoken is ledgered, so a slip is visible.
//! `fixtures/claims/corpus.tsv` pins every sentence verdict.

/// How the session speaks one model sentence while its turn has no executed evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Verdict {
    /// Asserts no completed action: spoken as written.
    Plain,
    /// Might assert a completed action: spoken with [`DRAFT_DISCLOSURE`] after it.
    Disclose,
    /// Asserts a completed action: replaced by [`DRAFT_DISCLAIMER`].
    Replace,
}

/// What the session speaks instead of a sentence that claims completion.
pub const DRAFT_DISCLAIMER: &str = "That was only a draft; nothing has happened.";
/// What the session speaks once at the end of a turn that may have claimed an action.
pub const DRAFT_DISCLOSURE: &str = "Nothing has been done.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    /// Acts on someone's behalf (send, pay, book, notify, cancel, order…): a completed form is
    /// the claim the rule exists for.
    Acts,
    /// "done", "complete": the bare completion words.
    Completion,
    /// A general action (set, add, save, move…): a claim with a task or the assistant as its
    /// subject, a possible one otherwise.
    General,
    /// An action whose participle mostly describes a state (closed, changed, locked…).
    State,
    /// A claim only next to a transaction noun ("order placed", "payment processed").
    TaskOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Form {
    /// Past tense or past participle.
    Done,
    /// Progressive: "sending".
    Ing,
}

#[derive(Clone, Copy, Debug)]
struct Verb {
    lemma: &'static str,
    class: Class,
    form: Form,
}

struct Entry {
    lemma: &'static str,
    class: Class,
    done: &'static [&'static str],
    ing: &'static [&'static str],
}

const fn e(lemma: &'static str, class: Class, done: &'static [&'static str], ing: &'static [&'static str]) -> Entry {
    Entry {
        lemma,
        class,
        done,
        ing,
    }
}

use Class::{Acts, Completion, General, State, TaskOnly};

const VERBS: &[Entry] = &[
    e("send", Acts, &["sent"], &["sending"]),
    e("pay", Acts, &["paid"], &["paying"]),
    e("book", Acts, &["booked"], &["booking"]),
    e("schedule", Acts, &["scheduled"], &["scheduling"]),
    e("reschedule", Acts, &["rescheduled"], &["rescheduling"]),
    e("notify", Acts, &["notified"], &["notifying"]),
    e("cancel", Acts, &["cancelled", "canceled"], &["cancelling", "canceling"]),
    e("order", Acts, &["ordered"], &["ordering"]),
    e("text", Acts, &["texted"], &["texting"]),
    e("email", Acts, &["emailed"], &["emailing"]),
    e("message", Acts, &["messaged"], &["messaging"]),
    e("transfer", Acts, &["transferred"], &["transferring"]),
    e("submit", Acts, &["submitted"], &["submitting"]),
    e("purchase", Acts, &["purchased"], &["purchasing"]),
    e("buy", Acts, &["bought"], &["buying"]),
    e("reserve", Acts, &["reserved"], &["reserving"]),
    e("forward", Acts, &["forwarded"], &["forwarding"]),
    e("refund", Acts, &["refunded"], &["refunding"]),
    e("wire", Acts, &["wired"], &["wiring"]),
    e("deposit", Acts, &["deposited"], &["depositing"]),
    e("withdraw", Acts, &["withdrew", "withdrawn"], &["withdrawing"]),
    e("dial", Acts, &["dialed", "dialled"], &["dialing", "dialling"]),
    e("call", Acts, &["called"], &["calling"]),
    e("phone", Acts, &["phoned"], &["phoning"]),
    e("ping", Acts, &["pinged"], &["pinging"]),
    e("contact", Acts, &["contacted"], &["contacting"]),
    e("inform", Acts, &["informed"], &["informing"]),
    e("alert", Acts, &["alerted"], &["alerting"]),
    e("tell", Acts, &["told"], &["telling"]),
    e("invite", Acts, &["invited"], &["inviting"]),
    e("remind", Acts, &["reminded"], &["reminding"]),
    e("reply", Acts, &["replied"], &["replying"]),
    e("respond", Acts, &["responded"], &["responding"]),
    e("upload", Acts, &["uploaded"], &["uploading"]),
    e("post", Acts, &["posted"], &["posting"]),
    e("share", Acts, &["shared"], &["sharing"]),
    e("deliver", Acts, &["delivered"], &["delivering"]),
    e("renew", Acts, &["renewed"], &["renewing"]),
    e("subscribe", Acts, &["subscribed"], &["subscribing"]),
    e("unsubscribe", Acts, &["unsubscribed"], &["unsubscribing"]),
    e("receive", Acts, &["received"], &[]),
    e("write", Acts, &["wrote", "written"], &["writing"]),
    e("reach", Acts, &["reached"], &["reaching"]),
    e("issue", Acts, &["issued"], &["issuing"]),
    e("ship", Acts, &["shipped"], &["shipping"]),
    e("dispatch", Acts, &["dispatched"], &["dispatching"]),
    e("do", Completion, &["done"], &[]),
    e("complete", Completion, &["complete"], &[]),
    e("set", General, &["set"], &["setting"]),
    e("add", General, &["added"], &["adding"]),
    e("save", General, &["saved"], &["saving"]),
    e("create", General, &["created"], &["creating"]),
    e("delete", General, &["deleted"], &["deleting"]),
    e("remove", General, &["removed"], &["removing"]),
    e("move", General, &["moved"], &["moving"]),
    e("update", General, &["updated"], &["updating"]),
    e("finish", General, &["finished"], &["finishing"]),
    e("complete", General, &["completed"], &["completing"]),
    e("confirm", General, &["confirmed"], &["confirming"]),
    e("approve", General, &["approved"], &["approving"]),
    e("sign", General, &["signed"], &["signing"]),
    e("download", General, &["downloaded"], &["downloading"]),
    e("install", General, &["installed"], &["installing"]),
    e("file", General, &["filed"], &["filing"]),
    e("archive", General, &["archived"], &["archiving"]),
    e("return", General, &["returned"], &["returning"]),
    e("print", General, &["printed"], &["printing"]),
    e("copy", General, &["copied"], &["copying"]),
    e("assign", General, &["assigned"], &["assigning"]),
    e("arrange", General, &["arranged"], &["arranging"]),
    e(
        "organize",
        General,
        &["organized", "organised"],
        &["organizing", "organising"],
    ),
    e("handle", General, &["handled"], &["handling"]),
    e("record", General, &["recorded"], &["recording"]),
    e("log", General, &["logged"], &["logging"]),
    e("note", General, &["noted"], &["noting"]),
    e("accept", General, &["accepted"], &["accepting"]),
    e("decline", General, &["declined"], &["declining"]),
    e("answer", General, &["answered"], &["answering"]),
    e("turn", General, &["turned"], &["turning"]),
    e("plan", General, &["planned"], &["planning"]),
    e("fix", General, &["fixed"], &["fixing"]),
    e("reset", General, &["reset"], &["resetting"]),
    e("rename", General, &["renamed"], &["renaming"]),
    e("attach", General, &["attached"], &["attaching"]),
    e("close", State, &["closed"], &["closing"]),
    e("open", State, &["opened"], &["opening"]),
    e("change", State, &["changed"], &["changing"]),
    e("lock", State, &["locked"], &["locking"]),
    e("unlock", State, &["unlocked"], &["unlocking"]),
    e("clear", State, &["cleared"], &["clearing"]),
    e("start", State, &["started"], &["starting"]),
    e("stop", State, &["stopped"], &["stopping"]),
    e("pause", State, &["paused"], &["pausing"]),
    e("block", State, &["blocked"], &["blocking"]),
    e("unblock", State, &["unblocked"], &["unblocking"]),
    e("enable", State, &["enabled"], &["enabling"]),
    e("disable", State, &["disabled"], &["disabling"]),
    e("mute", State, &["muted"], &["muting"]),
    e("place", TaskOnly, &["placed"], &[]),
    e("process", TaskOnly, &["processed"], &[]),
    e("make", TaskOnly, &["made"], &[]),
    e("execute", TaskOnly, &["executed"], &[]),
];

/// Verbs that act only with one of these words right after them ("turned off", "reached out",
/// "wrote to Sam"); otherwise "it's turned cold" or "Shakespeare wrote Hamlet".
const PARTICLES: &[(&str, &[&str])] = &[
    ("turn", &["off", "on", "up", "down"]),
    ("reach", &["out"]),
    ("write", &["to", "back"]),
];

/// Participles of send / pay / book / transfer in Spanish, French, German, Italian and
/// Portuguese. Any of them outside a question is treated as a claim (negation is not parsed).
const FOREIGN: &[&str] = &[
    "enviado",
    "enviada",
    "enviados",
    "enviadas",
    "envié",
    "enviei",
    "pagado",
    "pagada",
    "pagué",
    "paguei",
    "transferido",
    "transferida",
    "reservado",
    "reservada",
    "reservei",
    "envoyé",
    "envoyée",
    "envoyés",
    "envoyées",
    "payé",
    "payée",
    "réservé",
    "réservée",
    "viré",
    "gesendet",
    "geschickt",
    "verschickt",
    "bezahlt",
    "überwiesen",
    "gebucht",
    "inviato",
    "inviata",
    "pagato",
    "pagata",
    "prenotato",
    "prenotata",
    "bonificato",
];

const BE: &[&str] = &["is", "are", "was", "were", "be", "been", "being", "am"];
const HAVE: &[&str] = &["has", "have", "had", "having"];
const GET: &[&str] = &["got", "gets", "get", "gotten", "getting"];
const MODALS: &[&str] = &[
    "will",
    "would",
    "can",
    "could",
    "should",
    "may",
    "might",
    "must",
    "shall",
    "gonna",
    "won't",
    "wouldn't",
    "can't",
    "couldn't",
    "shouldn't",
    "cannot",
    "mustn't",
    "i'll",
    "we'll",
    "you'll",
    "it'll",
    "they'll",
    "he'll",
    "she'll",
    "that'll",
];
const NEGATORS: &[&str] = &[
    "not",
    "never",
    "nothing",
    "no",
    "none",
    "nobody",
    "neither",
    "nor",
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
    "ain't",
    "unable",
    "without",
    "yet",
];
/// Negators that are themselves the subject: "nothing was sent".
const NEG_SUBJECTS: &[&str] = &["nothing", "none", "nobody", "neither"];
const CONDITIONAL: &[&str] = &[
    "if", "once", "when", "whenever", "after", "before", "until", "unless", "whether", "till",
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
    "seldom",
];
/// Habit markers that may follow the verb; they exempt only general and state verbs.
const HABIT_AFTER: &[&str] = &[
    "every",
    "daily",
    "weekly",
    "monthly",
    "yearly",
    "mondays",
    "tuesdays",
    "wednesdays",
    "thursdays",
    "fridays",
    "saturdays",
    "sundays",
    "weekends",
    "weekdays",
    "nightly",
];
/// Before a subject: "want it sent", "let me set".
const DESIRE: &[&str] = &[
    "want", "wants", "wanted", "need", "needs", "needed", "like", "let", "lets", "prefer", "rather",
];
/// Between a subject and its verb.
const ADVERBS: &[&str] = &[
    "just",
    "already",
    "also",
    "now",
    "all",
    "successfully",
    "officially",
    "finally",
    "fully",
    "quickly",
    "actually",
    "then",
    "ahead",
    "gone",
    "went",
    "really",
    "both",
    "still",
    "even",
    "automatically",
    "securely",
    "safely",
    "immediately",
    "instantly",
    "recently",
    "right",
    "go",
];
/// Before a bare participle: "Successfully sent", "All done", "Okay, done".
const BARE_FILLER: &[&str] = &[
    "successfully",
    "all",
    "just",
    "okay",
    "ok",
    "sure",
    "yes",
    "yep",
    "great",
    "alright",
    "and",
    "so",
    "also",
    "now",
    "already",
    "finally",
    "and",
    "then",
];
const ARTICLES: &[&str] = &[
    "the", "a", "an", "every", "each", "any", "some", "this", "these", "those",
];
const POSSESSIVES: &[&str] = &["your", "my", "our", "their", "his", "her", "its"];
/// May start a noun phrase.
const DETERMINERS: &[&str] = &[
    "the", "a", "an", "your", "my", "our", "their", "his", "her", "its", "this", "that", "these", "those", "every",
    "each", "some", "any", "no", "all", "both", "of",
];
const OBJECT_PRONOUNS: &[&str] = &["it", "them", "this", "that", "him", "her", "me", "us"];
/// After a participle in a headline or passive: "sent to Priya", "set for 6", "booked."
const AFTER_PARTICIPLE: &[&str] = &[
    "to",
    "for",
    "at",
    "on",
    "in",
    "by",
    "from",
    "via",
    "with",
    "into",
    "onto",
    "over",
    "out",
    "off",
    "up",
    "back",
    "down",
    "successfully",
    "already",
    "now",
    "just",
    "today",
    "tonight",
    "tomorrow",
    "yesterday",
    "earlier",
    "as",
    "through",
    "before",
    "after",
    "until",
    "safely",
    "securely",
    "automatically",
    "instantly",
    "immediately",
    "and",
    "too",
];
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
    "transfers",
    "invoice",
    "invoices",
    "bill",
    "bills",
    "rent",
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
    "calls",
    "appointment",
    "appointments",
    "booking",
    "bookings",
    "reservation",
    "reservations",
    "table",
    "room",
    "hotel",
    "flight",
    "flights",
    "ticket",
    "tickets",
    "order",
    "orders",
    "purchase",
    "event",
    "calendar",
    "invite",
    "invites",
    "invitation",
    "invitations",
    "file",
    "files",
    "document",
    "documents",
    "photo",
    "photos",
    "form",
    "forms",
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
    "package",
    "transaction",
    "deposit",
    "timer",
    "contact",
    "thermostat",
    "car",
    "ride",
    "dinner",
    "lunch",
    "order",
    "receipt",
    "letter",
    "card",
    "account",
    "password",
    "door",
];
/// Things an assistant might claim to have switched on or off.
const DEVICES: &[&str] = &[
    "light",
    "lights",
    "alarm",
    "alarms",
    "reminder",
    "reminders",
    "timer",
    "timers",
    "thermostat",
    "heating",
    "heater",
    "fan",
    "tv",
    "notifications",
];
/// Transaction nouns that make "made", "placed", "processed" a claim.
const TASK_STRONG: &[&str] = &[
    "payment",
    "payments",
    "transfer",
    "reservation",
    "reservations",
    "booking",
    "bookings",
    "order",
    "orders",
    "purchase",
    "appointment",
    "deposit",
    "refund",
    "transaction",
    "call",
    "calls",
    "request",
];
const NUMBER_WORDS: &[&str] = &[
    "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "fifteen",
    "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety", "hundred", "thousand", "million",
    "dollars", "dollar", "euros", "euro", "pounds", "bucks", "cents", "rupees", "yen",
];
/// Reported speech: the sentence restates what the user said.
const HEARSAY: &[&[&str]] = &[
    &["you", "said"],
    &["you", "told", "me"],
    &["you", "mentioned"],
    &["according", "to", "you"],
    &["according", "to", "what"],
    &["from", "what", "you"],
    &["you've", "told", "me"],
];
/// Words a framing colon follows: "Here's a draft: …", "You could say: …".
const FRAMES: &[&str] = &[
    "draft",
    "say",
    "says",
    "write",
    "read",
    "reads",
    "wording",
    "message",
    "reply",
    "text",
    "email",
    "note",
    "example",
    "like",
    "version",
    "suggestion",
    "template",
];
/// Opens a clause that is a question even without a question mark at its end.
const INVERTED: &[&str] = &[
    "have", "has", "had", "did", "do", "does", "is", "are", "was", "were", "can", "could", "would", "will", "should",
    "shall", "may", "might", "must", "am", "haven't", "hasn't", "didn't", "don't", "doesn't", "isn't", "aren't",
    "wasn't", "weren't",
];

fn is(word: &str, set: &[&str]) -> bool {
    set.contains(&word)
}

fn verb(word: &str) -> Option<Verb> {
    VERBS.iter().find_map(|entry| {
        let form = if entry.done.contains(&word) {
            Form::Done
        } else if entry.ing.contains(&word) {
            Form::Ing
        } else {
            return None;
        };
        Some(Verb {
            lemma: entry.lemma,
            class: entry.class,
            form,
        })
    })
}

fn is_number(word: &str) -> bool {
    word.chars().next().is_some_and(|c| c.is_ascii_digit()) || is(word, NUMBER_WORDS)
}

fn is_year(word: &str) -> bool {
    let digits = word.strip_suffix('s').unwrap_or(word);
    digits.len() == 4
        && digits.chars().all(|c| c.is_ascii_digit())
        && digits.parse::<u32>().is_ok_and(|y| (1000..=2099).contains(&y))
}

fn has_phrase(w: &[&str], phrase: &[&str]) -> bool {
    w.windows(phrase.len()).any(|win| win == phrase)
}

/// A clause marked historical: "in 1889", "since then", "years ago", "last year".
fn historical(w: &[&str]) -> bool {
    w.iter().enumerate().any(|(i, t)| {
        let next = w.get(i + 1).copied().unwrap_or("");
        matches!(*t, "centuries" | "century" | "decades" | "historically" | "originally")
            || (*t == "years" && next == "ago")
            || (matches!(*t, "in" | "since" | "by" | "until" | "from" | "of" | "around") && is_year(next))
            || (*t == "since" && next == "then")
            || (*t == "back" && next == "then")
            || (*t == "last" && matches!(next, "year" | "century" | "decade"))
            || (*t == "the" && next == "past")
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Subject {
    /// None found: a bare participle ("Sent!") or an elided subject.
    None,
    /// The assistant: "I", "we".
    First,
    /// The user: "you".
    Second,
    /// He, she, they, someone.
    Third,
    /// It, that, this, everything.
    Thing,
    /// A noun phrase ending at `head`.
    Noun { head: usize },
    /// "Nothing", "none": negated.
    Negative,
}

/// What precedes one verb in its clause: its subject and the verb group between them.
#[derive(Clone, Copy, Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "four independent facts about one verb group, each read on its own"
)]
struct Context {
    subject: Subject,
    /// An auxiliary (be/have/get, or a contracted one) stands between subject and verb.
    aux: bool,
    /// The auxiliary is a form of "be" or "been": passive.
    passive: bool,
    /// The auxiliary is past ("was sent", "had been sent"), not present ("is", "has been").
    past: bool,
    /// A negator, modal, infinitive, conditional, habitual or wish governs the verb.
    governed: bool,
}

/// How a subject pronoun reads, with the auxiliary a contraction carries.
fn pronoun(word: &str) -> Option<(Subject, bool, bool)> {
    // (subject, has an auxiliary, passive-capable "be")
    Some(match word {
        "i" | "we" => (Subject::First, false, false),
        "i've" | "we've" | "i'd" | "we'd" => (Subject::First, true, false),
        "i'm" | "we're" => (Subject::First, true, true),
        "you" => (Subject::Second, false, false),
        "you've" | "you'd" => (Subject::Second, true, false),
        "you're" => (Subject::Second, true, true),
        "he" | "she" | "they" | "someone" | "somebody" | "everyone" | "everybody" => (Subject::Third, false, false),
        "they've" | "they'd" | "he'd" | "she'd" => (Subject::Third, true, false),
        "he's" | "she's" | "they're" => (Subject::Third, true, true),
        "it" | "that" | "this" | "everything" | "something" | "these" | "those" => (Subject::Thing, false, false),
        "it's" | "that's" | "everything's" | "something's" | "it'd" | "that'd" => (Subject::Thing, true, true),
        _ => return None,
    })
}

/// Scans back from just before `end` to the verb's subject, collecting the verb group.
fn scan_back(w: &[&str], end: usize) -> Context {
    let mut ctx = Context {
        subject: Subject::None,
        aux: false,
        passive: false,
        past: false,
        governed: false,
    };
    let mut present = false;
    let mut j = end;
    while j > 0 {
        j -= 1;
        let t = w[j];
        if is(t, NEG_SUBJECTS) {
            ctx.subject = Subject::Negative;
            ctx.governed = true;
            return ctx;
        }
        if let Some((subject, aux, passive)) = pronoun(t) {
            // "I've got it sent", "I had it delivered": the pronoun is an object; keep going.
            if is(t, OBJECT_PRONOUNS) && j > 0 && (is(w[j - 1], GET) || is(w[j - 1], HAVE)) {
                continue;
            }
            // "I'd have sent it": "would have", not "had".
            if matches!(
                t,
                "i'd" | "we'd" | "you'd" | "it'd" | "that'd" | "they'd" | "he'd" | "she'd"
            ) && w.get(j + 1) == Some(&"have")
            {
                ctx.governed = true;
            }
            ctx.subject = subject;
            ctx.aux |= aux;
            ctx.passive |= passive;
            ctx.past &= !(present || aux);
            ctx.governed |= governs_subject(w, j, j);
            return ctx;
        }
        if is(t, MODALS) || t == "to" {
            ctx.governed = true;
            continue;
        }
        if is(t, NEGATORS) || is(t, HABITUAL) {
            ctx.governed = true;
            continue;
        }
        if is(t, BE) {
            ctx.aux = true;
            ctx.passive = true;
            ctx.past |= matches!(t, "was" | "were");
            present |= matches!(t, "is" | "are" | "am");
            continue;
        }
        if is(t, HAVE) || is(t, GET) {
            ctx.aux = true;
            ctx.past |= t == "had";
            present |= matches!(t, "has" | "have");
            continue;
        }
        if is(t, ADVERBS) {
            continue;
        }
        if is(t, CONDITIONAL) {
            // "once sent": nothing between the conditional and the verb but the verb group.
            ctx.governed = true;
            return ctx;
        }
        if is(t, BARE_FILLER) && w[..j].iter().all(|x| is(x, BARE_FILLER)) {
            return ctx;
        }
        // A noun phrase ends here; find where it starts.
        let mut start = j;
        while start > 0 && (is(w[start - 1], DETERMINERS) || is_number(w[start - 1]) || is(w[start - 1], TASK_NOUNS)) {
            start -= 1;
        }
        ctx.subject = Subject::Noun { head: j };
        ctx.past &= !present;
        ctx.governed |= governs_subject(w, start, j);
        return ctx;
    }
    ctx.past &= !present;
    ctx
}

/// Whether a word just before (or inside) the subject spanning `start..=head` governs the verb:
/// "if it's sent", "once the invoice is paid", "no message was sent", "want it sent".
fn governs_subject(w: &[&str], start: usize, head: usize) -> bool {
    if w[start..=head].iter().any(|t| is(t, NEGATORS)) {
        return true;
    }
    let Some(before) = start.checked_sub(1).map(|i| w[i]) else {
        return false;
    };
    is(before, CONDITIONAL)
        || is(before, HABITUAL)
        || is(before, NEGATORS)
        || (is(before, DESIRE) && (start == head) && (is(w[head], OBJECT_PRONOUNS) || head > start))
        || (is(before, DESIRE) && is(w[head], OBJECT_PRONOUNS))
}

/// The context of a clause's first verb group, inherited by a following clause that has a verb
/// but no subject of its own: "I've written the email and sent it".
fn lead_context(w: &[&str]) -> Option<Context> {
    let p = w.iter().position(|t| {
        is(t, BE) || is(t, HAVE) || is(t, GET) || is(t, MODALS) || verb(t).is_some() || pronoun(t).is_some_and(|p| p.1)
    })?;
    let end = if verb(w[p]).is_some() { p } else { p + 1 };
    let ctx = scan_back(w, end);
    (ctx.subject != Subject::None || ctx.governed).then_some(ctx)
}

/// "is called a haiku", "called it a day": naming, not phoning.
fn naming(w: &[&str], i: usize) -> bool {
    let next = w.get(i + 1).copied().unwrap_or("");
    let after = w.get(i + 2).copied().unwrap_or("");
    let be_before = i > 0 && (is(w[i - 1], BE) || matches!(w[i - 1], "it's" | "that's" | "he's" | "she's"));
    (be_before && !next.is_empty() && !is(next, AFTER_PARTICIPLE))
        || (is(next, OBJECT_PRONOUNS) && matches!(after, "a" | "an" | "the" | "quits"))
        || is(next, &["a", "an"])
}

fn participle_position(w: &[&str], i: usize) -> bool {
    w.get(i + 1)
        .is_none_or(|next| is(next, AFTER_PARTICIPLE) || is_number(next))
}

/// A clause-initial "-ing" form that is the subject of a later verb: "Sending money needs …".
fn gerund_subject(w: &[&str], i: usize) -> bool {
    w[i + 1..].iter().any(|t| {
        is(t, BE)
            || is(t, MODALS)
            || matches!(
                *t,
                "needs" | "requires" | "takes" | "costs" | "means" | "works" | "has" | "is" | "involves" | "helps"
            )
    })
}

struct Clause<'a> {
    words: Vec<&'a str>,
    join: Join,
    /// Inside quotation marks or after a framing colon ("Here's a draft: …"): a draft's
    /// words, not the assistant's, so disclosed at most.
    quoted: bool,
}

/// How a clause joins the previous one, and what it inherits when it has no subject of its own.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Join {
    /// The sentence's first clause, or the first after a quotation mark or a colon.
    Start,
    /// A comma, semicolon or dash: "I've paid the bill, booked the table" (subject and governors).
    Punctuation,
    /// "and", "or": "I haven't paid it and sent it" (subject and governors).
    And,
    /// "but", "so": "I didn't pay it but sent it" (the subject only).
    But,
}

fn words(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for w in text
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .map(|w| w.trim_matches('\''))
        .filter(|w| !w.is_empty())
    {
        // "The alarm's set", "Mom's been called": a noun's "'s" is read as "is" (or "has").
        match w.strip_suffix("'s") {
            Some(stem) if pronoun(w).is_none() && !stem.is_empty() => {
                out.push(stem);
                out.push("is");
            }
            _ => out.push(w),
        }
    }
    out
}

fn clauses(sentence: &str) -> Vec<Clause<'_>> {
    let mut out = Vec::new();
    let mut join = Join::Start;
    let (mut in_quotes, mut framed) = (false, false);
    let mut rest = sentence;
    loop {
        let cut = rest.find([',', ';', ':', '—', '–', '(', ')', '"']);
        let (part, separator) = match cut {
            Some(at) => {
                let c = rest[at..].chars().next().unwrap_or(',');
                (&rest[..at], Some(c))
            }
            None => (rest, None),
        };
        let mut current: Vec<&str> = Vec::new();
        let mut current_join = join;
        for w in words(part) {
            let split = match w {
                "and" | "or" => Some(Join::And),
                "but" | "so" => Some(Join::But),
                _ => None,
            };
            if let Some(next_join) = split {
                if !current.is_empty() {
                    out.push(Clause {
                        words: std::mem::take(&mut current),
                        join: current_join,
                        quoted: in_quotes || framed,
                    });
                }
                current_join = next_join;
            } else {
                current.push(w);
            }
        }
        if !current.is_empty() {
            out.push(Clause {
                words: current,
                join: current_join,
                quoted: in_quotes || framed,
            });
        }
        let Some(c) = separator else { break };
        join = Join::Punctuation;
        match c {
            '"' => {
                in_quotes = !in_quotes;
                join = Join::Start;
            }
            ':' => {
                framed |= words(part).iter().any(|w| is(w, FRAMES));
                join = Join::Start;
            }
            _ => {}
        }
        rest = &rest[cut.unwrap_or(rest.len()) + c.len_utf8()..];
    }
    out
}

fn is_question(w: &[&str], last_of_question: bool) -> bool {
    let first = w.first().copied().unwrap_or("");
    last_of_question || (is(first, INVERTED) && w.len() > 1 && (pronoun(w[1]).is_some() || is(w[1], DETERMINERS)))
}

/// Splits after `.`, `!`, `?` or `…` followed by whitespace or the end; each part keeps whether
/// it was a question.
fn sentences(text: &str) -> Vec<(&str, bool)> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut iter = text.char_indices().peekable();
    while let Some((i, c)) = iter.next() {
        if matches!(c, '.' | '!' | '?' | '…') {
            let end = i + c.len_utf8();
            let at_break = iter.peek().is_none_or(|(_, n)| n.is_whitespace() || *n == '"');
            if at_break {
                let part = &text[start..end];
                if !part.trim().is_empty() {
                    out.push((part, c == '?'));
                }
                start = end;
            }
        }
    }
    if !text[start..].trim().is_empty() {
        out.push((&text[start..], false));
    }
    out
}

/// Opens an informational request ("What is…", "Tell me…"): unless it names an action or a
/// task, the reply to it needs no turn-level disclosure.
const INFORMATIONAL: &[&str] = &[
    "what",
    "what's",
    "whats",
    "when",
    "when's",
    "where",
    "where's",
    "who",
    "who's",
    "whose",
    "why",
    "how",
    "how's",
    "which",
    "is",
    "are",
    "was",
    "were",
    "do",
    "does",
    "can",
    "could",
    "would",
    "will",
    "should",
    "tell",
    "explain",
    "describe",
    "define",
    "summarize",
    "summarise",
    "translate",
    "compare",
    "list",
    "show",
    "give",
    "help",
    "write",
    "draft",
    "suggest",
    "recommend",
];

/// Whether a heard request may ask for something to be done on the user's behalf, so that a
/// reply to it may claim it was done. Fails closed: only a question or an informational request
/// that names no action verb (in any form) and no task, and is not phrased as a request ("could
/// you…", "will you…", "please…"), counts as informational; any other request, statements
/// included, may ask for an action. A turn that answers such a request ends
/// with the turn-level disclosure (D-027).
pub fn request_may_ask_for_action(heard: &str) -> bool {
    let lowered = heard.to_lowercase().replace(['\u{2019}', '\u{2018}'], "'");
    let w = words(&lowered);
    let names_action = w.iter().enumerate().any(|(i, t)| {
        // "tell me", "remind me" differ: telling the user is what the assistant does.
        if matches!(*t, "tell" | "telling") && matches!(w.get(i + 1), Some(&("me" | "us"))) {
            return false;
        }
        is(t, TASK_NOUNS) || is(t, FOREIGN) || action_form(t)
    });
    // "Did you…", "Have you…", "Has it been…": asking whether something was done.
    let asks_if_done = w.windows(2).any(|p| {
        matches!(p[0], "did" | "have" | "has" | "had") && matches!(p[1], "you" | "it" | "that" | "they" | "she" | "he")
    });
    // A request phrased as a question, whatever its verb: "could you sort out the plumber?",
    // "will you…", "any chance you…", "do you mind…", "please…".
    let polite = w.windows(2).any(|p| {
        matches!(
            p[0],
            "can" | "could" | "would" | "will" | "won't" | "wouldn't" | "can't" | "couldn't"
        ) && matches!(p[1], "you" | "u")
    }) || w.iter().any(|t| matches!(*t, "please" | "pls"))
        || has_phrase(&w, &["any", "chance"])
        || w.first() == Some(&"mind")
        || has_phrase(&w, &["you", "mind"]);
    if names_action || asks_if_done || polite {
        return true;
    }
    let question = lowered.trim_end().ends_with('?');
    let informational = question || w.first().is_some_and(|f| is(f, INFORMATIONAL));
    !informational
}

/// Any form of an action verb: its base, third person, past, participle or progressive.
fn action_form(word: &str) -> bool {
    let stem = |suffix: &str| word.strip_suffix(suffix);
    verb(word).is_some()
        || VERBS.iter().any(|e| {
            word == e.lemma
                || stem("s") == Some(e.lemma)
                || stem("es") == Some(e.lemma)
                || (e.lemma.ends_with('y') && stem("ies").is_some_and(|s| e.lemma.strip_suffix('y') == Some(s)))
        })
}

/// Classifies one reply sentence (or several: each is judged and the strongest verdict wins).
pub fn classify(text: &str) -> Verdict {
    let lowered = text
        .to_lowercase()
        .replace(['\u{2019}', '\u{2018}', '`'], "'")
        .replace(['\u{201c}', '\u{201d}', '«', '»'], "\"");
    sentences(&lowered)
        .into_iter()
        .map(|(s, question)| classify_sentence(s, question))
        .max()
        .unwrap_or(Verdict::Plain)
}

/// Whether a sentence would be replaced or disclosed: it may claim that an action happened.
pub fn claims_completion(text: &str) -> bool {
    classify(text) != Verdict::Plain
}

fn classify_sentence(sentence: &str, question: bool) -> Verdict {
    let all = words(sentence);
    let hearsay = HEARSAY.iter().any(|p| has_phrase(&all, p));
    let clauses = clauses(sentence);
    let last = clauses.len().saturating_sub(1);
    let mut verdict = Verdict::Plain;
    let mut previous: Option<Context> = None;
    for (n, clause) in clauses.iter().enumerate() {
        let w = &clause.words;
        let inherited = match clause.join {
            Join::Start => None,
            Join::Punctuation | Join::And => previous,
            Join::But => previous.map(|c| Context { governed: false, ..c }),
        };
        if !is_question(w, question && n == last) {
            let v = classify_clause(w, inherited, hearsay);
            verdict = verdict.max(if clause.quoted { v.min(Verdict::Disclose) } else { v });
        }
        previous = lead_context(w).or(inherited);
    }
    verdict
}

fn classify_clause(w: &[&str], inherited: Option<Context>, hearsay: bool) -> Verdict {
    if w.iter().any(|t| is(t, FOREIGN)) {
        return Verdict::Replace;
    }
    let mut verdict = phrases(w);
    // "Transfer complete", "Payment of $42 to Priya complete".
    if let Some(last) = w.last()
        && matches!(*last, "complete" | "completed" | "done" | "finished")
        && w.iter().any(|t| is(t, TASK_NOUNS) || is_number(t))
        && !w[..w.len() - 1].iter().any(|t| is(t, NEGATORS))
        && !w[w.len().saturating_sub(3)..w.len() - 1]
            .iter()
            .any(|t| is(t, MODALS) || *t == "to")
    {
        verdict = Verdict::Replace;
    }
    for i in 0..w.len() {
        if let Some(v) = verb(w[i]) {
            verdict = verdict.max(judge(w, i, v, inherited, hearsay));
        }
    }
    verdict
}

/// Completion phrases and possession claims.
fn phrases(w: &[&str]) -> Verdict {
    let mut verdict = Verdict::Plain;
    let at = |phrase: &[&str]| w.windows(phrase.len()).position(|win| win == phrase);
    for phrase in [&["taken", "care", "of"][..], &["all", "set"]] {
        if let Some(i) = at(phrase) {
            let ctx = scan_back(w, i);
            // "You've taken care of it": the user's doing.
            let users = ctx.subject == Subject::Second && !ctx.passive && phrase[0] == "taken";
            if !(ctx.governed || users) {
                verdict = Verdict::Replace;
            }
        }
    }
    for phrase in [
        &["on", "its", "way"][..],
        &["on", "the", "way"],
        &["on", "their", "way"],
        &["went", "out"],
        &["gone", "out"],
        &["went", "through"],
        &["gone", "through"],
        &["got", "through"],
    ] {
        if let Some(i) = at(phrase) {
            let ctx = scan_back(w, i);
            if ctx.governed {
                continue;
            }
            let taskish = match ctx.subject {
                Subject::Thing => true,
                Subject::Noun { head } => is(w[head], TASK_NOUNS) || is_number(w[head]),
                _ => false,
            };
            if taskish {
                verdict = Verdict::Replace;
            } else if phrase[0] == "on" && !matches!(ctx.subject, Subject::Second | Subject::First) {
                verdict = verdict.max(Verdict::Disclose);
            }
        }
    }
    // "Consider it done", "consider it sent".
    if let Some(i) = at(&["consider", "it"]).or_else(|| at(&["consider", "that"]))
        && w.get(i + 2).is_some_and(|t| verb(t).is_some() || *t == "taken")
    {
        verdict = Verdict::Replace;
    }
    // "I let her know", "I've let Priya know".
    if let Some(i) = w.iter().position(|t| *t == "let")
        && w[i + 1..].iter().take(3).any(|t| *t == "know")
        && w.get(i + 1) != Some(&"me")
        && w.get(i + 1) != Some(&"us")
    {
        let ctx = scan_back(w, i);
        if ctx.subject == Subject::First && !ctx.governed {
            verdict = Verdict::Replace;
        }
    }
    // "Priya now has the message": a delivery stated as possession.
    for (i, t) in w.iter().enumerate() {
        if matches!(*t, "has" | "have" | "got")
            && w[i + 1..]
                .iter()
                .take(3)
                .find(|x| !matches!(**x, "now" | "already" | "the" | "your" | "my" | "her" | "his" | "their"))
                .is_some_and(|x| is(x, TASK_STRONG) || matches!(*x, "message" | "money" | "invite" | "email" | "text"))
        {
            let ctx = scan_back(w, i);
            if !ctx.governed && matches!(ctx.subject, Subject::Third | Subject::Noun { .. }) {
                verdict = verdict.max(Verdict::Disclose);
            }
        }
    }
    // "The lights are off now", "Reminder's on for 9": a device or reminder put in a state.
    for (i, t) in w.iter().enumerate() {
        if matches!(*t, "on" | "off")
            && i >= 2
            && is(w[i - 1], BE)
            && is(w[i - 2], DEVICES)
            && !w[..i]
                .iter()
                .any(|x| is(x, NEGATORS) || is(x, MODALS) || is(x, CONDITIONAL))
        {
            verdict = verdict.max(Verdict::Disclose);
        }
    }
    verdict
}

/// Uses of an action verb's form that are not actions: "turned cold", "is called a haiku",
/// "as I told you", "well done", "made a list", "done when the edges are golden".
fn other_sense(w: &[&str], i: usize, v: Verb) -> bool {
    let next = w.get(i + 1).copied();
    let strong = matches!(v.class, Class::Acts | Class::Completion | Class::TaskOnly);
    PARTICLES
        .iter()
        .find(|(lemma, _)| *lemma == v.lemma)
        .is_some_and(|(_, particles)| !next.is_some_and(|n| is(n, particles)))
        || (v.lemma == "call" && naming(w, i))
        || (v.lemma == "tell" && matches!(next, Some("you" | "me")))
        || (v.lemma == "do" && i > 0 && w[i - 1] == "well")
        || (v.class == Class::TaskOnly && !w.iter().any(|t| is(t, TASK_STRONG)))
        || ((!strong || v.class == Class::Completion)
            && next.is_some_and(|n| matches!(n, "when" | "once" | "if" | "whenever")))
}

fn judge(w: &[&str], i: usize, v: Verb, inherited: Option<Context>, hearsay: bool) -> Verdict {
    let next = w.get(i + 1).copied();
    if other_sense(w, i, v) {
        return Verdict::Plain;
    }
    let strong = matches!(v.class, Class::Acts | Class::Completion | Class::TaskOnly);
    // Attributive: "the paid version", "your scheduled meeting".
    if i > 0 && is(w[i - 1], ARTICLES) {
        return Verdict::Plain;
    }
    if i > 0 && is(w[i - 1], POSSESSIVES) {
        return if v.class == Class::Acts {
            Verdict::Disclose
        } else {
            Verdict::Plain
        };
    }
    let mut ctx = scan_back(w, i);
    if ctx.subject == Subject::None
        && !ctx.governed
        && let Some(prev) = inherited
    {
        ctx = Context {
            aux: ctx.aux || prev.aux,
            passive: ctx.passive || prev.passive,
            ..prev
        };
        if let Subject::Noun { .. } = ctx.subject {
            // The inherited head indexes the previous clause; it counts as a task subject.
            ctx.subject = Subject::Noun { head: usize::MAX };
        }
    }
    if ctx.governed {
        return Verdict::Plain;
    }
    let task = w.iter().any(|t| is(t, TASK_NOUNS) || is_number(t));
    // The verdict for a claim-shaped construction, by verb class.
    let claim = match v.class {
        Class::Acts | Class::Completion | Class::TaskOnly => Verdict::Replace,
        Class::General if task => Verdict::Replace,
        Class::General | Class::State => Verdict::Disclose,
    };
    let possible = if strong { Verdict::Disclose } else { Verdict::Plain };
    if v.form == Form::Ing {
        let asserted = match ctx.subject {
            Subject::First => ctx.passive,
            Subject::None => !gerund_subject(w, i),
            Subject::Thing => ctx.aux,
            _ => false,
        };
        return match (asserted, ctx.subject) {
            (false, _) => Verdict::Plain,
            (true, Subject::Thing) => Verdict::Disclose,
            (true, _) if v.class == Class::Acts => Verdict::Replace,
            (true, _) => Verdict::Disclose,
        };
    }
    let base = match ctx.subject {
        Subject::Negative => return Verdict::Plain,
        // The user's own action ("you told me you sent it"), unless passive ("you're booked").
        Subject::Second if !ctx.passive => return Verdict::Plain,
        // The assistant's own report ("I've added milk to your list"), or the user as the one
        // acted on ("you're booked for 7").
        Subject::First | Subject::Second => claim,
        Subject::Thing if ctx.aux => claim,
        // Active: "that changed everything".
        Subject::Thing => possible,
        Subject::Third if !ctx.passive => possible,
        Subject::Third | Subject::Noun { .. } => {
            if !(ctx.aux || participle_position(w, i)) {
                // Active with an object: "Priya sent the money".
                possible
            } else if ctx.past && !task {
                // "Many letters were sent during the war": a past passive about something
                // other than a task is a possible claim at most.
                Verdict::Disclose
            } else {
                claim
            }
        }
        Subject::None => {
            // "Set a reminder for 9": an imperative, not a headline.
            if matches!(w[i], "set" | "complete" | "reset")
                && next.is_some_and(|n| is(n, DETERMINERS) || pronoun(n).is_some())
            {
                return Verdict::Plain;
            }
            if v.class == Class::State {
                Verdict::Disclose
            } else {
                Verdict::Replace
            }
        }
    };
    // Softeners: a historical marker, a habit after the verb, or reported speech.
    let first = ctx.subject == Subject::First;
    if historical(w) && !(first && v.class == Class::Acts) {
        return Verdict::Plain;
    }
    if !strong && w.iter().any(|t| is(t, HABIT_AFTER)) {
        return Verdict::Plain;
    }
    if hearsay && !first {
        return Verdict::Plain;
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_negations_and_futures_are_plain() {
        for s in [
            "Have you sent it?",
            "Nothing was sent.",
            "It will be sent once you approve.",
            "I didn't pay it.",
            "The payment needs to be complete.",
            "I'd have sent it, but I can't.",
        ] {
            assert_eq!(classify(s), Verdict::Plain, "{s}");
        }
    }

    #[test]
    fn session_texts_are_plain() {
        assert_eq!(classify(DRAFT_DISCLAIMER), Verdict::Plain);
        assert_eq!(classify(DRAFT_DISCLOSURE), Verdict::Plain);
    }

    #[test]
    fn a_claim_before_a_tag_question_is_still_a_claim() {
        assert_eq!(
            classify("I've booked the table for 7, anything else?"),
            Verdict::Replace
        );
        assert_eq!(
            classify("I've sent it to Priya. Is there anything else?"),
            Verdict::Replace
        );
    }
}
