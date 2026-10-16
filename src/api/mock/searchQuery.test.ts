import { describe, expect, it } from "vitest";

import type { Message } from "../types";
import { createDemoState } from "./data";
import {
  MAX_TERM_CHARS,
  type SearchQuery,
  isEmptySearch,
  isNarrowEnoughForServers,
  matchesSearch,
  parseSearch,
} from "./searchQuery";

const FIELD_NAMES = {
  any: "A",
  from: "From",
  to: "To",
  cc: "Cc",
  subject: "Subject",
  body: "Body",
} as const;

/** "A:invoice", "Subject:q3 plan [p]": the notation of the shared test vectors. */
function terms(query: SearchQuery): string[] {
  return query.terms.map((t) => `${FIELD_NAMES[t.field]}:${t.text}${t.phrase ? " [p]" : ""}`);
}

// The core's parser tests (crates/core/src/search/mod.rs) share these cases.
const VECTORS: [input: string, terms: string[], hasAttachment: boolean][] = [
  ["invoice", ["A:invoice"], false],
  ["Invoice March", ["A:invoice", "A:march"], false],
  ['"quarterly report"', ["A:quarterly report [p]"], false],
  ["from:megan", ["From:megan"], false],
  ["From:Megan lunch", ["From:megan", "A:lunch"], false],
  [
    'subject:"Quarterly Report" from:megan@contoso.com',
    ["Subject:quarterly report [p]", "From:megan@contoso.com"],
    false,
  ],
  ["to:", [], false],
  ["from: megan", ["A:megan"], false],
  ["has:attachment budget", ["A:budget"], true],
  ["HasAttachments:yes", [], true],
  ["has:pdf", [], false],
  ["re:hello", ["A:re", "A:hello"], false],
  ['"unterminated phrase', ["A:unterminated phrase [p]"], false],
  ["“curly quotes”", ["A:curly quotes [p]"], false],
  ["*** ??", [], false],
  ["-draft +final", ["A:draft", "A:final"], false],
  ["o’brien.", ["A:o'brien"], false],
  ["AND OR NOT", ["A:and", "A:or", "A:not"], false],
  ['subject:"', [], false],
  ["lucía", ["A:lucía"], false],
  ['from:"Megan Bowen"', ["From:megan bowen [p]"], false],
  ["cc:x@y.z", ["Cc:x@y.z"], false],
  ["2023-118", ["A:2023-118"], false],
  ["हिन्दी ಕನ್ನಡ", ["A:हिन्दी", "A:ಕನ್ನಡ"], false],
  ["re\u0301sume\u0301", ["A:résumé"], false],
  ["\u0301abc -\u0301x\u0301", ["A:abc", "A:x\u0301"], false],
  ['"a\u0007b\tc\u0000"', ["A:ab c [p]"], false],
  ["a b c d e f g h i j k l m n o p q r", "abcdefghijklmnop".split("").map((c) => `A:${c}`), false],
];

describe("isNarrowEnoughForServers", () => {
  // As the core's single_letters_are_left_to_the_device.
  it.each(["a", "a b c", "from:a", '"a"', "c++ x", "日"])("leaves %j to the device", (input) => {
    expect(isNarrowEnoughForServers(parseSearch(input))).toBe(false);
  });
  it.each(["ab", "a ab", "from:al", '"a b"', "日本", "has:attachment"])(
    "asks the servers for %j",
    (input) => {
      expect(isNarrowEnoughForServers(parseSearch(input))).toBe(true);
    },
  );
});

