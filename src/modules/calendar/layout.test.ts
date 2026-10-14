import { describe, expect, it } from "vitest";

import type { CalendarEvent } from "../../api/types";
import { toAllDayIso } from "./dates";
import { withTimeZone } from "./testing";
import {
  type TimedSegment,
  cellLanes,
  hiddenCount,
  laneTable,
  layoutDay,
  layoutLanes,
  rangeTitle,
  rowItems,
  shiftAnchor,
  spanForDays,
  timedSegmentsForDay,
  viewRange,
  visibleLanes,
} from "./layout";

function seg(key: string, start: number, end: number): TimedSegment<string> {
  return { item: key, key, start, end, continuesBefore: false, continuesAfter: false };
}

function byKey<T extends { key: string }>(items: T[]): Record<string, T> {
  return Object.fromEntries(items.map((i) => [i.key, i]));
}

function ev(id: string, start: Date | string, end: Date | string, isAllDay = false): CalendarEvent {
  return {
    id,
    accountId: "a",
    calendarId: "c",
    remoteId: id,
    subject: id,
    location: null,
    bodyHtml: null,
    bodyPreview: null,
    start: typeof start === "string" ? start : start.toISOString(),
    end: typeof end === "string" ? end : end.toISOString(),
    isAllDay,
    organizer: null,
    attendees: [],
    showAs: "busy",
    responseStatus: "organizer",
    isOnlineMeeting: false,
    onlineMeetingUrl: null,
    reminderMinutes: null,
    isRecurring: false,
    isCancelled: false,
  };
}

describe("overlap layout", () => {
  it("gives non-overlapping events the full width", () => {
    const placed = byKey(layoutDay([seg("a", 540, 600), seg("b", 600, 660)]));
    expect(placed.a).toMatchObject({ column: 0, columns: 1, span: 1 });
    expect(placed.b).toMatchObject({ column: 0, columns: 1, span: 1 });
  });

  it("puts overlapping events side by side", () => {
    const placed = byKey(layoutDay([seg("a", 540, 630), seg("b", 570, 600)]));
    expect(placed.a).toMatchObject({ column: 0, columns: 2 });
    expect(placed.b).toMatchObject({ column: 1, columns: 2 });
  });

  it("orders events starting together longest first", () => {
    const placed = byKey(layoutDay([seg("short", 600, 630), seg("long", 600, 720)]));
    expect(placed.long!.column).toBe(0);
    expect(placed.short!.column).toBe(1);
  });

  it("reuses a freed column within a chain of overlaps", () => {
    // a overlaps b, b overlaps c, but a and c do not: two columns suffice.
    const placed = byKey(layoutDay([seg("a", 540, 600), seg("b", 570, 660), seg("c", 600, 690)]));
    expect(placed.a).toMatchObject({ column: 0, columns: 2 });
    expect(placed.b).toMatchObject({ column: 1, columns: 2 });
    expect(placed.c).toMatchObject({ column: 0, columns: 2 });
  });

  it("stretches events over free columns to the right", () => {
    // a, b and c overlap at 10:00; d only overlaps a, so it can take two columns.
    const placed = byKey(
      layoutDay([seg("a", 540, 720), seg("b", 600, 630), seg("c", 600, 630), seg("d", 660, 700)]),
    );
    expect(placed.a).toMatchObject({ column: 0, columns: 3, span: 1 });
    expect(placed.b).toMatchObject({ column: 1, columns: 3, span: 1 });
    expect(placed.c).toMatchObject({ column: 2, columns: 3, span: 1 });
    expect(placed.d).toMatchObject({ column: 1, columns: 3, span: 2 });
  });

  it("starts a new cluster once the previous one has ended", () => {
    const placed = byKey(layoutDay([seg("a", 540, 600), seg("b", 570, 600), seg("c", 600, 630)]));
    expect(placed.a!.columns).toBe(2);
    expect(placed.c).toMatchObject({ column: 0, columns: 1 });
  });

  it("treats very short events as tall enough for their title", () => {
    // Back-to-back 5-minute events would overlap visually at a 20-minute minimum.
    const placed = byKey(layoutDay([seg("a", 600, 605), seg("b", 605, 610)], 20));
    expect(placed.a!.columns).toBe(2);
    expect(byKey(layoutDay([seg("a", 600, 605), seg("b", 605, 610)])).a!.columns).toBe(1);
  });
});

describe("timed segments", () => {
  const day = new Date(2026, 9, 14);

  it("splits events that cross midnight", () => {
    const e = ev("late", new Date(2026, 9, 14, 22, 0), new Date(2026, 9, 15, 1, 30));
    expect(timedSegmentsForDay([e], day)).toEqual([
      expect.objectContaining({ start: 22 * 60, end: 24 * 60, continuesAfter: true }),
    ]);
    expect(timedSegmentsForDay([e], new Date(2026, 9, 15))).toEqual([
      expect.objectContaining({ start: 0, end: 90, continuesBefore: true }),
    ]);
  });

  it("leaves all-day and 24-hour events to the all-day lane", () => {
    const allDay = ev("ad", toAllDayIso(day), toAllDayIso(new Date(2026, 9, 15)), true);
    const long = ev("long", new Date(2026, 9, 14, 9), new Date(2026, 9, 15, 9));
    expect(timedSegmentsForDay([allDay, long], day)).toEqual([]);
  });

  it("keeps zero-length events on their day only", () => {
    const at = new Date(2026, 9, 14, 0, 0);
    const e = ev("ping", at, at);
    expect(timedSegmentsForDay([e], day)).toHaveLength(1);
    expect(timedSegmentsForDay([e], new Date(2026, 9, 13))).toHaveLength(0);
  });
});

