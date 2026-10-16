// Worst-case demo data for the in-memory backend, loaded with `?worst`: the
// longest, shortest, emptiest, most crowded and least Latin values that real
// accounts produce, so the UI can be stress-tested against them. Every person
// and company is fictional. Dates are relative to "now" like the regular demo,
// and the variety comes from a seeded generator, so each load is the same.

import type {
  Account,
  Attendee,
  Calendar,
  CalendarEvent,
  Contact,
  EmailAddress,
  Folder,
  Importance,
  Message,
  Phone,
  ResponseStatus,
  Task,
  TaskList,
} from "../types";
import { type MockState, allDay, at, dateOnly, folder, iso, recount } from "./data";

// --- Generators ------------------------------------------------------------------

/** mulberry32: a tiny seeded generator, so the "random" data never changes. */
function seeded(seed: number) {
  let state = seed >>> 0;
  const next = (): number => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4_294_967_296;
  };
  return {
    next,
    int: (min: number, max: number) => min + Math.floor(next() * (max - min + 1)),
    pick: <T>(items: readonly T[]): T => items[Math.floor(next() * items.length)]!,
    chance: (p: number) => next() < p,
    shuffle<T>(items: readonly T[]): T[] {
      const out = [...items];
      for (let i = out.length - 1; i > 0; i--) {
        const j = Math.floor(next() * (i + 1));
        [out[i], out[j]] = [out[j]!, out[i]!];
      }
      return out;
    },
  };
}

type Rng = ReturnType<typeof seeded>;

const ALPHANUMERIC = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

function token(rng: Rng, length: number, alphabet = ALPHANUMERIC): string {
  return Array.from({ length }, () => rng.pick([...alphabet])).join("");
}

/** A time `minutes` before now, on a whole minute: keeps the newest mail in order. */
function minutesAgo(minutes: number): Date {
  const d = new Date(Date.now() - minutes * 60_000);
  d.setSeconds(0, 0);
  return d;
}

function escapeHtml(text: string): string {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function para(...lines: string[]): string {
  return lines.map((l) => `<p>${l}</p>`).join("");
}

/** "Wiśniewska-Kowalczyk" → "wisniewska.kowalczyk", for generated addresses. */
function asciiSlug(text: string): string {
  return text
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/[łŁ]/g, "l")
    .replace(/[øØ]/g, "o")
    .replace(/[æÆ]/g, "ae")
    .replace(/[ðÐ]/g, "d")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, ".")
    .replace(/^\.+|\.+$/g, "");
}

// --- Accounts --------------------------------------------------------------------

const LONG = "acct-long";
const J = "acct-j";
const JP = "acct-jp";

/** 92 characters without a single break opportunity: no spaces, hyphens or slashes. */
const ME_LONG: EmailAddress = {
  name: "Aleksandra Wiśniewska-Kowalczyk (Northwind Industries Holdings — Procurement & Vendor Operations, EMEA)",
  address:
    "aleksandra.wisniewska.kowalczyk.procurement@northwindindustriesholdingsinternational.example",
};
const ME_J: EmailAddress = { name: "J", address: "j@jo.example" };
const ME_JP: EmailAddress = { name: "山田 太郎", address: "taro.yamada@sample-shoji.example" };

const ME: Record<string, EmailAddress> = { [LONG]: ME_LONG, [J]: ME_J, [JP]: ME_JP };

const SYNC_ERROR =
  "IMAP sync failed for imap.jo.example:993 — TLS handshake error: certificate verify failed " +
  "(unable to get local issuer certificate). The certificate chain presented by the server " +
  "includes an intermediate certificate that expired on 2026-09-30 23:59:59 UTC, so no folders " +
  "have been synchronized since then. Check the server name and port in Settings, or ask your " +
  "mail provider to renew the certificate.";

function accounts(): Account[] {
  return [
    {
      id: LONG,
      kind: "microsoft",
      displayName: ME_LONG.name!,
      email: ME_LONG.address,
      color: "#8a4f7d",
      settings: { kind: "microsoft", tenantId: "worst-tenant", userId: "worst-user" },
      createdAt: iso(at(-2400, 9)),
      lastSyncedAt: iso(new Date()),
      syncError: null,
    },
    {
      id: J,
      kind: "imap",
      displayName: ME_J.name!,
      email: ME_J.address,
      color: "#8c6a10",
      settings: {
        kind: "imap",
        imapHost: "imap.jo.example",
        imapPort: 993,
        imapTls: "tls",
        smtpHost: "smtp.jo.example",
        smtpPort: 587,
        smtpTls: "startTls",
        username: "j",
      },
      createdAt: iso(at(-400, 9)),
      lastSyncedAt: iso(at(-14, 23, 59)),
      syncError: SYNC_ERROR,
    },
    {
      id: JP,
      kind: "microsoft",
      displayName: ME_JP.name!,
      email: ME_JP.address,
      color: "#b8434f",
      settings: { kind: "microsoft", tenantId: "worst-tenant-jp", userId: "worst-user-jp" },
      createdAt: iso(at(-30, 9)),
      lastSyncedAt: iso(new Date()),
      syncError: null,
    },
  ];
}

// --- People ----------------------------------------------------------------------

// First names cover A–Z, plus letters that don't fold to A–Z (Ł, Ø, Æ).
const FIRST_NAMES = (
  "Aleksandra Amara Bartholomew Beatriz Chiamaka Cormac Dmitri Dagny Eleanor Emeka Fatima " +
  "Florian Guðrún Giacomo Hiroshi Hamish Ingrid Ifeoma Javier Jo Kwame Kalinda Lakshmi " +
  "Leopold Maximilian Małgorzata Nkechi Niamh Oluwaseun Orsolya Pádraig Priyanka Quentin " +
  "Qiu Rosalind Rafael Siobhán Søren Thandiwe Tobias Ulrich Úna Valentina Viggo Wojciech " +
  "Wilhelmina Xiomara Xavier Yusuf Yetunde Zoë Zbigniew Łukasz Øyvind Æsa"
).split(" ");

const SURNAMES = (
  "Wiśniewska-Kowalczyk|Fitzgerald|Okonkwo-Adebayo|Ivanov|Montgomery III|Al-Rashid|" +
  "Jónsdóttir|Takahashi|Lindqvist|de la Fuente|Mensah|Subramanian|" +
  "von Hohenberg-Schwarzenfeld|Nguyễn|Adeyemi|Ó Briain|Dubois|Hargreaves-Smith|" +
  "Ní Bhraonáin|Ndlovu|Schäfer|Rossi|Brzęczyszczykiewicz|Castillo|Özdemir|O'Connor|Li|Ng|" +
  "Smith|Papadopoulos|da Silva Santos|Van der Merwe|MacGillivray|Øvrebø|Abdi|Wu|Ek|" +
  "Oyelaran-Oyeyinka|Szczepańska|D'Souza"
).split("|");

const DOMAINS = [
  "northwindindustriesholdings.example",
  "contoso-partners.example",
  "fabrikam.example",
  "woodgrove-legal.example",
  "tailspin-logistics.example",
  "litware.example",
  "wideworldimporters.example",
  "mailbox.example",
];

interface Person {
  first: string;
  last: string;
  address: EmailAddress;
}

/** Every first × last name pairing, shuffled once: a stable pool of unique people. */
function peoplePool(rng: Rng): Person[] {
  const all = FIRST_NAMES.flatMap((first) => SURNAMES.map((last) => ({ first, last })));
  return rng.shuffle(all).map(({ first, last }) => ({
    first,
    last,
    address: {
      name: `${first} ${last}`,
      address: `${asciiSlug(first)}.${asciiSlug(last)}@${rng.pick(DOMAINS)}`,
    },
  }));
}

/** 120 characters: a shared mailbox's display name from a large organization. */
const SENDER_120: EmailAddress = {
  name: "Northwind Industries Holdings International — Global Procurement, Vendor Onboarding and Supplier Compliance Office, EMEA",
  address: "supplier.compliance@northwindindustriesholdings.example",
};

const NO_NAME: EmailAddress = {
  name: null,
  address:
    "noreply.notifications.billing.accountsreceivable@accounts.northwindindustriesholdingsinternational.example",
};

const JO: EmailAddress = { name: "Jo", address: "jo@northwindindustriesholdings.example" };

// --- Mail ------------------------------------------------------------------------

/** [name, content type, bytes] */
type AttachmentSeed = [string, string, number];

