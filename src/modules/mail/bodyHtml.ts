// Pure helpers for displaying message bodies: the sandbox document, its
// Content Security Policy, link handling and plain-text linkification.

/**
 * How the message is painted. Emails are authored for light backgrounds:
 * - "plain": light theme, blends into the pane.
 * - "paper": dark theme, shown on a light sheet so the original colours stay readable.
 * - "dark": dark theme, colours inverted (images are inverted back).
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

// The frame is a separate document: tokens from the app don't reach it, and
// the email needs light-theme colours whatever the app theme is.
const PAPER = {
  text: "#242424",
  muted: "#616161",
  link: "#0f6cbd",
  rule: "#d1d1d1",
  // In "dark" mode the whole frame is inverted: #e0e0e0 becomes the app's
  // dark surface (#1f1f1f).
  invertedBase: "#e0e0e0",
};

function baseCss(appearance: BodyAppearance, fontFamily: string): string {
  const padding = appearance === "paper" ? "20px 24px" : "4px 2px 12px";
  const background =
    appearance === "paper" ? "#ffffff" : appearance === "dark" ? PAPER.invertedBase : "transparent";
  const invertMedia =
    appearance === "dark"
      ? `img, video, picture, svg, [background], [style*="background-image"] {
           filter: invert(1) hue-rotate(180deg);
         }`
      : "";
  return `
    :root { color-scheme: light; }
    html { background: ${background}; overflow-y: hidden; }
    body {
      margin: 0;
      padding: ${padding};
      font-family: ${fontFamily};
      font-size: 14px;
      line-height: 1.5;
      color: ${PAPER.text};
      overflow-wrap: break-word;
      word-wrap: break-word;
      -webkit-text-size-adjust: 100%;
    }
    img { max-width: 100%; height: auto; }
    table { max-width: 100%; }
    pre, code { white-space: pre-wrap; font-family: ui-monospace, monospace; }
    a { color: ${PAPER.link}; cursor: pointer; }
    blockquote {
      margin: 8px 0;
      padding-left: 12px;
      border-left: 3px solid ${PAPER.rule};
      color: ${PAPER.muted};
    }
    hr { border: none; border-top: 1px solid ${PAPER.rule}; }
    ::selection { background: #cfe4fa; }
    ${invertMedia}
  `;
}

export interface SrcDocOptions {
  allowRemote: boolean;
  appearance: BodyAppearance;
  fontFamily?: string;
}

export const DEFAULT_FONT = '"Segoe UI", "Inter", "Cantarell", "Noto Sans", system-ui, sans-serif';

export function buildSrcDoc(html: string, options: SrcDocOptions): string {
  const csp = contentSecurityPolicy(options.allowRemote);
  return (
    "<!doctype html><html><head>" +
    '<meta charset="utf-8">' +
    `<meta http-equiv="Content-Security-Policy" content="${csp}">` +
    '<meta name="color-scheme" content="light">' +
    '<meta name="referrer" content="no-referrer">' +
    `<style>${baseCss(options.appearance, options.fontFamily ?? DEFAULT_FONT)}</style>` +
    `</head><body>${html}</body></html>`
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
