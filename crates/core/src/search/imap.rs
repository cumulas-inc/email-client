//! Compiles a query to IMAP `SEARCH` criteria (RFC 3501 §6.4.4), and renders
//! them for the wire. Pure: the provider sends what this returns.
//!
//! IMAP matches substrings, so words and phrases alike are sent as they
//! are; terms of every field must all match (criteria are ANDed). Read and
//! flagged become `SEEN`, `UNSEEN` and `FLAGGED`, and dates `SINCE` and
//! `BEFORE`, widened so the server's matches are never fewer than the
//! device's (which checks every match exactly).

use std::fmt::Write as _;

use chrono::{Datelike, Days, Duration, NaiveDate};

use super::dates::Received;
use super::{Field, SearchQuery};

/// Criteria for one `UID SEARCH`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImapCriteria {
    /// Search keys (`FROM`, `SUBJECT`...), any of which may match, and
    /// their string.
    keys: Vec<(&'static [&'static str], String)>,
    has_attachment: bool,
    read: Option<bool>,
    flagged: bool,
    /// `SINCE`: the first day a match may have arrived on.
    since: Option<NaiveDate>,
    /// `BEFORE`: the day after the last a match may have arrived on.
    before: Option<NaiveDate>,
}

/// Where a plain word is looked for: where the local index looks (sender,
/// To, Cc, subject and text). `TEXT` would look in every header too, and a
/// word in all of them (the user's own domain, the provider's name, as in
/// Received, DKIM or Message-ID lines) would match most of a mailbox.
const ANYWHERE: &[&str] = &["FROM", "TO", "CC", "SUBJECT", "BODY"];

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

/// Months as `SEARCH` dates name them (RFC 3501 `date-month`).
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// A `SEARCH` date: `1-Oct-2026`.
fn search_date(date: NaiveDate) -> String {
    format!(
        "{}-{}-{}",
        date.day(),
        MONTHS[date.month0() as usize],
        date.year()
    )
}

impl ImapCriteria {
    /// `None` for an empty query.
    ///
    /// `SINCE` and `BEFORE` compare the day a message arrived on, in the
    /// zone of its `INTERNALDATE` (RFC 3501 §6.4.4), which may be any from
    /// UTC−12 to UTC+14: `since` is the day it is in UTC−12 then, and
    /// `before` the day after the one it is in UTC+14, so the server's set
    /// is always a superset of the device's.
    pub fn new(query: &SearchQuery, received: Received) -> Option<Self> {
        if query.is_empty() {
            return None;
        }
        let keys = query
            .terms
            .iter()
            .map(|term| {
                let keys: &[&str] = match term.field {
                    Field::Any => ANYWHERE,
                    Field::From => &["FROM"],
                    Field::To => &["TO"],
                    Field::Cc => &["CC"],
                    Field::Subject => &["SUBJECT"],
                    Field::Body => &["BODY"],
                };
                // As typed: servers may ignore the case of ASCII letters
                // only. Literals can't carry NUL; line breaks never reach a
                // term.
                (keys, term.typed.replace(['\0', '\r', '\n'], " "))
            })
            .collect();
        Some(Self {
            keys,
            has_attachment: query.has_attachment,
            read: query.read,
            flagged: query.flagged,
            since: received
                .since
                .map(|since| (since - Duration::hours(12)).date_naive()),
            before: received.before.map(|before| {
                let day = (before + Duration::hours(14)).date_naive();
                day.checked_add_days(Days::new(1)).unwrap_or(day)
            }),
        })
    }

    /// Whether matches must have attachments; the provider checks each
    /// one's structure, as the `HEADER` criterion is approximate.
    pub fn has_attachment(&self) -> bool {
        self.has_attachment
    }

    /// Whether every string goes quoted, with no literal: the command can
    /// be sent whole, with others, without waiting for the server.
    pub fn is_quoted(&self) -> bool {
        self.keys.iter().all(|(_, value)| quotable(value))
    }

    /// The command's segments after `UID SEARCH `. A string that isn't
    /// plain ASCII goes as a literal, and makes the command say `CHARSET
    /// UTF-8`; ASCII-only queries never do, as some servers support no
    /// other charset than US-ASCII. With [`Literals::Sync`], every segment
    /// but the last ends with an announced literal (`{n}`), whose bytes start
    /// the next segment, to be sent after the server's continuation. Keys
    /// any of which may match are `OR`ed.
    pub fn render(&self, literals: Literals) -> Vec<String> {
        self.render_below(literals, None)
    }