interface MailSeed {
  folderId: string;
  from: EmailAddress | null;
  to?: EmailAddress[];
  cc?: EmailAddress[];
  bcc?: EmailAddress[];
  subject: string;
  html?: string;
  /** A plain-text body (no HTML part). */
  text?: string;
  received: Date;
  read?: boolean;
  flagged?: boolean;
  importance?: Importance;
  draft?: boolean;
  attachments?: AttachmentSeed[];
  conversationId?: string;
}

function messageFactory(prefix = "wm") {
  let seq = 0;
  return (accountId: string, seed: MailSeed): Message => {
    const id = `${prefix}-${++seq}`;
    const plain = (seed.text ?? seed.html ?? "")
      .replace(/<style[\s\S]*?<\/style>/g, "")
      .replace(/<[^>]+>/g, " ")
      .replace(/&nbsp;/g, " ")
      .replace(/&lt;/g, "<")
      .replace(/&gt;/g, ">")
      .replace(/&amp;/g, "&")
      .replace(/\s+/g, " ")
      .trim();
    const attachments = seed.attachments ?? [];
    return {
      id,
      accountId,
      folderId: seed.folderId,
      remoteId: id,
      conversationId: seed.conversationId ?? null,
      subject: seed.subject,
      from: seed.from,
      to: seed.to ?? [ME[accountId]!],
      cc: seed.cc ?? [],
      preview: plain.slice(0, 180),
      receivedAt: iso(seed.received),
      isRead: seed.read ?? true,
      isFlagged: seed.flagged ?? false,
      isDraft: seed.draft ?? false,
      hasAttachments: attachments.length > 0,
      importance: seed.importance ?? "normal",
      bcc: seed.bcc ?? [],
      replyTo: [],
      bodyHtml: seed.text !== undefined ? null : (seed.html ?? ""),
      bodyText: seed.text ?? plain,
      hasBlockedContent: false,
      attachments: attachments.map(([name, contentType, size], i) => ({
        id: `${id}-att-${i}`,
        name,
        contentType,
        size,
        isInline: false,
        contentId: null,
      })),
      internetMessageId: `<${id}@worst.example>`,
      inReplyTo: null,
      references: [],
    };
  };
}

const L = {
  inbox: "l-inbox",
  receipts: "l-receipts",
  drafts: "l-drafts",
  sent: "l-sent",
  archive: "l-archive",
  junk: "l-junk",
  trash: "l-trash",
  outbox: "l-outbox",
  notifications: "l-notifications",
  long: "l-long",
  empty: "l-empty",
  arabic: "l-arabic",
  pinned: "l-pinned",
};

/** Eight levels of nesting: Clients › Northwind … › Signed › Scanned originals. */
const DEEP_PATH = [
  "Clients",
  "Northwind Industries Holdings",
  "2026",
  "Q3",
  "Contracts",
  "Amendments",
  "Signed",
  "Scanned originals",
];
const deepId = (level: number) => `l-deep-${level}`;

/** Exactly 60 characters. */
const FOLDER_60 = "Vendor invoices awaiting three-way match — EMEA & APAC, 2026";

function folders(): Folder[] {
  return [
    folder(LONG, L.inbox, "Inbox", "inbox"),
    folder(LONG, L.receipts, "Receipts", "other", L.inbox),
    folder(LONG, L.drafts, "Drafts", "drafts"),
    folder(LONG, L.sent, "Sent Items", "sent"),
    folder(LONG, L.archive, "Archive", "archive"),
    folder(LONG, L.junk, "Junk Email", "junk"),
    folder(LONG, L.trash, "Deleted Items", "trash"),
    folder(LONG, L.outbox, "Outbox", "outbox"),
    folder(LONG, L.notifications, "Notifications", "other"),
    folder(LONG, L.long, FOLDER_60, "other"),
    folder(LONG, L.empty, "Newsletters (unsubscribed)", "other"),
    folder(LONG, L.arabic, "العملاء والموردون", "other"),
    folder(LONG, L.pinned, "📌 Follow up", "other"),
    ...DEEP_PATH.map((name, level) =>
      folder(LONG, deepId(level), name, "other", level === 0 ? null : deepId(level - 1)),
    ),
    folder(J, "j-inbox", "INBOX", "inbox"),
    folder(J, "j-drafts", "Drafts", "drafts"),
    folder(J, "j-sent", "Sent", "sent"),
    folder(J, "j-archive", "Archive", "archive"),
    folder(J, "j-junk", "Junk", "junk"),
    folder(J, "j-trash", "Trash", "trash"),
    // Microsoft names the well-known folders in the mailbox's language.
    folder(JP, "jp-inbox", "受信トレイ", "inbox"),
    folder(JP, "jp-drafts", "下書き", "drafts"),
    folder(JP, "jp-sent", "送信済みアイテム", "sent"),
    folder(JP, "jp-archive", "アーカイブ", "archive"),
    folder(JP, "jp-junk", "迷惑メール", "junk"),
    folder(JP, "jp-trash", "削除済みアイテム", "trash"),
    folder(JP, "jp-clients", "取引先", "other"),
    folder(
      JP,
      "jp-sample",
      "株式会社サンプル商事（購買部・経理部・法務部 共有）",
      "other",
      "jp-clients",
    ),
  ];
}

/** What the server reports per folder; the difference to the local mail is "remote". */
const SERVER_COUNTS: Record<string, { unread: number; total: number }> = {
  [L.inbox]: { unread: 12_345, total: 48_210 },
  [L.notifications]: { unread: 1_000, total: 1_000 },
  [L.receipts]: { unread: 999, total: 2_400 },
  [L.drafts]: { unread: 0, total: 1_204 },
  [L.archive]: { unread: 0, total: 251_877 },
  [L.junk]: { unread: 87, total: 87 },
  [deepId(3)]: { unread: 42, total: 310 },
};

const ATTACHMENTS_12: AttachmentSeed[] = [
  [
    "Northwind_Industries_Holdings_MSA_v14_consolidated_redlines_Legal_Procurement_Finance_FINAL_final_v2.docx",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    1_843_200,
  ],
  [
    "Schedule C — Pricing, volume rebates and payment terms (EMEA, effective 1 January 2027) — clean copy.pdf",
    "application/pdf",
    912_384,
  ],
  [
    "Q4 vendor consolidation workstreams, owners, milestones and RAID log (draft for steering committee).xlsx",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    3_407_872,
  ],
  [
    "Квартальный отчёт о закупках за III квартал 2026 года (окончательная редакция).pdf",
    "application/pdf",
    2_048_000,
  ],
  [
    "見積書_株式会社サンプル商事_購買部御中_2026年10月14日発行_有効期限30日.pdf",
    "application/pdf",
    480_256,
  ],
  ["تقرير_الموردين_الربع_الثالث_٢٠٢٦.pdf", "application/pdf", 733_184],
  [
    "Scan_2026-10-13_14-22-07_IMG_0001_IMG_0002_IMG_0003_compressed_OCR_searchable.pdf",
    "application/pdf",
    24_117_248,
  ],
  [
    "Site walk-through Building C (4K, uncompressed) — east wing, floors 1–4.mov",
    "video/quicktime",
    157_286_400,
  ],
  [
    "~$rthwind_Industries_Holdings_MSA_v14.docx",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    162,
  ],
  ["invoice.pdf", "application/pdf", 0],
  ["winmail.dat", "application/ms-tnef", 18_944],
  ["Supplier onboarding checklist.docx.zip", "application/zip", 52_428_800],
];

/** A 300-character Safe Links URL: no spaces, and a long token with no breaks at all. */
function safeLink(rng: Rng): string {
  const target = encodeURIComponent(
    "https://northwindindustriesholdings.sharepoint.example/sites/ProcurementEMEA/Shared Documents/Reconciliation/2026/Q3/nightly-mismatches.xlsx",
  );
  const head =
    `https://eur03.safelinks.protection.outlook.example/?url=${target}` +
    `&data=05%7C02%7C${token(rng, 32, "0123456789abcdef")}%7C0%7C0%7C638645432109876543%7CUnknown&sdata=`;
  const tail = "%3D&reserved=0";
  return head + token(rng, 300 - head.length - tail.length) + tail;
}

