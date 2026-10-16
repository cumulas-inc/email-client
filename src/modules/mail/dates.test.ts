import { describe, expect, it } from "vitest";

import { FUTURE_GROUP, fullDate, listDate, listGroup, messageDate, NO_DATE_GROUP } from "./dates";

const NOW = new Date(2026, 9, 14, 12, 0);

describe("messageDate", () => {
  it("treats a missing, invalid or epoch date as no date", () => {
    expect(messageDate(null)).toBeNull();
    expect(messageDate("")).toBeNull();
    expect(messageDate("not a date")).toBeNull();
    expect(messageDate("1970-01-01T00:00:00Z")).toBeNull();
    expect(messageDate("1970-01-01T05:30:00+05:30")).toBeNull();
  });

  it("keeps real dates, old ones too", () => {
    expect(messageDate("1999-12-31T23:59:00Z")).toBeInstanceOf(Date);
    expect(messageDate("2026-10-14T09:42:00Z")).toBeInstanceOf(Date);
  });
});

describe("listDate and listGroup", () => {
  it("leave the date out and group under “No date” for epoch mail", () => {
    expect(listDate("1970-01-01T00:00:00Z", NOW)).toBe("");
    expect(listGroup("1970-01-01T00:00:00Z", NOW)).toBe(NO_DATE_GROUP);
    expect(fullDate("1970-01-01T00:00:00Z")).toBeNull();
  });

  it("show the year for mail dated after today, and group it apart", () => {
    const iso = new Date(2038, 0, 19, 3, 14).toISOString();
    expect(listDate(iso, NOW)).toMatch(/2038/);
    expect(listGroup(iso, NOW)).toBe(FUTURE_GROUP);
  });

  it("treat later today as today", () => {
    const iso = new Date(2026, 9, 14, 23, 0).toISOString();
    expect(listGroup(iso, NOW)).toBe("Today");
  });
});
