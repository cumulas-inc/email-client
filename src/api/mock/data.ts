// Demo data for the in-memory backend. Dates are relative to "now" so the
// demo always looks current. All people and companies are fictional.

import type {
  Account,
  AccountSignature,
  Attendee,
  Calendar,
  CalendarEvent,
  Contact,
  EmailAddress,
  Folder,
  FolderRole,
  LabelOption,
  LabelPolicy,
  Message,
  MessageLabel,
  Protection,
  Task,
  TaskList,
} from "../types";

export interface MockState {
  accounts: Account[];
  folders: Folder[];
  messages: Message[];
  calendars: Calendar[];
  events: CalendarEvent[];
  contacts: Contact[];
  taskLists: TaskList[];
  tasks: Task[];
  /**
   * Messages a folder holds on the server beyond the ones synced locally, as
   * large mailboxes report: added to the counts `recount` derives.
   */
  remoteCounts?: Record<string, { unread: number; total: number }>;
  /**
   * Messages only the servers have, older than the synced window: a server
   * search finds them and brings them onto the device (into `messages`).
   * The folders' counts include them already, as the servers' counts do.
   */
  serverMessages?: Message[];
  /** Signatures the demo starts with; other accounts start without any. */
  signatures?: AccountSignature[];
}

const WORK = "acct-work";
const HOME = "acct-home";

export const ME_WORK: EmailAddress = { name: "Alex Morgan", address: "alex.morgan@acme.example" };
export const ME_HOME: EmailAddress = { name: "Alex Morgan", address: "alex@morgan-family.example" };

const people = {
  priya: { name: "Priya Raman", address: "priya.raman@acme.example" },
  jonas: { name: "Jonas Becker", address: "jonas.becker@acme.example" },
  mei: { name: "Mei Tanaka", address: "mei.tanaka@acme.example" },
  sam: { name: "Sam Okafor", address: "sam.okafor@acme.example" },
  lucia: { name: "Lucía Fernández", address: "lucia@northwind-design.example" },
  it: { name: "Acme IT Service Desk", address: "servicedesk@acme.example" },
  hr: { name: "People Team", address: "people@acme.example" },
  dana: { name: "Dana Whitfield", address: "dana.whitfield@acme.example" },
  travel: { name: "Skyway Travel", address: "bookings@skyway-travel.example" },
  news: { name: "The Linux Weekly Digest", address: "digest@linuxweekly.example" },
  mom: { name: "Mom", address: "helen.morgan@mailbox.example" },
  ben: { name: "Ben Morgan", address: "ben.morgan@mailbox.example" },
  bank: { name: "Harbor Bank", address: "no-reply@harborbank.example" },
  club: { name: "Riverside Climbing Club", address: "hello@riversideclimbing.example" },
} satisfies Record<string, EmailAddress>;

export function at(daysFromToday: number, hours: number, minutes = 0): Date {
  const d = new Date();
  d.setDate(d.getDate() + daysFromToday);
  d.setHours(hours, minutes, 0, 0);
  return d;
}

export function iso(d: Date): string {
  return d.toISOString();
}

/** Today's demo mail has arrived by this time of day; none of it is later. */
export const TODAYS_MAIL_BY = { hours: 10, minutes: 0 };

/**
 * When a demo message arrived: `at` itself, except for today's mail before
 * {@link TODAYS_MAIL_BY}, when it's spread over the part of the day that has
 * passed, in the same order. So no message arrives in the future, whatever
 * the time, and today's mail is still today's.
 */
export function receivedBy(at: Date, now = new Date()): Date {
  const midnight = new Date(now);
  midnight.setHours(0, 0, 0, 0);
  const by = new Date(midnight);
  by.setHours(TODAYS_MAIL_BY.hours, TODAYS_MAIL_BY.minutes);
  if (now >= by || at < midnight) return at;
  const share = (now.getTime() - midnight.getTime()) / (by.getTime() - midnight.getTime());
  const spread =
    midnight.getTime() + (Math.min(at.getTime(), by.getTime()) - midnight.getTime()) * share;
  return new Date(spread);
}

/** Midnight UTC of a local calendar day, as all-day events are stored. */
export function allDay(daysFromToday: number): string {
  const d = at(daysFromToday, 0);
  return new Date(Date.UTC(d.getFullYear(), d.getMonth(), d.getDate())).toISOString();
}

