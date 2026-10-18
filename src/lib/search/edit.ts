// Editing the search text: the filters above the results and the search
// box's suggestions change the text the way a person would type it, so the
// text stays the one thing a search is (see query.ts). Clauses are found
// with the parser's tokenization on the text as typed (curly quotes read as
// `"`, nothing else changed), so their offsets are the text's own.

import { CURLY_QUOTES, OPERATORS } from "./query";

/** An operator and its value in the text: `from:megan`, `in:"Project X"`, a dangling `from:`. */
export interface Clause {
  /** Lowercased; `hasattachment` and `hasattachments` are "has". */
  name: string;
  /** As typed, without its quotes. */
  value: string;
  quoted: boolean;
  /** Where it is in the text (UTF-16 offsets, end exclusive). */
  start: number;
  end: number;
}

/** A piece of the text: a clause, a phrase or a bare run (words). */
interface Unit {
  start: number;
  end: number;
  clause: Clause | null;
  /** Its value: a phrase or a clause's without quotes, a bare run as typed. */
  value: string;
  quoted: boolean;
  /** For a clause, where its value starts. */
  valueStart: number;
}

const WHITESPACE = /\p{White_Space}/u;
const ASCII_LETTER = /[A-Za-z]/;

const isQuote = (c: string | undefined) => c === '"' || (!!c && CURLY_QUOTES.includes(c));
const isSpace = (c: string | undefined) => !!c && WHITESPACE.test(c);

/** Every unit of `text`, in order, as the parser reads it. */
function units(text: string): Unit[] {
  const out: Unit[] = [];
  let i = 0;
  /** From `at`: to the next quote, and where the text goes on after it. */
  const toQuote = (at: number): [value: string, next: number] => {
    let end = at;
    while (end < text.length && !isQuote(text[end])) end++;
    return [text.slice(at, end), Math.min(end + 1, text.length)];
  };
  /** From `at`: a run of anything but whitespace, stopping before a quote. */
  const bareEnd = (at: number) => {
    let end = at;
    while (end < text.length && !isQuote(text[end]) && !isSpace(text[end])) end++;
    return end;
  };
  while (i < text.length) {
    const c = text[i];
    if (isSpace(c)) {
      i++;
      continue;
    }
    const start = i;
    if (isQuote(c)) {
      const [value, next] = toQuote(i + 1);
      out.push({ start, end: next, clause: null, value, quoted: true, valueStart: start + 1 });
      i = next;
      continue;
    }
    let letters = i;
    while (letters < text.length && ASCII_LETTER.test(text[letters]!)) letters++;
    const name = text.slice(i, letters).toLowerCase();
    if (letters > i && text[letters] === ":" && OPERATORS.has(name)) {
      const valueStart = letters + 1;
      const quoted = isQuote(text[valueStart]);
      const [value, end] = quoted
        ? toQuote(valueStart + 1)
        : [text.slice(valueStart, bareEnd(valueStart)), bareEnd(valueStart)];
      const clause: Clause = {
        name: name.startsWith("has") ? "has" : name,
        value,
        quoted,
        start,
        end,
      };
      out.push({ start, end, clause, value, quoted, valueStart });
      i = end;
      continue;
    }
    const end = bareEnd(i);
    out.push({ start, end, clause: null, value: text.slice(i, end), quoted: false, valueStart: i });
    i = end;
  }
  return out;
}

/** Every operator in the text with its value, including a dangling `from:` (value ""). */
export function findClauses(text: string): Clause[] {
  return units(text).flatMap((u) => (u.clause ? [u.clause] : []));
}

/** Removes every clause passing `test`; where one was, one space at most is left. Trimmed. */
export function removeClauses(text: string, test: (c: Clause) => boolean): string {
  const kept: string[] = [];
  let at = 0;
  for (const c of findClauses(text)) {
    if (!test(c)) continue;
    kept.push(text.slice(at, c.start));
    at = c.end;
  }
  kept.push(text.slice(at));
  // Each seam, the whitespace on both sides of what went, becomes one space.
  return kept
    .reduce((joined, part) => {
      const left = joined.replace(/\p{White_Space}+$/u, "");
      const right = part.replace(/^\p{White_Space}+/u, "");
      return left && right ? `${left} ${right}` : left + right;
    })
    .trim();
}

