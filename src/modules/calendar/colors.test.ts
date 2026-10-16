import { describe, expect, it } from "vitest";

import {
  DARK_SURFACE,
  GRAPHIC_CONTRAST,
  LIGHT_SURFACE,
  META_CONTRAST,
  TEXT_CONTRAST,
  calendarTints,
  contrast,
  oklch,
  parseColor,
  tintStyle,
} from "./colors";

/** The account and calendar palette offered in settings. */
const PALETTE = [
  "#0b7a75", // teal
  "#2b7aa8", // sky
  "#b35f26", // apricot
  "#8a4f7d", // plum
  "#4d7a2e", // moss
  "#8c6a10", // gold
  "#b8434f", // rose
  "#5e6b7a", // slate
];

/** Colors that arrive from servers or older data. */
const EXTREMES = [
  "#ffb900", // bright yellow: hard to read on light backgrounds
  "#13a10e", // vivid green
  "#c239b3", // magenta
  "#1a1a6e", // very dark navy
  "#000000",
  "#ffffff",
  "#abc",
];

function hueDistance(a: number, b: number): number {
  const d = Math.abs(a - b) % 360;
  return d > 180 ? 360 - d : d;
}

describe("calendar color tints", () => {
  it("parses hex colors", () => {
    expect(parseColor("#0b7a75")).toEqual([11, 122, 117]);
    expect(parseColor("abc")).toEqual([170, 187, 204]);
    expect(parseColor("not a color")).toBeNull();
  });

  it.each([...PALETTE, ...EXTREMES])("keeps %s readable in both themes", (color) => {
    const { light, dark } = calendarTints(color);
    for (const [tint, surface] of [
      [light, LIGHT_SURFACE],
      [dark, DARK_SURFACE],
    ] as const) {
      for (const ground of [tint.bg, tint.bgHover]) {
        expect(contrast(tint.fg, ground)).toBeGreaterThanOrEqual(TEXT_CONTRAST);
        expect(contrast(tint.sub, ground)).toBeGreaterThanOrEqual(META_CONTRAST);
        // The inset stripe and outlines show on the tint and on the card.
        expect(contrast(tint.bar, ground)).toBeGreaterThanOrEqual(GRAPHIC_CONTRAST);
      }
      expect(contrast(tint.bar, surface)).toBeGreaterThanOrEqual(GRAPHIC_CONTRAST);
      // Free events are outlined on the card with title text.
      expect(contrast(tint.fg, surface)).toBeGreaterThanOrEqual(TEXT_CONTRAST);
      // Check marks and badge letters on filled swatches.
      expect(contrast(tint.onSolid, tint.solid)).toBeGreaterThanOrEqual(META_CONTRAST);
    }
    // Light tints stay quiet on the white card, dark ones on the dark card.
    expect(contrast(light.bg, LIGHT_SURFACE)).toBeLessThan(1.3);
    expect(contrast(dark.bg, DARK_SURFACE)).toBeLessThan(1.4);
  });

  it.each(PALETTE.filter((c) => c !== "#5e6b7a"))("keeps the hue of %s", (color) => {
    const [, , hue] = oklch(color);
    const { light, dark } = calendarTints(color);
    for (const tint of [light, dark]) {
      for (const shade of [tint.bg, tint.fg, tint.sub, tint.bar]) {
        const [, chroma, shadeHue] = oklch(shade);
        // Tinted, not grey, and still recognizably the same color.
        expect(chroma).toBeGreaterThan(0.015);
        expect(hueDistance(shadeHue, hue)).toBeLessThan(15);
      }
    }
  });

  it("uses the palette colors as they are for stripes and swatches in light mode", () => {
    for (const color of PALETTE) {
      expect(calendarTints(color).light.bar).toBe(color);
      expect(calendarTints(color).light.solid).toBe(color);
    }
  });

  it("darkens colors that would vanish on white", () => {
    const { light } = calendarTints("#ffb900");
    expect(light.bar).not.toBe("#ffb900");
    expect(oklch(light.bar)[0]).toBeLessThan(oklch("#ffb900")[0]);
  });

  it("gives every calendar the same lightness steps", () => {
    const lightness = PALETTE.map((c) => oklch(calendarTints(c).light.bg)[0]);
    const darkness = PALETTE.map((c) => oklch(calendarTints(c).dark.bg)[0]);
    expect(Math.max(...lightness) - Math.min(...lightness)).toBeLessThan(0.01);
    expect(Math.max(...darkness) - Math.min(...darkness)).toBeLessThan(0.01);
  });

  it("leans neutral colors toward the warm paper", () => {
    for (const tint of Object.values(calendarTints("#808080"))) {
      const [r, , b] = parseColor(tint.bg)!;
      expect(r).toBeGreaterThan(b);
    }
  });

  it("falls back to the brand teal for missing colors", () => {
    expect(calendarTints(undefined)).toEqual(calendarTints("#0b7a75"));
    expect(calendarTints("bogus").light.bar).toBe("#0b7a75");
  });

  it("exposes both themes as CSS custom properties", () => {
    const style = tintStyle("#2b7aa8") as Record<string, string>;
    for (const name of ["bg", "bg-hover", "fg", "sub", "bar", "solid", "on-solid"]) {
      expect(style[`--tint-l-${name}`]).toMatch(/^#[0-9a-f]{6}$/);
      expect(style[`--tint-d-${name}`]).toMatch(/^#[0-9a-f]{6}$/);
    }
  });
});
