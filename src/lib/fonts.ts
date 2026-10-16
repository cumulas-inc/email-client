// Font and text-size settings, turned into CSS.
//
// A font family name is user and system data: it comes from the font files
// on this computer and from saved settings. It never goes into CSS as is.
// `cssString` quotes it with CSS string escaping, after `cleanFamily` drops
// the characters that have no business in a name.

import type { InterfaceFont, ReadingFont, Settings, TextSize } from "../api/types";

/** The interface stack in tokens.css (`--font-sans`); a test keeps them equal. */
export const SANS_STACK =
  '"Inter Variable", "Inter", "Cantarell", "Noto Sans", system-ui, sans-serif';
/** The reading stack in tokens.css (`--font-serif`); a test keeps them equal. */
export const SERIF_STACK = '"Charis SIL", "Charter", "Bitstream Charter", Georgia, serif';

/** Zoom factor per text size. `TextSize::zoom` in crates/core/src/model.rs must match. */
export const TEXT_SCALE: Record<TextSize, number> = {
  smaller: 0.92,
  default: 1,
  larger: 1.1,
  largest: 1.2,
};

/** Must match `FONT_FAMILY_MAX_CHARS` in crates/core/src/model.rs. */
const MAX_FAMILY_CHARS = 200;

/**
 * C0 and C1 controls, line and paragraph separators, and bidirectional
 * marks, embeddings, overrides and isolates (they'd reorder the text
 * around the name). Mirrors `is_disallowed_in_font_family` in the core.
 */
function isUnsafeInName(ch: string): boolean {
  const c = ch.codePointAt(0)!;
  return (
    c <= 0x1f ||
    (c >= 0x7f && c <= 0x9f) ||
    c === 0x2028 ||
    c === 0x2029 ||
    c === 0x200e ||
    c === 0x200f ||
    (c >= 0x202a && c <= 0x202e) ||
    (c >= 0x2066 && c <= 0x2069)
  );
}

function stripUnsafe(text: string): string {
  let out = "";
  for (const ch of text) if (!isUnsafeInName(ch)) out += ch;
  return out;
}

const SPACE = /^\p{White_Space}$/u;

/**
 * `text` without Unicode White_Space at either end, like Rust's `str::trim`.
 * `String.prototype.trim` also drops U+FEFF, but a font whose name starts
 * with one is listed and saved with it, and only matches with it.
 */
function trimSpace(text: string): string {
  let start = 0;
  let end = text.length;
  while (start < end && SPACE.test(text[start]!)) start++;
  while (end > start && SPACE.test(text[end - 1]!)) end--;
  return text.slice(start, end);
}

/**
 * The family name with control characters and bidirectional controls
 * removed and the ends trimmed, or null when nothing usable is left (or it
 * is absurdly long).
 */
export function cleanFamily(name: string): string | null {
  const clean = trimSpace(stripUnsafe(name));
  if (!clean || [...clean].length > MAX_FAMILY_CHARS) return null;
  return clean;
}

/**
 * `value` as a double-quoted CSS string. Quotes and backslashes are escaped;
 * "<" and ">" become hex escapes so the string can't close a <style>
 * element it is written into (the message frame's style sheet); controls
 * and line breaks, which a CSS string can't hold, are dropped.
 */
export function cssString(value: string): string {
  let out = '"';
  for (const ch of value) {
    if (isUnsafeInName(ch)) continue;
    if (ch === '"' || ch === "\\") out += `\\${ch}`;
    else if (ch === "<" || ch === ">") out += `\\${ch.codePointAt(0)!.toString(16)} `;
    else out += ch;
  }
  return `${out}"`;
}

/** The chosen family in front of `fallback`, or just `fallback` if the name is unusable. */
export function withFamily(family: string, fallback: string): string {
  const clean = cleanFamily(family);
  return clean ? `${cssString(clean)}, ${fallback}` : fallback;
}

/** The `font-family` for the interface. */
export function interfaceStack(font: InterfaceFont): string {
  switch (font.kind) {
    case "system":
      return `system-ui, ${SANS_STACK}`;
    case "family":
      return withFamily(font.family, SANS_STACK);
    default:
      return SANS_STACK;
  }
}

/** The `font-family` for reading: subjects, titles, messages and the editor. */
export function readingStack(settings: Pick<Settings, "interfaceFont" | "readingFont">): string {
  const font: ReadingFont = settings.readingFont;
  switch (font.kind) {
    case "interface":
      return interfaceStack(settings.interfaceFont);
    case "family":
      return withFamily(font.family, SERIF_STACK);
    default:
      return SERIF_STACK;
  }
}

/** Identifies a font choice: its kind, or `family:` and the family name. */
export function fontKey(font: InterfaceFont | ReadingFont): string {
  return font.kind === "family" ? `family:${font.family}` : font.kind;
}

/** Whether these are the font and size choices the app ships with. */
export function isDefaultType(
  settings: Pick<Settings, "interfaceFont" | "readingFont" | "textSize">,
): boolean {
  return (
    settings.interfaceFont.kind === "default" &&
    settings.readingFont.kind === "default" &&
    settings.textSize === "default"
  );
}

export const DEFAULT_TYPE: Pick<Settings, "interfaceFont" | "readingFont" | "textSize"> = {
  interfaceFont: { kind: "default" },
  readingFont: { kind: "default" },
  textSize: "default",
};