/** A plain-text report table wider than any reading pane. */
function asciiTable(rng: Rng): string {
  const vendors = [
    "Contoso Ltd.",
    "Fabrikam GmbH",
    "Tailspin Logistics sp. z o.o.",
    "Wide World Importers",
    "Litware Inc.",
    "Woodgrove Legal LLP",
  ];
  const money = (n: number) =>
    n.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
  const header = [
    "Region",
    "Vendor",
    "PO number",
    "Invoice",
    "Invoice date",
    "PO amount (EUR)",
    "Invoiced (EUR)",
    "Difference (EUR)",
    "Status",
    "Owner",
  ];
  const rows = Array.from({ length: 10 }, (_, i) => {
    const po = rng.int(1_000, 2_500_000) + rng.int(0, 99) / 100;
    const diff = i === 3 ? -1_234_567.89 : i % 3 === 0 ? rng.int(-9_000, 9_000) + 0.5 : 0;
    return [
      rng.pick(["EMEA", "APAC", "AMER", "LATAM"]),
      rng.pick(vendors),
      `PO-2026-${String(rng.int(1, 999_999)).padStart(6, "0")}`,
      `INV-${rng.int(100_000, 9_999_999)}`,
      dateOnly(-rng.int(1, 120)),
      money(po),
      money(po + diff),
      money(diff),
      diff === 0 ? "Matched" : "MISMATCH — needs review",
      rng.pick(["a.wisniewska", "j", "b.fitzgerald", "c.okonkwo"]),
    ];
  });
  const widths = header.map((h, c) => Math.max(h.length, ...rows.map((r) => r[c]!.length)));
  const line = `+${widths.map((w) => "-".repeat(w + 2)).join("+")}+`;
  const row = (cells: string[]) => `| ${cells.map((v, c) => v.padEnd(widths[c]!)).join(" | ")} |`;
  return [line, row(header), line, ...rows.map(row), line].join("\n");
}

/** A newsletter laid out as a fixed 900px table, as many marketing tools do. */
function wideHtmlTable(rng: Rng): string {
  const cells = (n: number, tag: "td" | "th") =>
    Array.from(
      { length: n },
      (_, i) =>
        `<${tag} width="${i === 0 ? 220 : 113}" style="border:1px solid #c8c8c8;padding:6px">${
          tag === "th"
            ? ["Supplier", "On time", "In full", "Quality", "Price", "Risk", "Score"][i]
            : i === 0
              ? escapeHtml(rng.pick(["Contoso Ltd.", "Fabrikam GmbH", "Litware Inc."]))
              : `${rng.int(61, 100)}%`
        }</${tag}>`,
    ).join("");
  const rows = Array.from({ length: 8 }, () => `<tr>${cells(7, "td")}</tr>`).join("");
  return (
    `<div style="width:900px;background:#1f3864;color:#ffffff;padding:24px;font:600 22px Arial,sans-serif">Supplier scorecard — October 2026</div>` +
    `<table width="900" cellpadding="0" cellspacing="0" style="width:900px;table-layout:fixed;border-collapse:collapse;font:13px Arial,sans-serif">` +
    `<tr style="background:#dfe6f3">${cells(7, "th")}</tr>${rows}</table>` +
    `<p style="width:900px;font:11px Arial,sans-serif;color:#666">You are receiving this because you are a supplier contact. Manage preferences in the supplier portal.</p>`
  );
}

interface Turn {
  from: EmailAddress;
  text: string;
  when: Date;
}

/** Each reply quotes the whole conversation before it, nesting one level deeper. */
function quotedThread(turns: Turn[]): string[] {
  const bodies: string[] = [];
  turns.forEach((turn, i) => {
    const prev = turns[i - 1];
    const quote = prev
      ? `<p>On ${prev.when.toUTCString()}, ${escapeHtml(`${prev.from.name} <${prev.from.address}>`)} wrote:</p>` +
        `<blockquote style="margin:0 0 0 4px;padding-left:12px;border-left:3px solid #c8c8c8">${bodies[i - 1]!}</blockquote>`
      : "";
    bodies.push(para(turn.text) + quote);
  });
  return bodies;
}

/** Plain-text quoting the way most clients do it: ">" deeper on every reply. */
function quotedPlain(turns: Turn[]): string {
  return turns.reduce((acc, turn, i) => {
    if (i === 0) return turn.text;
    const prev = turns[i - 1]!;
    const quoted = acc
      .split("\n")
      .map((l) => (l.startsWith(">") ? `>${l}` : l ? `> ${l}` : ">"))
      .join("\n");
    return `${turn.text}\n\nOn ${prev.when.toUTCString()}, ${prev.from.name} wrote:\n${quoted}`;
  }, "");
}

const TOPICS: [string, string][] = [
  ["Vendor onboarding: missing W-8BEN-E", "The supplier still hasn't sent the tax form."],
  ["PO approval needed", "This purchase order is waiting for your approval."],
  ["Contract renewal reminder", "The agreement renews automatically in 30 days."],
  ["Supplier audit findings", "Three minor findings, no major non-conformities."],
  ["Payment run results", "This week's payment run completed with two rejections."],
  ["Travel approval: Kraków supplier summit", "Flights and hotel are within policy."],
  ["Steering committee pre-read", "Pre-read for Friday is attached; please skim the risks."],
  ["Price increase notification", "We are writing to inform you of a 7.5% price adjustment."],
  ["Delivery delayed", "Shipment 4471 is held at customs pending paperwork."],
  ["Quarterly business review", "Proposed dates for the QBR are below."],
  ["Invoice dispute", "We disagree with the credit note amount."],
  ["Rate card 2027", "The updated rate card is attached for review."],
  ["Out of office", "I am out of the office with limited access to email."],
  ["New supplier request", "A business unit has requested a new supplier."],
  ["Expense report returned", "Your expense report was returned for a missing receipt."],
  ["Bank details change request", "Please confirm the new remittance details by phone."],
  ["Sustainability questionnaire", "Please complete the annual ESG questionnaire."],
  ["Meeting notes", "Notes and actions from today's call are below."],
];

const PREFIXES = ["", "", "", "Re: ", "RE: ", "Fwd: ", "FW: ", "RE: FW: ", "[EXTERNAL] "];

