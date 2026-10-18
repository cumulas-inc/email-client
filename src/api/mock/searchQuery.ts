// A matcher that finds what the core's full-text index finds, and orders it
// as the core ranks it: the demo backend searches with these, so the UI is
// tested against the app's own semantics. The search language itself is
// src/lib/search/query.ts. Keep both in step with the core
// (crates/core/src/search, crates/core/src/store/mail.rs).

import { type HitPatterns, findHits, toRuns } from "../../lib/search/highlight";
import {
  type ResolvedDates,
  type SearchField,
  type SearchQuery,
  type SearchTerm,
  withinDates,
} from "../../lib/search/query";
import type { EmailAddress, Message, MessageSummary, TextRun } from "../types";

// --- Matching -----------------------------------------------------------------

/** As the index compares text: without accents, lowercased. */
function normalize(text: string): string {
  return text.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();
}

/** The index's tokens: runs of letters and digits. */
function tokens(text: string): string[] {
  return normalize(text)
    .split(/[^\p{L}\p{N}]+/u)
    .filter(Boolean);
}

function addressText(a: EmailAddress | null): string {
  return a ? `${a.name ?? ""} ${a.address}` : "";
}

/** The term's tokens in a row in the field's, the last one as a prefix unless quoted. */
function tokensMatch(term: string[], field: string[], phrase: boolean): boolean {
  if (term.length === 0) return false;
  for (let start = 0; start + term.length <= field.length; start++) {
    const all = term.every((t, k) => {
      const word = field[start + k]!;
      return k === term.length - 1 && !phrase ? word.startsWith(t) : word === t;
    });
    if (all) return true;
  }
  return false;
}

export interface MatchOptions {
  /**
   * Also match inside words, as the app's local index does for subjects and
   * senders (3 characters or more); a server search doesn't.
   */
  infix: boolean;
  /** Match anywhere in each field, as IMAP's SEARCH does (substrings). */
  substrings?: boolean;
}

type FieldTexts = Record<Exclude<SearchField, "any">, string>;

function fieldTexts(message: Message): FieldTexts {
  return {
    from: addressText(message.from),
    to: message.to.map(addressText).join(" "),
    cc: message.cc.map(addressText).join(" "),
    subject: message.subject,
    // The text of an opened message; callers pass others without it, as
    // the app indexes their preview until then.
    body: message.bodyText ?? message.preview,
  };
}

function fieldsOf(term: SearchTerm) {
  return term.field === "any"
    ? (["subject", "from", "to", "cc", "body"] as const)
    : ([term.field] as const);
}

/** Whether the term's words are in one of `fields`, as the index finds words. */
function termInFields(term: SearchTerm, text: FieldTexts, fields: readonly (keyof FieldTexts)[]) {
  const words = tokens(term.text);
  return fields.some((f) => tokensMatch(words, tokens(text[f]), term.phrase));
}

/** Whether the message matches every term of the query (its words; admits checks the rest). */
export function matchesSearch(
  query: SearchQuery,
  message: Message,
  { infix, substrings = false }: MatchOptions,
): boolean {
  const text = fieldTexts(message);
  return query.terms.every((term) => {
    const fields = fieldsOf(term);
    if (substrings) return fields.some((f) => normalize(text[f]).includes(normalize(term.text)));
    if (termInFields(term, text, fields)) return true;
    if (!infix || Array.from(term.text).length < 3) return false;
    const inside = normalize(term.text);
    const infixFields = fields.filter((f) => f === "subject" || f === "from");
    return infixFields.some((f) => normalize(text[f]).includes(inside));
  });
}

/**
 * Whether the message passes every filter but the words, as the core's
 * SearchQuery::admits: attachments, read, flagged, the dates and, when
 * given, the folders (local ids).
 */
export function admits(
  query: SearchQuery,
  message: MessageSummary,
  dates: ResolvedDates,
  folderIds: ReadonlySet<string> | null,
): boolean {
  return (
    (!query.hasAttachment || message.hasAttachments) &&
    (query.read === null || message.isRead === query.read) &&
    (!query.flagged || message.isFlagged) &&
    withinDates(dates, new Date(message.receivedAt)) &&
    (!folderIds || folderIds.has(message.folderId))
  );
}

// --- Ranking (crates/core/src/search/rank.rs) -------------------------------------

/** What a ranked term found only outside the subject and sender counts for. */
export const OTHER_FIELD_SHARE = 0.3;
/** Matched only inside words (subjects and senders). */
export const INFIX_RELEVANCE = 0.6;
/** A server hit the index doesn't match: it matched in the body beyond the preview. */
export const SERVER_RELEVANCE = 0.225;
export const RECENCY_HALF_LIFE_DAYS = 30;
export const RECENCY_FLOOR = 0.4;
/** Rows that get a snippet, the first in the final order. */
export const MAX_SNIPPETS = 100;

