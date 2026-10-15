// Pure helpers for displaying message bodies: the sandbox document, its
// Content Security Policy, link handling and plain-text linkification.

import charis400Italic from "@fontsource/charis-sil/files/charis-sil-latin-400-italic.woff2?inline";
import charis400 from "@fontsource/charis-sil/files/charis-sil-latin-400-normal.woff2?inline";
import charis700Italic from "@fontsource/charis-sil/files/charis-sil-latin-700-italic.woff2?inline";
import charis700 from "@fontsource/charis-sil/files/charis-sil-latin-700-normal.woff2?inline";

import { cssString, SERIF_STACK } from "../../lib/fonts";

/**
 * How the message is painted. Emails are authored for light backgrounds:
 * - "plain": light theme, blends into the pane.
 * - "paper": dark theme, shown on a light sheet so the original colors stay readable.
 * - "dark": dark theme, colors inverted (images are inverted back).
 */
export type BodyAppearance = "plain" | "paper" | "dark";

export function contentSecurityPolicy(allowRemote: boolean): string {
  const remote = allowRemote ? " https: http:" : "";
  return [
    "default-src 'none'",
    "style-src 'unsafe-inline'",
    `img-src data: cid:${remote}`,
    "font-src data:",
    "form-action 'none'",
    "base-uri 'none'",
  ].join("; ");
}

// The frame is a separate document: the app's tokens and @font-face rules
// don't reach it, and it has its own CSP (font-src data:). So the bundled
// reading serif travels inside it as data: URLs, whatever the reading font:
// it is the default, the fallback behind a chosen family that isn't
// installed (see bodyFont.ts), and the face mail written here names. That is
// the Latin subset of Charis SIL in four styles, about 92 KB of WOFF2 (124 KB
// as base64) that every message document carries. Measured in Chromium, it
// adds about 5 ms to opening a message, which buys the same letter face as
// the subject above it; a face that isn't used is never decoded. Other
// scripts fall back, glyph by glyph, to the system serif.
const LATIN_RANGE =
  "U+0000-00FF, U+0131, U+0152-0153, U+02BB-02BC, U+02C6, U+02DA, U+02DC, U+0304, " +
  "U+0308, U+0329, U+2000-206F, U+20AC, U+2122, U+2191, U+2193, U+2212, U+2215, U+FEFF, U+FFFD";

/** An @font-face rule for the frame: Latin `src` (a data: URL) as `family`. */
export function dataFace(family: string, src: string, style: string, weight: string): string {
  return (
    `@font-face { font-family: ${cssString(family)}; font-style: ${style}; font-weight: ${weight};` +
    ` font-display: block; src: url(${src}) format("woff2"); unicode-range: ${LATIN_RANGE}; }`
  );
}

const SERIF_FACES = (
  [
    [charis400, 400, "normal"],
    [charis400Italic, 400, "italic"],
    [charis700, 700, "normal"],
    [charis700Italic, 700, "italic"],
  ] as const
)
  .map(([src, weight, style]) => dataFace("Charis SIL", src, style, String(weight)))
  .join("\n");

/** The face for message text that doesn't choose its own. */
export interface BodyFont {
  /** A `font-family` value, with any family name already quoted and escaped. */
  family: string;
  /**
   * @font-face rules that travel inside the frame (data: URLs) besides
   * Charis SIL's, which it always carries; or "".
   */
  faces: string;
}

/** The reading serif, Charis SIL. */
export const DEFAULT_BODY_FONT: BodyFont = { family: SERIF_STACK, faces: "" };

interface Palette {
  text: string;
  muted: string;
  link: string;
  rule: string;
  selection: string;
  background: string;
}

// Mail is written for light backgrounds, so in "plain" and "paper" the frame
// keeps the light theme's warm ink and teal, whatever the app theme is.
const PAPER: Palette = {
  text: "#1d1c1a",
  muted: "#57534c",
  link: "#0a6f6a",
  rule: "#e1ddd5",
  selection: "#c9e5e0",
  background: "transparent",
};