function mail(rng: Rng, people: Person[]): Message[] {
  const make = messageFactory();
  const m = (seed: MailSeed) => make(LONG, seed);
  const p = (i: number) => people[i]!.address;
  const lead = p(0);
  const analyst = p(1);
  const organizer = p(2);
  const out: Message[] = [];

  // The first screen of the inbox: one worst case per row.
  out.push(
    m({
      folderId: L.inbox,
      from: {
        name: "Building Management System",
        address: "bms-alerts@facilities.northwindindustriesholdings.example",
      },
      subject: "Scheduled fire alarm test — Building C, all floors",
      text: "Automated notice. This device's clock is not synchronized.",
      // A device with a broken clock: sorts above everything, forever.
      received: new Date(Date.UTC(2038, 0, 19, 3, 14, 7)),
      read: false,
      importance: "high",
    }),
    m({
      folderId: L.inbox,
      from: lead,
      cc: [analyst, organizer, JO],
      subject:
        "RE: FW: RE: RE: [EXTERNAL] ACTION REQUIRED: Updated Master Services Agreement (MSA) v14 — consolidated redlines from Legal, Procurement & Finance for the Northwind Industries Holdings EMEA renewal effective 1 January 2027; please review §4.2, §7.1(b) and Schedule C before Friday's steering committee",
      html: para(
        "Aleksandra,",
        "Consolidated redlines attached. Legal accepted most of Finance's comments; §7.1(b) is still open.",
      ),
      received: minutesAgo(5),
      read: false,
      flagged: true,
      importance: "high",
      attachments: [ATTACHMENTS_12[0]!],
    }),
    m({
      folderId: L.inbox,
      from: JO,
      subject: "",
      html: "",
      received: minutesAgo(12),
      read: false,
      attachments: [["IMG_4032.HEIC", "image/heic", 3_921_408]],
    }),
    m({
      folderId: L.inbox,
      from: NO_NAME,
      subject: "Invoice INV-2026-0098812 is overdue",
      html: para("Invoice INV-2026-0098812 (EUR 1,234,567.89) is 94 days overdue."),
      received: minutesAgo(20),
      read: false,
      importance: "high",
    }),
    m({
      folderId: L.inbox,
      from: SENDER_120,
      subject: "Supplier Code of Conduct: annual attestation due",
      html: para("Please complete your annual attestation in the supplier portal."),
      received: minutesAgo(31),
      read: false,
      flagged: true,
      importance: "low",
    }),
    m({
      folderId: L.inbox,
      from: { name: "Zoë 🧗‍♀️ Adeyemi", address: "zoe.adeyemi@mailbox.example" },
      subject: "🎉🎂🥳🎈🍰🎁👨‍👩‍👧‍👦🏳️‍🌈",
      html: para("🙌🙌🙌"),
      received: minutesAgo(44),
      read: false,
    }),
    m({
      folderId: L.inbox,
      from: { name: "مكتب السفر — قسم الحجوزات", address: "bookings@safar-travel.example" },
      subject: "تأكيد الحجز: رحلتك إلى دبي يوم الخميس ٢٣ أكتوبر",
      html:
        '<div dir="rtl">' +
        para(
          "مرحباً ألكسندرا،",
          "تم تأكيد حجزك. رقم الحجز: <b>QX7P2M</b>. يرجى الوصول إلى المطار قبل ثلاث ساعات من موعد الإقلاع.",
          "مع أطيب التحيات،<br>مكتب السفر",
        ) +
        "</div>",
      received: minutesAgo(58),
      read: false,
      attachments: [["تذكرة_إلكترونية_QX7P2M.pdf", "application/pdf", 96_412]],
    }),
    m({
      folderId: L.inbox,
      from: { name: "נועה כהן (Noa Cohen)", address: "noa.cohen@atlas-partners.example" },
      subject: "Re: עדכון לגבי Project Atlas v2.1 — 15% הנחה ל-Q4 (deadline: 31/10)",
      html: para(
        "היי Aleksandra, ה-PO החדש (PO-2026-004471) אושר, אבל ה-SLA עדיין 99.5% ולא 99.9%.",
      ),
      received: minutesAgo(75),
    }),
  );

  // 40 recipients, 12 attachments with long names and a 0-byte one.
  const crowd = people.slice(10, 50).map((x) => x.address);
  crowd[5] = { name: null, address: crowd[5]!.address };
  crowd[17] = ME_LONG;
  out.push(
    m({
      folderId: L.inbox,
      from: organizer,
      to: crowd.slice(0, 28),
      cc: crowd.slice(28),
      subject: "Q4 vendor consolidation kickoff — materials for all workstreams",
      html: para(
        "Hello all,",
        "Everything for Monday's kickoff is attached. Please read at least the RAID log and Schedule C.",
      ),
      received: minutesAgo(90),
      read: false,
      flagged: true,
      importance: "high",
      attachments: ATTACHMENTS_12,
    }),
  );

  // Plain text: a 300-character URL and a table wider than the reading pane.
  out.push(
    m({
      folderId: L.inbox,
      from: {
        name: "Ops Reporting Bot",
        address: "ops-reports@northwindindustriesholdings.example",
      },
      subject: "Nightly reconciliation report — 4 mismatches",
      text: [
        "Nightly three-way match finished with 4 mismatches.",
        "",
        "Full report:",
        safeLink(rng),
        "",
        asciiTable(rng),
        "",
        "-- ",
        "This message was generated automatically. Do not reply.",
      ].join("\n"),
      received: minutesAgo(110),
    }),
    m({
      folderId: L.inbox,
      from: { name: "Supplier Portal", address: "portal@supplier-portal.example" },
      subject: "Supplier scorecard — October 2026",
      html: wideHtmlTable(rng),
      received: minutesAgo(130),
      importance: "low",
    }),
  );

  // A seven-message thread where each reply quotes everything before it.
  const threadPeople = [lead, ME_LONG, analyst, organizer, lead, ME_LONG, analyst];
  const threadLines = [
    "Sharing the first cut of the Q4 forecast assumptions. Volume +4%, price +2.5%.",
    "Thanks. Price +2.5% feels light given the rate card. Can we model +4%?",
    "Modelled both. +4% pushes us over budget in APAC unless we drop the rebate tier.",
    "We can't drop the rebate tier — it's contractual until March.",
    "Then the only lever left is volume. I'll ask the BUs to re-forecast.",
    "Agreed. Let's freeze assumptions at Friday's committee.",
    "Frozen. Final numbers attached; nothing else changes this quarter.",
  ];
  const turns = threadPeople.map((from, i) => ({
    from,
    text: threadLines[i]!,
    when: minutesAgo(150 + (threadPeople.length - 1 - i) * 97),
  }));
  quotedThread(turns).forEach((html, i) => {
    const turn = turns[i]!;
    out.push(
      m({
        folderId: turn.from === ME_LONG ? L.sent : L.inbox,
        from: turn.from,
        to: [i === 0 ? ME_LONG : turns[i - 1]!.from],
        cc: [lead, analyst, organizer].filter((x) => x !== turn.from),
        subject: `${"RE: ".repeat(i)}Q4 forecast assumptions`,
        html,
        received: turn.when,
        read: i < threadPeople.length - 1,
        conversationId: "conv-forecast",
        attachments: i === threadPeople.length - 1 ? [ATTACHMENTS_12[2]!] : undefined,
      }),
    );
  });

  out.push(
    m({
      folderId: L.inbox,
      // Mail from a cron job: no From header at all.
      from: null,
      subject: "Cron <root@build-07> /usr/local/bin/backup.sh --full --verify",
      text: "rsync: [sender] write error: Broken pipe (32)\nrsync error: error in rsync protocol data stream (code 12) at io.c(823) [sender=3.2.7]",
      received: minutesAgo(170),
      read: false,
    }),
    m({
      folderId: L.inbox,
      from: analyst,
      subject: "Re: Re: Re: Re: Re: Re: Parking at Building C",
      text: quotedPlain(
        [analyst, JO, analyst, JO, analyst, JO, analyst].map((from, i) => ({
          from,
          text: [
            "Is anyone using the visitor spaces on Thursday?",
            "Yes, two suppliers.",
            "Both of them? All day?",
            "Morning only.",
            "Then I'll take one in the afternoon.",
            "Fine by me.",
            "Great, booked.",
          ][i]!,
          when: minutesAgo(200 + (6 - i) * 41),
        })),
      ),
      received: minutesAgo(200),
    }),
    m({
      folderId: L.inbox,
      from: { name: "The Procurement Digest", address: "digest@procurement-digest.example" },
      // Bcc-only newsletter: no visible recipients.
      to: [],
      subject: "This week: tariffs, freight rates and the 2027 rate card season",
      html: para("Freight rates fell for the sixth week in a row."),
      received: minutesAgo(230),
      importance: "low",
    }),
    m({
      folderId: L.inbox,
      from: { name: "Đặng Thị Ngọc Hân", address: "han.dang@tailspin-logistics.example" },
      subject: "Xác nhận đơn hàng số 2026-10-14 — giao hàng dự kiến thứ Năm",
      html: para("Kính gửi chị Aleksandra, đơn hàng đã được xác nhận."),
      received: minutesAgo(260),
      read: false,
    }),
    m({
      folderId: L.inbox,
      from: { name: "สมชาย ใจดี", address: "somchai@wideworldimporters.example" },
      subject: "ยืนยันการสั่งซื้อและกำหนดการจัดส่งสินค้า ไตรมาสที่ ๔",
      html: para("เรียนคุณอเล็กซานดรา ขอบคุณสำหรับคำสั่งซื้อครับ"),
      received: minutesAgo(290),
    }),
    m({
      folderId: L.inbox,
      from: { name: "ERP Workflow", address: "erp-workflow@northwindindustriesholdings.example" },
      subject:
        "PROC-REQ-2026-000123456789_NorthwindIndustriesHoldings_EMEA_VendorMasterDataChangeRequest_APPROVED",
      html: para("Your change request was approved."),
      received: minutesAgo(320),
    }),
    m({
      folderId: L.inbox,
      from: { name: "Supplier Portal", address: "portal@supplier-portal.example" },
      // A template placeholder that was never filled in, and markup in plain text.
      subject: "Welcome to the <Partner Name> supplier portal — R&D <b>access</b> & Q&A",
      html: para("Hello &lt;First Name&gt;, your account is ready."),
      received: minutesAgo(350),
    }),
  );

  // Everything else in the inbox: a year and a half of ordinary mail, with
  // every flagged × importance × read × attachment combination.
  const importances: Importance[] = ["normal", "high", "normal", "low", "normal", "normal"];
  for (let i = 0; i < 120; i++) {
    const sender = people[rng.int(0, 120)]!;
    const [topic, line] = TOPICS[i % TOPICS.length]!;
    out.push(
      m({
        folderId: L.inbox,
        from: sender.address,
        subject: `${rng.pick(PREFIXES)}${topic}`,
        html:
          para(`Hi Aleksandra,`, line, "Best regards,") +
          `<p>${escapeHtml(sender.address.name!)}</p>`,
        received: at(-1 - Math.floor(i * 4.5), rng.int(6, 22), rng.int(0, 59)),
        read: i > 40 || rng.chance(0.4),
        flagged: i % 5 === 0,
        importance: importances[i % importances.length]!,
        attachments:
          i % 4 === 0
            ? [[`${asciiSlug(topic)}-${i}.pdf`, "application/pdf", rng.int(1, 9_999_999)]]
            : undefined,
      }),
    );
  }

  out.push(
    m({
      folderId: L.inbox,
      from: { name: "IT Department", address: "it@northwindindustriesholdings.example" },
      subject: "Y2K readiness: final checklist before midnight",
      text: "All systems have been tested. Please do not leave your PC on over the holiday.",
      received: new Date(Date.UTC(1999, 11, 31, 22, 59)),
    }),
    m({
      folderId: L.inbox,
      from: { name: "MAILER-DAEMON", address: "mailer-daemon@legacy.example" },
      // A message with no Date header, parsed as the Unix epoch.
      subject: "Undelivered Mail Returned to Sender",
      text: "This is the mail system at host legacy.example.",
      received: new Date(0),
    }),
  );

  // Other folders.
  const filler = (folderId: string, count: number, unread: boolean, firstDay: number) => {
    for (let i = 0; i < count; i++) {
      const [topic, line] = rng.pick(TOPICS);
      out.push(
        m({
          folderId,
          from: rng.pick(people.slice(0, 80)).address,
          subject: topic,
          html: para(line),
          received: at(-firstDay - i * 3, rng.int(7, 19), rng.int(0, 59)),
          read: !unread,
        }),
      );
    }
  };
  filler(L.archive, 25, false, 200);
  filler(L.trash, 3, false, 2);
  filler(L.receipts, 10, true, 1);
  filler(L.long, 4, true, 3);
  filler(L.arabic, 2, false, 10);
  filler(L.pinned, 1, true, 4);
  DEEP_PATH.forEach((_, level) => filler(deepId(level), level === 7 ? 3 : 1, level >= 6, 20));
  for (let i = 0; i < 30; i++) {
    out.push(
      m({
        folderId: L.notifications,
        from: { name: "Jira", address: "jira@northwindindustriesholdings.example" },
        subject: `[JIRA] (PROC-${4_000 + i * 37}) ${rng.pick(TOPICS)[0]}`,
        html: para(`${rng.pick(people).address.name} changed the status to In Review.`),
        received: minutesAgo(400 + i * 53),
        read: false,
      }),
    );
  }
  out.push(
    m({
      folderId: L.drafts,
      from: ME_LONG,
      to: [],
      subject: "",
      html: "",
      received: minutesAgo(15),
      draft: true,
    }),
    m({
      folderId: L.drafts,
      from: ME_LONG,
      to: crowd.slice(0, 28),
      cc: crowd.slice(28),
      subject: "RE: Q4 vendor consolidation kickoff — materials for all workstreams",
      html: para("Thanks Wojciech, one question on the RAID log:"),
      received: minutesAgo(80),
      draft: true,
    }),
    m({
      folderId: L.outbox,
      from: ME_LONG,
      to: [lead],
      subject: "Site walk-through video (stuck in Outbox: 150 MB)",
      html: para("Video attached."),
      received: minutesAgo(25),
      attachments: [ATTACHMENTS_12[7]!],
    }),
    m({
      folderId: L.sent,
      from: ME_LONG,
      to: [SENDER_120],
      subject: "Attestation submitted",
      html: para("Done."),
      received: at(-2, 9, 12),
    }),
  );
  const spam: [string, string][] = [
    ["𝐂𝐨𝐧𝐠𝐫𝐚𝐭𝐮𝐥𝐚𝐭𝐢𝐨𝐧𝐬!!! 𝐘𝐨𝐮 𝐡𝐚𝐯𝐞 𝐰𝐨𝐧 €1,000,000", "Lucky Draw Ⓦⓘⓝⓝⓔⓡ"],
    ["Ｆｒｅｅ　ｇｉｆｔ　ｃａｒｄ　ｗａｉｔｉｎｇ　ｆｏｒ　ｙｏｕ", "Ｒｅｗａｒｄｓ"],
    ["Ваш аккаунт будет заблокирован через 24 часа", "Служба поддержки"],
    ["URGENT!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!", "CEO"],
  ];
  spam.forEach(([subject, name], i) =>
    out.push(
      m({
        folderId: L.junk,
        from: { name, address: `x${i}@${token(rng, 12).toLowerCase()}.example` },
        subject,
        html: para("Click here."),
        received: minutesAgo(60 * (i + 3)),
        read: false,
      }),
    ),
  );

  // The one-letter account: a single message, a one-letter subject.
  out.push(
    make(J, {
      folderId: "j-inbox",
      from: { name: "Jo", address: "jo@jo.example" },
      subject: "k",
      text: "k",
      received: at(-15, 8, 1),
      read: false,
    }),
  );

  // The Japanese account.
  const jp = (seed: MailSeed) => out.push(make(JP, seed));
  const jpSenders: EmailAddress[] = [
    { name: "佐藤 花子", address: "hanako.sato@sample-shoji.example" },
    { name: "株式会社サンプル商事 経理部", address: "keiri@sample-shoji.example" },
    { name: "王小明", address: "xiaoming.wang@fabrikam.example" },
    { name: "김민준", address: "minjun.kim@litware.example" },
    { name: "ｶﾌﾞｼｷｶﾞｲｼｬ ｻﾝﾌﾟﾙ", address: "legacy-system@sample-shoji.example" },
  ];
  const jpSubjects = [
    "【重要】2026年度下期 取引先別発注実績および請求書照合結果のご報告（第3四半期分・確定版）について",
    "会議日程の調整のお願い",
    "关于第四季度采购计划的通知",
    "4분기 구매 계획 안내",
    "ｾｲｷｭｳｼｮ ｿｳﾌ ﾉ ｵｼﾗｾ",
    // An encoded-word the sender's software never decoded.
    "=?ISO-2022-JP?B?GyRCJWEhPCVrJE4lRiU5JUgbKEI=?=",
    "Re: 見積書送付のお願い（至急）",
  ];
  jpSubjects.forEach((subject, i) =>
    jp({
      folderId: "jp-inbox",
      from: jpSenders[i % jpSenders.length]!,
      subject,
      html: para(
        "山田様",
        "いつもお世話になっております。ご確認のほど、よろしくお願いいたします。",
      ),
      received: minutesAgo(33 + i * 211),
      read: i > 2,
      flagged: i === 0,
      importance: i === 0 ? "high" : "normal",
    }),
  );
  jp({
    folderId: "jp-sample",
    from: jpSenders[1]!,
    subject: "請求書（2026年9月分）",
    html: para("請求書を添付いたします。"),
    received: at(-3, 10),
    attachments: [["請求書_2026年9月分_株式会社サンプル商事.pdf", "application/pdf", 288_412]],
  });
  return out;
}

