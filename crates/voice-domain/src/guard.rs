//! The completion-claim rule (`docs/END_GOAL.md`, invariant 2). A reply is *drafted text*: a
//! sentence that says an action happened — sent, paid, booked, notified, scheduled, cancelled,
//! ordered, done — is spoken as it is only when this turn's ledger holds executed evidence (the
//! session checks that). No tool can run in the first release, so the rule fails closed: every
//! sentence that might assert a completed action gets a disclosure, and only sentences that
//! assert nothing of the kind are spoken untouched.
//!
//! [`classify`] gives one of three verdicts per sentence, and the deciding question is the
//! completion's *agent* (D-028): only a completion attributed to the assistant is a claim —
//! explicitly ("I've texted Priya") or implied, where an agentless change lands on the user's
//! remit — the people, things and arrangements this assistant manages ("she's been notified",
//! "your repeat's gone through", "your nine is now a ten"). A completion attributed to anyone
//! else is not the assistant's claim: a named third party ("Priya sent the money"), quoted or
//! reported speech ("she said she'd paid it", `He wrote: "…"`), the world's machinery ("the
//! parcel was delivered", "your card was declined") or the user ("you've already paid it").
//! - [`Verdict::Replace`] — it says an action happened: a past, perfect, passive or headline
//!   form of an action verb attributed to the assistant, explicit or implied ("I've texted
//!   Priya", "Priya has been notified", "$50 sent to Priya", "Sent!"), a completion phrase
//!   ("on its way", "went out", "consider it done") on a remit subject, or "I'm sending it
//!   now". The session speaks [`DRAFT_DISCLAIMER`] instead.
//! - [`Verdict::Disclose`] — it might say so: a less certain form, such as a general verb with
//!   a pronoun subject ("It's set for 9") or a completion whose agent cannot be resolved
//!   ("it's on its way", "it was dispatched", "the letter reached her"). The session speaks
//!   it as written, and its turn ends with [`DRAFT_DISCLOSURE`].
//! - [`Verdict::Plain`] — no completed action is asserted, or the completion is someone
//!   else's: spoken as written.
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
//! [`DRAFT_DISCLOSURE`]. And because a request that asked for no action cannot produce an
//! evidenced claim, sentences on an informational turn also pass the recall-tilted
//! [`uninvited_action`] screen: a possible claim is still spoken, but its turn ends disclosed.
//! What was spoken is ledgered, so a slip is visible.
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
/// Hypotheticals — the embedded event is not asserted.
const CONDITIONAL: &[&str] = &["if", "unless", "whether"];
/// Temporal subordinators order events in time; they are not hypotheticals. An
/// episodic completed form under one is asserted ("after the 30,000 I used" —
/// the use happened); a present or generic reading stays governed ("once the
/// reminder is in, it stays").
const TEMPORAL: &[&str] = &["once", "when", "whenever", "after", "before", "until", "till"];
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
    "sprinkler",
    "sprinklers",
    "kettle",
    "kettles",
    "plug",
    "plugs",
    "charger",
    "chargers",
    "ac",
    "aircon",
    "dehumidifier",
    "purifier",
    "vacuum",
    "mower",
    "blinds",
    "curtains",
    "shutters",
    "garage",
    "gate",
    "lock",
    "locks",
    "door",
    "doors",
    "camera",
    "cameras",
    "speaker",
    "speakers",
    "router",
    "wifi",
    "lamp",
    "lamps",
    "hob",
    "grill",
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
/// Speech and reporting verbs: when a third party or the user is their subject, the words that
/// follow are the reporter's, not the assistant's ("she said…", "tracking says…").
const REPORT_VERBS: &[&str] = &[
    "say",
    "says",
    "said",
    "tell",
    "tells",
    "told",
    "write",
    "writes",
    "wrote",
    "written",
    "read",
    "reads",
    "report",
    "reports",
    "reported",
    "claim",
    "claims",
    "claimed",
    "reckon",
    "reckons",
    "reckoned",
    "mention",
    "mentions",
    "mentioned",
    "state",
    "states",
    "stated",
    "reply",
    "replies",
    "replied",
    "respond",
    "responds",
    "responded",
    "text",
    "texts",
    "texted",
    "message",
    "messages",
    "messaged",
    "email",
    "emails",
    "emailed",
    "post",
    "posts",
    "posted",
    "note",
    "notes",
    "noted",
    "add",
    "adds",
    "added",
    "insist",
    "insists",
    "insisted",
    "confirm",
    "confirms",
    "confirmed",
    "admit",
    "admits",
    "admitted",
    "boast",
    "boasts",
    "boasted",
    "complain",
    "complains",
    "complained",
    "explain",
    "explains",
    "explained",
    "suggest",
    "suggests",
    "suggested",
    "announce",
    "announces",
    "announced",
    "reveal",
    "reveals",
    "revealed",
    "think",
    "thinks",
    "thought",
    "figure",
    "figures",
    "figured",
    "hear",
    "hears",
    "heard",
    "expect",
    "expects",
    "expected",
    "warn",
    "warns",
    "warned",
    "remind",
    "reminds",
    "reminded",
    "ask",
    "asks",
    "asked",
    "advise",
    "advises",
    "advised",
    "swear",
    "swears",
    "swore",
    "promise",
    "promises",
    "promised",
    "phone",
    "phones",
    "phoned",
    "call",
    "calls",
    "called",
    "rang",
];
/// The user's remit beyond `TASK_NOUNS`, `DEVICES` and `DELIVERABLES`: things this assistant
/// manages whose change an agentless report attributes to it.
const REMIT_NOUNS: &[&str] = &[
    "repeat",
    "repeats",
    "prescription",
    "prescriptions",
    "refill",
    "refills",
    "medication",
    "medications",
    "medicine",
    "dose",
    "doses",
    "bet",
    "bets",
    "wager",
    "wagers",
    "stake",
    "stakes",
    "slot",
    "slots",
    "schedule",
    "schedules",
    "itinerary",
    "itineraries",
    "membership",
    "memberships",
    "voucher",
    "vouchers",
    "place",
    "places",
    "spot",
    "spots",
    "rota",
    "rotas",
    "roster",
    "rosters",
    "diary",
    "diaries",
    "shift",
    "shifts",
    "savings",
    "letters",
    "boiler",
    "boilers",
    "furnace",
    "furnaces",
    "radiator",
    "radiators",
    "dishwasher",
    "dishwashers",
    "fridge",
    "freezer",
    "oven",
    "ovens",
    "cooker",
    "cookers",
    "washing",
    "laundry",
    "lawn",
    "lawns",
    "hedge",
    "hedges",
    "garden",
    "gardens",
    "gutter",
    "gutters",
    "chimney",
    "chimneys",
    "fence",
    "fences",
    "recording",
    "recordings",
];
/// People an assistant contacts on the user's behalf; an agentless report about one ("the
/// gardener's been asked") reads as the assistant's doing.
const CONTACTS: &[&str] = &[
    "gardener",
    "gardeners",
    "cleaner",
    "cleaners",
    "plumber",
    "plumbers",
    "electrician",
    "electricians",
    "decorator",
    "decorators",
    "handyman",
    "builder",
    "builders",
    "mechanic",
    "mechanics",
    "dentist",
    "dentists",
    "doctor",
    "doctors",
    "vet",
    "vets",
    "hairdresser",
    "hairdressers",
    "barber",
    "barbers",
    "landlord",
    "landlords",
    "landlady",
    "solicitor",
    "solicitors",
    "accountant",
    "accountants",
    "nanny",
    "nannies",
    "babysitter",
    "babysitters",
    "sitter",
    "tutor",
    "tutors",
    "instructor",
    "instructors",
    "carer",
    "carers",
    "nurse",
    "nurses",
    "physio",
    "physios",
    "engineer",
    "engineers",
    "technician",
    "technicians",
    "plasterer",
    "plasterers",
    "roofer",
    "roofers",
    "caterer",
    "caterers",
    "driver",
    "drivers",
    "chauffeur",
    "surgeon",
    "surgeons",
    "hygienist",
    "optician",
    "opticians",
    "tradesman",
    "workman",
    "workmen",
];
/// The user's own people: "your mum's been told", "she knows you're coming".
const RELATIONS: &[&str] = &[
    "mum",
    "mom",
    "mother",
    "dad",
    "father",
    "parent",
    "parents",
    "sister",
    "sisters",
    "brother",
    "brothers",
    "wife",
    "husband",
    "partner",
    "son",
    "sons",
    "daughter",
    "daughters",
    "aunt",
    "auntie",
    "uncle",
    "uncles",
    "cousin",
    "cousins",
    "grandma",
    "grandpa",
    "grandmother",
    "grandfather",
    "nan",
    "granny",
    "gran",
    "niece",
    "nephew",
    "boss",
    "neighbour",
    "neighbours",
    "neighbor",
    "neighbors",
    "friend",
    "friends",
    "flatmate",
    "flatmates",
    "roommate",
    "roommates",
    "colleague",
    "colleagues",
    "client",
    "clients",
    "boyfriend",
    "girlfriend",
    "fiance",
    "fiancee",
];
/// Places the user's things can be put: "the voucher's in your wallet".
const SPACE: &[&str] = &[
    "wallet",
    "wallets",
    "account",
    "accounts",
    "inbox",
    "inboxes",
    "mailbox",
    "mailboxes",
    "calendar",
    "calendars",
    "diary",
    "diaries",
    "list",
    "lists",
    "phone",
    "phones",
    "folder",
    "folders",
    "basket",
    "baskets",
    "library",
    "app",
    "apps",
    "bag",
    "bags",
    "drawer",
    "drawers",
    "purse",
    "handbag",
    "locker",
    "lockers",
];
/// Spaces the assistant can deliver into on its own account: "the sheet is in your inbox",
/// "the ticket's in the LNER app", "the directions are on your phone". A possessive or any
/// determiner is enough — where it is implies who put it there.
const DIGITAL_SPACE: &[&str] = &[
    "inbox",
    "inboxes",
    "mailbox",
    "mailboxes",
    "app",
    "apps",
    "phone",
    "phones",
    "account",
    "accounts",
    "calendar",
    "calendars",
    "diary",
    "diaries",
    "wallet",
    "wallets",
    "message",
    "messages",
    "text",
    "texts",
    "email",
    "emails",
    "mail",
    "drive",
    "drives",
    "file",
    "files",
    "folder",
    "folders",
    "download",
    "downloads",
    "note",
    "notes",
    "reminder",
    "reminders",
    "notification",
    "notifications",
    "basket",
    "baskets",
    "cart",
    "carts",
    "list",
    "lists",
    "watchlist",
    "library",
    "photos",
    "gallery",
    "cloud",
    "tablet",
    "tablets",
    "laptop",
    "laptops",
    "computer",
    "computers",
    "desktop",
    "nav",
    "satnav",
    "gps",
    "maps",
];
/// Words whose "says" is another's words, not the assistant's: "her email says…", "the
/// app says…", "the tracking says…". `nounish` cannot name them — they double as report
/// frames — so a report verb needs them listed on its own.
const REPORT_NOUNS: &[&str] = &[
    "email",
    "emails",
    "text",
    "texts",
    "message",
    "messages",
    "letter",
    "letters",
    "note",
    "notes",
    "post",
    "posts",
    "reply",
    "replies",
    "memo",
    "memos",
    "invoice",
    "invoices",
    "statement",
    "statements",
    "confirmation",
    "confirmations",
    "notice",
    "notices",
    "notification",
    "notifications",
    "alert",
    "alerts",
    "app",
    "apps",
    "site",
    "sites",
    "website",
    "websites",
    "page",
    "pages",
    "receipt",
    "receipts",
    "form",
    "forms",
    "document",
    "documents",
    "tracking",
    "terms",
    "ad",
    "ads",
    "advert",
    "adverts",
    "flyer",
    "flyers",
    "label",
    "labels",
    "chart",
    "charts",
    "screenshot",
    "screenshots",
    "voicemail",
    "voicemails",
    "recording",
    "recordings",
    "transcript",
    "transcripts",
    "review",
    "reviews",
    "listing",
    "listings",
    "update",
    "updates",
    "feed",
    "feeds",
];
/// A place a charge is *reported* rather than delivered into — "a charge is on your card"
/// is a bill, not a delivery — so it flags only behind a completed delivery verb ("loaded
/// on your card").
const LOAD_SPACES: &[&str] = &[
    "card",
    "cards",
    "statement",
    "statements",
    "bill",
    "bills",
    "tab",
    "tabs",
    "invoice",
    "invoices",
];
/// Quantified things the user can be reported holding: "you've got two nights", "you've
/// got three seats".
const DELIVERABLE_UNITS: &[&str] = &[
    "night", "nights", "day", "days", "week", "weeks", "weekend", "weekends", "room", "rooms", "place", "places",
    "entry", "entries", "spot", "spots", "name", "names", "minute", "minutes", "hour", "hours", "session", "sessions",
];
/// A state an act produces on a device or remit thing: "off", "on", "running", "locked",
/// "sorted". The world's adjectives — weather, time, mood — are never an agentless act.
const ACTIVE_STATES: &[&str] = &[
    "off",
    "on",
    "open",
    "shut",
    "closed",
    "live",
    "armed",
    "locked",
    "unlocked",
    "muted",
    "silent",
    "paused",
    "stopped",
    "started",
    "queued",
    "ready",
    "sorted",
    "set",
    "active",
    "running",
    "ringing",
    "recording",
    "playing",
    "charging",
    "waiting",
    "holding",
];
/// States that double as prepositions or locations — "it's on the Seine" is where the
/// city is, not a change — so they claim only at clause end or before a state adverb
/// ("the kettle's off already", "it's on for tomorrow").
const LOCATIVE_STATES: &[&str] = &[
    "on", "off", "open", "shut", "closed", "live", "armed", "muted", "silent", "paused", "active",
];
const STATE_FOLLOWERS: &[&str] = &[
    "now", "already", "just", "still", "finally", "too", "again", "for", "as", "since", "at", "till", "until",
];
const NATURAL_STATES: &[&str] = &[
    "raining",
    "snowing",
    "drizzling",
    "sunny",
    "windy",
    "cloudy",
    "foggy",
    "hot",
    "cold",
    "warm",
    "cool",
    "dry",
    "wet",
    "humid",
    "fine",
    "bright",
    "dark",
    "early",
    "late",
    "light",
    "clear",
    "busy",
    "quiet",
    "loud",
    "noisy",
    "calm",
    "empty",
    "full",
];
/// What couriers, airlines, banks and venues move or hold — a completion on one is the
/// machinery's status, not the assistant's claim.
const WORLD_CARGO: &[&str] = &[
    "parcel",
    "parcels",
    "package",
    "packages",
    "order",
    "orders",
    "shipment",
    "shipments",
    "item",
    "items",
    "goods",
    "groceries",
    "takeaway",
    "takeaways",
    "refund",
    "refunds",
    "card",
    "cards",
    "flight",
    "flights",
    "train",
    "trains",
    "coach",
    "coaches",
    "courier",
    "couriers",
    "mail",
    "delivery",
    "deliveries",
];
/// Completed forms whose actor is the world's machinery — couriers, banks, airlines — whoever
/// holds the subject. "Sent" stays out: it is the assistant's own verb, so "it's been sent"
/// remains a claim.
const MACHINERY_WORDS: &[&str] = &[
    "delayed",
    "diverted",
    "landed",
    "departed",
    "arrived",
    "expected",
    "grounded",
    "detained",
    "declined",
    "delivered",
    "processed",
    "dispatched",
    "shipped",
    "charged",
    "scheduled",
    "rejected",
    "returned",
    "refunded",
    "confirmed",
    "held",
    "bounced",
    "blocked",
];
/// Days and months that can be a reassigned value: "your meeting is now Friday".
const DAYS_MONTHS: &[&str] = &[
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "tonight",
    "tomorrow",
    "morning",
    "afternoon",
    "evening",
    "midday",
    "noon",
    "midnight",
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];
/// Time-span tokens — "an hour ago", "this morning", "a hundred times" are durations, not
/// objects ("was placed an hour ago" is not "has placed an hour").
const TIME_WORDS: &[&str] = &[
    "hour",
    "hours",
    "minute",
    "minutes",
    "second",
    "seconds",
    "moment",
    "moments",
    "instant",
    "day",
    "days",
    "week",
    "weeks",
    "fortnight",
    "month",
    "months",
    "year",
    "years",
    "decade",
    "decades",
    "century",
    "centuries",
    "morning",
    "mornings",
    "afternoon",
    "afternoons",
    "evening",
    "evenings",
    "night",
    "nights",
    "tonight",
    "midnight",
    "noon",
    "midday",
    "weekend",
    "weekends",
    "while",
    "time",
    "times",
    "ago",
    "earlier",
    "later",
];
const D_STEMS: &[&str] = &[
    "i", "you", "he", "she", "we", "they", "it", "that", "there", "who", "what", "where", "when", "how", "why",
];
/// Auxiliaries folded into a negator: they still count as a finite verb.
const FINITE_NEGATORS: &[&str] = &[
    "can't",
    "cannot",
    "won't",
    "wouldn't",
    "couldn't",
    "shouldn't",
    "mustn't",
    "isn't",
    "aren't",
    "wasn't",
    "weren't",
    "don't",
    "doesn't",
    "didn't",
    "haven't",
    "hasn't",
    "hadn't",
    "ain't",
];
/// After "by", a word that is a channel, a time or a manner — never the agent.
const NON_AGENT: &[&str] = &[
    "me",
    "i",
    "you",
    "myself",
    "yourself",
    "himself",
    "herself",
    "itself",
    "ourselves",
    "themselves",
    "post",
    "mail",
    "email",
    "text",
    "texts",
    "phone",
    "fax",
    "website",
    "online",
    "app",
    "hand",
    "sea",
    "air",
    "road",
    "rail",
    "today",
    "tomorrow",
    "yesterday",
    "tonight",
    "morning",
    "afternoon",
    "evening",
    "night",
    "noon",
    "midnight",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "weekend",
    "week",
    "month",
    "year",
    "now",
    "then",
    "end",
    "far",
    "mistake",
    "accident",
    "chance",
    "default",
    "force",
    "luck",
    "heart",
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
            // "'d" is "had" before a completed form, "been" or "got" ("they'd moved",
            // "they'd been moved", "they'd got it"); before anything else it is "would"
            // — "they'd be moved" is conditional, "I'd have sent it" is "would have".
            if matches!(
                t,
                "i'd" | "we'd" | "you'd" | "it'd" | "that'd" | "they'd" | "he'd" | "she'd"
            ) {
                let next = w[j + 1..].iter().find(|x| !is(x, ADVERBS)).copied().unwrap_or("");
                if !done_shaped(next) && !matches!(next, "been" | "got" | "gotten") {
                    ctx.governed = true;
                }
            }
            ctx.subject = subject;
            ctx.aux |= aux;
            ctx.passive |= passive;
            ctx.past &= !(present || aux);
            ctx.governed |= governs_subject(w, j, j, end, present);
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
        if is(t, CONDITIONAL) || is(t, TEMPORAL) {
            // "if sent", "once sent": nothing between the governor and the verb
            // but the verb group — a result-state or generic reading, governed.
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
        ctx.governed |= governs_subject(w, start, j, end, present);
        return ctx;
    }
    ctx.past &= !present;
    ctx
}

