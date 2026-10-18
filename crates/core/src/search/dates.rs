//! Dates in queries: `received:`, `after:` and `before:`, and the span of
//! time they leave, in the computer's time zone.
//!
//! A value is a day (`2026-10-01` or `2026/10/1`), a range of days for
//! `received:` (`2026-10-01..2026-10-15`, `2026-10-01..`, `..2026-10-15`),
//! or one of KQL's words (`today`, `yesterday`, `"this week"`, `"last
//! week"`, `"this month"`, `"last month"`, `"this year"`, `"last year"`).
//! `received:` means its span, `after:` from its start on (inclusive, as
//! Gmail's), `before:` up to its start (exclusive). Anything else is no
//! date, and is ignored.

use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, Offset, TimeZone, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateOp {
    After,
    Before,
    Received,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateKeyword {
    Today,
    Yesterday,
    ThisWeek,
    LastWeek,
    ThisMonth,
    LastMonth,
    ThisYear,
    LastYear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateSpec {
    Day(NaiveDate),
    /// The days from the first through the last; `None` is open.
    Range(Option<NaiveDate>, Option<NaiveDate>),
    Keyword(DateKeyword),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateClause {
    pub op: DateOp,
    pub spec: DateSpec,
}

/// Received-time bounds (UTC): since inclusive, before exclusive; None is open.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Received {
    pub since: Option<DateTime<Utc>>,
    pub before: Option<DateTime<Utc>>,
}

impl Received {
    /// No bounds: any time.
    pub fn is_open(&self) -> bool {
        self.since.is_none() && self.before.is_none()
    }

    /// No time at all (a reversed range, or clauses that don't overlap).
    pub fn is_impossible(&self) -> bool {
        matches!((self.since, self.before), (Some(since), Some(before)) if since >= before)
    }

    pub fn contains(&self, t: DateTime<Utc>) -> bool {
        self.since.is_none_or(|since| t >= since) && self.before.is_none_or(|before| t < before)
    }

    /// The bounds as servers can be told them: within 1970 through 9998,
    /// so that their dates, widened by a day or so, keep four-digit years,
    /// as IMAP's `SEARCH` dates must (RFC 3501 `date-year`). A bound beyond
    /// is dropped, or moved to the edge, only ever taking in more: the
    /// device checks what the servers find against the exact bounds.
    pub fn for_servers(&self) -> Received {
        let earliest = DateTime::UNIX_EPOCH;
        let latest = NaiveDate::from_ymd_opt(9998, 12, 31)
            .expect("a valid date")
            .and_time(NaiveTime::MIN)
            .and_utc();
        Received {
            since: self.since.filter(|&t| t >= earliest).map(|t| t.min(latest)),
            before: self
                .before
                .filter(|&t| t <= latest)
                .map(|t| t.max(earliest)),
        }
    }
}

/// The keywords, as typed (lowercased, one space between words).
const KEYWORDS: &[(&str, DateKeyword)] = &[
    ("today", DateKeyword::Today),
    ("yesterday", DateKeyword::Yesterday),
    ("this week", DateKeyword::ThisWeek),
    ("last week", DateKeyword::LastWeek),
    ("this month", DateKeyword::ThisMonth),
    ("last month", DateKeyword::LastMonth),
    ("this year", DateKeyword::ThisYear),
    ("last year", DateKeyword::LastYear),
];

/// `None` for a value that isn't a date this operator takes.
pub fn parse_clause(op: DateOp, value: &str) -> Option<DateClause> {
    let value = value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let spec = if let Some(&(_, keyword)) = KEYWORDS.iter().find(|(k, _)| *k == value) {
        DateSpec::Keyword(keyword)
    } else if let Some((first, last)) = value.split_once("..") {
        if op != DateOp::Received || (first.is_empty() && last.is_empty()) {
            return None;
        }
        let side = |text: &str| -> Option<Option<NaiveDate>> {
            if text.is_empty() {
                Some(None)
            } else {
                day(text).map(Some)
            }
        };
        DateSpec::Range(side(first)?, side(last)?)
    } else {
        DateSpec::Day(day(&value)?)
    };
    Some(DateClause { op, spec })
}

/// A day written `YYYY-M-D` or `YYYY/M/D` (the year in four digits, the
/// month and day in one or two), if there is such a day.
fn day(text: &str) -> Option<NaiveDate> {
    let separator = if text.contains('/') { '/' } else { '-' };
    let mut parts = text.split(separator);
    let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
    let digits = |s: &str, lengths: &[usize]| {
        lengths.contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())
    };
    if parts.next().is_some()
        || !digits(year, &[4])
        || !digits(month, &[1, 2])
        || !digits(day, &[1, 2])
    {
        return None;
    }
    NaiveDate::from_ymd_opt(year.parse().ok()?, month.parse().ok()?, day.parse().ok()?)
}

/// The span `clauses` leave together, in `tz`, at `now`. A week starts on
/// `week_starts_on`: 0 = Sunday … 6 = Saturday; others count modulo 7.
pub fn resolve<Tz: TimeZone>(
    clauses: &[DateClause],
    now: DateTime<Utc>,
    tz: &Tz,
    week_starts_on: u8,
) -> Received {
    let today = now.with_timezone(tz).date_naive();
    let mut received = Received::default();
    for clause in clauses {
        let (first, end) = days(clause.spec, today, week_starts_on);
        let start = first.map(|d| midnight(tz, d));
        let end = end.map(|d| midnight(tz, d));
        let (since, before) = match clause.op {
            DateOp::Received => (start, end),
            DateOp::After => (start, None),
            DateOp::Before => (None, start),
        };
        if let Some(since) = since {
            received.since = Some(received.since.map_or(since, |s| s.max(since)));
        }
        if let Some(before) = before {
            received.before = Some(received.before.map_or(before, |b| b.min(before)));
        }
    }
    received
}

/// A spec's days: the first, and the one after the last (`None` is open).
fn days(
    spec: DateSpec,
    today: NaiveDate,
    week_starts_on: u8,
) -> (Option<NaiveDate>, Option<NaiveDate>) {
    let next = |d: NaiveDate| d.succ_opt().unwrap_or(d);
    let back = |d: NaiveDate, n: u64| d.checked_sub_days(chrono::Days::new(n)).unwrap_or(d);
    let ahead = |d: NaiveDate, n: u64| d.checked_add_days(chrono::Days::new(n)).unwrap_or(d);
    let month_start = |d: NaiveDate| d.with_day(1).unwrap_or(d);
    let year_start = |year: i32| NaiveDate::from_ymd_opt(year, 1, 1).unwrap_or(today);
    let (first, end) = match spec {
        DateSpec::Day(d) => (d, next(d)),
        DateSpec::Range(first, last) => return (first, last.map(next)),
        DateSpec::Keyword(keyword) => match keyword {
            DateKeyword::Today => (today, next(today)),
            DateKeyword::Yesterday => (back(today, 1), today),
            DateKeyword::ThisWeek | DateKeyword::LastWeek => {
                let into_week = (today.weekday().num_days_from_sunday() + 7
                    - u32::from(week_starts_on % 7))
                    % 7;
                let start = back(today, u64::from(into_week));
                match keyword {
                    DateKeyword::ThisWeek => (start, ahead(start, 7)),
                    _ => (back(start, 7), start),
                }
            }
            DateKeyword::ThisMonth => {
                let start = month_start(today);
                (start, month_start(ahead(start, 31)))
            }
            DateKeyword::LastMonth => {
                let start = month_start(today);
                (month_start(back(start, 1)), start)
            }
            DateKeyword::ThisYear => (year_start(today.year()), year_start(today.year() + 1)),
            DateKeyword::LastYear => (year_start(today.year() - 1), year_start(today.year())),
        },
    };
    (Some(first), Some(end))
}

/// When `day` starts in `tz`. Where the clocks skip midnight, at 1:00;
/// where they repeat it, the first time it happens.
fn midnight<Tz: TimeZone>(tz: &Tz, day: NaiveDate) -> DateTime<Utc> {
    let start = day.and_time(NaiveTime::MIN);
    let at = |t: NaiveDateTime| tz.from_local_datetime(&t).earliest();
    match at(start).or_else(|| at(start + chrono::Duration::hours(1))) {
        Some(t) => t.with_timezone(&Utc),
        // A day the zone skipped whole: its offset at that hour in UTC.
        None => {
            let offset = tz.offset_from_utc_datetime(&start).fix();
            Utc.from_utc_datetime(&(start - offset))
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::FixedOffset;

    use super::*;

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn clause(op: DateOp, value: &str) -> Option<DateSpec> {
        parse_clause(op, value).map(|c| c.spec)
    }

    #[test]
    fn days_ranges_and_keywords() {
        use DateOp::*;
        assert_eq!(
            clause(Received, "2026-10-01"),
            Some(DateSpec::Day(ymd(2026, 10, 1)))
        );
        assert_eq!(
            clause(After, "2026/10/16"),
            Some(DateSpec::Day(ymd(2026, 10, 16)))
        );
        assert_eq!(
            clause(Before, "2026-1-5"),
            Some(DateSpec::Day(ymd(2026, 1, 5)))
        );
        assert_eq!(
            clause(Received, "2026-10-01..2026-10-15"),
            Some(DateSpec::Range(
                Some(ymd(2026, 10, 1)),
                Some(ymd(2026, 10, 15))
            ))
        );
        assert_eq!(
            clause(Received, "2026/10/01.."),
            Some(DateSpec::Range(Some(ymd(2026, 10, 1)), None))
        );
        assert_eq!(
            clause(Received, "..2026-1-5"),
            Some(DateSpec::Range(None, Some(ymd(2026, 1, 5))))
        );
        // Reversed: kept, and matches nothing.
        assert_eq!(
            clause(Received, "2026-10-15..2026-10-01"),
            Some(DateSpec::Range(
                Some(ymd(2026, 10, 15)),
                Some(ymd(2026, 10, 1))
            ))
        );
        for (text, keyword) in KEYWORDS {
            assert_eq!(
                clause(Received, text),
                Some(DateSpec::Keyword(*keyword)),
                "{text}"
            );
            assert_eq!(clause(After, text), Some(DateSpec::Keyword(*keyword)));
        }
        assert_eq!(
            clause(Received, "  This \t WEEK "),
            Some(DateSpec::Keyword(DateKeyword::ThisWeek))
        );
        assert_eq!(
            clause(Before, "Yesterday"),
            Some(DateSpec::Keyword(DateKeyword::Yesterday))
        );
    }

    #[test]
    fn anything_else_is_no_date() {
        use DateOp::*;
        for (op, text) in [
            (Received, "tomorrow"),
            (Received, "2026-02-30"),
            (Received, "2026-13-01"),
            (Received, ".."),
            (Received, "2026-10-01..yesterday"),
            (Received, "today..2026-10-01"),
            (Received, "2026-10-01..2026-10-02..2026-10-03"),
            (After, "2026-10-01..2026-10-02"),
            (Before, "..2026-10-02"),
            (Received, "26-10-01"),
            (Received, "02026-10-01"),
            (Received, "2026-001-01"),
            (Received, "2026-10/01"),
            (Received, "2026-10"),
            (Received, "2026-10-01-02"),
            (Received, "2026-+1-01"),
            (Received, "２０２６-10-01"),
            (Received, "thisweek"),
            (Received, ""),
        ] {
            assert_eq!(parse_clause(op, text), None, "{op:?} {text:?}");
        }
    }

    /// Sunday 18 October 2026, 10:00 where it is.
    fn sunday(tz: &FixedOffset) -> DateTime<Utc> {
        tz.with_ymd_and_hms(2026, 10, 18, 10, 0, 0)
            .unwrap()
            .with_timezone(&Utc)
    }

    /// The span `values` leave, as local days: the first, and the one
    /// after the last.
    fn span(
        tz: &FixedOffset,
        now: DateTime<Utc>,
        week_starts_on: u8,
        clauses: &[(DateOp, &str)],
    ) -> (Option<NaiveDateTime>, Option<NaiveDateTime>) {
        let clauses: Vec<DateClause> = clauses
            .iter()
            .map(|(op, value)| parse_clause(*op, value).unwrap())
            .collect();
        let received = resolve(&clauses, now, tz, week_starts_on);
        let local = |t: Option<DateTime<Utc>>| t.map(|t| t.with_timezone(tz).naive_local());
        (local(received.since), local(received.before))
    }

    fn at(y: i32, m: u32, d: u32) -> Option<NaiveDateTime> {
        Some(ymd(y, m, d).and_time(NaiveTime::MIN))
    }

    #[test]
    fn keywords_resolve_to_local_days() {
        use DateOp::*;
        for tz in [
            FixedOffset::west_opt(4 * 3600).unwrap(),
            FixedOffset::east_opt(9 * 3600).unwrap(),
        ] {
            let now = sunday(&tz);
            let span = |start: u8, value: &str| span(&tz, now, start, &[(Received, value)]);
            assert_eq!(span(0, "today"), (at(2026, 10, 18), at(2026, 10, 19)));
            assert_eq!(span(0, "yesterday"), (at(2026, 10, 17), at(2026, 10, 18)));
            // Weeks from Sunday: today starts one.
            assert_eq!(span(0, "this week"), (at(2026, 10, 18), at(2026, 10, 25)));
            assert_eq!(span(0, "last week"), (at(2026, 10, 11), at(2026, 10, 18)));
            // From Monday: today ends one.
            assert_eq!(span(1, "this week"), (at(2026, 10, 12), at(2026, 10, 19)));
            assert_eq!(span(1, "last week"), (at(2026, 10, 5), at(2026, 10, 12)));
            // From Saturday, and 8 counting as Monday.
            assert_eq!(span(6, "this week"), (at(2026, 10, 17), at(2026, 10, 24)));
            assert_eq!(span(8, "this week"), span(1, "this week"));
            assert_eq!(span(0, "this month"), (at(2026, 10, 1), at(2026, 11, 1)));
            assert_eq!(span(0, "last month"), (at(2026, 9, 1), at(2026, 10, 1)));
            assert_eq!(span(0, "this year"), (at(2026, 1, 1), at(2027, 1, 1)));
            assert_eq!(span(0, "last year"), (at(2025, 1, 1), at(2026, 1, 1)));
            // Days and ranges.
            assert_eq!(span(0, "2026-10-01"), (at(2026, 10, 1), at(2026, 10, 2)));
            assert_eq!(
                span(0, "2026-10-01..2026-10-15"),
                (at(2026, 10, 1), at(2026, 10, 16))
            );
            assert_eq!(span(0, "2026-10-01.."), (at(2026, 10, 1), None));
            assert_eq!(span(0, "..2026-10-15"), (None, at(2026, 10, 16)));
        }
        // Across January: last month is December, this month January.
        let tz = FixedOffset::west_opt(4 * 3600).unwrap();
        let january = tz
            .with_ymd_and_hms(2026, 1, 15, 10, 0, 0)
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            span(&tz, january, 0, &[(Received, "last month")]),
            (at(2025, 12, 1), at(2026, 1, 1))
        );
        assert_eq!(
            span(&tz, january, 0, &[(Received, "this month")]),
            (at(2026, 1, 1), at(2026, 2, 1))
        );
        // A day that is still yesterday in UTC.
        let late = tz
            .with_ymd_and_hms(2026, 10, 18, 23, 0, 0)
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            span(&tz, late, 0, &[(Received, "today")]),
            (at(2026, 10, 18), at(2026, 10, 19))
        );
    }

    #[test]
    fn after_before_and_intersections() {
        use DateOp::*;
        let tz = FixedOffset::west_opt(4 * 3600).unwrap();
        let now = sunday(&tz);
        // `after:` includes its day, `before:` doesn't.
        assert_eq!(
            span(&tz, now, 0, &[(After, "2026-10-01")]),
            (at(2026, 10, 1), None)
        );
        assert_eq!(
            span(&tz, now, 0, &[(Before, "2026-10-16")]),
            (None, at(2026, 10, 16))
        );
        assert_eq!(
            span(&tz, now, 0, &[(Before, "yesterday")]),
            (None, at(2026, 10, 17))
        );
        assert_eq!(
            span(&tz, now, 0, &[(After, "this month")]),
            (at(2026, 10, 1), None)
        );
        // Clauses intersect.
        assert_eq!(
            span(
                &tz,
                now,
                0,
                &[
                    (After, "2026-10-01"),
                    (Before, "2026/10/16"),
                    (Received, "this month")
                ]
            ),
            (at(2026, 10, 1), at(2026, 10, 16))
        );
        assert_eq!(
            span(
                &tz,
                now,
                1,
                &[(Received, "this week"), (Received, "this month")]
            ),
            (at(2026, 10, 12), at(2026, 10, 19))
        );

        let resolved = |clauses: &[(DateOp, &str)]| {
            let clauses: Vec<DateClause> = clauses
                .iter()
                .map(|(op, value)| parse_clause(*op, value).unwrap())
                .collect();
            resolve(&clauses, now, &tz, 0)
        };
        assert!(resolved(&[]).is_open());
        assert!(!resolved(&[(Received, "today")]).is_open());
        assert!(resolved(&[(Received, "2026-10-15..2026-10-01")]).is_impossible());
        assert!(resolved(&[(Received, "today"), (Received, "yesterday")]).is_impossible());
        assert!(resolved(&[(After, "2026-10-02"), (Before, "2026-10-02")]).is_impossible());
        assert!(!resolved(&[(After, "2026-10-01"), (Before, "2026-10-02")]).is_impossible());
    }

    #[test]
    fn contains_includes_since_and_excludes_before() {
        let since = Utc.with_ymd_and_hms(2026, 10, 1, 4, 0, 0).unwrap();
        let before = Utc.with_ymd_and_hms(2026, 10, 2, 4, 0, 0).unwrap();
        let received = Received {
            since: Some(since),
            before: Some(before),
        };
        let second = chrono::Duration::seconds(1);
        assert!(received.contains(since));
        assert!(received.contains(before - second));
        assert!(!received.contains(since - second));
        assert!(!received.contains(before));
        assert!(Received::default().contains(since));
        assert!(
            Received {
                since: Some(since),
                before: None
            }
            .contains(before + chrono::Duration::days(9000))
        );
    }

    #[test]
    fn servers_hear_dates_with_four_digit_years_only() {
        let utc = |y, m, d| Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap();
        let bounds = |since, before| Received { since, before };
        let resolved = |clauses: &[(DateOp, &str)]| {
            let clauses: Vec<DateClause> = clauses
                .iter()
                .map(|(op, value)| parse_clause(*op, value).unwrap())
                .collect();
            resolve(&clauses, utc(2026, 10, 18), &Utc, 0)
        };
        // Within the years servers take: as they are.
        let october = bounds(Some(utc(2026, 10, 1)), Some(utc(2026, 10, 2)));
        assert_eq!(october.for_servers(), october);
        assert_eq!(Received::default().for_servers(), Received::default());
        // A mistyped year (0201 for 2021): no lower bound, or one that
        // takes in all a server has.
        let typo = resolved(&[(DateOp::After, "0201-10-01")]);
        assert!(typo.since.unwrap().year() < 1000);
        assert_eq!(typo.for_servers(), Received::default());
        assert_eq!(
            resolved(&[(DateOp::Before, "0201-10-01")]).for_servers(),
            bounds(None, Some(utc(1970, 1, 1)))
        );
        // The far end of the calendar.
        assert_eq!(
            resolved(&[(DateOp::Received, "9999-12-31")]).for_servers(),
            bounds(Some(utc(9998, 12, 31)), None)
        );
        assert_eq!(
            resolved(&[(DateOp::Before, "9999-12-31")]).for_servers(),
            Received::default()
        );
        // Never less than the exact bounds.
        for (since, before) in [
            (Some(utc(1969, 12, 31)), Some(utc(2026, 1, 1))),
            (Some(utc(2026, 1, 1)), Some(utc(9999, 6, 1))),
        ] {
            let exact = bounds(since, before);
            let sent = exact.for_servers();
            for t in [
                since.unwrap(),
                before.unwrap() - chrono::Duration::seconds(1),
            ] {
                assert!(exact.contains(t) && sent.contains(t), "{t}");
            }
        }
    }

    /// Where midnight doesn't happen (the clocks skip it), a day starts
    /// when they show it next.
    #[test]
    fn a_skipped_midnight_starts_the_day_after_the_gap() {
        /// A zone at UTC−3 that moves to UTC−2 at midnight of 2026-10-04,
        /// as Paraguay's or Chile's clocks once did.
        #[derive(Debug, Clone, Copy)]
        struct Springing;
        impl TimeZone for Springing {
            type Offset = FixedOffset;
            fn from_offset(_: &FixedOffset) -> Self {
                Springing
            }
            fn offset_from_local_date(&self, _: &NaiveDate) -> chrono::LocalResult<FixedOffset> {
                unimplemented!()
            }
            fn offset_from_local_datetime(
                &self,
                local: &NaiveDateTime,
            ) -> chrono::LocalResult<FixedOffset> {
                let change = ymd(2026, 10, 4).and_time(NaiveTime::MIN);
                let gap_end = change + chrono::Duration::hours(1);
                if *local < change {
                    chrono::LocalResult::Single(FixedOffset::west_opt(3 * 3600).unwrap())
                } else if *local < gap_end {
                    chrono::LocalResult::None
                } else {
                    chrono::LocalResult::Single(FixedOffset::west_opt(2 * 3600).unwrap())
                }
            }
            fn offset_from_utc_date(&self, _: &NaiveDate) -> FixedOffset {
                unimplemented!()
            }
            fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> FixedOffset {
                if *utc < ymd(2026, 10, 4).and_hms_opt(3, 0, 0).unwrap() {
                    FixedOffset::west_opt(3 * 3600).unwrap()
                } else {
                    FixedOffset::west_opt(2 * 3600).unwrap()
                }
            }
        }
        assert_eq!(
            midnight(&Springing, ymd(2026, 10, 4)),
            Utc.with_ymd_and_hms(2026, 10, 4, 3, 0, 0).unwrap()
        );
        assert_eq!(
            midnight(&Springing, ymd(2026, 10, 3)),
            Utc.with_ymd_and_hms(2026, 10, 3, 3, 0, 0).unwrap()
        );
    }
}