export function dateOnly(daysFromToday: number): string {
  const d = at(daysFromToday, 12);
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${d.getFullYear()}-${m}-${day}`;
}

export function folder(
  accountId: string,
  id: string,
  name: string,
  role: FolderRole,
  parentId: string | null = null,
): Folder {
  return {
    id,
    accountId,
    remoteId: id,
    parentId,
    name,
    role,
    unreadCount: 0,
    totalCount: 0,
  };
}

function para(...lines: string[]): string {
  return lines.map((l) => `<p>${l}</p>`).join("");
}

function signature(name: string, title?: string): string {
  return `<p style="color:#716c63;margin-top:16px">— ${name}${title ? `<br>${title}` : ""}</p>`;
}

let messageSeq = 0;

interface MessageSeed {
  /** A fixed id instead of the next msg-N. */
  id?: string;
  folderId: string;
  from: EmailAddress;
  to?: EmailAddress[];
  cc?: EmailAddress[];
  subject: string;
  html?: string;
  /** A plain-text body (no HTML part). */
  plainText?: string;
  received: Date;
  read?: boolean;
  flagged?: boolean;
  importance?: "low" | "normal" | "high";
  attachments?: { name: string; contentType: string; size: number }[];
  draft?: boolean;
}

function message(accountId: string, me: EmailAddress, seed: MessageSeed): Message {
  messageSeq += 1;
  const id = seed.id ?? `msg-${messageSeq}`;
  const text = (seed.plainText ?? seed.html ?? "")
    .replace(/<style[\s\S]*?<\/style>/g, "")
    .replace(/<[^>]+>/g, " ")
    .replace(/&nbsp;/g, " ")
    .replace(/&amp;/g, "&")
    .replace(/\s+/g, " ")
    .trim();
  return {
    id,
    accountId,
    folderId: seed.folderId,
    remoteId: id,
    conversationId: null,
    subject: seed.subject,
    from: seed.from,
    to: seed.to ?? [me],
    cc: seed.cc ?? [],
    preview: text.slice(0, 180),
    receivedAt: iso(receivedBy(seed.received)),
    isRead: seed.read ?? true,
    isFlagged: seed.flagged ?? false,
    isDraft: seed.draft ?? false,
    hasAttachments: (seed.attachments?.length ?? 0) > 0,
    importance: seed.importance ?? "normal",
    bcc: [],
    replyTo: [],
    bodyHtml: seed.plainText !== undefined ? null : (seed.html ?? ""),
    bodyText: seed.plainText ?? text,
    hasBlockedContent: false,
    attachments: (seed.attachments ?? []).map((a, i) => ({
      id: `${id}-att-${i}`,
      name: a.name,
      contentType: a.contentType,
      size: a.size,
      isInline: false,
      contentId: null,
    })),
    internetMessageId: `<${id}@demo.example>`,
    inReplyTo: null,
    references: [],
    protection: null,
    label: null,
    requested: null,
  };
}

function workMessages(): Message[] {
  const m = (seed: MessageSeed) => message(WORK, ME_WORK, seed);
  return [
    m({
      folderId: "w-inbox",
      from: people.priya,
      cc: [people.jonas],
      subject: "Atlas launch checklist — final review today",
      html:
        para(
          "Hi Alex,",
          "I've folded everyone's comments into the launch checklist. Could you take a last look at the <b>rollback plan</b> section before our 2pm review? The open items are:",
        ) +
        "<ul><li>Confirm the on-call rotation for launch week</li><li>Sign-off from Security on the data retention change</li><li>Customer comms draft (Mei has it)</li></ul>" +
        para("Thanks!") +
        signature("Priya", "Program Manager, Atlas"),
      received: at(0, 9, 42),
      read: false,
      flagged: true,
      importance: "high",
      attachments: [
        {
          name: "Atlas-launch-checklist-v7.docx",
          contentType: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
          size: 184_320,
        },
      ],
    }),
    m({
      folderId: "w-inbox",
      from: people.it,
      subject: "Scheduled maintenance: VPN gateway, Saturday 06:00–08:00",
      html:
        para(
          "Hello,",
          "The primary VPN gateway will be unavailable on <b>Saturday between 06:00 and 08:00</b> while we apply security updates. The secondary gateway will remain available.",
          "No action is required.",
        ) + signature("Acme IT Service Desk"),
      received: at(0, 8, 15),
      read: false,
    }),
    m({
      folderId: "w-inbox",
      from: people.mei,
      subject: "Customer announcement draft for Atlas",
      html:
        para(
          "Alex — first draft of the customer announcement is attached. I kept it short; happy to expand the FAQ if you think it's needed.",
          "Can you check the pricing paragraph? I wasn't sure whether we're announcing the migration credits publicly.",
        ) + signature("Mei"),
      received: at(0, 7, 58),
      read: false,
      attachments: [
        { name: "Atlas-announcement-draft.pdf", contentType: "application/pdf", size: 412_005 },
      ],
    }),
    m({
      folderId: "w-inbox",
      from: people.jonas,
      subject: "Re: Load test results",
      html:
        para(
          "Numbers from last night's run look good: p95 latency stayed under 180 ms up to 4× expected peak. The one regression was the export endpoint — I've opened a ticket.",
          'Full report on the <a href="https://wiki.acme.example/atlas/load-tests">wiki</a>.',
        ) +
        `<blockquote style="border-left:3px solid #e1ddd5;margin:0;padding-left:12px;color:#716c63">On Monday, Alex Morgan wrote:<br>Can we get a load test in before the review?</blockquote>` +
        signature("Jonas"),
      received: at(-1, 17, 31),
    }),
    m({
      folderId: "w-inbox",
      from: people.dana,
      to: [ME_WORK, people.priya, people.jonas, people.mei, people.sam],
      cc: [{ name: "Engineering Leads", address: "eng-leads@acme.example" }],
      subject: "Quarterly planning: please add your team's top three bets",
      html:
        para(
          "Hi all,",
          "Planning for next quarter kicks off next week. Please add your team's top three bets to the shared planning doc by Friday, with a rough size (S/M/L) and the metric you expect to move.",
          "Thanks for keeping it brief — we'll go deeper in the review.",
        ) + signature("Dana Whitfield", "VP Engineering"),
      received: at(-1, 11, 5),
      flagged: true,
    }),
    m({
      folderId: "w-inbox",
      from: people.lucia,
      subject: "Updated icon set and onboarding illustrations",
      html:
        para(
          "Hello Alex,",
          "Here are the revised illustrations for onboarding, plus the updated icon set in SVG. I adjusted the contrast on the dark theme variants as discussed.",
          "Let me know if anything needs another pass.",
        ) + signature("Lucía Fernández", "Northwind Design Studio"),
      received: at(-2, 15, 20),
      attachments: [
        { name: "onboarding-illustrations.zip", contentType: "application/zip", size: 8_412_220 },
        { name: "icons-v3.svg", contentType: "image/svg+xml", size: 52_310 },
      ],
    }),
    m({
      folderId: "w-inbox",
      from: people.hr,
      subject: "Reminder: benefits enrollment closes Friday",
      html:
        para(
          "This is a friendly reminder that open enrollment for next year's benefits closes this <b>Friday at 17:00</b>.",
          "If you don't make any changes, your current elections will roll over.",
        ) + signature("People Team"),
      received: at(-3, 10, 0),
    }),
    m({
      folderId: "w-inbox",
      from: people.sam,
      subject: "Lunch on Thursday?",
      html:
        para("Hey! Want to grab lunch Thursday? New ramen place opened round the corner.") +
        signature("Sam"),
      received: at(-3, 12, 41),
    }),
    m({
      folderId: "w-inbox",
      from: people.news,
      subject: "This week: GNOME 51 lands, kernel 7.2 merge window, and more",
      html: `<table width="100%" cellpadding="0" cellspacing="0" style="font-family:Georgia,serif;background:#f7f4ee"><tr><td align="center" style="padding:24px">
<img src="https://images.linuxweekly.example/header.png" width="560" alt="The Linux Weekly Digest">
<h1 style="font-size:24px;color:#1b1b1b">GNOME 51 lands with a refreshed Files app</h1>
<p style="font-size:16px;line-height:1.5;color:#333;max-width:520px">This week's release brings faster thumbnails, a new search experience and better portal support for sandboxed apps.</p>
<img src="https://track.linuxweekly.example/open.gif" width="1" height="1" alt="">
</td></tr></table>`,
      received: at(-4, 6, 30),
      importance: "low",
    }),
    m({
      folderId: "w-inbox",
      from: people.travel,
      subject: "Your trip to Lisbon: booking confirmed",
      html: para(
        "Your booking is confirmed.",
        "<b>Outbound:</b> Tue 09:10 → 11:45<br><b>Return:</b> Fri 18:30 → 21:05",
        "Booking reference: <b>QX7P2M</b>",
      ),
      received: at(-6, 18, 12),
      attachments: [{ name: "e-ticket-QX7P2M.pdf", contentType: "application/pdf", size: 96_412 }],
    }),
    m({
      folderId: "w-inbox",
      from: people.priya,
      subject: "Atlas: risk register updated",
      html:
        para("Added two new risks (vendor SLA, translation timing). Both are amber for now.") +
        signature("Priya"),
      received: at(-9, 14, 2),
    }),
    m({
      folderId: "w-inbox",
      from: people.jonas,
      subject: "Code freeze dates",
      html:
        para(
          "Proposed code freeze is two weeks before launch. Shout if that clashes with anything.",
        ) + signature("Jonas"),
      received: at(-12, 9, 30),
    }),
    m({
      folderId: "w-atlas",
      from: people.mei,
      subject: "Atlas naming research summary",
      html: para("Summary of the naming research is in the attached deck.") + signature("Mei"),
      received: at(-15, 16, 0),
      attachments: [
        {
          name: "naming-research.pptx",
          contentType: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
          size: 2_310_442,
        },
      ],
    }),
    m({
      folderId: "w-sent",
      from: ME_WORK,
      to: [people.jonas],
      subject: "Load test results",
      html: para("Can we get a load test in before the review?") + signature("Alex"),
      received: at(-2, 9, 12),
    }),
    m({
      folderId: "w-sent",
      from: ME_WORK,
      to: [people.dana],
      subject: "Re: Headcount request",
      html: para("Thanks Dana — I'll send the updated plan by Thursday.") + signature("Alex"),
      received: at(-5, 16, 48),
    }),
    m({
      folderId: "w-drafts",
      from: ME_WORK,
      to: [people.priya],
      subject: "Thoughts on the rollback plan",
      html: para("Priya, a few thoughts on the rollback plan:"),
      received: at(-1, 19, 3),
      draft: true,
    }),
    m({
      folderId: "w-archive",
      from: people.dana,
      subject: "Team offsite agenda",
      html: para("Agenda for the offsite is attached. Bring your laptop!") + signature("Dana"),
      received: at(-30, 10, 0),
    }),
    m({
      folderId: "w-junk",
      from: { name: "Prize Department", address: "winner@lucky-prizes.example" },
      subject: "You have been selected!!!",
      html: para("Click here to claim your prize."),
      received: at(-2, 3, 14),
      read: false,
    }),
  ];
}

function homeMessages(): Message[] {
  const m = (seed: MessageSeed) => message(HOME, ME_HOME, seed);
  return [
    m({
      folderId: "h-inbox",
      from: people.mom,
      subject: "Sunday lunch",
      html: para(
        "Hi sweetie, are you and Ben still coming on Sunday? I'm making lemon pie. Let me know by 1 pm.",
        "Love, Mom",
      ),
      received: at(0, 8, 2),
      read: false,
    }),
    m({
      folderId: "h-inbox",
      from: people.ben,
      subject: "Climbing on Thursday?",
      plainText: [
        "Hey Alex,",
        "",
        "Are you up for climbing on Thursday evening? The new routes look fun:",
        "https://riversideclimbing.example/routes",
        "",
        "If you can't make it, could you let Sam know (sam.okafor@acme.example)? He was asking about a partner.",
        "",
        "Ben",
      ].join("\n"),
      received: at(0, 7, 15),
      read: false,
    }),
    m({
      folderId: "h-inbox",
      from: people.club,
      subject: "New bouldering routes this weekend",
      html: para(
        "We're resetting the north wall on Saturday — 30 new problems from V0 to V8. See you on the mats!",
      ),
      received: at(-1, 18, 45),
    }),
    m({
      folderId: "h-inbox",
      from: people.bank,
      subject: "Your statement is ready",
      html: para("Your monthly statement is now available in online banking."),
      received: at(-4, 7, 0),
    }),
    m({
      folderId: "h-inbox",
      from: people.ben,
      subject: "Photos from the hike",
      html: para("Finally got round to sorting the photos — a few favorites attached!"),
      received: at(-8, 21, 13),
      attachments: [
        { name: "summit.jpg", contentType: "image/jpeg", size: 2_412_004 },
        { name: "lake.jpg", contentType: "image/jpeg", size: 1_998_222 },
      ],
    }),
  ];
}

/** Older work mail, so the inbox has more than one page and several date groups. */
function olderWorkMessages(): Message[] {
  const senders = [
    people.priya,
    people.jonas,
    people.mei,
    people.sam,
    people.dana,
    people.lucia,
    people.it,
    people.hr,
  ];
  const topics: [string, string][] = [
    [
      "Weekly sync notes",
      "Notes from this week's sync are below. Main decision: we keep the Thursday demo slot.",
    ],
    [
      "Design review follow-up",
      "Thanks for the review. I've updated the flows based on your comments.",
    ],
    [
      "Budget check-in",
      "We're tracking slightly under budget for the quarter. No action needed yet.",
    ],
    [
      "Customer feedback roundup",
      "Top themes this month: faster search, offline mode and better calendar sharing.",
    ],
    ["Release notes draft", "First draft of the release notes is ready for a look."],
    [
      "Incident review: API latency",
      "Root cause was a misconfigured connection pool. Follow-ups are tracked in the board.",
    ],
    [
      "Hiring loop schedule",
      "Interview slots for next week are confirmed. Please check your calendar.",
    ],
    ["Roadmap questions", "A few questions came up in the roadmap session — could you weigh in?"],
    [
      "Docs migration plan",
      "We'll move the docs to the new site in three phases, starting with the API reference.",
    ],
    [
      "Security training reminder",
      "Please complete the annual security training by the end of the month.",
    ],
    [
      "Quarterly metrics",
      "Quarterly metrics are in. Activation is up 4% and support tickets are down.",
    ],
    [
      "Vendor contract renewal",
      "The analytics vendor contract renews next month. Any concerns before we sign?",
    ],
    ["Onboarding buddy", "Would you be the onboarding buddy for our new engineer starting Monday?"],
    [
      "Accessibility audit results",
      "The audit found 12 issues, mostly color contrast. Details in the report.",
    ],
  ];
  const out: Message[] = [];
  for (let i = 0; i < 64; i++) {
    const from = senders[i % senders.length]!;
    const [topic, line] = topics[(i * 5) % topics.length]!;
    out.push(
      message(WORK, ME_WORK, {
        folderId: "w-inbox",
        from,
        subject: i % 4 === 1 ? `Re: ${topic}` : topic,
        html: para("Hi Alex,", line) + signature((from.name ?? "").split(" ")[0] ?? ""),
        received: at(-14 - Math.floor(i * 2.6), 8 + (i % 9), (i * 17) % 60),
        flagged: i % 19 === 3,
        attachments:
          i % 11 === 4
            ? [
                {
                  name: `${topic.toLowerCase().replace(/\W+/g, "-")}.pdf`,
                  contentType: "application/pdf",
                  size: 120_000 + i * 997,
                },
              ]
            : undefined,
      }),
    );
  }
  return out;
}

/**
 * Mail from years back that only the servers still have: a server search
 * finds it. Its words aren't in the synced mail, so it shows what searching
 * the whole mailbox adds ("invoice", "offsite", "lease").
 */
function serverOnlyMessages(): Message[] {
  const work = (seed: MessageSeed) => message(WORK, ME_WORK, seed);
  const home = (seed: MessageSeed) => message(HOME, ME_HOME, seed);
  return [
    work({
      folderId: "w-archive",
      from: people.lucia,
      subject: "Northwind invoice 2023-118",
      html:
        para(
          "Hi Alex,",
          "Please find attached our invoice 2023-118 for the brand refresh workshops in March. Payment terms are 30 days from the invoice date.",
          "Thank you for a lovely project!",
        ) + signature("Lucía Fernández", "Northwind Design Studio"),
      received: at(-430, 16, 5),
      attachments: [
        { name: "Northwind-invoice-2023-118.pdf", contentType: "application/pdf", size: 96_412 },
      ],
    }),
    work({
      folderId: "w-archive",
      from: people.dana,
      cc: [people.priya, people.jonas],
      subject: "Kickoff notes from the 2022 offsite",
      html:
        para("Team,", "Thanks for two great days. The decisions from the kickoff session:") +
        "<ul><li>Atlas becomes the platform team's first priority</li><li>We move the weekly sync to Thursdays</li><li>Each team picks three bets for the year</li></ul>" +
        signature("Dana"),
      received: at(-1050, 18, 30),
    }),
    work({
      folderId: "w-archive",
      from: people.mei,
      subject: "Offsite venue options",
      html:
        para(
          "Three venues can take the whole team in October: the lake lodge, the old print works downtown and the vineyard. Prices and photos are in the sheet.",
        ) + signature("Mei"),
      received: at(-1090, 11, 20),
    }),
    work({
      folderId: "w-sent",
      from: ME_WORK,
      to: [people.sam],
      subject: "Re: Vendor shortlist for the data warehouse",
      html: para(
        "Sam, thanks for pulling this together. Let's invite the two cheapest vendors to demo, and ask the third for a reference customer first.",
      ),
      received: at(-700, 9, 48),
    }),
    home({
      folderId: "h-archive",
      from: { name: "Harborview Properties", address: "leasing@harborview.example" },
      subject: "Your apartment lease renewal",
      html: para(
        "Dear Alex Morgan,",
        "Your lease for apartment 4B ends on 31 August. The renewal agreement is attached; please sign and return it by 15 July.",
        "Kind regards, Harborview Properties",
      ),
      received: at(-500, 10, 0),
      attachments: [
        { name: "Lease-renewal-4B.pdf", contentType: "application/pdf", size: 231_870 },
      ],
    }),
    home({
      folderId: "h-inbox",
      from: people.ben,
      subject: "Safari photos from Etosha",
      html: para(
        "The zebras at the waterhole were the highlight. Here are the best shots from the trip!",
      ),
      received: at(-800, 20, 12),
      attachments: [{ name: "waterhole.jpg", contentType: "image/jpeg", size: 3_104_220 }],
    }),
  ];
}

// --- Protected mail ---------------------------------------------------------------

/** A sensitivity label as Microsoft 365 lists it, before its check. */
export type CatalogLabel = Omit<LabelOption, "check">;

/** The demo organization's sensitivity labels' ids (lowercase GUIDs). */
export const DEMO_LABEL_IDS = {
  public: "0f6c58a2-4a6b-4d3e-9b1a-1c2d3e4f5a60",
  general: "2a7f3c10-8b4e-4f6a-a1d2-3b4c5d6e7f80",
  confidential: "5c1e9d72-3f4a-4b8c-9d0e-1f2a3b4c5d60",
  allEmployees: "7b2d4e61-9c3a-4e5f-8a7b-6c5d4e3f2a10",
  atlas: "9e8d7c6b-5a4f-4e3d-8c2b-1a0f9e8d7c60",
} as const;

/** What Microsoft 365 lists for the demo's work account: its labels in order, and the policy. */
export function demoLabelCatalog(): { labels: CatalogLabel[]; policy: LabelPolicy } {
  const label = (
    id: string,
    name: string,
    color: string,
    sensitivity: number,
    extra: Partial<CatalogLabel> = {},
  ): CatalogLabel => ({
    id,
    name,
    parentName: null,
    tooltip: null,
    color,
    hasProtection: false,
    sensitivity,
    ...extra,
  });
  return {
    labels: [
      label(DEMO_LABEL_IDS.public, "Public", "#3a7d44", 0, {
        tooltip: "Information anyone may see.",
      }),
      label(DEMO_LABEL_IDS.general, "General", "#2b7aa8", 1, {
        tooltip: "Business information for Acme staff and partners.",
      }),
      label(DEMO_LABEL_IDS.confidential, "Confidential", "#b35f26", 2, {
        tooltip: "Sensitive business information. Share only with those who need it.",
      }),
      label(DEMO_LABEL_IDS.allEmployees, "All Employees", "#b35f26", 3, {
        parentName: "Confidential",
        hasProtection: true,
        tooltip: "Encrypted. Only Acme employees can open it.",
      }),
      label(DEMO_LABEL_IDS.atlas, "Project Atlas", "#a4262c", 4, {
        parentName: "Highly Confidential",
        hasProtection: true,
        tooltip: "Encrypted. Only the Atlas team can open it.",
      }),
    ],
    policy: {
      required: true,
      defaultLabelId: DEMO_LABEL_IDS.general,
      justifyDowngrade: true,
      moreInfoUrl: "https://acme.example/labels",
    },
  };
}

/** A label of the demo catalog as a message carries it. */
function demoLabel(id: string): MessageLabel {
  const label = demoLabelCatalog().labels.find((l) => l.id === id)!;
  return {
    id,
    name: label.parentName ? `${label.parentName} › ${label.name}` : label.name,
    color: label.color,
    fromCatalog: true,
  };
}

/** Protection with the message's attachments at `containers` as its containers. */
function protect(
  m: Message,
  protection: Omit<Protection, "containerIds">,
  containers: number[] = [],
): Message {
  m.protection = { ...protection, containerIds: containers.map((i) => m.attachments[i]!.id) };
  return m;
}

/** Microsoft's portal, as Purview Message Encryption links to it. */
export const DEMO_PORTAL_URL =
  "https://outlook.office365.com/Encryption/retrieve.ashx?recipientemailaddress=alex.morgan%40acme.example";
/** The same for the home address (the rpmsg-plus-link form a spoofed sender copied). */
const HOME_PORTAL_URL =
  "https://outlook.office365.com/Encryption/retrieve.ashx?recipientemailaddress=alex%40morgan-family.example";

/**
 * Protected and labeled mail (`?protected`): encrypted with Purview, a link
 * to Microsoft's portal with a look-alike button above it, S/MIME signed and
 * encrypted, a Sent copy that asked for encryption and a label, and a
 * spoofed sender. Ids are fixed: msg-protected-1 to 8.
 */
export function protectedMessages(): Message[] {
  const work = (n: number, seed: Omit<MessageSeed, "id">) =>
    message(WORK, ME_WORK, { ...seed, id: `msg-protected-${n}` });
  const home = (n: number, seed: Omit<MessageSeed, "id">) =>
    message(HOME, ME_HOME, { ...seed, id: `msg-protected-${n}` });
  const rpmsg = {
    name: "message.rpmsg",
    contentType: "application/x-microsoft-rpmsg-message",
    size: 48_213,
  };

  const forecast = protect(
    work(1, {
      folderId: "w-inbox",
      from: people.dana,
      subject: "Q4 forecast",
      html: para(
        "Dana Whitfield (dana.whitfield@acme.example) has sent you a protected message.",
        "Open it in Outlook to read it.",
      ),
      received: at(0, 6, 40),
      read: false,
      attachments: [rpmsg],
    }),
    { kind: "rightsManaged", portalUrl: null, contentHidden: false },
    [0],
  );
  forecast.label = demoLabel(DEMO_LABEL_IDS.allEmployees);

  const orgChart = work(2, {
    folderId: "w-inbox",
    from: people.hr,
    subject: "Updated org chart",
    html:
      para(
        "Hi all,",
        "The org chart is updated with this month's changes. You'll find it on the intranet under People.",
      ) + signature("People Team"),
    received: at(0, 6, 20),
  });
  orgChart.label = demoLabel(DEMO_LABEL_IDS.general);

  const contract = protect(
    work(3, {
      folderId: "w-inbox",
      from: people.lucia,
      subject: "Contract draft",
      html:
        '<p><a href="https://evil.example/login" style="display:inline-block;padding:12px 28px;border-radius:4px;background:#0078d4;color:#ffffff;font-weight:bold;text-decoration:none">Read the message</a></p>' +
        para(
          "Lucía Fernández (lucia@northwind-design.example) has sent you a protected message.",
          `<a href="${DEMO_PORTAL_URL}">Read the message</a>`,
          "Learn about messages protected by Microsoft Purview Message Encryption.",
        ),
      received: at(0, 5, 55),
      read: false,
    }),
    { kind: "portalLink", portalUrl: DEMO_PORTAL_URL, contentHidden: false },
  );

  const vpn = protect(
    work(4, {
      folderId: "w-inbox",
      from: people.it,
      subject: "VPN certificate renewal",
      html:
        para(
          "Hello,",
          "Your VPN certificate expires at the end of the month. The renewal instructions are in the attached PDF.",
        ) + signature("Acme IT Service Desk"),
      received: at(-1, 9, 10),
      attachments: [{ name: "smime.p7m", contentType: "multipart/signed", size: 214_771 }],
    }),
    { kind: "smimeSigned", portalUrl: null, contentHidden: true },
    [0],
  );

  const tax = protect(
    home(5, {
      folderId: "h-inbox",
      from: people.ben,
      subject: "Tax documents",
      html: "",
      received: at(0, 6, 5),
      read: false,
      attachments: [{ name: "smime.p7m", contentType: "application/pkcs7-mime", size: 389_120 }],
    }),
    { kind: "smimeEncrypted", portalUrl: null, contentHidden: false },
    [0],
  );

  const pricing = work(6, {
    folderId: "w-sent",
    from: ME_WORK,
    to: [people.priya],
    subject: "Re: Atlas pricing",
    html:
      para(
        "Priya, here are the price points we agreed on for the launch, before Finance's review.",
      ) + signature("Alex"),
    received: at(-1, 16, 25),
  });
  pricing.requested = { action: "encrypt", label: demoLabel(DEMO_LABEL_IDS.confidential) };

  const spoofed = protect(
    home(7, {
      folderId: "h-inbox",
      from: { name: "IT Service Desk <it@acme.example>\u202e", address: "alerts@notacme.example" },
      subject: "Your protected message",
      html: para(
        "You have received a protected message.",
        `<a href="${HOME_PORTAL_URL}">Read the message</a>`,
      ),
      received: at(0, 4, 30),
      read: false,
      attachments: [rpmsg],
    }),
    { kind: "rightsManaged", portalUrl: HOME_PORTAL_URL, contentHidden: false },
    [0],
  );

  const releaseNotes = work(8, {
    folderId: "w-inbox",
    from: people.jonas,
    subject: "Atlas public release notes",
    html:
      para(
        "Here's the first draft of the public release notes for Atlas. Comments welcome before Friday.",
      ) + signature("Jonas"),
    received: at(-1, 14, 50),
  });
  releaseNotes.label = demoLabel(DEMO_LABEL_IDS.public);

  return [forecast, orgChart, contract, vpn, tax, pricing, spoofed, releaseNotes];
}

/**
 * Mail that shows how search ranks and filters. "porto" is in three work
 * messages and nowhere else: best match lists the subjects first (the
 * recent one, then the old one) and the mention in a preview last; newest
 * first, the mention leads. The server has an old flagged message and one
 * with an attachment from last year, which only `is:flagged` and
 * `has:attachment received:"last year"` on the server find; and Sent and
 * Projects have an unread message each, for `in:`.
 */
function searchDemoMessages(): { local: Message[]; server: Message[] } {
  const work = (seed: MessageSeed) => message(WORK, ME_WORK, seed);
  const lastYear = new Date();
  lastYear.setFullYear(lastYear.getFullYear() - 1, 6, 1);
  lastYear.setHours(9, 40, 0, 0);
  const itinerary = [
    "Day one: arrive in the afternoon and check in at the riverside guesthouse.",
    "Day two: the workshop runs from nine until four, with lunch in the old market hall.",
    "Day three: a free morning, then the team dinner by the bridge.",
    "Day four: the early train back; taxis are booked for six.",
  ];
  return {
    local: [
      work({
        folderId: "w-archive",
        from: people.travel,
        subject: "Porto offsite: tickets and hotel",
        html:
          para(
            "Your tickets for the team trip are attached, with the hotel booking for three nights.",
            "Check-in opens at 14:00. Breakfast is included.",
          ) + signature("Skyway Travel"),
        received: at(-2, 9, 5),
        flagged: true,
        attachments: [
          { name: "tickets-and-hotel.pdf", contentType: "application/pdf", size: 88_310 },
        ],
      }),
      work({
        folderId: "w-archive",
        from: people.mei,
        subject: "Porto itinerary",
        html: para(
          "Hi Alex,",
          "Here is the plan for the four days, as we discussed:",
          ...itinerary,
        ),
        received: at(-200, 11, 30),
      }),
      work({
        folderId: "w-archive",
        from: people.sam,
        subject: "Friday plans",
        html:
          para(
            "Quick one: we fly to porto on Friday, so the planning review moves to Thursday afternoon.",
            "I'll send the updated invite.",
          ) + signature("Sam"),
        received: at(-1, 16, 20),
      }),
      work({
        folderId: "w-sent",
        from: ME_WORK,
        to: [people.jonas],
        subject: "Notes for the export endpoint fix",
        html: para("Jonas, my notes from the call are below; the ticket has the details."),
        received: at(-1, 10, 12),
        read: false,
      }),
      work({
        folderId: "w-projects",
        from: people.priya,
        subject: "Projects board: what moves next sprint",
        html: para(
          "Hi Alex,",
          "I've sorted the projects board for next sprint. Could you check the order of the top five?",
        ),
        received: at(-1, 14, 45),
        read: false,
      }),
    ],
    server: [
      work({
        folderId: "w-archive",
        from: people.dana,
        subject: "Signed partnership agreement with Fabrikam",
        html:
          para(
            "Alex,",
            "Fabrikam signed the partnership agreement today. Keep this one handy for the renewal talks.",
          ) + signature("Dana"),
        received: at(-400, 15, 10),
        flagged: true,
      }),
      work({
        folderId: "w-archive",
        from: people.lucia,
        subject: "Brand guidelines, final edition",
        html:
          para(
            "Hello Alex,",
            "The final edition of the brand guidelines is attached, with the color and type sections updated.",
          ) + signature("Lucía"),
        received: lastYear,
        attachments: [
          { name: "brand-guidelines-final.pdf", contentType: "application/pdf", size: 2_481_118 },
        ],
      }),
    ],
  };
}

function contact(
  accountId: string,
  id: string,
  person: EmailAddress,
  extra: Partial<Contact> = {},
): Contact {
  const [given, ...rest] = (person.name ?? "").split(" ");
  return {
    id,
    accountId,
    remoteId: id,
    displayName: person.name ?? person.address,
    givenName: given ?? null,
    surname: rest.join(" ") || null,
    emails: [{ name: null, address: person.address }],
    phones: [],
    company: null,
    jobTitle: null,
    department: null,
    address: null,
    birthday: null,
    notes: null,
    ...extra,
  };
}

function event(
  id: string,
  calendarId: string,
  accountId: string,
  subject: string,
  start: Date,
  minutes: number,
  extra: Partial<CalendarEvent> = {},
): CalendarEvent {
  return {
    id,
    accountId,
    calendarId,
    remoteId: id,
    subject,
    location: null,
    bodyHtml: null,
    bodyPreview: null,
    start: iso(start),
    end: iso(new Date(start.getTime() + minutes * 60_000)),
    isAllDay: false,
    organizer: ME_WORK,
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
}

function workdaysAround(): number[] {
  // Day offsets of weekdays from two weeks ago to three weeks ahead.
  const out: number[] = [];
  for (let offset = -14; offset <= 21; offset++) {
    const day = at(offset, 12).getDay();
    if (day !== 0 && day !== 6) out.push(offset);
  }
  return out;
}

// Acme's mark for its signature: a white A on a teal tile, as an inline SVG
// image so the demo (and its screenshots) need no network.
const ACME_LOGO =
  '<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 48 48">' +
  '<rect width="48" height="48" rx="11" fill="#0b7a75"/>' +
  '<path fill="#fff" fill-rule="evenodd" d="M24 11 36 37h-5.3l-2.5-5.6h-8.4L17.3 37H12zm-2.4 15.8h4.8L24 21.4z"/>' +
  "</svg>";

/**
 * Alex's signature as Acme sets it with Cumulas Signatures: a table with the
 * logo beside the name, title, company, phone and website, styles inline.
 */
export const ACME_SIGNATURE =
  '<table cellpadding="0" cellspacing="0" border="0" role="presentation" style="border-collapse:collapse;font-family:Arial,Helvetica,sans-serif;font-size:13px;line-height:19px;color:#3c4043">' +
  "<tr>" +
  '<td style="padding:2px 14px 2px 0;vertical-align:top;border-right:2px solid #0b7a75">' +
  `<img src="data:image/svg+xml;base64,${btoa(ACME_LOGO)}" width="48" height="48" alt="Acme" style="display:block;border:0">` +
  "</td>" +
  '<td style="padding:0 0 0 14px;vertical-align:top">' +
  '<div style="font-size:15px;line-height:21px;font-weight:bold;color:#1d1c1a">Alex Morgan</div>' +
  "<div>Engineering Manager, Atlas</div>" +
  '<div style="padding-top:6px"><span style="font-weight:bold;color:#0b7a75">Acme</span> · 1 Canal Street, Seattle</div>' +
  '<div><a href="tel:+12065550147" style="color:#3c4043;text-decoration:none">+1 (206) 555-0147</a> · ' +
  '<a href="https://acme.example" style="color:#0b7a75;text-decoration:none">acme.example</a></div>' +
  "</td>" +
  "</tr>" +
  "</table>";

/** The demo's signatures: Acme's on the work account, Alex's own at home. */
function demoSignatures(): AccountSignature[] {
  return [
    {
      accountId: WORK,
      ownHtml: "",
      ownOnReplies: true,
      organization: {
        state: "ready",
        html: ACME_SIGNATURE,
        onReplies: true,
        checkedAt: iso(new Date(Date.now() - 5 * 60_000)),
        stale: false,
        checkableAt: null,
      },
    },
    {
      accountId: HOME,
      ownHtml: "<div>Alex Morgan</div><div>+1 (206) 555-0199</div>",
      ownOnReplies: true,
      organization: {
        state: "unsupported",
        html: null,
        onReplies: false,
        checkedAt: null,
        stale: false,
        checkableAt: null,
      },
    },
  ];
}

export function createDemoState(): MockState {
  messageSeq = 0;
  const accounts: Account[] = [
    {
      id: WORK,
      kind: "microsoft",
      displayName: "Alex Morgan",
      email: ME_WORK.address,
      color: "#0b7a75",
      settings: { kind: "microsoft", tenantId: "demo-tenant", userId: "demo-user" },
      createdAt: iso(at(-90, 9)),
      lastSyncedAt: iso(new Date()),
      syncError: null,
      needsPermission: [],
      refusedFeatures: [],
    },
    {
      id: HOME,
      kind: "imap",
      displayName: "Alex (Personal)",
      email: ME_HOME.address,
      color: "#b35f26",
      settings: {
        kind: "imap",
        imapHost: "imap.morgan-family.example",
        imapPort: 993,
        imapTls: "tls",
        smtpHost: "smtp.morgan-family.example",
        smtpPort: 465,
        smtpTls: "tls",
        username: ME_HOME.address,
      },
      createdAt: iso(at(-60, 9)),
      lastSyncedAt: iso(new Date()),
      syncError: null,
      needsPermission: [],
      refusedFeatures: [],
    },
  ];

  const folders: Folder[] = [
    folder(WORK, "w-inbox", "Inbox", "inbox"),
    folder(WORK, "w-drafts", "Drafts", "drafts"),
    folder(WORK, "w-sent", "Sent Items", "sent"),
    folder(WORK, "w-archive", "Archive", "archive"),
    folder(WORK, "w-junk", "Junk Email", "junk"),
    folder(WORK, "w-trash", "Deleted Items", "trash"),
    folder(WORK, "w-projects", "Projects", "other"),
    folder(WORK, "w-atlas", "Atlas", "other", "w-projects"),
    folder(HOME, "h-inbox", "Inbox", "inbox"),
    folder(HOME, "h-drafts", "Drafts", "drafts"),
    folder(HOME, "h-sent", "Sent", "sent"),
    folder(HOME, "h-archive", "Archive", "archive"),
    folder(HOME, "h-junk", "Spam", "junk"),
    folder(HOME, "h-trash", "Trash", "trash"),
  ];

  const messages = [...workMessages(), ...homeMessages(), ...olderWorkMessages()];
  // After the synced mail, so its ids stay as they were.
  const serverMessages = serverOnlyMessages();
  // Last, so every id before stays as it was.
  const searchDemo = searchDemoMessages();
  messages.push(...searchDemo.local);
  serverMessages.push(...searchDemo.server);

  const calendars: Calendar[] = [
    {
      id: "cal-work",
      accountId: WORK,
      remoteId: "cal-work",
      name: "Calendar",
      color: "#0b7a75",
      canEdit: true,
      isDefault: true,
    },
    {
      id: "cal-team",
      accountId: WORK,
      remoteId: "cal-team",
      name: "Atlas team",
      color: "#2b7aa8",
      canEdit: true,
      isDefault: false,
    },
    {
      id: "cal-holidays",
      accountId: WORK,
      remoteId: "cal-holidays",
      name: "US holidays",
      color: "#8c6a10",
      canEdit: false,
      isDefault: false,
    },
    {
      id: "cal-home",
      accountId: HOME,
      remoteId: "local:calendar",
      name: "Calendar",
      color: "#b35f26",
      canEdit: true,
      isDefault: true,
    },
  ];

  const events: CalendarEvent[] = [];
  let e = 0;
  const nextId = () => `evt-${++e}`;
  for (const d of workdaysAround()) {
    events.push(
      event(nextId(), "cal-team", WORK, "Atlas stand-up", at(d, 9, 30), 15, {
        isRecurring: true,
        isOnlineMeeting: true,
        onlineMeetingUrl: "https://meet.example/atlas-standup",
        organizer: people.priya,
        responseStatus: "accepted",
        attendees: [people.priya, people.jonas, people.mei, ME_WORK].map((p) => ({
          email: p,
          kind: "required" as const,
          response: "accepted" as const,
        })),
      }),
    );
  }
  events.push(
    event(nextId(), "cal-work", WORK, "Atlas launch review", at(0, 14), 60, {
      location: "Room 4.12 / Online",
      isOnlineMeeting: true,
      onlineMeetingUrl: "https://meet.example/atlas-review",
      organizer: people.priya,
      responseStatus: "accepted",
      bodyHtml:
        "<p>Final go/no-go review for the Atlas launch. Please read the checklist beforehand.</p>",
      bodyPreview: "Final go/no-go review for the Atlas launch.",
      attendees: [
        { email: people.priya, kind: "required", response: "organizer" },
        { email: people.jonas, kind: "required", response: "accepted" },
        { email: people.mei, kind: "required", response: "tentative" },
        { email: people.dana, kind: "optional", response: "notResponded" },
        { email: ME_WORK, kind: "required", response: "accepted" },
      ],
    }),
    event(nextId(), "cal-work", WORK, "Focus time", at(0, 10, 30), 90, {
      showAs: "busy",
    }),
    event(nextId(), "cal-work", WORK, "1:1 Dana / Alex", at(1, 11), 30, {
      organizer: people.dana,
      responseStatus: "accepted",
      isRecurring: true,
      attendees: [
        { email: people.dana, kind: "required", response: "organizer" },
        { email: ME_WORK, kind: "required", response: "accepted" },
      ],
    }),
    event(nextId(), "cal-work", WORK, "Quarterly planning kickoff", at(2, 13), 120, {
      location: "Auditorium",
      organizer: people.dana,
      responseStatus: "notResponded",
      showAs: "tentative",
      attendees: [
        { email: people.dana, kind: "required", response: "organizer" },
        { email: ME_WORK, kind: "required", response: "notResponded" },
        { email: people.priya, kind: "required", response: "accepted" },
      ],
    }),
    event(nextId(), "cal-work", WORK, "Lunch with Sam", at(3, 12, 30), 60, {
      location: "Ramen Ichi",
      showAs: "free",
    }),
    event(nextId(), "cal-work", WORK, "Design sync with Northwind", at(-1, 15), 45, {
      organizer: ME_WORK,
      attendees: [{ email: people.lucia, kind: "required", response: "accepted" }],
      isOnlineMeeting: true,
      onlineMeetingUrl: "https://meet.example/design-sync",
    }),
    event(nextId(), "cal-work", WORK, "Lisbon trip", new Date(allDay(8)), 4 * 24 * 60, {
      isAllDay: true,
      showAs: "outOfOffice",
      reminderMinutes: null,
    }),
    event(nextId(), "cal-holidays", WORK, "Company holiday", new Date(allDay(15)), 24 * 60, {
      isAllDay: true,
      showAs: "free",
      reminderMinutes: null,
    }),
    event(nextId(), "cal-home", HOME, "Sunday lunch at Mom's", at(nextWeekday(0), 13), 180, {
      location: "Mom's",
      organizer: null,
      showAs: "busy",
    }),
    event(nextId(), "cal-home", HOME, "Climbing", at(1, 19), 120, {
      location: "Riverside Climbing",
      organizer: null,
    }),
    event(nextId(), "cal-home", HOME, "Dentist", at(5, 8, 30), 45, {
      organizer: null,
      reminderMinutes: 60,
    }),
  );

  // A busy current week: overlapping meetings, every "show as" state, a
  // canceled and a declined meeting, an event across midnight, multi-day bars.
  const attending = (...list: [EmailAddress, Attendee["response"]][]): Attendee[] =>
    list.map(([email, response]) => ({ email, kind: "required", response }));
  events.push(
    event(nextId(), "cal-team", WORK, "Atlas launch week", new Date(allDay(-2)), 5 * 24 * 60, {
      isAllDay: true,
      showAs: "free",
      reminderMinutes: null,
      organizer: people.priya,
      responseStatus: "accepted",
    }),
    event(nextId(), "cal-work", WORK, "Sprint planning", at(-2, 10), 90, {
      organizer: people.priya,
      responseStatus: "accepted",
      isOnlineMeeting: true,
      onlineMeetingUrl: "https://meet.example/sprint-planning",
      attendees: attending(
        [people.priya, "organizer"],
        [people.jonas, "accepted"],
        [people.mei, "accepted"],
        [ME_WORK, "accepted"],
      ),
    }),
    event(nextId(), "cal-work", WORK, "Lunch & learn: Rust async", at(-2, 12), 60, {
      location: "Auditorium",
      showAs: "free",
      organizer: people.jonas,
      responseStatus: "accepted",
    }),
    event(nextId(), "cal-work", WORK, "Hiring sync", at(-1, 11), 30, {
      organizer: people.dana,
      responseStatus: "accepted",
      isRecurring: true,
    }),
    event(nextId(), "cal-work", WORK, "Architecture forum", at(-1, 16), 60, {
      location: "Room 2.04",
      organizer: people.jonas,
      responseStatus: "accepted",
      isCancelled: true,
    }),
    event(nextId(), "cal-work", WORK, "Security review", at(0, 11), 60, {
      location: "Room 3.01",
      organizer: people.jonas,
      responseStatus: "tentative",
      showAs: "tentative",
      isOnlineMeeting: true,
      onlineMeetingUrl: "https://meet.example/security-review",
      reminderMinutes: 60,
      attendees: attending(
        [people.jonas, "organizer"],
        [people.it, "accepted"],
        [ME_WORK, "tentative"],
      ),
    }),
    event(nextId(), "cal-work", WORK, "Customer escalation: Contoso", at(0, 14, 30), 45, {
      organizer: people.sam,
      responseStatus: "notResponded",
      isOnlineMeeting: true,
      onlineMeetingUrl: "https://meet.example/contoso-escalation",
      bodyHtml:
        "<p>Contoso reported intermittent sync failures after the 4.2 rollout. Let's agree on next steps.</p>" +
        "<ul><li>Timeline and impact</li><li>Workaround status</li><li>Customer communication</li></ul>" +
        '<p>Incident: <a href="https://status.example/incidents/4821">INC-4821</a></p>',
      bodyPreview: "Contoso reported intermittent sync failures after the 4.2 rollout.",
      attendees: [
        { email: people.sam, kind: "required", response: "organizer" },
        { email: people.jonas, kind: "required", response: "accepted" },
        { email: ME_WORK, kind: "required", response: "notResponded" },
        { email: people.dana, kind: "optional", response: "tentative" },
        { email: people.priya, kind: "optional", response: "declined" },
      ],
    }),
    event(nextId(), "cal-work", WORK, "Coffee with Mei", at(0, 16), 30, {
      location: "Kitchen, 4th floor",
      showAs: "free",
      reminderMinutes: 5,
      attendees: attending([people.mei, "accepted"]),
    }),
    event(nextId(), "cal-work", WORK, "School pickup", at(1, 15, 30), 90, {
      showAs: "outOfOffice",
      reminderMinutes: 30,
    }),
    event(nextId(), "cal-work", WORK, "Team social planning", at(1, 16), 30, {
      organizer: people.mei,
      responseStatus: "declined",
    }),
    event(nextId(), "cal-work", WORK, "Working from home", new Date(allDay(2)), 24 * 60, {
      isAllDay: true,
      showAs: "workingElsewhere",
      reminderMinutes: null,
    }),
    event(nextId(), "cal-team", WORK, "Release window", at(2, 22), 180, {
      location: "On-call bridge",
      organizer: people.priya,
      responseStatus: "accepted",
      isOnlineMeeting: true,
      onlineMeetingUrl: "https://meet.example/release-bridge",
    }),
  );

  const contacts: Contact[] = [
    contact(WORK, "c-priya", people.priya, {
      company: "Acme",
      jobTitle: "Program Manager",
      department: "Atlas",
      phones: [
        { kind: "mobile", number: "+1 (206) 555-0123" },
        { kind: "business", number: "+1 (206) 555-0110" },
      ],
      address: "Acme Inc.\n1 Canal Street\nSeattle, WA 98101",
      birthday: "1989-11-06",
      notes:
        "Runs the Atlas launch. Prefers a quick chat over long email threads.\nOut of office on Fridays in December.",
    }),
    contact(WORK, "c-jonas", people.jonas, {
      company: "Acme",
      jobTitle: "Staff Engineer",
      department: "Platform",
      phones: [{ kind: "business", number: "+49 30 901820" }],
      address: "Acme GmbH\nFriedrichstraße 68\n10117 Berlin",
    }),
    contact(WORK, "c-mei", people.mei, { company: "Acme", jobTitle: "Product Marketing Lead" }),
    contact(WORK, "c-sam", people.sam, {
      company: "Acme",
      jobTitle: "Support Engineer",
      phones: [{ kind: "business", number: "+1 (206) 555-0132" }],
    }),
    contact(WORK, "c-dana", people.dana, {
      company: "Acme",
      jobTitle: "VP Engineering",
      department: "Engineering",
    }),
    contact(WORK, "c-lucia", people.lucia, {
      company: "Northwind Design Studio",
      jobTitle: "Illustrator",
      address: "Calle Mayor 12, Madrid",
    }),
    contact(WORK, "c-it", people.it, { company: "Acme" }),
    contact(
      WORK,
      "c-aisha",
      { name: "Aisha Khan", address: "aisha.khan@contoso-partners.example" },
      {
        emails: [
          { name: null, address: "aisha.khan@contoso-partners.example" },
          { name: null, address: "aisha@khan-advisory.example" },
        ],
        company: "Contoso Partners",
        jobTitle: "Account Director",
        phones: [
          { kind: "mobile", number: "+1 (312) 555-0178" },
          { kind: "business", number: "+1 (312) 555-0122" },
        ],
        address: "Contoso Partners\n22 King Street\nChicago, IL 60601",
        notes: "Main contact for the Contoso reseller agreement. Renewal talks start in January.",
      },
    ),
    contact(
      WORK,
      "c-carlos",
      { name: "Carlos Mendes", address: "carlos.mendes@fabrikam.example" },
      {
        company: "Fabrikam",
        jobTitle: "Data Scientist",
        department: "Analytics",
        phones: [{ kind: "mobile", number: "+351 912 345 678" }],
      },
    ),
    contact(
      WORK,
      "c-elena",
      { name: "Elena Petrova", address: "elena.petrova@fabrikam.example" },
      { company: "Fabrikam", jobTitle: "Procurement Lead", department: "Purchasing" },
    ),
    contact(
      WORK,
      "c-oliver",
      { name: "Oliver Grant", address: "oliver.grant@woodgrove-legal.example" },
      {
        company: "Woodgrove Legal",
        jobTitle: "Senior Counsel",
        phones: [{ kind: "business", number: "+1 (206) 555-0187" }],
      },
    ),
    contact(HOME, "c-mom", people.mom, {
      givenName: "Helen",
      surname: "Morgan",
      displayName: "Helen Morgan (Mom)",
      phones: [{ kind: "home", number: "+1 (503) 555-0155" }],
      address: "14 Orchard Lane\nPortland, OR 97201",
      birthday: dateOnly(23).replace(/^\d{4}/, "1961"),
    }),
    contact(HOME, "c-ben", people.ben, {
      phones: [{ kind: "mobile", number: "+1 (503) 555-0146" }],
    }),
    contact(HOME, "c-club", people.club, { notes: "Membership renews in March." }),
    contact(
      HOME,
      "c-zoe",
      { name: "Zoë Adeyemi", address: "zoe.adeyemi@mailbox.example" },
      {
        phones: [{ kind: "mobile", number: "+1 (503) 555-0121" }],
        birthday: "1991-04-27",
        notes: "Climbing partner. Allergic to peanuts.",
      },
    ),
    contact(
      HOME,
      "c-plumber",
      { name: "24/7 Emergency Plumbing", address: "help@247plumbing.example" },
      {
        givenName: null,
        surname: null,
        company: "24/7 Emergency Plumbing",
        phones: [{ kind: "business", number: "+1 (800) 555-0147" }],
      },
    ),
  ];

  const taskLists: TaskList[] = [
    { id: "tl-work", accountId: WORK, remoteId: "tl-work", name: "Tasks", isDefault: true },
    {
      id: "tl-atlas",
      accountId: WORK,
      remoteId: "tl-atlas",
      name: "Atlas launch",
      isDefault: false,
    },
    { id: "tl-home", accountId: HOME, remoteId: "local:tasks", name: "Tasks", isDefault: true },
    {
      id: "tl-groceries",
      accountId: HOME,
      remoteId: "local:groceries",
      name: "Groceries",
      isDefault: false,
    },
  ];

  const now = iso(new Date());
  let t = 0;
  const task = (
    listId: string,
    accountId: string,
    title: string,
    extra: Partial<Task> = {},
  ): Task => ({
    id: `task-${++t}`,
    accountId,
    listId,
    remoteId: `task-${t}`,
    title,
    notes: null,
    status: "notStarted",
    importance: "normal",
    dueDate: null,
    reminderAt: null,
    completedAt: null,
    createdAt: now,
    updatedAt: now,
    ...extra,
  });
  const created = (daysAgo: number) => ({ createdAt: iso(at(-daysAgo, 9)) });
  const tasks: Task[] = [
    task("tl-atlas", WORK, "Review rollback plan", {
      dueDate: dateOnly(0),
      importance: "high",
      reminderAt: iso(at(0, 16)),
    }),
    task("tl-atlas", WORK, "Confirm launch-week on-call rotation", { dueDate: dateOnly(1) }),
    task("tl-atlas", WORK, "Approve customer announcement", {
      dueDate: dateOnly(2),
      notes: "Check the pricing paragraph with Finance.",
    }),
    task("tl-atlas", WORK, "Book retro room", {
      status: "completed",
      completedAt: iso(at(-1, 16)),
    }),
    task("tl-work", WORK, "Add top three bets to planning doc", {
      dueDate: dateOnly(3),
      importance: "high",
    }),
    task("tl-work", WORK, "Benefits enrollment", { dueDate: dateOnly(3) }),
    task("tl-work", WORK, "Expense Lisbon trip", { dueDate: dateOnly(14) }),
    task("tl-work", WORK, "Read load test report"),
    task("tl-home", HOME, "Buy flowers for Sunday", { dueDate: dateOnly(nextWeekday(0) - 1) }),
    task("tl-home", HOME, "Renew climbing membership", { dueDate: dateOnly(20) }),
    task("tl-home", HOME, "Back up photos"),
    // More variety for the Tasks views: overdue, notes, completed, another list.
    task("tl-work", WORK, "Send Q3 metrics to Dana", {
      dueDate: dateOnly(-2),
      importance: "high",
      notes: "Use the dashboard export, not the spreadsheet.\nInclude churn by region.",
      ...created(6),
    }),
    task("tl-atlas", WORK, "Update status page copy", { dueDate: dateOnly(-1), ...created(4) }),
    task("tl-work", WORK, "Submit timesheet", {
      status: "completed",
      completedAt: iso(at(-2, 17)),
      ...created(7),
    }),
    task("tl-work", WORK, "Reply to Contoso about renewal terms", {
      importance: "high",
      ...created(2),
    }),
    task("tl-home", HOME, "Book car service", {
      dueDate: dateOnly(9),
      notes: "Ask about the rattle in the boot.",
      ...created(3),
    }),
    task("tl-home", HOME, "Pay council tax", {
      status: "completed",
      completedAt: iso(at(-3, 20)),
      ...created(10),
    }),
    task("tl-groceries", HOME, "Oat milk", created(1)),
    task("tl-groceries", HOME, "Coffee beans", created(1)),
    task("tl-groceries", HOME, "Lemons", created(1)),
  ];

  const state: MockState = {
    accounts,
    folders,
    messages,
    calendars,
    events,
    contacts,
    taskLists,
    tasks,
    signatures: demoSignatures(),
    serverMessages,
  };
  recount(state);
  return state;
}

/** Day offset of the next given weekday (0 = Sunday), at least 1 day ahead. */
function nextWeekday(weekday: number): number {
  for (let offset = 1; offset <= 7; offset++) {
    if (at(offset, 12).getDay() === weekday) return offset;
  }
  return 7;
}

/** Recomputes folder counts from the messages (plus any server-only ones). */
export function recount(state: MockState): void {
  const all = [...state.messages, ...(state.serverMessages ?? [])];
  for (const f of state.folders) {
    const inFolder = all.filter((m) => m.folderId === f.id);
    const remote = state.remoteCounts?.[f.id];
    f.totalCount = inFolder.length + (remote?.total ?? 0);
    f.unreadCount = inFolder.filter((m) => !m.isRead).length + (remote?.unread ?? 0);
  }
}