// In "dark" the whole frame goes through invert(1) hue-rotate(180deg), which
// is its own inverse. Each value here is the dark token run through that
// filter, so it comes out as the token itself: the card (#22211e), ink
// (#edebe6), secondary text (#bbb6ac), teal text (#5cc6bb), border (#3a3732)
// and selection (#1a3532). The mail's own colors invert the same way.
const INVERTED: Palette = {
  text: "#16140f",
  muted: "#4d483e",
  link: "#00685d",
  rule: "#cac7c2",
  selection: "#bbd6d3",
  background: "#dfdedb",
};

/** Mail laid out with tables sets its own width; anything else gets a measure. */
function isLaidOut(html: string): boolean {
  return /<table[\s>]/i.test(html);
}

/**
 * Whether the mail paints its own backgrounds (or brings a style sheet):
 * newsletters and other designed mail. Inverting those changes how they
 * look, so in the dark theme they keep the reader's choice of sheet.
 */
export function paintsOwnBackground(html: string): boolean {
  return /<style[\s>]|\bbgcolor\s*=|\bbackground(?:-color|-image)?\s*:|\sbackground\s*=/i.test(
    html,
  );
}

/**
 * How to paint a message. Light theme: plain, on the card. Dark theme: mail
 * that only sets text colors (most personal mail) is inverted, so it reads
 * light on dark like the rest of the app; designed mail follows `designed`,
 * the reader's choice between a light sheet and inverting it too.
 */
export function bodyAppearance(
  html: string,
  isDark: boolean,
  designed: "paper" | "dark" = "paper",
): BodyAppearance {
  if (!isDark) return "plain";
  return paintsOwnBackground(html) ? designed : "dark";
}

/**
 * "quiet" sets the text in the secondary color: the original under a reply
 * or forward, shown below what's being written.
 */
export type BodyTone = "normal" | "quiet";

function baseCss(
  appearance: BodyAppearance,
  font: BodyFont,
  measure: boolean,
  tone: BodyTone,
): string {
  const colors = appearance === "dark" ? INVERTED : PAPER;
  const padding = appearance === "paper" ? "24px 28px" : "2px 0 16px";
  const background = appearance === "paper" ? "#ffffff" : colors.background;
  const invertMedia =
    appearance === "dark"
      ? `img, video, picture, svg, [background], [style*="background-image"] {
           filter: invert(1) hue-rotate(180deg);
         }`
      : "";
  return `
    ${SERIF_FACES}
    ${font.faces}
    :root { color-scheme: light; }
    html { background: ${background}; overflow-y: hidden; }
    html:has(> body:dir(rtl)) { direction: rtl; }
    body {
      margin: 0;
      padding: ${padding};
      ${measure ? "max-width: 72ch;" : ""}
      font-family: ${font.family};
      font-size: 15.5px;
      line-height: 1.6;
      color: ${tone === "quiet" ? colors.muted : colors.text};
      overflow-wrap: break-word;
      word-wrap: break-word;
      -webkit-text-size-adjust: 100%;
      text-rendering: optimizeLegibility;
      -webkit-font-smoothing: antialiased;
    }
    p { margin: 0 0 0.75em; }
    ul, ol { margin: 0 0 0.75em; padding-inline-start: 1.5em; }
    li + li { margin-top: 0.2em; }
    li::marker { color: ${colors.muted}; }
    ul > li::marker { color: ${colors.link}; }
    img { max-width: 100%; height: auto; }
    table { max-width: 100%; }
    pre, code { white-space: pre-wrap; font-family: ui-monospace, "JetBrains Mono", monospace; font-size: 0.875em; }
    a { color: ${colors.link}; cursor: pointer; text-decoration-thickness: 1px; text-underline-offset: 2px; }
    blockquote {
      margin: 12px 0;
      padding-inline-start: 14px;
      border-inline-start: 2px solid ${colors.rule};
      color: ${colors.muted};
    }
    hr { border: none; border-top: 1px solid ${colors.rule}; margin: 16px 0; }
    ::selection { background: ${colors.selection}; }
    ${invertMedia}
  `;
}

