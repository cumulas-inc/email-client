import { afterEach, describe, expect, it, vi } from "vitest";

import {
  type DateClause,
  MAX_FILTER_VALUES,
  MAX_TERM_CHARS,
  type SearchQuery,
  accountMatches,
  folderMatches,
  isEmptySearch,
  isImpossible,
  isNarrowEnoughForServers,
  parseDateClause,
  parseSearch,
  ranks,
  resolveDates,
  scopeKey,
} from "./query";

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

function spec(clause: DateClause): string {
  switch (clause.spec.kind) {
    case "day":
      return clause.spec.day;
    case "range":
      return `${clause.spec.first ?? ""}..${clause.spec.last ?? ""}`;
    case "keyword":
      return clause.spec.keyword;
  }
}

/**
 * Filters as the shared test vectors write them, in a fixed order:
 * attachments, read, flagged, dates as typed, folders, accounts.
 */
function filters(query: SearchQuery): string[] {
  return [
    ...(query.hasAttachment ? ["has:attachment"] : []),
    ...(query.read === null ? [] : [query.read ? "is:read" : "is:unread"]),
    ...(query.flagged ? ["is:flagged"] : []),
    ...query.dates.map((d) => `${d.op}:${spec(d)}`),
    ...query.folders.map((f) => `in:${f}`),
    ...query.accounts.map((a) => `account:${a}`),
  ];
}

// The core's parser tests (crates/core/src/search/mod.rs) share these cases.
const VECTORS: [input: string, terms: string[], filters: string[]][] = [
  ["invoice", ["A:invoice"], []],
  ["Invoice March", ["A:invoice", "A:march"], []],
  ['"quarterly report"', ["A:quarterly report [p]"], []],
  ["from:megan", ["From:megan"], []],
  ["From:Megan lunch", ["From:megan", "A:lunch"], []],
  [
    'subject:"Quarterly Report" from:megan@contoso.com',
    ["Subject:quarterly report [p]", "From:megan@contoso.com"],
    [],
  ],
  ["to:", [], []],
  ["from: megan", ["A:megan"], []],
  ["has:attachment budget", ["A:budget"], ["has:attachment"]],
  ["HasAttachments:yes", [], ["has:attachment"]],
  ["has:pdf", [], []],
  ["re:hello", ["A:re", "A:hello"], []],
  ['"unterminated phrase', ["A:unterminated phrase [p]"], []],
  ["“curly quotes”", ["A:curly quotes [p]"], []],
  ["*** ??", [], []],
  ["-draft +final", ["A:draft", "A:final"], []],
  ["o’brien.", ["A:o'brien"], []],
  ["AND OR NOT", ["A:and", "A:or", "A:not"], []],
  ['subject:"', [], []],
  ["lucía", ["A:lucía"], []],
  ['from:"Megan Bowen"', ["From:megan bowen [p]"], []],
  ["cc:x@y.z", ["Cc:x@y.z"], []],
  ["2023-118", ["A:2023-118"], []],
  ["हिन्दी ಕನ್ನಡ", ["A:हिन्दी", "A:ಕನ್ನಡ"], []],
  ["re\u0301sume\u0301", ["A:résumé"], []],
  ["\u0301abc -\u0301x\u0301", ["A:abc", "A:x\u0301"], []],
  ['"a\u0007b\tc\u0000"', ["A:ab c [p]"], []],
  ["a b c d e f g h i j k l m n o p q r", "abcdefghijklmnop".split("").map((c) => `A:${c}`), []],
  // Filters.
  ["is:unread", [], ["is:unread"]],
  ["IS:Flagged invoice", ["A:invoice"], ["is:flagged"]],
  ["is:read is:unread", [], ["is:unread"]],
  ["is:starred", [], []],
  ['is:"unread"', [], ["is:unread"]],
  ["in:inbox budget", ["A:budget"], ["in:inbox"]],
  ['in:"Project  X" in:Archive', [], ["in:project x", "in:archive"]],
  ['in:" Clients"', [], ["in:clients"]],
  ["in:Q1\\Q2", [], ["in:q1q2"]],
  ['in:"Say Hi"', [], ["in:say hi"]],
  ["in:*** x", ["A:x"], []],
  ["account:Megan@Contoso.com", [], ["account:megan@contoso.com"]],
  ['account:"Work Mail"', [], ["account:work mail"]],
  ["after:2026-10-01 before:2026/10/16", [], ["after:2026-10-01", "before:2026-10-16"]],
  ["received:2026-10-01..2026-10-15", [], ["received:2026-10-01..2026-10-15"]],
  ["received:2026-10-01..", [], ["received:2026-10-01.."]],
  ["received:..2026-1-5", [], ["received:..2026-01-05"]],
  ['received:"This  Week" lunch', ["A:lunch"], ["received:this week"]],
  ['received:"last month"', [], ["received:last month"]],
  ["received:today received:yesterday", [], ["received:today", "received:yesterday"]],
  ["received:tomorrow", [], []],
  ["after:2026-02-30", [], []],
  ["after:2026-10-01..2026-10-02", [], []],
  ["received:2026-10-01..yesterday", [], []],
  ["received:2026-10-15..2026-10-01", [], ["received:2026-10-15..2026-10-01"]],
  ["before:yesterday", [], ["before:yesterday"]],
  ["in:", [], []],
  ["in: inbox", ["A:inbox"], []],
  ["received:", [], []],
  ["isunread is", ["A:isunread", "A:is"], []],
  [
    "has:attachment is:flagged in:sent received:yesterday account:work is:read",
    [],
    ["has:attachment", "is:read", "is:flagged", "received:yesterday", "in:sent", "account:work"],
  ],
  ['from:megan is:unread "q3 plan"', ["From:megan", "A:q3 plan [p]"], ["is:unread"]],
  ["ACCOUNT:x in:Inbox", [], ["in:inbox", "account:x"]],
];

