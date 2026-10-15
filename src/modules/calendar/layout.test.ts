import { addDays } from "date-fns";
import { afterEach, describe, expect, it } from "vitest";

import type { CalendarEvent } from "../../api/types";
import { setLocaleForTesting } from "../../lib/datetime";
import { toAllDayIso } from "./dates";
import { withTimeZone } from "./testing";
import {
  type TimedSegment,
  cellLanes,
  fitColumns,
  hiddenItems,
  laneTable,
  layoutDay,
  layoutLanes,
  limitColumns,
  rangeTitle,
  rowItems,
  shiftAnchor,
  spanForDays,
  timedSegmentsForDay,
  viewRange,
  weekLabel,
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

  it("numbers the clusters of a day", () => {
    const placed = byKey(layoutDay([seg("a", 540, 600), seg("b", 570, 600), seg("c", 600, 630)]));
    expect([placed.a!.cluster, placed.b!.cluster, placed.c!.cluster]).toEqual([0, 0, 1]);
  });
});

describe("crowded clusters", () => {
  // Seven meetings at 10:00 (the longest first), then one at 11:30 that
  // takes the third column once it is free.
  const crowd = [
    seg("legal", 600, 720),
    seg("interview", 600, 690),
    seg("budget", 600, 660),
    seg("steering", 600, 660),
    seg("coffee", 600, 645),
    seg("vendor", 600, 630),
    seg("supplier", 600, 630),
    seg("canceled", 690, 720),
    seg("lunch", 750, 780),
  ];

  it("fits as many columns as have room, beside a chip for the rest", () => {
    const fit = { minWidth: 48, shownWidth: 72, chipWidth: 30 };
    // Two events in a 109px column get 54px each: no chip needed.
    expect(fitColumns(109, 2, fit)).toBe(2);
    // Seven don't fit in 204px (a work-week column at 1440px): two are shown,
    // 87px each, beside the chip.
    expect(fitColumns(204, 7, fit)).toBe(2);
    expect(fitColumns(400, 9, fit)).toBe(5);
    // A 75px column shows one event and a chip, however narrow.
    expect(fitColumns(75, 2, fit)).toBe(1);
    expect(fitColumns(20, 9, fit)).toBe(1);
    expect(fitColumns(75, 1, fit)).toBe(1);
  });

  it("hides the columns that don't fit behind +N chips", () => {
    const { placed, overflow } = limitColumns(layoutDay(crowd), () => 3);
    const shown = byKey(placed);
    expect(Object.keys(shown).sort()).toEqual([
      "budget",
      "canceled",
      "interview",
      "legal",
      "lunch",
    ]);
    expect(shown.legal).toMatchObject({ column: 0, columns: 3, overflowed: true });
    expect(shown.budget).toMatchObject({ column: 2, columns: 3, span: 1 });
    // 11:30 takes the Interview's column once it ends, so it stays visible.
    expect(shown.canceled).toMatchObject({ column: 1, columns: 3 });
    // A later cluster that fits is left alone.
    expect(shown.lunch).toMatchObject({ column: 0, columns: 1 });
    expect(shown.lunch!.overflowed).toBeUndefined();
    expect(overflow).toEqual([
      {
        key: "more-steering",
        start: 600,
        end: 660,
        items: ["steering", "coffee", "supplier", "vendor"],
      },
    ]);
  });

  it("keeps hidden events that don't overlap in separate chips", () => {
    // a runs all morning; b and c overlap it but not each other.
    const day = layoutDay([seg("a", 540, 720), seg("b", 600, 630), seg("c", 660, 690)]);
    const { placed, overflow } = limitColumns(day, (columns) => (columns > 1 ? 1 : columns));
    expect(placed.map((p) => p.key)).toEqual(["a"]);
    expect(overflow.map((g) => [g.start, g.end, g.items])).toEqual([
      [600, 630, ["b"]],
      [660, 690, ["c"]],
    ]);
  });

  it("stops stretched events at the chip", () => {
    // d could stretch over two columns, but only two of three are shown.
    const day = layoutDay([
      seg("a", 540, 720),
      seg("b", 600, 630),
      seg("c", 600, 630),
      seg("d", 660, 700),
    ]);
    const { placed, overflow } = limitColumns(day, () => 2);
    expect(byKey(placed).d).toMatchObject({ column: 1, columns: 2, span: 1 });
    expect(overflow).toHaveLength(1);
    expect(overflow[0]!.items).toEqual(["c"]);
  });

  it("draws short hidden events at their minimum height", () => {
    const day = layoutDay([seg("a", 600, 660), seg("b", 600, 605)], 18);
    const { overflow } = limitColumns(day, () => 1, 18);
    expect(overflow[0]).toMatchObject({ start: 600, end: 618 });
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

  afterEach(() => setLocaleForTesting("en-US"));

  it("computes the visible days of each view", () => {
    expect(viewRange("day", new Date(2026, 9, 14, 15, 30)).days).toEqual([wednesday]);

    const work = viewRange("workweek", wednesday);
    expect(work.days.map((d) => d.getDate())).toEqual([12, 13, 14, 15, 16]);
    expect(work.end).toEqual(new Date(2026, 9, 17));

    // American weeks run Sunday to Saturday.
    const week = viewRange("week", wednesday);
    expect(week.days.map((d) => d.getDate())).toEqual([11, 12, 13, 14, 15, 16, 17]);

    const month = viewRange("month", wednesday);
    expect(month.days).toHaveLength(42);
    expect(month.start).toEqual(new Date(2026, 8, 27)); // Sunday before October 1
    expect(month.end).toEqual(new Date(2026, 10, 8));
  });

  it("starts weeks on Monday where the locale does", () => {
    setLocaleForTesting("en-GB");
    expect(viewRange("week", wednesday).days.map((d) => d.getDate())).toEqual([
      12, 13, 14, 15, 16, 17, 18,
    ]);
    expect(viewRange("month", wednesday).start).toEqual(new Date(2026, 8, 28));
  });

  it("shows the work week a weekend day belongs to", () => {
    const saturday = new Date(2026, 9, 17);
    const sunday = new Date(2026, 9, 18);
    const workDays = (d: Date) => viewRange("workweek", d).days.map((day) => day.getDate());
    expect(workDays(saturday)).toEqual([12, 13, 14, 15, 16]);
    // Where weeks start on Sunday, Sunday already looks ahead to Monday.
    expect(workDays(sunday)).toEqual([19, 20, 21, 22, 23]);
    setLocaleForTesting("en-GB");
    expect(workDays(sunday)).toEqual([12, 13, 14, 15, 16]);
  });

  it("moves by a day, a week or a month", () => {
    expect(shiftAnchor("day", wednesday, 1)).toEqual(new Date(2026, 9, 15));
    expect(shiftAnchor("workweek", wednesday, -1)).toEqual(new Date(2026, 9, 7));
    expect(shiftAnchor("week", wednesday, 1)).toEqual(new Date(2026, 9, 21));
    expect(shiftAnchor("month", new Date(2026, 0, 31), 1)).toEqual(new Date(2026, 1, 28));
  });

  it("titles ranges in American style", () => {
    expect(rangeTitle("day", wednesday)).toBe("Wednesday, October 14, 2026");
    expect(rangeTitle("workweek", wednesday)).toBe("October 12 – 16, 2026");
    expect(rangeTitle("week", wednesday)).toBe("October 11 – 17, 2026");
    expect(rangeTitle("month", wednesday)).toBe("October 2026");
    expect(rangeTitle("week", new Date(2026, 9, 1))).toBe("September 27 – October 3, 2026");
    expect(rangeTitle("week", new Date(2026, 11, 31))).toBe("December 27, 2026 – January 2, 2027");
  });

  it("has short titles for narrow windows", () => {
    expect(rangeTitle("day", wednesday, true)).toBe("Wed, Oct 14, 2026");
    expect(rangeTitle("workweek", wednesday, true)).toBe("Oct 12 – 16, 2026");
    expect(rangeTitle("week", new Date(2026, 9, 1), true)).toBe("Sep 27 – Oct 3, 2026");
    expect(rangeTitle("month", wednesday, true)).toBe("October 2026");
  });

  it("titles ranges in the locale's own style", () => {
    setLocaleForTesting("en-GB");
    expect(rangeTitle("day", wednesday)).toBe("Wednesday, 14 October 2026");
    // Newer ICU data puts spaces around the dash; either is the British style.
    expect(rangeTitle("workweek", wednesday)).toMatch(/^12 ?– ?16 October 2026$/);
  });

  it("numbers weeks only where the locale does", () => {
    // American calendars don't number weeks.
    expect(weekLabel("workweek", wednesday)).toBeNull();
    setLocaleForTesting("de-DE");
    expect(weekLabel("workweek", wednesday)).toBe("Week 42");
    expect(weekLabel("day", new Date(2026, 0, 1))).toBe("Week 1");
    // A month spans several weeks.
    expect(weekLabel("month", wednesday)).toBeNull();
  });

  it("keeps whole days across daylight-saving changes", () => {
    withTimeZone("Europe/Berlin", () => {
      // Clocks go back on Sunday, October 25, 2026 in Europe.
      const days = viewRange("week", new Date(2026, 9, 28)).days;
      expect(days.map((d) => [d.getDate(), d.getHours()])).toEqual(
        [25, 26, 27, 28, 29, 30, 31].map((d) => [d, 0]),
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
    // Wednesday has 4 items; with 3 lines, 3 show and "+1 more" counts the last.
    const visible = () => 3;
    expect(cellLanes(table, 2, visible)).toHaveLength(3);
    expect(hiddenItems(table, 2, 3).map((e) => e.id)).toEqual(["t3"]);
    expect(hiddenItems(table, 1, 3)).toEqual([]);

    // The bar is drawn once, from Tuesday, over three columns.
    const tuesday = cellLanes(table, 1, visible);
    expect(tuesday[0]!.piece).toMatchObject({ span: 3, joinsBefore: false, joinsAfter: false });
    expect(cellLanes(table, 2, visible)[0]!.piece).toBeNull();
  });

  it("draws a bar across every day when all days show the same lanes", () => {
    // Friday is full, the weekend is not: a Fri–Sun bar in the second lane is
    // drawn across all three days, or counted in "+N more" on each.
    const events = [
      ev("long", toAllDayIso(week[0]!), toAllDayIso(addDays(week[0]!, 7)), true),
      ev("fri", toAllDayIso(week[4]!), toAllDayIso(week[5]!), true),
      ev("summit", toAllDayIso(week[4]!), toAllDayIso(addDays(week[6]!, 1)), true),
      ev("t1", new Date(2026, 9, 16, 9), new Date(2026, 9, 16, 10)),
      ev("t2", new Date(2026, 9, 16, 11), new Date(2026, 9, 16, 12)),
    ];
    const placed = byKey(rowItems(events, week, { includeTimed: true }));
    expect(placed.summit).toMatchObject({ lane: 1, startCol: 4, endCol: 6 });
    const table = laneTable(Object.values(placed), 7);
    expect(cellLanes(table, 4, () => 2)[1]!.piece).toMatchObject({ span: 3 });
    expect(hiddenItems(table, 4, 2).map((e) => e.id)).toEqual(["fri", "t1", "t2"]);
    expect(hiddenItems(table, 5, 2)).toEqual([]);
    // With one line, the summit is counted on all three days instead.
    expect(cellLanes(table, 4, () => 1)).toHaveLength(1);
    expect([4, 5, 6].map((col) => hiddenItems(table, col, 1).length)).toEqual([4, 1, 1]);
  });
});