/// Whether `verb` asserts an event under a temporal subordinator: a completed
/// form not marked present — "after I sent it" and "once it was paid" are
/// episodic; "after I send it" and "once it is paid" are prospective or generic.
/// A BE auxiliary makes the participle a result state ("once it's sent"), which
/// is the generic reading and stays governed.
fn episodic(verb: &str, present: bool, be_aux: bool) -> bool {
    done_shaped(verb) && !present && !be_aux
}

/// Whether a word just before (or inside) the subject spanning `start..=head` governs the verb:
/// "if it's sent", "once the invoice is paid", "no message was sent", "want it sent".
fn governs_subject(w: &[&str], start: usize, head: usize, end: usize, present: bool) -> bool {
    if w[start..=head].iter().any(|t| is(t, NEGATORS)) {
        return true;
    }
    let Some(before) = start.checked_sub(1).map(|i| w[i]) else {
        return false;
    };
    let verb = w.get(end).copied().unwrap_or("");
    // "it's sent", "the bill's sent": a BE fused onto the subject counts the same
    // as a separate one — the participle is the result state, not the event.
    let be_aux = w[head].ends_with("'s") || w[head + 1..end.min(w.len())].iter().any(|t| is(t, BE));
    is(before, CONDITIONAL)
        || (is(before, TEMPORAL) && !episodic(verb, present, be_aux))
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
        // A single quote bounds a quotation only at a word edge — "…said 'I've moved
        // it'" — never inside a word ("don't") or on a possessive tail ("users'").
        let mut from = 0;
        let (part, separator) = loop {
            let Some(at) = rest[from..].find([',', ';', ':', '—', '–', '(', ')', '"', '\'']) else {
                break (rest, None);
            };
            let at = from + at;
            let c = rest[at..].chars().next().unwrap_or(',');
            let prev = rest[..at].chars().last();
            let next = rest[at + c.len_utf8()..].chars().next();
            // "2:30", "R3,450": a colon or comma inside a number is the number's
            // own — it does not open a clause.
            if matches!(c, ':' | ',')
                && prev.is_some_and(|p| p.is_ascii_digit())
                && next.is_some_and(|n| n.is_ascii_digit())
            {
                from = at + c.len_utf8();
                continue;
            }
            if c != '\'' {
                break (&rest[..at], Some(c));
            }
            let prev = rest[..at].chars().last();
            let next = rest[at + c.len_utf8()..].chars().next();
            let opens = next.is_some_and(char::is_alphanumeric)
                && prev.is_none_or(|p| p.is_whitespace() || matches!(p, ',' | ';' | ':' | '(' | '"' | '\''));
            let closes = in_quotes
                && prev.is_some_and(char::is_alphanumeric)
                && next.is_none_or(|n| {
                    n.is_whitespace() || matches!(n, '.' | ',' | '!' | '?' | ':' | ';' | ')' | '"' | '\'')
                });
            if opens || closes {
                break (&rest[..at], Some(c));
            }
            from = at + c.len_utf8();
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
            '"' | '\'' => {
                in_quotes = !in_quotes;
                join = Join::Start;
            }
            ':' => {
                // A report verb frames what follows the colon too ("he wrote: 'done'"), even
                // where single quotes leave it outside quotation marks.
                framed |= words(part).iter().any(|w| is(w, FRAMES) || is(w, REPORT_VERBS));
                join = Join::Start;
            }
            _ => {}
        }
        rest = &rest[part.len() + c.len_utf8()..];
    }
    fold_fragments(out)
}

