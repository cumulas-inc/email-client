//! Compiles a query for the local full-text indexes (see migration 5 in
//! `store/schema.rs`): `messages_fts` (words: subject, sender, To, Cc and
//! body) and `messages_infix` (trigrams of subject and sender, for matches
//! inside words, such as "voice" in "Invoice").

use super::{Field, SearchQuery, Term};

/// What a search adds to the `WHERE` clause on the messages row `m`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FtsPlan {
    /// SQL conditions on the messages row aliased `m`, joined with AND.
    pub conditions: Vec<String>,
    /// Parameters for `conditions`, in order (MATCH expressions only).
    pub params: Vec<String>,
}

/// One term matched by words, or inside words.
const EITHER: &str = "m.fts_rowid IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ? \
     UNION SELECT rowid FROM messages_infix WHERE messages_infix MATCH ?)";
/// Terms matched by words only.
const WORDS: &str = "m.fts_rowid IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?)";

/// Trigrams need three characters.
const MIN_INFIX_CHARS: usize = 3;

/// The plan for `query`; no conditions for an empty one.
pub fn plan(query: &SearchQuery) -> FtsPlan {
    let mut plan = FtsPlan::default();
    let mut words = Vec::new();
    for term in &query.terms {
        match infix_column(term) {
            Some(column) => {
                plan.conditions.push(EITHER.to_owned());
                plan.params.push(expression(term));
                plan.params.push(format!("{column}{}", quoted(&term.text)));
            }
            None => words.push(expression(term)),
        }
    }
    if !words.is_empty() {
        plan.conditions.push(WORDS.to_owned());
        plan.params.push(words.join(" AND "));
    }
    if query.has_attachment {
        plan.conditions.push("m.has_attachments = 1".to_owned());
    }
    plan
}

/// The term as an FTS5 expression on `messages_fts`: a quoted phrase, so
/// nothing in it is FTS syntax, whose last word is a prefix unless the term
/// was quoted. The tokenizer splits `megan@contoso.com` into a phrase of
/// three words; the prefix applies to the last.
fn expression(term: &Term) -> String {
    let column = match term.field {
        Field::Any => "",
        Field::From => "sender : ",
        Field::To => "to_recipients : ",
        Field::Cc => "cc_recipients : ",
        Field::Subject => "subject : ",
        Field::Body => "body : ",
    };
    let prefix = if term.phrase { "" } else { "*" };
    format!("{column}{}{prefix}", quoted(&term.text))
}

/// The column filter for matching the term inside words, when it can be:
/// in subjects and senders, for three characters or more.
fn infix_column(term: &Term) -> Option<&'static str> {
    if term.text.chars().count() < MIN_INFIX_CHARS {
        return None;
    }
    match term.field {
        Field::Any => Some(""),
        Field::From => Some("sender : "),
        Field::Subject => Some("subject : "),
        Field::To | Field::Cc | Field::Body => None,
    }
}

fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(input: &str) -> Vec<String> {
        plan(&SearchQuery::parse(input)).params
    }

    #[test]
    fn terms_too_short_or_not_in_subjects_or_senders_match_words_only() {
        let to = plan(&SearchQuery::parse("to:al"));
        assert_eq!(to.conditions, [WORDS]);
        assert_eq!(to.params, ["to_recipients : \"al\"*"]);
        assert_eq!(
            params("vo cc:dana body:pineapple to:alex"),
            [
                "\"vo\"* AND cc_recipients : \"dana\"* AND body : \"pineapple\"* \
              AND to_recipients : \"alex\"*"
            ]
        );
        assert_eq!(plan(&SearchQuery::parse("vo")).conditions, [WORDS]);
    }

    #[test]
    fn longer_terms_also_match_inside_words() {
        let invoice = plan(&SearchQuery::parse("invoice"));
        assert_eq!(invoice.conditions, [EITHER]);
        assert_eq!(invoice.params, ["\"invoice\"*", "\"invoice\""]);

        let mixed = plan(&SearchQuery::parse("from:megan subject:\"q3 plan\" to:al"));
        assert_eq!(mixed.conditions, [EITHER, EITHER, WORDS]);
        assert_eq!(
            mixed.params,
            [
                "sender : \"megan\"*",
                "sender : \"megan\"",
                "subject : \"q3 plan\"",
                "subject : \"q3 plan\"",
                "to_recipients : \"al\"*",
            ]
        );
        // The condition placeholders and the parameters line up.
        let placeholders: usize = mixed
            .conditions
            .iter()
            .map(|c| c.matches('?').count())
            .sum();
        assert_eq!(placeholders, mixed.params.len());
    }

    #[test]
    fn attachments_and_empty_queries() {
        let attached = plan(&SearchQuery::parse("has:attachment budget"));
        assert_eq!(attached.conditions, [EITHER, "m.has_attachments = 1"]);
        assert_eq!(plan(&SearchQuery::parse("has:pdf")), FtsPlan::default());
        assert_eq!(plan(&SearchQuery::default()), FtsPlan::default());
    }

    #[test]
    fn quotes_are_doubled() {
        // The parser never leaves one in a term; the plan doesn't rely on it.
        let query = SearchQuery {
            terms: vec![Term {
                field: Field::Subject,
                text: "say \"hi\"".into(),
                typed: "say \"hi\"".into(),
                phrase: true,
            }],
            has_attachment: false,
        };
        assert_eq!(
            plan(&query).params,
            [
                "subject : \"say \"\"hi\"\"\"",
                "subject : \"say \"\"hi\"\"\""
            ]
        );
    }

    #[test]
    fn addresses_stay_one_phrase() {
        assert_eq!(
            params("from:megan@contoso"),
            ["sender : \"megan@contoso\"*", "sender : \"megan@contoso\""]
        );
    }
}
