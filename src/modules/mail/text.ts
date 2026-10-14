// Pure text helpers for showing mail: file names cut in the middle, plain-text
// bodies split into paragraphs, quotes and preformatted blocks.

/** User-perceived characters, so a cut never splits an emoji or an accent. */
function graphemes(text: string): string[] {
  if (typeof Intl !== "undefined" && "Segmenter" in Intl) {
    const segmenter = new Intl.Segmenter(undefined, { granularity: "grapheme" });
    return Array.from(segmenter.segment(text), (s) => s.segment);
  }
  return Array.from(text);
}

/**
 * Splits a file name for middle truncation: the head can be cut with an
 * ellipsis while the tail (the end of the name and its extension) always
 * shows, so "report-final-v1.pdf" and "report-final-v2.pdf" stay apart.
 * Short names come back whole, as the head.
 */
export function splitFileName(name: string, keep = 10): { head: string; tail: string } {
  const ext = /\.[^.\s]{1,8}$/.exec(name)?.[0] ?? "";
  const stem = graphemes(name.slice(0, name.length - ext.length));
  if (stem.length <= keep + 4) return { head: name, tail: "" };
  return {
    head: stem.slice(0, stem.length - keep).join(""),
    tail: stem.slice(stem.length - keep).join("") + ext,
  };
}

// East Asian wide characters take about two Latin letters of room.
const WIDE = /[ᄀ-ᅟ⺀-〾ぁ-㏿㐀-䶿一-鿿가-힣豈-﫿︰-﹏＀-｠￠-￦]/u;

/** Roughly how many Latin letters of room a string needs. */
export function visualLength(text: string): number {
  let n = 0;
  for (const ch of graphemes(text)) n += WIDE.test(ch) ? 2 : 1;
  return n;
}

export type PlainBlock =
  /** Prose. Lines keep their breaks; runs of quoted lines are marked. */
  | { kind: "text"; lines: { text: string; quoted: boolean }[] }
  /** Columns, tables or box drawing: shown in monospace, without wrapping. */
  | { kind: "pre"; text: string }
  /** Everything after the "-- " signature separator. */
  | { kind: "signature"; text: string };

const BOX_DRAWING = /[─-▟]/;
/** "+-----+------+" or "|---|---|": the rules of an ASCII table. */
const TABLE_RULE = /^\s*[+|][-=+|:\s]{3,}[+|]\s*$/;
/** Words set apart by three or more spaces, or a tab: aligned columns. */
const COLUMN_GAP = /\S(?: {3,}|\t+)\S/;

/** Whether a paragraph was laid out with a fixed-width font in mind. */
export function looksPreformatted(lines: string[]): boolean {
  const content = lines.filter((l) => l.trim() !== "");
  if (content.length === 0) return false;
  if (content.some((l) => BOX_DRAWING.test(l) || TABLE_RULE.test(l))) return true;
  const aligned = content.filter((l) => COLUMN_GAP.test(l)).length;
  return aligned >= 2 && aligned * 2 >= content.length;
}

/**
 * Splits a plain-text body into paragraphs (at blank lines). Paragraphs that
 * look preformatted become "pre" blocks; the signature after "-- " is kept
 * apart so it can be set quieter.
 */
export function plainTextBlocks(text: string): PlainBlock[] {
  const lines = text.replace(/\r\n?/g, "\n").split("\n");
  const sigAt = lines.findIndex((l) => l === "-- " || l === "--");
  const body = sigAt >= 0 ? lines.slice(0, sigAt) : lines;
  const blocks: PlainBlock[] = [];
  let para: string[] = [];
  const flush = () => {
    if (para.length === 0) return;
    if (looksPreformatted(para)) blocks.push({ kind: "pre", text: para.join("\n") });
    else {
      blocks.push({
        kind: "text",
        lines: para.map((l) => ({ text: l, quoted: /^\s*>/.test(l) })),
      });
    }
    para = [];
  };
  for (const line of body) {
    if (line.trim() === "") flush();
    else para.push(line);
  }
  flush();
  if (sigAt >= 0) {
    const signature = lines
      .slice(sigAt + 1)
      .join("\n")
      .trim();
    if (signature) blocks.push({ kind: "signature", text: signature });
  }
  return blocks;
}