export interface SrcDocOptions {
  allowRemote: boolean;
  appearance: BodyAppearance;
  /** The reading font, for text the mail doesn't set itself (default Charis SIL). */
  font?: BodyFont;
  tone?: BodyTone;
}

export function buildSrcDoc(html: string, options: SrcDocOptions): string {
  const csp = contentSecurityPolicy(options.allowRemote);
  const css = baseCss(
    options.appearance,
    options.font ?? DEFAULT_BODY_FONT,
    !isLaidOut(html),
    options.tone ?? "normal",
  );
  return (
    "<!doctype html><html><head>" +
    '<meta charset="utf-8">' +
    `<meta http-equiv="Content-Security-Policy" content="${csp}">` +
    '<meta name="color-scheme" content="light">' +
    '<meta name="referrer" content="no-referrer">' +
    `<style>${css}</style>` +
    // dir="auto": a letter in Arabic or Hebrew sets itself right to left.
    `</head><body dir="auto">${html}</body></html>`
  );
}

// --- Editable signatures -------------------------------------------------------

/**
 * What a signature's frame may load: its pictures and web fonts (https: or
 * data:) and inline styles. No scripts, from anywhere.
 */
export const SIGNATURE_CSP = [
  "default-src 'none'",
  "img-src https: data:",
  "style-src 'unsafe-inline'",
  "font-src https: data:",
  "form-action 'none'",
  "base-uri 'none'",
].join("; ");

/**
 * Whether a signature chooses its own colors (text or background) or
 * brings a style sheet. Those colors were picked for a light page.
 */
export function setsOwnColors(html: string): boolean {
  return paintsOwnBackground(html) || /(?:^|[^-\w])color\s*:|<font\b[^>]*\scolor\s*=/i.test(html);
}

/**
 * How to paint an editable signature. Light theme: plain. Dark theme: one
 * with its own colors on a light sheet, as the quoted original is; one
 * without in the dark theme's own colors ("dark", not inverted: the frame
 * is set in light ink, so the caret and selection follow).
 */
export function signatureAppearance(html: string, isDark: boolean): BodyAppearance {
  if (!isDark) return "plain";
  return setsOwnColors(html) ? "paper" : "dark";
}

// The dark theme's ink, tertiary text, teal and selection.
const DARK_INK: Palette = {
  text: "#edebe6",
  muted: "#9a948a",
  link: "#5cc6bb",
  rule: "#3a3732",
  selection: "#1a3532",
  background: "transparent",
};

