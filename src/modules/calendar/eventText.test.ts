import { afterEach, describe, expect, it } from "vitest";

import { setLocaleForTesting } from "../../lib/datetime";
import { formatCompactTime, formatCompactTimeRange } from "./eventText";

const at = (hours: number, minutes = 0) => new Date(2026, 9, 14, hours, minutes);
/** The thin-spaced dash between the two times. */
const TO = " – ";

describe("compact event times", () => {
  afterEach(() => setLocaleForTesting("en-US"));

  it("puts AM/PM once, on the end, when both times share it", () => {
    setLocaleForTesting("en-US");
    expect(formatCompactTimeRange(at(22, 30), at(23))).toBe(`10:30${TO}11 PM`);
    expect(formatCompactTimeRange(at(14), at(15, 15))).toBe(`2${TO}3:15 PM`);
    expect(formatCompactTimeRange(at(9, 30), at(9, 45))).toBe(`9:30${TO}9:45 AM`);
    expect(formatCompactTimeRange(at(10), at(11, 30))).toBe(`10${TO}11:30 AM`);
  });

  it("keeps AM/PM on both times when they differ", () => {
    setLocaleForTesting("en-US");
    // "11 – 12 PM" would read as 11 at night.
    expect(formatCompactTimeRange(at(11), at(12))).toBe(`11 AM${TO}12 PM`);
    expect(formatCompactTimeRange(at(11, 30), at(13))).toBe(`11:30 AM${TO}1 PM`);
    expect(formatCompactTimeRange(at(10, 30), at(12))).toBe(`10:30 AM${TO}12 PM`);
    expect(formatCompactTimeRange(at(23), at(0))).toBe(`11 PM${TO}12 AM`);
  });

  it("keeps AM/PM on a lone start time", () => {
    setLocaleForTesting("en-US");
    expect(formatCompactTime(at(10, 30))).toBe("10:30 AM");
    expect(formatCompactTime(at(11))).toBe("11 AM");
    expect(formatCompactTime(at(0))).toBe("12 AM");
    expect(formatCompactTime(at(15, 5))).toBe("3:05 PM");
  });

  it("leaves 24-hour clocks as they are", () => {
    setLocaleForTesting("de-DE");
    expect(formatCompactTimeRange(at(10, 30), at(12))).toBe(`10:30${TO}12:00`);
    expect(formatCompactTimeRange(at(11), at(12))).toBe(`11:00${TO}12:00`);
    expect(formatCompactTimeRange(at(14), at(15, 15))).toBe(`14:00${TO}15:15`);
    expect(formatCompactTime(at(9, 30))).toBe("9:30");
    expect(formatCompactTime(at(15))).toBe("15:00");
  });
});