describe("parseSearch", () => {
  it.each(VECTORS)("parses %j like the core", (input, expected, hasAttachment) => {
    const query = parseSearch(input);
    expect(terms(query)).toEqual(expected);
    expect(query.hasAttachment).toBe(hasAttachment);
    expect(isEmptySearch(query)).toBe(expected.length === 0 && !hasAttachment);
  });

  it("keeps to the limits, cutting at a character", () => {
    const long = "é".repeat(200);
    const [term] = parseSearch(long).terms;
    expect(Array.from(term!.text)).toHaveLength(MAX_TERM_CHARS);
    // 1,024 characters are read: the word after them isn't.
    expect(terms(parseSearch(`${" ".repeat(1020)}abc def`))).toEqual(["A:abc"]);
    expect(terms(parseSearch(`${"x ".repeat(511)}ab`))).toHaveLength(16);
    // Emoji are two UTF-16 units but one character.
    const emoji = parseSearch(`"${"😀".repeat(130)} a"`);
    expect(emoji.terms).toEqual([]);
  });

  it("takes whitespace to be what Rust takes it to be", () => {
    // U+3000 and U+2028 are whitespace to both; U+FEFF only to JavaScript's
    // trim(), so a phrase keeps it, as the core's does.
    expect(parseSearch('"\u3000a\u2028b "').terms).toEqual([
      { field: "any", text: "a b", phrase: true },
    ]);
    expect(parseSearch('"\ufeffmegan"').terms).toEqual([
      { field: "any", text: "\ufeffmegan", phrase: true },
    ]);
  });

  it("never fails, whatever it is given", () => {
    for (const input of ['"', "\\", '\\"', "\u0000\u0007", "😀", "מה שלום", ":", "from::", '""']) {
      const query = parseSearch(input);
      for (const t of query.terms) {
        expect(t.text).not.toMatch(/["\\]/);
        expect(t.text).toMatch(/[\p{L}\p{N}]/u);
      }
    }
  });
});

describe("matchesSearch", () => {
  const messages = createDemoState().messages;
  const find = (subject: RegExp): Message => messages.find((m) => subject.test(m.subject))!;
  const lisbon = find(/Your trip to Lisbon/);
  const checklist = find(/Atlas launch checklist/);
  const matches = (text: string, m: Message, infix = true) =>
    matchesSearch(parseSearch(text), m, { infix });

  it("matches words by prefix, in any order", () => {
    expect(matches("lisb", lisbon)).toBe(true);
    expect(matches("trip lisbon", lisbon)).toBe(true);
    expect(matches("lisbonx", lisbon)).toBe(false);
  });

  it("matches phrases word for word", () => {
    expect(matches('"trip to lisbon"', lisbon)).toBe(true);
    expect(matches('"trip to lisb"', lisbon, false)).toBe(false);
    expect(matches('"lisbon trip"', lisbon)).toBe(false);
    // Locally a phrase is also found inside a subject, as the text it is.
    expect(matches('"trip to lisb"', lisbon)).toBe(true);
  });

  it("matches operators against their field only", () => {
    expect(matches("from:priya", checklist)).toBe(true);
    expect(matches("from:jonas", checklist)).toBe(false);
    expect(matches("cc:jonas", checklist)).toBe(true);
    expect(matches("to:alex", checklist)).toBe(true);
    expect(matches("subject:checklist", checklist)).toBe(true);
    expect(matches("subject:rollback", checklist)).toBe(false);
    expect(matches("body:rollback", checklist)).toBe(true);
    expect(matches("has:attachment checklist", checklist)).toBe(true);
    expect(matches("has:attachment", lisbon)).toBe(lisbon.hasAttachments);
  });

  it("matches inside subjects and senders only locally, and from 3 characters", () => {
    expect(matches("aunch", checklist)).toBe(true);
    expect(matches("aunch", checklist, false)).toBe(false);
    expect(matches("au", checklist)).toBe(false);
    expect(matches("subject:aunch", checklist)).toBe(true);
    expect(matches("from:aman", checklist)).toBe(true);
    // Not in the body: "ollback" is inside a word of the body only.
    expect(matches("ollback", checklist)).toBe(false);
  });

  it("ignores accents and case", () => {
    const lucia = messages.find((m) => m.from?.name === "Lucía Fernández")!;
    expect(matches("lucia", lucia)).toBe(true);
    expect(matches("LUCÍA fernandez", lucia)).toBe(true);
  });

  it("matches the start of an address across its parts", () => {
    expect(matches("from:priya.raman@acme", checklist)).toBe(true);
    expect(matches("from:priya.raman@acme.example", checklist)).toBe(true);
    expect(matches("from:priya.raman@example", checklist, false)).toBe(false);
  });
});