/// A comma-separated span with no verb group of its own is a noun-phrase
/// fragment — an appositive ("their receptionist, Niamh,"), a discourse marker,
/// a list item — so it folds forward into the clause it modifies and the verb
/// binds the whole subject ("Niamh, moved it" — the comma does not orphan
/// "moved"). A trailing fragment keeps its own clause ("…, yeah").
fn fold_fragments(out: Vec<Clause<'_>>) -> Vec<Clause<'_>> {
    let mut folded: Vec<Clause<'_>> = Vec::with_capacity(out.len());
    let mut carry: Vec<&str> = Vec::new();
    let mut carry_join = Join::Start;
    for c in out {
        if !c.quoted && !c.words.iter().any(|&t| verb_group(t)) {
            // A fragment carrying a negator or a governor is its own clause:
            // folding it forward would put the next clause's verb under its
            // scope ("no worries, sent it" — the "no" is not the sending's).
            if c.words
                .iter()
                .any(|&t| is(t, NEGATORS) || is(t, MODALS) || is(t, CONDITIONAL) || is(t, TEMPORAL))
            {
                if !carry.is_empty() {
                    folded.push(Clause {
                        words: std::mem::take(&mut carry),
                        join: carry_join,
                        quoted: false,
                    });
                }
                folded.push(c);
                continue;
            }
            if carry.is_empty() {
                carry_join = c.join;
            }
            carry.extend_from_slice(&c.words);
            continue;
        }
        if carry.is_empty() {
            folded.push(c);
        } else {
            let mut w = std::mem::take(&mut carry);
            w.extend_from_slice(&c.words);
            folded.push(Clause {
                words: w,
                join: carry_join,
                quoted: c.quoted,
            });
        }
    }
    if !carry.is_empty() {
        folded.push(Clause {
            words: carry,
            join: carry_join,
            quoted: false,
        });
    }
    folded
}

/// Whether `t` can anchor a verb group: a tabled verb, an auxiliary, a modal or
/// a finite negator. A clause without one is a noun-phrase fragment.
fn verb_group(t: &str) -> bool {
    verb(t).is_some()
        || is(t, BE)
        || is(t, HAVE)
        || is(t, GET)
        || is(t, MODALS)
        || is(t, FINITE_NEGATORS)
        || is(t, INVERTED)
        || pronoun(t).is_some_and(|(_, aux, _)| aux)
}

fn is_question(w: &[&str], last_of_question: bool) -> bool {
    let first = w.first().copied().unwrap_or("");
    // "Had it sorted out.", "got it booked.": have/get + object + participle is
    // a dropped-subject perfect or causative, not an inversion — a genuine
    // question ends with '?' and `last_of_question` covers it.
    let dropped_have = (is(first, HAVE) || is(first, GET))
        && matches!(w.get(1), Some(&("it" | "them" | "that" | "this")))
        && w[2..].iter().any(|t| done_shaped(t));
    last_of_question
        || (is(first, INVERTED) && w.len() > 1 && (pronoun(w[1]).is_some() || is(w[1], DETERMINERS)) && !dropped_have)
}

/// A period that closes one of these words is part of the word, not the end of the
/// sentence: "Dr. Okafor", "St. Albans", "etc.".
const NAMED_ABBREVIATIONS: &[&str] = &[
    "dr", "mr", "mrs", "ms", "prof", "sr", "jr", "st", "ave", "blvd", "rd", "ln", "mt", "ft", "etc", "approx", "dept",
    "est", "gen", "gov", "hon", "col", "capt", "lt", "sgt", "rev", "fr", "jan", "feb", "mar", "apr", "jun", "jul",
    "aug", "sep", "sept", "oct", "nov", "dec", "inc", "ltd", "co", "vs",
];

/// These abbreviate only before a number ("No. 5", "vol. 2", "fig. 4"); before a word they
/// are ordinary words — "No. I did" ends a sentence.
const NUMERIC_ABBREVIATIONS: &[&str] = &[
    "no", "nos", "vol", "fig", "ch", "sec", "pt", "ver", "pg", "pp", "art", "cl", "par",
];

/// Whether the '.' at byte `i` ends the sentence. It does not when it belongs to an
/// abbreviation ("Dr.", "St.", "No. 5"), an initial ("J. R. Hartley"), or a dotted token
/// ("a.m.", "p.m.", "e.g.", "U.S."). A dotted token is non-terminal only before a small
/// letter or digit — "Set for 6 a.m. Anything else?" still breaks, since a capital after
/// it starts a new sentence. A period inside a number ("3.5") never reaches here: its
/// next char is not whitespace.
pub(crate) fn terminal_period(text: &str, i: usize) -> bool {
    let token = text[..i]
        .rsplit(|c: char| c.is_whitespace())
        .next()
        .unwrap_or("")
        .trim_start_matches(|c: char| !c.is_alphanumeric());
    let lower = token.to_lowercase();
    let next = text[i + 1..].chars().find(|c| !c.is_whitespace());
    // "J. R." — a single letter is an initial.
    if lower.chars().count() == 1 && lower.chars().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    // "Dr.", "St." — the word itself is the abbreviation.
    if NAMED_ABBREVIATIONS.contains(&lower.as_str()) {
        return false;
    }
    // "No. 5" — an abbreviation only where a digit follows.
    if NUMERIC_ABBREVIATIONS.contains(&lower.as_str()) {
        return !matches!(next, Some(c) if c.is_ascii_digit());
    }
    // "a.m.", "e.g.", "U.S." — a token with a dot inside is abbreviated; the closing
    // period is terminal only before a capital or at the end.
    if token.contains('.') {
        return !matches!(next, Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit());
    }
    // "in 3. 4 cases" — a digit-final token before a digit is a stray decimal point.
    if token.chars().last().is_some_and(|c| c.is_ascii_digit()) && matches!(next, Some(c) if c.is_ascii_digit()) {
        return false;
    }
    true
}

/// Whether the char at byte `i` sits inside an open quotation — `"…"`, or a `'…'` that
/// bounds quoted speech ("you said 'I sent it'"). A single quote bounds a quotation only
/// at a word edge, so "don't" and "users'" never open one. Inside a quote a `.`/`!`/`?`
/// is the quoted party's — the sentence does not break there.
pub(crate) fn inside_quote(text: &str, i: usize) -> bool {
    let mut in_quotes = false;
    let mut prev: Option<char> = None;
    let mut chars = text[..i].chars().peekable();
    while let Some(c) = chars.next() {
        let next = chars.peek().copied();
        match c {
            '"' => in_quotes = !in_quotes,
            '\'' => {
                let opens = next.is_some_and(char::is_alphanumeric)
                    && prev.is_none_or(|p| p.is_whitespace() || matches!(p, ',' | ';' | ':' | '(' | '"' | '\''));
                let closes = in_quotes
                    && prev.is_some_and(char::is_alphanumeric)
                    && next.is_none_or(|n| {
                        n.is_whitespace() || matches!(n, '.' | ',' | '!' | '?' | ':' | ';' | ')' | '"' | '\'')
                    });
                if opens || closes {
                    in_quotes = !in_quotes;
                }
            }
            _ => {}
        }
        prev = Some(c);
    }
    in_quotes
}

/// Splits after `.`, `!`, `?` or `…` followed by whitespace or the end; each part keeps whether
/// it was a question. A `.` that belongs to an abbreviation, an initial or a number does not
/// end a sentence (`terminal_period`), and punctuation inside an open quotation is the
/// quoted party's (`inside_quote`).
fn sentences(text: &str) -> Vec<(&str, bool)> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut iter = text.char_indices().peekable();
    while let Some((i, c)) = iter.next() {
        if matches!(c, '.' | '!' | '?' | '…') {
            let end = i + c.len_utf8();
            let at_break = iter.peek().is_none_or(|(_, n)| n.is_whitespace() || *n == '"')
                && !inside_quote(text, i)
                && (c != '.' || terminal_period(text, i));
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

/// A polite frame that asks for one of these asks for speech, not an act on the user's
/// world: "can you explain…", "please describe…" is an answer request.
const ANSWER_VERBS: &[&str] = &[
    "tell",
    "say",
    "explain",
    "describe",
    "list",
    "show",
    "summarize",
    "summarise",
    "recap",
    "spell",
    "pronounce",
    "translate",
    "repeat",
    "define",
    "clarify",
    "read",
    "name",
];

/// Whether a heard request may ask for something to be done on the user's behalf, so that a
/// reply to it may claim it was done. The question is intent, not vocabulary: a request is
/// action-asking when it is *addressed* to the assistant — an imperative or a request form
/// ("remind me…", "book two seats", "can you move…", "please send…") — or asks whether the
/// assistant or the world already did something ("did you…", "has it been…"). A question
/// that merely mentions a task ("what time is my meeting?", "how do I pause my
/// membership?") asks for a fact and is informational, as is a request for speech ("can you
/// explain…", "tell me…") — answering is itself the assistant's act. Anything that is
/// neither a request nor question-shaped is ambiguous — a bare statement, a fragment — and
/// ambiguous fails safe to action: where intent is unclear the disclosure is unconditional.
/// A turn that answers such a request ends with the turn-level disclosure (D-027).
pub fn request_may_ask_for_action(heard: &str) -> bool {
    let lowered = heard.to_lowercase().replace(['\u{2019}', '\u{2018}'], "'");
    let w = words(&lowered);
    // "Did you…", "has it been…", "is my appointment confirmed…": asking whether
    // something was done.
    let asks_if_done = w.windows(2).any(|p| {
        matches!(p[0], "did" | "have" | "has" | "had") && matches!(p[1], "you" | "it" | "that" | "they" | "she" | "he")
    }) || (w.first().is_some_and(|f| is(f, BE))
        && matches!(w.get(1), Some(&("my" | "your" | "our")))
        && w.iter().any(|t| done_shaped(t)));
    // Addressed to the assistant as a request: "could you…", "please…", "any chance…",
    // "you mind…" — unless it asks for speech ("can you explain…"), which is an answer
    // request: telling the user is what the assistant does.
    let polite = w.windows(2).any(|p| {
        matches!(
            p[0],
            "can" | "could" | "would" | "will" | "won't" | "wouldn't" | "can't" | "couldn't"
        ) && matches!(p[1], "you" | "u" | "ye" | "ya" | "y'all" | "yall" | "thee")
    }) || w.iter().any(|t| matches!(*t, "please" | "pls" | "abeg"))
        || has_phrase(&w, &["any", "chance"])
        || w.first() == Some(&"mind")
        || has_phrase(&w, &["you", "mind"])
        // "Gonnae", "gonny": a fused request frame — Scots "will you" — when an
        // act or object follows it ("gonnae ring the plumber", "gonnae gie's a
        // hand"). Bare "gonnae rain" is the going-to prediction instead.
        || (matches!(w.first(), Some(&("gonnae" | "gonny" | "gony")))
            && w[1..].iter().any(|t| {
                verb(t).is_some() || is(t, DETERMINERS) || is(t, POSSESSIVES) || matches!(*t, "me" | "us")
            }));
    let asks_for_speech = polite && w.iter().any(|t| is(t, ANSWER_VERBS));
    if asks_if_done || (polite && !asks_for_speech) {
        return true;
    }
    let question = (lowered.trim_end().ends_with('?')
        || (w.first().is_some_and(|f| is(f, INFORMATIONAL)) && !imperative_head(&w)))
        && !(lowered.trim_end().ends_with('?') && tag_imperative(&w));
    !question
}

/// Informational-openers that can take a bare receiver — "tell Sam", "show her", "give
/// them the file", "help your mum": with a receiver other than the user the request is an
/// act on a third party, not speech to the user.
const RECEIVER_VERBS: &[&str] = &["tell", "show", "give", "write", "help", "suggest", "recommend"];

/// Determiner-led receivers that are people or places a message can go to: "tell the
/// babysitter", "tell the office", "show your mum". With a non-person object ("tell the
/// story") the same opener is speech.
const INSTITUTIONS: &[&str] = &[
    "office",
    "offices",
    "surgery",
    "practice",
    "school",
    "schools",
    "nursery",
    "work",
    "workplace",
    "team",
    "teams",
    "group",
    "groups",
    "committee",
    "council",
    "crew",
    "bank",
    "banks",
    "company",
    "companies",
    "firm",
    "firms",
    "hotel",
    "hotels",
    "restaurant",
    "restaurants",
    "pharmacy",
    "chemist",
    "chemists",
    "library",
    "gym",
    "clinic",
    "clinics",
    "hospital",
    "surgery",
];

/// After a receiver verb these read as content ("tell it", "say something"), not a person
/// being addressed.
const CONTENT_WORDS: &[&str] = &[
    "it",
    "something",
    "anything",
    "nothing",
    "everything",
    "what",
    "so",
    "please",
    "thanks",
    "sorry",
    "yes",
    "hello",
    "goodbye",
];

/// Whether the request opens with an imperative use of a word the informational set also
/// lists as a question opener: "do it", "do what I asked" commands rather than asks ("do
/// I…" is the auxiliary), and "tell him", "tell Sam", "show her" act on a third party
/// where "tell me" asks for speech.
fn imperative_head(w: &[&str]) -> bool {
    let second = w.get(1).copied().unwrap_or("");
    match w.first() {
        Some(&("do" | "does" | "did")) => !matches!(second, "i" | "you" | "u" | "we" | "they" | "he" | "she"),
        Some(&f) if is(f, INFORMATIONAL) => {
            !matches!(second, "me" | "us")
                && (matches!(second, "him" | "her" | "them")
                    || (is(f, RECEIVER_VERBS)
                        && ((!is(second, DETERMINERS) && !is(second, CONTENT_WORDS))
                            || w.get(2)
                                .is_some_and(|t| is(t, CONTACTS) || is(t, RELATIONS) || is(t, INSTITUTIONS)))))
        }
        _ => false,
    }
}

/// A trailing tag is a closed grammatical class, not a spelling list: an auxiliary
/// or modal plus the listener's pronoun ("…, will ya?", "…, won't ye?", "…, can't
/// you?") or a one-word confirmation ("…, alright?", "…, ta?"). A tag does not
/// turn the clause before it into a question — strip it and re-ask whether the
/// remainder is imperative-shaped. A tag on someone else's pronoun ("…, isn't
/// it?", "…, aren't they?") confirms a statement instead: it is a question still.
fn tag_imperative(w: &[&str]) -> bool {
    /// The tag's pronoun addresses the listener or the pair: "you", "ya", "ye",
    /// "we". "…, isn't it?" and "…, aren't they?" confirm a statement about
    /// someone else — no request is being made.
    const TAG_PRONOUNS: &[&str] = &["you", "ya", "ye", "u", "we", "y'all", "yall", "thee"];
    /// One-word confirmations — standard and dialect ("na", "abi", "no", "lah").
    const CONFIRMATIONS: &[&str] = &[
        "yeah",
        "yes",
        "yeh",
        "yep",
        "ya",
        "right",
        "ok",
        "okay",
        "alright",
        "ta",
        "eh",
        "aye",
        "huh",
        "innit",
        "correct",
        "na",
        "no",
        "naw",
        "abi",
        "se",
        "she",
        "neh",
        "lah",
        "leh",
        "hor",
        "meh",
        "ba",
        "don",
        "kan",
        "kah",
        "wae",
        "capisce",
        "comprende",
        "savvy",
        "understand",
    ];
    /// Terms of address in tag position: "…, mate?", "…, boss?", "…, love?".
    const VOCATIVES: &[&str] = &[
        "mate",
        "pal",
        "buddy",
        "hun",
        "honey",
        "love",
        "boss",
        "chief",
        "fam",
        "bruv",
        "bro",
        "sis",
        "dear",
        "duck",
        "hen",
        "pet",
        "son",
        "lad",
        "lass",
        "bhai",
        "yaar",
        "sha",
        "mucker",
        "guv",
        "sir",
        "madam",
        "darling",
        "sweetheart",
        "boyo",
        "babes",
        "luv",
    ];
    // The auxiliaries and modals a request tag is built from — INVERTED's
    // be/have/do set plus the modal contractions — and the fused forms that
    // contract the whole pair into one token ("d'ya?", "wontcha?").
    let tag_aux = |t: &str| is(t, INVERTED) || is(t, MODALS);
    let tag_len = match w {
        // "can you not", "will ye not"
        [.., aux, p, "not"] if tag_aux(aux) && is(p, TAG_PRONOUNS) => 3,
        // "…, you hear me?" — the hearsay check as a three-word tag.
        [.., p, "hear", "me"] if is(p, TAG_PRONOUNS) => 3,
        // "will ya", "won't ye", "can't you", "shall we", "didn't you"
        [.., aux, p] if tag_aux(aux) && is(p, TAG_PRONOUNS) => 2,
        // "…, you hear?", "…, ya hear?"
        [.., p, "hear"] if is(p, TAG_PRONOUNS) => 2,
        // "alright", "ok", "yeah", "ta", "innit", "d'ya", "mate", "na", "abi"
        [.., one]
            if is(one, CONFIRMATIONS)
                || is(one, VOCATIVES)
                || matches!(
                    *one,
                    "d'you" | "d'ya" | "d'ye" | "willya" | "wouldja" | "couldja" | "cancha" | "dontcha" | "wontcha"
                ) =>
        {
            1
        }
        _ => return false,
    };
    let rest = &w[..w.len() - tag_len];
    !rest.is_empty() && imperative_shaped(rest)
}

/// Whether the clause before a tag reads as a request: verb-first ("lock the
/// door", "sort it") or a receiver-verb phrase ("tell him", "show her"). A
/// pronoun-, determiner-, possessive- or auxiliary-led clause is a statement
/// with a confirmation tag — "the kids are fed, yeah?" asks, it does not tell.
fn imperative_shaped(w: &[&str]) -> bool {
    let Some(&first) = w.first() else {
        return false;
    };
    if imperative_head(w) {
        return true;
    }
    !(pronoun(first).is_some()
        || is(first, DETERMINERS)
        || is(first, POSSESSIVES)
        || is(first, ARTICLES)
        || is(first, BE)
        || is(first, HAVE)
        || is(first, GET)
        || is(first, MODALS)
        || is(first, NEGATORS)
        || is(first, FINITE_NEGATORS)
        || is(first, NEG_SUBJECTS)
        || is(first, CONDITIONAL)
        || is(first, TEMPORAL)
        || is(first, INFORMATIONAL))
}

/// Whether `t` is a completed form — "confirmed", "sorted", "gone", "told" — by the
/// lexicon's table or by its past shape.
fn done_shaped(t: &str) -> bool {
    past_shaped(t) || verb(t).is_some_and(|v| matches!(v.form, Form::Done))
}

/// Persons the reply named — "mr byrne rang…", "she spoke to chidi", "mum
/// mentioned bill": a nounish token next to a person-verb, a courtesy title, or
/// a dative/contact preposition names a party, so a later "bill moved it" binds
/// to him instead of reading the token as a remit noun.
fn agent_names(lowered: &str) -> Vec<&str> {
    const PERSON_VERBS: &[&str] = &[
        "rang",
        "called",
        "emailed",
        "texted",
        "text",
        "phoned",
        "spoke",
        "mailed",
        "wrote",
        "met",
        "mentioned",
        "thanked",
        "warned",
        "promised",
        "contacted",
        "messaged",
    ];
    const TITLES: &[&str] = &["mr", "mrs", "ms", "miss", "dr", "prof", "sir", "madam", "lady", "lord"];
    let w = words(lowered);
    let mut names = Vec::new();
    let candidate = |t: &str| nounish(t) && verb(t).is_none() && !is_number(t);
    for (i, t) in w.iter().enumerate() {
        // "told bill", "spoke to chidi", "a card from niamh".
        if (is(t, PERSON_VERBS) || matches!(*t, "to" | "by" | "with" | "from"))
            && w.get(i + 1).is_some_and(|x| candidate(x))
        {
            names.push(w[i + 1]);
        }
        // "chidi called", "mr byrne rang": the party on the verb's left.
        if (is(t, PERSON_VERBS) || is(t, TITLES)) && i > 0 && candidate(w[i - 1]) {
            names.push(w[i - 1]);
        }
    }
    names.sort_unstable();
    names.dedup();
    names
}

/// Classifies one reply sentence (or several: each is judged and the strongest verdict wins).
pub fn classify(text: &str) -> Verdict {
    let lowered = text
        .to_lowercase()
        .replace(['\u{2019}', '\u{2018}', '`'], "'")
        .replace(['\u{201c}', '\u{201d}', '«', '»'], "\"");
    let names = agent_names(&lowered);
    sentences(&lowered)
        .into_iter()
        .map(|(s, question)| classify_sentence(s, question, &names))
        .max()
        .unwrap_or(Verdict::Plain)
}

/// Whether a sentence would be replaced or disclosed: it may claim that an action happened.
pub fn claims_completion(text: &str) -> bool {
    classify(text) != Verdict::Plain
}

/// The second-pass option measured for D-029 — deliberately unshipped: where an
/// informational turn's reply survives `uninvited_action` plain, one answer to "does this
/// reply say the assistant did something?" would decide the disclosure. `judge` is
/// injected so the same path measures any implementation — the on-device model in the
/// app, a lexical stand-in or an oracle in tests — and no production code calls it.
pub fn second_pass(reply: &str, judge: impl Fn(&str) -> bool) -> bool {
    uninvited_action(reply) || judge(reply)
}

/// Arrangement-state idioms: a person, task or thing reported as put where it should be.
const ARRANGEMENT: &[&[&str]] = &[
    &["in", "the", "loop"],
    &["on", "the", "list"],
    &["on", "the", "rota"],
    &["on", "the", "roster"],
    &["on", "the", "docket"],
    &["on", "the", "calendar"],
    &["on", "the", "schedule"],
    &["in", "the", "diary"],
    &["in", "the", "calendar"],
    &["in", "the", "schedule"],
    &["in", "the", "post"],
    &["in", "the", "mail"],
    &["in", "the", "works"],
    &["in", "the", "pipeline"],
    &["under", "way"],
    &["down", "for"],
    &["waiting", "for", "you"],
    &["expecting", "you"],
    &["coming", "your", "way"],
    &["paid", "up"],
    &["squared", "away"],
    &["good", "to", "go"],
    &["in", "hand"],
    &["under", "control"],
    &["dealt", "with"],
    &["seen", "to"],
    &["sorted", "out"],
    &["ironed", "out"],
    &["firmed", "up"],
    &["locked", "in"],
    &["lined", "up"],
    &["pencilled", "in"],
    &["penciled", "in"],
    &["slated", "for"],
];

/// Things a "has" or "got" can deliver: "she has the files", "he's got the paperwork".
const DELIVERABLES: &[&str] = &[
    "file",
    "files",
    "key",
    "keys",
    "parcel",
    "parcels",
    "package",
    "packages",
    "score",
    "scores",
    "news",
    "result",
    "results",
    "paperwork",
    "document",
    "documents",
    "form",
    "forms",
    "cash",
    "cheque",
    "check",
    "confirmation",
    "receipt",
    "ticket",
    "tickets",
    "pass",
    "passes",
    "seat",
    "seats",
    "copy",
    "copies",
    "info",
    "information",
    "details",
    "goods",
    "mail",
    "invite",
    "invites",
];

/// Words between "has"/"got" and what is held.
const POSSESSION_FILLERS: &[&str] = &[
    "now", "already", "the", "a", "an", "your", "my", "her", "his", "their", "our", "some", "just", "finally", "also",
    "even", "still",
];

/// Irregular participles an "-ed" shape cannot mark ("I've dealt with it", "it was left").
const IRREGULAR_DONE: &[&str] = &[
    "taken",
    "dealt",
    "made",
    "done",
    "gone",
    "put",
    "left",
    "seen",
    "heard",
    "read",
    "met",
    "kept",
    "given",
    "found",
    "meant",
    "lent",
    "spent",
    "rung",
    "brought",
    "thought",
    "sought",
    "caught",
    "taught",
    "sold",
    "held",
    "handled",
    "told",
    "said",
    "went",
    "sent",
    "paid",
    "bought",
    "built",
    "felt",
    "hid",
    "laid",
    "led",
    "lost",
    "ran",
    "shot",
    "slept",
    "slid",
    "stood",
    "stuck",
    "swore",
    "swept",
    "swung",
    "rang",
    "took",
    "gave",
    "drove",
    "ate",
    "drank",
    "spoke",
    "broke",
    "chose",
    "froze",
    "stole",
    "saw",
    "fell",
    "grew",
    "knew",
    "threw",
    "blew",
    "flew",
    "drew",
    "sat",
    "fought",
    "forgot",
    "became",
    "came",
    "began",
    "dug",
    "hung",
    "lit",
    "struck",
    "spun",
    "stung",
    "wrung",
    "hit",
    "cut",
    "cost",
    "shut",
    "set",
    "bet",
    "fed",
    "bled",
    "bent",
    "leant",
    "leapt",
    "dreamt",
    "burst",
    "cast",
    "quit",
    "spat",
    "split",
    "spread",
    "thrust",
    "fled",
    "understood",
    "woke",
    "won",
    "wore",
    "wrote",
    "written",
];

/// "-ed" words that describe a state, not a completed act: "I'm tired", "it was noted" is a
/// claim but "I'm worried" is not, and only the lexicon misses them here.
const PARTICIPLE_ADJECTIVES: &[&str] = &[
    "tired",
    "worried",
    "surprised",
    "bored",
    "confused",
    "excited",
    "pleased",
    "disappointed",
    "embarrassed",
    "frustrated",
    "relieved",
    "amazed",
    "ashamed",
    "prepared",
    "interested",
    "married",
    "afraid",
    "involved",
    "related",
    "based",
    "used",
    "supposed",
    "located",
    "expected",
    "delighted",
    "thrilled",
    "satisfied",
    "concerned",
    "convinced",
    "depressed",
    "impressed",
    "offended",
    "stunned",
    "terrified",
    "exhausted",
    "fascinated",
    "motivated",
    "overwhelmed",
    "encouraged",
    "entertained",
    "annoyed",
    "appalled",
    "astonished",
    "bewildered",
    "comforted",
    "determined",
    "disgusted",
    "distressed",
    "disturbed",
    "energized",
    "enlightened",
    "honoured",
    "honored",
    "horrified",
    "humiliated",
    "inclined",
    "inspired",
    "insulted",
    "intrigued",
    "irritated",
    "limited",
    "minded",
    "qualified",
    "saddened",
    "scared",
    "skilled",
    "spirited",
    "strained",
    "stressed",
    "structured",
    "suited",
    "tempered",
    "touched",
    "troubled",
    "united",
    "crowded",
    "detailed",
    "gifted",
    "talented",
    "accustomed",
    "addicted",
    "assorted",
    "beloved",
    "blessed",
    "crooked",
    "learned",
    "marked",
    "ragged",
    "rugged",
    "sacred",
    "wicked",
    "wretched",
];

/// Whether `t` looks like a completed form the verb lexicon does not know.
fn past_shaped(t: &str) -> bool {
    if is(t, PARTICIPLE_ADJECTIVES) || is(t, BE) || is(t, HAVE) || is_number(t) {
        return false;
    }
    is(t, IRREGULAR_DONE)
        || t.strip_suffix("ed").is_some_and(|stem| stem.len() >= 3)
        || t.strip_suffix("'d")
            .is_some_and(|stem| stem.len() >= 2 && !is(stem, D_STEMS))
}

/// Whether a sentence asserts an action consequence the request never asked for (D-027). A
/// reply to an informational request has no executed evidence to draw on — nothing was asked
/// to be done and in this release nothing can have been — so where `classify` is a precision
/// instrument, this screen is deliberately recall-tilted and the session only ever lets a
/// positive earn the turn disclosure. It catches the claim shapes the classifier
/// structurally cannot: an action verb it does not know under a first-person subject ("I
/// sorted that"), a delivery stated as possession with any definite or quantified object
/// ("she's got it", "he has the forty dollars"), and the arrangement idioms ("in the loop",
/// "your reminder is in", "squared away").
pub fn uninvited_action(text: &str) -> bool {
    let lowered = text
        .to_lowercase()
        .replace(['\u{2019}', '\u{2018}', '`'], "'")
        .replace(['\u{201c}', '\u{201d}', '«', '»'], "\"");
    let names = agent_names(&lowered);
    sentences(&lowered).into_iter().any(|(sentence, question)| {
        let clauses = clauses(sentence);
        let last = clauses.len().saturating_sub(1);
        let mut reported = false;
        let mut hit = false;
        for (n, clause) in clauses.iter().enumerate() {
            let w = &clause.words;
            reported |= w.first().is_some_and(|t| {
                matches!(
                    *t,
                    "according" | "per" | "apparently" | "reportedly" | "supposedly" | "allegedly"
                )
            });
            let rep = report_at(w);
            let floor = w
                .first()
                .is_some_and(|t| pronoun(t).is_some_and(|(s, _, _)| s == Subject::First));
            if !clause.quoted
                && (!reported || floor)
                && !is_question(w, question && n == last)
                && uninvited_clause(w, rep, &names)
            {
                hit = true;
            }
            reported |= rep.is_some();
        }
        hit
    })
}

fn uninvited_clause(w: &[&str], rep: Option<usize>, names: &[&str]) -> bool {
    dropped_subject(w, rep)
        || remit_change(w, rep, names)
        || deliverable_in_space(w, rep)
        || informed_contact(w, rep)
        || assistant_accomplished(w, rep)
        || first_person_act(w, rep)
        || delivery_as_possession(w, rep)
        || arrangement_state(w, rep)
        || agentless_state(w, rep, names)
        || scheduled_change(w, rep, names)
}

/// "…the payment I sent on Tuesday", "which I did through your bank", "I may have already
/// paid it", "I rang the surgery": a first person doing any completed form is the
/// assistant's own report, whatever the verb and wherever in the clause it sits — a
/// relative, a hedge, an untabled verb. A modal or negator in the verb group governs it
/// away ("I will send", "I can't do it"), and "I was/am" is a state, not an act.
fn first_person_act(w: &[&str], rep: Option<usize>) -> bool {
    w.iter().enumerate().any(|(i, t)| {
        if rep.is_some_and(|r| i > r) || !matches!(*t, "i" | "we") {
            return false;
        }
        // "If I sent it", "whether I paid": a governor before the "I" owns the clause.
        if w[..i]
            .iter()
            .any(|x| is(x, CONDITIONAL) || is(x, NEGATORS) || is(x, MODALS) || *x == "to")
        {
            return false;
        }
        // "I have", "I may have", "I just…": auxiliaries, hedges and adverbs pass through
        // to the candidate; anything else is the candidate itself, and a modal, negator
        // or non-completed form ("I will send", "I need", "I think") claims nothing.
        let mut k = i + 1;
        while k < w.len() && (is(w[k], ADVERBS) || matches!(w[k], "have" | "'ve" | "had" | "may" | "might" | "been")) {
            k += 1;
        }
        // "I was worried", "I am tired": a state after "was/am/were" is no act.
        if w[i + 1..k].iter().any(|x| is(x, BE)) {
            return false;
        }
        k < w.len() && first_person_done(w, k)
    })
}

/// The verb a bare "I" claims: a completed shape by the table, an irregular past, or an
/// "-ed" with a two-letter stem ("I used the points"). States and governed forms are out,
/// and "I used to…" is a habit, not an act.
fn first_person_done(w: &[&str], k: usize) -> bool {
    let t = w[k];
    if is(t, BE)
        || is(t, HAVE)
        || is(t, MODALS)
        || is(t, NEGATORS)
        || is(t, DESIRE)
        || is(t, CONDITIONAL)
        || is_number(t)
        || pronoun(t).is_some()
    {
        return false;
    }
    if t == "used" {
        return w.get(k + 1) != Some(&"to");
    }
    if is(t, PARTICIPLE_ADJECTIVES) {
        // "I'm tired" is a state, but "I marked the job" is an act — an object
        // after the word settles it as a verb.
        return w
            .get(k + 1)
            .is_some_and(|n| is(n, OBJECT_PRONOUNS) || is(n, DETERMINERS) || is_number(n));
    }
    done_shaped(t) || t.strip_suffix("ed").is_some_and(|s| s.len() >= 2)
}

/// "A table is held under your name", "your usual is in for an 8 o'clock pickup",
/// "the heating's coming on at half five now instead of six": a passive hold, a
/// booked slot or a rescheduled state on the remit — with no other agent the
/// assistant is implied. A person in a state is their own business ("mum's in
/// for a surprise" — no slot value — and a named agent's change is theirs).
fn scheduled_change(w: &[&str], rep: Option<usize>, names: &[&str]) -> bool {
    let governed = |upto: usize| governed_prefix(w, upto);
    // "Held/kept/reserved under your name", "held for you": a passive hold for
    // the user — nobody else books under your name.
    let held = w.iter().enumerate().any(|(i, t)| {
        if rep.is_some_and(|r| i > r)
            || !matches!(*t, "held" | "kept" | "reserved" | "booked" | "put" | "saved")
            || governed(i)
        {
            return false;
        }
        let under_name = w.get(i + 1) == Some(&"under")
            && w.get(i + 2).is_some_and(|x| is(x, POSSESSIVES))
            && w.get(i + 3)
                .is_some_and(|x| matches!(*x, "name" | "names" | "account" | "accounts"));
        let for_you =
            w.get(i + 1) == Some(&"for") && matches!(w.get(i + 2), Some(&("you" | "ye" | "ya" | "u" | "thee")));
        if !(under_name || for_you) || by_agent(w, i) {
            return false;
        }
        // A "be" in the verb group makes it a passive: the subject is the thing
        // held. Without one the left subject would be the holder itself — the
        // assistant's own "I held…" claims, and so does a noun-headed ellipsis
        // ("a snug kept for ye" — the snug is held, nobody else kept it).
        if w[..i].iter().rev().take(4).any(|x| is(x, BE)) {
            true
        } else {
            match scan_back(w, i).subject {
                Subject::First => true,
                Subject::Noun { head } => {
                    remit_thing(w, head, names)
                        || (nounish(w[head]) && !is(w[head], CONTACTS) && !is(w[head], RELATIONS))
                }
                _ => false,
            }
        }
    });
    // "In for" a booked slot — a number or day inside the next few words ("in
    // for an 8 o'clock pickup", "in for nine"). "In for a surprise" has none.
    let in_for = w.iter().enumerate().any(|(i, t)| {
        if rep.is_some_and(|r| i > r)
            || *t != "in"
            || w.get(i + 1) != Some(&"for")
            || governed(i)
            || i == 0
            || !w[i + 2..].iter().take(4).any(|x| is_number(x) || is(x, DAYS_MONTHS))
        {
            return false;
        }
        // The subject is the thing booked — a remit noun, a possessed one ("your
        // usual") or a bare "it" — never a person whose slot it is.
        if is(w[i - 1], BE) {
            match scan_back(w, i - 1).subject {
                Subject::Noun { head } => remit_thing(w, head, names) || w[..i].iter().any(|x| is(x, POSSESSIVES)),
                Subject::Thing => true,
                _ => false,
            }
        } else {
            pronoun(w[i - 1]).is_some_and(|(s, aux, _)| aux && s == Subject::Thing)
        }
    });
    // "…instead of six", "…instead of Friday": a remit thing reported at a new
    // slot value — the reschedule is done to it, and nobody else did it.
    let instead = w.iter().enumerate().any(|(i, t)| {
        if rep.is_some_and(|r| i > r) || *t != "instead" || w.get(i + 1) != Some(&"of") || governed(i) {
            return false;
        }
        if !w[i + 2..].iter().take(3).any(|x| is_number(x) || is(x, DAYS_MONTHS)) {
            return false;
        }
        // An explicit agent on the left owns the change ("Niamh moved it to
        // five instead of six"); the assistant's own "I" claims anyway.
        if (0..i).any(|j| {
            owning_verb(w[j])
                && !is(w[j], BE)
                && !is(w[j], HAVE)
                && match scan_back(w, j).subject {
                    Subject::First | Subject::None | Subject::Negative => false,
                    Subject::Noun { head } => !remit_subject(w, head, names),
                    _ => true,
                }
        }) {
            return false;
        }
        w[..i]
            .iter()
            .any(|x| is(x, TASK_NOUNS) || is(x, DEVICES) || is(x, REMIT_NOUNS) || is(x, DELIVERABLES))
    });
    held || in_for || instead || state_facts(w, rep, names)
}

/// A governing word anywhere before index `upto`: negators, modals and the
/// subordinators all suppress the state that follows them.
fn governed_prefix(w: &[&str], upto: usize) -> bool {
    w[..upto]
        .iter()
        .any(|x| is(x, NEGATORS) || is(x, MODALS) || is(x, CONDITIONAL) || is(x, TEMPORAL))
}

/// World-fact state changes stated without their act — "a table for two under
/// your name now", "your account's on auto top-up", "the switch is under way":
/// the booking noun or the remit thing stands for the whole event, and only the
/// assistant puts things under your name or onto a service mode.
fn state_facts(w: &[&str], rep: Option<usize>, names: &[&str]) -> bool {
    // "A table for two under your name now": the hold stated without its verb.
    // The booking noun on the left stands for the whole event — nobody else
    // books under your name.
    let under_name = (0..w.len()).any(|i| {
        if rep.is_some_and(|r| i > r)
            || w[i] != "under"
            || !matches!(w.get(i + 2), Some(&("name" | "names" | "account" | "accounts")))
            || !w.get(i + 1).is_some_and(|t| is(t, POSSESSIVES))
            || governed_prefix(w, i)
        {
            return false;
        }
        // No agent verb on the left ("niamh booked a table under your name" is
        // hers — it still states the hold, but it is her report of it).
        w[..i].iter().any(|x| is(x, TASK_NOUNS) || is(x, BOOKINGS)) && !w[..i].iter().any(|x| owning_verb(x))
    });
    // "On auto top-up now", "the bill's on auto-renew": a remit thing reported
    // under a service mode — only the assistant puts an account on one.
    let on_mode = (0..w.len()).any(|i| {
        if rep.is_some_and(|r| i > r)
            || w[i] != "on"
            || i < 1
            || !is(w[i - 1], BE)
            || !w.get(i + 1).is_some_and(|t| is(t, SERVICE_MODES))
            || governed_prefix(w, i)
        {
            return false;
        }
        match scan_back(w, i - 1).subject {
            Subject::Noun { head } => remit_thing(w, head, names) || w[..i].iter().any(|x| is(x, POSSESSIVES)),
            Subject::Thing => true,
            _ => false,
        }
    });
    under_name || on_mode
}

/// A service or billing mode a remit thing can be put into: "on auto top-up",
/// "on auto-renew", "on hold", "on repeat".
const SERVICE_MODES: &[&str] = &[
    "auto",
    "automatic",
    "autopilot",
    "renew",
    "renewal",
    "rollover",
    "repeat",
    "repeating",
    "recurring",
    "pause",
    "paused",
    "hold",
    "standby",
    "snooze",
    "snoozed",
    "mute",
    "muted",
    "silent",
    "charge",
];

/// Booking nouns that can head a verb-less hold — "a table", "a snug", "a cab"
/// — the remit's seats and rooms, a few of which the task list does not carry.
const BOOKINGS: &[&str] = &[
    "seat", "seats", "snug", "cover", "covers", "berth", "cab", "cabs", "taxi", "taxis", "spot", "spots",
];

/// "It's ringing now", "your timer's running", "the kettle's off", "she does now": a
/// device, a remit thing or a managed person reported in a changed state with no other
/// agent — nothing else can have changed it. Weather and time states are the world's; a
/// person acting the verb itself is their own ("the train's running" is machinery's).
fn agentless_state(w: &[&str], rep: Option<usize>, names: &[&str]) -> bool {
    // "<person> does now": an elliptical informing — "she does now" = she knows now.
    let does_now = w.windows(2).enumerate().any(|(i, p)| {
        rep.is_none_or(|r| i < r)
            && matches!(p[0], "does" | "do")
            && matches!(p[1], "now" | "today" | "already" | "finally")
            && match scan_back(w, i).subject {
                Subject::Noun { head } => is(w[head], CONTACTS) || is(w[head], RELATIONS) || nounish(w[head]),
                Subject::Third => true,
                _ => false,
            }
            && !w[..i]
                .iter()
                .any(|x| is(x, NEGATORS) || is(x, MODALS) || is(x, CONDITIONAL) || is(x, TEMPORAL))
    });
    // "<remit thing or bare it> be <state>": only the assistant can have put it there.
    let state = w.iter().enumerate().any(|(i, t)| {
        if rep.is_some_and(|r| i > r) || !is(t, ACTIVE_STATES) || is(t, NATURAL_STATES) {
            return false;
        }
        // "It's on the Seine" locates; "it's on" or "it's off already" changes.
        if is(t, LOCATIVE_STATES) && w.get(i + 1).is_some_and(|n| !is(n, STATE_FOLLOWERS)) {
            return false;
        }
        let ctx = scan_back(w, i);
        if ctx.governed || !ctx.aux || by_agent(w, i) {
            return false;
        }
        match ctx.subject {
            Subject::Noun { head } => remit_thing(w, head, names),
            Subject::Thing => true,
            _ => false,
        }
    });
    does_now || state
}

/// "I sorted that", "I've noted it", "we dropped it off", "I made a note": the assistant
/// reports doing something. On a turn that asked for no action the lexicon's own judgment
/// is beside the point — a completed form under a first-person subject is an uninvited
/// claim whether or not `classify` has a verb for it (its task-only verbs only claim next
/// to a task word, so "made a note" is otherwise missed).
fn assistant_accomplished(w: &[&str], rep: Option<usize>) -> bool {
    w.iter().enumerate().any(|(i, t)| {
        if rep.is_some_and(|r| i > r) || !owning_verb(t) {
            return false;
        }
        let ctx = scan_back(w, i);
        ctx.subject == Subject::First && !ctx.governed && !ctx.passive
    })
}

/// "She's got it", "he has the forty dollars", "Sam has the files": a delivery stated as
/// possession. `phrases` names a few objects; an uninvited one may be anything definite
/// (it/them/the…) or quantified ("forty dollars", "three tickets"), while "she has a
/// brother" stays a fact about her.
fn delivery_as_possession(w: &[&str], rep: Option<usize>) -> bool {
    w.iter().enumerate().any(|(i, t)| {
        if rep.is_some_and(|r| i > r) || !matches!(*t, "has" | "have" | "got") {
            return false;
        }
        let after = &w[i + 1..];
        let Some((j, object)) = after
            .iter()
            .take(5)
            .enumerate()
            .find(|(_, x)| !is(x, POSSESSION_FILLERS))
        else {
            return false;
        };
        let delivered = matches!(*object, "it" | "them" | "that" | "this" | "everything")
            || is(object, TASK_NOUNS)
            || is(object, TASK_STRONG)
            || is(object, DELIVERABLES)
            || is(object, DELIVERABLE_UNITS)
            || (is_number(object)
                && after[j + 1..].first().is_some_and(|n| {
                    is_number(n) || is(n, TASK_NOUNS) || is(n, DELIVERABLES) || is(n, DELIVERABLE_UNITS)
                }));
        if !delivered {
            return false;
        }
        let ctx = scan_back(w, i);
        if ctx.governed {
            return false;
        }
        // "She got it sorted", "they got the car fixed": an object followed by a
        // participle is the owner's causative, not a delivery — the assistant's
        // own "I got it sorted" still claims.
        if after[j + 1..].iter().take(2).any(|x| past_shaped(x)) && ctx.subject != Subject::First {
            return false;
        }
        // "You've got it": a bare "it" under "you" reads as understanding or mere
        // possession; a concrete object ("you've got the seats", "two nights") claims
        // the delivery.
        if ctx.subject == Subject::Second && matches!(*object, "it" | "them" | "that" | "this" | "everything") {
            return false;
        }
        matches!(
            ctx.subject,
            Subject::First | Subject::Second | Subject::Third | Subject::Thing | Subject::Noun { .. }
        )
    })
}

/// "Your reminder is in", "it's in": a task or thing reported in the state "in" — and the
/// arrangement idioms a state can be phrased as.
fn arrangement_state(w: &[&str], rep: Option<usize>) -> bool {
    for phrase in ARRANGEMENT {
        if let Some(i) = w.windows(phrase.len()).position(|win| win == *phrase)
            && rep.is_none_or(|r| i <= r)
            && !scan_back(w, i).governed
            // "He had it sorted out": the arrangement is his causative.
            && !anothers_causative(w, i)
        {
            return true;
        }
    }
    w.iter()
        .enumerate()
        .any(|(i, t)| rep.is_none_or(|r| i <= r) && *t == "in" && i >= 1 && arranged_in(w, i))
}

/// "Your reminder is in", "it's in", "you're in": the state "in" claimed for a task, a thing
/// or a person — at a clause's end or before a time adverb, where "in" is the state itself
/// rather than the start of a place phrase ("the shop is in town").
fn arranged_in(w: &[&str], i: usize) -> bool {
    // "reminder is in", "it is in": BE before "in", a task or thing before BE.
    let via_be = i >= 2
        && is(w[i - 1], BE)
        && (is(w[i - 2], TASK_NOUNS) || matches!(w[i - 2], "it" | "they" | "that" | "this" | "everything" | "all"));
    // "it's in", "you're in": the auxiliary is inside the pronoun's contraction.
    let via_pronoun = pronoun(w[i - 1])
        .is_some_and(|(subject, aux, _)| aux && matches!(subject, Subject::Second | Subject::Third | Subject::Thing));
    (via_be || via_pronoun)
        && w.get(i + 1)
            .copied()
            .is_none_or(|n| matches!(n, "now" | "already" | "too" | "finally" | "for" | "as" | "since"))
        && !w[..i]
            .iter()
            .any(|x| is(x, NEGATORS) || is(x, MODALS) || is(x, CONDITIONAL) || is(x, TEMPORAL))
}

/// Whether `t` can head a noun phrase — anything that is not a verb form, a function word or
/// a question word. Loose on purpose: its callers bound it with the role they check.
fn nounish(t: &str) -> bool {
    !(is(t, BE)
        || is(t, HAVE)
        || is(t, GET)
        || is(t, MODALS)
        || is(t, NEGATORS)
        || is(t, FINITE_NEGATORS)
        || is(t, NEG_SUBJECTS)
        || is(t, ADVERBS)
        || is(t, HABITUAL)
        || is(t, CONDITIONAL)
        || is(t, TEMPORAL)
        || is(t, DESIRE)
        || is(t, FRAMES)
        || is(t, ARTICLES)
        || is(t, POSSESSIVES)
        || is(t, DETERMINERS)
        || is(t, OBJECT_PRONOUNS)
        || is(t, AFTER_PARTICIPLE)
        || is(t, INFORMATIONAL)
        || is(t, D_STEMS)
        || matches!(
            t,
            "someone" | "somebody" | "anyone" | "anybody" | "everyone" | "everybody"
        ))
}

/// Where the noun phrase ending at `head` begins — the same walk `scan_back` makes.
fn noun_phrase_start(w: &[&str], head: usize) -> usize {
    let mut start = head;
    while start > 0 && (is(w[start - 1], DETERMINERS) || is_number(w[start - 1]) || is(w[start - 1], TASK_NOUNS)) {
        start -= 1;
    }
    start
}

/// "He's had the car fixed", "they got it sorted": have/get + object + participle
/// is a causative — the arranging belongs to the have/get's subject, not to the
/// object the participle attaches to. Returns the have/get's index when the
/// participle at `i` sits in the frame.
fn causative_at(w: &[&str], i: usize) -> Option<usize> {
    if i == 0 {
        return None;
    }
    // The object: a bare pronoun ("had it fixed") or a noun phrase ("had the
    // car fixed", "got my papers sorted").
    let mut start = i - 1;
    while start > 0
        && (is(w[start - 1], DETERMINERS)
            || is(w[start - 1], POSSESSIVES)
            || is_number(w[start - 1])
            || is(w[start - 1], TASK_NOUNS))
    {
        start -= 1;
    }
    // The whole gap is the object: "has been paid" is a passive perfect — "been"
    // is an auxiliary, not an object — so the causative is not there.
    if !w[start..i]
        .iter()
        .all(|t| nounish(t) || is(t, OBJECT_PRONOUNS) || is(t, DETERMINERS) || is(t, POSSESSIVES) || is_number(t))
    {
        return None;
    }
    let before = start.checked_sub(1)?;
    (is(w[before], HAVE) || is(w[before], GET)).then_some(before)
}

/// Whether the participle at `i` is another party's causative — the have/get
/// subject is not the assistant. A first-person or dropped owner keeps the claim.
fn anothers_causative(w: &[&str], i: usize) -> bool {
    causative_at(w, i).is_some_and(|h| {
        let owner = scan_back(w, h);
        owner.governed || !matches!(owner.subject, Subject::First | Subject::None)
    })
}

/// The judge's verdict when the participle at `i` sits in a causative frame:
/// the assistant's own ("I've had it fixed", bare "had it sorted") still claims;
/// a governed or third-party owner makes it theirs. `None` when there is no
/// frame and the ordinary dispatch applies.
fn causative_verdict(w: &[&str], i: usize, claim: Verdict) -> Option<Verdict> {
    let h = causative_at(w, i)?;
    let owner = scan_back(w, h);
    Some(
        if !owner.governed && matches!(owner.subject, Subject::First | Subject::None) {
            claim
        } else {
            Verdict::Plain
        },
    )
}

/// A managed *thing*: a task noun, device, deliverable or remit noun — or a slot named by its
/// value ("your nine o'clock"). Its own change reads as the assistant's doing. Persons are
/// handled separately: they act for themselves.
fn remit_thing(w: &[&str], head: usize, names: &[&str]) -> bool {
    let Some(&h) = w.get(head) else { return false };
    // A person named earlier is not the remit, even when their name is also a
    // task noun — "mum mentioned bill; bill moved the meeting".
    if names.contains(&h) {
        return false;
    }
    if is(h, TASK_NOUNS) || is(h, DEVICES) || is(h, DELIVERABLES) || is(h, REMIT_NOUNS) || is(h, FOREIGN) {
        return true;
    }
    let start = noun_phrase_start(w, head);
    matches!(w[start], "the" | "your" | "my" | "his" | "her" | "their" | "its")
        && w[start..=head].iter().any(|t| is_number(t))
}

/// The user's remit: managed things, plus the people an assistant contacts on the user's
/// behalf ("the gardener's been asked", "your mum's been told").
fn remit_subject(w: &[&str], head: usize, names: &[&str]) -> bool {
    remit_thing(w, head, names) || w.get(head).is_some_and(|h| is(h, CONTACTS) || is(h, RELATIONS))
}

/// "Many letters were sent during the war": an indefinite or quantified subject is nobody's
/// remit — the user's things are definite ("the", "your") or named.
fn generic_subject(w: &[&str], head: usize) -> bool {
    head > 0
        && w[..head].iter().any(|t| {
            matches!(
                *t,
                "many"
                    | "some"
                    | "several"
                    | "few"
                    | "most"
                    | "lots"
                    | "plenty"
                    | "various"
                    | "countless"
                    | "hundreds"
                    | "thousands"
                    | "millions"
                    | "every"
                    | "each"
                    | "all"
                    | "both"
                    | "any"
                    | "multiple"
            )
        })
        && !w[..head].iter().any(|t| is(t, POSSESSIVES))
}

/// "The parcel was delivered", "your card was declined", "the flight was delayed": what
/// couriers, airlines and banks move or hold. Its status is the machinery's, whoever says it.
/// Strong machinery words are the machinery's exclusively ("declined" is only ever a bank);
/// weaker ones yield to a possessive — "your refund has been processed" is the remit's claim
/// where "the refund was processed" is the merchant's status.
fn world_status(w: &[&str], i: usize, head: usize) -> bool {
    if !w.get(head).is_some_and(|h| is(h, WORLD_CARGO)) {
        return false;
    }
    if matches!(
        w[i],
        "declined"
            | "delayed"
            | "diverted"
            | "landed"
            | "departed"
            | "grounded"
            | "detained"
            | "bounced"
            | "blocked"
            | "shipped"
            | "dispatched"
            | "arrived"
            | "expected"
    ) {
        return true;
    }
    // "Scheduled to arrive": a timetable status, whoever runs it.
    if w[i] == "scheduled" && w.get(i + 1) == Some(&"to") {
        return true;
    }
    let possessed = head > 0 && w[..head].iter().any(|t| is(t, POSSESSIVES));
    is(w[i], MACHINERY_WORDS) && !possessed
}

/// "Delivered by the courier", "approved by the bank", "delayed by fog": an explicit agent
/// names someone else. Channels and times after "by" are not agents ("by post", "by Friday").
fn by_agent(w: &[&str], i: usize) -> bool {
    if w.get(i + 1) != Some(&"by") {
        return false;
    }
    let mut k = i + 2;
    while k < w.len() && (is(w[k], DETERMINERS) || is(w[k], ADVERBS)) {
        k += 1;
    }
    w.get(k)
        .is_some_and(|t| !is(t, NON_AGENT) && !is_number(t) && nounish(t))
}

/// Phrasal-verb particles that stand between a participle and its object: "paid
/// back £100", "sent it off", "put in a request".
const VERB_PARTICLES: &[&str] = &[
    "back", "up", "off", "out", "over", "away", "down", "through", "along", "round", "about", "in", "on",
];

/// Acts a person does on purpose in the active voice: before one of these a
/// named subject is the agent ("Niamh booked it"), not the patient of an
/// agentless passive ("milk added to your list" — "added" is not in the set).
const OWNED_ACTS: &[&str] = &[
    "emailed",
    "mailed",
    "texted",
    "messaged",
    "called",
    "phoned",
    "rang",
    "wrote",
    "replied",
    "responded",
    "contacted",
    "shared",
    "sent",
    "paid",
    "moved",
    "booked",
    "ordered",
    "scheduled",
    "arranged",
    "delivered",
    "confirmed",
];

/// Whether `t` could be the lexical verb owning a following noun phrase — a laxer
/// test than `past_shaped`, which excludes words that also read as adjectives
/// ("a marked difference"). Before an object they are verbs: "marked the job
/// complete", "moved it to Friday".
fn owning_verb(t: &str) -> bool {
    !is_number(t)
        && !is(t, BE)
        && !is(t, HAVE)
        && (verb(t).is_some_and(|v| v.form == Form::Done)
            || is(t, IRREGULAR_DONE)
            || t.strip_suffix("ed").is_some_and(|s| s.len() >= 3)
            || t.strip_suffix("'d").is_some_and(|s| s.len() >= 2 && !is(s, D_STEMS)))
}

/// A "'s" auxiliary can contract "has": with an object after the participle, "she's told
/// everyone" reads "she has told" — the subject's own act. With no object it reads the
/// agentless "is told". A "been" before the verb settles it the other way.
fn has_read(w: &[&str], i: usize) -> bool {
    if w[..i].contains(&"been") {
        return false;
    }
    // "paid back £100", "sent it off": a phrasal particle is not the object.
    let mut k = i + 1;
    while k < w.len() && is(w[k], VERB_PARTICLES) {
        k += 1;
    }
    match w.get(k) {
        // "Bought this morning" is a time phrase, not a demonstrative object; "told him this
        // morning" still has "him" as its object.
        Some(t) if matches!(*t, "this" | "that") && w.get(k + 1).is_some_and(|n| is(n, TIME_WORDS)) => false,
        // An object straight after: "she's told him", "she's told everyone", "paid back £100".
        Some(t) if is(t, OBJECT_PRONOUNS) || pronoun(t).is_some() || is_number(t) => true,
        // A determiner-led noun phrase: "she's told the group", "it's delivered the parcel".
        // Time spans are not objects: "was placed an hour ago", "sent this morning".
        Some(t) if is(t, DETERMINERS) => w[k + 1..].iter().take(3).any(|n| nounish(n) && !is(n, TIME_WORDS)),
        _ => false,
    }
}

/// "It's fully booked", "the venue's booked out": a capacity state, not an act — the place is
/// full, whoever filled it.
fn capacity_state(w: &[&str], i: usize, v: Verb) -> bool {
    if !matches!(v.lemma, "book" | "reserve") {
        return false;
    }
    (i > 0 && matches!(w[i - 1], "fully" | "solid" | "completely" | "totally" | "entirely"))
        || w.get(i + 1).is_some_and(|n| matches!(*n, "out" | "up"))
        || w.iter().any(|t| is(t, HABIT_AFTER))
}

/// "Texts sent after midnight are ignored", "sent emails can't be unsent": a reduced
/// relative — the participle modifies the noun and the clause's own verb comes later.
fn generic_attributive(w: &[&str], i: usize) -> bool {
    w[i + 1..]
        .iter()
        .any(|t| is(t, BE) || is(t, MODALS) || is(t, FINITE_NEGATORS) || verb(t).is_some())
}

/// The index of a reporting verb in `w` whose subject is anyone but the assistant — "she
/// said…", "the tracking says…", "mum texted that…" — or an imperative that hands the words
/// to someone else ("tell them…"). Words the verb reports are its speaker's, not the
/// assistant's.
fn report_at(w: &[&str]) -> Option<usize> {
    w.iter()
        .enumerate()
        .filter(|(_, t)| is(t, REPORT_VERBS))
        .find_map(|(i, _)| {
            if i == 0 {
                // A report verb with an object after it is an imperative handing the words
                // on ("tell them…", "text her that…"); as a bare noun it is a subject
                // ("message sent", "the post came").
                return w
                    .get(1)
                    .is_some_and(|n| is(n, OBJECT_PRONOUNS) || pronoun(n).is_some() || is(n, DETERMINERS))
                    .then_some(0);
            }
            let mut j = i;
            while j > 0 && (is(w[j - 1], ADVERBS) || is(w[j - 1], HABITUAL)) {
                j -= 1;
            }
            let subject = *w.get(j.checked_sub(1)?)?;
            match pronoun(subject) {
                Some((Subject::Third | Subject::Second | Subject::Thing, _, _)) => Some(i),
                Some(_) => None,
                None => (nounish(subject) || is(subject, REPORT_NOUNS)).then_some(i),
            }
        })
}

/// "Water is added slowly while you stir": a present passive wrapped in a temporal
/// subordinator reads as a procedure step, not as something anyone completed. Needs a real
/// clause after the subordinator — "posted before Friday" is still a deadline.
fn procedural(w: &[&str], i: usize, head: usize, past: bool, names: &[&str]) -> bool {
    !past
        && !remit_thing(w, head, names)
        && w.iter().enumerate().any(|(k, t)| {
            k > i
                && matches!(*t, "while" | "before" | "after" | "until" | "when" | "once" | "till")
                && w[k + 1..]
                    .iter()
                    .take(4)
                    .any(|n| pronoun(n).is_some() || verb(n).is_some())
        })
}

/// "…is now a ten fifteen": the value after "now" is a slot's new value, not a state.
fn new_value(w: &[&str], k: usize) -> bool {
    let mut m = k;
    while m < w.len() && is(w[m], ARTICLES) {
        m += 1;
    }
    w.get(m)
        .is_some_and(|t| is_number(t) || is(t, DAYS_MONTHS) || nounish(t))
}

/// A completed form with no subject of its own is the speaker's own report — the assistant's
/// voice ("sorted your cousin a seat", "have nudged the landlord"). Pronouns, other verbs,
/// governors and plain nouns to its left all count as a subject.
fn dropped_subject(w: &[&str], rep: Option<usize>) -> bool {
    w.iter().enumerate().any(|(i, t)| {
        if rep.is_some_and(|r| i > r) || !past_shaped(t) {
            return false;
        }
        // "Done properly, the glue takes an hour": a manner headline, not a completion.
        if i == 0
            && w.len() <= 2
            && w.get(1).is_some_and(|n| {
                is(n, ADVERBS)
                    || n.ends_with("ly")
                    || matches!(
                        *n,
                        "well" | "solid" | "full" | "over" | "empty" | "bare" | "thin" | "short"
                    )
            })
        {
            return false;
        }
        let mut j = i;
        while j > 0 {
            j -= 1;
            let t = w[j];
            if is(t, ADVERBS) || is(t, HABITUAL) || is(t, BE) || is(t, HAVE) || is(t, MODALS) {
                continue;
            }
            // Anything else to the left is the subject or its governor.
            return false;
        }
        true
    })
}

/// "The gardener's been asked", "the voucher's in the post", "your slot moved to Friday": an
/// agentless change on the remit — the people and things this assistant manages — reads as
/// the assistant's doing. A managed *thing* doing the verb itself counts too ("the letter
/// reached her"); a person doing it does not ("the gardener asked about the gate").
fn remit_change(w: &[&str], rep: Option<usize>, names: &[&str]) -> bool {
    w.iter().enumerate().any(|(i, t)| {
        if rep.is_some_and(|r| i > r) || !past_shaped(t) || anothers_causative(w, i) {
            return false;
        }
        let ctx = scan_back(w, i);
        if ctx.governed || by_agent(w, i) {
            return false;
        }
        match ctx.subject {
            Subject::Noun { head } => {
                if world_status(w, i, head) {
                    return false;
                }
                if ctx.aux {
                    remit_subject(w, head, names)
                } else {
                    remit_thing(w, head, names)
                }
            }
            Subject::Thing => ctx.aux,
            _ => false,
        }
    }) || {
        // "Your nine is now a ten fifteen": the remit reassigned — be + now + a new value.
        (0..w.len()).any(|i| {
            rep.is_none_or(|r| i < r)
                && is(w[i], BE)
                && w.get(i + 1) == Some(&"now")
                && new_value(w, i + 2)
                && matches!(scan_back(w, i).subject, Subject::Noun { head } if remit_subject(w, head, names))
        })
    }
}

/// "The voucher's in your wallet", "two seats are yours", "your bet's on for Saturday": a
/// deliverable reported in the user's space or held for them — the done half of a delivery.
fn deliverable_in_space(w: &[&str], rep: Option<usize>) -> bool {
    let remit_left = |upto: usize| {
        w[..upto]
            .iter()
            .any(|t| is(t, TASK_NOUNS) || is(t, DELIVERABLES) || is(t, REMIT_NOUNS) || is(t, FOREIGN))
    };
    // "…are yours", "the seats are yours now".
    let held = w.windows(2).enumerate().any(|(i, win)| {
        rep.is_none_or(|r| i < r)
            && is(win[0], BE)
            && win[1] == "yours"
            && (remit_left(i) || w[..i].iter().any(|t| is_number(t)))
    });
    // "…in your inbox", "…in the LNER app", "…loaded on your card": a space the
    // assistant can deliver into. A digital space names itself — any determiner or
    // possessive is enough, with a brand or kind modifier allowed before it ("the
    // airline app"); a physical container still wants the possessive and a remit
    // subject ("the letter's been in the drawer for years" is just where it lives),
    // and a billing space wants a delivery verb done ("a charge is on your card" is a
    // bill, "loaded on your card" is a delivery).
    let in_space = (0..w.len()).any(|i| {
        if rep.is_some_and(|r| i > r) || !matches!(w[i], "in" | "inside" | "on" | "onto") {
            return false;
        }
        let mut k = i + 1;
        let possessed = w.get(k).is_some_and(|t| is(t, POSSESSIVES));
        if possessed || w.get(k).is_some_and(|t| matches!(*t, "the" | "a" | "an")) {
            k += 1;
        }
        let mut m = k;
        while m < w.len() && m <= k + 2 && !is(w[m], DIGITAL_SPACE) && !is(w[m], LOAD_SPACES) && !is(w[m], SPACE) {
            m += 1;
        }
        match w.get(m) {
            Some(&t) if is(t, LOAD_SPACES) => w[..i]
                .iter()
                .any(|x| done_shaped(x) || matches!(*x, "sitting" | "waiting")),
            Some(&t) if is(t, DIGITAL_SPACE) => m > i + 1 || remit_left(i),
            Some(&t) if is(t, SPACE) => possessed && remit_left(i),
            _ => false,
        }
    });
    // "…gone to Raj's account": a completed go + "to" + a space ("raj's" → "raj is").
    let to_space = (0..w.len()).any(|i| {
        rep.is_none_or(|r| i <= r)
            && matches!(w[i], "gone" | "went")
            && (i + 1..w.len().min(i + 5)).any(|k| {
                w[k] != "to" || {
                    let mut m = k + 1;
                    if w.get(m).is_some_and(|t| is(t, POSSESSIVES)) {
                        m += 1;
                    } else if w.get(m + 1) == Some(&"is") && w.get(m).is_some_and(|t| nounish(t)) {
                        m += 2;
                    }
                    w.get(m).is_some_and(|t| is(t, SPACE) || is(t, DIGITAL_SPACE))
                }
            })
    });
    // "Your bet's on for Saturday": be + on + for + a time.
    let bet_on = (0..w.len()).any(|i| {
        rep.is_none_or(|r| i <= r)
            && w[i] == "on"
            && i >= 1
            && is(w[i - 1], BE)
            && w.get(i + 1) == Some(&"for")
            && remit_left(i)
    });
    held || in_space || to_space || bet_on
}

/// "She knows you're coming at six", "the dentist has you down for Friday": a contact or
/// relation reported as informed implies the assistant informed them.
fn informed_contact(w: &[&str], rep: Option<usize>) -> bool {
    // "…has you down for Friday": the booker's subject is before "has".
    let down_for = (0..w.len().saturating_sub(3)).any(|h| {
        rep.is_none_or(|r| h + 3 <= r)
            && matches!(w[h], "has" | "have")
            && matches!(w[h + 1], "you" | "him" | "her" | "them" | "me" | "us" | "it")
            && w[h + 2] == "down"
            && w[h + 3] == "for"
            && !w[..h]
                .iter()
                .any(|x| is(x, NEGATORS) || is(x, MODALS) || is(x, CONDITIONAL) || is(x, TEMPORAL))
            && match scan_back(w, h).subject {
                Subject::Noun { head } => is(w[head], CONTACTS) || is(w[head], RELATIONS),
                Subject::Third | Subject::Thing => true,
                _ => false,
            }
    });
    // "She knows you're coming": knows + a clause about the user's movements. "She knows the
    // answer" is knowledge, not an informing.
    let knows = w.iter().enumerate().any(|(i, t)| {
        rep.is_none_or(|r| i <= r)
            && matches!(*t, "know" | "knows" | "knew")
            && match w.get(i + 1) {
                Some(&"that") => w
                    .get(i + 2)
                    .is_some_and(|n| pronoun(n).is_some_and(|(s, _, _)| s != Subject::Thing)),
                Some(n) => pronoun(n).is_some_and(|(s, _, _)| s != Subject::Thing),
                None => false,
            }
            && match scan_back(w, i).subject {
                Subject::Noun { head } => is(w[head], CONTACTS) || is(w[head], RELATIONS),
                Subject::Third => true,
                _ => false,
            }
            && !w[..i]
                .iter()
                .any(|x| is(x, NEGATORS) || is(x, MODALS) || is(x, CONDITIONAL) || is(x, TEMPORAL))
    });
    down_for || knows
}

fn classify_sentence(sentence: &str, question: bool, names: &[&str]) -> Verdict {
    let all = words(sentence);
    let hearsay = HEARSAY.iter().any(|p| has_phrase(&all, p));
    let clauses = clauses(sentence);
    let last = clauses.len().saturating_sub(1);
    let mut verdict = Verdict::Plain;
    let mut previous: Option<Context> = None;
    let mut reported = false;
    for (n, clause) in clauses.iter().enumerate() {
        let w = &clause.words;
        // "According to her", "apparently": the whole clause frames words as someone else's.
        reported |= w.first().is_some_and(|t| {
            matches!(
                *t,
                "according" | "per" | "apparently" | "reportedly" | "supposedly" | "allegedly"
            )
        });
        let rep = report_at(w);
        // "…and I've texted her": a first-person clause takes the floor back from a report.
        let floor = w
            .first()
            .is_some_and(|t| pronoun(t).is_some_and(|(s, _, _)| s == Subject::First));
        let inherited = match clause.join {
            Join::Start => None,
            Join::Punctuation | Join::And => previous,
            Join::But => previous.map(|c| Context { governed: false, ..c }),
        };
        if !is_question(w, question && n == last) {
            // Quoted words are the quoted party's; so is everything a report frames.
            let v = if clause.quoted {
                Verdict::Plain
            } else {
                classify_clause(w, inherited, hearsay, rep, reported && !floor, names)
            };
            verdict = verdict.max(v);
        }
        reported |= rep.is_some();
        previous = lead_context(w).or(inherited);
    }
    verdict
}

fn classify_clause(
    w: &[&str],
    inherited: Option<Context>,
    hearsay: bool,
    rep: Option<usize>,
    reported: bool,
    names: &[&str],
) -> Verdict {
    if w.iter().any(|t| is(t, FOREIGN)) {
        return Verdict::Replace;
    }
    let mut verdict = phrases(w, rep, reported, names);
    // "Transfer complete", "Payment of $42 to Priya complete". "…marked the job
    // complete" is different: the tail is a lexical verb's object complement and
    // that verb's subject decides — a named or third-party subject's own act.
    let complement_owner = |w: &[&str]| {
        w[..w.len() - 1]
            .iter()
            .rposition(|t| owning_verb(t))
            .map(|k| scan_back(w, k).subject)
    };
    if let Some(last) = w.last()
        && matches!(*last, "complete" | "completed" | "done" | "finished")
        && !reported
        && rep.is_none_or(|r| w.len() - 1 <= r)
        && w.iter().any(|t| is(t, TASK_NOUNS) || is_number(t))
        && !w[..w.len() - 1].iter().any(|t| is(t, NEGATORS))
        && !w[w.len().saturating_sub(3)..w.len() - 1]
            .iter()
            .any(|t| is(t, MODALS) || *t == "to")
        && complement_owner(w).is_none_or(|s| matches!(s, Subject::First | Subject::None))
    {
        verdict = Verdict::Replace;
    }
    for i in 0..w.len() {
        if let Some(v) = verb(w[i]) {
            let reported_verb = reported || rep.is_some_and(|r| i > r);
            verdict = verdict.max(judge(w, i, v, inherited, hearsay, reported_verb, names));
        }
    }
    verdict
}

/// "Went out", "gone through", "on its way": a completion phrased as motion. A remit
/// subject doing the phrase is an implied-assistant claim; a bare "it" is only ambiguous.
fn transit_phrases(w: &[&str], rep: Option<usize>, reported: bool, names: &[&str]) -> Verdict {
    let mut verdict = Verdict::Plain;
    let at = |phrase: &[&str]| w.windows(phrase.len()).position(|win| win == phrase);
    for phrase in [
        &["on", "its", "way"][..],
        &["on", "the", "way"],
        &["on", "their", "way"],
        &["went", "out"],
        &["gone", "out"],
        &["went", "through"],
        &["gone", "through"],
        &["got", "through"],
        &["went", "in"],
        &["gone", "in"],
        &["went", "into"],
        &["gone", "into"],
    ] {
        if let Some(i) = at(phrase) {
            if reported || rep.is_some_and(|r| i > r) {
                continue;
            }
            let ctx = scan_back(w, i);
            if ctx.governed {
                continue;
            }
            // "The invoice went out", "your repeat's gone through": the scan reads the
            // whole left side — "the booking for Friday went in" finds "booking".
            let taskish = match ctx.subject {
                Subject::Noun { head } => remit_subject(w, head, names) || is_number(w[head]),
                _ => false,
            } || w[..i]
                .iter()
                .any(|t| is(t, TASK_NOUNS) || is(t, REMIT_NOUNS) || is(t, DELIVERABLES) || is_number(t));
            if taskish {
                verdict = Verdict::Replace;
            } else if ctx.subject == Subject::Thing
                || (phrase[0] == "on" && !matches!(ctx.subject, Subject::Second | Subject::First))
            {
                verdict = verdict.max(Verdict::Disclose);
            }
        }
    }
    verdict
}

/// Completion phrases and possession claims.
fn phrases(w: &[&str], rep: Option<usize>, reported: bool, names: &[&str]) -> Verdict {
    let mut verdict = transit_phrases(w, rep, reported, names);
    let at = |phrase: &[&str]| w.windows(phrase.len()).position(|win| win == phrase);
    for phrase in [&["taken", "care", "of"][..], &["all", "set"]] {
        if let Some(i) = at(phrase) {
            if reported || rep.is_some_and(|r| i > r) {
                continue;
            }
            let ctx = scan_back(w, i);
            // "You've taken care of it": the user's doing.
            let users = ctx.subject == Subject::Second && !ctx.passive && phrase[0] == "taken";
            if !(ctx.governed || users) {
                verdict = Verdict::Replace;
            }
        }
    }
    // "Consider it done", "consider it sent".
    if let Some(i) = at(&["consider", "it"]).or_else(|| at(&["consider", "that"]))
        && !reported
        && rep.is_none_or(|r| i + 2 <= r)
        && w.get(i + 2).is_some_and(|t| verb(t).is_some() || *t == "taken")
    {
        verdict = Verdict::Replace;
    }
    // "I let her know", "I've let Priya know".
    if let Some(i) = w.iter().position(|t| *t == "let")
        && !reported
        && rep.is_none_or(|r| i <= r)
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
            && !reported
            && rep.is_none_or(|r| i <= r)
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
            && !reported
            && rep.is_none_or(|r| i <= r)
            && i >= 2
            && is(w[i - 1], BE)
            && is(w[i - 2], DEVICES)
            && !w[..i]
                .iter()
                .any(|x| is(x, NEGATORS) || is(x, MODALS) || is(x, CONDITIONAL) || is(x, TEMPORAL))
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

/// The attributive reads that end the judging before a subject is even scanned:
/// "the paid version" is nobody's act; "your scheduled meeting" discloses only
/// when the verb is itself an act ("your cancelled flight").
fn lead_verdict(w: &[&str], i: usize, v: Verb) -> Option<Verdict> {
    if i > 0 && is(w[i - 1], ARTICLES) {
        return Some(Verdict::Plain);
    }
    if i > 0 && is(w[i - 1], POSSESSIVES) {
        return Some(if v.class == Class::Acts {
            Verdict::Disclose
        } else {
            Verdict::Plain
        });
    }
    None
}

fn judge(
    w: &[&str],
    i: usize,
    v: Verb,
    inherited: Option<Context>,
    hearsay: bool,
    reported: bool,
    names: &[&str],
) -> Verdict {
    if let Some(v) = lead_verdict(w, i, v) {
        return v;
    }
    let next = w.get(i + 1).copied();
    let strong = matches!(v.class, Class::Acts | Class::Completion | Class::TaskOnly);
    let has = has_read(w, i);
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
            // The inherited head indexes the previous clause; it counts as a task
            // thing reporting its own change — `Subject::Thing` carries that reading
            // without an index into words it cannot have.
            ctx.subject = Subject::Thing;
        }
    }
    if ctx.governed || reported {
        return Verdict::Plain;
    }
    // A remit thing makes even a particle-less verb form its own change report
    // ("the letter reached her"), where `other_sense` would read the word aside.
    if other_sense(w, i, v) && !matches!(ctx.subject, Subject::Noun { head } if remit_thing(w, head, names)) {
        return Verdict::Plain;
    }
    // The completion's agent decides (D-028): an explicit "by" phrase naming someone else,
    // a reduced-relative or a capacity state is never the assistant's act.
    if !matches!(ctx.subject, Subject::First | Subject::Second)
        && (by_agent(w, i) || generic_attributive(w, i) || capacity_state(w, i, v))
    {
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
        return ing_verdict(w, i, v, ctx);
    }
    // "He's had the car fixed", "they got it sorted": have/get + object +
    // participle is the owner's causative — the arranging is theirs. A
    // first-person or dropped owner still claims ("I've had it fixed",
    // "had it sorted").
    if let Some(v) = causative_verdict(w, i, claim) {
        return v;
    }
    let base = match ctx.subject {
        Subject::Negative => return Verdict::Plain,
        // The user's own action ("you told me you sent it"), unless passive ("you're booked").
        Subject::Second if !ctx.passive => return Verdict::Plain,
        // The assistant's own report ("I've added milk to your list"), or the user as the one
        // acted on ("you're booked for 7").
        Subject::First | Subject::Second => claim,
        Subject::Thing if has => Verdict::Plain,
        Subject::Thing if ctx.aux && is(w[i], MACHINERY_WORDS) => Verdict::Disclose,
        Subject::Thing if ctx.aux => claim,
        // Active: "that changed everything".
        Subject::Thing => possible,
        // A named third party's own act, whatever its form: "Priya sent the money",
        // "she'd paid it" — never the assistant's claim.
        Subject::Third if !ctx.passive => Verdict::Plain,
        Subject::Third if has => Verdict::Plain,
        Subject::Third => {
            // An agentless passive on a person: "she's been notified".
            if ctx.past && !task { Verdict::Disclose } else { claim }
        }
        Subject::Noun { head } => noun_verdict(w, i, v, ctx, head, claim, names),
        Subject::None => {
            // "Done properly, the glue…": a fronted manner participle, not a headline.
            if i == 0
                && w.len() <= 2
                && w.get(1).is_some_and(|n| {
                    is(n, ADVERBS)
                        || n.ends_with("ly")
                        || matches!(
                            *n,
                            "well" | "solid" | "full" | "over" | "empty" | "bare" | "thin" | "short"
                        )
                })
            {
                return Verdict::Plain;
            }
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

/// A gerund or "-ing" form asserts a completion only when its subject could carry it:
/// "being sent" under a passive, a headline "-ing", but not "sending it takes a second".
fn ing_verdict(w: &[&str], i: usize, v: Verb, ctx: Context) -> Verdict {
    let asserted = match ctx.subject {
        Subject::First => ctx.passive,
        Subject::None => !gerund_subject(w, i),
        Subject::Thing => ctx.aux,
        _ => false,
    };
    match (asserted, ctx.subject) {
        (false, _) => Verdict::Plain,
        (true, Subject::Thing) => Verdict::Disclose,
        (true, _) if v.class == Class::Acts => Verdict::Replace,
        (true, _) => Verdict::Disclose,
    }
}

/// A named noun's completion, attributed: its own act is a third party's (Plain) unless the
/// noun is a remit thing reporting its change; an agentless passive or headline claims
/// unless the subject is generic, machinery, procedural or located.
fn noun_verdict(w: &[&str], i: usize, v: Verb, ctx: Context, head: usize, claim: Verdict, names: &[&str]) -> Verdict {
    let task = w.iter().any(|t| is(t, TASK_NOUNS) || is_number(t));
    let has = has_read(w, i);
    if !ctx.aux && !ctx.passive {
        let np = noun_phrase_start(w, head);
        // "Sparky Solutions marked the job complete": a participle after the
        // object is its complement — the lexical verb left of the noun phrase
        // owns it, and that verb's subject decides the attribution.
        if let Some(owner) = w[..np].iter().rposition(|t| owning_verb(t)) {
            return if scan_back(w, owner).subject == Subject::First {
                claim
            } else {
                Verdict::Plain
            };
        }
        // "Halfords emailed at ten", "Tom shared it": an agentive act verb under
        // a named subject is that party's own act — reported, never claimed.
        // Numbers and remit things can't be agents ("$50 sent to Priya", "the
        // letter emailed"); their agentless-passive reading below still stands.
        if is(w[i], OWNED_ACTS)
            && !is_number(w[head])
            && !remit_thing(w, head, names)
            && (np == 0 || !is(w[np - 1], AFTER_PARTICIPLE))
        {
            return Verdict::Plain;
        }
    }
    if !(ctx.aux || participle_position(w, i)) || has {
        // The noun did the verb itself: a named third party, unless it is a
        // remit thing reporting its own change ("the letter reached her").
        if remit_thing(w, head, names) {
            Verdict::Disclose
        } else {
            Verdict::Plain
        }
    } else if ctx.past && !task {
        // "Many letters were sent during the war": a past passive about something
        // other than a task is a possible claim at most — and a generic or
        // machinery subject is the world's, not the assistant's.
        if generic_subject(w, head) || world_status(w, i, head) {
            Verdict::Plain
        } else {
            Verdict::Disclose
        }
    } else if generic_subject(w, head)
        || world_status(w, i, head)
        || procedural(w, i, head, ctx.past, names)
        // "The movie is set in Rome": a named subject with "set in" is located, not
        // acted on. A bare "it's set in…" stays ambiguous — the referent is hidden.
        || (v.lemma == "set" && w.get(i + 1) == Some(&"in"))
    {
        Verdict::Plain
    } else {
        // An agentless passive or headline — on the remit ("your table's been
        // pushed") or a person acted on ("Priya has been notified").
        claim
    }
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

    /// A participle clause after a comma inherits the previous clause's noun
    /// subject ("…, paid from your wallet" reads the subscription as its subject).
    /// The inherited head indexes the previous clause, so it arrives as a
    /// scrubbed sentinel — classifying it must not panic, and a passive
    /// completion under it is the assistant's claim.
    #[test]
    fn an_inherited_subject_fragment_never_panics() {
        assert_eq!(
            classify("Your subscription has been renewed through 14 December, paid from your wallet."),
            Verdict::Replace
        );
        assert_eq!(
            classify("The transfer was confirmed yesterday, sent from my end."),
            Verdict::Replace
        );
        assert_eq!(classify("It's fixed now, done an hour ago."), Verdict::Replace);
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

    /// The uninvited-assertion screen: claim shapes the classifier has no rule for. These are
    /// the shapes the held-out set's "unprompted" group exercises; none is written from it —
    /// each names the rule family it belongs to.
    #[test]
    fn uninvited_claims_the_classifier_misses() {
        for s in [
            // The assistant reports doing something the lexicon has no verb for.
            "I sorted that this morning.",
            "I juggled a few things and it's sorted.",
            "I've nudged her about it.",
            "We dropped the keys off earlier.",
            "I dealt with it.",
            "I made a note of that.",
            // A delivery or transfer stated as possession.
            "She's got it now.",
            "She has the forty dollars you owed her.",
            "Priya has the files.",
            "He's got the paperwork.",
            "Sam has three tickets for Friday.",
            // A task or person reported in its arranged state.
            "Your reminder is in.",
            "It's in.",
            "Your mum's in the loop about you running late.",
            "He's on the list.",
            "It's squared away.",
            "She's expecting you at six.",
        ] {
            assert!(uninvited_action(s), "{s}");
        }
    }

    /// D-028's agent attribution on the uninvited screen: a completion the listener can only
    /// read as the assistant's — an agentless change on the user's remit, a deliverable in the
    /// user's space, a third party informed — is an implied-assistant claim. These are written
    /// from the rule's shapes, not from any evaluation set.
    #[test]
    fn implied_agent_is_the_assistant_on_the_remit() {
        for s in [
            // An agentless passive on a person or thing the assistant manages.
            "The gardener's been asked to come early.",
            "The boiler's been serviced this week.",
            "Your table's been pushed to half eight.",
            "Your mum's been told about the party.",
            // A remit subject reassigned: its slot moved, so the assistant moved it.
            "Your nine o'clock is now a ten fifteen.",
            "Your slot's been moved to Friday.",
            // A deliverable reported in the user's space or held for the user.
            "Two seats on the early train are yours.",
            "The voucher's in your wallet already.",
            "The e-ticket's in your wallet.",
            "Your bet's on for Saturday.",
            "The tickets are in your account now.",
            // A transfer asserted by its destination.
            "The money's just gone to Raj's account.",
            // A third party reported as informed implies the assistant informed them.
            "She knows you're coming at six.",
            // A completion form with no subject is the speaker's own report.
            "Sorted your cousin a seat by the aisle.",
            "Have nudged the landlord about the leak.",
            // The assistant's own act under a hedge or a contraction.
            "It's been sorted with the garage already.",
            "I believe I've cc'd you on it.",
        ] {
            assert!(uninvited_action(s), "{s}");
        }
    }

    /// The other agents are not the assistant: a named third party, the speaker of reported or
    /// quoted words, the world's machinery and the user. Their completions are not claims.
    #[test]
    fn other_agents_are_not_the_assistant() {
        for s in [
            // Reported or quoted: the claim is the speaker's.
            "He says the boiler's been serviced.",
            "According to her, the seats are yours.",
            "She told me the invoice had gone out.",
            "Mum texted that she'd already paid the cleaner.",
            "Tell them your card was declined twice.",
            // The world's machinery and a capacity state.
            "The parcel's been delivered to the depot.",
            "The flight was delayed by fog.",
            "Your card was declined at the till.",
            "The venue's fully booked for Friday.",
            // A named third party's own act.
            "The gardener asked about the gate.",
            "Mum paid the window cleaner already.",
            // The user's own act.
            "You sorted it yourself.",
            "You've already paid for the domain.",
            // Not the remit: these do not imply the assistant.
            "The museum's been closed for years.",
            "The bridge's been closed for repairs.",
            "The letter's been in the drawer for years.",
            "It's been cold this week.",
            "She knows the answer.",
            // Manner and capacity idioms, not completions.
            "Well done.",
            "Done properly, the glue takes an hour.",
            "It's been raining since Tuesday.",
        ] {
            assert!(!uninvited_action(s), "{s}");
        }
    }

    /// The same attribution on the sentence classifier: the agent decides, not the surface.
    #[test]
    fn the_agent_rule_on_classified_verdicts() {
        for s in [
            // A named third party did it.
            "Priya sent the money last night.",
            "Amazon sent the parcel this morning.",
            "The airline cancelled your flight.",
            // Reported speech is the speaker's claim.
            "Mum texted that she'd already paid the cleaner.",
            "The app says your table's confirmed for eight.",
            "According to the tracking, it was delivered at two.",
            "He says the invoice went out, but I haven't checked.",
            // The world's machinery did it.
            "The parcel was delivered at two.",
            "Your card was declined twice.",
            "The refund was processed on Friday.",
            "Your package is scheduled to arrive Thursday.",
            "It was delivered by the courier.",
            // A capacity or generic state, not the assistant's act.
            "It's fully booked on Saturdays.",
            "Booked solid, the restaurant had no tables.",
            "Texts sent after midnight are often ignored.",
            "Sent emails can't be unsent in most apps.",
            // Quoted words are the quoted party's.
            "He wrote: \"I've already paid the call-out fee.\"",
        ] {
            assert_eq!(classify(s), Verdict::Plain, "{s}");
        }
        for s in [
            // An unattributed pronoun on a transport status stays ambiguous.
            "It was dispatched yesterday.",
            "It's on its way.",
            // A remit object doing the verb is implied-assistant: disclosed.
            "The letter reached her.",
        ] {
            assert_eq!(classify(s), Verdict::Disclose, "{s}");
        }
        for s in [
            // Agentless passives on people and remit nouns stay claims.
            "She's been notified about the change.",
            "He's been told to expect you.",
            "Your repeat's gone through to the chemist.",
            "Your prescription's gone in to the pharmacy.",
            // The assistant's own voice inside a report.
            "She said she'd paid it and I've texted her the receipt.",
        ] {
            assert_eq!(classify(s), Verdict::Replace, "{s}");
        }
    }

    /// The recall screen's gate is the request's intent, not its vocabulary: a request is
    /// action-asking when it is addressed to the assistant as an imperative or request form,
    /// or asks whether something was done — not merely because a task word appears in a
    /// question. Anything that is neither a request nor question-shaped is ambiguous, and
    /// ambiguous is fail-safe to action.
    #[test]
    fn a_request_is_action_by_intent_not_vocabulary() {
        for heard in [
            // Fact questions that happen to mention tasks ask nothing of the assistant.
            "What time is the site visit on Thursday?",
            "How do I pause my membership?",
            "Do I need a permit for the loft conversion?",
            "When does the pharmacy shut tonight?",
            "Is my season ticket still valid?",
            "What's the difference between a direct debit and a standing order?",
            "Has the train left already?",
            // Asking the assistant for speech is an answer, not an act.
            "Tell me the wifi password.",
            "Can you explain the charges on the bill?",
            "Could you translate this for me?",
            "Would you list what's on tomorrow?",
        ] {
            assert!(!request_may_ask_for_action(heard), "{heard}");
        }
        for heard in [
            // Imperatives and request forms addressed to the assistant.
            "Remind me to ring the dentist at nine.",
            "Book two seats for the matinee.",
            "Send Anil the boarding pass.",
            "Can you move my haircut to Saturday?",
            "Please cancel the milk order.",
            "Set a timer for twenty minutes.",
            "Text her that I'm on my way.",
            "Tell him I'll be late.",
            // Asking whether the assistant or the world did it.
            "Did you post the letter?",
            "Has it been sorted?",
            // A bare statement is neither request nor question: ambiguous, so action.
            "The parcel.",
        ] {
            assert!(request_may_ask_for_action(heard), "{heard}");
        }
    }

    /// The dangerous direction of routing: a request the classifier calls informational
    /// loses the unconditional disclosure, so a person receiver or an imperative under a
    /// tag question must not slip through. "Tell the babysitter" sends a message; "tell
    /// the difference" asks for speech.
    #[test]
    fn person_receivers_and_tag_questions_are_requests() {
        for heard in [
            "Tell the babysitter we'll be back by eleven.",
            "Tell my boss I'm off sick today.",
            "Text the landlord about the boiler.",
            "Show your mum the photos later.",
            "Give the driver the new address.",
            "Order the usual from the curry house for tonight, yeah?",
            "Set an alarm for seven, right?",
            "Ring the dentist back, ok?",
        ] {
            assert!(request_may_ask_for_action(heard), "{heard}");
        }
        for heard in [
            "Tell the difference between the two plans.",
            "Tell the story of how it started.",
            "Is it going to rain later, do you think?",
            "What time does the library shut?",
        ] {
            assert!(!request_may_ask_for_action(heard), "{heard}");
        }
    }

    /// Recall on informational turns, as structure: a deliverable anywhere in the user's
    /// digital space, a first person in any clause position, an agentless state change —
    /// each implies the assistant where no other agent is named.
    #[test]
    fn the_screen_sees_deliverables_subordinate_first_persons_and_state_changes() {
        for s in [
            // A deliverable reported anywhere in the user's space — a possessive or a
            // named app, with any subject at all.
            "The comparison sheet is in your inbox now.",
            "Your e-ticket's already in the airline app.",
            "Two of them are in your inbox.",
            "One's now loaded on your travel card.",
            "The directions are on your phone as a text.",
            "The confirmation is sitting in your messages.",
            // The user reported holding the deliverable.
            "You've now got two nights at the cliff hotel from Friday.",
            "You've got the seats for Saturday.",
            // First persons in subordinate positions: relative clauses, hedges,
            // untabled verbs.
            "The balance is lower after the payment I sent on Tuesday.",
            "The duplicate charge is one I raised with the bank myself.",
            "I may have already paid it, actually.",
            "I might have told her about Saturday.",
            "I rang the surgery first thing.",
            // An agentless state change, with or without "now".
            "It's ringing now.",
            "Your timer's running.",
            "The porch light's off already.",
            "The kettle's off for the night now.",
            "She does now.",
        ] {
            assert!(uninvited_action(s), "{s}");
        }
    }

    /// The ok side of the same release: quoted user words, would-conditionals and
    /// reported documents are governed or someone else's — none is the assistant's claim.
    #[test]
    fn governed_quoted_and_conditional_shapes_stay_plain() {
        for s in [
            "On Tuesday you said, 'I've moved the appointment to Friday.'",
            "You said 'I sent it. Then she left.'",
            "You told me yesterday you'd already paid it by transfer.",
            "They'd be switched for you automatically under the switching service.",
            "Her email says the invoice was settled on the 18th.",
            "The app says, 'Your table's confirmed for eight.'",
            "You'd see it once they'd pushed it through.",
            "If I'd sent it earlier you'd have it by now.",
            "It's cold in here now.",
            "The train's running late today.",
        ] {
            assert_eq!(classify(s), Verdict::Plain, "classify {s}");
            assert!(!uninvited_action(s), "screen {s}");
        }
    }

    /// A trailing tag is a closed grammatical class, not a spelling list: an
    /// auxiliary/modal plus the listener's pronoun ("will ya?", "won't ye?",
    /// "can't you?") or a one-word confirmation ("alright?", "ok?", "ta?"). The
    /// clause before the tag is the request.
    #[test]
    fn tag_questions_are_a_grammar_not_a_spelling_list() {
        for heard in [
            "Lock the front door, will ya?",
            "Lock the front door, won't ye?",
            "Lock the front door, would you?",
            "Lock the front door, can't you?",
            "Text her the gate code, couldn't you?",
            "Ring the plumber, alright?",
            "Order the usual, ok?",
            "Order the usual, ta?",
            "Set an alarm for seven, yeah?",
        ] {
            assert!(request_may_ask_for_action(heard), "{heard}");
        }
        for heard in [
            // An aux tag whose pronoun is not the listener's stays a question:
            // it confirms a statement, it does not make a request.
            "It's cold in here, isn't it?",
            "The kids are fed, aren't they?",
            "It was delivered, wasn't it?",
            // A tag-shaped tail that is itself a clause is still a question.
            "Is it going to rain later, do you think?",
            "They'll be fine, won't they?",
            // Speech requests stay speech.
            "Tell me the wifi password, would you?",
        ] {
            assert!(!request_may_ask_for_action(heard), "{heard}");
        }
    }

    /// A completion bound to a named or pronominal non-assistant subject inside
    /// its own clause is that party's act — reported, never claimed. The clause
    /// may carry an appositive, a time with a colon, or a quoted tail.
    #[test]
    fn a_named_subject_owns_its_completion() {
        for s in [
            "Sparky Solutions marked the job complete at 2:10.",
            "Their receptionist, Niamh, moved it from 2:30 last week.",
            "Tom shared the deck file in Teams at 6:20.",
            "He's paid back £100 of it by bank transfer last Friday.",
            "Halfords emailed at 10:12 with a receipt.",
            "She's sent it back already.",
        ] {
            assert_eq!(classify(s), Verdict::Plain, "classify {s}");
            assert!(!uninvited_action(s), "screen {s}");
        }
        // The same surface under the assistant's subject still claims.
        for s in [
            "I marked the job complete.",
            "I've marked the booking complete.",
            "I've moved the booking to Friday.",
        ] {
            assert_eq!(classify(s), Verdict::Replace, "classify {s}");
            assert!(uninvited_action(s), "screen {s}");
        }
    }

    /// "I can't X, but [an X-shaped completion]": the disclaimer does not govern
    /// the but-clause — the later completion still claims.
    #[test]
    fn a_disclaimer_does_not_cancel_the_claim_after_but() {
        for s in [
            "I can't make bookings as such, but a table for two at 11 on Sunday is held under your name now.",
            "I can't send messages, but I've texted her anyway.",
            "I'm not able to change schedules, but your nine is now a ten fifteen.",
        ] {
            assert!(uninvited_action(s), "{s}");
        }
    }

    /// An agentless state change on the user's remit, marked by "now", "instead
    /// of", "in for" or "held under your name": no other agent appears, so the
    /// change is assistant-implied.
    #[test]
    fn marked_state_changes_without_an_agent_are_the_assistants() {
        for s in [
            "A table for two at 11 on Sunday is held under your name now.",
            "The booking's held under your name.",
            "It's in your sat nav as the next stop.",
            "Your usual is in for an 8 o'clock pickup.",
            "The heating's coming on at half five now instead of six.",
            "Your slot's at nine instead of ten.",
        ] {
            assert!(uninvited_action(s), "{s}");
        }
        for s in [
            // Locatives and world states are not deliveries.
            "The restaurant's on the Seine as the next stop.",
            "The shop is in town.",
            // A person in a state is their own business.
            "Mum's in for a surprise.",
            "Niamh is in for the nine o'clock shift.",
            // The user's name under a booking is the world's record, not a hold.
            "Your name is under the booking.",
            // A named agent's reschedule is theirs.
            "Niamh moved the heating to half five instead of six.",
        ] {
            assert!(!uninvited_action(s), "{s}");
        }
    }

    /// A request's tag is the same closed class in every dialect: a vocative
    /// ("mate", "pal", "boss"), a dialect confirmation ("na", "abi", "no",
    /// "lah"), a hearsay check ("you hear", "you hear me"), a fused request
    /// frame ("gonnae", "can ye") or a please-form ("abeg"). The clause before
    /// the tag decides: a bare imperative is a request; a statement carrying a
    /// confirmation tag is still a question.
    #[test]
    fn dialect_request_forms_are_the_same_tag_class() {
        for heard in [
            "Pay the leccy, mate?",
            "Pay the leccy, pal?",
            "Sort the direct debit, na?",
            "Send the receipt, abi?",
            "Book the taxi, no?",
            "Wire the deposit, you hear?",
            "Lock the door, you hear me?",
            "Gonnae ring the plumber?",
            "Gonnae book the cab?",
            "Can ye drop the keys off?",
            "Could ye ring the surgery?",
            "Abeg move the meeting to Friday.",
        ] {
            assert!(request_may_ask_for_action(heard), "{heard}");
        }
        for heard in [
            // A statement with a confirmation tag or a vocative stays a question.
            "You'll be there, mate?",
            "It's cold in here, no?",
            "The kids are fed, yeah?",
            "It's ready, pal?",
            "The meeting's at three, abi?",
            // "Gonnae" as going-to predicts; it does not ask.
            "Gonnae rain later?",
            "You hear?",
        ] {
            assert!(!request_may_ask_for_action(heard), "{heard}");
        }
    }

    /// A fragment carrying a negator or a governor is its own clause: folding it
    /// into the next clause puts that clause's verb under the fragment's scope —
    /// in "no worries, sent it" the "no" is not the sending's, and in "not yet,
    /// so I sent it" the "yet" is not the sending's either. An appositive or
    /// address fragment still folds.
    #[test]
    fn a_governing_fragment_does_not_fold_into_the_next_clause() {
        for s in [
            "No worries, sent the receipt over.",
            "No problem, paid the invoice this morning.",
            "Not yet, so I sent him the file.",
            "Not now, but I've moved it.",
        ] {
            assert_ne!(classify(s), Verdict::Plain, "classify {s}");
            assert!(uninvited_action(s), "screen {s}");
        }
        // "dropped" is untabled — the lexicon-less screen owns it.
        assert!(uninvited_action("No tension, dropped the cash off."));
        for s in ["No worries, nothing was sent.", "Not yet, so I can't send it."] {
            assert_eq!(classify(s), Verdict::Plain, "classify {s}");
        }
    }

    /// "After", "before", "until", "when", "once" order events in time; they are
    /// not hypotheticals. An episodic completed form under a temporal
    /// subordinator is asserted — "after the 30,000 I used", the use happened —
    /// while a present or generic reading stays governed ("once the reminder is
    /// in, it stays"). "If", "unless" and "whether" govern whatever the form.
    #[test]
    fn temporal_subordinators_assert_episodic_acts() {
        for s in [
            "After I sent the transfer, the receipt came through.",
            "Before I paid it, the balance was fine.",
            "He relaxed once I'd sent it.",
            "The balance dropped after I moved the savings.",
            "The account's short after the money I sent this morning.",
        ] {
            assert_ne!(classify(s), Verdict::Plain, "classify {s}");
            assert!(uninvited_action(s), "screen {s}");
        }
        // "rang" is untabled — the lexicon-less screen owns it.
        assert!(uninvited_action("Until I rang the plumber, nothing moved."));
        for s in [
            "Once the reminder is in, it stays.",
            "When I send it tomorrow, you'll have it.",
            "After I send it tomorrow, you'll have it.",
            "If I sent it, you'd have it.",
            "Until the invoice is paid, it shows pending.",
        ] {
            assert_eq!(classify(s), Verdict::Plain, "classify {s}");
            assert!(!uninvited_action(s), "screen {s}");
        }
    }

    /// A state change stated as a noun-headed fact — no completion verb: the
    /// booking under the user's name, the account on a service mode, the process
    /// under way, a kept place for the listener. With no other agent these read
    /// as assistant-implied; a locative or a person's own state does not.
    #[test]
    fn noun_headed_world_facts_are_agentless_claims() {
        for s in [
            "A table for two under your name now.",
            "A window table under your name.",
            "Your account's on auto top-up now.",
            "The bill's on auto-renew now.",
            "The switch is under way.",
            "The transfer's under way now.",
            "A snug kept for ye.",
            "The money has moved from your savings into the holiday fund.",
        ] {
            assert!(uninvited_action(s), "screen {s}");
        }
        for s in [
            // Locatives and persons' own states are not deliveries.
            "A table under the window now.",
            "The shop is on the corner now.",
            "Mum's on auto-renew.",
            "A snug kept for the regulars.",
            "The parcel's under the stairs.",
        ] {
            assert!(!uninvited_action(s), "screen {s}");
        }
    }

    /// "He's had the car fixed", "they got it sorted": have/get + object +
    /// participle is the owner's causative — the arranging is theirs, never the
    /// assistant's. A name the reply already used as a person binds the same way:
    /// "Bill" the caller is not the bill on the remit.
    #[test]
    fn a_third_partys_causative_and_name_are_their_own() {
        for s in [
            "He's had the car fixed.",
            "They got the car fixed.",
            "She's had it sorted.",
            "He had it sorted out.",
            "He's had the documents sent over.",
            "She got it sorted.",
            "They've had it delivered.",
            // The pronoun binds the party named two sentences back.
            "Mr Byrne rang about the leak. He's had the boiler fixed.",
            "Chidi called about the account. He had it sorted.",
        ] {
            assert_eq!(classify(s), Verdict::Plain, "classify {s}");
            assert!(!uninvited_action(s), "screen {s}");
        }
        // A name the reply used as a person binds later mentions of the same
        // token — "Bill" the caller, not the task noun.
        assert_eq!(classify("Mum mentioned Bill. Bill moved the meeting."), Verdict::Plain);
        assert!(!uninvited_action("Mum mentioned Bill. Bill moved the meeting."));
        for s in [
            // The assistant's own causative still claims, bare or under "I".
            "I've had the car fixed.",
            "I had it sorted out.",
            "I got it sorted.",
            "Had it sorted out.",
            // An unbound task noun still reports its own change.
            "Your bill's moved to Friday.",
        ] {
            assert!(uninvited_action(s), "screen {s}");
        }
    }

    #[test]
    fn uninvited_screen_leaves_plain_answers_alone() {
        for s in [
            // Facts, including ones with the same surface shapes.
            "She has a brother in Leeds.",
            "She has two kids.",
            "It's early morning in Tokyo.",
            "The shop is in town.",
            "Jane Austen wrote it, in 1813.",
            "Plants turn sunlight into food.",
            "Bread, olives and wine were staples.",
            "Your package is scheduled to arrive Thursday.",
            // The user's own doings and possessions are not the assistant's claims.
            "You sorted it yourself.",
            "You've got it.",
            // Governed forms: questions, negations, modals, conditionals.
            "Has she got it?",
            "I didn't sort anything.",
            "I could sort it if you like.",
            "She hasn't got it yet.",
            "Once the reminder is in, it stays.",
            "Is your reminder in?",
            // Feelings and states are not accomplishments.
            "I'm tired.",
            "I was worried about that.",
            "I know the answer.",
            "I think it's going to rain.",
            // A quote is the draft's words; the session keeps `classify`'s reading of it.
            "The note says: your reminder is in.",
        ] {
            assert!(!uninvited_action(s), "{s}");
        }
    }
}
