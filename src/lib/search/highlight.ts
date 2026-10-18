// Finding a search's words in text, to mark them: in the rows of the results
// (subjects, senders and previews) and in the open message. Words are found
// as the index finds them: folded (accents off, lowercased), by their start
// unless quoted, and inside subjects and senders from three characters on.
// The message frame's bridge (public/message-bridge.js) folds and matches
// the same way; highlightVectors.ts holds the cases both are tested with.

import type { TextRun } from "../../api/types";
import type { SearchQuery, SearchTerm } from "./query";

export interface HitPattern {
  /** Folded words, matched as consecutive words of the text. */
  words: string[];
  /** The last word matches the start of a word (the term wasn't quoted). */
  prefix: boolean;
  /** Also found inside words: the folded term, 3+ chars (subjects and senders only). */
  inside: string | null;
}

export interface HitPatterns {
  subject: HitPattern[];
  sender: HitPattern[];
  body: HitPattern[];
}

/** How much of a text findHits reads by default: long plain-text bodies stop there. */
export const MAX_SCAN_CHARS = 250_000;
/** Most hits findHits returns. */
export const MAX_HITS = 1000;
/** Characters a term needs to be found inside words, as the index's trigrams do. */
const INSIDE_CHARS = 3;

const NOT_ASCII = /[\u0080-\uffff]/;
const MARKS = /\p{M}/gu;
const TOKEN = /[\p{L}\p{N}]+/gu;

/** Each code point's fold: it doesn't depend on its neighbors. */
const folds = new Map<number, string>();

function foldCodePoint(cp: number, char: string): string {
  let f = folds.get(cp);
  if (f === undefined) {
    f = char.normalize("NFD").replace(MARKS, "").toLowerCase();
    folds.set(cp, f);
  }
  return f;
}

/**
 * `text` without accents, lowercased, and for each of its UTF-16 units where
 * it came from in `text`: `starts[k]` to `ends[k]`. A mark stays with the
 * letter before it, so a hit takes in the accents and vowel signs of its
 * last letter.
 */
export function fold(text: string): { folded: string; starts: number[]; ends: number[] } {
  if (!NOT_ASCII.test(text)) {
    const starts = Array.from({ length: text.length }, (_, k) => k);
    return { folded: text.toLowerCase(), starts, ends: starts.map((k) => k + 1) };
  }
  let folded = "";
  const starts: number[] = [];
  const ends: number[] = [];
  let i = 0;
  for (const char of text) {
    const cp = char.codePointAt(0)!;
    const end = i + char.length;
    const f = foldCodePoint(cp, char);
    if (f) {
      folded += f;
      for (let k = 0; k < f.length; k++) {
        starts.push(i);
        ends.push(end);
      }
    } else if (ends.length > 0) {
      ends[ends.length - 1] = end;
    }
    i = end;
  }
  return { folded, starts, ends };
}

interface Token {
  text: string;
  start: number;
  end: number;
}

function tokens(folded: string): Token[] {
  return Array.from(folded.matchAll(TOKEN), (m) => ({
    text: m[0],
    start: m.index,
    end: m.index + m[0].length,
  }));
}

/** A term's pattern, without or with what it's found inside words by; null when it has no words. */
function pattern(term: SearchTerm, inside: boolean): HitPattern | null {
  const folded = fold(term.text).folded;
  const words = tokens(folded).map((t) => t.text);
  if (words.length === 0) return null;
  return {
    words,
    prefix: !term.phrase,
    inside: inside && Array.from(term.text).length >= INSIDE_CHARS ? folded : null,
  };
}

/** What to mark of the query's words in subjects, senders and bodies; null without words. */
export function hitPatterns(query: SearchQuery): HitPatterns | null {
  if (query.terms.length === 0) return null;
  const patterns: HitPatterns = { subject: [], sender: [], body: [] };
  const add = (list: HitPattern[], p: HitPattern | null) => {
    if (p) list.push(p);
  };
  for (const term of query.terms) {
    switch (term.field) {
      case "any":
        add(patterns.subject, pattern(term, true));
        add(patterns.sender, pattern(term, true));
        add(patterns.body, pattern(term, false));
        break;
      case "subject":
        add(patterns.subject, pattern(term, true));
        break;
      case "from":
        add(patterns.sender, pattern(term, true));
        break;
      case "body":
        add(patterns.body, pattern(term, false));
        break;
      // To and Cc aren't shown in rows.
    }
  }
  return patterns;
}

/** Sorted, with ranges that overlap or touch made one. */
function merge(ranges: [number, number][]): [number, number][] {
  const sorted = [...ranges].sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  const out: [number, number][] = [];
  for (const [start, end] of sorted) {
    const last = out[out.length - 1];
    if (last && start <= last[1]) last[1] = Math.max(last[1], end);
    else out.push([start, end]);
  }
  return out;
}

/**
 * Where the patterns are found in `text`: UTF-16 ranges, merged, at most
 * 1,000. Reads at most `maxChars` of the text.
 */
export function findHits(
  text: string,
  patterns: readonly HitPattern[],
  maxChars = MAX_SCAN_CHARS,
): [number, number][] {
  if (patterns.length === 0 || !text) return [];
  const { folded, starts, ends } = fold(text.length > maxChars ? text.slice(0, maxChars) : text);
  const words = tokens(folded);
  const ranges: [number, number][] = [];
  const hit = (from: number, to: number) => ranges.push([starts[from]!, ends[to - 1]!]);
  for (const p of patterns) {
    const last = p.words.length - 1;
    for (let t = 0; t + last < words.length; t++) {
      const all = p.words.every((w, k) =>
        k === last && p.prefix ? words[t + k]!.text.startsWith(w) : words[t + k]!.text === w,
      );
      if (!all) continue;
      const end = p.prefix ? words[t + last]!.start + p.words[last]!.length : words[t + last]!.end;
      hit(words[t]!.start, end);
    }
    if (p.inside) {
      for (let at = folded.indexOf(p.inside); at >= 0;) {
        hit(at, at + p.inside.length);
        at = folded.indexOf(p.inside, at + p.inside.length);
      }
    }
  }
  return merge(ranges).slice(0, MAX_HITS);
}

/** The text as runs, hits marked. */
export function toRuns(text: string, hits: readonly [number, number][]): TextRun[] {
  const runs: TextRun[] = [];
  let at = 0;
  for (const [start, end] of hits) {
    if (start > at) runs.push({ text: text.slice(at, start), hit: false });
    if (end > start) runs.push({ text: text.slice(start, end), hit: true });
    at = Math.max(at, end);
  }
  if (at < text.length) runs.push({ text: text.slice(at), hit: false });
  return runs;
}

/**
 * The runs with what comes before the first hit cut to about its last
 * `max` characters, from a word on, after "…": a row only a line wide
 * then shows the hit.
 */
export function leadToFirstHit(runs: readonly TextRun[], max = 40): TextRun[] {
  const [first, ...rest] = runs;
  if (!first || first.hit || !rest.some((r) => r.hit)) return [...runs];
  const chars = Array.from(first.text);
  if (chars.length <= max) return [...runs];
  let lead = chars.slice(-max).join("");
  const space = lead.search(/\s\S/u);
  if (space >= 0) lead = lead.slice(space + 1);
  return [{ text: `…${lead}`, hit: false }, ...rest];
}