    /// [`ImapCriteria::render`] for the messages whose UIDs are below
    /// `below`, when set (more than 1).
    pub fn render_below(&self, literals: Literals, below: Option<u32>) -> Vec<String> {
        let mut segments = Vec::new();
        let mut current = String::new();
        if self.keys.iter().any(|(_, value)| !value.is_ascii()) {
            current.push_str("CHARSET UTF-8 ");
        }
        if let Some(below) = below {
            let _ = write!(current, "UID 1:{} ", below.saturating_sub(1).max(1));
        }
        for (keys, value) in &self.keys {
            for _ in 1..keys.len() {
                current.push_str("OR ");
            }
            for key in *keys {
                current.push_str(key);
                current.push(' ');
                if quotable(value) {
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
        }
        if self.flagged {
            current.push_str("FLAGGED ");
        }
        match self.read {
            Some(true) => current.push_str("SEEN "),
            Some(false) => current.push_str("UNSEEN "),
            None => {}
        }
        if let Some(since) = self.since {
            let _ = write!(current, "SINCE {} ", search_date(since));
        }
        if let Some(before) = self.before {
            let _ = write!(current, "BEFORE {} ", search_date(before));
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

/// Whether a string can go quoted: plain ASCII, without control
/// characters.
fn quotable(value: &str) -> bool {
    value.is_ascii() && !value.chars().any(|c| c.is_ascii_control())
}

/// A quoted string, as `connection::quote` writes mailbox names.
fn quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use chrono::{FixedOffset, TimeZone, Utc};

    use super::*;
    use crate::search::{Term, dates};

    fn criteria(input: &str) -> Option<ImapCriteria> {
        ImapCriteria::new(&SearchQuery::parse(input), Received::default())
    }

    fn rendered(input: &str, literals: Literals) -> Vec<String> {
        criteria(input).unwrap().render(literals)
    }

    /// `input` with its dates resolved in a zone `east_hours` from UTC, on
    /// Sunday 18 October 2026.
    fn dated(input: &str, east_hours: i32) -> Vec<String> {
        let query = SearchQuery::parse(input);
        let tz = FixedOffset::east_opt(east_hours * 3600).unwrap();
        let now = tz
            .with_ymd_and_hms(2026, 10, 18, 10, 0, 0)
            .unwrap()
            .with_timezone(&Utc);
        let received = dates::resolve(&query.dates, now, &tz, 0);
        ImapCriteria::new(&query, received)
            .unwrap()
            .render(Literals::Sync)
    }

    /// A plain word's keys, each with `value` (already rendered).
    fn anywhere(value: &str) -> String {
        format!("OR OR OR OR FROM {value} TO {value} CC {value} SUBJECT {value} BODY {value}")
    }

    #[test]
    fn fields_become_keys() {
        for literals in [Literals::NonSync, Literals::Sync] {
            assert_eq!(
                rendered("from:megan subject:\"q3 plan\"", literals),
                ["FROM \"megan\" SUBJECT \"q3 plan\" UNDELETED"]
            );
        }
        // A plain word, where the local index looks: not TEXT, which looks
        // in every header.
        assert_eq!(
            rendered("invoice to:alex cc:dana body:pineapple", Literals::Sync),
            [format!(
                "{} TO \"alex\" CC \"dana\" BODY \"pineapple\" UNDELETED",
                anywhere("\"invoice\"")
            )]
        );
        assert_eq!(
            rendered("has:attachment", Literals::Sync),
            ["HEADER Content-Type \"multipart/mixed\" UNDELETED"]
        );
        assert_eq!(
            rendered("budget has:attachment", Literals::NonSync),
            [format!(
                "{} HEADER Content-Type \"multipart/mixed\" UNDELETED",
                anywhere("\"budget\"")
            )]
        );
        assert!(criteria("budget has:attachment").unwrap().has_attachment());
    }

    #[test]
    fn read_and_flagged_become_flags() {
        assert_eq!(rendered("is:unread", Literals::Sync), ["UNSEEN UNDELETED"]);
        assert_eq!(
            rendered("is:read is:flagged invoice", Literals::Sync),
            [format!(
                "{} FLAGGED SEEN UNDELETED",
                anywhere("\"invoice\"")
            )]
        );
        assert_eq!(
            rendered("has:attachment is:flagged", Literals::NonSync),
            ["FLAGGED HEADER Content-Type \"multipart/mixed\" UNDELETED"]
        );
        // Where to look is the provider's business.
        assert_eq!(
            rendered("is:flagged in:inbox account:x", Literals::Sync),
            ["FLAGGED UNDELETED"]
        );
        assert!(criteria("is:flagged").unwrap().is_quoted());
    }

    #[test]
    fn dates_hold_for_every_zone_a_server_may_date_mail_in() {
        // The same day, as the computer's zone has it: the server's days
        // reach far enough either way.
        for east in [-4, 1, 9] {
            assert_eq!(
                dated("received:2026-10-01", east),
                ["SINCE 30-Sep-2026 BEFORE 3-Oct-2026 UNDELETED"],
                "UTC{east:+}"
            );
        }
        assert_eq!(
            dated("received:2026-10-03", -4),
            ["SINCE 2-Oct-2026 BEFORE 5-Oct-2026 UNDELETED"]
        );
        assert_eq!(
            dated("after:2026-01-01", -4),
            ["SINCE 31-Dec-2025 UNDELETED"]
        );
        assert_eq!(
            dated("before:2026-10-01 is:unread", -4),
            ["UNSEEN BEFORE 2-Oct-2026 UNDELETED"]
        );
        // In order, after the words and the flags, before the attachment
        // key; the charset and older UIDs first.
        let query = SearchQuery::parse("subject:é has:attachment is:flagged received:2026-10-01");
        let tz = FixedOffset::east_opt(0).unwrap();
        let now = tz.with_ymd_and_hms(2026, 10, 18, 10, 0, 0).unwrap();
        let received = dates::resolve(&query.dates, now.with_timezone(&Utc), &tz, 0);
        assert_eq!(
            ImapCriteria::new(&query, received)
                .unwrap()
                .render_below(Literals::Sync, Some(301)),
            [
                "CHARSET UTF-8 UID 1:300 SUBJECT {2}",
                "é FLAGGED SINCE 30-Sep-2026 BEFORE 3-Oct-2026 \
                 HEADER Content-Type \"multipart/mixed\" UNDELETED"
            ]
        );
    }

    #[test]
    fn strings_keep_the_case_they_were_typed_in() {
        assert_eq!(
            rendered("Invoice FROM:Megan", Literals::Sync),
            [format!(
                "{} FROM \"Megan\" UNDELETED",
                anywhere("\"Invoice\"")
            )]
        );
        assert_eq!(
            rendered("subject:İstanbul", Literals::NonSync),
            ["CHARSET UTF-8 SUBJECT {9+}\r\nİstanbul UNDELETED"]
        );
    }

    #[test]
    fn non_ascii_strings_go_as_literals_with_a_charset() {
        assert_eq!(
            rendered("café", Literals::NonSync),
            [format!(
                "CHARSET UTF-8 {} UNDELETED",
                anywhere("{5+}\r\ncafé")
            )]
        );
        assert_eq!(
            rendered("café", Literals::Sync),
            [
                "CHARSET UTF-8 OR OR OR OR FROM {5}",
                "café TO {5}",
                "café CC {5}",
                "café SUBJECT {5}",
                "café BODY {5}",
                "café UNDELETED"
            ]
        );
        // ASCII strings in the same command stay quoted; counts are bytes.
        assert_eq!(
            rendered("from:josé subject:über plan", Literals::Sync),
            [
                "CHARSET UTF-8 FROM {5}".to_owned(),
                "josé SUBJECT {5}".to_owned(),
                format!("über {} UNDELETED", anywhere("\"plan\""))
            ]
        );
        assert_eq!(
            rendered("subject:日本", Literals::NonSync),
            ["CHARSET UTF-8 SUBJECT {6+}\r\n日本 UNDELETED"]
        );
    }

    #[test]
    fn only_quoted_criteria_go_whole() {
        assert!(
            criteria("from:megan \"q3 plan\" has:attachment")
                .unwrap()
                .is_quoted()
        );
        assert!(!criteria("café").unwrap().is_quoted());
    }

    #[test]
    fn older_matches_are_asked_for_by_uid() {
        let criteria = criteria("subject:é").unwrap();
        assert_eq!(
            criteria.render_below(Literals::Sync, Some(301)),
            ["CHARSET UTF-8 UID 1:300 SUBJECT {2}", "é UNDELETED"]
        );
    }

    #[test]
    fn quotes_backslashes_and_control_characters() {
        let criteria = |text: &str| ImapCriteria {
            keys: vec![(&["SUBJECT"][..], text.to_owned())],
            has_attachment: false,
            read: None,
            flagged: false,
            since: None,
            before: None,
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
            ..SearchQuery::default()
        };
        assert_eq!(
            ImapCriteria::new(&query, Received::default())
                .unwrap()
                .render(Literals::Sync),
            ["BODY \"x y  z\" UNDELETED"]
        );
    }

    #[test]
    fn empty_queries_have_none() {
        assert_eq!(criteria("to: has:pdf"), None);
        assert_eq!(criteria("in:inbox account:x is:starred"), None);
        assert!(criteria("is:read").is_some());
    }
}
