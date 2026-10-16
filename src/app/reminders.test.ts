import { beforeAll, describe, expect, it } from "vitest";

import type { ActiveReminder } from "../api/types";
import { setLocaleForTesting } from "../lib/datetime";
import {
  announcement,
  durationText,
  joinUrl,
  reminderDetail,
  reminderTime,
  snoozeChoices,
} from "./reminders";

beforeAll(() => setLocaleForTesting("en-US"));

/** Wednesday, October 14, 2026, 10:24 AM, local time. */
const NOW = new Date(2026, 9, 14, 10, 24);
const at = (day: number, hour: number, minute = 0) =>
  new Date(2026, 9, day, hour, minute).toISOString();

function reminder(extra: Partial<ActiveReminder> = {}): ActiveReminder {
  return {
    key: "event:e1:1",
    kind: "event",
    itemId: "e1",
    accountId: "a1",
    title: "Focus time",
    startsAt: at(14, 10, 30),
    endsAt: at(14, 12),
    location: null,
    onlineMeetingUrl: null,
    fireAt: at(14, 10, 15),
    snoozedUntil: null,
    overdue: false,
    missed: false,
    ...extra,
  };
}

describe("reminder times", () => {
  it("says durations briefly", () => {
    expect(durationText(20_000)).toBe("1 min");
    expect(durationText(6 * 60_000)).toBe("6 min");
    expect(durationText(80 * 60_000)).toBe("1 h 20 min");
    expect(durationText(120 * 60_000)).toBe("2 h");
    expect(durationText(26 * 3_600_000)).toBe("1 day");
    expect(durationText(3 * 86_400_000)).toBe("3 days");
  });

  it("counts down to the start, then up", () => {
    expect(reminderTime(reminder(), NOW)).toEqual({ text: "in 6 min", tone: "soon" });
    const started = reminder({ startsAt: at(14, 10, 12), overdue: true });
    expect(reminderTime(started, NOW)).toEqual({ text: "12 min overdue", tone: "overdue" });
    const now = reminder({ startsAt: new Date(NOW.getTime() - 20_000).toISOString() });
    expect(reminderTime(now, NOW).text).toBe("Now");
    const task = reminder({ kind: "task", startsAt: null, endsAt: null, fireAt: at(14, 10, 12) });
    expect(reminderTime(task, NOW)).toEqual({ text: "12 min overdue", tone: "overdue" });
  });

  it("says when a reminder is snoozed, missed or over", () => {
    expect(reminderTime(reminder({ snoozedUntil: at(14, 10, 45) }), NOW)).toEqual({
      text: "Snoozed until 10:45 AM",
      tone: "snoozed",
    });
    expect(reminderTime(reminder({ snoozedUntil: at(15, 9) }), NOW).text).toBe(
      "Snoozed until tomorrow at 9:00 AM",
    );
    expect(reminderTime(reminder({ missed: true }), NOW)).toEqual({
      text: "Missed",
      tone: "missed",
    });
    expect(reminderTime(reminder({ endsAt: at(14, 10) }), NOW).text).toBe("Ended");
  });

  it("gives all-day events their day", () => {
    const allDay = (day: number) => reminder({ startsAt: at(day, 0), endsAt: at(day + 1, 0) });
    expect(reminderTime(allDay(14), NOW)).toEqual({ text: "Today, all day", tone: "allDay" });
    expect(reminderTime(allDay(15), NOW).text).toBe("Tomorrow, all day");
    expect(reminderTime(allDay(16), NOW).text).toBe("Fri, Oct 16, all day");
  });

  it("adds the time span and the place", () => {
    expect(reminderDetail(reminder({ location: " Room 3.01 " }), NOW)).toBe(
      "10:30 AM – 12:00 PM · Room 3.01",
    );
    expect(reminderDetail(reminder({ startsAt: at(15, 9), endsAt: at(15, 9, 30) }), NOW)).toBe(
      "Thu, Oct 15, 9:00 – 9:30 AM",
    );
    const task = reminder({ kind: "task", startsAt: null, endsAt: null, fireAt: at(14, 10, 12) });
    expect(reminderDetail(task, NOW)).toBe("Task · reminder at 10:12 AM");
    // A task's reminder stays until dismissed: from another day, it says which.
    expect(reminderDetail({ ...task, fireAt: at(13, 23) }, NOW)).toBe(
      "Task · reminder Tue, Oct 13 at 11:00 PM",
    );
    expect(reminderDetail({ ...task, fireAt: at(12, 23), snoozedUntil: at(14, 10, 39) }, NOW)).toBe(
      "Task · reminder Mon, Oct 12 at 11:00 PM",
    );
  });
});

describe("reminder actions", () => {
  it("joins only https meetings", () => {
    expect(joinUrl(reminder({ onlineMeetingUrl: "https://meet.example/abc" }))).toBe(
      "https://meet.example/abc",
    );
    for (const url of [
      "http://meet.example/abc",
      "javascript:alert(1)",
      "meet.example",
      "",
      null,
    ]) {
      expect(joinUrl(reminder({ onlineMeetingUrl: url }))).toBeNull();
    }
  });

  it("snoozes for set times, or until shortly before the start", () => {
    const choices = snoozeChoices(reminder({ startsAt: at(14, 11) }), NOW);
    expect(choices.map((c) => c.label)).toEqual([
      "5 minutes",
      "10 minutes",
      "15 minutes",
      "30 minutes",
      "1 hour",
      "1 day",
      "5 minutes before start",
    ]);
    expect(choices[4]!.snooze).toEqual({ minutes: 60 });
    expect(choices[6]).toEqual({
      label: "5 minutes before start",
      snooze: { until: at(14, 10, 55) },
      disabled: false,
    });
    // Starting in five minutes: five before is now.
    expect(snoozeChoices(reminder({ startsAt: at(14, 10, 29) }), NOW)[6]!.disabled).toBe(true);
    expect(snoozeChoices(reminder({ startsAt: at(14, 10, 31) }), NOW)[6]!.disabled).toBe(false);
    // Tasks have no start.
    const task = reminder({ kind: "task", startsAt: null, endsAt: null });
    expect(snoozeChoices(task, NOW)).toHaveLength(6);
  });

  it("announces reminders as they come due", () => {
    expect(announcement([], NOW)).toBe("");
    expect(announcement([reminder()], NOW)).toBe("Reminder: Focus time, in 6 min.");
    expect(
      announcement(
        [1, 2, 3, 4].map(() => reminder()),
        NOW,
      ),
    ).toBe("4 reminders are due.");
  });
});
