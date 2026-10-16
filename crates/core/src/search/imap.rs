//! Compiles a query to IMAP `SEARCH` criteria (RFC 3501 §6.4.4), and renders
//! them for the wire. Pure: the provider sends what this returns.
//!
//! IMAP matches substrings, so words and phrases alike are sent as they
//! are; terms of every field must all match (criteria are ANDed).

use std::fmt::Write as _;

use super::{Field, SearchQuery};

/// Criteria for one `UID SEARCH`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImapCriteria {
    /// Search keys (`TEXT`, `FROM`, ...) and their strings.
    keys: Vec<(&'static str, String)>,
    has_attachment: bool,
}

/// How the server takes literals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Literals {
    /// Inline, without waiting for the server (`LITERAL+`, or `LITERAL-`
    /// up to 4,096 bytes; strings here are at most 512).
    NonSync,
    /// Announced, then sent once the server says to go on (`+`).
    Sync,
}

/// A multipart/mixed message usually carries attachments; the provider
/// checks each match's structure, which is exact.
const ATTACHMENT_KEY: &str = "HEADER Content-Type \"multipart/mixed\"";

impl ImapCriteria {
    /// `None` for an empty query.
    pub fn new(query: &SearchQuery) -> Option<Self> {
        if query.is_empty() {
            return None;
        }
        let keys = query
            .terms
            .iter()
            .map(|term| {
                let key = match term.field {
                    Field::Any => "TEXT",
                    Field::From => "FROM",
                    Field::To => "TO",
                    Field::Cc => "CC",
                    Field::Subject => "SUBJECT",
                    Field::Body => "BODY",
                };
                // As typed: servers may ignore the case of ASCII letters
                // only. Literals can't carry NUL; line breaks never reach a
                // term.
                (key, term.typed.replace(['\0', '\r', '\n'], " "))
            })
            .collect();
        Some(Self {
            keys,
            has_attachment: query.has_attachment,
        })
    }

    /// Whether matches must have attachments; the provider checks each
    /// one's structure, as the `HEADER` criterion is approximate.
    pub fn has_attachment(&self) -> bool {
        self.has_attachment
    }

    /// The command's segments after `UID SEARCH `. A string that isn't
    /// plain ASCII goes as a literal, and makes the command say `CHARSET
    /// UTF-8`; ASCII-only queries never do, as some servers support no
    /// other charset than US-ASCII. With [`Literals::Sync`], every segment
    /// but the last ends with an announced literal (`{n}`), whose bytes start
    /// the next segment, to be sent after the server's continuation.
    pub fn render(&self, literals: Literals) -> Vec<String> {
        let mut segments = Vec::new();
        let mut current = String::new();
        if self.keys.iter().any(|(_, value)| !value.is_ascii()) {
            current.push_str("CHARSET UTF-8 ");
        }
        for (key, value) in &self.keys {
            current.push_str(key);
            current.push(' ');
            if value.is_ascii() && !value.chars().any(|c| c.is_ascii_control()) {
                current.push_str(&quoted(value));
            } else {
                // Byte counts, not chars.
                let n = value.len();
                match literals {
                    Literals::NonSync => {
                        let _ = write!(current, "{{{n}+}}\r\n{value}");
                    }
                    Literals::Sync => {
                        let _ = write!(current, "{{{n}}}");
                        segments.push(std::mem::take(&mut current));
                        current.push_str(value);
                    }
                }
            }
            current.push(' ');
        }
        if self.has_attachment {
            current.push_str(ATTACHMENT_KEY);
            current.push(' ');
        }
        // Messages marked for deletion are as good as gone.
        current.push_str("UNDELETED");
        segments.push(current);
        segments
    }
}

/// A quoted string, as `connection::quote` writes mailbox names.
fn quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::Term;

    fn rendered(input: &str, literals: Literals) -> Vec<String> {
        ImapCriteria::new(&SearchQuery::parse(input))
            .unwrap()
            .render(literals)
    }

    #[test]
    fn fields_become_keys() {
        for literals in [Literals::NonSync, Literals::Sync] {
            assert_eq!(
                rendered("from:megan subject:\"q3 plan\"", literals),
                ["FROM \"megan\" SUBJECT \"q3 plan\" UNDELETED"]
            );
        }
        assert_eq!(
            rendered("invoice to:alex cc:dana body:pineapple", Literals::Sync),
            ["TEXT \"invoice\" TO \"alex\" CC \"dana\" BODY \"pineapple\" UNDELETED"]
        );
        assert_eq!(
            rendered("has:attachment", Literals::Sync),
            ["HEADER Content-Type \"multipart/mixed\" UNDELETED"]
        );
        assert_eq!(
            rendered("budget has:attachment", Literals::NonSync),
            ["TEXT \"budget\" HEADER Content-Type \"multipart/mixed\" UNDELETED"]
        );
        assert!(
            ImapCriteria::new(&SearchQuery::parse("budget has:attachment"))
                .unwrap()
                .has_attachment()
        );
    }

    #[test]
    fn strings_keep_the_case_they_were_typed_in() {
        assert_eq!(
            rendered("Invoice FROM:Megan", Literals::Sync),
            ["TEXT \"Invoice\" FROM \"Megan\" UNDELETED"]
        );
        assert_eq!(
            rendered("İstanbul", Literals::NonSync),
            ["CHARSET UTF-8 TEXT {9+}\r\nİstanbul UNDELETED"]
        );
    }

    #[test]
    fn non_ascii_strings_go_as_literals_with_a_charset() {
        assert_eq!(
            rendered("café", Literals::NonSync),
            ["CHARSET UTF-8 TEXT {5+}\r\ncafé UNDELETED"]
        );
        assert_eq!(
            rendered("café", Literals::Sync),
            ["CHARSET UTF-8 TEXT {5}", "café UNDELETED"]
        );
        // ASCII strings in the same command stay quoted; counts are bytes.
        assert_eq!(
            rendered("from:josé subject:über plan", Literals::Sync),
            [
                "CHARSET UTF-8 FROM {5}",
                "josé SUBJECT {5}",
                "über TEXT \"plan\" UNDELETED"
            ]
        );
        assert_eq!(
            rendered("日本", Literals::NonSync),
            ["CHARSET UTF-8 TEXT {6+}\r\n日本 UNDELETED"]
        );
    }

    #[test]
    fn quotes_backslashes_and_control_characters() {
        let criteria = |text: &str| ImapCriteria {
            keys: vec![("SUBJECT", text.to_owned())],
            has_attachment: false,
        };
        assert_eq!(
            criteria(r#"say "hi" \o/"#).render(Literals::Sync),
            [r#"SUBJECT "say \"hi\" \\o/" UNDELETED"#]
        );
        // Not quotable: a literal, but plain ASCII needs no charset.
        assert_eq!(
            criteria("a\u{7}b").render(Literals::NonSync),
            ["SUBJECT {3+}\r\na\u{7}b UNDELETED"]
        );
        let query = SearchQuery {
            terms: vec![Term {
                field: Field::Body,
                text: "x\0y\r\nz".into(),
                typed: "x\0y\r\nz".into(),
                phrase: true,
            }],
            has_attachment: false,
        };
        assert_eq!(
            ImapCriteria::new(&query).unwrap().render(Literals::Sync),
            ["BODY \"x y  z\" UNDELETED"]
        );
    }

    #[test]
    fn empty_queries_have_none() {
        assert_eq!(ImapCriteria::new(&SearchQuery::parse("to: has:pdf")), None);
    }
}
