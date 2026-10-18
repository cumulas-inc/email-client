//! Orders local matches best first: where the words were found first, then
//! how well they matched (bm25), then how recent the message is.
//!
//! A term found in the subject or sender counts 1, one found only elsewhere
//! (To, Cc, the text) [`OTHER_FIELD_SHARE`]; a message's field share is the
//! mean over the ranked terms. Its relevance is the field share, of which
//! bm25 decides the last [`BM25_SHARE`]; its score is the relevance, of
//! which recency decides all but [`RECENCY_FLOOR`] (halving every
//! [`RECENCY_HALF_LIFE_DAYS`]).
//!
//! So a match in the subject or sender scores at least `0.75 × 0.4 = 0.3`,
//! and a match only elsewhere at most `0.3`: subject and sender matches come
//! first at any age, and within each group, recent strong matches lead.

pub const MAX_RANKED_CANDIDATES: u32 = 5000;
/// Rows that get a snippet, the first in the final order; later rows show
/// their preview, highlighted by the UI.
pub const MAX_SNIPPETS: usize = 100;
/// What a ranked term found only outside the subject and sender counts for.
pub const OTHER_FIELD_SHARE: f64 = 0.3;
/// How much of relevance bm25 decides, within a field share.
pub const BM25_SHARE: f64 = 0.25;
/// Matched only inside words (the trigram index: subjects and senders).
pub const INFIX_RELEVANCE: f64 = 0.6;
/// A server hit the index doesn't match: it matched in the body beyond the
/// preview (OTHER_FIELD_SHARE × (1 − BM25_SHARE)).
pub const SERVER_RELEVANCE: f64 = 0.225;
pub const RECENCY_HALF_LIFE_DAYS: f64 = 30.0;
pub const RECENCY_FLOOR: f64 = 0.4;

const SECONDS_PER_DAY: f64 = 86_400.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub id: String,
    pub fts_rowid: Option<i64>,
    pub received_at: i64,
    /// bm25 for the rank expression; None when the row doesn't match it.
    pub bm25: Option<f64>,
    /// With `bm25`: the mean over the ranked terms of 1.0 (in the subject or
    /// sender) or OTHER_FIELD_SHARE. Ignored without.
    pub field: f64,
    pub from_server: bool,
}

/// Best first: score desc, then received_at desc, then id asc.
pub fn order(candidates: &mut [Candidate], now: i64) {
    // bm25 is negative; the most negative matched best.
    let best = candidates
        .iter()
        .filter_map(|c| c.bm25)
        .min_by(f64::total_cmp);
    let mut scored: Vec<(f64, Candidate)> = candidates
        .iter()
        .map(|c| (score(c, best, now), c.clone()))
        .collect();
    scored.sort_by(|(a, x), (b, y)| {
        b.total_cmp(a)
            .then(y.received_at.cmp(&x.received_at))
            .then(x.id.cmp(&y.id))
    });
    for (slot, (_, c)) in candidates.iter_mut().zip(scored) {
        *slot = c;
    }
}