const DAY_MS = 86_400_000;

export interface RankedHit {
  /** As the index has it: the text of an opened message, the preview of others. */
  message: Message;
  /** Found by a server search (an included id). */
  fromServer: boolean;
}

/**
 * The share of the query's ranked terms (plain, `subject:`, `body:`) found
 * in the subject or sender: 1 for a `subject:` term, OTHER_FIELD_SHARE for
 * a `body:` one, and for a plain term 1 if its words start a word of the
 * subject or sender (name or address), else OTHER_FIELD_SHARE; the mean.
 */
function fieldShare(query: SearchQuery, message: Message): number {
  const text = fieldTexts(message);
  const shares = query.terms.flatMap((term) => {
    switch (term.field) {
      case "subject":
        return [1];
      case "body":
        return [OTHER_FIELD_SHARE];
      case "any":
        return [termInFields(term, text, ["subject", "from"]) ? 1 : OTHER_FIELD_SHARE];
      default:
        return [];
    }
  });
  return shares.length ? shares.reduce((a, b) => a + b, 0) / shares.length : 1;
}

/**
 * The core's score with bm25 taken as the best for every row (the demo has
 * no bm25), so a row's relevance is its field share: words in the subject
 * or sender first, then recent mail.
 */
export function score(query: SearchQuery, hit: RankedHit, now: Date): number {
  let relevance: number;
  if (matchesSearch(query, hit.message, { infix: false })) {
    relevance = fieldShare(query, hit.message);
  } else {
    relevance = hit.fromServer ? SERVER_RELEVANCE : INFIX_RELEVANCE;
  }
  const age = Math.max(0, now.getTime() - Date.parse(hit.message.receivedAt)) / DAY_MS;
  const recency = 0.5 ** (age / RECENCY_HALF_LIFE_DAYS);
  return relevance * (RECENCY_FLOOR + (1 - RECENCY_FLOOR) * recency);
}

/** Best first: score, then newest, then by id. */
export function rankHits<T extends RankedHit>(hits: T[], query: SearchQuery, now: Date): T[] {
  const scored = hits.map((hit) => ({ hit, score: score(query, hit, now) }));
  scored.sort(
    (a, b) =>
      b.score - a.score ||
      b.hit.message.receivedAt.localeCompare(a.hit.message.receivedAt) ||
      (a.hit.message.id < b.hit.message.id ? -1 : a.hit.message.id > b.hit.message.id ? 1 : 0),
  );
  return scored.map((s) => s.hit);
}

// --- Snippets -----------------------------------------------------------------------

/** Words of context before the first hit, and the most a snippet holds. */
const SNIPPET_WORDS_BEFORE = 8;
const SNIPPET_CHARS = 160;

/**
 * The body's words around its first hit, as runs: about 8 words before it,
 * at most 160 characters, "…" where cut, whitespace runs as one space. Null
 * without a body (unopened mail is indexed by its preview, which the row
 * shows already) or without a hit in it.
 */
export function mockSnippet(message: Message, patterns: HitPatterns | null): TextRun[] | null {
  const body = message.bodyText;
  if (!body || !patterns || patterns.body.length === 0) return null;
  const hits = findHits(body, patterns.body);
  const first = hits[0];
  if (!first) return null;
  let start = first[0];
  for (let words = 0; start > 0 && words < SNIPPET_WORDS_BEFORE; words++) {
    // Back over the whitespace before, then the word before it.
    while (start > 0 && /\s/.test(body[start - 1]!)) start--;
    while (start > 0 && !/\s/.test(body[start - 1]!)) start--;
  }
  const end = Math.min(body.length, start + SNIPPET_CHARS);
  const inside = hits
    .filter(([s]) => s < end)
    .map(([s, e]): [number, number] => [s - start, Math.min(e, end) - start]);
  const runs = toRuns(body.slice(start, end), inside).map((r) => ({
    ...r,
    text: r.text.replace(/\s+/g, " "),
  }));
  if (start > 0) runs.unshift({ text: "…", hit: false });
  if (end < body.length) runs.push({ text: "…", hit: false });
  // Neighbors alike made one; empty ones gone.
  const merged: TextRun[] = [];
  for (const run of runs) {
    const last = merged[merged.length - 1];
    if (!run.text) continue;
    if (last && last.hit === run.hit) last.text += run.text;
    else merged.push({ ...run });
  }
  return merged.some((r) => r.hit) ? merged : null;
}
