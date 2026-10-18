//! Compiles a query for Microsoft Graph: KQL for `$search` on messages
//! (Exchange's Keyword Query Language) when it has words, and a `$filter`
//! when it has only filters.

use chrono::{DateTime, Days, NaiveDate, SecondsFormat, Utc};

use super::dates::Received;
use super::{Field, SearchQuery};

/// Bare words match as prefixes (`invoice*`), as they do locally. Should
/// a mailbox treat the wildcard badly, turning this off sends bare words.
const PREFIX_WILDCARD: bool = true;

/// Received dates go to the server, widened a day each side (the device
/// filters exactly). A hard off switch: false sends no dates.
const KQL_DATES: bool = true;

/// KQL for Microsoft Graph `$search`; None without terms. `Received::default()`
/// sends no dates (the provider does that after a mailbox refused them).
///
/// Terms are joined with `AND`; a word gets a trailing `*` (prefix match),
/// a phrase is quoted, and fields become KQL properties. A word with `..`
/// in it is quoted too: bare, it is KQL's range operator (`size:1..500`),
/// which would fail the search or find something else. Words are
/// lowercased by the parser, so KQL never sees `AND`, `OR`, `NOT` or `NEAR`
/// as operators, and a leading `-` was trimmed, so nothing excludes. Without
/// a property, Graph searches from, subject and body.
///
/// Then `hasAttachments:true`, and the dates as Exchange documents them
/// (`received>=MM/DD/YYYY`, `received<=MM/DD/YYYY`), a day wider on each
/// side: KQL compares dates in UTC, and the day may be another in the
/// computer's zone. KQL can't say read, flagged or folders; the device
/// checks those, and the exact dates, of what the server finds.
pub fn kql(query: &SearchQuery, received: Received) -> Option<String> {
    if !query.has_terms() {
        return None;
    }
    let mut parts: Vec<String> = query
        .terms
        .iter()
        .map(|term| {
            let property = match term.field {
                Field::Any => "",
                Field::From => "from:",
                Field::To => "to:",
                Field::Cc => "cc:",
                Field::Subject => "subject:",
                Field::Body => "body:",
            };
            let value = if term.phrase || term.text.contains("..") {
                format!("\"{}\"", term.text.replace('"', ""))
            } else if PREFIX_WILDCARD && term.text.ends_with(char::is_alphanumeric) {
                format!("{}*", term.text)
            } else {
                term.text.clone()
            };
            format!("{property}{value}")
        })
        .collect();
    if query.has_attachment {
        parts.push("hasAttachments:true".to_owned());
    }
    if KQL_DATES {
        let day = |t: DateTime<Utc>, wider: fn(NaiveDate) -> Option<NaiveDate>| {
            let date = t.date_naive();
            wider(date).unwrap_or(date).format("%m/%d/%Y").to_string()
        };
        if let Some(since) = received.since {
            parts.push(format!(
                "received>={}",
                day(since, |d| d.checked_sub_days(Days::new(1)))
            ));
        }
        if let Some(before) = received.before {
            parts.push(format!(
                "received<={}",
                day(before, |d| d.checked_add_days(Days::new(1)))
            ));
        }
    }
    Some(parts.join(" AND "))
}

/// `$filter` for a query without words (the filters Graph can say), with
/// receivedDateTime first, as `$orderby=receivedDateTime desc` needs.
/// Exact: the received bounds as they are, read, flagged and attachments.
/// Graph can't filter by several folders; the device checks those.
pub fn graph_filter(query: &SearchQuery, received: Received) -> String {
    let instant = |t: DateTime<Utc>| t.to_rfc3339_opts(SecondsFormat::Secs, true);
    let mut parts = vec![format!(
        "receivedDateTime ge {}",
        received
            .since
            .map_or_else(|| "1900-01-01T00:00:00Z".to_owned(), instant)
    )];
    if let Some(before) = received.before {
        parts.push(format!("receivedDateTime lt {}", instant(before)));
    }
    if let Some(read) = query.read {
        parts.push(format!("isRead eq {read}"));
    }
    if query.flagged {
        parts.push("flag/flagStatus eq 'flagged'".to_owned());
    }
    if query.has_attachment {
        parts.push("hasAttachments eq true".to_owned());
    }
    parts.join(" and ")
}

