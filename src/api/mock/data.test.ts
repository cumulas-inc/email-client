import { afterEach, describe, expect, it, vi } from "vitest";

import { TODAYS_MAIL_BY, createDemoState, receivedBy } from "./data";

/** Today at `hours`:`minutes`, local time. */
function today(hours: number, minutes: number): Date {
  const d = new Date();
  d.setHours(hours, minutes, 0, 0);
  return d;
}

afterEach(() => {
  vi.useRealTimers();
});

describe("the demo's mail", () => {
  // Through the day, around the last of today's mail (9:42) and after it.
  for (const [hours, minutes] of [
    [0, 1],
    [6, 15],
    [9, 41],
    [9, 43],
    [TODAYS_MAIL_BY.hours, TODAYS_MAIL_BY.minutes],
    [23, 59],
  ] as const) {
    it(`never arrives in the future, and today's stays today's, at ${hours}:${minutes}`, () => {
      const now = today(hours, minutes);
      vi.useFakeTimers({ toFake: ["Date"] });
      vi.setSystemTime(now);
      const messages = createDemoState().messages;
      const late = messages.filter((m) => Date.parse(m.receivedAt) > now.getTime());
      expect(late.map((m) => m.subject)).toEqual([]);
      const checklist = messages.find((m) => /Atlas launch checklist/.test(m.subject))!;
      expect(new Date(checklist.receivedAt).toDateString()).toBe(now.toDateString());
    });
  }

  it("keeps today's mail in its order early in the day, and its times later", () => {
    const early = today(6, 0);
    const [first, second] = [today(5, 55), today(9, 42)].map((d) => receivedBy(d, early));
    expect(first!.getTime()).toBeLessThan(second!.getTime());
    expect(second!.getTime()).toBeLessThanOrEqual(early.getTime());
    const later = today(14, 0);
    expect(receivedBy(today(9, 42), later)).toEqual(today(9, 42));
    // Earlier days' mail is never moved.
    const yesterday = new Date(today(9, 42).getTime() - 24 * 60 * 60 * 1000);
    expect(receivedBy(yesterday, early)).toEqual(yesterday);
  });
});