/** Whether HTML shows nothing: no text and no picture. */
export function isBlankHtml(html: string): boolean {
  if (/<img[\s>]/i.test(html)) return false;
  return (
    html
      .replace(/<style[\s\S]*?<\/style>/gi, "")
      .replace(/<[^>]*>/g, "")
      .replace(/&nbsp;|&#160;|\u00a0/gi, "")
      .trim() === ""
  );
}

export interface SignatureDocOptions {
  appearance: BodyAppearance;
  font: BodyFont;
  /**
   * Text the signature doesn't style is set like the letter, in the reading
   * font's size and leading (the person's own signature, which goes out in
   * the letter's style). Otherwise the frame stays neutral (an
   * organization's, which keeps its own).
   */
  letter: boolean;
  /** Shown while it's empty. */
  placeholder: string;
  /** The body's accessible name. */
  label: string;
  /** Lines it's tall at least, so a click anywhere in it lands in it. */
  minLines: number;
}

function attribute(value: string): string {
  return value.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");
}

/**
 * The document an editable signature is shown in: neutral, without the
 * reader's measure or paragraph spacing, so the signature looks as it will
 * be sent; its body is the editing host. The signature itself goes in once
 * the frame has loaded (see SignatureEditor).
 */
export function buildSignatureSrcDoc(options: SignatureDocOptions): string {
  const { appearance, font, letter } = options;
  const colors = appearance === "dark" ? DARK_INK : PAPER;
  const paper = appearance === "paper";
  const lineHeight = letter ? "1.6" : "normal";
  const css = `
    ${SERIF_FACES}
    ${font.faces}
    :root { color-scheme: ${appearance === "dark" ? "dark" : "light"}; }
    html { background: ${paper ? "#ffffff" : "transparent"}; overflow: hidden; }
    body {
      margin: 0;
      padding: ${paper ? "12px 16px" : "0"};
      min-height: calc(${options.minLines} * ${letter ? "1.6em" : "1.25em"});
      font-family: ${font.family};
      font-size: 15.5px;
      line-height: ${lineHeight};
      color: ${colors.text};
      caret-color: ${colors.text};
      overflow-wrap: break-word;
      word-wrap: break-word;
      outline: none;
      cursor: text;
      -webkit-text-size-adjust: 100%;
      -webkit-font-smoothing: antialiased;
    }
    a { color: ${colors.link}; cursor: text; }
    img { max-width: 100%; height: auto; }
    ::selection { background: ${colors.selection}; }
    body[data-empty]::before {
      content: attr(data-placeholder);
      float: left;
      height: 0;
      color: ${colors.muted};
      pointer-events: none;
    }
  `;
  return (
    "<!doctype html><html><head>" +
    '<meta charset="utf-8">' +
    `<meta http-equiv="Content-Security-Policy" content="${SIGNATURE_CSP}">` +
    '<meta name="referrer" content="no-referrer">' +
    `<style>${css}</style>` +
    "</head>" +
    '<body contenteditable="true" role="textbox" aria-multiline="true" spellcheck="true" dir="auto"' +
    ` aria-label="${attribute(options.label)}" data-placeholder="${attribute(options.placeholder)}"` +
    " data-empty></body></html>"
  );
}

export type LinkAction =
  { kind: "open"; url: string } | { kind: "mailto"; url: string } | { kind: "ignore" };

/** What clicking a link in a message does: only http(s) and mailto are followed. */
export function classifyLink(href: string | null | undefined): LinkAction {
  if (!href) return { kind: "ignore" };
  let url: URL;
  try {
    url = new URL(href.trim());
  } catch {
    return { kind: "ignore" };
  }
  if (url.protocol === "http:" || url.protocol === "https:") return { kind: "open", url: url.href };
  if (url.protocol === "mailto:") return { kind: "mailto", url: href.trim() };
  return { kind: "ignore" };
}

interface NodeLike {
  nodeName: string;
  parentNode: NodeLike | null;
  getAttribute?: (name: string) => string | null;
}

/** The <a href> at or above `target` (works across the frame's realm). */
export function anchorOf(target: EventTarget | null): NodeLike | null {
  let node = target as NodeLike | null;
  while (node) {
    if (node.nodeName === "A" && node.getAttribute?.("href")) return node;
    node = node.parentNode;
  }
  return null;
}

// --- Plain text ------------------------------------------------------------------

const LINK_PATTERN =
  /\b(?:https?:\/\/[^\s<>"']+|mailto:[^\s<>"']+|www\.[^\s<>"']+|[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,})/gi;
const TRAILING = /[.,;:!?)\]}'"]+$/;

/** Splits text into plain runs and links (with the URL each link opens). */
export function linkify(text: string): ({ text: string } | { text: string; href: string })[] {
  const out: ({ text: string } | { text: string; href: string })[] = [];
  let last = 0;
  for (const match of text.matchAll(LINK_PATTERN)) {
    let raw = match[0];
    const trailing = TRAILING.exec(raw)?.[0] ?? "";
    raw = raw.slice(0, raw.length - trailing.length);
    if (!raw) continue;
    const start = match.index ?? 0;
    if (start > last) out.push({ text: text.slice(last, start) });
    const href = /^(https?:|mailto:)/i.test(raw)
      ? raw
      : raw.startsWith("www.")
        ? `https://${raw}`
        : `mailto:${raw}`;
    out.push({ text: raw, href });
    last = start + raw.length;
  }
  if (last < text.length) out.push({ text: text.slice(last) });
  return out;
}
