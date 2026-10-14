import { describe, expect, it } from "vitest";

import {
  combineDateTime,
  defaultNewEventTimes,
  eventRange,
  fromAllDayIso,
  htmlToText,
  isAllDayLike,
  moveStart,
  setAllDay,
  textToHtml,
  timesFromEvent,
  timesFromRange,
  timesToIso,
  toAllDayIso,
  validateTimes,
} from "./dates";
import { withTimeZone } from "./testing";

const ZONES = ["UTC", "America/Los_Angeles", "Europe/London", "Asia/Kolkata", "Pacific/Auckland"];

describe("all-day representation", () => {
  it.each(ZONES)("stores local days as midnight UTC in %s", (tz) => {
    withTimeZone(tz, () => {
      const day = new Date(2026, 9, 22); // local midnight, 22 October
      expect(toAllDayIso(day)).toBe("2026-10-22T00:00:00.000Z");
      // Any time on that local day maps to the same stored value.
      expect(toAllDayIso(new Date(2026, 9, 22, 23, 59))).toBe("2026-10-22T00:00:00.000Z");
    });
  });

  it.each(ZONES)("reads stored all-day boundaries as local days in %s", (tz) => {
    withTimeZone(tz, () => {
      const day = fromAllDayIso("2026-10-22T00:00:00.000Z");
      expect([day.getFullYear(), day.getMonth(), day.getDate(), day.getHours()]).toEqual([
        2026, 9, 22, 0,
      ]);
    });
  });

  it.each(ZONES)("spans whole local days for a multi-day event in %s", (tz) => {
    withTimeZone(tz, () => {
      const range = eventRange({
        start: "2026-10-22T00:00:00.000Z",
        end: "2026-10-26T00:00:00.000Z",
        isAllDay: true,
      });
      expect(range.start).toEqual(new Date(2026, 9, 22));
      expect(range.end).toEqual(new Date(2026, 9, 26));
    });
  });

  it("keeps whole days across a daylight-saving change", () => {
    withTimeZone("Europe/London", () => {
      // Clocks go back on October 25, 2026: the local day is 25 hours long.
      const times = timesFromRange(new Date(2026, 9, 25), new Date(2026, 9, 26), true);
      expect(times).toMatchObject({ startDate: "2026-10-25", endDate: "2026-10-25" });
      expect(timesToIso(times)).toEqual({
        start: "2026-10-25T00:00:00.000Z",
        end: "2026-10-26T00:00:00.000Z",
      });
    });
  });

  it("round-trips through the form values", () => {
    withTimeZone("America/Los_Angeles", () => {
      const event = {
        start: "2026-10-22T00:00:00.000Z",
        end: "2026-10-26T00:00:00.000Z",
        isAllDay: true,
      };
      const times = timesFromEvent(event);
      // The form shows the inclusive last day.
      expect(times).toMatchObject({
        allDay: true,
        startDate: "2026-10-22",
        endDate: "2026-10-25",
      });
      expect(timesToIso(times)).toEqual({ start: event.start, end: event.end });
    });
  });

  it("puts 24-hour timed events in the all-day lane", () => {
    expect(
      isAllDayLike({ start: "2026-10-14T09:00:00Z", end: "2026-10-15T09:00:00Z", isAllDay: false }),
    ).toBe(true);
    expect(
      isAllDayLike({ start: "2026-10-14T22:00:00Z", end: "2026-10-15T01:00:00Z", isAllDay: false }),
    ).toBe(false);
  });
});

