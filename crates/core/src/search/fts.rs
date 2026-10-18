//! Compiles a query for the local full-text indexes (see migration 5 in
//! `store/schema.rs`): `messages_fts` (words: subject, sender, To, Cc and
//! body) and `messages_infix` (trigrams of subject and sender, for matches
//! inside words, such as "voice" in "Invoice"); and what ranking the matches
//! needs of `messages_fts` (see `rank.rs` and `Store::search`).

use super::{Field, SearchQuery, Term};

/// What a search adds to the `WHERE` clause on the messages row `m`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FtsPlan {
    /// SQL conditions on the messages row aliased `m`, joined with AND.
    pub conditions: Vec<String>,
    /// Parameters for `conditions`, in order (MATCH expressions only).
    pub params: Vec<String>,
    /// Conditions on `m` for the filters (attachments, read, flagged), with
    /// no parameters. They apply to server hits too.
    pub filters: Vec<String>,
}

/// Column weights for bm25: subject, sender, To, Cc, body.
pub const BM25_WEIGHTS: [f64; 5] = [10.0, 8.0, 3.0, 2.0, 1.0];

/// How a ranked term counts toward a row's field share (see rank.rs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldShare {
    /// `subject:`: in the subject by definition (1.0).
    Subject,
    /// `body:`: never in the subject or sender (OTHER_FIELD_SHARE).
    Elsewhere,
    /// A plain term: 1.0 for the rows this expression matches, else
    /// OTHER_FIELD_SHARE. It is `{subject sender} : (<the term's expression>)`.
    Probe(String),
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
        plan.filters.push("m.has_attachments = 1".to_owned());
    }
    match query.read {
        Some(true) => plan.filters.push("m.is_read = 1".to_owned()),
        Some(false) => plan.filters.push("m.is_read = 0".to_owned()),
        None => {}
    }
    if query.flagged {
        plan.filters.push("m.is_flagged = 1".to_owned());
    }
    plan
}

/// Whether the query has words to rank by: a plain, `subject:` or `body:`
/// term. `from:`, `to:` and `cc:` only pick messages by address.
pub fn ranks(query: &SearchQuery) -> bool {
    query
        .terms
        .iter()
        .any(|t| matches!(t.field, Field::Any | Field::Subject | Field::Body))
}

/// Every term's word expression (`expression`), ANDed: what `WORDS` would
/// match if no term went to the trigram index. For bm25() and snippet().
/// None unless `ranks(query)`.
pub fn rank_expression(query: &SearchQuery) -> Option<String> {
    ranks(query).then(|| {
        query
            .terms
            .iter()
            .map(expression)
            .collect::<Vec<_>>()
            .join(" AND ")
    })
}

/// One per ranked term (plain, `subject:`, `body:`), in order; empty unless `ranks`.
pub fn field_shares(query: &SearchQuery) -> Vec<FieldShare> {
    query
        .terms
        .iter()
        .filter_map(|term| match term.field {
            Field::Any => Some(FieldShare::Probe(format!(
                "{{subject sender}} : ({})",
                expression(term)
            ))),
            Field::Subject => Some(FieldShare::Subject),
            Field::Body => Some(FieldShare::Elsewhere),
            Field::From | Field::To | Field::Cc => None,
        })
        .collect()
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
        assert_eq!(attached.conditions, [EITHER]);
        assert_eq!(attached.filters, ["m.has_attachments = 1"]);
        let alone = plan(&SearchQuery::parse("has:attachment"));
        assert!(alone.conditions.is_empty() && alone.params.is_empty());
        assert_eq!(alone.filters, ["m.has_attachments = 1"]);
        assert_eq!(plan(&SearchQuery::parse("has:pdf")), FtsPlan::default());
        assert_eq!(plan(&SearchQuery::default()), FtsPlan::default());
        // Where to look and dates are for the store to say.
        assert_eq!(
            plan(&SearchQuery::parse("in:inbox account:x received:today")),
            FtsPlan::default()
        );
    }

    #[test]
    fn read_and_flagged_filters() {
        let filters = |text: &str| plan(&SearchQuery::parse(text)).filters;
        assert_eq!(filters("is:unread"), ["m.is_read = 0"]);
        assert_eq!(filters("is:read"), ["m.is_read = 1"]);
        assert_eq!(filters("is:flagged"), ["m.is_flagged = 1"]);
        assert_eq!(
            filters("is:flagged is:unread has:attachment x"),
            ["m.has_attachments = 1", "m.is_read = 0", "m.is_flagged = 1"]
        );
    }

    /// The shared ranking vectors: words rank, addresses alone don't.
    #[test]
    fn which_queries_rank() {
        for text in [
            "invoice",
            "subject:q3",
            "body:pineapple",
            "from:megan invoice",
            "\"q3 plan\" is:unread",
        ] {
            assert!(ranks(&SearchQuery::parse(text)), "{text:?}");
        }
        for text in [
            "from:megan",
            "to:al cc:dana",
            "from:megan@contoso.com is:unread",
            "is:flagged",
            "in:inbox",
            "has:attachment received:today",
        ] {
            assert!(!ranks(&SearchQuery::parse(text)), "{text:?}");
        }
    }

    #[test]
    fn the_rank_expression_joins_every_term() {
        let expression = |text: &str| rank_expression(&SearchQuery::parse(text));
        // Terms that went to the trigram index too.
        assert_eq!(
            expression("from:megan invoice vo").as_deref(),
            Some("sender : \"megan\"* AND \"invoice\"* AND \"vo\"*")
        );
        assert_eq!(
            expression("subject:\"q3 plan\" body:x").as_deref(),
            Some("subject : \"q3 plan\" AND body : \"x\"*")
        );
        for text in [
            "is:flagged has:attachment",
            "from:megan",
            "to:al cc:dana",
            "",
        ] {
            assert_eq!(expression(text), None, "{text:?}");
        }
    }

    #[test]
    fn field_shares_per_ranked_term() {
        assert_eq!(
            field_shares(&SearchQuery::parse("invoice subject:q3 body:x from:megan")),
            [
                FieldShare::Probe("{subject sender} : (\"invoice\"*)".into()),
                FieldShare::Subject,
                FieldShare::Elsewhere,
            ]
        );
        assert_eq!(
            field_shares(&SearchQuery::parse("\"q3 plan\"")),
            [FieldShare::Probe("{subject sender} : (\"q3 plan\")".into())]
        );
        assert!(field_shares(&SearchQuery::parse("from:megan to:al")).is_empty());
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
            ..SearchQuery::default()
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
