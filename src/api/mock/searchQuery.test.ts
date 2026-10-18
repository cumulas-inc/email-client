import { describe, expect, it } from "vitest";

import { hitPatterns } from "../../lib/search/highlight";
import { parseSearch, resolveDates } from "../../lib/search/query";
import type { Message } from "../types";
import { createDemoState } from "./data";
import {
  INFIX_RELEVANCE,
  SERVER_RELEVANCE,
  admits,
  matchesSearch,
  mockSnippet,
  rankHits,
  score,
} from "./searchQuery";

// The parser and its shared test vectors are src/lib/search/query.ts's.

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

describe("admits", () => {
  const messages = createDemoState().messages;
  const checklist = messages.find((m) => /Atlas launch checklist/.test(m.subject))!;
  const open = { since: null, before: null };
  const passes = (text: string, m: Message = checklist) =>
    admits(parseSearch(text), m, resolveDates(parseSearch(text), new Date(), 0), null);

  it("checks every filter but the words, as the core does", () => {
    // Unread, flagged, with an attachment, today.
    expect(passes("nothing like it")).toBe(true);
    expect(passes("is:unread is:flagged has:attachment received:today")).toBe(true);
    expect(passes("is:read")).toBe(false);
    expect(passes("received:yesterday")).toBe(false);
    const read = { ...checklist, isRead: true, isFlagged: false, hasAttachments: false };
    expect(passes("is:read", read)).toBe(true);
    expect(passes("is:flagged", read)).toBe(false);
    expect(passes("has:attachment", read)).toBe(false);
    // Folders by local id, when given.
    const query = parseSearch("x");
    expect(admits(query, checklist, open, new Set(["w-inbox"]))).toBe(true);
    expect(admits(query, checklist, open, new Set(["w-archive"]))).toBe(false);
    expect(admits(query, checklist, open, new Set())).toBe(false);
  });
});

describe("rankHits", () => {
  const now = new Date("2026-10-18T10:00:00Z");
  const message = (id: string, subject: string, daysOld: number, preview = ""): Message => ({
    ...createDemoState().messages[0]!,
    id,
    subject,
    preview,
    bodyText: null,
    from: { name: "Sam", address: "sam@example.org" },
    to: [],
    cc: [],
    receivedAt: new Date(now.getTime() - daysOld * 86_400_000).toISOString(),
  });
  const ids = (hits: { message: Message }[]) => hits.map((h) => h.message.id);

  it("puts words in the subject or sender first, then recent mail", () => {
    const query = parseSearch("porto");
    const p1 = { message: message("p1", "Porto offsite: tickets", 2), fromServer: false };
    const p2 = { message: message("p2", "Porto itinerary", 200), fromServer: false };
    const p3 = {
      message: message("p3", "Friday plans", 1, "we fly to porto on Friday"),
      fromServer: false,
    };
    expect(ids(rankHits([p3, p2, p1], query, now))).toEqual(["p1", "p2", "p3"]);
    expect(score(query, p1, now)).toBeCloseTo(0.973, 3);
    expect(score(query, p2, now)).toBeCloseTo(0.406, 3);
    expect(score(query, p3, now)).toBeCloseTo(0.296, 3);
  });

  it("scores matches inside words and server-only hits as the core does", () => {
    const query = parseSearch("ortof");
    const inside = { message: message("i", "Portoffice move", 0), fromServer: false };
    const server = { message: message("s", "Nothing here", 0), fromServer: true };
    expect(score(query, inside, now)).toBeCloseTo(INFIX_RELEVANCE, 6);
    expect(score(query, server, now)).toBeCloseTo(SERVER_RELEVANCE, 6);
    // Equal scores: newest first, then by id.
    const a = { message: message("a", "x", 3), fromServer: true };
    const b = { message: message("b", "x", 3), fromServer: true };
    const c = { message: message("c", "x", 1), fromServer: true };
    expect(ids(rankHits([b, a, c], parseSearch("zzz"), now))).toEqual(["c", "a", "b"]);
  });

  it("counts subject: terms as in the subject and body: terms as elsewhere", () => {
    const m = { message: message("m", "Plans", 0, "porto"), fromServer: false };
    expect(score(parseSearch("subject:plans"), m, now)).toBeCloseTo(1, 6);
    expect(score(parseSearch("body:porto"), m, now)).toBeCloseTo(0.3, 6);
    expect(score(parseSearch("subject:plans body:porto"), m, now)).toBeCloseTo(0.65, 6);
  });
});

describe("mockSnippet", () => {
  const body =
    "Hi Alex, the plan for the week is set: we leave early on Monday, then we could get " +
    "pineapple pizza on Friday at the harbor, and come back on the late train.";
  const opened = { ...createDemoState().messages[0]!, bodyText: body };

  it("shows the words around the first hit in the body", () => {
    const snippet = mockSnippet(opened, hitPatterns(parseSearch("pineapple")))!;
    expect(snippet.filter((r) => r.hit).map((r) => r.text)).toEqual(["pineapple"]);
    const text = snippet.map((r) => r.text).join("");
    expect(text.startsWith("…")).toBe(true);
    expect(text).toContain("then we could get pineapple pizza");
    expect(text.length).toBeLessThanOrEqual(162);
  });

  it("is null without a body or a hit in it", () => {
    expect(
      mockSnippet({ ...opened, bodyText: null }, hitPatterns(parseSearch("pizza"))),
    ).toBeNull();
    expect(mockSnippet(opened, hitPatterns(parseSearch("zebra")))).toBeNull();
    expect(mockSnippet(opened, hitPatterns(parseSearch("subject:pizza")))).toBeNull();
    expect(mockSnippet(opened, null)).toBeNull();
  });
});
