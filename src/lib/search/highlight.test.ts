import { describe, expect, it } from "vitest";

import {
  type HitPattern,
  MAX_HITS,
  MAX_PATTERN_WORDS,
  MAX_WORD_UNITS,
  findHits,
  fold,
  hitPatterns,
  leadToFirstHit,
  toRuns,
} from "./highlight";
import { HIGHLIGHT_VECTORS } from "./highlightVectors";
import { parseSearch } from "./query";

const patternsOf = (query: string) => hitPatterns(parseSearch(query));

describe("findHits", () => {
  it.each(HIGHLIGHT_VECTORS)(
    "marks $query in $field “$text”",
    ({ text, query, field, hits, ranges }) => {
      const found = findHits(text, patternsOf(query)?.[field] ?? []);
      expect(found.map(([start, end]) => text.slice(start, end))).toEqual(hits);
      if (ranges) expect(found).toEqual(ranges);
    },
  );

  it("merges hits that overlap or touch", () => {
    const words = (w: string): HitPattern => ({ words: [w], prefix: true, inside: null });
    // "ab" and "abc" overlap; "invoice" touches "invoice" in "invoiceinvoice" (inside).
    expect(findHits("abc", [words("ab"), words("abc")])).toEqual([[0, 3]]);
    expect(findHits("invoiceinvoice", [{ words: ["x"], prefix: true, inside: "invoice" }])).toEqual(
      [[0, 14]],
    );
    expect(findHits("a b", [words("a"), words("b")])).toEqual([
      [0, 1],
      [2, 3],
    ]);
  });

  it("stops at 1,000 hits, and reads no further than it is told", () => {
    const text = "ab ".repeat(1500);
    const many = findHits(text, [{ words: ["ab"], prefix: true, inside: null }]);
    expect(many).toHaveLength(MAX_HITS);
    const pattern: HitPattern[] = [{ words: ["zebra"], prefix: true, inside: null }];
    expect(findHits(`${"x ".repeat(100)}zebra`, pattern, 200)).toEqual([]);
    expect(findHits(`${"x ".repeat(100)}zebra`, pattern, 205)).toEqual([[200, 205]]);
  });

  it("folds ASCII the quick way exactly as the plain definition does", () => {
    /** The definition, a code point at a time, without the shortcut or the memo. */
    const plain = (text: string) => {
      let folded = "";
      const starts: number[] = [];
      const ends: number[] = [];
      let i = 0;
      for (const c of text) {
        const f = c.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();
        folded += f;
        for (let k = 0; k < f.length; k++) {
          starts.push(i);
          ends.push(i + c.length);
        }
        if (!f && ends.length) ends[ends.length - 1] = i + c.length;
        i += c.length;
      }
      return { folded, starts, ends };
    };
    for (const { text } of HIGHLIGHT_VECTORS) {
      expect(fold(text)).toEqual(plain(text));
      // Again, from the memo.
      expect(fold(text)).toEqual(plain(text));
    }
    expect(fold("Invoice #2023").folded).toBe("invoice #2023");
    expect(fold("Résumé İ").folded).toBe("resume i");
  });
});

describe("hitPatterns", () => {
  it("marks each term where rows show it", () => {
    expect(patternsOf("")).toBeNull();
    expect(patternsOf("is:unread")).toBeNull();
    expect(patternsOf('invoice subject:"q3 plan" from:al body:x to:dana cc:sam')).toEqual({
      subject: [
        { words: ["invoice"], prefix: true, inside: "invoice" },
        { words: ["q3", "plan"], prefix: false, inside: "q3 plan" },
      ],
      sender: [
        { words: ["invoice"], prefix: true, inside: "invoice" },
        { words: ["al"], prefix: true, inside: null },
      ],
      body: [
        { words: ["invoice"], prefix: true, inside: null },
        { words: ["x"], prefix: true, inside: null },
      ],
    });
  });

  it("keeps to what the message's bridge takes, so every view marks the same", () => {
    // A pasted sentence of 17 words: its first 16, each whole.
    const sentence =
      "Please let me know if you have any questions about the attached invoice for the October work";
    const [phrase] = patternsOf(`"${sentence}"`)!.body;
    expect(phrase).toEqual({
      words: sentence.toLowerCase().split(" ").slice(0, MAX_PATTERN_WORDS),
      prefix: false,
      inside: null,
    });
    expect(findHits(sentence, [phrase!])).toEqual([[0, sentence.indexOf(" work")]]);
    // Unquoted, the last word kept is whole too.
    const dotted = Array.from({ length: 17 }, (_, i) => `w${i}`).join(".");
    expect(patternsOf(dotted)!.body[0]).toMatchObject({ prefix: false });
    expect(patternsOf(dotted)!.body[0]!.words).toHaveLength(MAX_PATTERN_WORDS);
    // A word that folds longer than the bridge takes (each syllable is
    // three letters apart) isn't marked anywhere.
    const long = "한".repeat(50);
    expect(fold(long).folded.length).toBeGreaterThan(MAX_WORD_UNITS);
    expect(patternsOf(long)).toEqual({ subject: [], sender: [], body: [] });
  });
});

describe("toRuns", () => {
  it("splits the text at the hits", () => {
    expect(toRuns("we could get pineapple pizza", [[13, 22]])).toEqual([
      { text: "we could get ", hit: false },
      { text: "pineapple", hit: true },
      { text: " pizza", hit: false },
    ]);
    expect(toRuns("ab", [[0, 2]])).toEqual([{ text: "ab", hit: true }]);
    expect(toRuns("ab", [])).toEqual([{ text: "ab", hit: false }]);
    expect(toRuns("", [])).toEqual([]);
  });
});

describe("leadToFirstHit", () => {
  const hit = (text: string) => ({ text, hit: true });
  const plain = (text: string) => ({ text, hit: false });

  it("cuts what comes before the first hit to its last words", () => {
    const runs = [
      plain("Hi Alex, the plan for the week is set: we could get "),
      hit("pizza"),
      plain(" later"),
    ];
    expect(leadToFirstHit(runs, 20)).toEqual([
      plain("…set: we could get "),
      hit("pizza"),
      plain(" later"),
    ]);
    // Short enough, or no hit, or a hit first: as it was.
    expect(leadToFirstHit(runs, 80)).toEqual(runs);
    expect(leadToFirstHit([plain("x".repeat(100))], 20)).toEqual([plain("x".repeat(100))]);
    expect(leadToFirstHit([hit("pizza"), plain(" x")], 1)).toEqual([hit("pizza"), plain(" x")]);
    // One long word: its end.
    expect(leadToFirstHit([plain("y".repeat(30)), hit("z")], 10)).toEqual([
      plain(`…${"y".repeat(10)}`),
      hit("z"),
    ]);
  });
});