/// The `$search` parameter's value: the KQL in double quotes, with `\` and
/// `"` escaped by a backslash (Graph's `$search` docs).
pub fn search_param(kql: &str) -> String {
    format!("\"{}\"", kql.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::Term;

    use chrono::{FixedOffset, TimeZone};

    use crate::search::dates;

    fn compiled(input: &str) -> Option<String> {
        kql(&SearchQuery::parse(input), Received::default())
    }

    /// `input` with its dates resolved at UTC−4 on Sunday 18 October 2026.
    fn with_dates(input: &str) -> (SearchQuery, Received) {
        let query = SearchQuery::parse(input);
        let tz = FixedOffset::west_opt(4 * 3600).unwrap();
        let now = tz
            .with_ymd_and_hms(2026, 10, 18, 10, 0, 0)
            .unwrap()
            .with_timezone(&Utc);
        let received = dates::resolve(&query.dates, now, &tz, 0);
        (query, received)
    }

    #[test]
    fn words_phrases_and_properties() {
        let kql = compiled("from:megan subject:\"q3 plan\" invoice has:attachment").unwrap();
        assert_eq!(
            kql,
            "from:megan* AND subject:\"q3 plan\" AND invoice* AND hasAttachments:true"
        );
        assert_eq!(
            search_param(&kql),
            r#""from:megan* AND subject:\"q3 plan\" AND invoice* AND hasAttachments:true""#
        );
        assert_eq!(
            compiled("megan@contoso.com").as_deref(),
            Some("megan@contoso.com*")
        );
        assert_eq!(compiled("-draft").as_deref(), Some("draft*"));
        assert_eq!(
            compiled("to:alex cc:dana body:pineapple").as_deref(),
            Some("to:alex* AND cc:dana* AND body:pineapple*")
        );
        assert_eq!(
            compiled("AND OR NOT near").as_deref(),
            Some("and* AND or* AND not* AND near*")
        );
        assert_eq!(compiled("2023-118").as_deref(), Some("2023-118*"));
        // Filters alone go to `$filter`.
        assert_eq!(compiled("has:attachment"), None);
    }

    #[test]
    fn dates_a_day_wider_each_side() {
        let kql_of = |input: &str| {
            let (query, received) = with_dates(input);
            kql(&query, received)
        };
        // 1 October at UTC−4 is 04:00 UTC on the 1st to 04:00 on the 2nd.
        assert_eq!(
            kql_of("invoice received:2026-10-01").as_deref(),
            Some("invoice* AND received>=09/30/2026 AND received<=10/03/2026")
        );
        assert_eq!(
            kql_of("invoice after:2026-10-01").as_deref(),
            Some("invoice* AND received>=09/30/2026")
        );
        assert_eq!(
            kql_of("invoice before:2026-10-01").as_deref(),
            Some("invoice* AND received<=10/02/2026")
        );
        assert_eq!(
            kql_of("invoice has:attachment received:2026-12-31..").as_deref(),
            Some("invoice* AND hasAttachments:true AND received>=12/30/2026")
        );
        // Read, flagged and folders are left to the device.
        assert_eq!(
            kql_of("invoice is:unread is:flagged in:inbox account:x").as_deref(),
            Some("invoice*")
        );
        // A mailbox that refused dates gets none.
        let (query, _) = with_dates("invoice received:2026-10-01");
        assert_eq!(
            kql(&query, Received::default()).as_deref(),
            Some("invoice*")
        );
    }

    #[test]
    fn filters_without_words() {
        let filter = |input: &str| {
            let (query, received) = with_dates(input);
            graph_filter(&query, received)
        };
        assert_eq!(
            filter("is:flagged"),
            "receivedDateTime ge 1900-01-01T00:00:00Z and flag/flagStatus eq 'flagged'"
        );
        assert_eq!(
            filter("received:2026-10-01"),
            "receivedDateTime ge 2026-10-01T04:00:00Z and receivedDateTime lt 2026-10-02T04:00:00Z"
        );
        assert_eq!(
            filter("is:read"),
            "receivedDateTime ge 1900-01-01T00:00:00Z and isRead eq true"
        );
        assert_eq!(
            filter("is:unread is:flagged has:attachment"),
            "receivedDateTime ge 1900-01-01T00:00:00Z and isRead eq false \
             and flag/flagStatus eq 'flagged' and hasAttachments eq true"
        );
        assert_eq!(
            filter("before:2026-10-01 has:attachment in:inbox"),
            "receivedDateTime ge 1900-01-01T00:00:00Z and receivedDateTime lt 2026-10-01T04:00:00Z \
             and hasAttachments eq true"
        );
    }

    #[test]
    fn ranges_are_quoted_as_text() {
        assert_eq!(
            compiled("subject:1..5 v1..v2 foo...bar").as_deref(),
            Some("subject:\"1..5\" AND \"v1..v2\" AND \"foo...bar\"")
        );
        assert_eq!(compiled("v1.2").as_deref(), Some("v1.2*"));
    }

    #[test]
    fn the_wildcard_follows_only_letters_and_digits() {
        // The parser never ends a word otherwise; a term built by hand might.
        let query = SearchQuery {
            terms: vec![Term {
                field: crate::search::Field::Any,
                text: "c++".into(),
                typed: "c++".into(),
                phrase: false,
            }],
            ..SearchQuery::default()
        };
        assert_eq!(kql(&query, Received::default()).as_deref(), Some("c++"));
    }

    #[test]
    fn empty_queries_have_none() {
        assert_eq!(compiled(""), None);
        assert_eq!(compiled("has:pdf to:"), None);
    }

    #[test]
    fn the_search_parameter_escapes_quotes_and_backslashes() {
        assert_eq!(search_param("a"), "\"a\"");
        assert_eq!(search_param(r#"x "y" \z"#), r#""x \"y\" \\z""#);
        assert_eq!(
            compiled("subject:\"q3 plan\"")
                .map(|k| search_param(&k))
                .as_deref(),
            Some(r#""subject:\"q3 plan\"""#)
        );
    }
}
