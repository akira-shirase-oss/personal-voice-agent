//! Transcript normalization for D-013's "slot value literally present in the heard text".
//!
//! Both the heard text and a slot value are reduced to the same token form: lowercase,
//! currency symbols and punctuation dropped, digit grouping removed (`1,000` → `1000`),
//! trailing zero decimals dropped (`42.00` → `42`), and English number words turned into
//! digits (`forty two` → `42`). A slot is present when its tokens appear contiguously in
//! the heard tokens. The rule is deliberately literal: a model that "corrects" a name or an
//! amount produces a value that is not present, and the proposal goes to confirmation.

/// Tokens of `text` in normalized form.
pub fn tokens(text: &str) -> Vec<String> {
    let lowered = text.to_lowercase();
    let chars: Vec<char> = lowered.chars().collect();
    let mut cleaned = String::with_capacity(lowered.len());
    for (i, &c) in chars.iter().enumerate() {
        let prev_digit = i > 0 && chars[i - 1].is_ascii_digit();
        let next_digit = chars.get(i + 1).is_some_and(char::is_ascii_digit);
        if c.is_alphanumeric() {
            cleaned.push(c);
        } else if (c == ',' && prev_digit && next_digit) || (c == '\'' && i > 0 && chars[i - 1].is_alphabetic()) {
            // Digit grouping and in-word apostrophes vanish: "1,000" → "1000", "don't" → "dont".
        } else if c == '.' && prev_digit && next_digit {
            cleaned.push('.');
        } else {
            cleaned.push(' ');
        }
    }
    let raw: Vec<&str> = cleaned.split_whitespace().collect();
    merge_number_words(&raw)
        .into_iter()
        .map(|t| trim_zero_decimals(&t))
        .collect()
}

/// True when every token of `value` appears, contiguously and in order, in `heard`.
pub fn contains_literal(heard: &[String], value: &str) -> bool {
    let needle = tokens(value);
    if needle.is_empty() {
        return false;
    }
    heard.windows(needle.len()).any(|w| w == needle.as_slice())
}

fn trim_zero_decimals(token: &str) -> String {
    if let Some((whole, frac)) = token.split_once('.')
        && !whole.is_empty()
        && whole.bytes().all(|b| b.is_ascii_digit())
        && frac.bytes().all(|b| b == b'0')
    {
        return whole.to_owned();
    }
    token.to_owned()
}

fn unit_value(word: &str) -> Option<u64> {
    Some(match word {
        "zero" => 0,
        "one" => 1,
        "two" => 2,
        "three" => 3,
        "four" => 4,
        "five" => 5,
        "six" => 6,
        "seven" => 7,
        "eight" => 8,
        "nine" => 9,
        "ten" => 10,
        "eleven" => 11,
        "twelve" => 12,
        "thirteen" => 13,
        "fourteen" => 14,
        "fifteen" => 15,
        "sixteen" => 16,
        "seventeen" => 17,
        "eighteen" => 18,
        "nineteen" => 19,
        "twenty" => 20,
        "thirty" => 30,
        "forty" => 40,
        "fifty" => 50,
        "sixty" => 60,
        "seventy" => 70,
        "eighty" => 80,
        "ninety" => 90,
        _ => return None,
    })
}

fn scale_value(word: &str) -> Option<u64> {
    match word {
        "hundred" => Some(100),
        "thousand" => Some(1_000),
        "million" => Some(1_000_000),
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Last {
    Tens,
    Small,
    Scale,
}

/// Replaces runs of number words with their digits. A tens word may take one unit
/// ("forty two" → 42); a scale multiplies ("two thousand three hundred" → 2300); "and"
/// joins only after a scale ("one hundred and five" → 105). Anything else starts a new
/// number, so "five five" stays two numbers and "one and two" stays "1 and 2".
fn merge_number_words(raw: &[&str]) -> Vec<String> {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let Some(first) = unit_value(raw[i]) else {
            out.push(raw[i].to_owned());
            i += 1;
            continue;
        };
        let mut total: u64 = 0;
        let mut current: u64 = first;
        let mut last = if first >= 20 { Last::Tens } else { Last::Small };
        let mut j = i + 1;
        while j < raw.len() {
            let word = raw[j];
            if let Some(v) = unit_value(word) {
                let fits = match last {
                    Last::Scale => true,
                    Last::Tens => (1..=9).contains(&v),
                    Last::Small => false,
                };
                if !fits {
                    break;
                }
                current = current.saturating_add(v);
                last = if v >= 20 { Last::Tens } else { Last::Small };
                j += 1;
            } else if let Some(scale) = scale_value(word) {
                if scale == 100 {
                    if last == Last::Scale {
                        break;
                    }
                    current = current.saturating_mul(100);
                } else {
                    total = total.saturating_add(current.max(1).saturating_mul(scale));
                    current = 0;
                }
                last = Last::Scale;
                j += 1;
            } else if word == "and" && last == Last::Scale && raw.get(j + 1).is_some_and(|n| unit_value(n).is_some()) {
                j += 1;
            } else {
                break;
            }
        }
        out.push(total.saturating_add(current).to_string());
        i = j;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Vec<String> {
        tokens(s)
    }

    #[test]
    fn recognizer_and_model_spellings_meet_in_one_form() {
        assert_eq!(
            t("Send $42 to Priya tomorrow."),
            ["send", "42", "to", "priya", "tomorrow"]
        );
        assert_eq!(t("send forty two dollars"), ["send", "42", "dollars"]);
        assert_eq!(t("forty-two"), ["42"]);
        assert_eq!(t("$1,000.00"), ["1000"]);
        assert_eq!(t("one hundred and five"), ["105"]);
        assert_eq!(t("two thousand three hundred"), ["2300"]);
        assert_eq!(t("rock and roll"), ["rock", "and", "roll"]);
        assert_eq!(t("Priya's"), ["priyas"]);
        assert_eq!(t("12.50"), ["12.50"]);
        assert_eq!(t("one and two"), ["1", "and", "2"]);
        assert_eq!(t("five five five"), ["5", "5", "5"]);
        assert_eq!(t("twenty twenty"), ["20", "20"]);
        assert_eq!(t("ninety nine thousand"), ["99000"]);
    }

    #[test]
    fn slot_presence_is_literal_and_contiguous() {
        let heard = t("Send $42 to Priya tomorrow.");
        assert!(contains_literal(&heard, "42"));
        assert!(contains_literal(&heard, "$42.00"));
        assert!(contains_literal(&heard, "forty two"));
        assert!(contains_literal(&heard, "Priya"));
        assert!(
            !contains_literal(&heard, "Pria"),
            "a corrected name is not what was heard"
        );
        assert!(!contains_literal(&heard, "420"));
        assert!(!contains_literal(&heard, "Priya Sharma"), "added words are not heard");
        assert!(!contains_literal(&heard, ""), "an empty value is never present");
        assert!(!contains_literal(&heard, "..."));
    }
}
