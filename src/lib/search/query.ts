// The search language, as the core parses it (crates/core/src/search): words,
// phrases, operators and filters, what dates they leave, and the folders and
// accounts `in:` and `account:` name. The UI parses with it to know when the
// servers are asked and what the filters above the results show; the demo
// backend searches with it, so the UI is tested against the app's own
// semantics. Keep it in step with the core; query.test.ts holds the test
// vectors the core's tests share.

import {
  addDays,
  addMonths,
  startOfDay,
  startOfMonth,
  startOfWeek,
  startOfYear,
  type Day,
} from "date-fns";

import type { Account, Folder, FolderRole } from "../../api/types";

export type SearchField = "any" | "from" | "to" | "cc" | "subject" | "body";

export interface SearchTerm {
  field: SearchField;
  /** Lowercased; never contains `"` or `\`; at least one letter or digit. */
  text: string;
  /** Quoted: the words exactly, without matching the last one as a prefix. */
  phrase: boolean;
}

export type DateOp = "after" | "before" | "received";

export type DateKeyword =
  | "today"
  | "yesterday"
  | "this week"
  | "last week"
  | "this month"
  | "last month"
  | "this year"
  | "last year";

/** A day, `YYYY-MM-DD`. */
export type PlainDay = string;

export type DateSpec =
  | { kind: "day"; day: PlainDay }
  /** The days from the first through the last; null is open. */
  | { kind: "range"; first: PlainDay | null; last: PlainDay | null }
  | { kind: "keyword"; keyword: DateKeyword };

export interface DateClause {
  op: DateOp;
  spec: DateSpec;
}

export interface SearchQuery {
  /** All must match. */
  terms: SearchTerm[];
  hasAttachment: boolean;
  /** `is:read` (true) or `is:unread` (false); the last typed wins. */
  read: boolean | null;
  /** `is:flagged`. */
  flagged: boolean;
  /** `received:`, `after:`, `before:`: all must hold. */
  dates: DateClause[];
  /** `in:` values as scope keys: a message in any of the folders they name matches. */
  folders: string[];
  /** `account:` values as scope keys: any. */
  accounts: string[];
}

export const MAX_TERMS = 16;
/** Dates, folders and accounts kept, each; later ones are dropped. */
export const MAX_FILTER_VALUES = 8;
export const MAX_TERM_CHARS = 128;
export const MAX_INPUT_CHARS = 1024;

/** Operator names, lowercased, before their `:`. */
export const OPERATORS: ReadonlySet<string> = new Set([
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
]);

const FIELDS: Partial<Record<string, SearchField>> = {
  from: "from",
  to: "to",
  cc: "cc",
  subject: "subject",
  body: "body",
};

/** The date words, as typed (lowercased, one space between words). */
export const DATE_KEYWORDS: readonly DateKeyword[] = [
  "today",
  "yesterday",
  "this week",
  "last week",
  "this month",
  "last month",
  "this year",
  "last year",
];

// Rust's char::is_alphanumeric, char::is_whitespace and char::is_control.
const ALPHANUMERIC = /[\p{Alphabetic}\p{N}]/u;
const WHITESPACE = /\p{White_Space}/u;
const WHITESPACE_RUNS = /\p{White_Space}+/gu;
const CONTROL = /\p{Cc}/u;
/** Combining marks (viramas, vowel signs, accents typed on their own): part of the word before. */
const MARK = /\p{M}/u;
/** Kept inside words (addresses, "o'brien", "2023-118"), trimmed off their ends. */
const WORD_CHARS = new Set(["@", ".", "_", "-", "'", "+"]);
const ASCII_LETTER = /^[A-Za-z]$/;
/** Curly double quotes, read as `"`. */
export const CURLY_QUOTES = "“”„‟″";

const isSpace = (c: string) => WHITESPACE.test(c);
const hasAlphanumeric = (s: string) => ALPHANUMERIC.test(s);

/** At most `max` characters (code points, as Rust counts them). */
function cut(text: string, max: number): string {
  const chars = Array.from(text);
  return chars.length > max ? chars.slice(0, max).join("") : text;
}

/** Whitespace runs as one space, trimmed (Rust's split_whitespace and join). */
function singleSpaced(text: string): string {
  return text.split(WHITESPACE_RUNS).filter(Boolean).join(" ");
}