/**
 * Years of order confirmations that only the server has: a server search for
 * "purchase order" finds more than a page of them (50), and more than two.
 */
function serverMail(): Message[] {
  const make = messageFactory("ws");
  const erp: EmailAddress = {
    name: "Northwind ERP (no reply)",
    address: "erp-noreply@northwindindustriesholdings.example",
  };
  return Array.from({ length: 130 }, (_, i) => {
    const order = `4500${String(118_200 + i * 7)}`;
    return make(LONG, {
      folderId: L.archive,
      from: erp,
      subject: `Purchase order ${order} confirmed`,
      text: `Purchase order ${order} was confirmed by the supplier. Delivery is scheduled within 14 days.`,
      received: at(-400 - i * 6, 7 + (i % 10), (i * 13) % 60),
    });
  });
}

/** Sets each folder's server-only counts so its totals match what the server reports. */
function setServerCounts(state: MockState): void {
  state.remoteCounts = {};
  const all = [...state.messages, ...(state.serverMessages ?? [])];
  for (const [folderId, target] of Object.entries(SERVER_COUNTS)) {
    const known = all.filter((m) => m.folderId === folderId);
    state.remoteCounts[folderId] = {
      unread: Math.max(0, target.unread - known.filter((m) => !m.isRead).length),
      total: Math.max(0, target.total - known.length),
    };
  }
}

// --- Calendar --------------------------------------------------------------------

