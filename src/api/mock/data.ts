// Demo data for the in-memory backend. Dates are relative to "now" so the
// demo always looks current. All people and companies are fictional.

import type {
  Account,
  Calendar,
  CalendarEvent,
  Contact,
  EmailAddress,
  Folder,
  FolderRole,
  Message,
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
  mum: { name: "Mum", address: "helen.morgan@mailbox.example" },
  ben: { name: "Ben Morgan", address: "ben.morgan@mailbox.example" },
  bank: { name: "Harbor Bank", address: "no-reply@harborbank.example" },
  club: { name: "Riverside Climbing Club", address: "hello@riversideclimbing.example" },
} satisfies Record<string, EmailAddress>;

function at(daysFromToday: number, hours: number, minutes = 0): Date {
  const d = new Date();
  d.setDate(d.getDate() + daysFromToday);
  d.setHours(hours, minutes, 0, 0);
  return d;
}

function iso(d: Date): string {
  return d.toISOString();
}

/** Midnight UTC of a local calendar day, as all-day events are stored. */
function allDay(daysFromToday: number): string {
  const d = at(daysFromToday, 0);
  return new Date(Date.UTC(d.getFullYear(), d.getMonth(), d.getDate())).toISOString();
}

function dateOnly(daysFromToday: number): string {
  const d = at(daysFromToday, 12);
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${d.getFullYear()}-${m}-${day}`;
}

function folder(
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
  return `<p style="color:#605e5c;margin-top:16px">— ${name}${title ? `<br>${title}` : ""}</p>`;
}

let messageSeq = 0;

interface MessageSeed {
  folderId: string;
  from: EmailAddress;
  to?: EmailAddress[];
  cc?: EmailAddress[];
  subject: string;
  html: string;
  received: Date;
  read?: boolean;
  flagged?: boolean;
  importance?: "low" | "normal" | "high";
  attachments?: { name: string; contentType: string; size: number }[];
  draft?: boolean;
}

function message(accountId: string, me: EmailAddress, seed: MessageSeed): Message {
  messageSeq += 1;
  const id = `msg-${messageSeq}`;
  const text = seed.html
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
    receivedAt: iso(seed.received),
    isRead: seed.read ?? true,
    isFlagged: seed.flagged ?? false,
    isDraft: seed.draft ?? false,
    hasAttachments: (seed.attachments?.length ?? 0) > 0,
    importance: seed.importance ?? "normal",
    bcc: [],
    replyTo: [],
    bodyHtml: seed.html,
    bodyText: text,
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
        "<ul><li>Confirm on-call rota for launch week</li><li>Sign-off from Security on the data retention change</li><li>Customer comms draft (Mei has it)</li></ul>" +
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
          "Full report on the wiki.",
        ) +
        `<blockquote style="border-left:3px solid #c8c6c4;margin:0;padding-left:12px;color:#605e5c">On Monday, Alex Morgan wrote:<br>Can we get a load test in before the review?</blockquote>` +
        signature("Jonas"),
      received: at(-1, 17, 31),
    }),
    m({
      folderId: "w-inbox",
      from: people.dana,
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
      from: people.mum,
      subject: "Sunday lunch",
      html: para(
        "Hello love, are you and Ben still coming on Sunday? I'm making the lemon tart. Let me know about 1pm.",
        "Mum x",
      ),
      received: at(0, 8, 2),
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
      html: para("Finally got round to sorting the photos — a few favourites attached!"),
      received: at(-8, 21, 13),
      attachments: [
        { name: "summit.jpg", contentType: "image/jpeg", size: 2_412_004 },
        { name: "lake.jpg", contentType: "image/jpeg", size: 1_998_222 },
      ],
    }),
  ];
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

export function createDemoState(): MockState {
  messageSeq = 0;
  const accounts: Account[] = [
    {
      id: WORK,
      kind: "microsoft",
      displayName: "Alex Morgan",
      email: ME_WORK.address,
      color: "#0f6cbd",
      settings: { kind: "microsoft", tenantId: "demo-tenant", userId: "demo-user" },
      createdAt: iso(at(-90, 9)),
      lastSyncedAt: iso(new Date()),
      syncError: null,
    },
    {
      id: HOME,
      kind: "imap",
      displayName: "Alex (Personal)",
      email: ME_HOME.address,
      color: "#c239b3",
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

  const messages = [...workMessages(), ...homeMessages()];

  const calendars: Calendar[] = [
    {
      id: "cal-work",
      accountId: WORK,
      remoteId: "cal-work",
      name: "Calendar",
      color: "#0f6cbd",
      canEdit: true,
      isDefault: true,
    },
    {
      id: "cal-team",
      accountId: WORK,
      remoteId: "cal-team",
      name: "Atlas team",
      color: "#038387",
      canEdit: true,
      isDefault: false,
    },
    {
      id: "cal-holidays",
      accountId: WORK,
      remoteId: "cal-holidays",
      name: "Public holidays",
      color: "#8764b8",
      canEdit: false,
      isDefault: false,
    },
    {
      id: "cal-home",
      accountId: HOME,
      remoteId: "local:calendar",
      name: "Calendar",
      color: "#c239b3",
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
      reminderMinutes: null,
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
    event(nextId(), "cal-home", HOME, "Sunday lunch at Mum's", at(nextWeekday(0), 13), 180, {
      location: "Mum's",
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

  const contacts: Contact[] = [
    contact(WORK, "c-priya", people.priya, {
      company: "Acme",
      jobTitle: "Program Manager",
      department: "Atlas",
      phones: [{ kind: "mobile", number: "+44 7700 900123" }],
    }),
    contact(WORK, "c-jonas", people.jonas, {
      company: "Acme",
      jobTitle: "Staff Engineer",
      department: "Platform",
    }),
    contact(WORK, "c-mei", people.mei, { company: "Acme", jobTitle: "Product Marketing Lead" }),
    contact(WORK, "c-sam", people.sam, {
      company: "Acme",
      jobTitle: "Support Engineer",
      phones: [{ kind: "business", number: "+44 20 7946 0321" }],
    }),
    contact(WORK, "c-dana", people.dana, { company: "Acme", jobTitle: "VP Engineering" }),
    contact(WORK, "c-lucia", people.lucia, {
      company: "Northwind Design Studio",
      jobTitle: "Illustrator",
      address: "Calle Mayor 12, Madrid",
    }),
    contact(WORK, "c-it", people.it, { company: "Acme" }),
    contact(HOME, "c-mum", people.mum, {
      givenName: "Helen",
      surname: "Morgan",
      displayName: "Helen Morgan (Mum)",
      phones: [{ kind: "home", number: "+44 1632 960555" }],
      birthday: dateOnly(23),
    }),
    contact(HOME, "c-ben", people.ben, { phones: [{ kind: "mobile", number: "+44 7700 900456" }] }),
    contact(HOME, "c-club", people.club, { notes: "Membership renews in March." }),
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
    completedAt: null,
    createdAt: now,
    updatedAt: now,
    ...extra,
  });
  const tasks: Task[] = [
    task("tl-atlas", WORK, "Review rollback plan", { dueDate: dateOnly(0), importance: "high" }),
    task("tl-atlas", WORK, "Confirm launch-week on-call rota", { dueDate: dateOnly(1) }),
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

/** Recomputes folder counts from the messages. */
export function recount(state: MockState): void {
  for (const f of state.folders) {
    const inFolder = state.messages.filter((m) => m.folderId === f.id);
    f.totalCount = inFolder.length;
    f.unreadCount = inFolder.filter((m) => !m.isRead).length;
  }
}