describe("form times", () => {
  it("converts timed values in the local time zone", () => {
    withTimeZone("America/New_York", () => {
      const times = timesFromRange(new Date(2026, 9, 14, 10), new Date(2026, 9, 14, 10, 30));
      expect(times).toEqual({
        allDay: false,
        startDate: "2026-10-14",
        startTime: "10:00",
        endDate: "2026-10-14",
        endTime: "10:30",
      });
      // 10:00 EDT is 14:00 UTC.
      expect(timesToIso(times)).toEqual({
        start: "2026-10-14T14:00:00.000Z",
        end: "2026-10-14T14:30:00.000Z",
      });
    });
  });

  it("switches to all day and back keeping the days", () => {
    const timed = timesFromRange(new Date(2026, 9, 14, 22), new Date(2026, 9, 15, 0));
    // Ending at midnight does not occupy the next day.
    const allDay = setAllDay(timed, true);
    expect(allDay).toMatchObject({ allDay: true, startDate: "2026-10-14", endDate: "2026-10-14" });
    expect(timesToIso(allDay)).toEqual({
      start: "2026-10-14T00:00:00.000Z",
      end: "2026-10-15T00:00:00.000Z",
    });

    const back = setAllDay(allDay, false);
    expect(back.allDay).toBe(false);
    expect(validateTimes(back)).toBeNull();

    const multi = setAllDay(
      timesFromRange(new Date(2026, 9, 14, 9), new Date(2026, 9, 16, 17)),
      true,
    );
    expect(multi).toMatchObject({ startDate: "2026-10-14", endDate: "2026-10-16" });
  });

  it("keeps the duration when the start moves", () => {
    const t = timesFromRange(new Date(2026, 9, 14, 10), new Date(2026, 9, 14, 11, 30));
    expect(moveStart(t, "2026-10-14", "23:00")).toMatchObject({
      endDate: "2026-10-15",
      endTime: "00:30",
    });
    const allDay = timesFromRange(new Date(2026, 9, 22), new Date(2026, 9, 26), true);
    expect(moveStart(allDay, "2026-10-29", "09:00")).toMatchObject({
      startDate: "2026-10-29",
      endDate: "2026-11-01",
    });
  });

  it("validates the end after the start", () => {
    const t = timesFromRange(new Date(2026, 9, 14, 10), new Date(2026, 9, 14, 11));
    expect(validateTimes(t)).toBeNull();
    expect(validateTimes({ ...t, endTime: "10:00" })).toBe("The end must be after the start.");
    expect(validateTimes({ ...t, endDate: "2026-10-13" })).toBe("The end must be after the start.");
    expect(validateTimes({ ...t, startDate: "2026-02-30" })).toBe("Enter a valid start and end.");
    expect(validateTimes({ ...t, allDay: true, endDate: "2026-10-14" })).toBeNull();
  });

  it("builds instants from wall-clock times across DST gaps", () => {
    withTimeZone("Europe/London", () => {
      // 1:30 AM does not exist on March 29, 2026; JavaScript moves it forward.
      const d = combineDateTime("2026-03-29", "01:30")!;
      expect(d.getHours()).toBe(2);
      expect(combineDateTime("2026-03-29", "25:00")).toBeNull();
    });
  });

  it("suggests the next half hour today and 09:00 on other days", () => {
    const now = new Date(2026, 9, 14, 12, 10);
    expect(defaultNewEventTimes(new Date(2026, 9, 14), now)).toMatchObject({
      startDate: "2026-10-14",
      startTime: "12:30",
      endTime: "13:00",
    });
    expect(defaultNewEventTimes(new Date(2026, 9, 20), now)).toMatchObject({
      startDate: "2026-10-20",
      startTime: "09:00",
      endTime: "09:30",
    });
  });
});

describe("descriptions", () => {
  it("escapes plain text into paragraphs", () => {
    expect(textToHtml("Agenda <b>\n- one\n\nThanks & bye")).toBe(
      "<p>Agenda &lt;b&gt;<br>- one</p><p>Thanks &amp; bye</p>",
    );
  });

  it("reads HTML back as text", () => {
    expect(
      htmlToText("<p>Hello <b>team</b></p><ul><li>One</li><li>Two</li></ul><p>Bye<br>A</p>"),
    ).toBe("Hello team\n\n• One\n• Two\nBye\nA");
    expect(htmlToText("<script>alert(1)</script><p>Safe</p>")).toBe("Safe");
  });
});