describe("view ranges", () => {
  const wednesday = new Date(2026, 9, 14);

  it("computes the visible days of each view", () => {
    expect(viewRange("day", new Date(2026, 9, 14, 15, 30)).days).toEqual([wednesday]);

    const work = viewRange("workweek", wednesday);
    expect(work.days.map((d) => d.getDate())).toEqual([12, 13, 14, 15, 16]);
    expect(work.end).toEqual(new Date(2026, 9, 17));

    const week = viewRange("week", wednesday);
    expect(week.days.map((d) => d.getDate())).toEqual([12, 13, 14, 15, 16, 17, 18]);

    const month = viewRange("month", wednesday);
    expect(month.days).toHaveLength(42);
    expect(month.start).toEqual(new Date(2026, 8, 28)); // Monday before 1 October
    expect(month.end).toEqual(new Date(2026, 10, 9));
  });

  it("shows Monday to Friday of the same week on weekends", () => {
    const sunday = new Date(2026, 9, 18);
    expect(viewRange("workweek", sunday).days.map((d) => d.getDate())).toEqual([
      12, 13, 14, 15, 16,
    ]);
  });

  it("moves by a day, a week or a month", () => {
    expect(shiftAnchor("day", wednesday, 1)).toEqual(new Date(2026, 9, 15));
    expect(shiftAnchor("workweek", wednesday, -1)).toEqual(new Date(2026, 9, 7));
    expect(shiftAnchor("week", wednesday, 1)).toEqual(new Date(2026, 9, 21));
    expect(shiftAnchor("month", new Date(2026, 0, 31), 1)).toEqual(new Date(2026, 1, 28));
  });

  it("titles ranges like Outlook", () => {
    expect(rangeTitle("day", wednesday)).toBe("Wednesday, 14 October 2026");
    expect(rangeTitle("workweek", wednesday)).toBe("12–16 October 2026");
    expect(rangeTitle("week", wednesday)).toBe("12–18 October 2026");
    expect(rangeTitle("month", wednesday)).toBe("October 2026");
    expect(rangeTitle("week", new Date(2026, 9, 1))).toBe("28 September – 4 October 2026");
    expect(rangeTitle("week", new Date(2026, 11, 31))).toBe("28 December 2026 – 3 January 2027");
  });

  it("keeps whole days across daylight-saving changes", () => {
    withTimeZone("Europe/Berlin", () => {
      // Clocks go back on Sunday 25 October 2026 in Europe.
      const days = viewRange("week", new Date(2026, 9, 22)).days;
      expect(days.map((d) => [d.getDate(), d.getHours()])).toEqual(
        [19, 20, 21, 22, 23, 24, 25].map((d) => [d, 0]),
      );
    });
  });
});

describe("lanes for multi-day bars", () => {
  const week = Array.from({ length: 7 }, (_, i) => new Date(2026, 9, 12 + i));

  it("finds the columns an event covers", () => {
    const range = { start: new Date(2026, 9, 10), end: new Date(2026, 9, 14) };
    expect(spanForDays(range, week)).toEqual({
      startCol: 0,
      endCol: 1,
      continuesBefore: true,
      continuesAfter: false,
    });
    expect(spanForDays({ start: new Date(2026, 9, 20), end: new Date(2026, 9, 21) }, week)).toBe(
      null,
    );
  });

  it("assigns the lowest free lane", () => {
    const placed = layoutLanes([
      {
        item: "a",
        key: "a",
        startCol: 0,
        endCol: 2,
        continuesBefore: false,
        continuesAfter: false,
      },
      {
        item: "b",
        key: "b",
        startCol: 1,
        endCol: 3,
        continuesBefore: false,
        continuesAfter: false,
      },
      {
        item: "c",
        key: "c",
        startCol: 3,
        endCol: 4,
        continuesBefore: false,
        continuesAfter: false,
      },
      {
        item: "d",
        key: "d",
        startCol: 4,
        endCol: 4,
        continuesBefore: false,
        continuesAfter: false,
      },
    ]);
    expect(placed.map((p) => [p.key, p.lane])).toEqual([
      ["a", 0],
      ["b", 1],
      ["c", 0],
      ["d", 1],
    ]);
  });

  it("puts bars above single timed events and hides what does not fit", () => {
    const events = [
      ev("trip", toAllDayIso(week[1]!), toAllDayIso(week[4]!), true), // Tue–Thu
      ev("t1", new Date(2026, 9, 14, 9), new Date(2026, 9, 14, 10)),
      ev("t2", new Date(2026, 9, 14, 11), new Date(2026, 9, 14, 12)),
      ev("t3", new Date(2026, 9, 14, 13), new Date(2026, 9, 14, 14)),
    ];
    const placed = rowItems(events, week, { includeTimed: true });
    expect(byKey(placed).trip).toMatchObject({ lane: 0, startCol: 1, endCol: 3 });
    expect(byKey(placed).t1!.lane).toBe(1);

    const table = laneTable(placed, 7);
    // Wednesday has 4 items; with 3 lines only 2 show plus "+2 more".
    const visible = (col: number) => visibleLanes(table[col]!.length, 3);
    expect(visible(2)).toBe(2);
    expect(hiddenCount(table, 2, visible(2))).toBe(2);
    expect(hiddenCount(table, 1, visible(1))).toBe(0);

    // The bar is drawn once, from Tuesday, over three columns.
    const tuesday = cellLanes(table, 1, visible);
    expect(tuesday[0]!.piece).toMatchObject({ span: 3, joinsBefore: false, joinsAfter: false });
    expect(cellLanes(table, 2, visible)[0]!.piece).toBeNull();
  });
});
