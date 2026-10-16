//! Mail search queries: what the user types in the search box, parsed once
//! and compiled for each place that searches (the local full-text index,
//! Microsoft Graph's `$search`, IMAP `SEARCH`).
//!
//! The syntax is a small, forgiving subset of Outlook's: plain words (all
//! must match, as word prefixes), `"exact phrases"`, the operators `from:`
//! `to:` `cc:` `subject:` `body:` and `has:attachment`. Anything else is
//! plain words, and nothing is ever an error: `-word`, `AND` or `re:` are
//! just words. The demo backend in `src/api/mock/searchQuery.ts` ports this
//! parser; both pass the same test vectors.

pub mod fts;
pub mod imap;
pub mod kql;

use icu_normalizer::ComposingNormalizerBorrowed;
use icu_properties::CodePointMapData;
use icu_properties::props::{GeneralCategory, GeneralCategoryGroup};

/// The part of a message a term must match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Any,
    From,
    To,
    Cc,
    Subject,
    Body,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Term {
    pub field: Field,
    /// Lowercased; never contains `"` or `\`; at least one alphanumeric char.
    pub text: String,
    /// `text` in the case it was typed in, for IMAP: a server may ignore
    /// the case of ASCII letters only, and lowercasing respells some
    /// letters ("İ" becomes "i" and a combining dot).
    pub typed: String,
    /// Exact phrase (quoted): no prefix matching on the last word.
    pub phrase: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchQuery {
    /// All must match.
    pub terms: Vec<Term>,
    pub has_attachment: bool,
}

/// Terms kept; later ones are dropped.
pub const MAX_TERMS: usize = 16;
/// Characters kept of each term.
pub const MAX_TERM_CHARS: usize = 128;
/// Characters of the input read.
pub const MAX_INPUT_CHARS: usize = 1024;
/// Letters and digits one term at least needs for the servers to be asked.
pub const MIN_SERVER_TERM_CHARS: usize = 2;

/// Operator names, lowercased, before their `:`.
const OPERATORS: &[&str] = &[
    "from",
    "to",
    "cc",
    "subject",
    "body",
    "has",
    "hasattachment",
    "hasattachments",
];

/// Characters that may be part of a word besides letters and digits, as in
/// `megan@contoso.com`, `2023-118` or `o'brien`; never at its ends.
fn is_word_joiner(c: char) -> bool {
    matches!(c, '@' | '.' | '_' | '-' | '\'' | '+')
}

/// A combining mark (`\p{M}`): a virama or vowel sign (as in "हिन्दी"), or
/// an accent typed on its own. Part of the word it follows.
fn is_mark(c: char) -> bool {
    GeneralCategoryGroup::Mark.contains(CodePointMapData::<GeneralCategory>::new().get(c))
}

impl SearchQuery {
    pub fn parse(input: &str) -> Self {
        // Composed (NFC), as mail is: "é" typed as "e" and an accent is the
        // same word.
        let head: String = input.chars().take(MAX_INPUT_CHARS).collect();
        let chars: Vec<char> = ComposingNormalizerBorrowed::new_nfc()
            .normalize(&head)
            .chars()
            .take(MAX_INPUT_CHARS)
            .map(|c| match c {
                '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{201f}' | '\u{2033}' => '"',
                '\u{2019}' | '\u{2018}' => '\'',
                c => c,
            })
            .collect();
        let mut query = SearchQuery::default();
        let mut at = 0;
        while at < chars.len() {
            let c = chars[at];
            if c.is_whitespace() {
                at += 1;
            } else if c == '"' {
                let (text, next) = phrase(&chars, at + 1);
                query.add_phrase(Field::Any, &text);
                at = next;
            } else if let Some((name, value_at)) = operator(&chars, at) {
                at = query.add_operator(&name, &chars, value_at);
            } else {
                let end = bare_end(&chars, at);
                query.add_words(Field::Any, &chars[at..end]);
                at = end;
            }
        }
        query.terms.truncate(MAX_TERMS);
        query
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty() && !self.has_attachment
    }

    /// Whether the servers are worth asking: some term has
    /// [`MIN_SERVER_TERM_CHARS`] letters or digits, or it asks for
    /// attachments. Single letters (`a b`, `from:a`) match most of a
    /// mailbox, and every server would look through every folder for them.
    pub fn is_narrow_enough_for_servers(&self) -> bool {
        self.has_attachment
            || self.terms.iter().any(|t| {
                t.text.chars().filter(|c| c.is_alphanumeric()).count() >= MIN_SERVER_TERM_CHARS
            })
    }

    /// Reads the value of operator `name` starting at `at`; returns where
    /// parsing goes on.
    fn add_operator(&mut self, name: &str, chars: &[char], at: usize) -> usize {
        let quoted = chars.get(at) == Some(&'"');
        let (value, next) = if quoted {
            phrase(chars, at + 1)
        } else {
            let end = bare_end(chars, at);
            (chars[at..end].iter().collect(), end)
        };
        if !quoted && value.is_empty() {
            // `from:` alone: the operator is dropped and what follows
            // parsed as it is.
            return at;
        }
        let field = match name {
            "from" => Field::From,
            "to" => Field::To,
            "cc" => Field::Cc,
            "subject" => Field::Subject,
            "body" => Field::Body,
            _ => {
                let value = if quoted {
                    phrase_text(&value).to_lowercase()
                } else {
                    value.to_lowercase()
                };
                let yes: &[&str] = match name {
                    "has" => &["attachment", "attachments"],
                    _ => &["true", "yes"],
                };
                self.has_attachment |= yes.contains(&value.as_str());
                return next;
            }
        };
        if quoted {
            self.add_phrase(field, &value);
        } else {
            let value: Vec<char> = value.chars().collect();
            self.add_words(field, &value);
        }
        next
    }

    fn add_phrase(&mut self, field: Field, text: &str) {
        self.push(field, &phrase_text(text), true);
    }

    /// Splits a bare run into words, each a term.
    fn add_words(&mut self, field: Field, run: &[char]) {
        let run: String = run.iter().collect();
        let in_word = |c: char| c.is_alphanumeric() || is_word_joiner(c) || is_mark(c);
        for piece in run.split(|c: char| !in_word(c)) {
            let word = piece
                .trim_start_matches(|c: char| is_word_joiner(c) || is_mark(c))
                .trim_end_matches(is_word_joiner);
            self.push(field, word, false);
        }
    }

    fn push(&mut self, field: Field, typed: &str, phrase: bool) {
        let text = cut(typed.to_lowercase());
        if text.chars().any(char::is_alphanumeric) {
            self.terms.push(Term {
                field,
                text,
                typed: cut(typed.to_owned()),
                phrase,
            });
        }
    }
}

/// At most [`MAX_TERM_CHARS`] of `text`.
fn cut(mut text: String) -> String {
    if let Some((at, _)) = text.char_indices().nth(MAX_TERM_CHARS) {
        text.truncate(at);
    }
    text
}

/// The phrase from `at` to the next `"` (or the end), and where parsing
/// goes on after it.
fn phrase(chars: &[char], at: usize) -> (String, usize) {
    let end = chars[at.min(chars.len())..]
        .iter()
        .position(|&c| c == '"')
        .map_or(chars.len(), |n| at + n);
    (
        chars[at.min(end)..end].iter().collect(),
        (end + 1).min(chars.len()),
    )
}

/// A phrase's text: without backslashes or control characters, whitespace
/// runs as one space, trimmed.
fn phrase_text(raw: &str) -> String {
    raw.chars()
        .filter(|&c| c != '\\' && (c.is_whitespace() || !c.is_control()))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Where a bare run starting at `at` ends: at whitespace or a `"`.
fn bare_end(chars: &[char], at: usize) -> usize {
    chars[at..]
        .iter()
        .position(|&c| c.is_whitespace() || c == '"')
        .map_or(chars.len(), |n| at + n)
}

/// A known operator (`letters:`) at `at`: its lowercased name and where its
/// value starts.
fn operator(chars: &[char], at: usize) -> Option<(String, usize)> {
    let letters = chars[at..]
        .iter()
        .take_while(|c| c.is_ascii_alphabetic())
        .count();
    if letters == 0 || chars.get(at + letters) != Some(&':') {
        return None;
    }
    let name: String = chars[at..at + letters]
        .iter()
        .collect::<String>()
        .to_ascii_lowercase();
    OPERATORS
        .contains(&name.as_str())
        .then_some((name, at + letters + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Terms as the shared test vectors write them: `A:invoice`,
    /// `Subject:q3 plan [p]`.
    fn terms(input: &str) -> (Vec<String>, bool) {
        let query = SearchQuery::parse(input);
        let terms = query
            .terms
            .iter()
            .map(|t| {
                let field = match t.field {
                    Field::Any => "A",
                    Field::From => "From",
                    Field::To => "To",
                    Field::Cc => "Cc",
                    Field::Subject => "Subject",
                    Field::Body => "Body",
                };
                let phrase = if t.phrase { " [p]" } else { "" };
                format!("{field}:{}{phrase}", t.text)
            })
            .collect();
        (terms, query.has_attachment)
    }

    /// The vectors `src/api/mock/searchQuery.test.ts` checks as well.
    #[test]
    fn parses_the_shared_test_vectors() {
        let vectors: &[(&str, &[&str], bool)] = &[
            ("invoice", &["A:invoice"], false),
            ("Invoice March", &["A:invoice", "A:march"], false),
            ("\"quarterly report\"", &["A:quarterly report [p]"], false),
            ("from:megan", &["From:megan"], false),
            ("From:Megan lunch", &["From:megan", "A:lunch"], false),
            (
                "subject:\"Quarterly Report\" from:megan@contoso.com",
                &["Subject:quarterly report [p]", "From:megan@contoso.com"],
                false,
            ),
            ("to:", &[], false),
            ("from: megan", &["A:megan"], false),
            ("has:attachment budget", &["A:budget"], true),
            ("HasAttachments:yes", &[], true),
            ("has:pdf", &[], false),
            ("re:hello", &["A:re", "A:hello"], false),
            (
                "\"unterminated phrase",
                &["A:unterminated phrase [p]"],
                false,
            ),
            (
                "\u{201c}curly quotes\u{201d}",
                &["A:curly quotes [p]"],
                false,
            ),
            ("*** ??", &[], false),
            ("-draft +final", &["A:draft", "A:final"], false),
            ("o\u{2019}brien.", &["A:o'brien"], false),
            ("AND OR NOT", &["A:and", "A:or", "A:not"], false),
            ("subject:\"", &[], false),
            ("lucía", &["A:lucía"], false),
            ("from:\"Megan Bowen\"", &["From:megan bowen [p]"], false),
            ("cc:x@y.z", &["Cc:x@y.z"], false),
            ("2023-118", &["A:2023-118"], false),
            // Combining marks stay in their word; typed accents compose.
            ("हिन्दी ಕನ್ನಡ", &["A:हिन्दी", "A:ಕನ್ನಡ"], false),
            ("re\u{301}sume\u{301}", &["A:résumé"], false),
            (
                "\u{301}abc -\u{301}x\u{301}",
                &["A:abc", "A:x\u{301}"],
                false,
            ),
            ("\"a\u{7}b\tc\u{0}\"", &["A:ab c [p]"], false),
            (
                "a b c d e f g h i j k l m n o p q r",
                &[
                    "A:a", "A:b", "A:c", "A:d", "A:e", "A:f", "A:g", "A:h", "A:i", "A:j", "A:k",
                    "A:l", "A:m", "A:n", "A:o", "A:p",
                ],
                false,
            ),
        ];
        for (input, expected, has_attachment) in vectors {
            assert_eq!(
                terms(input),
                (
                    expected.iter().map(|t| t.to_string()).collect(),
                    *has_attachment
                ),
                "{input:?}"
            );
        }
    }

    #[test]
    fn terms_keep_the_case_they_were_typed_in() {
        let query = SearchQuery::parse("ÜBER from:İstanbul subject:\"Q3  Plan\"");
        let typed: Vec<(&str, &str)> = query
            .terms
            .iter()
            .map(|t| (t.text.as_str(), t.typed.as_str()))
            .collect();
        assert_eq!(
            typed,
            [
                ("über", "ÜBER"),
                ("i\u{307}stanbul", "İstanbul"),
                ("q3 plan", "Q3 Plan")
            ]
        );
        // Cut to as many chars as the text, each on its own.
        let long = SearchQuery::parse(&"İ".repeat(200));
        assert_eq!(long.terms[0].typed.chars().count(), MAX_TERM_CHARS);
        assert_eq!(long.terms[0].text.chars().count(), MAX_TERM_CHARS);
    }

    #[test]
    fn single_letters_are_left_to_the_device() {
        for input in ["a", "a b c", "from:a", "\"a\"", "c++ x", "日"] {
            assert!(
                !SearchQuery::parse(input).is_narrow_enough_for_servers(),
                "{input:?}"
            );
        }
        for input in ["ab", "a ab", "from:al", "\"a b\"", "日本", "has:attachment"] {
            assert!(
                SearchQuery::parse(input).is_narrow_enough_for_servers(),
                "{input:?}"
            );
        }
    }

    #[test]
    fn empty_queries() {
        for input in ["", "   ", "to:", "has:pdf", "*** ??", "subject:\"", "\"\""] {
            assert!(SearchQuery::parse(input).is_empty(), "{input:?}");
        }
        assert!(!SearchQuery::parse("has:attachment").is_empty());
    }

    #[test]
    fn operators_and_their_values() {
        // Case-insensitive names; the value stops before a quote, which
        // starts a phrase of its own.
        assert_eq!(
            terms("SUBJECT:lunch\"next week\""),
            (
                vec!["Subject:lunch".into(), "A:next week [p]".into()],
                false
            )
        );
        // A bare value of several words gives each the field.
        assert_eq!(
            terms("from:megan.bowen,alex"),
            (vec!["From:megan.bowen".into(), "From:alex".into()], false)
        );
        // Unknown operators and lookalikes are words.
        assert_eq!(
            terms("x-from:megan bcc:dana"),
            (
                vec![
                    "A:x-from".into(),
                    "A:megan".into(),
                    "A:bcc".into(),
                    "A:dana".into()
                ],
                false
            )
        );
        assert_eq!(terms("hasattachment:true"), (vec![], true));
        assert_eq!(terms("hasattachments:no"), (vec![], false));
        assert_eq!(terms("has:Attachments"), (vec![], true));
        // Phrases lose backslashes and extra whitespace.
        assert_eq!(
            terms("body:\"  a\\b \t c \""),
            (vec!["Body:ab c [p]".into()], false)
        );
    }

    #[test]
    fn limits() {
        // The input is read up to 1,024 chars: what follows is lost.
        let query = SearchQuery::parse(&format!("{}y tail", " ".repeat(MAX_INPUT_CHARS - 2)));
        assert_eq!(query.terms.len(), 1);
        assert_eq!(query.terms[0].text, "y");
        // Terms are cut to 128 chars at a char boundary.
        let query = SearchQuery::parse(&"é".repeat(200));
        assert_eq!(query.terms[0].text, "é".repeat(MAX_TERM_CHARS));
        let query = SearchQuery::parse(&format!("subject:\"{}\"", "ü".repeat(200)));
        assert_eq!(query.terms[0].text, "ü".repeat(MAX_TERM_CHARS));
        // At most 16 terms, phrases and operators included.
        let many = (0..20).map(|i| format!("from:w{i}")).collect::<Vec<_>>();
        assert_eq!(SearchQuery::parse(&many.join(" ")).terms.len(), MAX_TERMS);
    }

    #[test]
    fn never_panics_on_awkward_input() {
        let awkward = [
            "\"",
            "\"\"\"",
            "\\",
            "\\\"",
            "from:\"",
            "from:\"\"",
            "has:",
            "has:\"attachment\"",
            ":",
            "::from:",
            "\u{0}\u{1}\u{7f}",
            "a\r\nb\tc",
            "😀 🎉👍🏽",
            "שלום עולם",
            "مرحبا from:عالم",
            "\u{202e}gnp.exe",
            "a\u{301}b \u{0902}",
            "“",
            "„from:x‟",
            "subject:″x″",
        ];
        for input in awkward {
            let query = SearchQuery::parse(input);
            for term in &query.terms {
                assert!(!term.text.contains(['"', '\\']), "{input:?}: {term:?}");
                assert!(
                    term.text.chars().any(char::is_alphanumeric),
                    "{input:?}: {term:?}"
                );
                assert_eq!(term.text, term.text.to_lowercase(), "{input:?}");
            }
        }
        assert_eq!(terms("has:\" Attach\\ment \""), (vec![], true));
        assert_eq!(terms("has:attachment,"), (vec![], false));
        assert_eq!(terms("„from:x‟"), (vec!["A:from:x [p]".into()], false));
    }
}