/** The words of a bare run: split on anything but letters, digits and WORD_CHARS. */
function words(run: string): string[] {
  const out: string[] = [];
  let piece = "";
  const flush = () => {
    let start = 0;
    const chars = Array.from(piece);
    let end = chars.length;
    while (start < end && (WORD_CHARS.has(chars[start]!) || MARK.test(chars[start]!))) start++;
    while (end > start && WORD_CHARS.has(chars[end - 1]!)) end--;
    const word = chars.slice(start, end).join("");
    if (hasAlphanumeric(word)) out.push(word.toLowerCase());
    piece = "";
  };
  for (const c of run) {
    if (ALPHANUMERIC.test(c) || WORD_CHARS.has(c) || MARK.test(c)) piece += c;
    else flush();
  }
  flush();
  return out;
}

/** A quoted phrase's text: no backslashes or control characters, single spaces, as typed. */
function phraseText(raw: string): string {
  // Trimmed by hand: trim() also takes U+FEFF, which isn't whitespace to Rust.
  return singleSpaced(
    Array.from(raw.replaceAll("\\", ""))
      .filter((c) => isSpace(c) || !CONTROL.test(c))
      .join(""),
  );
}

// --- Dates ------------------------------------------------------------------------

function daysInMonth(year: number, month: number): number {
  if (month === 2) return (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0 ? 29 : 28;
  return [4, 6, 9, 11].includes(month) ? 30 : 31;
}

const pad = (n: number, width = 2) => String(n).padStart(width, "0");

/**
 * A day written `YYYY-M-D` or `YYYY/M/D` (the year in four digits, the month
 * and day in one or two), as `YYYY-MM-DD`, if there is such a day.
 */
export function parseDay(text: string): PlainDay | null {
  const parts = text.split(text.includes("/") ? "/" : "-");
  if (parts.length !== 3) return null;
  const [y, m, d] = parts as [string, string, string];
  if (!/^[0-9]{4}$/.test(y) || !/^[0-9]{1,2}$/.test(m) || !/^[0-9]{1,2}$/.test(d)) return null;
  const year = Number(y);
  const month = Number(m);
  const day = Number(d);
  if (month < 1 || month > 12 || day < 1 || day > daysInMonth(year, month)) return null;
  return `${pad(year, 4)}-${pad(month)}-${pad(day)}`;
}

/** `null` for a value that isn't a date this operator takes. */
export function parseDateClause(op: DateOp, value: string): DateClause | null {
  const text = singleSpaced(value).toLowerCase();
  const keyword = DATE_KEYWORDS.find((k) => k === text);
  if (keyword) return { op, spec: { kind: "keyword", keyword } };
  const dots = text.indexOf("..");
  if (dots >= 0) {
    const first = text.slice(0, dots);
    const last = text.slice(dots + 2);
    if (op !== "received" || (!first && !last)) return null;
    const firstDay = first ? parseDay(first) : null;
    const lastDay = last ? parseDay(last) : null;
    if ((first && !firstDay) || (last && !lastDay)) return null;
    return { op, spec: { kind: "range", first: firstDay, last: lastDay } };
  }
  const day = parseDay(text);
  return day ? { op, spec: { kind: "day", day } } : null;
}

/** Received-time bounds: since inclusive, before exclusive; null is open. */
export interface ResolvedDates {
  since: Date | null;
  before: Date | null;
}

/** When `day` starts here (local time); where the clocks skip midnight, the first moment after. */
export function localMidnight(day: PlainDay): Date {
  const [y, m, d] = day.split("-").map(Number) as [number, number, number];
  const date = new Date(0);
  date.setFullYear(y, m - 1, d);
  date.setHours(0, 0, 0, 0);
  return date;
}

/**
 * A spec's span here: its first moment, and the one after its last (null is
 * open). Each bound is its own day's start, as the core takes it: where the
 * clocks skip one day's midnight, that day starts at 1:00, the others still
 * at 0:00.
 */
function span(spec: DateSpec, today: Date, weekStartsOn: number): [Date | null, Date | null] {
  const [start, end] = days(spec, today, weekStartsOn);
  return [start && startOfDay(start), end && startOfDay(end)];
}

/** A spec's first day, and the day after its last (null is open), at some time of each. */
function days(spec: DateSpec, today: Date, weekStartsOn: number): [Date | null, Date | null] {
  switch (spec.kind) {
    case "day": {
      const start = localMidnight(spec.day);
      return [start, addDays(start, 1)];
    }
    case "range":
      return [
        spec.first ? localMidnight(spec.first) : null,
        spec.last ? addDays(localMidnight(spec.last), 1) : null,
      ];
    case "keyword": {
      switch (spec.keyword) {
        case "today":
          return [today, addDays(today, 1)];
        case "yesterday":
          return [addDays(today, -1), today];
        case "this week":
        case "last week": {
          const day = (((weekStartsOn % 7) + 7) % 7) as Day;
          const start = startOfWeek(today, { weekStartsOn: day });
          return spec.keyword === "this week"
            ? [start, addDays(start, 7)]
            : [addDays(start, -7), start];
        }
        case "this month": {
          const start = startOfMonth(today);
          return [start, addMonths(start, 1)];
        }
        case "last month": {
          const start = startOfMonth(today);
          return [addMonths(start, -1), start];
        }
        case "this year": {
          const start = startOfYear(today);
          return [start, addMonths(start, 12)];
        }
        case "last year": {
          const start = startOfYear(today);
          return [addMonths(start, -12), start];
        }
      }
    }
  }
}

/**
 * The span the query's dates leave together, in this computer's time zone,
 * at `now`. A week starts on `weekStartsOn`: 0 = Sunday … 6 = Saturday.
 */
export function resolveDates(query: SearchQuery, now: Date, weekStartsOn: number): ResolvedDates {
  const today = new Date(now);
  today.setHours(0, 0, 0, 0);
  let since: Date | null = null;
  let before: Date | null = null;
  for (const clause of query.dates) {
    const [start, end] = span(clause.spec, today, weekStartsOn);
    const [from, until] =
      clause.op === "received"
        ? [start, end]
        : clause.op === "after"
          ? [start, null]
          : [null, start];
    if (from && (!since || from > since)) since = from;
    if (until && (!before || until < before)) before = until;
  }
  return { since, before };
}

/** No time at all: a reversed range, or dates that don't overlap. */
export function isImpossible(dates: ResolvedDates): boolean {
  return !!dates.since && !!dates.before && dates.since >= dates.before;
}

/** Whether a message received at `at` is within the dates. */
export function withinDates(dates: ResolvedDates, at: Date): boolean {
  return (!dates.since || at >= dates.since) && (!dates.before || at < dates.before);
}

// --- Folders and accounts -----------------------------------------------------------

/**
 * NFC; quotes, backslashes and control characters removed; whitespace runs as
 * one space; trimmed; lowercased. What `in:` and `account:` values are kept
 * as, and what names are compared by. Curly apostrophes become `'`, as the
 * parser reads them.
 */
export function scopeKey(s: string): string {
  const kept = Array.from(s.normalize("NFC"))
    .map((c) => (c === "’" || c === "‘" ? "'" : c))
    .filter(
      (c) =>
        c !== '"' && c !== "\\" && !CURLY_QUOTES.includes(c) && (isSpace(c) || !CONTROL.test(c)),
    )
    .join("");
  return singleSpaced(kept).toLowerCase();
}

/** The role an `in:` value names, as the well-known folders' names (and their other names) say. */
const ROLES: Partial<Record<string, FolderRole>> = {
  inbox: "inbox",
  drafts: "drafts",
  draft: "drafts",
  sent: "sent",
  archive: "archive",
  junk: "junk",
  spam: "junk",
  trash: "trash",
  deleted: "trash",
  outbox: "outbox",
};

/** The role `in:value` names, if it names one (`in:inbox`, `in:spam`). */
export function folderRoleOf(value: string): FolderRole | null {
  return ROLES[scopeKey(value)] ?? null;
}

/** Whether `in:value` names `folder`: by its role or by its name. Not its subfolders. */
export function folderMatches(value: string, folder: Pick<Folder, "name" | "role">): boolean {
  const key = scopeKey(value);
  return ROLES[key] === folder.role || scopeKey(folder.name) === key;
}

/** Whether `account:value` names `account`: by its address or its name. */
export function accountMatches(
  value: string,
  account: Pick<Account, "email" | "displayName">,
): boolean {
  const key = scopeKey(value);
  return scopeKey(account.email) === key || scopeKey(account.displayName) === key;
}

// --- Parsing ------------------------------------------------------------------------

export function emptyQuery(): SearchQuery {
  return {
    terms: [],
    hasAttachment: false,
    read: null,
    flagged: false,
    dates: [],
    folders: [],
    accounts: [],
  };
}

/** A filter's value (`has:`, `is:`, `in:`, `account:` and dates); one it doesn't know is left out. */
function addFilter(query: SearchQuery, name: string, value: string, quoted: boolean): void {
  const text = (quoted ? phraseText(value) : value).toLowerCase();
  switch (name) {
    case "has":
      if (text === "attachment" || text === "attachments") query.hasAttachment = true;
      break;
    case "hasattachment":
    case "hasattachments":
      if (text === "true" || text === "yes") query.hasAttachment = true;
      break;
    case "is":
      if (text === "unread") query.read = false;
      else if (text === "read") query.read = true;
      else if (text === "flagged") query.flagged = true;
      break;
    case "in":
    case "account": {
      // Kept whole, as a name: `in:Q1\Q2`, `in:"Project X"`.
      const key = scopeKey(value);
      const values = name === "in" ? query.folders : query.accounts;
      if (hasAlphanumeric(key) && values.length < MAX_FILTER_VALUES) values.push(key);
      break;
    }
    default: {
      const clause = parseDateClause(name as DateOp, text);
      if (clause && query.dates.length < MAX_FILTER_VALUES) query.dates.push(clause);
    }
  }
}

/** Parses a search the way the core does; never fails. */
export function parseSearch(input: string): SearchQuery {
  // Composed (NFC), as mail is: "é" typed as "e" and an accent is the same word.
  const head = Array.from(input).slice(0, MAX_INPUT_CHARS).join("").normalize("NFC");
  const chars = Array.from(head)
    .slice(0, MAX_INPUT_CHARS)
    .map((c) => {
      if (CURLY_QUOTES.includes(c)) return '"';
      if (c === "’" || c === "‘") return "'";
      return c;
    });
  const query = emptyQuery();
  const push = (field: SearchField, text: string, phrase: boolean) => {
    const cutText = cut(text.toLowerCase(), MAX_TERM_CHARS);
    if (query.terms.length < MAX_TERMS && hasAlphanumeric(cutText)) {
      query.terms.push({ field, text: cutText, phrase });
    }
  };
  let i = 0;
  /** From `i`: to the next `"` (consumed) or the end. */
  const quoted = () => {
    const close = chars.indexOf('"', i);
    const end = close < 0 ? chars.length : close;
    const text = chars.slice(i, end).join("");
    i = close < 0 ? chars.length : close + 1;
    return text;
  };
  /** From `i`: a run of anything but whitespace, stopping before `"`. */
  const bare = () => {
    const start = i;
    while (i < chars.length && chars[i] !== '"' && !isSpace(chars[i]!)) i++;
    return chars.slice(start, i).join("");
  };

  while (i < chars.length) {
    const c = chars[i]!;
    if (isSpace(c)) {
      i++;
      continue;
    }
    if (c === '"') {
      i++;
      push("any", phraseText(quoted()), true);
      continue;
    }
    let letters = i;
    while (letters < chars.length && ASCII_LETTER.test(chars[letters]!)) letters++;
    const name = chars.slice(i, letters).join("").toLowerCase();
    if (letters > i && chars[letters] === ":" && OPERATORS.has(name)) {
      i = letters + 1;
      let value: string;
      let phrase = false;
      if (chars[i] === '"') {
        i++;
        value = quoted();
        phrase = true;
      } else {
        value = bare();
        // "from:" alone says nothing: what follows is read as usual.
        if (!value) continue;
      }
      const field = FIELDS[name];
      if (!field) addFilter(query, name, value, phrase);
      else if (phrase) push(field, phraseText(value), true);
      else for (const word of words(value)) push(field, word, false);
      continue;
    }
    // Plain words, and unknown operators (re:hello) as words.
    for (const word of words(bare())) push("any", word, false);
  }
  return query;
}

/**
 * Nothing to look for. Where to look (`in:`, `account:`) alone only narrows
 * a search, so it counts as nothing.
 */
export function isEmptySearch(query: SearchQuery): boolean {
  return (
    query.terms.length === 0 &&
    !query.hasAttachment &&
    query.read === null &&
    !query.flagged &&
    query.dates.length === 0
  );
}

/** Only where to look (`in:`, `account:`), and nothing to look for. */
export function isScopeOnly(query: SearchQuery): boolean {
  return isEmptySearch(query) && (query.folders.length > 0 || query.accounts.length > 0);
}

export function hasTerms(query: SearchQuery): boolean {
  return query.terms.length > 0;
}

/** Letters and digits one term at least needs for the servers to be asked. */
export const MIN_SERVER_TERM_CHARS = 2;

/**
 * Whether the servers are worth asking, as the core decides: some term has
 * two letters or digits, or it asks for attachments, unread or flagged
 * mail, or dates. Single letters ("a b", "from:a") match most of a mailbox,
 * and every server would look through every folder for them; so does
 * `is:read`, and `in:` or `account:` alone.
 */
export function isNarrowEnoughForServers(query: SearchQuery): boolean {
  return (
    query.hasAttachment ||
    query.read === false ||
    query.flagged ||
    query.dates.length > 0 ||
    query.terms.some(
      (t) => Array.from(t.text).filter((c) => hasAlphanumeric(c)).length >= MIN_SERVER_TERM_CHARS,
    )
  );
}

/**
 * Whether the query has words to rank by: a plain, `subject:` or `body:`
 * term. `from:`, `to:` and `cc:` only pick messages by address, so they are
 * listed newest first.
 */
export function ranks(query: SearchQuery): boolean {
  return query.terms.some((t) => t.field === "any" || t.field === "subject" || t.field === "body");
}