/** Removes every clause passing `test`, then appends `clause` (if any). */
export function setClause(
  text: string,
  test: (c: Clause) => boolean,
  clause: string | null,
): string {
  const rest = removeClauses(text, test);
  if (!clause) return rest;
  return rest ? `${rest} ${clause}` : clause;
}

/** A value as a clause writes it: bare when it can be, else quoted (without quotes or backslashes in it). */
export function formatValue(value: string): string {
  if (/^[^\s\p{White_Space}"“”„‟″]+$/u.test(value)) return value;
  return `"${value.replace(/["“”„‟″\\]/g, "")}"`;
}

/** Operators the filters above the results write; words, to:, cc:, subject: and body: stay. */
export const FILTER_NAMES: ReadonlySet<string> = new Set([
  "from",
  "has",
  "is",
  "in",
  "account",
  "received",
  "after",
  "before",
]);

export const isFilterClause = (c: Clause) => FILTER_NAMES.has(c.name);

/** Removes every filter (from:, has:, is:, in:, account:, dates), keeping the words. */
export function clearFilters(text: string): string {
  return removeClauses(text, isFilterClause);
}

/** Whether the text has filters for Clear filters to remove. */
export function hasFilters(text: string): boolean {
  return findClauses(text).some(isFilterClause);
}

// The filters' own clauses (values compared lowercased).
const lower = (c: Clause) => c.value.toLowerCase();
export const isReadClause = (c: Clause) =>
  c.name === "is" && (lower(c) === "read" || lower(c) === "unread");
export const isFlaggedClause = (c: Clause) => c.name === "is" && lower(c) === "flagged";
export const isHasClause = (c: Clause) => c.name === "has";
export const isDateClause = (c: Clause) =>
  c.name === "received" || c.name === "after" || c.name === "before";
export const isInClause = (c: Clause) => c.name === "in";
export const isAccountClause = (c: Clause) => c.name === "account";
export const isFromClause = (c: Clause) => c.name === "from";

/** The piece of the text being typed at `caret`, for suggestions. */
export interface Token {
  start: number;
  end: number;
  /** The operator whose value is being typed (lowercased, has* as "has"), or null for words. */
  operator: string | null;
  /** The value or word, without its quotes. */
  value: string;
  /** In quotes: a phrase or a quoted value. */
  quoted: boolean;
}

/**
 * The token the caret is in or just after; after whitespace, or before
 * anything typed, an empty one at the caret.
 */
export function tokenAt(text: string, caret: number): Token {
  const unit = units(text).find((u) => u.start < caret && caret <= u.end);
  if (!unit) return { start: caret, end: caret, operator: null, value: "", quoted: false };
  const { clause } = unit;
  if (clause && caret >= unit.valueStart) {
    return {
      start: unit.start,
      end: unit.end,
      operator: clause.name,
      value: unit.value,
      quoted: unit.quoted,
    };
  }
  // In an operator's name, or in words: the text as typed.
  return {
    start: unit.start,
    end: unit.end,
    operator: null,
    value: clause ? text.slice(unit.start, unit.end) : unit.value,
    quoted: !clause && unit.quoted,
  };
}

/**
 * Puts `insertion` in place of the token, and a space after it with `space`
 * (unless one follows already); the caret goes after them.
 */
export function replaceToken(
  text: string,
  token: { start: number; end: number },
  insertion: string,
  space: boolean,
): { text: string; caret: number } {
  const before = text.slice(0, token.start);
  const after = text.slice(token.end);
  const spaced = space && !isSpace(after[0]);
  const next = before + insertion + (spaced ? " " : "") + after;
  return { text: next, caret: before.length + insertion.length + (space ? 1 : 0) };
}