function calendars(): Calendar[] {
  const cal = (
    id: string,
    accountId: string,
    name: string,
    color: string,
    extra: Partial<Calendar> = {},
  ): Calendar => ({
    id,
    accountId,
    remoteId: id,
    name,
    color,
    canEdit: true,
    isDefault: false,
    ...extra,
  });
  return [
    cal("wcal-default", LONG, "Calendar", "#8a4f7d", { isDefault: true }),
    cal(
      "wcal-long",
      LONG,
      "Northwind Industries Holdings — EMEA Procurement Steering Committee & Vendor Governance Board (shared, read-only)",
      "#2b7aa8",
      { canEdit: false },
    ),
    cal("wcal-team", LONG, "Procurement team", "#4d7a2e"),
    cal("wcal-room", LONG, "Konferenzraum „Zugspitze“ (Building C, 4.12, 12 seats)", "#5e6b7a"),
    cal("wcal-kids", LONG, "Kids' activities", "#fce100"),
    cal("wcal-birthdays", LONG, "Birthdays", "#b8434f", { canEdit: false }),
    cal("wcal-holidays", LONG, "Holidays in Poland", "#8c6a10", { canEdit: false }),
    cal("wcal-j", J, "Calendar", "#8c6a10", { isDefault: true, remoteId: "local:calendar" }),
    cal("wcal-jp", JP, "カレンダー", "#b8434f", { isDefault: true }),
    cal("wcal-jp-holidays", JP, "日本の祝日", "#0b7a75", { canEdit: false }),
  ];
}

/** Seven meetings that all start at 10:00 today: [calendar, title, minutes]. */
const SAME_TIME: [string, string, number][] = [
  ["wcal-default", "Vendor review: Contoso", 30],
  ["wcal-team", "Budget sync", 60],
  ["wcal-long", "Steering committee pre-read", 60],
  ["wcal-default", "Interview: Senior Buyer (panel)", 90],
  ["wcal-room", "Legal redlines walkthrough", 120],
  ["wcal-team", "Coffee with Jo", 45],
  ["wcal-default", "Supplier onboarding Q&A", 30],
];

const FORTY_WORD_TITLE =
  "Quarterly business review with Northwind Industries Holdings EMEA procurement, finance, legal and supplier quality teams to walk through vendor consolidation progress, contract renegotiation status, open audit findings, payment term changes and the agreed next steps for the 2027 renewal cycle";

function events(rng: Rng, people: Person[], cals: Calendar[]): CalendarEvent[] {
  const byId = new Map(cals.map((c) => [c.id, c]));
  let seq = 0;
  const ev = (
    calendarId: string,
    subject: string,
    start: Date,
    minutes: number,
    extra: Partial<CalendarEvent> = {},
  ): CalendarEvent => {
    const calendar = byId.get(calendarId)!;
    const id = `we-${++seq}`;
    return {
      id,
      accountId: calendar.accountId,
      calendarId,
      remoteId: id,
      subject,
      location: null,
      bodyHtml: null,
      bodyPreview: null,
      start: iso(start),
      end: iso(new Date(start.getTime() + minutes * 60_000)),
      isAllDay: false,
      organizer: ME[calendar.accountId]!,
      attendees: [],
      showAs: "busy",
      responseStatus: "organizer",
      isOnlineMeeting: false,
      onlineMeetingUrl: null,
      reminderMinutes: 15,
      isRecurring: false,
      isCancelled: false,
      ...extra,
    };
  };
  const allDayEv = (
    calendarId: string,
    subject: string,
    firstDay: number,
    days: number,
    extra: Partial<CalendarEvent> = {},
  ) =>
    ev(calendarId, subject, new Date(allDay(firstDay)), days * 24 * 60, {
      isAllDay: true,
      showAs: "free",
      reminderMinutes: null,
      ...extra,
    });
  const responses: ResponseStatus[] = ["accepted", "accepted", "tentative", "declined", "none"];
  const invite = (from: number, count: number): Attendee[] =>
    people.slice(from, from + count).map((x, i) => ({
      email: x.address,
      kind: i % 9 === 8 ? "optional" : "required",
      response: rng.pick(responses),
    }));
  const meeting = (organizer: Person, attendees: Attendee[]): Partial<CalendarEvent> => ({
    organizer: organizer.address,
    responseStatus: "accepted",
    attendees: [...attendees, { email: ME_LONG, kind: "required", response: "accepted" }],
  });

  const out: CalendarEvent[] = [];
  // A daily stand-up on weekdays, so the month view is never sparse.
  for (let d = -21; d <= 28; d++) {
    const day = at(d, 12).getDay();
    if (day === 0 || day === 6) continue;
    out.push(
      ev("wcal-team", "Procurement stand-up", at(d, 9, 30), 15, {
        isRecurring: true,
        ...meeting(people[3]!, invite(3, 5)),
      }),
    );
  }

  out.push(
    // Today: a 5-minute event, seven meetings starting at once, a zero-length
    // deadline, an untitled hold, a 40-word title and a night that crosses midnight.
    ev("wcal-default", "Badge photo", at(0, 9, 0), 5, { location: "Reception" }),
    ...SAME_TIME.map(([calendarId, subject, minutes], i) =>
      ev(calendarId, subject, at(0, 10, 0), minutes, {
        showAs: i % 3 === 2 ? "tentative" : "busy",
        ...(i % 2 ? meeting(people[i + 20]!, invite(i * 3, 4)) : {}),
      }),
    ),
    ev("wcal-default", "", at(0, 12, 30), 30, { showAs: "tentative" }),
    ev("wcal-long", FORTY_WORD_TITLE, at(0, 14, 0), 60, {
      location:
        "Konferenzraum „Zugspitze“ (Building C, 4th floor, east wing — access via the visitor lift; badge required after 18:00)",
      isOnlineMeeting: true,
      onlineMeetingUrl: `https://teams.microsoft.example/l/meetup-join/19%3ameeting_${token(rng, 120)}%40thread.v2/0?context=%7b%22Tid%22%3a%22${token(rng, 36)}%22%7d`,
      ...meeting(people[4]!, invite(40, 8)),
    }),
    ev("wcal-default", "Expense report deadline", at(0, 17, 0), 0, { showAs: "free" }),
    ev("wcal-team", "Data center cutover (APAC window)", at(0, 22, 0), 240, {
      location: "On-call bridge",
      ...meeting(people[5]!, invite(60, 3)),
    }),
    ev("wcal-kids", "Swimming 🏊", at(0, 16, 30), 60, { showAs: "free" }),
    ev("wcal-default", "عرض المورد: الخدمات اللوجستية", at(0, 15, 0), 45, {
      location: "قاعة الاجتماعات ٢",
    }),
    ev(
      "wcal-default",
      "Cancelled: Contract renewal sync with Fabrikam GmbH legal and finance",
      at(0, 11, 30),
      30,
      {
        isCancelled: true,
        ...meeting(people[6]!, invite(70, 3)),
      },
    ),
    // Stacked all-day events today, and one that spans three weeks.
    allDayEv(
      "wcal-team",
      "Northwind ERP migration freeze — no PO or vendor master changes",
      -6,
      21,
      {
        showAs: "busy",
      },
    ),
    allDayEv("wcal-default", "Office closed: Building C fire alarm testing", 0, 1, {
      showAs: "outOfOffice",
    }),
    allDayEv("wcal-birthdays", "Zoë Adeyemi's birthday 🎂", 0, 1),
    allDayEv("wcal-team", "Wojciech Brzęczyszczykiewicz — out of office (parental leave)", -1, 3),
    allDayEv("wcal-holidays", "Dzień Edukacji Narodowej", 0, 1),
    // Tomorrow: a 60-person all-hands, an untimed multi-day summit.
    ev("wcal-default", "All-hands: FY2027 planning", at(1, 15, 0), 60, {
      isOnlineMeeting: true,
      onlineMeetingUrl: "https://meet.example/all-hands",
      bodyHtml: para(
        "Agenda: FY2026 results, FY2027 priorities, organization changes, Q&A.",
        "Please submit questions in advance.",
      ),
      bodyPreview: "Agenda: FY2026 results, FY2027 priorities, organization changes, Q&A.",
      ...meeting(people[7]!, invite(80, 59)),
    }),
    ev("wcal-team", "Supplier summit, Kraków", at(1, 9, 0), (2 * 24 + 8) * 60, {
      location: "ICE Kraków Congress Centre, ul. Marii Konopnickiej 17, 30-302 Kraków, Poland",
      showAs: "outOfOffice",
    }),
  );

  // The day after tomorrow: fifteen meetings, for month view's "+N more".
  for (let i = 0; i < 15; i++) {
    const [topic] = TOPICS[i % TOPICS.length]!;
    out.push(
      ev(i % 3 ? "wcal-default" : "wcal-room", topic, at(2, 8, 0 + i * 35), 30, {
        ...(i % 2 ? meeting(people[i + 30]!, invite(i * 2, 3)) : {}),
      }),
    );
  }

  // The rest of the month: back-to-back room bookings and kids' activities.
  for (let d = -10; d <= 20; d += 3) {
    out.push(
      ev("wcal-room", `Booked by ${rng.pick(people).address.name}`, at(d, rng.int(8, 15)), 120),
      ev("wcal-kids", rng.pick(["Football", "Piano", "Ballet", "Chess club"]), at(d, 17), 60, {
        showAs: "free",
      }),
    );
  }

  out.push(
    ev("wcal-jp", "週次定例ミーティング（営業部・購買部・経理部 合同）", at(0, 10, 0), 60, {
      location: "本社 12階 第3会議室",
    }),
    ev("wcal-jp", "株式会社サンプル商事様 来訪", at(1, 14, 0), 90),
    allDayEv("wcal-jp-holidays", "スポーツの日", -2, 1),
  );
  return out;
}

