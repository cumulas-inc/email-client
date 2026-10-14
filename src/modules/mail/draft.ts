// Pure helpers for composing: address parsing, reply/forward prefill and
// quoting the original message.

import type { Account, EmailAddress, Message, ReplyKind } from "../../api/types";
import { formatDate, formatTime } from "../../lib/datetime";
import { displayName } from "../../lib/format";
import { messageDate } from "./dates";
import type { ComposeDraft } from "./mailStore";

export type DraftInput = Omit<ComposeDraft, "id" | "dirty">;

const ADDRESS = /^[^\s@<>()",;:[\]\\]+@[^\s@<>()",;:[\]\\]+\.[^\s@<>()",;:[\]\\]+$/;

export function isValidAddress(address: string): boolean {
  return ADDRESS.test(address.trim());
}

/** Parses one entry: `a@b.c`, `Name <a@b.c>` or `"Name" <a@b.c>`. */
export function parseAddress(text: string): EmailAddress | null {
  const trimmed = text.trim();
  if (!trimmed) return null;
  const angle = /^(.*?)<\s*([^<>]+?)\s*>\s*$/.exec(trimmed);
  if (angle) {
    const address = angle[2]!.replace(/^mailto:/i, "");
    if (!isValidAddress(address)) return null;
    const name = angle[1]!
      .trim()
      .replace(/^"(.*)"$/, "$1")
      .trim();
    return { name: name || null, address };
  }
  const address = trimmed.replace(/^mailto:/i, "");
  return isValidAddress(address) ? { name: null, address } : null;
}

/** Splits a pasted list on commas, semicolons and new lines (not inside quotes or <>). */
export function splitAddressList(text: string): string[] {
  const parts: string[] = [];
  let current = "";
  let quoted = false;
  let angle = false;
  for (const ch of text) {
    if (ch === '"') quoted = !quoted;
    else if (ch === "<" && !quoted) angle = true;
    else if (ch === ">" && !quoted) angle = false;
    if (!quoted && !angle && (ch === "," || ch === ";" || ch === "\n" || ch === "\r")) {
      if (current.trim()) parts.push(current.trim());
      current = "";
    } else {
      current += ch;
    }
  }
  if (current.trim()) parts.push(current.trim());
  return parts;
}

export function parseAddressList(text: string): { valid: EmailAddress[]; invalid: string[] } {
  const valid: EmailAddress[] = [];
  const invalid: string[] = [];
  for (const part of splitAddressList(text)) {
    const parsed = parseAddress(part);
    if (parsed) valid.push(parsed);
    else invalid.push(part);
  }
  return { valid, invalid };
}

export function sameAddress(a: EmailAddress, b: EmailAddress): boolean {
  return a.address.toLowerCase() === b.address.toLowerCase();
}

/** Removes duplicates and the excluded addresses, keeping the first occurrence. */
export function uniqueAddresses(
  list: EmailAddress[],
  exclude: EmailAddress[] = [],
): EmailAddress[] {
  const seen = new Set(exclude.map((e) => e.address.toLowerCase()));
  const out: EmailAddress[] = [];
  for (const a of list) {
    const key = a.address.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(a);
  }
  return out;
}

const RE_PREFIX = /^\s*(re|aw|sv|antw)\s*(\[\d+\])?\s*:/i;
const FWD_PREFIX = /^\s*(fwd?|wg|tr|rv|enc)\s*(\[\d+\])?\s*:/i;

export function prefixSubject(subject: string, kind: ReplyKind): string {
  const s = subject.trim();
  if (kind === "forward") return FWD_PREFIX.test(s) ? s : `Fwd: ${s}`;
  return RE_PREFIX.test(s) ? s : `Re: ${s}`;
}

export function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

export function textToHtml(text: string): string {
  return escapeHtml(text).replace(/\r?\n/g, "<br>");
}

function formatAddressHtml(a: EmailAddress): string {
  return a.name ? `${escapeHtml(a.name)} &lt;${escapeHtml(a.address)}&gt;` : escapeHtml(a.address);
}

/** When the original was sent: "Wed, Oct 14, 2026, at 9:42 AM". */
function quoteDate(date: Date): string {
  return `${formatDate(date, "weekdayShortYear")}, at ${formatTime(date)}`;
}

/**
 * The line over a quoted reply: "On Wed, Oct 14, 2026, at 9:42 AM, Priya
 * Raman wrote:". A message without a date (sent as 1 Jan 1970) gets no "On".
 */
export function attributionLine(original: Message): string {
  const sent = messageDate(original.receivedAt);
  const who = original.from ? displayName(original.from) : null;
  if (!sent) return `${who ?? "The sender"} wrote:`;
  return `On ${quoteDate(sent)}, ${who ?? "the sender"} wrote:`;
}

// The quote is sent, so its styles are inline and its colors are fixed: the
// light theme's hairline and tertiary text. The reader shows them as they
// are on a light page and inverts them with the rest of the mail in dark.
const QUOTE_RULE = "#d8d4cb";
const QUOTE_LABEL = "#716c63";

/**
 * Removes `data-signature-stamp` from quoted HTML. The signature relay reads
 * that marker as "already signed", so a stamp in the quoted original would
 * stop the reply from getting the sender's own signature.
 */
export function stripSignatureStamps(html: string): string {
  if (!/data-signature-stamp/i.test(html)) return html;
  // A <template> parses without loading images or running anything.
  const template = document.createElement("template");
  template.innerHTML = html;
  const strip = (root: DocumentFragment) => {
    for (const el of root.querySelectorAll("[data-signature-stamp]")) {
      el.removeAttribute("data-signature-stamp");
    }
    for (const inner of root.querySelectorAll("template")) strip(inner.content);
  };
  strip(template.content);
  return template.innerHTML;
}

/** A forward's header: "Forwarded message", then From, Date, Subject, To and Cc. */
function forwardHeader(original: Message): string {
  const sent = messageDate(original.receivedAt);
  const list = (addresses: EmailAddress[]) => addresses.map(formatAddressHtml).join(", ");
  const rows: [string, string][] = [];
  if (original.from) rows.push(["From", formatAddressHtml(original.from)]);
  if (sent) rows.push(["Date", escapeHtml(quoteDate(sent))]);
  if (original.subject.trim()) rows.push(["Subject", escapeHtml(original.subject)]);
  if (original.to.length > 0) rows.push(["To", list(original.to)]);
  if (original.cc.length > 0) rows.push(["Cc", list(original.cc)]);
  // Quiet labels in a column, values in ink; long lists wrap under the value.
  const row = ([label, value]: [string, string]) =>
    `<div style="padding-left:4.5em;text-indent:-4.5em">` +
    `<span style="display:inline-block;width:4.5em;text-indent:0;color:${QUOTE_LABEL}">${label}</span>` +
    `${value}</div>`;
  return (
    `<div style="margin:0 0 16px;font-size:0.9375em;line-height:1.5">` +
    `<div style="margin:0 0 2px;color:${QUOTE_LABEL}">Forwarded message</div>` +
    rows.map(row).join("") +
    `</div>`
  );
}

/**
 * The original under a reply or forward. A reply quotes it under one
 * attribution line, in a <blockquote type="cite"> with a rule down its side.
 * A forward passes it on as it is, under a compact header.
 */
export function quoteOriginal(original: Message, kind: ReplyKind = "reply"): string {
  const body = stripSignatureStamps(original.bodyHtml ?? textToHtml(original.bodyText ?? ""));
  if (kind === "forward") {
    return (
      `<div class="cumulas-quote cumulas-forward">` +
      forwardHeader(original) +
      `<div>${body}</div>` +
      `</div>`
    );
  }
  return (
    `<div class="cumulas-quote">` +
    `<p style="margin:0 0 8px">${escapeHtml(attributionLine(original))}</p>` +
    `<blockquote type="cite" style="margin:0;padding:0 0 0 12px;border-left:2px solid ${QUOTE_RULE}">` +
    body +
    `</blockquote>` +
    `</div>`
  );
}

/** Recipients for a reply, excluding the user's own addresses. */
export function replyRecipients(
  original: Message,
  kind: ReplyKind,
  own: EmailAddress[],
): { to: EmailAddress[]; cc: EmailAddress[] } {
  if (kind === "forward") return { to: [], cc: [] };
  const fromMe = original.from ? own.some((o) => sameAddress(o, original.from!)) : false;
  // Replying to something we sent goes back to its recipients.
  const primary = fromMe
    ? original.to
    : original.replyTo.length > 0
      ? original.replyTo
      : original.from
        ? [original.from]
        : [];
  if (kind === "reply") {
    const to = uniqueAddresses(primary, fromMe ? own : []);
    return { to: to.length > 0 ? to : primary.slice(0, 1), cc: [] };
  }
  const to = uniqueAddresses([...primary, ...original.to], own);
  const cc = uniqueAddresses(original.cc, [...own, ...to]);
  return { to, cc };
}

/** Draft for replying to or forwarding `original`. */
export function replyDraft(original: Message, kind: ReplyKind, accounts: Account[]): DraftInput {
  const own = accounts.map((a) => ({ name: a.displayName, address: a.email }));
  const { to, cc } = replyRecipients(original, kind, own);
  return {
    accountId: original.accountId,
    to,
    cc,
    bcc: [],
    showCc: cc.length > 0,
    showBcc: false,
    subject: prefixSubject(original.subject, kind),
    bodyHtml: "",
    quoteHtml: quoteOriginal(original, kind),
    attachments: [],
    importance: "normal",
    replyToMessageId: original.id,
    replyKind: kind,
    focus: kind === "forward" ? "to" : "body",
  };
}

export function newDraft(accountId: string, init: Partial<DraftInput> = {}): DraftInput {
  return {
    to: [],
    cc: [],
    bcc: [],
    subject: "",
    bodyHtml: "",
    quoteHtml: null,
    attachments: [],
    importance: "normal",
    replyToMessageId: null,
    replyKind: null,
    focus: "to",
    ...init,
    accountId,
    showCc: init.showCc ?? (init.cc?.length ?? 0) > 0,
    showBcc: init.showBcc ?? (init.bcc?.length ?? 0) > 0,
  };
}

/** Parses a mailto: URL into draft fields. */
export function parseMailto(url: string): Partial<DraftInput> | null {
  if (!/^mailto:/i.test(url)) return null;
  const rest = url.slice(7);
  const [path = "", query = ""] = rest.split("?", 2);
  const decode = (s: string) => {
    try {
      return decodeURIComponent(s.replace(/\+/g, " "));
    } catch {
      return s;
    }
  };
  const to = parseAddressList(decode(path)).valid;
  const out: Partial<DraftInput> = { to };
  for (const pair of query.split("&")) {
    if (!pair) continue;
    const [k = "", v = ""] = pair.split("=", 2);
    const value = decode(v);
    switch (k.toLowerCase()) {
      case "to":
        out.to = [...(out.to ?? []), ...parseAddressList(value).valid];
        break;
      case "cc":
        out.cc = parseAddressList(value).valid;
        break;
      case "bcc":
        out.bcc = parseAddressList(value).valid;
        break;
      case "subject":
        out.subject = value;
        break;
      case "body":
        out.bodyHtml = `<p>${textToHtml(value)}</p>`;
        break;
    }
  }
  return out;
}

/** True when the editor holds no text (TipTap's empty document is `<p></p>`). */
export function isEmptyHtml(html: string): boolean {
  return html.replace(/<(p|br|\/p)\s*\/?>/gi, "").trim() === "";
}

// The serif the editor shows, so recipients see what was written. Charter
// and Georgia stand in where Charis SIL isn't installed.
const BODY_STYLE =
  "font-family:'Charis SIL',Charter,'Bitstream Charter',Georgia,serif;font-size:12pt;line-height:1.5";

/**
 * The HTML that gets sent: the editor content, then the quoted original.
 * Paragraphs get no margins (each Enter is one line, as in most mail
 * programs) and empty ones keep their height.
 */
export function composeBodyHtml(bodyHtml: string, quoteHtml: string | null): string {
  const body = (isEmptyHtml(bodyHtml) ? "<p></p>" : bodyHtml)
    .replace(/<p><\/p>/g, "<p><br></p>")
    .replace(/<p>/g, '<p style="margin:0">');
  const quote = quoteHtml ? `<br>${quoteHtml}` : "";
  return `<div style="${BODY_STYLE}">${body}${quote}</div>`;
}

export const ATTACHMENT_WARN_BYTES = 20 * 1024 * 1024;

/** Reads a File as base64 (without the data: URL prefix). */
export function readFileBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error ?? new Error(`Couldn't read ${file.name}.`));
    reader.onload = () => {
      const result = String(reader.result ?? "");
      const comma = result.indexOf(",");
      resolve(comma >= 0 ? result.slice(comma + 1) : result);
    };
    reader.readAsDataURL(file);
  });
}
