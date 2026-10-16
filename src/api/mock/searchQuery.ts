// The search language, as the core parses it (crates/core/src/search), and
// a matcher that finds what the core's full-text index finds: the demo
// backend searches with these, so the UI is tested against the app's own
// semantics. Keep both in step with the core; searchQuery.test.ts holds the
// test vectors the core's tests share.

import type { EmailAddress, Message } from "../types";

export type SearchField = "any" | "from" | "to" | "cc" | "subject" | "body";

export interface SearchTerm {
  field: SearchField;
  /** Lowercased; never contains `"` or `\`; at least one letter or digit. */
  text: string;
  /** Quoted: the words exactly, without matching the last one as a prefix. */
  phrase: boolean;
}

export interface SearchQuery {
  terms: SearchTerm[];
  hasAttachment: boolean;
}

export const MAX_TERMS = 16;
export const MAX_TERM_CHARS = 128;
export const MAX_INPUT_CHARS = 1024;

const FIELDS: Record<string, SearchField> = {
  from: "from",
  to: "to",
  cc: "cc",
  subject: "subject",
  body: "body",
};
const HAS = new Set(["has", "hasattachment", "hasattachments"]);

// Rust's char::is_alphanumeric and char::is_whitespace.
const ALPHANUMERIC = /[\p{Alphabetic}\p{N}]/u;
const WHITESPACE = /\p{White_Space}/u;
/** Kept inside words (addresses, "o'brien", "2023-118"), trimmed off their ends. */
const WORD_CHARS = new Set(["@", ".", "_", "-", "'", "+"]);
const ASCII_LETTER = /^[A-Za-z]$/;

const isSpace = (c: string) => WHITESPACE.test(c);
const hasAlphanumeric = (s: string) => ALPHANUMERIC.test(s);

/** At most `max` characters (code points, as Rust counts them). */
function cut(text: string, max: number): string {
  const chars = Array.from(text);
  return chars.length > max ? chars.slice(0, max).join("") : text;
}

/** The words of a bare run: split on anything but letters, digits and WORD_CHARS. */
function words(run: string): string[] {
  const out: string[] = [];
  let piece = "";
  const flush = () => {
    let start = 0;
    const chars = Array.from(piece);
    let end = chars.length;
    while (start < end && WORD_CHARS.has(chars[start]!)) start++;
    while (end > start && WORD_CHARS.has(chars[end - 1]!)) end--;
    const word = chars.slice(start, end).join("");
    if (hasAlphanumeric(word)) out.push(word.toLowerCase());
    piece = "";
  };
  for (const c of run) {
    if (ALPHANUMERIC.test(c) || WORD_CHARS.has(c)) piece += c;
    else flush();
  }
  flush();
  return out;
}

/** A quoted phrase's text: no backslashes, single spaces, lowercased. */
function phraseText(raw: string): string {
  return Array.from(raw.replaceAll("\\", ""))
    .map((c) => (isSpace(c) ? " " : c))
    .join("")
    .replace(/ {2,}/g, " ")
    .trim()
    .toLowerCase();
}

/** Parses a search the way the core does; never fails. */
export function parseSearch(input: string): SearchQuery {
  const chars = Array.from(input)
    .slice(0, MAX_INPUT_CHARS)
    .map((c) => {
      if ("“”„‟″".includes(c)) return '"';
      if (c === "’" || c === "‘") return "'";
      return c;
    });
  const query: SearchQuery = { terms: [], hasAttachment: false };
  const push = (field: SearchField, text: string, phrase: boolean) => {
    const cutText = cut(text, MAX_TERM_CHARS);
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
    if (letters > i && chars[letters] === ":" && (name in FIELDS || HAS.has(name))) {
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
      if (HAS.has(name)) {
        const answer = (phrase ? phraseText(value) : value).toLowerCase();
        const yes =
          name === "has"
            ? answer === "attachment" || answer === "attachments"
            : answer === "true" || answer === "yes";
        if (yes) query.hasAttachment = true;
      } else {
        const field = FIELDS[name]!;
        if (phrase) push(field, phraseText(value), true);
        else for (const word of words(value)) push(field, word, false);
      }
      continue;
    }
    // Plain words, and unknown operators (re:hello) as words.
    for (const word of words(bare())) push("any", word, false);
  }
  return query;
}

export function isEmptySearch(query: SearchQuery): boolean {
  return query.terms.length === 0 && !query.hasAttachment;
}

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
}

/** Whether the message matches every part of the query. */
export function matchesSearch(
  query: SearchQuery,
  message: Message,
  { infix }: MatchOptions,
): boolean {
  if (query.hasAttachment && !message.hasAttachments) return false;
  const subject = message.subject;
  const sender = addressText(message.from);
  const text: Record<Exclude<SearchField, "any">, string> = {
    from: sender,
    to: message.to.map(addressText).join(" "),
    cc: message.cc.map(addressText).join(" "),
    subject,
    // The demo treats every body as already fetched (the app indexes a body
    // once it was opened, the preview until then).
    body: message.bodyText ?? message.preview,
  };
  return query.terms.every((term) => {
    const fields =
      term.field === "any"
        ? (["subject", "from", "to", "cc", "body"] as const)
        : ([term.field] as const);
    const words = tokens(term.text);
    if (fields.some((f) => tokensMatch(words, tokens(text[f]), term.phrase))) return true;
    if (!infix || Array.from(term.text).length < 3) return false;
    const inside = normalize(term.text);
    const infixFields = fields.filter((f) => f === "subject" || f === "from");
    return infixFields.some((f) => normalize(text[f]).includes(inside));
  });
}