// --- Contacts --------------------------------------------------------------------

/** About 2,000 characters of notes, as a long-standing account accumulates. */
function longNote(rng: Rng): string {
  const entries = [
    "Met at the Kraków supplier summit; prefers calls before 10:00 CET.",
    "Escalation path: her manager first, then the regional VP. Never skip the manager.",
    "Asked for invoices to be split by cost center from next quarter.",
    "Out on Fridays during school holidays. Deputy covers approvals.",
    "Allergic to shellfish — remember for the QBR dinner.",
    "Agreed a 2.5% early-payment discount verbally; not yet in the contract.",
    "Sensitive about the 2024 audit findings; frame questions around improvements.",
    "Uses a personal phone for urgent issues; work mobile goes to voicemail.",
    "Prefers PDFs over spreadsheets for anything that needs a signature.",
  ];
  const lines: string[] = [];
  let length = 0;
  for (let i = 0; length < 2_000; i++) {
    const line = `${dateOnly(-30 * (i + 1))}: ${entries[i % entries.length]} ${rng.pick([
      "Follow up next month.",
      "No action needed.",
      "Logged in the CRM.",
    ])}`;
    lines.push(line);
    length += line.length + 1;
  }
  return `${lines.join("\n").slice(0, 1_999)}…`;
}

function contacts(rng: Rng, people: Person[]): Contact[] {
  let seq = 0;
  const c = (accountId: string, displayName: string, extra: Partial<Contact> = {}): Contact => {
    const id = `wc-${++seq}`;
    return {
      id,
      accountId,
      remoteId: id,
      displayName,
      givenName: null,
      surname: null,
      emails: [],
      phones: [],
      company: null,
      jobTitle: null,
      department: null,
      address: null,
      birthday: null,
      notes: null,
      ...extra,
    };
  };
  const email = (address: string): EmailAddress => ({ name: null, address });
  const out: Contact[] = [
    // Only an email: auto-created from mail, the address is the name.
    c(LONG, "bartholomew.fitzgerald@northwind-industries-holdings.example", {
      emails: [email("bartholomew.fitzgerald@northwind-industries-holdings.example")],
    }),
    // Only a phone number, synced from a phone.
    c(LONG, "+48 601 234 567", { phones: [{ kind: "mobile", number: "+48 601 234 567" }] }),
    c(
      LONG,
      "María Fernanda de los Ángeles Guadalupe Hernández-Villaseñor y Rodríguez de la Fuente Ortiz-Cabrales",
      {
        givenName: "María Fernanda de los Ángeles Guadalupe",
        surname: "Hernández-Villaseñor y Rodríguez de la Fuente Ortiz-Cabrales",
        emails: [email("mf.hernandez.villasenor@fabrikam.example")],
        company: "Fabrikam Servicios Logísticos Integrales de México, S.A.P.I. de C.V.",
        jobTitle:
          "Senior Vice President, Global Strategic Sourcing, Indirect Procurement & Supplier Diversity",
        department: "Corporate Procurement Center of Excellence (CoE), Latin America & Caribbean",
      },
    ),
    c(LONG, "J", { emails: [email("j@jo.example")] }),
    c(LONG, "Jo", { emails: [JO.address].map(email), company: "Northwind Industries Holdings" }),
    c(LONG, "🦄✨🌈", { emails: [email("unicorn@mailbox.example")] }),
    c(LONG, "Mom ❤️", { phones: [{ kind: "home", number: "+48 12 345 67 89" }] }),
    c(LONG, "Bartholomew Fitzgerald", {
      givenName: "Bartholomew",
      surname: "Fitzgerald",
      emails: [
        "b.fitzgerald@northwindindustriesholdings.example",
        "bartholomew.fitzgerald@northwind-industries-holdings.example",
        "bart@fitzgerald-family.example",
        "bfitzgerald@contoso-partners.example",
        "bart.fitzgerald.procurement.emea.shared.mailbox@northwindindustriesholdings.example",
        "fitzgerald.b@woodgrove-legal.example",
        "bart.f@mailbox.example",
        "b.fitzgerald+invoices@northwindindustriesholdings.example",
        "bartholomew@fitzgerald.example",
        "bf@litware.example",
      ].map(email),
      phones: (
        [
          ["mobile", "+44 7700 900123"],
          ["mobile", "+48 601 987 654"],
          ["business", "+44 20 7946 0958 ext. 4471"],
          ["business", "+1 (206) 555-0123"],
          ["home", "+44 113 496 0000"],
          ["other", "+353 1 555 0199"],
          ["business", "+49 30 901820-4471"],
          ["other", "*31#+44 7700 900456"],
          ["mobile", "+81 90-1234-5678"],
          ["business", "+971 4 555 0100"],
        ] as [Phone["kind"], string][]
      ).map(([kind, number]) => ({ kind, number })),
      company: "Northwind Industries Holdings",
      jobTitle: "Head of Vendor Governance",
      address:
        "Northwind Industries Holdings International Ltd.\nBuilding C, 4th floor, east wing\n1 Canal Street\nLondon EC2A 4NE\nUnited Kingdom",
      birthday: "1604-05-17",
    }),
    c(LONG, "Chiamaka Okonkwo-Adebayo", {
      emails: [email("chiamaka.okonkwo@northwindindustriesholdings.example")],
      company: "Northwind Industries Holdings",
      notes: longNote(rng),
      birthday: "1996-02-29",
    }),
    // Names that start with digits or symbols, or letters that don't fold to A–Z.
    ...[
      "24/7 Locksmith (Building C)",
      "3rd-floor reception desk",
      "007 Courier Services",
      "@home Wi-Fi support",
      "#ProcurementHelp (shared mailbox)",
      "(unknown caller)",
      "_old import — do not use",
      "¡Hola! Café Barcelona",
      '"Bart" Fitzgerald',
      "'Nana' Okafor",
      "Łukasz Brzęczyszczykiewicz",
      "Øyvind Øvrebø",
      "Ærøskøbing Ferry Office",
    ].map((name, i) =>
      c(LONG, name, {
        emails: i % 2 ? [email(`${asciiSlug(name) || "contact"}@mailbox.example`)] : [],
        phones: [{ kind: "business", number: `+44 20 7946 ${String(1000 + i * 37).slice(-4)}` }],
      }),
    ),
  ];

  // Non-Latin names, shared between both Microsoft accounts.
  const world: [string, string, string][] = [
    [JP, "山田 花子", "hanako.yamada@sample-shoji.example"],
    [JP, "佐藤 健太郎", "kentaro.sato@sample-shoji.example"],
    [JP, "王小明", "xiaoming.wang@fabrikam.example"],
    [JP, "陳大文", "taiman.chan@litware.example"],
    [JP, "김민준", "minjun.kim@litware.example"],
    [JP, "ｶﾌﾞｼｷｶﾞｲｼｬ ｻﾝﾌﾟﾙ", "legacy-system@sample-shoji.example"],
    [LONG, "ليلى حسن", "layla.hassan@safar-travel.example"],
    [LONG, "عمر خالد السيد", "omar.elsayed@wideworldimporters.example"],
    [LONG, "Александра Владимировна Кузнецова", "a.kuznetsova@tailspin-logistics.example"],
    [LONG, "Дмитрий Иванов", "d.ivanov@tailspin-logistics.example"],
    [LONG, "נועה כהן", "noa.cohen@atlas-partners.example"],
    [LONG, "สมชาย ใจดี", "somchai@wideworldimporters.example"],
    [LONG, "प्रिया शर्मा", "priya.sharma@litware.example"],
    [LONG, "Γιώργος Παπαδόπουλος", "g.papadopoulos@fabrikam.example"],
    [LONG, "Đặng Thị Ngọc Hân", "han.dang@tailspin-logistics.example"],
    [LONG, "ნინო ბერიძე", "nino.beridze@mailbox.example"],
    [LONG, "山田 太郎", ME_JP.address],
  ];
  out.push(
    ...world.map(([accountId, name, address]) => c(accountId, name, { emails: [email(address)] })),
  );

  // Fill up to 300 with ordinary people: most fields optional, some missing.
  const companies = [
    "Contoso Ltd.",
    "Fabrikam GmbH",
    "Tailspin Logistics sp. z o.o.",
    "Woodgrove Legal LLP",
    "Litware Inc.",
    "Northwind Industries Holdings",
  ];
  const titles = ["Buyer", "Category Manager", "Account Director", "AP Clerk", "General Counsel"];
  for (let i = 0; out.length < 300; i++) {
    const person = people[i]!;
    const work = rng.chance(0.7);
    out.push(
      c(i % 15 === 0 ? JP : LONG, person.address.name!, {
        givenName: person.first,
        surname: person.last,
        emails: rng.chance(0.9) ? [person.address] : [],
        phones: rng.chance(0.5)
          ? [
              {
                kind: rng.pick(["mobile", "business", "home"] as const),
                number: `+44 7700 9${rng.int(10_000, 99_999)}`,
              },
            ]
          : [],
        company: work ? rng.pick(companies) : null,
        jobTitle: work && rng.chance(0.6) ? rng.pick(titles) : null,
        birthday: rng.chance(0.2)
          ? `19${rng.int(50, 99)}-0${rng.int(1, 9)}-1${rng.int(0, 9)}`
          : null,
      }),
    );
  }
  return out;
}

