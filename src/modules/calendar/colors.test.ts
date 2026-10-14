import { describe, expect, it } from "vitest";

import { TEXT_CONTRAST, calendarTints, contrast, parseColor, tintStyle } from "./colors";

const SAMPLES = [
  "#0f6cbd", // blue
  "#038387", // teal
  "#8764b8", // purple
  "#c239b3", // magenta
  "#ca5010", // orange
  "#ffb900", // yellow: hard to read on light backgrounds
  "#13a10e", // green
  "#000000",
  "#ffffff",
  "#abc",
];

describe("calendar colour tints", () => {
  it("parses hex colours", () => {
    expect(parseColor("#0f6cbd")).toEqual([15, 108, 189]);
    expect(parseColor("abc")).toEqual([170, 187, 204]);
    expect(parseColor("not a colour")).toBeNull();
  });

  it.each(SAMPLES)("keeps event text readable on %s in both themes", (color) => {
    const { light, dark } = calendarTints(color);
    for (const tint of [light, dark]) {
      expect(contrast(tint.fg, tint.bg)).toBeGreaterThanOrEqual(TEXT_CONTRAST);
      expect(contrast(tint.fg, tint.bgHover)).toBeGreaterThanOrEqual(TEXT_CONTRAST);
      expect(contrast(tint.onSolid, tint.solid)).toBeGreaterThanOrEqual(4.5);
    }
    // Light tints are light and dark tints are dark.
    expect(contrast(light.bg, "#ffffff")).toBeLessThan(1.5);
    expect(contrast(dark.bg, "#1f1f1f")).toBeLessThan(2.5);
    // The accent bar stands out from the dark surface.
    expect(contrast(dark.bar, "#1f1f1f")).toBeGreaterThanOrEqual(3);
  });

  it("falls back to the brand colour for missing colours", () => {
    expect(calendarTints(undefined)).toEqual(calendarTints("#0f6cbd"));
    expect(calendarTints("bogus").light.bar).toBe("#0f6cbd");
  });

  it("exposes both themes as CSS custom properties", () => {
    const style = tintStyle("#038387") as Record<string, string>;
    expect(style["--tint-l-bg"]).toMatch(/^#[0-9a-f]{6}$/);
    expect(style["--tint-d-fg"]).toMatch(/^#[0-9a-f]{6}$/);
  });
});
