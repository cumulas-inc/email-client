import { describe, expect, it } from "vitest";

import {
  clearFilters,
  findClauses,
  formatValue,
  hasFilters,
  isDateClause,
  isFlaggedClause,
  isHasClause,
  isReadClause,
  removeClauses,
  replaceToken,
  setClause,
  tokenAt,
} from "./edit";
import { parseSearch } from "./query";

describe("findClauses", () => {
  it("finds operators with their values where they are in the text", () => {
    const text = 'lunch from:megan in:"Project X" "a phrase" is:unread';
    expect(findClauses(text)).toEqual([
      { name: "from", value: "megan", quoted: false, start: 6, end: 16 },
      { name: "in", value: "Project X", quoted: true, start: 17, end: 31 },
      { name: "is", value: "unread", quoted: false, start: 43, end: 52 },
    ]);
  });

  it("reads curly quotes as quotes, and a dangling operator as one with no value", () => {
    expect(findClauses("in:“Say Hi” x")).toEqual([
      { name: "in", value: "Say Hi", quoted: true, start: 0, end: 11 },
    ]);
    expect(findClauses("x from:")).toEqual([
      { name: "from", value: "", quoted: false, start: 2, end: 7 },
    ]);
    expect(findClauses("from: megan")).toEqual([
      { name: "from", value: "", quoted: false, start: 0, end: 5 },
    ]);
    // Unterminated: to the end.
    expect(findClauses('subject:"q3 pl')).toEqual([
      { name: "subject", value: "q3 pl", quoted: true, start: 0, end: 14 },
    ]);
  });

  it("names attachment aliases has, and leaves what isn't an operator alone", () => {
    expect(findClauses("HasAttachments:yes hasattachment:true Has:x").map((c) => c.name)).toEqual([
      "has",
      "has",
      "has",
    ]);
    // Inside a phrase, or not an operator, or not at the start of a run.
    expect(findClauses('"from:megan" re:hello xfrom:a constructor:x')).toEqual([]);
    expect(findClauses("IS:Flagged")).toEqual([
      { name: "is", value: "Flagged", quoted: false, start: 0, end: 10 },
    ]);
  });
});

describe("removeClauses and setClause", () => {
  it("leave one space where a clause went, and trim", () => {
    expect(removeClauses("a  is:read   b", isReadClause)).toBe("a b");
    expect(removeClauses("is:read b", isReadClause)).toBe("b");
    expect(removeClauses("b is:read", isReadClause)).toBe("b");
    expect(removeClauses("a is:read is:flagged b", () => true)).toBe("a b");
    expect(removeClauses('x"y"is:read"z"', isReadClause)).toBe('x"y" "z"');
    expect(removeClauses("  untouched  ", isReadClause)).toBe("untouched");
  });

  it("change only the filter they're for", () => {
    // Unread on, with Flagged there.
    expect(setClause("is:flagged is:read x", isReadClause, "is:unread")).toBe(
      "is:flagged x is:unread",
    );
    // Unread off.
    expect(removeClauses("is:read is:unread is:flagged", isReadClause)).toBe("is:flagged");
    // Flagged off.
    expect(removeClauses("is:flagged is:unread", isFlaggedClause)).toBe("is:unread");
    // Has attachment off.
    expect(removeClauses("has:attachment x", isHasClause)).toBe("x");
    // A date preset over other dates.
    expect(setClause("after:2026-10-01 before:2026-10-16 x", isDateClause, "received:today")).toBe(
      "x received:today",
    );
    expect(setClause("", isDateClause, "received:today")).toBe("received:today");
    expect(setClause("x received:today", isDateClause, null)).toBe("x");
  });
});

describe("clearFilters", () => {
  it("removes every filter and keeps the words, to:, cc:, subject: and body:", () => {
    expect(
      clearFilters(
        'budget from:megan to:al has:attachment is:unread in:"Project X" account:work ' +
          'received:today after:2026-01-01 before:2026-02-01 cc:dana subject:q3 body:x "a b"',
      ),
    ).toBe('budget to:al cc:dana subject:q3 body:x "a b"');
    expect(hasFilters("budget is:unread")).toBe(true);
    expect(hasFilters("budget to:al subject:x")).toBe(false);
  });
});

describe("formatValue", () => {
  it("writes a value bare when it can, else quoted without quotes or backslashes", () => {
    expect(formatValue("megan@contoso.com")).toBe("megan@contoso.com");
    expect(formatValue("Q1\\Q2")).toBe("Q1\\Q2");
    expect(formatValue("Project  X")).toBe('"Project  X"');
    expect(formatValue(" Clients")).toBe('" Clients"');
    expect(formatValue("Q1\\Q2 plans")).toBe('"Q1Q2 plans"');
    expect(formatValue('Say "Hi"')).toBe('"Say Hi"');
    expect(formatValue("Say“Hi”")).toBe('"SayHi"');
    expect(formatValue("")).toBe('""');
    // What it writes reads back as the same folder.
    for (const name of ["Project  X", "Q1\\Q2 plans", 'Say "Hi"', "a　b"]) {
      expect(parseSearch(`in:${formatValue(name)}`).folders).toHaveLength(1);
    }
  });
});

describe("tokenAt", () => {
  it("finds the word or clause being typed", () => {
    // At the start: nothing typed yet.
    expect(tokenAt("lunch", 0)).toEqual({
      start: 0,
      end: 0,
      operator: null,
      value: "",
      quoted: false,
    });
    // In the middle of a word, and at its end.
    expect(tokenAt("lunch friday", 3)).toEqual({
      start: 0,
      end: 5,
      operator: null,
      value: "lunch",
      quoted: false,
    });
    expect(tokenAt("lunch fri", 9)).toMatchObject({ start: 6, end: 9, value: "fri" });
    // After a space: a new, empty token.
    expect(tokenAt("lunch ", 6)).toEqual({
      start: 6,
      end: 6,
      operator: null,
      value: "",
      quoted: false,
    });
    // An operator's value.
    expect(tokenAt("x from:meg", 10)).toEqual({
      start: 2,
      end: 10,
      operator: "from",
      value: "meg",
      quoted: false,
    });
    expect(tokenAt("x from:", 7)).toMatchObject({ operator: "from", value: "" });
    // Inside quotes.
    expect(tokenAt('in:"Project X', 13)).toEqual({
      start: 0,
      end: 13,
      operator: "in",
      value: "Project X",
      quoted: true,
    });
    expect(tokenAt('"quarterly re', 13)).toEqual({
      start: 0,
      end: 13,
      operator: null,
      value: "quarterly re",
      quoted: true,
    });
    // In an operator's name: a word as typed.
    expect(tokenAt("from:megan", 2)).toMatchObject({ operator: null, value: "from:megan" });
  });
});

describe("replaceToken", () => {
  it("puts the insertion in the token's place, with a space and the caret after", () => {
    expect(replaceToken("lunch fr", { start: 6, end: 8 }, "from:", false)).toEqual({
      text: "lunch from:",
      caret: 11,
    });
    expect(replaceToken("x is:un", { start: 2, end: 7 }, "is:unread", true)).toEqual({
      text: "x is:unread ",
      caret: 12,
    });
    // A space already there isn't doubled.
    expect(replaceToken("meg lunch", { start: 0, end: 3 }, "from:megan@x.org", true)).toEqual({
      text: "from:megan@x.org lunch",
      caret: 17,
    });
    expect(replaceToken("", { start: 0, end: 0 }, "is:flagged", true)).toEqual({
      text: "is:flagged ",
      caret: 11,
    });
  });
});
