//! Compiles a query to KQL for Microsoft Graph's `$search` on messages
//! (Exchange's Keyword Query Language).

use super::{Field, SearchQuery};

/// Bare words match as prefixes (`invoice*`), as they do locally. Should
/// a mailbox treat the wildcard badly, turning this off sends bare words.
const PREFIX_WILDCARD: bool = true;

/// KQL for Microsoft Graph `$search`, or `None` for an empty query.
///
/// Terms are joined with `AND`; a word gets a trailing `*` (prefix match),
/// a phrase is quoted, and fields become KQL properties. Words are
/// lowercased by the parser, so KQL never sees `AND`, `OR`, `NOT` or `NEAR`
/// as operators, and a leading `-` was trimmed, so nothing excludes. Without
/// a property, Graph searches from, subject and body.
pub fn kql(query: &SearchQuery) -> Option<String> {
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
            let value = if term.phrase {
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
    (!parts.is_empty()).then(|| parts.join(" AND "))
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

    fn compiled(input: &str) -> Option<String> {
        kql(&SearchQuery::parse(input))
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
        assert_eq!(
            compiled("has:attachment").as_deref(),
            Some("hasAttachments:true")
        );
    }

    #[test]
    fn the_wildcard_follows_only_letters_and_digits() {
        // The parser never ends a word otherwise; a term built by hand might.
        let query = SearchQuery {
            terms: vec![Term {
                field: crate::search::Field::Any,
                text: "c++".into(),
                phrase: false,
            }],
            has_attachment: false,
        };
        assert_eq!(kql(&query).as_deref(), Some("c++"));
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