// --- Tasks -----------------------------------------------------------------------

const VERBS = (
  "Review|Approve|Chase|Reconcile|Send|Update|Schedule|Draft|Renew|Cancel|Book|Call|Sign|" +
  "File|Prepare|Follow up on|Escalate"
).split("|");

const OBJECTS = [
  "Contoso invoice INV-2026-0098812",
  "Fabrikam MSA redlines",
  "Q4 vendor scorecards",
  "travel expenses for Kraków",
  "the rate card for 2027",
  "PO-2026-004471",
  "the supplier code of conduct attestation",
  "Litware credit note",
  "Tailspin customs paperwork",
  "the steering committee pre-read",
  "insurance certificates",
  "W-8BEN-E from Wide World Importers",
  "bank details change for Woodgrove Legal",
  "ESG questionnaire",
  "QBR dates with Contoso",
];

const TITLE_200 =
  "Reconcile the September vendor statements for Northwind Industries Holdings EMEA against the ERP open-items report, flag every invoice older than 90 days and send an exception list to Accounts Payable";

const MULTI_PARAGRAPH_NOTE = [
  "Context: the September statements from the top 40 vendors arrived late, and three of them use a new format that the ERP import can't read.",
  "Steps:\n1. Export open items from the ERP (vendor ledger, company codes 1000, 1100 and 4400).\n2. Match statement lines by invoice number, then by amount ± 0.50 EUR.\n3. Anything unmatched after 90 days goes on the exception list.",
  "Watch out for:\n- Credit notes booked as negative invoices\n- Duplicate invoice numbers across company codes\n- Contoso's statement is in GBP, everything else in EUR",
  "Template: https://northwindindustriesholdings.sharepoint.example/sites/ProcurementEMEA/Shared%20Documents/Templates/VendorStatementReconciliation_v7.xlsx",
  "Previous owner: Wojciech (on parental leave until January). Ask Jo for access to the AP shared mailbox.",
].join("\n\n");

const LONG_LIST_NAME =
  "Northwind Industries Holdings — FY2027 vendor consolidation, contract renegotiation & supplier offboarding checklist (EMEA)";

function taskData(rng: Rng): { taskLists: TaskList[]; tasks: Task[] } {
  const list = (id: string, accountId: string, name: string, isDefault = false): TaskList => ({
    id,
    accountId,
    remoteId: id,
    name,
    isDefault,
  });
  const taskLists = [
    list("wtl-tasks", LONG, "Tasks", true),
    list("wtl-long", LONG, LONG_LIST_NAME),
    list("wtl-done", LONG, "Q3 close"),
    list("wtl-empty", LONG, "Someday / maybe"),
    list("wtl-emoji", LONG, "🏠 Home renovation 🛠️"),
    list("wtl-j", J, "Tasks", true),
    list("wtl-jp", JP, "タスク", true),
    list("wtl-jp-shopping", JP, "買い物リスト"),
  ];

  let seq = 0;
  const task = (listId: string, title: string, extra: Partial<Task> = {}): Task => {
    const id = `wt-${++seq}`;
    const accountId = taskLists.find((l) => l.id === listId)!.accountId;
    const created = iso(at(-rng.int(1, 60), 9));
    return {
      id,
      accountId,
      listId,
      remoteId: id,
      title,
      notes: null,
      status: "notStarted",
      importance: "normal",
      dueDate: null,
      reminderAt: null,
      completedAt: null,
      createdAt: created,
      updatedAt: created,
      ...extra,
    };
  };
  const done = (daysAgo: number): Partial<Task> => ({
    status: "completed",
    completedAt: iso(at(-daysAgo, 17)),
  });

  const tasks: Task[] = [
    task("wtl-long", TITLE_200, { dueDate: dateOnly(1), importance: "high" }),
    task("wtl-long", "Collect signed amendments from all 40 vendors", {
      notes: MULTI_PARAGRAPH_NOTE,
      dueDate: dateOnly(-3),
    }),
    task("wtl-long", "Renew the vendor risk insurance certificate", {
      // Overdue by years.
      dueDate: "2019-03-14",
      importance: "high",
      createdAt: "2019-01-07T09:00:00.000Z",
    }),
    task("wtl-long", "Offboard Litware Inc. from the supplier portal", {
      dueDate: dateOnly(-1_100),
    }),
    task("wtl-long", "Archive 2016 contracts", { dueDate: "2017-12-31", importance: "low" }),
    task("wtl-long", "Pay off the warehouse lease", { dueDate: "2051-06-30" }),
    task("wtl-emoji", "🎨 Pick paint colours", { dueDate: dateOnly(5) }),
    task("wtl-emoji", "🚿🚿🚿", { importance: "high" }),
    task("wtl-j", "Gym", { dueDate: dateOnly(0) }),
    task("wtl-jp", "株式会社サンプル商事様への見積書送付（第3四半期分・至急）", {
      dueDate: dateOnly(0),
      importance: "high",
    }),
    task("wtl-jp", "経費精算", { dueDate: dateOnly(-400) }),
    task("wtl-jp", "会議室の予約", done(2)),
    task("wtl-jp-shopping", "牛乳"),
    task("wtl-jp-shopping", "卵"),
  ];

  // A list where everything is done.
  for (let i = 0; i < 12; i++) {
    tasks.push(task("wtl-done", `${rng.pick(VERBS)} ${rng.pick(OBJECTS)}`, done(i + 1)));
  }

  // 200 tasks in the default list: years overdue to years ahead, some done.
  for (let i = 0; i < 200; i++) {
    const status = rng.chance(0.2) ? "completed" : rng.chance(0.1) ? "inProgress" : "notStarted";
    const due = rng.chance(0.75) ? dateOnly(rng.int(-2_000, 900)) : null;
    tasks.push(
      task("wtl-tasks", `${rng.pick(VERBS)} ${rng.pick(OBJECTS)}`, {
        status,
        dueDate: due,
        importance: rng.chance(0.15) ? "high" : rng.chance(0.1) ? "low" : "normal",
        notes: rng.chance(0.2) ? rng.pick(TOPICS)[1] : null,
        ...(status === "completed" ? { completedAt: iso(at(-rng.int(0, 400), 16)) } : {}),
      }),
    );
  }
  return { taskLists, tasks };
}

// --- State -----------------------------------------------------------------------

export function createWorstState(): MockState {
  const people = peoplePool(seeded(1));
  const cals = calendars();
  const { taskLists, tasks } = taskData(seeded(5));
  const state: MockState = {
    accounts: accounts(),
    folders: folders(),
    messages: mail(seeded(2), people),
    calendars: cals,
    events: events(seeded(3), people, cals),
    contacts: contacts(seeded(4), people),
    taskLists,
    tasks,
    serverMessages: serverMail(),
  };
  setServerCounts(state);
  recount(state);
  return state;
}