/// `best_bm25` is the most negative bm25 among the candidates (None if
/// none has one); `now` and `received_at` are Unix seconds.
pub fn score(c: &Candidate, best_bm25: Option<f64>, now: i64) -> f64 {
    let relevance = match c.bm25 {
        Some(bm25) => {
            let strength = match best_bm25 {
                Some(best) if best < 0.0 => (bm25 / best).clamp(0.0, 1.0),
                _ => 1.0,
            };
            c.field * (1.0 - BM25_SHARE + BM25_SHARE * strength)
        }
        None if c.from_server => SERVER_RELEVANCE,
        None => INFIX_RELEVANCE,
    };
    // Mail dated in the future is as recent as can be.
    let age_days = now.saturating_sub(c.received_at).max(0) as f64 / SECONDS_PER_DAY;
    let recency = 0.5_f64.powf(age_days / RECENCY_HALF_LIFE_DAYS);
    relevance * (RECENCY_FLOOR + (1.0 - RECENCY_FLOOR) * recency)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;
    const DAY: i64 = 86_400;

    fn candidate(id: &str, bm25: Option<f64>, field: f64, days_old: i64) -> Candidate {
        Candidate {
            id: id.into(),
            fts_rowid: Some(1),
            received_at: NOW - days_old * DAY,
            bm25,
            field,
            from_server: false,
        }
    }

    fn ids(candidates: &[Candidate]) -> Vec<&str> {
        candidates.iter().map(|c| c.id.as_str()).collect()
    }

    #[test]
    fn subject_and_sender_first_then_strength_then_recency() {
        let mut e = candidate("E", None, 0.0, 2);
        e.from_server = true;
        let mut candidates = vec![
            candidate("C", Some(-3.0), OTHER_FIELD_SHARE, 0),
            candidate("B", Some(-10.0), 1.0, 365),
            e,
            candidate("D", None, 0.0, 0),
            candidate("F", Some(-10.0), 0.65, 0),
            candidate("A", Some(-10.0), 1.0, 1),
        ];
        let best = Some(-10.0);
        let scores: Vec<(&str, f64)> = candidates
            .iter()
            .map(|c| (c.id.as_str(), score(c, best, NOW)))
            .collect();
        for (id, expected) in [
            ("A", 0.986),
            ("F", 0.650),
            ("D", 0.600),
            ("B", 0.400),
            ("C", 0.2475),
            ("E", 0.219),
        ] {
            let (_, got) = scores.iter().find(|(i, _)| *i == id).unwrap();
            assert!((got - expected).abs() < 0.0006, "{id}: {got}");
        }
        order(&mut candidates, NOW);
        assert_eq!(ids(&candidates), ["A", "F", "D", "B", "C", "E"]);
    }

    #[test]
    fn mail_from_the_future_is_new() {
        let future = candidate("x", Some(-1.0), 1.0, -3);
        assert!((score(&future, Some(-1.0), NOW) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn ties_go_by_date_then_id() {
        // Future-dated mail all scores the same: the newest leads.
        let mut candidates = vec![
            candidate("b", Some(-2.0), 1.0, -1),
            candidate("a", Some(-2.0), 1.0, -1),
            candidate("c", Some(-2.0), 1.0, -2),
        ];
        order(&mut candidates, NOW);
        assert_eq!(ids(&candidates), ["c", "a", "b"]);
    }

    #[test]
    fn equal_scores_or_no_bm25_order_by_recency() {
        // Equal relevance: recency alone decides.
        let mut equal = vec![
            candidate("old", Some(-5.0), 1.0, 40),
            candidate("new", Some(-5.0), 1.0, 1),
            candidate("mid", Some(-5.0), 1.0, 10),
        ];
        order(&mut equal, NOW);
        assert_eq!(ids(&equal), ["new", "mid", "old"]);
        // No bm25 at all (infix-only matches).
        let mut infix = vec![
            candidate("old", None, 0.0, 400),
            candidate("new", None, 0.0, 0),
            candidate("mid", None, 0.0, 30),
        ];
        order(&mut infix, NOW);
        assert_eq!(ids(&infix), ["new", "mid", "old"]);
    }

    #[test]
    fn an_old_subject_match_stays_above_a_new_body_match() {
        let mut candidates = vec![
            candidate("body", Some(-30.0), OTHER_FIELD_SHARE, 0),
            candidate("subject", Some(-0.5), 1.0, 3 * 365),
        ];
        order(&mut candidates, NOW);
        assert_eq!(ids(&candidates), ["subject", "body"]);
        let [subject, body] = &candidates[..] else {
            unreachable!()
        };
        assert!(score(subject, Some(-30.0), NOW) >= score(body, Some(-30.0), NOW));
    }

    #[test]
    fn a_best_bm25_of_zero_counts_every_match_as_strong() {
        let c = candidate("x", Some(0.0), 1.0, 0);
        assert!((score(&c, Some(0.0), NOW) - 1.0).abs() < 1e-9);
        assert!((score(&c, None, NOW) - 1.0).abs() < 1e-9);
    }
}
