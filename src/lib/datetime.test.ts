import { afterEach, describe, expect, it } from "vitest";

import {
  formatDate,
  formatDateRange,
  formatDateTime,
  formatHour,
  formatTime,
  formatTimeSpan,
  setLocaleForTesting,
  uses24HourClock,
  weekStartsOn,
} from "./datetime";

const wed = new Date(2026, 9, 14, 15, 5);

afterEach(() => setLocaleForTesting("en-US"));

describe("American English (default)", () => {
  it("formats dates the American way", () => {
    expect(formatDate(wed, "short")).toBe("Oct 14");
    expect(formatDate(wed, "shortYear")).toBe("Oct 14, 2026");
    expect(formatDate(wed, "longYear")).toBe("October 14, 2026");
    expect(formatDate(wed, "weekdayLongYear")).toBe("Wednesday, October 14, 2026");
    expect(formatDate(wed, "monthYear")).toBe("October 2026");
  });

  it("uses a 12-hour clock", () => {
    expect(uses24HourClock()).toBe(false);
    expect(formatTime(wed)).toBe("3:05 PM");
    expect(formatHour(new Date(2026, 9, 14, 8))).toBe("8 AM");
    expect(formatDateTime(wed)).toBe("Wed, Oct 14, 2026, 3:05 PM");
  });

  it("merges ranges", () => {
    expect(formatDateRange(new Date(2026, 9, 12), new Date(2026, 9, 16), "longYear")).toBe(
      "October 12 – 16, 2026",
    );
    expect(formatTimeSpan(new Date(2026, 9, 14, 9), new Date(2026, 9, 14, 10, 30))).toBe(
      "9:00 – 10:30 AM",
    );
    expect(formatTimeSpan(new Date(2026, 9, 14, 21), new Date(2026, 9, 15, 1))).toBe(
      "Oct 14, 9:00 PM – Oct 15, 1:00 AM",
    );
  });

  it("starts weeks on Sunday", () => {
    expect(weekStartsOn()).toBe(0);
  });
});

describe("other locales keep their conventions", () => {
  it("British English", () => {
    setLocaleForTesting("en-GB");
    expect(formatDate(wed, "short")).toBe("14 Oct");
    expect(formatTime(wed)).toBe("15:05");
    expect(formatHour(new Date(2026, 9, 14, 8))).toBe("08:00");
    expect(weekStartsOn()).toBe(1);
  });

  it("German", () => {
    setLocaleForTesting("de-DE");
    expect(formatDate(wed, "longYear")).toBe("14. Oktober 2026");
    expect(uses24HourClock()).toBe(true);
    expect(weekStartsOn()).toBe(1);
  });
});
