import { afterEach, describe, expect, it } from "vitest";

import { setLocaleForTesting } from "../../lib/datetime";
import { formatClock } from "./eventText";

describe("formatClock", () => {
  afterEach(() => setLocaleForTesting("en-US"));

  it("leaves out AM and PM, which the hour labels already show", () => {
    expect(formatClock(new Date(2026, 9, 14, 10, 30))).toBe("10:30");
    expect(formatClock(new Date(2026, 9, 14, 15, 5))).toBe("3:05");
    expect(formatClock(new Date(2026, 9, 14, 0, 0))).toBe("12:00");
  });

  it("keeps 24-hour clocks as they are", () => {
    setLocaleForTesting("de-DE");
    expect(formatClock(new Date(2026, 9, 14, 15, 5))).toBe("15:05");
    expect(formatClock(new Date(2026, 9, 14, 9, 30))).toBe("9:30");
  });
});