describe("parseSearch", () => {
  it.each(VECTORS)("parses %j like the core", (input, expectedTerms, expectedFilters) => {
    const query = parseSearch(input);
    expect(terms(query)).toEqual(expectedTerms);
    expect(filters(query)).toEqual(expectedFilters);
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

  it("keeps eight values of each filter, and filters apart from terms", () => {
    const many = (op: string, value: (i: number) => string) =>
      Array.from({ length: 9 }, (_, i) => `${op}:${value(i)}`).join(" ");
    const folders = parseSearch(many("in", (i) => `f${i}`)).folders;
    expect(folders).toHaveLength(MAX_FILTER_VALUES);
    expect(folders[7]).toBe("f7");
    expect(parseSearch(many("account", (i) => `a${i}`)).accounts).toHaveLength(MAX_FILTER_VALUES);
    expect(parseSearch(many("after", (i) => `2026-10-0${i + 1}`)).dates).toHaveLength(
      MAX_FILTER_VALUES,
    );
    const words = Array.from({ length: 20 }, (_, i) => `w${i}`).join(" ");
    const query = parseSearch(`${words} in:inbox`);
    expect(query.terms).toHaveLength(16);
    expect(query.folders).toEqual(["inbox"]);
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

  it("reads names of the object's own properties as words, not operators", () => {
    expect(terms(parseSearch("constructor:x toString:y"))).toEqual([
      "A:constructor",
      "A:x",
      "A:tostring",
      "A:y",
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

describe("what a query asks for", () => {
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

  // The shared narrowness vectors.
  it.each([
    "is:unread",
    "is:flagged",
    "received:today",
    "after:2020-01-01",
    "before:2010-01-01",
    "has:attachment in:inbox",
  ])("finds %j narrow enough for the servers", (input) => {
    expect(isNarrowEnoughForServers(parseSearch(input))).toBe(true);
  });
  it.each(["is:read", "in:inbox", "account:work", "is:read in:inbox account:work", "a is:read"])(
    "finds %j too broad for the servers",
    (input) => {
      expect(isNarrowEnoughForServers(parseSearch(input))).toBe(false);
    },
  );

  // The shared emptiness vectors.
  it.each([
    "",
    "to:",
    "has:pdf",
    "in:inbox",
    "account:x",
    "in:inbox account:x",
    "is:starred",
    "received:tomorrow",
  ])("finds nothing to look for in %j", (input) => {
    expect(isEmptySearch(parseSearch(input))).toBe(true);
  });
  it.each(["is:read", "received:today", "is:flagged", "has:attachment"])(
    "finds something to look for in %j",
    (input) => {
      expect(isEmptySearch(parseSearch(input))).toBe(false);
    },
  );

  // The shared ranking vectors.
  it.each(["invoice", "subject:q3", "body:pineapple", "from:megan invoice", '"q3 plan" is:unread'])(
    "ranks %j",
    (input) => {
      expect(ranks(parseSearch(input))).toBe(true);
    },
  );
  it.each([
    "from:megan",
    "to:al cc:dana",
    "from:megan@contoso.com is:unread",
    "is:flagged",
    "in:inbox",
    "has:attachment received:today",
  ])("lists %j newest first", (input) => {
    expect(ranks(parseSearch(input))).toBe(false);
  });
});

describe("dates", () => {
  it("reads days, ranges and words", () => {
    expect(parseDateClause("received", "2026-10-01")).toEqual({
      op: "received",
      spec: { kind: "day", day: "2026-10-01" },
    });
    expect(parseDateClause("after", "2026/10/16")?.spec).toEqual({
      kind: "day",
      day: "2026-10-16",
    });
    expect(parseDateClause("received", "2026/10/01..")?.spec).toEqual({
      kind: "range",
      first: "2026-10-01",
      last: null,
    });
    expect(parseDateClause("received", "  This \t WEEK ")?.spec).toEqual({
      kind: "keyword",
      keyword: "this week",
    });
    expect(parseDateClause("before", "Yesterday")?.spec).toEqual({
      kind: "keyword",
      keyword: "yesterday",
    });
    expect(parseDateClause("received", "2024-02-29")?.spec).toEqual({
      kind: "day",
      day: "2024-02-29",
    });
  });

  it.each([
    ["received", "tomorrow"],
    ["received", "2026-02-30"],
    ["received", "2025-02-29"],
    ["received", "2026-13-01"],
    ["received", ".."],
    ["received", "2026-10-01..yesterday"],
    ["received", "today..2026-10-01"],
    ["received", "2026-10-01..2026-10-02..2026-10-03"],
    ["after", "2026-10-01..2026-10-02"],
    ["before", "..2026-10-02"],
    ["received", "26-10-01"],
    ["received", "02026-10-01"],
    ["received", "2026-001-01"],
    ["received", "2026-10/01"],
    ["received", "2026-10"],
    ["received", "2026-10-01-02"],
    ["received", "2026-+1-01"],
    ["received", "２０２６-10-01"],
    ["received", "thisweek"],
    ["received", ""],
  ] as const)("finds no date in %s:%j", (op, value) => {
    expect(parseDateClause(op, value)).toBeNull();
  });

  // A Sunday morning, as the core's tests resolve them (in this test run's zone).
  const now = new Date(2026, 9, 18, 10, 0);
  const day = (y: number, m: number, d: number) => new Date(y, m - 1, d);
  const resolve = (text: string, weekStartsOn = 0, at = now) =>
    resolveDates(parseSearch(text), at, weekStartsOn);

  afterEach(() => vi.useRealTimers());

  it("resolves words to days here, with the week starting where it does", () => {
    expect(resolve("received:today")).toEqual({
      since: day(2026, 10, 18),
      before: day(2026, 10, 19),
    });
    expect(resolve("received:yesterday")).toEqual({
      since: day(2026, 10, 17),
      before: day(2026, 10, 18),
    });
    expect(resolve('received:"this week"')).toEqual({
      since: day(2026, 10, 18),
      before: day(2026, 10, 25),
    });
    expect(resolve('received:"this week"', 1)).toEqual({
      since: day(2026, 10, 12),
      before: day(2026, 10, 19),
    });
    expect(resolve('received:"last week"', 1)).toEqual({
      since: day(2026, 10, 5),
      before: day(2026, 10, 12),
    });
    expect(resolve('received:"this month"')).toEqual({
      since: day(2026, 10, 1),
      before: day(2026, 11, 1),
    });
    // Across the turn of the year.
    expect(resolve('received:"last month"', 0, new Date(2026, 0, 15, 9))).toEqual({
      since: day(2025, 12, 1),
      before: day(2026, 1, 1),
    });
    expect(resolve('received:"this year"')).toEqual({
      since: day(2026, 1, 1),
      before: day(2027, 1, 1),
    });
    expect(resolve('received:"last year"')).toEqual({
      since: day(2025, 1, 1),
      before: day(2026, 1, 1),
    });
  });

  it("takes after: from its day on, before: up to it, and what every clause leaves", () => {
    expect(resolve("after:2026-10-01")).toEqual({ since: day(2026, 10, 1), before: null });
    expect(resolve("before:2026-10-01")).toEqual({ since: null, before: day(2026, 10, 1) });
    expect(resolve("received:2026-10-01..2026-10-15")).toEqual({
      since: day(2026, 10, 1),
      before: day(2026, 10, 16),
    });
    expect(resolve("received:..2026-10-15")).toEqual({ since: null, before: day(2026, 10, 16) });
    expect(resolve("after:2026-10-01 before:2026-10-16 received:2026-10-10..")).toEqual({
      since: day(2026, 10, 10),
      before: day(2026, 10, 16),
    });
    expect(isImpossible(resolve("received:2026-10-15..2026-10-01"))).toBe(true);
    expect(isImpossible(resolve("received:today received:yesterday"))).toBe(true);
    expect(isImpossible(resolve("after:2026-10-01"))).toBe(false);
    expect(resolve("invoice")).toEqual({ since: null, before: null });
  });

  // As the core's a_skipped_midnight_starts_the_day_after_the_gap: Chile's
  // clocks go from 0:00 to 1:00 on 6 September 2026 (UTC−4 to UTC−3).
  it("starts a day whose midnight the clocks skip at 1:00, and the days around it at 0:00", () => {
    const zone = process.env.TZ;
    process.env.TZ = "America/Santiago";
    try {
      const at = new Date(2026, 8, 6, 10);
      const sat = new Date("2026-09-05T04:00:00Z"); // 0:00 at UTC−4
      const sun = new Date("2026-09-06T04:00:00Z"); // 1:00 at UTC−3
      const mon = new Date("2026-09-07T03:00:00Z"); // 0:00 at UTC−3
      expect(resolve("received:yesterday", 0, at)).toEqual({ since: sat, before: sun });
      expect(resolve("received:today", 0, at)).toEqual({ since: sun, before: mon });
      expect(resolve("received:2026-09-05", 0, at)).toEqual({ since: sat, before: sun });
      expect(resolve("received:..2026-09-05", 0, at)).toEqual({ since: null, before: sun });
      expect(resolve('received:"this week"', 6, at)).toEqual({
        since: sat,
        before: new Date("2026-09-12T03:00:00Z"),
      });
    } finally {
      if (zone === undefined) delete process.env.TZ;
      else process.env.TZ = zone;
    }
  });
});

describe("folders and accounts", () => {
  // The shared scope-key vectors.
  it.each([
    ["Project  X", "project x"],
    [" Clients", "clients"],
    ["Q1\\Q2 plans", "q1q2 plans"],
    ['Say "Hi"', "say hi"],
    ["Say “Hi”", "say hi"],
    ["ÜBERWEISUNGEN", "überweisungen"],
    ["U\u0308", "ü"],
    ["a\tb\u0007c\u0000", "a bc"],
    ["„x‟ ″y″", "x y"],
    ["Bob’s", "bob's"],
  ])("keys %j as %j", (name, key) => {
    expect(scopeKey(name)).toBe(key);
    expect(scopeKey(key)).toBe(key);
  });

  const folder = (name: string, role: Parameters<typeof folderMatches>[1]["role"] = "other") => ({
    name,
    role,
  });

  it("names folders by their role or their name, not their subfolders'", () => {
    expect(folderMatches("inbox", folder("Posteingang", "inbox"))).toBe(true);
    expect(folderMatches("posteingang", folder("Posteingang", "inbox"))).toBe(true);
    expect(folderMatches("spam", folder("Bulk Mail", "junk"))).toBe(true);
    expect(folderMatches("junk", folder("Bulk Mail", "junk"))).toBe(true);
    expect(folderMatches("deleted", folder("Deleted Items", "trash"))).toBe(true);
    expect(folderMatches("deleted items", folder("Deleted Items", "trash"))).toBe(true);
    expect(folderMatches("draft", folder("Entwürfe", "drafts"))).toBe(true);
    expect(folderMatches("ÜBERWEISUNGEN", folder("Überweisungen"))).toBe(true);
    expect(folderMatches("inbox", folder("Archive", "archive"))).toBe(false);
    expect(folderMatches("projects", folder("Atlas"))).toBe(false);
  });

  it("names the folder a filter wrote, whatever its name", () => {
    for (const name of ["Project  X", " Clients", "Q1\\Q2 plans", 'Say "Hi"', "Überweisungen"]) {
      // As the Folder filter writes it (formatValue in edit.ts).
      const bare = /^[^\s"“”„‟″]+$/u.test(name);
      const written = bare ? name : `"${name.replace(/["“”„‟″\\]/g, "")}"`;
      const [value] = parseSearch(`in:${written}`).folders;
      expect(folderMatches(value!, folder(name)), name).toBe(true);
    }
  });

  it("names accounts by their address or their name", () => {
    const account = { email: "Megan@Contoso.com", displayName: "Work  Mail" };
    expect(accountMatches("megan@contoso.com", account)).toBe(true);
    expect(accountMatches("work mail", account)).toBe(true);
    expect(accountMatches("work", account)).toBe(false);
  });
});
