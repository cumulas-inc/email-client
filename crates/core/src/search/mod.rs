//! Mail search queries: what the user types in the search box, parsed once
//! and compiled for each place that searches (the local full-text index,
//! Microsoft Graph's `$search` or `$filter`, IMAP `SEARCH`).
//!
//! The syntax is a small, forgiving subset of Outlook's and Gmail's: plain
//! words (all must match, as word prefixes), `"exact phrases"`, the
//! operators `from:` `to:` `cc:` `subject:` `body:` and `has:attachment`,
//! and filters: `is:unread` `is:read` `is:flagged`, `in:<folder>`,
//! `account:<account>`, and dates, `received:` (a day such as `2026-10-01`,
//! a range such as `2026-10-01..2026-10-15`, or a word such as `today` or
//! `"this week"`), `after:` and `before:` (see [`dates`]). Anything else is
//! plain words, and nothing is ever an error: `-word`, `AND` or `re:` are
//! just words, and a filter with a value it doesn't know (`is:starred`,
//! `received:tomorrow`) is left out. The UI's port in
//! `src/lib/search/query.ts` passes the same test vectors.

pub mod dates;
pub mod fts;
pub mod imap;
pub mod kql;
pub mod rank;
pub mod scope;

use std::collections::HashSet;

use icu_normalizer::ComposingNormalizerBorrowed;
use icu_properties::CodePointMapData;
use icu_properties::props::{GeneralCategory, GeneralCategoryGroup};

use crate::model::MessageSummary;

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
    /// `is:read` (Some(true)) or `is:unread` (Some(false)); the last typed wins.
    pub read: Option<bool>,
    /// `is:flagged`.
    pub flagged: bool,
    /// `received:`, `after:`, `before:`: all must hold.
    pub dates: Vec<dates::DateClause>,
    /// `in:` values as scope keys: a message in any of the folders they name matches.
    pub folders: Vec<String>,
    /// `account:` values as scope keys: any.
    pub accounts: Vec<String>,
}

/// Terms kept; later ones are dropped.
pub const MAX_TERMS: usize = 16;
/// Dates, folders and accounts kept, each; later ones are dropped.
pub const MAX_FILTER_VALUES: usize = 8;
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
    "is",
    "in",
    "account",
    "after",
    "before",
    "received",
];

/// Characters that may be part of a word besides letters and digits, as in
/// `megan@contoso.com`, `2023-118` or `o'brien`; never at its ends.
fn is_word_joiner(c: char) -> bool {
    matches!(c, '@' | '.' | '_' | '-' | '\'' | '+')
}

