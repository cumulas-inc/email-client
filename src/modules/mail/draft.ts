// Pure helpers for composing: address parsing, reply/forward prefill and
// quoting the original message.

import type { Account, EmailAddress, Message, ReplyKind } from "../../api/types";
import { formatDate, formatTime } from "../../lib/datetime";
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

/** The quote header's date: "Wednesday, October 14, 2026 3:05 PM". */
function sentLine(date: Date): string {
  return `${formatDate(date, "weekdayLongYear")} ${formatTime(date)}`;
}

/** The usual From/Sent/To/Subject block, followed by the original body. */
export function quoteOriginal(original: Message): string {
  const sent = messageDate(original.receivedAt);
  const rows: [string, string][] = [
    ["From", original.from ? formatAddressHtml(original.from) : ""],
    // A message without a date (sent as 1 Jan 1970) gets no "Sent" line.
    ...(sent ? [["Sent", escapeHtml(sentLine(sent))] as [string, string]] : []),
    ["To", original.to.map(formatAddressHtml).join("; ")],
  ];
  if (original.cc.length > 0) rows.push(["Cc", original.cc.map(formatAddressHtml).join("; ")]);
  rows.push(["Subject", escapeHtml(original.subject)]);
  const header = rows.map(([k, v]) => `<b>${k}:</b> ${v}`).join("<br>");
  const body = original.bodyHtml ?? textToHtml(original.bodyText ?? "");
  return (
    `<div class="cumulas-quote">` +
    `<hr style="border:none;border-top:1px solid #d8d4cb;margin:16px 0 8px">` +
    `<p style="margin:0 0 12px">${header}</p>` +
    `<div>${body}</div>` +
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
    quoteHtml: quoteOriginal(original),
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