/// A combining mark (`\p{M}`): a virama or vowel sign (as in "हिन्दी"), or
/// an accent typed on its own. Part of the word it follows.
pub(crate) fn is_mark(c: char) -> bool {
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

    /// Nothing to look for. Where to look (`in:`, `account:`) alone only
    /// narrows a search, so it counts as nothing.
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
            && !self.has_attachment
            && self.read.is_none()
            && !self.flagged
            && self.dates.is_empty()
    }

    pub fn has_terms(&self) -> bool {
        !self.terms.is_empty()
    }

    /// Whether the servers are worth asking: some term has
    /// [`MIN_SERVER_TERM_CHARS`] letters or digits, or it asks for
    /// attachments, unread or flagged mail, or dates. Single letters (`a
    /// b`, `from:a`) match most of a mailbox, and every server would look
    /// through every folder for them; so does `is:read`, and `in:` or
    /// `account:` alone.
    pub fn is_narrow_enough_for_servers(&self) -> bool {
        self.has_attachment
            || self.read == Some(false)
            || self.flagged
            || !self.dates.is_empty()
            || self.terms.iter().any(|t| {
                t.text.chars().filter(|c| c.is_alphanumeric()).count() >= MIN_SERVER_TERM_CHARS
            })
    }

    /// Whether `m` passes every filter but the words: attachments, read,
    /// flagged, `received` and, when `Some`, the folders (local ids).
    pub fn admits(
        &self,
        m: &MessageSummary,
        received: &dates::Received,
        folders: Option<&HashSet<String>>,
    ) -> bool {
        (!self.has_attachment || m.has_attachments)
            && self.read.is_none_or(|read| m.is_read == read)
            && (!self.flagged || m.is_flagged)
            && received.contains(m.received_at)
            && folders.is_none_or(|folders| folders.contains(&m.folder_id))
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
                self.add_filter(name, &value, quoted);
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

    /// A filter's value (`has:`, `is:`, `in:`, `account:` and dates); one
    /// it doesn't know is left out.
    fn add_filter(&mut self, name: &str, value: &str, quoted: bool) {
        let text = if quoted {
            phrase_text(value).to_lowercase()
        } else {
            value.to_lowercase()
        };
        match name {
            "has" => self.has_attachment |= matches!(text.as_str(), "attachment" | "attachments"),
            "hasattachment" | "hasattachments" => {
                self.has_attachment |= matches!(text.as_str(), "true" | "yes");
            }
            "is" => match text.as_str() {
                "unread" => self.read = Some(false),
                "read" => self.read = Some(true),
                "flagged" => self.flagged = true,
                _ => {}
            },
            // Kept whole, as a name: `in:Q1\Q2`, `in:"Project X"`.
            "in" | "account" => {
                let key = scope::scope_key(value);
                let values = match name {
                    "in" => &mut self.folders,
                    _ => &mut self.accounts,
                };
                if key.chars().any(char::is_alphanumeric) && values.len() < MAX_FILTER_VALUES {
                    values.push(key);
                }
            }
            _ => {
                let op = match name {
                    "after" => dates::DateOp::After,
                    "before" => dates::DateOp::Before,
                    _ => dates::DateOp::Received,
                };
                if let Some(clause) = dates::parse_clause(op, &text)
                    && self.dates.len() < MAX_FILTER_VALUES
                {
                    self.dates.push(clause);
                }
            }
        }
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
    use chrono::{Duration, TimeZone, Utc};

    use super::dates::{DateKeyword, DateOp, DateSpec, Received};
    use super::*;
    use crate::model::{EmailAddress, Importance};

    /// Terms and filters as the shared test vectors write them: terms
    /// `A:invoice`, `Subject:q3 plan [p]`; filters in a fixed order
    /// (attachments, read, flagged, dates as typed, folders, accounts), as
    /// `is:unread` or `received:2026-10-01..`.
    fn parsed(input: &str) -> (Vec<String>, Vec<String>) {
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
        let mut filters = Vec::new();
        if query.has_attachment {
            filters.push("has:attachment".to_owned());
        }
        match query.read {
            Some(true) => filters.push("is:read".to_owned()),
            Some(false) => filters.push("is:unread".to_owned()),
            None => {}
        }
        if query.flagged {
            filters.push("is:flagged".to_owned());
        }
        for clause in &query.dates {
            let op = match clause.op {
                DateOp::After => "after",
                DateOp::Before => "before",
                DateOp::Received => "received",
            };
            let day = |d: Option<chrono::NaiveDate>| {
                d.map(|d| d.format("%Y-%m-%d").to_string())
                    .unwrap_or_default()
            };
            let spec = match clause.spec {
                DateSpec::Day(d) => day(Some(d)),
                DateSpec::Range(first, last) => format!("{}..{}", day(first), day(last)),
                DateSpec::Keyword(keyword) => match keyword {
                    DateKeyword::Today => "today",
                    DateKeyword::Yesterday => "yesterday",
                    DateKeyword::ThisWeek => "this week",
                    DateKeyword::LastWeek => "last week",
                    DateKeyword::ThisMonth => "this month",
                    DateKeyword::LastMonth => "last month",
                    DateKeyword::ThisYear => "this year",
                    DateKeyword::LastYear => "last year",
                }
                .to_owned(),
            };
            filters.push(format!("{op}:{spec}"));
        }
        filters.extend(query.folders.iter().map(|f| format!("in:{f}")));
        filters.extend(query.accounts.iter().map(|a| format!("account:{a}")));
        (terms, filters)
    }

    /// Terms alone, and whether it asks for attachments.
    fn terms(input: &str) -> (Vec<String>, bool) {
        let query = SearchQuery::parse(input);
        (parsed(input).0, query.has_attachment)
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    /// The vectors `src/lib/search/query.test.ts` checks as well.
    #[test]
    fn parses_the_shared_test_vectors() {
        let vectors: &[(&str, &[&str], &[&str])] = &[
            ("invoice", &["A:invoice"], &[]),
            ("Invoice March", &["A:invoice", "A:march"], &[]),
            ("\"quarterly report\"", &["A:quarterly report [p]"], &[]),
            ("from:megan", &["From:megan"], &[]),
            ("From:Megan lunch", &["From:megan", "A:lunch"], &[]),
            (
                "subject:\"Quarterly Report\" from:megan@contoso.com",
                &["Subject:quarterly report [p]", "From:megan@contoso.com"],
                &[],
            ),
            ("to:", &[], &[]),
            ("from: megan", &["A:megan"], &[]),
            ("has:attachment budget", &["A:budget"], &["has:attachment"]),
            ("HasAttachments:yes", &[], &["has:attachment"]),
            ("has:pdf", &[], &[]),
            ("re:hello", &["A:re", "A:hello"], &[]),
            ("\"unterminated phrase", &["A:unterminated phrase [p]"], &[]),
            ("\u{201c}curly quotes\u{201d}", &["A:curly quotes [p]"], &[]),
            ("*** ??", &[], &[]),
            ("-draft +final", &["A:draft", "A:final"], &[]),
            ("o\u{2019}brien.", &["A:o'brien"], &[]),
            ("AND OR NOT", &["A:and", "A:or", "A:not"], &[]),
            ("subject:\"", &[], &[]),
            ("lucía", &["A:lucía"], &[]),
            ("from:\"Megan Bowen\"", &["From:megan bowen [p]"], &[]),
            ("cc:x@y.z", &["Cc:x@y.z"], &[]),
            ("2023-118", &["A:2023-118"], &[]),
            // Combining marks stay in their word; typed accents compose.
            ("हिन्दी ಕನ್ನಡ", &["A:हिन्दी", "A:ಕನ್ನಡ"], &[]),
            ("re\u{301}sume\u{301}", &["A:résumé"], &[]),
            ("\u{301}abc -\u{301}x\u{301}", &["A:abc", "A:x\u{301}"], &[]),
            ("\"a\u{7}b\tc\u{0}\"", &["A:ab c [p]"], &[]),
            (
                "a b c d e f g h i j k l m n o p q r",
                &[
                    "A:a", "A:b", "A:c", "A:d", "A:e", "A:f", "A:g", "A:h", "A:i", "A:j", "A:k",
                    "A:l", "A:m", "A:n", "A:o", "A:p",
                ],
                &[],
            ),
            // Filters.
            ("is:unread", &[], &["is:unread"]),
            ("IS:Flagged invoice", &["A:invoice"], &["is:flagged"]),
            ("is:read is:unread", &[], &["is:unread"]),
            ("is:starred", &[], &[]),
            ("is:\"unread\"", &[], &["is:unread"]),
            ("in:inbox budget", &["A:budget"], &["in:inbox"]),
            (
                "in:\"Project  X\" in:Archive",
                &[],
                &["in:project x", "in:archive"],
            ),
            ("in:\" Clients\"", &[], &["in:clients"]),
            ("in:Q1\\Q2", &[], &["in:q1q2"]),
            ("in:\"Say Hi\"", &[], &["in:say hi"]),
            ("in:*** x", &["A:x"], &[]),
            (
                "account:Megan@Contoso.com",
                &[],
                &["account:megan@contoso.com"],
            ),
            ("account:\"Work Mail\"", &[], &["account:work mail"]),
            (
                "after:2026-10-01 before:2026/10/16",
                &[],
                &["after:2026-10-01", "before:2026-10-16"],
            ),
            (
                "received:2026-10-01..2026-10-15",
                &[],
                &["received:2026-10-01..2026-10-15"],
            ),
            ("received:2026-10-01..", &[], &["received:2026-10-01.."]),
            ("received:..2026-1-5", &[], &["received:..2026-01-05"]),
            (
                "received:\"This  Week\" lunch",
                &["A:lunch"],
                &["received:this week"],
            ),
            ("received:\"last month\"", &[], &["received:last month"]),
            (
                "received:today received:yesterday",
                &[],
                &["received:today", "received:yesterday"],
            ),
            ("received:tomorrow", &[], &[]),
            ("after:2026-02-30", &[], &[]),
            ("after:2026-10-01..2026-10-02", &[], &[]),
            ("received:2026-10-01..yesterday", &[], &[]),
            (
                "received:2026-10-15..2026-10-01",
                &[],
                &["received:2026-10-15..2026-10-01"],
            ),
            ("before:yesterday", &[], &["before:yesterday"]),
            ("in:", &[], &[]),
            ("in: inbox", &["A:inbox"], &[]),
            ("received:", &[], &[]),
            ("isunread is", &["A:isunread", "A:is"], &[]),
            (
                "has:attachment is:flagged in:sent received:yesterday account:work is:read",
                &[],
                &[
                    "has:attachment",
                    "is:read",
                    "is:flagged",
                    "received:yesterday",
                    "in:sent",
                    "account:work",
                ],
            ),
            (
                "from:megan is:unread \"q3 plan\"",
                &["From:megan", "A:q3 plan [p]"],
                &["is:unread"],
            ),
            ("ACCOUNT:x in:Inbox", &[], &["in:inbox", "account:x"]),
        ];
        for (input, terms, filters) in vectors {
            assert_eq!(
                parsed(input),
                (strings(terms), strings(filters)),
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

    /// The shared narrowness vectors: filters that leave few messages are
    /// worth the servers' time; read mail and where to look aren't.
    #[test]
    fn filters_that_narrow_enough_for_the_servers() {
        for input in [
            "is:unread",
            "is:flagged",
            "received:today",
            "after:2020-01-01",
            "before:2010-01-01",
            "has:attachment in:inbox",
        ] {
            assert!(
                SearchQuery::parse(input).is_narrow_enough_for_servers(),
                "{input:?}"
            );
        }
        for input in [
            "is:read",
            "in:inbox",
            "account:work",
            "is:read in:inbox account:work",
            "a is:read",
        ] {
            assert!(
                !SearchQuery::parse(input).is_narrow_enough_for_servers(),
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
        // The shared emptiness vectors: where to look alone, and filters
        // left out, are nothing to look for.
        for input in [
            "in:inbox",
            "account:x",
            "in:inbox account:x",
            "is:starred",
            "received:tomorrow",
        ] {
            assert!(SearchQuery::parse(input).is_empty(), "{input:?}");
        }
        for input in ["is:read", "received:today", "is:flagged"] {
            assert!(!SearchQuery::parse(input).is_empty(), "{input:?}");
        }
        assert!(SearchQuery::parse("x").has_terms());
        assert!(!SearchQuery::parse("is:unread in:inbox").has_terms());
    }

    #[test]
    fn filter_values_are_capped() {
        let many = |op: &str, value: &dyn Fn(usize) -> String| {
            (0..9)
                .map(|i| format!("{op}:{}", value(i)))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let query = SearchQuery::parse(&many("in", &|i| format!("f{i}")));
        assert_eq!(query.folders.len(), MAX_FILTER_VALUES);
        assert_eq!(query.folders[7], "f7");
        let query = SearchQuery::parse(&many("account", &|i| format!("a{i}")));
        assert_eq!(query.accounts.len(), MAX_FILTER_VALUES);
        let query = SearchQuery::parse(&many("after", &|i| format!("2026-10-{:02}", i + 1)));
        assert_eq!(query.dates.len(), MAX_FILTER_VALUES);
        // Filters don't count as terms, nor terms as filters.
        let words = (0..20).map(|i| format!("w{i}")).collect::<Vec<_>>();
        let query = SearchQuery::parse(&format!("{} in:inbox", words.join(" ")));
        assert_eq!(query.terms.len(), MAX_TERMS);
        assert_eq!(query.folders, ["inbox"]);
    }

    fn summary() -> MessageSummary {
        MessageSummary {
            id: "m".into(),
            account_id: "a".into(),
            folder_id: "inbox".into(),
            remote_id: "1".into(),
            conversation_id: None,
            subject: "Lunch".into(),
            from: Some(EmailAddress::new(Some("Ada"), "ada@example.com")),
            to: vec![],
            cc: vec![],
            preview: String::new(),
            received_at: Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap(),
            is_read: false,
            is_flagged: false,
            is_draft: false,
            has_attachments: false,
            importance: Importance::Normal,
        }
    }

    #[test]
    fn admits_checks_every_filter_but_the_words() {
        let m = summary();
        let open = Received::default();
        let admits =
            |text: &str, m: &MessageSummary| SearchQuery::parse(text).admits(m, &open, None);
        // Words are the index's business.
        assert!(admits("nothing like it", &m));
        assert!(admits("is:unread", &m));
        assert!(!admits("is:read", &m));
        assert!(!admits("is:flagged", &m));
        assert!(!admits("has:attachment", &m));
        let flagged = MessageSummary {
            is_read: true,
            is_flagged: true,
            has_attachments: true,
            ..summary()
        };
        assert!(admits("is:read is:flagged has:attachment", &flagged));
        assert!(!admits("is:unread", &flagged));

        // Received: since inclusive, before exclusive.
        let query = SearchQuery::parse("x");
        let at = m.received_at;
        let hour = Duration::hours(1);
        let span = |since, before| Received { since, before };
        assert!(query.admits(&m, &span(Some(at), Some(at + hour)), None));
        assert!(!query.admits(&m, &span(Some(at + hour), None), None));
        assert!(!query.admits(&m, &span(None, Some(at)), None));

        // Folders, by local id, when given.
        let inbox: HashSet<String> = ["inbox".to_owned()].into();
        let archive: HashSet<String> = ["archive".to_owned()].into();
        assert!(query.admits(&m, &open, Some(&inbox)));
        assert!(!query.admits(&m, &open, Some(&archive)));
        assert!(!query.admits(&m, &open, Some(&HashSet::new())));
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
