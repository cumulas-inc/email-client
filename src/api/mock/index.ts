// In-memory implementation of the Backend, mirroring the Rust service's
// behavior closely enough for UI development, tests and demos.

import type { Backend, Unsubscribe } from "../backend";
import type {
  Account,
  AccountFeature,
  AccountSignature,
  ActiveReminder,
  AdminConsentLink,
  AppInfo,
  BackendError,
  BackgroundState,
  CalendarEvent,
  CheckItem,
  CheckResult,
  ComposeSignature,
  Contact,
  DataChangedEvent,
  DataKind,
  EmailAddress,
  ErrorCode,
  LabelPolicy,
  Message,
  MessageLabel,
  MessageSummary,
  Navigate,
  OrgSetupAdmin,
  OrgSetupChoices,
  OrgSetupInfo,
  OrgSetupPlan,
  OrgSetupResult,
  OrgSetupScript,
  OrgSetupScriptRequest,
  OrgSetupStatus,
  OrganizationSignature,
  PermissionRequest,
  PermissionRequiredReason,
  ProtectionSetup,
  PushStatus,
  RemindersChangedEvent,
  ReplyKind,
  SearchHit,
  SearchOptions,
  ServerSearchResult,
  ServerSearchStatus,
  Settings,
  SignInBlockedReason,
  SnoozeFor,
  SyncStatus,
  Task,
  UpdateState,
} from "../types";
import { cleanFamily } from "../../lib/fonts";
import {
  ACTION_NAMES,
  CHECK_VALID_DAYS,
  MAX_CHECK_ITEMS,
  isEncrypted,
  labelTitle,
} from "../../modules/mail/protection";
import {
  FEATURE_COPY,
  ON_REQUEST_FEATURES,
  emailDomain,
  isConsumerDomain,
  isPersonalAccount,
} from "../../modules/settings/microsoft";
import {
  OWN_PREFIX,
  SETUP_APP_NAME,
  SIGNED_OUT,
  psQuote,
} from "../../modules/settings/orgsetup/copy";
import { checkDetails } from "./checkDetails";
import {
  type CatalogLabel,
  type MockState,
  DEMO_LABEL_IDS,
  createDemoState,
  demoLabelCatalog,
  protectedMessages,
  recount,
} from "./data";
import { hitPatterns } from "../../lib/search/highlight";
import {
  accountMatches,
  folderMatches,
  isEmptySearch,
  isImpossible,
  isNarrowEnoughForServers,
  parseSearch,
  ranks,
  resolveDates,
} from "../../lib/search/query";
import {
  type DemoOrgSetup,
  type DemoTenant,
  type OrgSetupSignInFailure,
  type PurviewLabel,
  type SetupContext,
  SETUP_CLIENT_ID,
  SetupLog,
  demoTenant,
  invalidChoice,
  makeChanges,
  mockSetupConsentUrl,
  orgSetupSignInError,
  orgSetupStatus,
  planOrgSetup,
  policyNames,
  setupConsentError,
} from "./orgsetup";
import { MAX_SNIPPETS, admits, matchesSearch, mockSnippet, rankHits } from "./searchQuery";
import { createWorstState } from "./worst";

export type { DemoOrgSetup, OrgSetupSignInFailure } from "./orgsetup";
export { mockSetupConsentUrl } from "./orgsetup";

const DEFAULT_SETTINGS: Settings = {
  theme: "system",
  readingPane: "right",
  remoteContent: "block",
  markReadAfterSeconds: 0,
  syncIntervalMinutes: 5,
  microsoftClientId: null,
  microsoftTenant: null,
  notificationsEnabled: true,
  interfaceFont: { kind: "default" },
  readingFont: { kind: "default" },
  textSize: "default",
  notifyScope: "inbox",
  notifyShowPreview: true,
  notifySound: true,
  mutedAccountIds: [],
  notificationsPausedUntil: null,
  remindersEnabled: true,
  defaultSnoozeMinutes: 5,
  instantPush: true,
  usePushRelay: true,
  runInBackground: true,
  startAtLogin: false,
  startHidden: true,
  showTrayIcon: true,
};

/** What a GNOME desktop with a few extra fonts reports, sorted as the Rust side sorts. */
export const DEMO_FONTS = [
  "Adwaita Mono",
  "Adwaita Sans",
  "Atkinson Hyperlegible Next",
  "Cantarell",
  "DejaVu Sans",
  "DejaVu Sans Mono",
  "DejaVu Serif",
  "EB Garamond",
  "Fira Code",
  "Fira Sans",
  "IBM Plex Sans",
  "IBM Plex Serif",
  "Inter",
  "JetBrains Mono",
  "Lato",
  "Liberation Mono",
  "Liberation Sans",
  "Liberation Serif",
  "Literata",
  "Merriweather",
  "Noto Naskh Arabic",
  "Noto Sans",
  "Noto Sans Arabic",
  "Noto Sans CJK JP",
  "Noto Sans Devanagari",
  "Noto Sans Hebrew",
  "Noto Sans Mono",
  "Noto Serif",
  "Noto Serif CJK JP",
  "Open Sans",
  "Roboto",
  "Roboto Slab",
  "Source Code Pro",
  "Source Sans 3",
  "Source Serif 4",
  "Ubuntu",
  "Ubuntu Mono",
  "Ubuntu Sans",
];

/** Mirrors `is_valid_font_family` in crates/core/src/model.rs. */
function validFamily(family: string | undefined): boolean {
  return family === undefined || cleanFamily(family) === family;
}

function fail(code: ErrorCode, message: string, details?: BackendError["details"]): never {
  const error: BackendError = { code, message, ...(details ? { details } : {}) };
  throw error;
}

function clone<T>(value: T): T {
  return structuredClone(value);
}

function summary(m: Message): MessageSummary {
  return {
    id: m.id,
    accountId: m.accountId,
    folderId: m.folderId,
    remoteId: m.remoteId,
    conversationId: m.conversationId,
    subject: m.subject,
    from: m.from,
    to: m.to,
    cc: m.cc,
    preview: m.preview,
    receivedAt: m.receivedAt,
    isRead: m.isRead,
    isFlagged: m.isFlagged,
    isDraft: m.isDraft,
    hasAttachments: m.hasAttachments,
    importance: m.importance,
  };
}

const REMOTE_IMG = /<img\b[^>]*\bsrc=["']https?:[^"']*["'][^>]*>/gi;

function newestFirst(a: { receivedAt: string }, b: { receivedAt: string }) {
  return b.receivedAt.localeCompare(a.receivedAt);
}

/** Like the core's searches: newest first, then by id. */
function searchOrder(a: Message, b: Message) {
  return newestFirst(a, b) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
}

/** Like the core: what a local search returns from the index (server hits come on top). */
const LOCAL_SEARCH_LIMIT = 200;
/** Like the core: how many server hits a local search takes in. */
const MAX_INCLUDED_IDS = 1000;
/** Like the core: a server search's page, per account. */
const SERVER_SEARCH_PAGE = 50;
/** Like the core: most pages one server search call reads. */
const SERVER_SEARCH_MAX_PAGES = 5;
/** Like the core: how many ended search ids are remembered. */
const RETIRED_SEARCHES = 32;

// The demo's text size (see setZoom). CSS zoom on the root scales the page,
// but media queries and viewport units go on measuring the whole window,
// where the desktop webview's zoom has them measure the zoomed one. So the
// lengths in media queries scale up (at 1.2, a 900px query matches a 1080px
// window) and viewport units down (100vh is the window's height again).
// Style sheets that come later (the modules load lazily) or change (hot
// reload) follow.
const MEDIA_LENGTH = /(\d*\.?\d+)(px)\b/g;
const VIEWPORT_LENGTH = /(\d*\.?\d+)([sld]?v(?:w|h|min|max))\b/g;
const zoomed = { factor: 1, original: new WeakMap<CSSRule, string>(), watching: false };

function scaleLengths(text: string, lengths: RegExp, by: number): string {
  if (by === 1) return text;
  return text.replace(
    lengths,
    (_, n: string, unit: string) => `${Math.round(Number(n) * by * 1000) / 1000}${unit}`,
  );
}

function zoomViewport(factor: number): void {
  zoomed.factor = factor;
  const visit = (rules: CSSRuleList) => {
    for (const rule of Array.from(rules)) {
      if (rule instanceof CSSMediaRule) {
        const text = zoomed.original.get(rule) ?? rule.media.mediaText;
        zoomed.original.set(rule, text);
        const scaled = scaleLengths(text, MEDIA_LENGTH, factor);
        if (rule.media.mediaText !== scaled) rule.media.mediaText = scaled;
      } else if (rule instanceof CSSStyleRule) {
        const text = zoomed.original.get(rule) ?? rule.style.cssText;
        const scaled = scaleLengths(text, VIEWPORT_LENGTH, 1 / factor);
        if (scaled !== text) zoomed.original.set(rule, text);
        if (zoomed.original.has(rule) && rule.style.cssText !== scaled) rule.style.cssText = scaled;
      }
      if ("cssRules" in rule) visit(rule.cssRules as CSSRuleList);
    }
  };
  for (const sheet of Array.from(document.styleSheets)) {
    let rules: CSSRuleList;
    try {
      rules = sheet.cssRules;
    } catch {
      continue; // Another origin's style sheet can't be read; the app has none.
    }
    visit(rules);
  }
  if (zoomed.watching || factor === 1) return;
  zoomed.watching = true;
  const again = () => zoomViewport(zoomed.factor);
  new MutationObserver(again).observe(document.head, {
    childList: true,
    subtree: true,
    characterData: true,
  });
  // A linked style sheet (in a build) has its rules once it has loaded.
  document.addEventListener(
    "load",
    (e) => {
      if (e.target instanceof HTMLLinkElement) again();
    },
    true,
  );
}

/** Microsoft's tenant for personal accounts, which never have company signatures. */
const CONSUMER_TENANT = "9188040d-6c67-4c5b-b112-36a304b66dad";
const MAX_SIGNATURE_BYTES = 512 * 1024;
/** Starting a message asks an organization that signs again after this long. */
const COMPOSE_RECHECK_MS = 60 * 60_000;
/** Like the core: a check that ended less than this long ago isn't repeated. */
const CHECK_INTERVAL_MS = 60_000;
/** Like the core: the shortest and the longest wait a service's Retry-After can ask for. */
const MIN_RETRY_AFTER_MS = 60_000;
const MAX_RETRY_AFTER_MS = 6 * 60 * 60_000;
/** Like the core: a time kept more than this after now was kept while the clock was ahead. */
const CLOCK_SLACK_MS = 60_000;

/** What an organization says when asked for its signature, when it answers. */
type OrganizationReply = Pick<OrganizationSignature, "state" | "html" | "onReplies">;

/**
 * What an organization says when asked for its signature: "unreachable" when
 * it can't be asked, and `{ retryAfter }` when its service is busy and says
 * to ask again in that many seconds.
 */
export type OrganizationAnswer = OrganizationReply | "unreachable" | { retryAfter: number };

const NOT_USED: OrganizationReply = {
  state: "notUsed",
  html: null,
  onReplies: false,
};

/**
 * Like the core's signature sanitizer, roughly: no scripts, frames or event
 * handlers, and a signature with neither text nor a picture is none.
 */
function cleanSignature(html: string): string {
  const template = document.createElement("template");
  template.innerHTML = html;
  const root = template.content;
  root.querySelectorAll("script, iframe, object, embed, form").forEach((el) => el.remove());
  for (const el of root.querySelectorAll("*")) {
    for (const attribute of Array.from(el.attributes)) {
      if (/^on/i.test(attribute.name)) el.removeAttribute(attribute.name);
    }
  }
  if (!root.textContent?.trim() && !root.querySelector("img")) return "";
  return template.innerHTML;
}

/**
 * States the demo's Microsoft account's company signature can show
 * (`?signature=` in the URL): the organization adds it on send (to
 * replies and forwards too), hasn't allowed Cumulas Mail, couldn't be
 * reached since its last answer, doesn't use Cumulas Signatures, or
 * couldn't be reached to ask at all.
 */
export type DemoSignature = "addedOnSend" | "notAllowed" | "stale" | "notUsed" | "unreachable";

/** Push states the demo can show for its Microsoft account (`?push=` in the URL). */
export type DemoPush = "consent" | "refused" | "unreachable";

/**
 * An update of the Flatpak the demo has installed (`?update=` in the URL):
 * restarting into it works ("installed"), or fails and can be tried again
 * ("restartfails").
 */
export type DemoUpdate = "installed" | "restartfails";

/** The installed update's commit in the demo. */
export const DEMO_UPDATE_COMMIT = "demo-commit-2";

const RESTART_FAILED = "Couldn't restart. Try again, or quit Cumulas Mail and open it again.";

/** The optional features of a Microsoft account, in the order the core lists them. */
const FEATURES: readonly AccountFeature[] = [
  "calendar",
  "contacts",
  "tasks",
  "mailboxSettings",
  "sensitivityLabels",
];

function inOrder(features: readonly AccountFeature[]): AccountFeature[] {
  return FEATURES.filter((f) => features.includes(f));
}

/** Cumulas Mail's published app registration (connector.rs). */
const PUBLISHED_CLIENT_ID = "10abb2b9-1a7a-4dbc-ba0f-c9a0de9088ad";
/** An organization's own app registration (`?customclient`). */
const CUSTOM_CLIENT_ID = "11111111-2222-3333-4444-555555555555";
/** Where Microsoft sends an administrator who approved the published registration. */
const ADMIN_PAGE_URL = "https://flatpak.cumulas.co/admin-consent/";
/** Every Graph permission, in the order the approval link names them. */
const GRAPH_SCOPES = [
  "User.Read",
  "Mail.ReadWrite",
  "Mail.Send",
  "MailboxSettings.Read",
  "Calendars.ReadWrite",
  "Contacts.ReadWrite",
  "Tasks.ReadWrite",
  "InformationProtectionPolicy.Read",
];

/**
 * Like the core's admin consent link: for the published registration it
 * covers the push service too and comes back to the page for administrators;
 * for an organization's own (`customClient`), it comes back to localhost.
 */
export function mockAdminConsentUrl(tenant: string, customClient = false): string {
  const clientId = customClient ? CUSTOM_CLIENT_ID : PUBLISHED_CLIENT_ID;
  const scopes = [
    "openid",
    "profile",
    "email",
    "offline_access",
    ...GRAPH_SCOPES.map((s) => `https://graph.microsoft.com/${s}`),
    ...(customClient ? [] : [`api://${clientId}/Sync.Receive`]),
  ];
  // Encoded as the core's form_urlencoded does: spaces become "+".
  const query = new URLSearchParams({
    client_id: clientId,
    scope: scopes.join(" "),
    redirect_uri: customClient ? "http://localhost" : ADMIN_PAGE_URL,
  });
  return `https://login.microsoftonline.com/${tenant}/v2.0/adminconsent?${query.toString()}`;
}

/** Like the core's: where an administrator approves Cumulas Mail for `tenant`. */
function adminConsentLink(tenant: string, customClient: boolean): AdminConsentLink {
  return {
    adminConsentUrl: mockAdminConsentUrl(tenant, customClient),
    tenant,
    adminPageUrl: customClient ? null : ADMIN_PAGE_URL,
  };
}

/** The core's messages for sign-ins the organization's settings block. */
const BLOCKED_MESSAGES: Record<SignInBlockedReason, string> = {
  notAssigned:
    "Your organization hasn't given you access to Cumulas Mail. Ask your IT team to assign you to the app.",
  appDisabled:
    "Your organization has blocked Cumulas Mail. Ask your IT team whether they can allow it.",
  appNotSetUp:
    "Microsoft 365 in your organization isn't set up for Cumulas Mail yet. Ask your IT team to approve it.",
  serviceNotEnabled:
    "Your organization's Microsoft 365 doesn't include a service Cumulas Mail needs, usually Exchange Online mail. Ask your IT team to check your subscription.",
  securityInfo:
    "Your organization needs you to finish setting up your sign-in security info, such as multifactor authentication. Ask your IT team for help.",
  conditionalAccess:
    "Your organization's sign-in rules didn't allow this sign-in, for example because of the device, the app or where you're signing in from. Ask your IT team what's required.",
};

/** Demo sign-ins the organization blocks, by the login hint's start, with Microsoft's code. */
const BLOCKED_HINTS: [string, SignInBlockedReason, number][] = [
  ["unassigned@", "notAssigned", 50105],
  ["disabled@", "appDisabled", 7000112],
  ["notsetup@", "appNotSetUp", 65005],
  ["noservice@", "serviceNotEnabled", 650052],
  ["mfa@", "securityInfo", 53004],
  ["device@", "conditionalAccess", 53000],
  ["blocked@", "conditionalAccess", 53003],
];

const NO_ADMIN_NEEDED = "Personal Microsoft accounts don't need an administrator's approval.";

/**
 * The demo work account's protection checks (`?protection=` in the URL):
 * Encrypt, Do Not Forward and most labels passed ("setup"); the same with
 * Encrypt's check too old ("expired"); a check of Encrypt waiting for its
 * test message ("checking"); or mail going through the Cumulas Signatures
 * relay, which Microsoft 365 treats as outside mail when it comes back, so
 * Encrypt and Do Not Forward passed and labels didn't ("relay").
 */
export type DemoProtection = "setup" | "checking" | "expired" | "relay";

// The core's messages about sending protected mail and its checks.
const WORK_ACCOUNTS_ONLY = "Only Microsoft 365 work and school accounts can send protected mail.";
const OUTLOOK_ONLY = "Only Microsoft 365 messages open in Outlook on the web.";
const PROTECTED_ORIGINAL =
  "This message is protected. Reply to it or forward it in Outlook on the web.";
const ENCRYPTED_ORIGINAL =
  "This message is encrypted. Cumulas Mail can't reply to it or forward it without removing the encryption.";
const OPEN_CONTAINER =
  "This file holds a protected message's content, which Cumulas Mail can't open. Open the message in Outlook on the web.";
const NO_TEST_MESSAGE = "There's no test message to open. Check again.";
// The core's messages about organization setup.
const ORG_SETUP_WORK_ONLY =
  "Only Microsoft 365 work and school accounts have an organization to set up.";
const SETUP_NOT_AVAILABLE =
  "Automatic setup isn't available in this version of Cumulas Mail. Use the PowerShell script instead.";
const READ_FIRST = "Read your organization's setup first: select Read again.";
/** Cumulas Signatures' page for administrators (AppInfo.signatureServer). */
const SIGNATURES_MANAGE_URL = "https://signature.cumulas.co/manage";
/** The demo organization's administrator, and their Microsoft Entra object ID. */
const SETUP_ADMIN = { name: "sam.rivera", displayName: "Sam Rivera" };
const SETUP_ADMIN_OBJECT_ID = "5f3a9c2e-7b1d-4e8f-a6c4-2d9b0e1f3a7c";
/** A label Purview lists that isn't published to the demo's work account. */
const BOARD_LABEL: PurviewLabel = {
  id: "c3d4e5f6-0718-4a29-b3c4-d5e6f7a8b9c0",
  name: "Board",
  title: "Highly Confidential › Board",
  encrypts: true,
};
/** How long after setup changed auto-labeling policies the demo's label checks are too soon to tell. */
const TOO_SOON_MS = 2 * 60_000;

/** What a check decided, as the core keeps it: never "expired" or "waiting". */
type CheckRecord = Omit<CheckResult, "passedAt">;

const NOT_CHECKED: CheckRecord = {
  state: "notChecked",
  at: null,
  note: null,
  testMessage: false,
  otherLabelId: null,
  otherLabelOnControl: false,
  changedAt: null,
};
const NO_POLICY: LabelPolicy = {
  required: false,
  defaultLabelId: null,
  justifyDowngrade: false,
  moreInfoUrl: null,
};
const DAY_MS = 86_400_000;

/** A work account's checks and labels, as the core keeps them. */
interface MockProtection {
  /** What the last check decided, which stands while a new one waits. */
  encrypt: CheckRecord;
  doNotForward: CheckRecord;
  labels: Map<string, CheckRecord>;
  /** What Microsoft 365 last listed; null until then, or without the permission. */
  catalog: { labels: CatalogLabel[]; policy: LabelPolicy } | null;
  listedAt: string | null;
  error: string | null;
  /** The check waiting for its messages, when it sent them, and how often they were looked for. */
  run: { items: CheckItem[]; sentAt: string; looks: number } | null;
}

/**
 * How each check of the demo's "setup" turned out: most arrived protected,
 * and Project Atlas's request reached delivery but no policy acted on it.
 * Through the relay (`relay`), labels came back as outside mail.
 */
function demoOutcome(item: CheckItem, at: string, relay = false): CheckRecord {
  const result = (state: CheckResult["state"], note: CheckResult["note"] = null): CheckRecord => ({
    ...NOT_CHECKED,
    state,
    at,
    note,
    testMessage: true,
  });
  if (item.kind === "action") return result("applied");
  // Acme labels all mail Public by default: the control arrives labeled too.
  if (item.labelId === DEMO_LABEL_IDS.public) return result("unknown", "otherRule");
  if (relay) return result("notApplied", "relayNotInternal");
  if (item.labelId === DEMO_LABEL_IDS.atlas) return result("notApplied", "requestIgnored");
  return result("applied");
}

function checkKey(item: CheckItem): string {
  return item.kind === "action" ? item.action : item.labelId;
}

/** Like the core: what a check decided as of now; a pass older than CHECK_VALID_DAYS is expired. */
function decidedState(record: CheckRecord): CheckResult["state"] {
  return record.state === "applied" &&
    record.at &&
    Date.now() - Date.parse(record.at) > CHECK_VALID_DAYS * DAY_MS
    ? "expired"
    : record.state;
}

/** Sizes as the core's diagnostics give them. */
function diagnosticSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} bytes`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export interface MockOptions {
  /** Artificial latency in ms for every call (default 0). */
  latency?: number;
  /** How long a server search takes, in ms (default: `latency`). */
  serverSearchLatency?: number;
  /** Messages a server search page holds, before the filters are applied (default 50). */
  serverSearchPage?: number;
  /** Most pages one server search call reads (default 5). */
  serverSearchMaxPages?: number;
  /** Start without any accounts (shows onboarding). */
  empty?: boolean;
  /** Start with worst-case data (long, empty, huge and non-Latin values). */
  worst?: boolean;
  /** Show the Microsoft account's push in this state. */
  push?: DemoPush;
  /** Show the Microsoft account's company signature in this state. */
  signature?: DemoSignature;
  /** IMAP accounts' server searches fail, as with an unreachable server (`?searchfail`). */
  searchFail?: boolean;
  /** Add a task reminder that went off 12 minutes ago (`?reminders`), besides the events' own. */
  overdueReminder?: boolean;
  /** What the desktop reports (the tray, the portal…). */
  background?: Partial<
    Pick<BackgroundState, "backgroundAllowed" | "autostart" | "trayAvailable" | "notifications">
  >;
  /** The demo's Microsoft account lacks these features' permissions (`?missing=calendar,contacts,tasks,mailboxSettings`). */
  missing?: AccountFeature[];
  /** Microsoft 365 refuses these features of the demo's Microsoft account (`?refused=calendar,contacts,tasks`). */
  refused?: AccountFeature[];
  /** Microsoft sign-in uses an organization's own app registration: approval links return to localhost and there's no page for administrators (`?customclient`). */
  customClient?: boolean;
  /** Adds protected and labeled demo mail (`?protected`). */
  protectedMail?: boolean;
  /** The demo work account's protection checks (`?protection=setup|checking|expired|relay`); default: labels listed, nothing checked. */
  protection?: DemoProtection;
  /** The work account's organization as setup finds it (`?orgsetup=`); default "fresh". */
  orgSetup?: DemoOrgSetup;
  /** An update of the Flatpak is installed (`?update=installed|restartfails`). */
  update?: DemoUpdate;
  /** Started by a restart into an update (`?updated`). */
  restartedToUpdate?: boolean;
  /** What a restart that works does; the browser demo reloads as restarted. */
  onRestart?: () => void;
}

/** The demo's Microsoft account's push status for `?push=`. */
function demoPush(
  kind: DemoPush,
  customClient: boolean,
): Pick<PushStatus, "mode" | "health" | "detail" | "consentRefused" | "adminConsent"> {
  const polling = { kind: "polling", everySecs: 30 } as const;
  switch (kind) {
    case "consent":
      return {
        mode: polling,
        health: { kind: "consentMissing" },
        detail: "Sign in again to turn on instant updates.",
        consentRefused: false,
        adminConsent: null,
      };
    case "refused": {
      const link = adminConsentLink("acme.example", customClient);
      return {
        mode: polling,
        health: { kind: "consentMissing" },
        detail:
          "Your organization requires an administrator to approve instant updates. Send the approval link to your IT department; until then, the Inbox is checked every 30 seconds.",
        consentRefused: true,
        adminConsent: {
          url: link.adminConsentUrl,
          tenant: link.tenant,
          adminPageUrl: link.adminPageUrl,
        },
      };
    }
    case "unreachable":
      return {
        mode: polling,
        health: { kind: "degraded", reason: "Could not reach the push service." },
        detail: null,
        consentRefused: false,
        adminConsent: null,
      };
  }
}

export class MockBackend implements Backend {
  private state: MockState;
  private settings: Settings = { ...DEFAULT_SETTINGS };
  private dataHandlers = new Set<(e: DataChangedEvent) => void>();
  private syncHandlers = new Set<(s: SyncStatus) => void>();
  private pushHandlers = new Set<(s: PushStatus) => void>();
  private navigateHandlers = new Set<(to: Navigate) => void>();
  private backgroundHandlers = new Set<(state: BackgroundState) => void>();
  private quitHandlers = new Set<() => void>();
  private updateHandlers = new Set<(state: UpdateState) => void>();
  private restartHandlers = new Set<() => void>();
  private update: UpdateState;
  /** Restarting fails (`?update=restartfails`). */
  private failRestart: boolean;
  private restartedToUpdate: boolean;
  private onRestart: (() => void) | undefined;
  private reminderHandlers = new Set<(event: RemindersChangedEvent) => void>();
  /** What became of reminders: snoozed until, or dismissed. */
  private reminderStates = new Map<string, { snoozedUntil?: number; dismissed?: boolean }>();
  /** The keys due, and all active, when the reminders were last looked at. */
  private dueReminders = new Set<string>();
  private lastReminders = "[]";
  /** When each account's push status last changed. */
  private pushSince = new Map<string, string>();
  /** Push statuses set by tests or the demo, by account. */
  private pushOverrides = new Map<string, Partial<PushStatus>>();
  private desktop: Pick<
    BackgroundState,
    "backgroundAllowed" | "autostart" | "trayAvailable" | "notifications"
  >;
  private pendingNavigation: Navigate | null = null;
  /** Calls the desktop app would have acted on, for tests. */
  readonly calls = {
    quit: 0,
    quitCanceled: 0,
    restart: 0,
    testNotifications: 0,
    /** What was opened in Outlook on the web: message ids, or `check:<account>:<item>`. */
    openedInOutlook: [] as string[],
    /** How often the setup sign-in was forgotten (endOrgSetup). */
    orgSetupEnded: 0,
  };
  private seq = 1000;
  private latency: number;
  private pendingSignIn: (() => void) | null = null;
  /** Microsoft sign-in uses an organization's own app registration (`?customclient`). */
  private customClient: boolean;
  /** "workflow@…" asked for approval once: Microsoft's page now offers no request. */
  private approvalRequested = false;
  /** Accounts a simulated sync runs for: their statuses say so, as the core's do. */
  private syncing = new Set<string>();
  /** Messages sent through the mock, for tests. */
  readonly sent: Parameters<Backend["sendMessage"]>[0][] = [];
  /** Each account's signatures, once asked for. */
  private signatures = new Map<string, AccountSignature>();
  /** What each account's organization says when asked; Cumulas Signatures isn't used by default. */
  private organizationAnswers = new Map<string, OrganizationAnswer>();
  /** When each account's organization was last asked (ms). */
  private signatureChecks = new Map<string, number>();
  /** Until when each account's organization's service said to wait (ms). */
  private signatureRetries = new Map<string, number>();
  private serverSearchLatency: number;
  private serverSearchPage: number;
  private serverSearchMaxPages: number;
  /**
   * The current server search, like the core's: a new id replaces it,
   * canceling the old one. Per account, the matches its next page takes from.
   */
  private serverSearch: { id: string; rest: Map<string, string[]> } | null = null;
  /**
   * Ids of searches that ended, like the core's: a call for one that comes
   * late is canceled instead of replacing the search that followed it.
   */
  private retiredSearches: string[] = [];
  /** Holds server searches while set (pauseServerSearch). */
  private serverSearchGate: Promise<void> | null = null;
  /** Messages opened: local search looks in their text, in the others' previews. */
  private opened = new Set<string>();
  /** Messages deleted on the server unbeknownst to the app (removeFromServer). */
  private goneFromServer = new Set<string>();
  /** How each account's server search fails, when it does (failServerSearch). */
  private serverSearchFailures = new Map<string, BackendError>();
  /** Each work account's protection checks and labels, once looked at. */
  private protection = new Map<string, MockProtection>();
  /** The checks the demo's work account starts with (`?protection=`). */
  private demoProtection: { accountId: string; kind: DemoProtection } | null = null;
  /** How work accounts' organizations start out for setup (`?orgsetup=`). */
  private orgSetupKind: DemoOrgSetup;
  /** Each organization as setup finds it, by tenant, once looked at. */
  private organizations = new Map<string, DemoTenant>();
  /** The setup sign-in, like the core's: at most one, for one organization; whether it read yet. */
  private orgSetupSession: { tenantId: string; admin: OrgSetupAdmin; read: boolean } | null = null;
  /** The calls and sign-ins "Copy setup details" names. */
  private setupLog = new SetupLog();
  /** Ends the setup sign-in waiting in the browser, as Cancel does. */
  private pendingOrgSetupSignIn: (() => void) | null = null;
  /** Holds setup sign-ins in the browser while set (holdOrgSetupSignIn). */
  private orgSetupSignInGate: Promise<void> | null = null;
  /** How setup sign-ins fail, when they do (failOrgSetupSignIn). */
  private orgSetupSignInFailure: OrgSetupSignInFailure | null = null;
  /** Organizations whose administrator opened the setup app's approval link. */
  private setupApproved = new Set<string>();
  /** When setup last changed each organization's rules and policies from here, by tenant. */
  private orgSetupChanged = new Map<
    string,
    { rulesChangedAt: string | null; policiesChangedAt: string | null }
  >();

  constructor(options: MockOptions = {}) {
    this.latency = options.latency ?? 0;
    this.serverSearchLatency = options.serverSearchLatency ?? this.latency;
    this.serverSearchPage = Math.max(1, options.serverSearchPage ?? SERVER_SEARCH_PAGE);
    this.serverSearchMaxPages = Math.max(
      1,
      options.serverSearchMaxPages ?? SERVER_SEARCH_MAX_PAGES,
    );
    this.state = options.empty
      ? emptyState()
      : options.worst
        ? createWorstState()
        : createDemoState();
    this.desktop = {
      backgroundAllowed: true,
      autostart: "xdgAutostartFile",
      trayAvailable: true,
      notifications: "freedesktop",
      ...options.background,
    };
    this.customClient = options.customClient ?? false;
    this.orgSetupKind = options.orgSetup ?? "fresh";
    this.update = options.update
      ? { kind: "installed", commit: DEMO_UPDATE_COMMIT, restart: "idle" }
      : { kind: "none" };
    this.failRestart = options.update === "restartfails";
    this.restartedToUpdate = options.restartedToUpdate ?? false;
    this.onRestart = options.onRestart;
    if (options.protectedMail && !options.empty && !options.worst) {
      this.state.messages.push(...protectedMessages());
      recount(this.state);
    }
    const microsoft = this.state.accounts.find((a) => a.kind === "microsoft");
    if (options.protection && microsoft) {
      this.demoProtection = { accountId: microsoft.id, kind: options.protection };
    }
    if (options.push && microsoft) {
      this.pushOverrides.set(microsoft.id, demoPush(options.push, this.customClient));
    }
    if (microsoft) {
      // Like the core: the two lists never share a feature, and Microsoft 365
      // never refuses the mailbox time zone (it falls back instead).
      const missing = inOrder(options.missing ?? []);
      const refused = inOrder(options.refused ?? []).filter(
        (f) => f !== "mailboxSettings" && !missing.includes(f),
      );
      microsoft.needsPermission = missing;
      microsoft.refusedFeatures = refused;
      for (const feature of [...missing, ...refused]) this.dropFeatureData(microsoft.id, feature);
    }
    for (const signature of this.state.signatures ?? []) {
      // As the core keeps them: sanitized.
      const organization = signature.organization;
      if (organization.html) organization.html = cleanSignature(organization.html);
      this.signatures.set(signature.accountId, signature);
      const { state, html, onReplies } = organization;
      if (state !== "unsupported" && state !== "unchecked") {
        this.organizationAnswers.set(signature.accountId, { state, html, onReplies });
      }
    }
    if (options.signature && microsoft) this.demoSignature(microsoft.id, options.signature);
    // An account whose sync fails can't be searched on its server either
    // (`?worst`), as in the app.
    for (const account of this.state.accounts) {
      if (account.syncError || (options.searchFail && account.kind === "imap")) {
        const host =
          account.settings.kind === "imap" ? account.settings.imapHost : "Microsoft Graph";
        this.serverSearchFailures.set(account.id, {
          code: "network",
          message: `Could not reach ${host}.`,
        });
      }
    }
    if (options.overdueReminder) {
      const metrics = this.state.tasks.find((t) => t.title === "Send Q3 metrics to Dana");
      if (metrics) metrics.reminderAt = new Date(Date.now() - 12 * 60_000).toISOString();
    }
    const active = this.reminders();
    this.dueReminders = new Set(active.filter((r) => r.snoozedUntil === null).map((r) => r.key));
    this.lastReminders = JSON.stringify(active);
  }

  private async delay(): Promise<void> {
    if (this.latency > 0) await new Promise((r) => setTimeout(r, this.latency));
  }

  private id(prefix: string): string {
    this.seq += 1;
    return `${prefix}-${this.seq}`;
  }

  private emit(kinds: DataKind[], accountId: string | null = null): void {
    const event = { kinds, accountId };
    queueMicrotask(() => this.dataHandlers.forEach((h) => h(event)));
    // The scheduler looks again when events or tasks change.
    if (kinds.includes("calendar") || kinds.includes("tasks")) this.checkReminders();
  }

  private message(id: string): Message {
    return this.state.messages.find((m) => m.id === id) ?? fail("notFound", "message not found");
  }

  private account(id: string): Account {
    return this.state.accounts.find((a) => a.id === id) ?? fail("notFound", "account not found");
  }

  // --- app --------------------------------------------------------------------

  async getAppInfo(): Promise<AppInfo> {
    return {
      version: "0.1.0-demo",
      persistentSecrets: true,
      microsoftClientIdBuiltIn: true,
      syncServer: "sync.cumulas.co",
      signatureServer: "signature.cumulas.co",
      restartedToUpdate: this.restartedToUpdate,
    };
  }

  async openLink(url: string) {
    // The demo's administrator approves the setup app on its approval page.
    const tenant = /^https:\/\/login\.microsoftonline\.com\/([^/?]+)\/v2\.0\/adminconsent\?/.exec(
      url,
    )?.[1];
    if (tenant && url === mockSetupConsentUrl(tenant)) this.setupApproved.add(tenant);
    window.open(url, "_blank", "noopener,noreferrer");
  }

  async quitApp() {
    // A page can't quit the browser; the count is for tests.
    this.calls.quit += 1;
  }

  async cancelQuit() {
    this.calls.quitCanceled += 1;
  }

  // --- accounts & settings ------------------------------------------------------

  async listAccounts() {
    await this.delay();
    return clone(this.state.accounts);
  }

  async addImapAccount(input: Parameters<Backend["addImapAccount"]>[0]) {
    await this.delay();
    if (!input.email.includes("@")) fail("invalidInput", "Enter a valid email address.");
    if (input.password === "wrong")
      fail("authRequired", "The server rejected the user name or password.");
    if (this.state.accounts.some((a) => a.email.toLowerCase() === input.email.toLowerCase())) {
      fail("invalidInput", `${input.email} has already been added.`);
    }
    const account: Account = {
      id: this.id("acct"),
      kind: "imap",
      displayName: input.displayName || input.email,
      email: input.email,
      color: "#8a4f7d",
      settings: { kind: "imap", ...input.settings },
      createdAt: new Date().toISOString(),
      lastSyncedAt: new Date().toISOString(),
      syncError: null,
      needsPermission: [],
      refusedFeatures: [],
    };
    this.state.accounts.push(account);
    for (const [name, role] of [
      ["Inbox", "inbox"],
      ["Drafts", "drafts"],
      ["Sent", "sent"],
      ["Archive", "archive"],
      ["Junk", "junk"],
      ["Trash", "trash"],
    ] as const) {
      const id = this.id("folder");
      this.state.folders.push({
        id,
        accountId: account.id,
        remoteId: id,
        parentId: null,
        name,
        role,
        unreadCount: 0,
        totalCount: 0,
      });
    }
    this.state.calendars.push({
      id: this.id("cal"),
      accountId: account.id,
      remoteId: "local:calendar",
      name: "Calendar",
      color: account.color,
      canEdit: true,
      isDefault: true,
    });
    this.state.taskLists.push({
      id: this.id("tl"),
      accountId: account.id,
      remoteId: "local:tasks",
      name: "Tasks",
      isDefault: true,
    });
    this.emit(["accounts", "mail", "calendar", "tasks"], account.id);
    return clone(account);
  }

  async addMicrosoftAccount(loginHint?: string, permissions: PermissionRequest = { kind: "all" }) {
    await this.delay();
    const hint = loginHint?.trim() ?? "";
    this.demoSignInFailure(hint, permissions);
    if (hint.toLowerCase().startsWith("slow@")) {
      // Never finishes on its own, like a browser tab the user walked away from.
      await new Promise<void>((resolve) => {
        this.pendingSignIn = resolve;
      });
      fail("signInCancelled", "Sign-in was canceled.");
    }
    const email = hint.includes("@") ? hint : "new.user@contoso.example";
    // Like the core, which finds the account by Microsoft's user id: signing
    // in again updates it.
    const existing = this.state.accounts.find(
      (a) => a.kind === "microsoft" && a.email.toLowerCase() === email.toLowerCase(),
    );
    const personal = existing
      ? isPersonalAccount(existing)
      : isConsumerDomain(emailDomain(email) ?? "");
    if (
      personal &&
      permissions.kind === "feature" &&
      ON_REQUEST_FEATURES.includes(permissions.feature)
    ) {
      fail("invalidInput", "Sensitivity labels are only for work and school accounts.");
    }
    if (existing) {
      const before = [...existing.needsPermission, ...existing.refusedFeatures];
      // Every sign-in checks refused features again.
      existing.refusedFeatures = [];
      // "All" never asks for what's turned on by itself; it keeps what it had.
      if (permissions.kind === "all") {
        existing.needsPermission = existing.needsPermission.filter((f) =>
          ON_REQUEST_FEATURES.includes(f),
        );
      } else if (permissions.kind === "feature") {
        existing.needsPermission = existing.needsPermission.filter(
          (f) => f !== permissions.feature,
        );
      }
      this.emit(["accounts"], existing.id);
      // Like the core, which syncs the account after a sign-in: what came
      // back arrives with that sync, after the sign-in returns.
      const back = before.filter((f) => !existing.needsPermission.includes(f));
      this.simulateSync(existing.id, () =>
        back.flatMap((feature) => this.addDefaultContainer(existing, feature) ?? []),
      );
      return clone(existing);
    }
    // Like the core: personal accounts can't have sensitivity labels, and
    // they're asked for only when turned on by themselves.
    const features = FEATURES.filter((f) => !personal || !ON_REQUEST_FEATURES.includes(f));
    const account: Account = {
      id: this.id("acct"),
      kind: "microsoft",
      displayName: email.split("@")[0] ?? email,
      email,
      color: "#2b7aa8",
      settings: {
        kind: "microsoft",
        tenantId: personal ? CONSUMER_TENANT : "demo",
        userId: this.id("user"),
      },
      createdAt: new Date().toISOString(),
      lastSyncedAt: null,
      syncError: null,
      needsPermission:
        permissions.kind === "mailOnly"
          ? features
          : permissions.kind === "feature"
            ? features.filter((f) => f !== permissions.feature)
            : features.filter((f) => ON_REQUEST_FEATURES.includes(f)),
      refusedFeatures: [],
    };
    this.state.accounts.push(account);
    this.state.folders.push({
      id: this.id("folder"),
      accountId: account.id,
      remoteId: "inbox",
      parentId: null,
      name: "Inbox",
      role: "inbox",
      unreadCount: 0,
      totalCount: 0,
    });
    this.emit(["accounts", "mail"], account.id);
    return clone(account);
  }

  /**
   * Demo hooks, by the login hint's start: how Microsoft answers sign-ins in
   * organizations set up in different ways (docs/microsoft-365.md). Any
   * other address signs in.
   */
  private demoSignInFailure(hint: string, permissions: PermissionRequest): void {
    const lower = hint.toLowerCase();
    const tenant = hint.split("@")[1] || "organizations";
    const link = adminConsentLink(tenant, this.customClient);
    const needsApproval = (canRequestApproval: boolean): never =>
      fail(
        "adminConsentRequired",
        canRequestApproval
          ? "Your organization requires an administrator to approve Cumulas Mail before you can sign in. You can request approval on Microsoft's sign-in page, or send the approval link to your IT department."
          : "Your organization requires an administrator to approve Cumulas Mail before you can sign in. Send the approval link to your IT department.",
        { ...link, canRequestApproval },
      );
    const declined = (): never =>
      fail(
        "consentDeclined",
        "Cumulas Mail needs the requested permissions to show your mail, calendar, contacts and tasks. If you asked your administrator for approval, sign in again once it has been granted.",
      );
    // Microsoft's default: mail needs an administrator's approval too.
    if (lower.startsWith("admin@")) needsApproval(false);
    // An organization that approved mail but not the rest.
    if (lower.startsWith("partial@") && permissions.kind !== "mailOnly") needsApproval(false);
    if (lower.startsWith("workflow@")) {
      if (this.approvalRequested) declined();
      this.approvalRequested = true;
      needsApproval(true);
    }
    if (lower.startsWith("declined@")) declined();
    if (lower.startsWith("cancel@")) fail("signInCancelled", "Sign-in was canceled.");
    if (lower.startsWith("timeout@")) {
      fail("signInCancelled", "Sign-in was not completed in time. Try again.", {
        reason: "timeout",
      });
    }
    for (const [prefix, reason, aadsts] of BLOCKED_HINTS) {
      if (!lower.startsWith(prefix)) continue;
      fail("signInBlocked", BLOCKED_MESSAGES[reason], {
        reason,
        aadsts,
        tenant,
        adminPageUrl: link.adminPageUrl,
        ...(reason === "appNotSetUp" ? { adminConsentUrl: link.adminConsentUrl } : {}),
      });
    }
  }

  async cancelMicrosoftSignIn() {
    this.pendingSignIn?.();
    this.pendingSignIn = null;
  }

  async microsoftAdminConsent(accountId?: string, loginHint?: string) {
    await this.delay();
    let tenant = "organizations";
    if (accountId) {
      const account = this.account(accountId);
      if (account.settings.kind !== "microsoft")
        fail("invalidInput", "Only Microsoft accounts need this.");
      if (isPersonalAccount(account)) fail("invalidInput", NO_ADMIN_NEEDED);
      tenant = account.settings.tenantId;
    } else {
      const domain = emailDomain(loginHint);
      if (domain && isConsumerDomain(domain)) fail("invalidInput", NO_ADMIN_NEEDED);
      if (domain) tenant = domain;
    }
    return adminConsentLink(tenant, this.customClient);
  }

  async retryMicrosoftFeatures(accountId: string) {
    await this.delay();
    if (this.account(accountId).kind !== "microsoft") {
      fail("invalidInput", "Only Microsoft accounts need this.");
    }
    // The demo's mailbox still lacks what Microsoft 365 refused, so checking
    // again finds the same.
    this.emit(["accounts"], accountId);
  }

  /**
   * An administrator approved the feature, and a token refresh picked it up
   * (tests, demos): it's on again, with its default container.
   */
  grantFeature(accountId: string, feature: AccountFeature): void {
    const account = this.account(accountId);
    account.needsPermission = account.needsPermission.filter((f) => f !== feature);
    account.refusedFeatures = account.refusedFeatures.filter((f) => f !== feature);
    const kind = this.addDefaultContainer(account, feature);
    this.emit(kind ? ["accounts", kind] : ["accounts"], accountId);
  }

  /**
   * The feature's permission is gone, or Microsoft 365 refused it (a 403 on
   * its listing), as a sync would find (tests, demos): its data goes.
   */
  revokeFeature(
    accountId: string,
    feature: AccountFeature,
    reason: PermissionRequiredReason = "notGranted",
  ): void {
    const account = this.account(accountId);
    const others = (list: AccountFeature[]) => list.filter((f) => f !== feature);
    if (reason === "notGranted") {
      account.needsPermission = inOrder([...account.needsPermission, feature]);
      account.refusedFeatures = others(account.refusedFeatures);
    } else {
      account.refusedFeatures = inOrder([...account.refusedFeatures, feature]);
      account.needsPermission = others(account.needsPermission);
    }
    const kind = this.dropFeatureData(accountId, feature);
    this.emit(kind ? ["accounts", kind] : ["accounts"], accountId);
  }

  /** Like the core: a feature's cached data goes when it can't be used. */
  private dropFeatureData(accountId: string, feature: AccountFeature): DataKind | null {
    const s = this.state;
    const other = (item: { accountId: string }) => item.accountId !== accountId;
    switch (feature) {
      case "calendar":
        s.calendars = s.calendars.filter(other);
        s.events = s.events.filter(other);
        return "calendar";
      case "contacts":
        s.contacts = s.contacts.filter(other);
        return "contacts";
      case "tasks":
        s.taskLists = s.taskLists.filter(other);
        s.tasks = s.tasks.filter(other);
        return "tasks";
      case "mailboxSettings":
        return null;
      case "sensitivityLabels": {
        const protection = this.protection.get(accountId);
        if (!protection?.catalog) return null;
        protection.catalog = null;
        protection.listedAt = null;
        protection.error = null;
        return "protection";
      }
    }
  }

  /** As a sync after the feature comes back would find: its default calendar or task list. */
  private addDefaultContainer(account: Account, feature: AccountFeature): DataKind | null {
    switch (feature) {
      case "calendar":
        if (!this.state.calendars.some((c) => c.accountId === account.id)) {
          const id = this.id("cal");
          this.state.calendars.push({
            id,
            accountId: account.id,
            remoteId: id,
            name: "Calendar",
            color: account.color,
            canEdit: true,
            isDefault: true,
          });
        }
        return "calendar";
      case "contacts":
        return "contacts";
      case "tasks":
        if (!this.state.taskLists.some((l) => l.accountId === account.id)) {
          const id = this.id("tl");
          this.state.taskLists.push({
            id,
            accountId: account.id,
            remoteId: id,
            name: "Tasks",
            isDefault: true,
          });
        }
        return "tasks";
      case "mailboxSettings":
        return null;
      case "sensitivityLabels": {
        // The sync lists them at once.
        const protection = this.protectionOf(account);
        if (!protection) return null;
        protection.catalog = demoLabelCatalog();
        protection.listedAt = new Date().toISOString();
        protection.error = null;
        return "protection";
      }
    }
  }

  /** Like the core's stand-in provider for a Microsoft account's feature it can't use. */
  private checkFeature(accountId: string, feature: AccountFeature): void {
    const account = this.account(accountId);
    const reason: PermissionRequiredReason | null = account.needsPermission.includes(feature)
      ? "notGranted"
      : account.refusedFeatures.includes(feature)
        ? "refused"
        : null;
    if (!reason) return;
    const noun = FEATURE_COPY[feature].noun;
    fail(
      "permissionRequired",
      reason === "notGranted"
        ? `Cumulas Mail needs permission to use the ${noun} of ${account.email}. Turn it on in Settings → Accounts.`
        : `Microsoft 365 refused access to the ${noun} of ${account.email}. See Settings → Accounts.`,
      { feature, accountId, reason },
    );
  }

  async updateImapAccount(accountId: string, update: Parameters<Backend["updateImapAccount"]>[1]) {
    await this.delay();
    const account = this.account(accountId);
    if (account.settings.kind !== "imap") {
      fail("unsupported", "Only IMAP accounts have server settings.");
    }
    if (update.password === "wrong") {
      fail("authRequired", "The server rejected the user name or password.");
    }
    if (update.settings) {
      const { imapHost, smtpHost, imapTls, smtpTls } = update.settings;
      if (!imapHost.trim() || !smtpHost.trim()) {
        fail("invalidInput", "Enter the IMAP and SMTP server names.");
      }
      const local = (host: string) => ["localhost", "127.0.0.1", "::1"].includes(host.trim());
      if ((imapTls === "none" && !local(imapHost)) || (smtpTls === "none" && !local(smtpHost))) {
        fail("invalidInput", "Unencrypted connections are only allowed to this computer.");
      }
      account.settings = { kind: "imap", ...update.settings };
    }
    account.syncError = null;
    this.emit(["accounts"], accountId);
    return clone(account);
  }

  async updateAccount(accountId: string, displayName?: string, color?: string) {
    await this.delay();
    const account = this.account(accountId);
    if (displayName?.trim()) account.displayName = displayName.trim();
    if (color) account.color = color;
    this.emit(["accounts"], accountId);
    return clone(account);
  }

  async removeAccount(accountId: string) {
    await this.delay();
    this.account(accountId);
    const s = this.state;
    s.accounts = s.accounts.filter((a) => a.id !== accountId);
    s.folders = s.folders.filter((f) => f.accountId !== accountId);
    s.messages = s.messages.filter((m) => m.accountId !== accountId);
    s.serverMessages = s.serverMessages?.filter((m) => m.accountId !== accountId);
    s.calendars = s.calendars.filter((c) => c.accountId !== accountId);
    s.events = s.events.filter((e) => e.accountId !== accountId);
    s.contacts = s.contacts.filter((c) => c.accountId !== accountId);
    s.taskLists = s.taskLists.filter((l) => l.accountId !== accountId);
    s.tasks = s.tasks.filter((t) => t.accountId !== accountId);
    this.signatures.delete(accountId);
    this.organizationAnswers.delete(accountId);
    this.signatureChecks.delete(accountId);
    this.signatureRetries.delete(accountId);
    this.protection.delete(accountId);
    this.emit(["accounts", "mail", "calendar", "contacts", "tasks", "signatures", "protection"]);
  }

  async getSettings() {
    await this.delay();
    return clone(this.settings);
  }

  async updateSettings(settings: Settings) {
    await this.delay();
    if (settings.syncIntervalMinutes < 1)
      fail("invalidInput", "Sync interval must be at least one minute.");
    if (settings.defaultSnoozeMinutes < 1)
      fail("invalidInput", "Reminders must snooze for at least one minute.");
    const families = [settings.interfaceFont, settings.readingFont].map((f) =>
      f.kind === "family" ? f.family : undefined,
    );
    if (!families.every(validFamily))
      fail("invalidInput", "That font name can't be used. Choose a font from the list.");
    const pushChanged =
      settings.instantPush !== this.settings.instantPush ||
      settings.usePushRelay !== this.settings.usePushRelay;
    // Like the desktop app, these change only through their own calls.
    const { runInBackground, startAtLogin, startHidden } = this.settings;
    const remindersChanged = settings.remindersEnabled !== this.settings.remindersEnabled;
    this.settings = { ...clone(settings), runInBackground, startAtLogin, startHidden };
    if (remindersChanged) this.checkReminders();
    if (pushChanged) {
      this.pushSince.clear();
      const statuses = await this.getPushStatuses();
      queueMicrotask(() => statuses.forEach((s) => this.pushHandlers.forEach((h) => h(s))));
    }
    return clone(this.settings);
  }

  async listFonts() {
    await this.delay();
    return [...DEMO_FONTS];
  }

  async setZoom(factor: number) {
    // A page can't zoom itself the way the desktop webview does; CSS zoom on
    // the root, with the window's measures scaled to match, comes close
    // enough for the demo.
    document.documentElement.style.setProperty("zoom", factor === 1 ? null : String(factor));
    zoomViewport(factor);
  }

  async syncAccount(accountId: string) {
    this.simulateSync(accountId);
  }

  /** Reports a sync, which ends a moment later with what `synced` brings in. */
  private simulateSync(accountId: string, synced: () => DataKind[] = () => []): void {
    const account = this.account(accountId);
    const status = (state: SyncStatus["state"], message: string | null): SyncStatus => ({
      accountId,
      state,
      message,
      lastSyncedAt: account.lastSyncedAt,
    });
    this.syncing.add(accountId);
    this.syncHandlers.forEach((h) => h(status("syncing", "Syncing Inbox…")));
    setTimeout(() => {
      this.syncing.delete(accountId);
      if (!this.state.accounts.includes(account)) return;
      account.lastSyncedAt = new Date().toISOString();
      const kinds = synced();
      this.syncHandlers.forEach((h) => h(status("idle", null)));
      this.emit(["accounts", ...kinds], accountId);
    }, 600);
  }

  async syncAll() {
    for (const a of this.state.accounts) await this.syncAccount(a.id);
  }

  /**
   * Like the desktop app: IMAP accounts idle, Microsoft accounts use the push
   * service, or check the Inbox every 30 seconds when it's turned off.
   */
  async getPushStatuses(): Promise<PushStatus[]> {
    const now = new Date().toISOString();
    return this.state.accounts.map((a) => {
      if (!this.pushSince.has(a.id)) this.pushSince.set(a.id, now);
      return {
        accountId: a.id,
        mode: !this.settings.instantPush
          ? { kind: "off" }
          : a.kind === "imap"
            ? { kind: "imapIdle" }
            : this.settings.usePushRelay
              ? { kind: "graphRelay" }
              : { kind: "polling", everySecs: 30 },
        health: { kind: "live" },
        detail: null,
        consentRefused: false,
        adminConsent: null,
        since: this.pushSince.get(a.id)!,
        ...this.pushOverrides.get(a.id),
      };
    });
  }

  /** Sets an account's push status, as the desktop app would report it (tests, demos). */
  setPushStatus(accountId: string, status: Partial<PushStatus>) {
    this.pushOverrides.set(accountId, status);
    void this.getPushStatuses().then((all) => {
      const changed = all.find((s) => s.accountId === accountId);
      if (changed) this.pushHandlers.forEach((h) => h(changed));
    });
  }

  // --- desktop ------------------------------------------------------------------

  private backgroundState(): BackgroundState {
    return {
      runInBackground: this.settings.runInBackground,
      startAtLogin: this.settings.startAtLogin,
      startHidden: this.settings.startHidden,
      ...this.desktop,
    };
  }

  private announceBackground(): BackgroundState {
    const state = this.backgroundState();
    queueMicrotask(() => this.backgroundHandlers.forEach((h) => h(clone(state))));
    return state;
  }

  async getBackgroundState() {
    await this.delay();
    return this.backgroundState();
  }

  async setRunInBackground(enabled: boolean) {
    await this.delay();
    this.settings = { ...this.settings, runInBackground: enabled };
    return this.announceBackground();
  }

  async setStartAtLogin(enabled: boolean, hidden: boolean) {
    await this.delay();
    if (this.desktop.autostart === "unsupported") {
      fail("unsupported", "Starting at login isn't available on this system.");
    }
    if (enabled && this.desktop.backgroundAllowed === false) {
      this.announceBackground();
      fail(
        "unsupported",
        "Your system didn't allow Cumulas Mail to start at login. You can allow it in your system's settings for apps.",
      );
    }
    this.settings = { ...this.settings, startAtLogin: enabled, startHidden: hidden };
    return this.announceBackground();
  }

  async sendTestNotification() {
    await this.delay();
    if (!this.desktop.notifications) {
      fail("unsupported", "Couldn't show a notification. No notification service is running.");
    }
    this.calls.testNotifications += 1;
  }

  async takePendingNavigation() {
    const pending = this.pendingNavigation;
    this.pendingNavigation = null;
    return pending;
  }

  /**
   * What clicking a notification does in the desktop app: sends the page
   * somewhere, or keeps it for takePendingNavigation when nothing listens.
   */
  navigate(to: Navigate) {
    if (this.navigateHandlers.size === 0) this.pendingNavigation = to;
    else this.navigateHandlers.forEach((h) => h(to));
  }

  /** What the tray's Quit does while the page is there: asks the page. */
  requestQuit() {
    this.quitHandlers.forEach((h) => h());
  }

  async getUpdateState() {
    await this.delay();
    return clone(this.update);
  }

  async restartApp() {
    await this.delay();
    const update = this.update;
    if (update.kind !== "installed") fail("unsupported", "There's no new version to restart into.");
    // Like the desktop app: one restart at a time.
    if (update.restart === "restarting") return;
    this.calls.restart += 1;
    this.setUpdateState({ ...update, restart: "restarting" });
    await this.delay();
    if (this.failRestart) {
      this.setUpdateState({ ...update, restart: "failed" });
      fail("unsupported", RESTART_FAILED);
    }
    // The desktop app exits here, once the new version has taken over.
    this.onRestart?.();
  }

  /** Sets the update state, as the Flatpak portal would report it (tests, demos). */
  setUpdateState(state: UpdateState) {
    this.update = clone(state);
    queueMicrotask(() => this.updateHandlers.forEach((h) => h(clone(state))));
  }

  /**
   * What the tray's Restart to update, or a notification's Restart, does while
   * the page is there: asks the page.
   */
  requestRestart() {
    this.restartHandlers.forEach((h) => h());
  }

  // --- reminders ----------------------------------------------------------------

  /**
   * The reminders due (or snoozed) now, worked out from the events and tasks
   * much as the desktop app does: an event's from when its reminder goes off
   * until it ends, a task's from its reminder time; a day back at most.
   */
  private reminders(now = Date.now()): ActiveReminder[] {
    const lookback = now - 24 * 3_600_000;
    const out: ActiveReminder[] = [];
    const add = (r: Omit<ActiveReminder, "key" | "snoozedUntil" | "overdue" | "missed">) => {
      const fire = Date.parse(r.fireAt);
      if (fire > now || fire < lookback) return;
      const key = `${r.kind}:${r.itemId}:${Math.floor(fire / 1000)}`;
      const state = this.reminderStates.get(key);
      if (state?.dismissed) return;
      const snoozed = state?.snoozedUntil !== undefined && state.snoozedUntil > now;
      out.push({
        ...r,
        key,
        snoozedUntil: snoozed ? new Date(state.snoozedUntil!).toISOString() : null,
        overdue: Date.parse(r.startsAt ?? r.fireAt) <= now,
        missed: false,
      });
    };
    for (const e of this.state.events) {
      if (e.reminderMinutes === null || e.isCancelled || e.responseStatus === "declined") continue;
      // All-day events start at local midnight of their (UTC) date.
      const start = e.isAllDay
        ? new Date(
            new Date(e.start).getUTCFullYear(),
            new Date(e.start).getUTCMonth(),
            new Date(e.start).getUTCDate(),
          ).getTime()
        : Date.parse(e.start);
      const end = e.isAllDay
        ? start + (Date.parse(e.end) - Date.parse(e.start))
        : Date.parse(e.end);
      if (end <= now) continue;
      add({
        kind: "event",
        itemId: e.id,
        accountId: e.accountId,
        title: e.subject,
        startsAt: new Date(start).toISOString(),
        endsAt: new Date(end).toISOString(),
        location: e.location,
        onlineMeetingUrl: e.onlineMeetingUrl,
        fireAt: new Date(start - e.reminderMinutes * 60_000).toISOString(),
      });
    }
    for (const t of this.state.tasks) {
      if (t.reminderAt === null || t.status === "completed") continue;
      add({
        kind: "task",
        itemId: t.id,
        accountId: t.accountId,
        title: t.title,
        startsAt: null,
        endsAt: null,
        location: null,
        onlineMeetingUrl: null,
        fireAt: t.reminderAt,
      });
    }
    const order = (r: ActiveReminder) =>
      [
        r.snoozedUntil === null ? 0 : 1,
        Date.parse(r.snoozedUntil ?? r.startsAt ?? r.fireAt),
      ] as const;
    return out.sort((a, b) => {
      const [x, y] = [order(a), order(b)];
      return x[0] - y[0] || x[1] - y[1] || a.key.localeCompare(b.key);
    });
  }

  /**
   * Looks at the reminders again and tells the page what changed, as the
   * desktop app's scheduler does: on changes to events and tasks, and when
   * a test or the demo calls it.
   */
  checkReminders(): ActiveReminder[] {
    const active = this.settings.remindersEnabled ? this.reminders() : [];
    const due = new Set(active.filter((r) => r.snoozedUntil === null).map((r) => r.key));
    const newlyDue = active.filter((r) => due.has(r.key) && !this.dueReminders.has(r.key));
    const cleared = [...this.dueReminders].filter((k) => !due.has(k)).sort();
    this.dueReminders = due;
    const json = JSON.stringify(active);
    if (json !== this.lastReminders || newlyDue.length > 0 || cleared.length > 0) {
      this.lastReminders = json;
      const event = clone({ active, newlyDue, cleared });
      queueMicrotask(() => this.reminderHandlers.forEach((h) => h(event)));
    }
    return active;
  }

  private reminder(key: string): ActiveReminder {
    return (
      this.reminders().find((r) => r.key === key) ??
      fail("notFound", "That reminder is no longer due.")
    );
  }

  async listActiveReminders() {
    await this.delay();
    return clone(this.settings.remindersEnabled ? this.reminders() : []);
  }

  async snoozeReminder(key: string, snooze: SnoozeFor) {
    await this.delay();
    const now = Date.now();
    const until = "minutes" in snooze ? now + snooze.minutes * 60_000 : Date.parse(snooze.until);
    if ("minutes" in snooze && snooze.minutes < 1)
      fail("invalidInput", "Snooze for at least one minute.");
    if (!(until > now)) fail("invalidInput", "Choose a time that hasn't passed.");
    if (until > now + 30 * 86_400_000) fail("invalidInput", "Snooze for 30 days at most.");
    this.reminder(key);
    this.reminderStates.set(key, { snoozedUntil: until });
    return clone(this.checkReminders());
  }

  async dismissReminders(keys: string[]) {
    await this.delay();
    const active = new Set(this.reminders().map((r) => r.key));
    for (const key of keys.filter((k) => active.has(k))) {
      this.reminderStates.set(key, { dismissed: true });
    }
    return clone(this.checkReminders());
  }

  async getSyncStatuses(): Promise<SyncStatus[]> {
    return this.state.accounts.map((a) => ({
      accountId: a.id,
      ...(this.syncing.has(a.id)
        ? { state: "syncing", message: "Syncing Inbox…" }
        : { state: a.syncError ? "error" : "idle", message: a.syncError }),
      lastSyncedAt: a.lastSyncedAt,
    }));
  }

  // --- mail ---------------------------------------------------------------------

  async listFolders(accountId?: string) {
    await this.delay();
    const rank = ["inbox", "drafts", "sent", "archive", "junk", "trash", "outbox", "other"];
    return clone(
      this.state.folders
        .filter((f) => !accountId || f.accountId === accountId)
        .sort(
          (a, b) =>
            rank.indexOf(a.role) - rank.indexOf(b.role) ||
            a.name.localeCompare(b.name, undefined, { sensitivity: "base" }),
        ),
    );
  }

  async listMessages(folderId: string, query: Parameters<Backend["listMessages"]>[1]) {
    await this.delay();
    const all = this.state.messages
      .filter((m) => m.folderId === folderId)
      .filter((m) => !query.unreadOnly || !m.isRead)
      .filter((m) => !query.flaggedOnly || m.isFlagged)
      .sort(newestFirst);
    const limit = query.limit || 100;
    return {
      items: all.slice(query.offset, query.offset + limit).map(summary),
      total: all.length,
    };
  }

  /**
   * Where a search looks, like the core: the accounts `account:` names (of
   * `accountId`'s, when given) and the folders `in:` names in them. Null is
   * everywhere; an empty set, nowhere.
   */
  private searchScope(
    query: ReturnType<typeof parseSearch>,
    accountId?: string,
  ): { accounts: Set<string> | null; folders: Set<string> | null } {
    const accounts =
      query.accounts.length === 0
        ? accountId
          ? new Set([accountId])
          : null
        : new Set(
            this.state.accounts
              .filter((a) => !accountId || a.id === accountId)
              .filter((a) => query.accounts.some((v) => accountMatches(v, a)))
              .map((a) => a.id),
          );
    const folders =
      query.folders.length === 0
        ? null
        : new Set(
            this.state.folders
              .filter((f) => !accounts || accounts.has(f.accountId))
              .filter((f) => query.folders.some((v) => folderMatches(v, f)))
              .map((f) => f.id),
          );
    return { accounts, folders };
  }

  /**
   * Like the core: the messages the index finds (word prefixes, and inside
   * subjects and senders) that pass the filters, plus the ones server
   * searches found that pass them. Best first while the text has words to
   * rank by (the 200 best, and every server hit), else newest first. An
   * order it doesn't know is refused, as the core's command refuses it.
   */
  async searchMessages(
    text: string,
    accountId?: string,
    includeIds: string[] = [],
    options?: SearchOptions | null,
  ): Promise<SearchHit[]> {
    await this.delay();
    const sort: unknown = options?.sort;
    if (sort !== undefined && sort !== "relevance" && sort !== "newest") {
      fail("invalidInput", `Not an order of search results: ${String(sort)}.`);
    }
    const now = new Date();
    const query = parseSearch(text);
    const dates = resolveDates(query, now, options?.weekStartsOn ?? 0);
    const scope = this.searchScope(query, accountId);
    const inScope = (m: Message) =>
      (!scope.accounts || scope.accounts.has(m.accountId)) &&
      (!scope.folders || scope.folders.has(m.folderId)) &&
      !isImpossible(dates) &&
      admits(query, m, dates, null);
    // As the app's index: a message's text once it was opened, its preview
    // before.
    const indexed = (m: Message) => (this.opened.has(m.id) ? m : { ...m, bodyText: null });
    const matches = isEmptySearch(query)
      ? []
      : this.state.messages.filter(
          (m) => inScope(m) && matchesSearch(query, indexed(m), { infix: true }),
        );
    const matched = new Set(matches.map((m) => m.id));
    const included = new Set(includeIds.slice(0, MAX_INCLUDED_IDS));
    const fromServer = this.state.messages.filter((m) => included.has(m.id) && inScope(m));
    const ranked = (options?.sort ?? "relevance") === "relevance" && ranks(query);

    let found: Message[];
    if (ranked) {
      const hits = [
        ...matches.map((m) => ({ message: indexed(m), fromServer: included.has(m.id) })),
        ...fromServer
          .filter((m) => !matched.has(m.id))
          .map((m) => ({ message: indexed(m), fromServer: true })),
      ];
      let kept = 0;
      found = rankHits(hits, query, now)
        .filter((h) => h.fromServer || kept++ < LOCAL_SEARCH_LIMIT)
        .map((h) => this.message(h.message.id));
    } else {
      const local = matches.sort(searchOrder).slice(0, LOCAL_SEARCH_LIMIT);
      found = [...new Set([...local, ...fromServer])].sort(searchOrder);
    }

    // The body's words around what matched there, for the first rows, when
    // the text has words to rank by (either order).
    const patterns = ranks(query) ? hitPatterns(query) : null;
    return found.map((m, i) => ({
      ...summary(m),
      snippet:
        patterns && i < MAX_SNIPPETS && matchesSearch(query, indexed(m), { infix: false })
          ? mockSnippet(indexed(m), patterns)
          : null,
    }));
  }

  /**
   * Like the core: searches one account's mail on its "server" (what is on
   * the device and `serverMessages`), a page at a time, and brings what it
   * finds that passes the text's filters onto the device. Graph matches
   * words by their start and, with words, can't say read or flagged;
   * without words it filters by all of them; either way it looks in one
   * folder `in:` names, but not in several. IMAP matches anywhere in words
   * and says every filter. A call reads
   * pages until half a page's worth pass, the matches run out, or it read
   * `serverSearchMaxPages`.
   */
  async searchServer(
    searchId: string,
    text: string,
    accountId: string,
    more = false,
    options?: SearchOptions | null,
  ): Promise<ServerSearchResult> {
    if (!searchId || searchId.length > 100) fail("invalidInput", "Invalid search.");
    const account = this.account(accountId);
    const answer = (
      status: ServerSearchStatus,
      result: Partial<ServerSearchResult> = {},
    ): ServerSearchResult => ({
      searchId,
      accountId,
      status,
      messageIds: [],
      added: 0,
      hasMore: false,
      error: null,
      ...result,
    });
    const query = parseSearch(text);
    const dates = resolveDates(query, new Date(), options?.weekStartsOn ?? 0);
    // Like the core, before the search is started: it cancels nothing.
    const otherAccount =
      query.accounts.length > 0 && !query.accounts.some((v) => accountMatches(v, account));
    if (!isNarrowEnoughForServers(query) || otherAccount || isImpossible(dates)) {
      return answer("done");
    }
    if (this.retiredSearches.includes(searchId)) return answer("cancelled");
    if (this.serverSearch?.id !== searchId) {
      this.retireServerSearch();
      this.serverSearch = { id: searchId, rest: new Map() };
    }
    const search = this.serverSearch;

    if (this.serverSearchLatency > 0) {
      await new Promise((r) => setTimeout(r, this.serverSearchLatency));
    }
    while (this.serverSearchGate) await this.serverSearchGate;
    if (this.serverSearch !== search) return answer("cancelled");
    const failure = this.serverSearchFailures.get(accountId);
    if (failure) return answer("failed", { error: clone(failure) });

    const scope = this.state.folders.filter(
      (f) => f.accountId === accountId && query.folders.some((v) => folderMatches(v, f)),
    );
    if (query.folders.length > 0 && scope.length === 0) return answer("done");
    const scopeIds = query.folders.length > 0 ? new Set(scope.map((f) => f.id)) : null;

    const server = [...this.state.messages, ...(this.state.serverMessages ?? [])];
    let matches: string[];
    if (more) {
      const rest = search.rest.get(accountId);
      if (!rest) return answer("done");
      matches = rest;
    } else {
      const imap = account.kind === "imap";
      const words = query.terms.length > 0;
      // What the server can say: IMAP everything; Graph's $search the words,
      // attachments and dates, its $filter every filter, and either looks in
      // one folder when `in:` names exactly one there, else everywhere.
      const graphFolder = scope.length === 1 ? scopeIds : null;
      const asked = imap
        ? (m: Message) => admits(query, m, dates, scopeIds)
        : words
          ? (m: Message) => admits({ ...query, read: null, flagged: false }, m, dates, graphFolder)
          : (m: Message) => admits(query, m, dates, graphFolder);
      matches = server
        .filter(
          (m) =>
            m.accountId === accountId &&
            asked(m) &&
            matchesSearch(query, m, { infix: false, substrings: imap }),
        )
        .sort(searchOrder)
        .map((m) => m.id);
    }
    // A page at a time, as the server sends them; then the filters.
    const wanted = Math.ceil(this.serverSearchPage / 2);
    const passed: Message[] = [];
    let pages = 0;
    while (matches.length > 0 && passed.length < wanted && pages < this.serverSearchMaxPages) {
      const page = matches.slice(0, this.serverSearchPage);
      matches = matches.slice(this.serverSearchPage);
      pages += 1;
      for (const id of page) {
        const m = server.find((x) => x.id === id);
        if (m && admits(query, m, dates, scopeIds)) passed.push(m);
      }
    }
    if (matches.length > 0) search.rest.set(accountId, matches);
    else search.rest.delete(accountId);

    const messageIds: string[] = [];
    let added = 0;
    const serverOnly = this.state.serverMessages ?? [];
    for (const m of passed) {
      const index = serverOnly.indexOf(m);
      if (index >= 0) {
        // Stored now; the folder's count already had it.
        this.state.messages.push(...serverOnly.splice(index, 1));
        added += 1;
        messageIds.push(m.id);
      } else if (this.state.messages.includes(m)) {
        messageIds.push(m.id);
      }
    }
    if (added > 0) this.emit(["mail"], accountId);
    return answer("done", { messageIds, added, hasMore: matches.length > 0 });
  }

  async cancelServerSearch(searchId: string) {
    if (this.serverSearch?.id === searchId) this.retireServerSearch();
    // Canceled before it started: it never will.
    else this.rememberRetiredSearch(searchId);
  }

  private retireServerSearch() {
    if (!this.serverSearch) return;
    this.rememberRetiredSearch(this.serverSearch.id);
    this.serverSearch = null;
  }

  private rememberRetiredSearch(searchId: string) {
    if (this.retiredSearches.includes(searchId)) return;
    if (this.retiredSearches.length === RETIRED_SEARCHES) this.retiredSearches.shift();
    this.retiredSearches.push(searchId);
  }

  /** Holds server searches until the returned function is called (tests). */
  pauseServerSearch(): () => void {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => (release = resolve));
    this.serverSearchGate = gate;
    return () => {
      if (this.serverSearchGate === gate) this.serverSearchGate = null;
      release();
    };
  }

  /**
   * Deletes a message on the "server" behind the app's back (tests): as in
   * the core, opening it then removes it here and fails with notFound.
   */
  removeFromServer(messageId: string): void {
    this.goneFromServer.add(messageId);
  }

  /** Makes the account's server searches fail with `error`, or succeed again with null. */
  failServerSearch(accountId: string, error: BackendError | null): void {
    if (error) this.serverSearchFailures.set(accountId, error);
    else this.serverSearchFailures.delete(accountId);
  }

  async getMessage(messageId: string, allowRemote = false) {
    await this.delay();
    if (this.goneFromServer.delete(messageId)) {
      const gone = this.message(messageId);
      this.state.messages = this.state.messages.filter((m) => m.id !== messageId);
      this.emit(["mail"], gone.accountId);
      fail("notFound", "This message is no longer on the server.");
    }
    const m = clone(this.message(messageId));
    this.opened.add(messageId);
    // Like the core: without the account's labels, only that the message
    // names one of its organization's, never by the name it gives.
    const protection = this.protectionOf(this.account(m.accountId));
    if (protection && !protection.catalog) {
      const unlisted = (l: MessageLabel): MessageLabel => ({
        id: l.id,
        name: null,
        color: null,
        fromCatalog: false,
      });
      if (m.label) m.label = unlisted(m.label);
      if (m.requested?.label) m.requested.label = unlisted(m.requested.label);
    }
    const allow = allowRemote || this.settings.remoteContent === "allow";
    if (m.bodyHtml && !allow && REMOTE_IMG.test(m.bodyHtml)) {
      m.bodyHtml = m.bodyHtml.replace(REMOTE_IMG, "");
      m.hasBlockedContent = true;
    }
    REMOTE_IMG.lastIndex = 0;
    return m;
  }

  async setRead(messageIds: string[], read: boolean) {
    await this.delay();
    for (const id of messageIds) this.message(id).isRead = read;
    recount(this.state);
    this.emit(["mail"]);
  }

  async setFlagged(messageIds: string[], flagged: boolean) {
    await this.delay();
    for (const id of messageIds) this.message(id).isFlagged = flagged;
    this.emit(["mail"]);
  }

  async moveMessages(messageIds: string[], folderId: string) {
    await this.delay();
    const destination =
      this.state.folders.find((f) => f.id === folderId) ?? fail("notFound", "folder not found");
    for (const id of messageIds) {
      const m = this.message(id);
      if (m.accountId !== destination.accountId) {
        fail("unsupported", "Messages can only be moved between folders of the same account.");
      }
      m.folderId = folderId;
    }
    recount(this.state);
    this.emit(["mail"], destination.accountId);
  }

  async archiveMessages(messageIds: string[]) {
    for (const id of messageIds) {
      const m = this.message(id);
      const archive =
        this.state.folders.find((f) => f.accountId === m.accountId && f.role === "archive") ??
        fail("unsupported", "This account has no Archive folder.");
      await this.moveMessages([id], archive.id);
    }
  }

  async deleteMessages(messageIds: string[]) {
    await this.delay();
    for (const id of messageIds) {
      const m = this.message(id);
      const current = this.state.folders.find((f) => f.id === m.folderId);
      const trash = this.state.folders.find(
        (f) => f.accountId === m.accountId && f.role === "trash",
      );
      if (trash && current && current.role !== "trash" && current.role !== "junk") {
        m.folderId = trash.id;
      } else {
        this.state.messages = this.state.messages.filter((x) => x.id !== id);
      }
    }
    recount(this.state);
    this.emit(["mail"]);
  }

  async sendMessage(message: Parameters<Backend["sendMessage"]>[0]) {
    await this.delay();
    if (message.to.length + message.cc.length + message.bcc.length === 0) {
      fail("invalidInput", "Add at least one recipient.");
    }
    const account = this.account(message.accountId);
    const requested = this.checkProtectedSend(account, message);
    this.sent.push(clone(message));
    const sentFolder = this.state.folders.find(
      (f) => f.accountId === account.id && f.role === "sent",
    );
    if (sentFolder) {
      const id = this.id("msg");
      const text = message.bodyHtml
        .replace(/<[^>]+>/g, " ")
        .replace(/\s+/g, " ")
        .trim();
      this.state.messages.push({
        id,
        accountId: account.id,
        folderId: sentFolder.id,
        remoteId: id,
        conversationId: null,
        subject: message.subject,
        from: { name: account.displayName, address: account.email },
        to: message.to,
        cc: message.cc,
        preview: text.slice(0, 180),
        receivedAt: new Date().toISOString(),
        isRead: true,
        isFlagged: false,
        isDraft: false,
        hasAttachments: message.attachments.length > 0,
        importance: message.importance ?? "normal",
        bcc: message.bcc,
        replyTo: [],
        bodyHtml: message.bodyHtml,
        bodyText: text,
        hasBlockedContent: false,
        attachments: message.attachments.map((a, i) => ({
          id: `${id}-att-${i}`,
          name: a.name,
          contentType: a.contentType,
          size: Math.floor((a.dataBase64.length * 3) / 4),
          isInline: false,
          contentId: null,
        })),
        internetMessageId: `<${id}@demo.example>`,
        inReplyTo: null,
        references: [],
        protection: null,
        label: null,
        requested,
      });
      recount(this.state);
    }
    this.emit(["mail"], account.id);
  }

  /**
   * Like the core's send_message: refuses replies to encrypted mail, a
   * missing label the organization requires, and encryption or labels that
   * aren't set up (or were checked too long ago). Returns what the Sent
   * copy says was asked for.
   */
  private checkProtectedSend(
    account: Account,
    message: Parameters<Backend["sendMessage"]>[0],
  ): Message["requested"] {
    const original = message.replyToMessageId
      ? this.state.messages.find((m) => m.id === message.replyToMessageId)
      : undefined;
    if (message.replyKind && original && isEncrypted(original.protection)) {
      fail("unsupported", account.kind === "imap" ? ENCRYPTED_ORIGINAL : PROTECTED_ORIGINAL, {
        reason: "protectedOriginal",
        messageId: original.id,
        protection: original.protection!.kind,
      });
    }
    const setup = this.protectionSetup(account);
    const protect = message.protect ?? null;
    const labelId = message.labelId?.trim().toLowerCase() || null;
    // Offered by the last decided check, while a new one waits too.
    const applied = setup.labels.some((l) => l.check.passedAt !== null);
    if (!labelId && setup.policy.required && applied) {
      fail("invalidInput", "Choose a sensitivity label. Your organization requires one.", {
        reason: "labelRequired",
      });
    }
    if (!protect && !labelId) return null;
    if (!setup.supported) fail("unsupported", WORK_ACCOUNTS_ONLY);
    const label = labelId ? setup.labels.find((l) => l.id === labelId) : undefined;
    if (labelId && !label) {
      fail("invalidInput", `That sensitivity label isn't available for ${account.email}.`);
    }
    const tooOld = (what: string) =>
      fail(
        "invalidInput",
        `${what} was last checked more than ${CHECK_VALID_DAYS} days ago. Check it again in Settings → Accounts.`,
      );
    const notSetUp = (what: string) =>
      fail(
        "invalidInput",
        `${what} isn't set up for ${account.email}. Check it in Settings → Accounts.`,
      );
    const protection = this.protectionOf(account);
    /** Refuses `item` unless its last decided check passed less than CHECK_VALID_DAYS ago. */
    const offered = (item: CheckItem, what: string) => {
      if (!protection) return;
      const state = decidedState(this.checkRecord(protection, item));
      if (state === "applied") return;
      if (this.isWaiting(protection, item)) {
        fail(
          "invalidInput",
          `${what} is being checked for ${account.email}. Wait for the check in Settings → Accounts to finish.`,
        );
      }
      if (state === "expired") tooOld(what);
      notSetUp(what);
    };
    // In the core's order: the label, then the encryption.
    let requestedLabel: MessageLabel | null = null;
    if (label) {
      const title = labelTitle(label);
      offered({ kind: "label", labelId: label.id }, `The label ${title}`);
      requestedLabel = { id: label.id, name: title, color: label.color, fromCatalog: true };
    }
    if (protect) {
      if (label?.hasProtection) {
        fail(
          "invalidInput",
          `The label ${labelTitle(label)} already encrypts this message. Choose either the label or ${ACTION_NAMES[protect]}.`,
        );
      }
      offered({ kind: "action", action: protect }, ACTION_NAMES[protect]);
    }
    return { action: protect, label: requestedLabel };
  }

  async suggestAddresses(text: string) {
    await this.delay();
    const q = text.trim().toLowerCase();
    if (!q) return this.recentCorrespondents();
    const seen = new Set<string>();
    const out: { name: string | null; address: string }[] = [];
    const add = (name: string | null, address: string) => {
      const key = address.toLowerCase();
      if (seen.has(key)) return;
      if (!(name ?? "").toLowerCase().includes(q) && !key.includes(q)) return;
      seen.add(key);
      out.push({ name, address });
    };
    for (const c of this.state.contacts) for (const e of c.emails) add(c.displayName, e.address);
    for (const m of this.state.messages) if (m.from) add(m.from.name, m.from.address);
    return out.slice(0, 8);
  }

  /**
   * Like the core for an empty text: the 8 people who wrote most in the
   * last 90 days (the most recent first among equals), not the accounts.
   */
  private recentCorrespondents(): EmailAddress[] {
    const since = Date.now() - 90 * 86_400_000;
    const own = new Set(this.state.accounts.map((a) => a.email.toLowerCase()));
    const senders = new Map<string, { from: EmailAddress; count: number; last: string }>();
    for (const m of this.state.messages) {
      if (!m.from || Date.parse(m.receivedAt) < since) continue;
      const key = m.from.address.toLowerCase();
      if (own.has(key)) continue;
      const seen = senders.get(key);
      if (!seen) senders.set(key, { from: m.from, count: 1, last: m.receivedAt });
      else {
        seen.count += 1;
        if (m.receivedAt > seen.last) {
          seen.last = m.receivedAt;
          seen.from = m.from;
        }
      }
    }
    return [...senders.values()]
      .sort((a, b) => b.count - a.count || b.last.localeCompare(a.last))
      .slice(0, 8)
      .map(({ from }) => ({ name: from.name, address: from.address }));
  }

  async saveAttachment(messageId: string, attachmentId: string) {
    await this.delay();
    const att =
      this.message(messageId).attachments.find((a) => a.id === attachmentId) ??
      fail("notFound", "attachment not found");
    return `/home/alex/Downloads/${att.name}`;
  }

  async openAttachment(messageId: string, attachmentId: string) {
    await this.delay();
    const message = this.message(messageId);
    if (!message.attachments.some((a) => a.id === attachmentId)) {
      fail("notFound", "attachment not found");
    }
    // Like the core: a protection's own files never open.
    if (message.protection?.containerIds.includes(attachmentId)) {
      fail("unsupported", OPEN_CONTAINER);
    }
  }

  /** Like the core: opens the message's webLink; IMAP messages have none. */
  async openInOutlook(messageId: string) {
    await this.delay();
    const message = this.message(messageId);
    if (this.account(message.accountId).kind !== "microsoft") fail("unsupported", OUTLOOK_ONLY);
    this.calls.openedInOutlook.push(messageId);
  }

  /** A text in the shape of the core's, from what the demo message says of itself. */
  async getMessageDiagnostics(messageId: string) {
    await this.delay();
    const m = this.message(messageId);
    const account = this.account(m.accountId);
    const microsoft = account.kind === "microsoft";
    const folder = this.state.folders.find((f) => f.id === m.folderId);
    const p = m.protection;
    const kind = p?.kind ?? null;
    const detected = {
      rightsManaged: "Microsoft Purview encryption",
      smimeEncrypted: "S/MIME encryption",
      smimeSigned: "S/MIME signature",
      portalLink: "encrypted message portal link",
    };
    const messageClass = {
      rightsManaged: "IPM.Note.rpmsg.Microsoft.Voting.Restricted",
      smimeEncrypted: "IPM.Note.SMIME",
      smimeSigned: "IPM.Note.SMIME.MultipartSigned",
      portalLink: "IPM.Note",
    };
    const body = m.bodyHtml?.trim()
      ? `HTML, ${m.bodyHtml.length} characters`
      : m.bodyText?.trim()
        ? `text, ${m.bodyText.length} characters`
        : "empty";
    const requested = m.requested;
    const lines = [
      `Cumulas Mail ${(await this.getAppInfo()).version}: protection details of one message`,
      "These details don't include the message's subject, addresses, text, links, or the names of its files other than the protection's own.",
      "",
      `Account: ${microsoft ? (isPersonalAccount(account) ? "Microsoft 365 (personal)" : "Microsoft 365 (work or school)") : "IMAP"}`,
      `Folder: ${folder?.role ?? "other"}`,
      `Detected: ${kind ? detected[kind] : "nothing"}`,
      `Content hidden in a container: ${p?.contentHidden ? "yes" : "no"}`,
      `Portal link: ${p?.portalUrl ? `yes (${new URL(p.portalUrl).host})` : "no"}`,
      `Content-Class: ${kind === "rightsManaged" ? "rpmsg.message" : "none"}`,
      `Message class: ${microsoft ? (kind ? messageClass[kind] : "IPM.Note") : "none"}`,
      `S/MIME content: ${kind === "smimeEncrypted" ? "enveloped" : kind === "smimeSigned" ? "signed" : "not applicable"}`,
      `MIME type: ${microsoft ? "none" : kind === "smimeEncrypted" ? "application/pkcs7-mime; smime-type=enveloped-data" : "multipart/alternative"}`,
      `Server says it has attachments: ${microsoft ? (m.hasAttachments ? "yes" : "no") : "not said"}`,
      `Body: ${body}`,
      "Body read from the message's MIME: no",
      "Properties read: yes",
      `Attachments: ${m.attachments.length}`,
      ...m.attachments.map((a) => {
        const container = p?.containerIds.includes(a.id) ?? false;
        const name = container ? a.name : "(name left out)";
        return `  - ${name} · ${a.contentType} · ${diagnosticSize(a.size)}${container ? " · protection file" : ""}`;
      }),
      `Label header: ${m.label ? "from the header" : "none"}`,
      ...(m.label
        ? [
            `  - ${m.label.id} · this organization: yes · enabled: yes · method: Privileged · in your labels: ${m.label.fromCatalog ? "yes" : "no list"}`,
          ]
        : []),
      `Requested by Cumulas Mail: ${requested?.action === "doNotForward" ? "do-not-forward" : (requested?.action ?? "none")} · label: ${requested?.label?.id ?? "none"}`,
      // The demo's messages don't say how Exchange handled them.
      "Relay (X-Signature-Stamped): no",
      "Counted as internal (AuthAs): not said",
      "Direction: not said",
      `Request read from: ${requested?.action || requested?.label ? "the message property" : "none"}`,
    ];
    return lines.join("\n");
  }

  // --- protection -------------------------------------------------------------------

  /** A Microsoft 365 work or school account: the only kind that can send protected mail. */
  private isWorkAccount(account: Account): boolean {
    return account.kind === "microsoft" && !isPersonalAccount(account);
  }

  /**
   * The account's checks and labels, as the core keeps them: labels listed
   * when it has the permission, every option unchecked unless the demo
   * starts otherwise (`?protection=`). Null for IMAP and personal accounts.
   */
  private protectionOf(account: Account): MockProtection | null {
    if (!this.isWorkAccount(account)) return null;
    let protection = this.protection.get(account.id);
    if (protection) return protection;
    const permitted =
      !account.needsPermission.includes("sensitivityLabels") &&
      !account.refusedFeatures.includes("sensitivityLabels");
    const catalog = permitted ? demoLabelCatalog() : null;
    protection = {
      encrypt: { ...NOT_CHECKED },
      doNotForward: { ...NOT_CHECKED },
      labels: new Map(),
      catalog,
      listedAt: catalog ? new Date().toISOString() : null,
      error: null,
      run: null,
    };
    const demo = this.demoProtection?.accountId === account.id ? this.demoProtection.kind : null;
    if (demo === "setup" || demo === "expired" || demo === "relay") {
      const twoDaysAgo = new Date(Date.now() - 2 * DAY_MS).toISOString();
      protection.encrypt = demoOutcome({ kind: "action", action: "encrypt" }, twoDaysAgo);
      protection.doNotForward = demoOutcome({ kind: "action", action: "doNotForward" }, twoDaysAgo);
      for (const label of demoLabelCatalog().labels) {
        protection.labels.set(
          label.id,
          demoOutcome({ kind: "label", labelId: label.id }, twoDaysAgo, demo === "relay"),
        );
      }
      if (demo === "expired") {
        // Passed, but longer ago than a check lasts.
        protection.encrypt.at = new Date(
          Date.now() - (CHECK_VALID_DAYS + 10) * DAY_MS,
        ).toISOString();
      }
    } else if (demo === "checking") {
      protection.run = {
        items: [{ kind: "action", action: "encrypt" }],
        sentAt: new Date().toISOString(),
        looks: 0,
      };
    }
    this.protection.set(account.id, protection);
    return protection;
  }

  /** The demo's mail goes through the Cumulas Signatures relay (`?protection=relay`). */
  private throughRelay(account: Account): boolean {
    return this.demoProtection?.accountId === account.id && this.demoProtection.kind === "relay";
  }

  /**
   * How a check of the demo turns out now: as in demoOutcome, but a label's
   * check within two minutes of setup changing the organization's
   * auto-labeling policies from here is too soon to tell, as the core
   * judges it.
   */
  private checkOutcome(account: Account, item: CheckItem, at: string): CheckRecord {
    const outcome = demoOutcome(item, at, this.throughRelay(account));
    const changed = this.orgSetupChanged.get(this.tenantOf(account))?.policiesChangedAt;
    if (
      item.kind === "label" &&
      changed &&
      outcome.note !== "otherRule" &&
      Date.parse(at) - Date.parse(changed) < TOO_SOON_MS
    ) {
      return { ...outcome, state: "notApplied", note: "tooSoon", changedAt: changed };
    }
    return outcome;
  }

  private checkRecord(protection: MockProtection, item: CheckItem): CheckRecord {
    if (item.kind === "label") return protection.labels.get(item.labelId) ?? { ...NOT_CHECKED };
    return item.action === "encrypt" ? protection.encrypt : protection.doNotForward;
  }

  /** Whether the waiting check sent a test message for `item`. */
  private isWaiting(protection: MockProtection, item: CheckItem): boolean {
    return !!protection.run?.items.some((i) => checkKey(i) === checkKey(item));
  }

  private setCheckRecord(protection: MockProtection, item: CheckItem, result: CheckRecord): void {
    if (item.kind === "label") protection.labels.set(item.labelId, result);
    else if (item.action === "encrypt") protection.encrypt = result;
    else protection.doNotForward = result;
  }

  /**
   * Like the core's protection_setup: a passed check older than
   * CHECK_VALID_DAYS shows as expired; an item a check is waiting for shows
   * as waiting, while the pass before it still counts (passedAt).
   */
  private protectionSetup(account: Account): ProtectionSetup {
    const protection = this.protectionOf(account);
    if (!protection) {
      return {
        accountId: account.id,
        supported: false,
        encrypt: { ...NOT_CHECKED, passedAt: null },
        doNotForward: { ...NOT_CHECKED, passedAt: null },
        labels: [],
        policy: { ...NO_POLICY },
        labelsListedAt: null,
        labelsError: null,
        checking: false,
      };
    }
    const run = protection.run;
    const shown = (item: CheckItem): CheckResult => {
      const record = this.checkRecord(protection, item);
      const state = decidedState(record);
      const passedAt = state === "applied" ? record.at : null;
      return run && this.isWaiting(protection, item)
        ? { ...NOT_CHECKED, state: "waiting", at: run.sentAt, passedAt }
        : { ...record, state, passedAt };
    };
    const catalog = protection.catalog;
    return {
      accountId: account.id,
      supported: true,
      encrypt: shown({ kind: "action", action: "encrypt" }),
      doNotForward: shown({ kind: "action", action: "doNotForward" }),
      labels: (catalog?.labels ?? []).map((l) => ({
        ...l,
        check: shown({ kind: "label", labelId: l.id }),
      })),
      policy: { ...(catalog?.policy ?? NO_POLICY) },
      labelsListedAt: catalog ? protection.listedAt : null,
      labelsError: protection.error,
      checking: protection.run !== null,
    };
  }

  async getProtectionSetup(accountId: string) {
    await this.delay();
    return this.protectionSetup(this.account(accountId));
  }

  async refreshSensitivityLabels(accountId: string) {
    await this.delay();
    const account = this.account(accountId);
    const protection = this.protectionOf(account);
    if (!protection) {
      fail("unsupported", "Only Microsoft 365 work and school accounts have sensitivity labels.");
    }
    this.checkFeature(accountId, "sensitivityLabels");
    protection.catalog = demoLabelCatalog();
    protection.listedAt = new Date().toISOString();
    protection.error = null;
    this.emit(["protection"], accountId);
    return this.protectionSetup(account);
  }

  /** Changes the account's label policy, as Microsoft 365 would list it next (tests, demos). */
  setLabelPolicy(accountId: string, policy: Partial<LabelPolicy>): void {
    const protection = this.protectionOf(this.account(accountId));
    if (!protection?.catalog) return;
    protection.catalog.policy = { ...protection.catalog.policy, ...policy };
    this.emit(["protection"], accountId);
  }

  /** Makes the account's label listing fail, as Microsoft 365's beta API can (tests, demos). */
  failLabelListing(accountId: string, error: string): void {
    const protection = this.protectionOf(this.account(accountId));
    if (!protection) return;
    protection.error = error;
    this.emit(["protection"], accountId);
  }

  async checkProtection(accountId: string, items: CheckItem[]) {
    await this.delay();
    const account = this.account(accountId);
    const protection = this.protectionOf(account);
    if (!protection) fail("unsupported", WORK_ACCOUNTS_ONLY);
    // In the core's order: each label listed, then how many.
    for (const item of items) {
      if (item.kind === "label" && !protection.catalog?.labels.some((l) => l.id === item.labelId)) {
        fail("invalidInput", `That sensitivity label isn't available for ${account.email}.`);
      }
    }
    if (items.length === 0) fail("invalidInput", "Choose what to check.");
    if (items.length > MAX_CHECK_ITEMS) {
      fail("invalidInput", `Cumulas Mail checks at most ${MAX_CHECK_ITEMS} at a time.`);
    }
    if (protection.run) fail("invalidInput", "Wait for the check that's running to finish.");
    // What each item's last check decided stands until its test message is judged.
    protection.run = { items: clone(items), sentAt: new Date().toISOString(), looks: 0 };
    this.emit(["protection"], accountId);
    return this.protectionSetup(account);
  }

  /**
   * Like the core: looks for the check's test messages. In the demo they
   * arrive by the second look, as in `?protection=setup`, and are moved to
   * Deleted Items once read.
   */
  async refreshProtectionCheck(accountId: string) {
    await this.delay();
    const account = this.account(accountId);
    const protection = this.protectionOf(account);
    const run = protection?.run;
    if (!protection || !run) return this.protectionSetup(account);
    run.looks += 1;
    if (run.looks >= 2) {
      const now = new Date().toISOString();
      for (const item of run.items) {
        this.setCheckRecord(protection, item, this.checkOutcome(account, item, now));
      }
      protection.run = null;
      this.deliverTestMessages(account, run.items);
      this.emit(["protection", "mail"], accountId);
    }
    return this.protectionSetup(account);
  }

  /** The check's control and test messages, as they arrived, in Deleted Items (where the core moves them). */
  private deliverTestMessages(account: Account, items: CheckItem[]): void {
    const trash = this.state.folders.find((f) => f.accountId === account.id && f.role === "trash");
    if (!trash) return;
    const me = { name: account.displayName, address: account.email };
    const catalog = demoLabelCatalog().labels;
    const token = Math.random().toString(32).slice(2, 10).padEnd(8, "a");
    const arrived = (label: MessageLabel | null, encrypted: boolean) => {
      const id = this.id("msg");
      const attachments = encrypted
        ? [
            {
              id: `${id}-att-0`,
              name: "message.rpmsg",
              contentType: "application/x-microsoft-rpmsg-message",
              size: 21_504,
              isInline: false,
              contentId: null,
            },
          ]
        : [];
      const text = "Automated setup check from Cumulas Mail. You can delete it.";
      this.state.messages.push({
        id,
        accountId: account.id,
        folderId: trash.id,
        remoteId: id,
        conversationId: null,
        subject: `Cumulas Mail setup check [${token}]`,
        from: me,
        to: [me],
        cc: [],
        preview: encrypted ? "" : text,
        receivedAt: new Date().toISOString(),
        isRead: true,
        isFlagged: false,
        isDraft: false,
        hasAttachments: encrypted,
        importance: "normal",
        bcc: [],
        replyTo: [],
        bodyHtml: encrypted ? "" : `<p>${text}</p>`,
        bodyText: encrypted ? "" : text,
        hasBlockedContent: false,
        attachments,
        internetMessageId: `<${id}@demo.example>`,
        inReplyTo: null,
        references: [],
        protection: encrypted
          ? {
              kind: "rightsManaged",
              containerIds: [`${id}-att-0`],
              portalUrl: null,
              contentHidden: false,
            }
          : null,
        label,
        requested: null,
      });
    };
    const labelOf = (id: string): MessageLabel | null => {
      const label = catalog.find((l) => l.id === id);
      return label ? { id, name: labelTitle(label), color: label.color, fromCatalog: true } : null;
    };
    // The control: Acme's default label is on all mail.
    arrived(
      items.some((i) => i.kind === "label" && i.labelId === DEMO_LABEL_IDS.public)
        ? labelOf(DEMO_LABEL_IDS.public)
        : null,
      false,
    );
    const protection = this.protectionOf(account);
    for (const item of items) {
      const applied = !!protection && this.checkRecord(protection, item).state !== "notApplied";
      if (item.kind === "action") arrived(null, applied);
      else {
        const label = applied ? labelOf(item.labelId) : null;
        const encrypts = catalog.find((l) => l.id === item.labelId)?.hasProtection ?? false;
        arrived(label, applied && encrypts);
      }
    }
    recount(this.state);
  }

  async openProtectionCheckMessage(accountId: string, item: CheckItem) {
    await this.delay();
    const account = this.account(accountId);
    // Like the core: only work and school accounts have checks.
    const protection = this.protectionOf(account);
    if (!protection) fail("unsupported", WORK_ACCOUNTS_ONLY);
    if (!this.checkRecord(protection, item).testMessage) {
      fail("notFound", NO_TEST_MESSAGE);
    }
    this.calls.openedInOutlook.push(`check:${accountId}:${checkKey(item)}`);
  }

  /** "Copy details for IT", in the core's shape, from what the demo's checks found. */
  async getProtectionCheckDetails(accountId: string) {
    await this.delay();
    const account = this.account(accountId);
    if (!this.protectionOf(account)) fail("unsupported", WORK_ACCOUNTS_ONLY);
    const setup = this.protectionSetup(account);
    const protection = this.protectionOf(account)!;
    const record = (item: CheckItem) => this.checkRecord(protection, item);
    const encrypt = { kind: "action", action: "encrypt" } as const;
    const doNotForward = { kind: "action", action: "doNotForward" } as const;
    const changed = this.orgSetupChanged.get(this.tenantOf(account));
    return checkDetails({
      version: (await this.getAppInfo()).version,
      email: account.email,
      tenant: this.tenantOf(account),
      relay: this.throughRelay(account),
      items: [
        { title: "Encrypt", item: encrypt, result: setup.encrypt, record: record(encrypt) },
        {
          title: "Do Not Forward",
          item: doNotForward,
          result: setup.doNotForward,
          record: record(doNotForward),
        },
        // Labels only once checked: an organization may list many.
        ...setup.labels
          .filter((l) => l.check.state !== "notChecked")
          .map((l) => {
            const item = { kind: "label", labelId: l.id } as const;
            return {
              title: `Label ${labelTitle(l)} (${l.id})`,
              item,
              result: l.check,
              record: record(item),
            };
          }),
      ],
      labels: setup.labels.map((l) => ({
        id: l.id,
        title: labelTitle(l),
        encrypts: l.hasProtection,
      })),
      rulesChangedAt: changed?.rulesChangedAt ?? null,
      policiesChangedAt: changed?.policiesChangedAt ?? null,
    });
  }

  // --- organization setup ---------------------------------------------------------
  //
  // The work account's organization as `?orgsetup=` has it set up
  // (orgsetup.ts): an administrator signs in with the setup app, reads what's
  // set up, and makes the changes the dialog confirms, or copies the
  // PowerShell script. "unconfigured" is a build without a setup app, which
  // offers only the script.

  /** Like the core: only work and school accounts have an organization to set up. */
  private setupAccount(accountId: string): Account {
    const account = this.account(accountId);
    if (!this.isWorkAccount(account)) fail("unsupported", ORG_SETUP_WORK_ONLY);
    return account;
  }

  private tenantOf(account: Account): string {
    return account.settings.kind === "microsoft" ? account.settings.tenantId : "";
  }

  /** What Purview lists: the demo's labels, and one the work account isn't given. */
  private purviewLabels(): PurviewLabel[] {
    return [
      ...demoLabelCatalog().labels.map((l) => ({
        id: l.id,
        name: l.name,
        title: labelTitle(l),
        encrypts: l.hasProtection,
      })),
      BOARD_LABEL,
    ];
  }

  /** The account's organization as setup finds it, set up as `?orgsetup=` says until changed. */
  private organization(account: Account): DemoTenant {
    const tenant = this.tenantOf(account);
    let organization = this.organizations.get(tenant);
    if (!organization) {
      organization = demoTenant(this.orgSetupKind, this.purviewLabels());
      this.organizations.set(tenant, organization);
    }
    return organization;
  }

  /** What the core knows of the account and the administrator without Microsoft. */
  private setupContext(account: Account, admin: OrgSetupAdmin): SetupContext {
    const catalog = this.protectionOf(account)?.catalog?.labels ?? null;
    return {
      email: account.email,
      catalog:
        catalog?.map((l) => ({ id: l.id, title: labelTitle(l), encrypts: l.hasProtection })) ??
        null,
      needsPermission: account.needsPermission.includes("sensitivityLabels"),
      refused: account.refusedFeatures.includes("sensitivityLabels"),
      admin,
      adminObjectId: SETUP_ADMIN_OBJECT_ID,
      signaturesManageUrl: SIGNATURES_MANAGE_URL,
      now: Date.now(),
    };
  }

  /** The setup sign-in for the account's organization; like the core, setupSignedOut without one. */
  private setupSession(account: Account): NonNullable<MockBackend["orgSetupSession"]> {
    const session = this.orgSetupSession;
    if (!session || session.tenantId !== this.tenantOf(account)) {
      fail("authRequired", SIGNED_OUT, { reason: "setupSignedOut" });
    }
    return session;
  }

  /** The demo's sessions take a while with latency: reading and making changes go through Microsoft. */
  private async microsoftCalls(times: number): Promise<void> {
    if (this.latency > 0) await new Promise((r) => setTimeout(r, this.latency * times));
  }

  async getOrgSetupInfo(accountId: string): Promise<OrgSetupInfo> {
    await this.delay();
    const account = this.setupAccount(accountId);
    const tenantId = this.tenantOf(account);
    const automatic = this.orgSetupKind !== "unconfigured";
    const session = this.orgSetupSession?.tenantId === tenantId ? this.orgSetupSession : null;
    const changed = this.orgSetupChanged.get(tenantId);
    return {
      automatic,
      tenantId,
      signedIn: session ? { ...session.admin } : null,
      approvalUrl: automatic ? mockSetupConsentUrl(tenantId) : null,
      rulesChangedAt: changed?.rulesChangedAt ?? null,
      policiesChangedAt: changed?.policiesChangedAt ?? null,
    };
  }

  /**
   * Like the core: ends the sign-in there was, then waits for Microsoft's
   * sign-in in the browser (Cancel ends it). In `consent` the setup app
   * needs approving first; failOrgSetupSignIn makes it fail otherwise.
   */
  async orgSetupSignIn(accountId: string): Promise<OrgSetupAdmin> {
    await this.delay();
    const account = this.setupAccount(accountId);
    if (this.orgSetupKind === "unconfigured") fail("notConfigured", SETUP_NOT_AVAILABLE);
    const tenant = this.tenantOf(account);
    await this.cancelOrgSetupSignIn();
    this.orgSetupSession = null;
    const at = Date.now();
    let cancel!: () => void;
    const canceled = await new Promise<boolean>((resolve) => {
      cancel = () => resolve(true);
      this.pendingOrgSetupSignIn = cancel;
      const gate = this.orgSetupSignInGate;
      if (gate) void gate.then(() => resolve(false));
      else if (this.latency > 0) setTimeout(() => resolve(false), this.latency * 10);
      else resolve(false);
    });
    if (this.pendingOrgSetupSignIn === cancel) this.pendingOrgSetupSignIn = null;
    if (canceled) {
      this.setupLog.signIn(at, tenant, "cancelled");
      fail("signInCancelled", "Sign-in was canceled.");
    }
    if (this.orgSetupKind === "consent" && !this.setupApproved.has(tenant)) {
      this.setupLog.signIn(
        at,
        tenant,
        `error consent_required · AADSTS65001 · "The user or administrator has not consented to use the application with ID '${SETUP_CLIENT_ID}' named '${SETUP_APP_NAME}'."`,
      );
      throw setupConsentError(tenant);
    }
    if (this.orgSetupSignInFailure) {
      const error = orgSetupSignInError(
        this.orgSetupSignInFailure,
        emailDomain(account.email) ?? account.email,
      );
      const aadsts = error.details?.aadsts;
      this.setupLog.signIn(
        at,
        tenant,
        `error · ${aadsts ? `AADSTS${aadsts} · ` : ""}"${error.message}"`,
      );
      throw error;
    }
    // An administrator of the organization; in `noRole`, the account's own person, who isn't one.
    const admin: OrgSetupAdmin =
      this.orgSetupKind === "noRole"
        ? { username: account.email, displayName: account.displayName }
        : {
            username: `${SETUP_ADMIN.name}@${emailDomain(account.email) ?? "acme.example"}`,
            displayName: SETUP_ADMIN.displayName,
          };
    this.setupLog.signIn(
      at,
      tenant,
      'code redeemed · granted "https://outlook.office365.com/Exchange.Manage openid profile offline_access" · ID token yes · refresh token yes',
    );
    this.setupLog.signedIn(admin.username);
    this.orgSetupSession = { tenantId: tenant, admin, read: false };
    return { ...admin };
  }

  async cancelOrgSetupSignIn() {
    this.pendingOrgSetupSignIn?.();
    this.pendingOrgSetupSignIn = null;
  }

  async readOrgSetup(accountId: string): Promise<OrgSetupStatus> {
    await this.delay();
    const account = this.setupAccount(accountId);
    if (this.orgSetupKind === "unconfigured") fail("notConfigured", SETUP_NOT_AVAILABLE);
    const session = this.setupSession(account);
    await this.microsoftCalls(10);
    // Signed out meanwhile: the core's read ends the same way.
    if (this.orgSetupSession !== session) this.setupSession(account);
    const organization = this.organization(account);
    const context = this.setupContext(account, session.admin);
    this.setupLog.read(context.now, organization, account.email);
    session.read = true;
    return orgSetupStatus(organization, context);
  }

  async planOrgSetup(accountId: string, choices: OrgSetupChoices): Promise<OrgSetupPlan> {
    await this.delay();
    const { organization, context } = this.setupToChange(accountId, choices);
    return planOrgSetup(organization, context, choices).plan;
  }

  /** The organization and what the core knows, for planning `choices` after a read. */
  private setupToChange(
    accountId: string,
    choices: OrgSetupChoices,
  ): {
    account: Account;
    session: { admin: OrgSetupAdmin };
    organization: DemoTenant;
    context: SetupContext;
  } {
    const account = this.setupAccount(accountId);
    if (this.orgSetupKind === "unconfigured") fail("notConfigured", SETUP_NOT_AVAILABLE);
    const session = this.setupSession(account);
    if (!session.read) fail("invalidInput", READ_FIRST);
    const organization = this.organization(account);
    const context = this.setupContext(account, session.admin);
    const invalid = invalidChoice(organization, context, choices);
    if (invalid) fail("invalidInput", invalid);
    return { account, session, organization, context };
  }

  /**
   * Like the core: reads again, and makes the changes only when they're
   * still the ones `confirmed`; else `stale`, with nothing changed. The
   * changes are what the demo's organization is made of from then on.
   */
  async applyOrgSetup(
    accountId: string,
    choices: OrgSetupChoices,
    confirmed: string[],
  ): Promise<OrgSetupResult> {
    await this.delay();
    const { account, session, organization, context } = this.setupToChange(accountId, choices);
    const tenant = this.tenantOf(account);
    const before = orgSetupStatus(organization, context);
    this.setupLog.read(context.now, organization, account.email);
    const { plan, planned } = planOrgSetup(organization, context, choices);
    const ids = new Set(plan.changes.map((c) => c.id));
    const asked = new Set(confirmed);
    if (ids.size !== asked.size || [...ids].some((id) => !asked.has(id))) {
      return { stale: true, signedOut: false, changes: [], status: before };
    }
    await this.microsoftCalls(15);
    if (this.orgSetupSession !== session) {
      // The dialog closed before Microsoft was asked: nothing was changed.
      return {
        stale: false,
        signedOut: false,
        changes: plan.changes.map((change) => ({
          change,
          outcome: "skipped",
          message: "The setup window was closed.",
        })),
        status: before,
      };
    }
    const now = Date.now();
    const made = makeChanges(organization, planned, now, this.setupLog);
    if (made.rulesChanged || made.policiesChanged) {
      const at = new Date(now).toISOString();
      const last = this.orgSetupChanged.get(tenant);
      this.orgSetupChanged.set(tenant, {
        rulesChangedAt: made.rulesChanged ? at : (last?.rulesChangedAt ?? null),
        policiesChangedAt: made.policiesChanged ? at : (last?.policiesChangedAt ?? null),
      });
    }
    let status = before;
    if (made.signedOut) {
      this.orgSetupSession = null;
      this.setupLog.note(now, "Signed out: Microsoft 365 didn't accept the sign-in any more.");
    } else {
      const after = this.setupContext(account, session.admin);
      this.setupLog.read(after.now, organization, account.email);
      status = orgSetupStatus(organization, after);
    }
    this.emit(["protection"], accountId);
    return { stale: false, signedOut: made.signedOut, changes: made.changes, status };
  }

  /**
   * A shortened script with the core's header lines, and the portal steps
   * of the labels it covers: the chosen ones, else the account's when
   * `request.labels` is set. Works without signing in.
   */
  async getOrgSetupScript(
    accountId: string,
    request: OrgSetupScriptRequest,
  ): Promise<OrgSetupScript> {
    await this.delay();
    const account = this.setupAccount(accountId);
    const tenant = this.tenantOf(account);
    const catalog = this.protectionOf(account)?.catalog?.labels ?? [];
    const chosen = request.choices?.labelIds.map((id) => id.trim().toLowerCase());
    // Like the core's validation: only labels the account or Purview lists.
    const known = this.purviewLabels();
    if (chosen?.some((id) => !known.some((l) => l.id === id))) {
      fail("invalidInput", "That label isn't one Purview lists.");
    }
    const labels = !request.labels
      ? []
      : chosen
        ? known.filter((l) => chosen.includes(l.id))
        : catalog.map((l) => ({ id: l.id, name: l.name, title: labelTitle(l) }));
    const portalSteps = labels.map((label) => ({
      labelTitle: label.title,
      ...policyNames(label),
      pattern: `(?i)^(${label.id})$`,
    }));
    const made = new Date().toISOString().slice(0, 16).replace("T", " ");
    const org = emailDomain(account.email) ?? account.email;
    const lines = [
      `# Cumulas Mail: encryption and sensitivity labels for ${org} (tenant ${tenant}).`,
      `# Made by Cumulas Mail ${(await this.getAppInfo()).version} on ${made} UTC. Safe to run again: it creates what's missing,`,
      "# repairs what differs, and never changes or deletes anything whose name doesn't start with",
      '# "Cumulas Mail:" (it may move other rules down a place, keeping their order).',
      "# (A shortened demo script.)",
      "",
      "$ErrorActionPreference = 'Stop'",
      "Connect-ExchangeOnline -ShowBanner:$false",
      ...(labels.length
        ? [
            `New-TransportRule -Name ${psQuote(`${OWN_PREFIX}ignore label requests from outside`)} -ExceptIfFromScope InOrganization -RemoveHeader 'x-cumulas-label' -Priority 0 -Mode Enforce`,
          ]
        : []),
      ...(request.actions
        ? [
            `New-TransportRule -Name ${psQuote(`${OWN_PREFIX}Encrypt`)} -ExceptIfFromScope NotInOrganization -HeaderMatchesMessageHeader 'x-cumulas-protect' -HeaderMatchesPatterns '^encrypt$' -ApplyRightsProtectionTemplate ${psQuote(request.choices?.encryptTemplate ?? "Encrypt")} -RemoveHeader 'x-cumulas-protect' -RuleErrorAction Defer -Mode Enforce`,
            `New-TransportRule -Name ${psQuote(`${OWN_PREFIX}Do Not Forward`)} -ExceptIfFromScope NotInOrganization -HeaderMatchesMessageHeader 'x-cumulas-protect' -HeaderMatchesPatterns '^do-not-forward$' -ApplyRightsProtectionTemplate ${psQuote(request.choices?.doNotForwardTemplate ?? "Do Not Forward")} -RemoveHeader 'x-cumulas-protect' -RuleErrorAction Defer -Mode Enforce`,
          ]
        : []),
      ...(labels.length
        ? [
            "Disconnect-ExchangeOnline -Confirm:$false",
            "Connect-IPPSSession",
            ...portalSteps.flatMap((steps) => [
              `New-AutoSensitivityLabelPolicy -Name ${psQuote(steps.policyName)} -ExchangeLocation 'All' -OverwriteLabel $true -Mode TestWithoutNotifications`,
              `New-AutoSensitivityLabelRule -Policy ${psQuote(steps.policyName)} -Name ${psQuote(steps.ruleName)} -Workload Exchange -HeaderMatchesPatterns @{ 'x-cumulas-label' = ${psQuote(steps.pattern)} }`,
            ]),
          ]
        : []),
      "Disconnect-ExchangeOnline -Confirm:$false",
    ];
    return { script: `${lines.join("\n")}\n`, portalSteps };
  }

  /** "Copy setup details": empty until a setup sign-in was tried. */
  async getOrgSetupDetails() {
    await this.delay();
    return this.setupLog.text((await this.getAppInfo()).version);
  }

  /** Forgets the setup sign-in; "Copy setup details" keeps what it did. */
  async endOrgSetup() {
    this.calls.orgSetupEnded += 1;
    this.orgSetupSession = null;
  }

  /** Keeps setup sign-ins waiting in the browser until the returned function is called, or Cancel (tests). */
  holdOrgSetupSignIn(): () => void {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => (release = resolve));
    this.orgSetupSignInGate = gate;
    return () => {
      if (this.orgSetupSignInGate === gate) this.orgSetupSignInGate = null;
      release();
    };
  }

  /** Makes setup sign-ins fail as the core says `failure`, or work again with null (tests, demos). */
  failOrgSetupSignIn(failure: OrgSetupSignInFailure | null): void {
    this.orgSetupSignInFailure = failure;
  }

  /** Ends the setup sign-in behind the dialog's back, as 60 minutes without use do (tests). */
  expireOrgSetup(): void {
    this.orgSetupSession = null;
  }

  /**
   * Sets the organizations up as `kind` has them, as another administrator
   * might while the dialog is open (tests): a plan shown before is stale.
   */
  setOrgSetup(kind: DemoOrgSetup): void {
    this.orgSetupKind = kind;
    this.organizations.clear();
  }

  // --- signatures -----------------------------------------------------------------

  /** Puts the demo's company signature in a state (`?signature=`). */
  private demoSignature(accountId: string, kind: DemoSignature): void {
    const answered = (answer: OrganizationReply) =>
      this.setOrganizationSignature(
        accountId,
        { ...answer, checkedAt: new Date().toISOString(), stale: false },
        answer,
      );
    switch (kind) {
      case "addedOnSend":
        return answered({ state: "addedOnSend", html: null, onReplies: true });
      case "notAllowed":
        return answered({ state: "notAllowed", html: null, onReplies: false });
      case "notUsed":
        return answered(NOT_USED);
      case "stale":
        return this.setOrganizationSignature(accountId, { stale: true }, "unreachable");
      case "unreachable":
        // Its first check failed: nothing is known.
        return this.setOrganizationSignature(
          accountId,
          { state: "unchecked", html: null, onReplies: false, checkedAt: null, stale: true },
          "unreachable",
        );
    }
  }

  /**
   * Sets what the account's organization last said (`known`) and what it
   * says when asked again, as the desktop app would have them (tests,
   * demos).
   */
  setOrganizationSignature(
    accountId: string,
    known: Partial<Omit<OrganizationSignature, "checkableAt">>,
    next: OrganizationAnswer,
  ): void {
    const signature = this.accountSignature(accountId);
    signature.organization = { ...signature.organization, ...known };
    this.organizationAnswers.set(accountId, next);
    this.emit(["signatures"], accountId);
  }

  /** Like the core: IMAP and personal Microsoft accounts have no company signature. */
  private accountSignature(accountId: string): AccountSignature {
    const account = this.account(accountId);
    let signature = this.signatures.get(accountId);
    if (!signature) {
      const supported =
        account.settings.kind === "microsoft" &&
        account.settings.tenantId !== "" &&
        account.settings.tenantId !== CONSUMER_TENANT;
      signature = {
        accountId,
        ownHtml: "",
        ownOnReplies: true,
        organization: {
          state: supported ? "unchecked" : "unsupported",
          html: null,
          onReplies: false,
          checkedAt: null,
          stale: false,
          checkableAt: null,
        },
      };
      this.signatures.set(accountId, signature);
    }
    return signature;
  }

  /**
   * Like the core: when checking would ask the organization again (ms), a
   * minute after the last check and not before its service said (unless it
   * said so while the clock was ahead); undefined when at once.
   */
  private checkableAt(accountId: string): number | undefined {
    const last = this.signatureChecks.get(accountId);
    if (last === undefined) return undefined;
    const now = Date.now();
    // The core times the minute on a clock that's never set: never more
    // than a minute from now.
    let at = Math.min(last, now) + CHECK_INTERVAL_MS;
    const retry = this.signatureRetries.get(accountId);
    if (retry !== undefined && last <= now + CLOCK_SLACK_MS) at = Math.max(at, retry);
    return at > now ? at : undefined;
  }

  /** The account's signatures as the UI gets them. */
  private publicSignature(accountId: string): AccountSignature {
    const signature = clone(this.accountSignature(accountId));
    const at = this.checkableAt(accountId);
    signature.organization.checkableAt = at === undefined ? null : new Date(at).toISOString();
    return signature;
  }

  async getAccountSignature(accountId: string) {
    await this.delay();
    return this.publicSignature(accountId);
  }

  async updateOwnSignature(accountId: string, html: string, onReplies: boolean) {
    await this.delay();
    const signature = this.accountSignature(accountId);
    if (new TextEncoder().encode(html).length > MAX_SIGNATURE_BYTES) {
      fail("invalidInput", "That signature is too large. Signatures can be 512 KB at most.");
    }
    signature.ownHtml = cleanSignature(html);
    signature.ownOnReplies = onReplies;
    this.emit(["signatures"], accountId);
    return this.publicSignature(accountId);
  }

  async checkOrganizationSignature(accountId: string) {
    await this.delay();
    const signature = this.accountSignature(accountId);
    const before = signature.organization;
    // Like the core, at most once a minute, and not before the service said.
    if (before.state === "unsupported" || this.checkableAt(accountId) !== undefined) {
      return this.publicSignature(accountId);
    }
    const now = Date.now();
    this.signatureChecks.set(accountId, now);
    const answer = this.organizationAnswers.get(accountId) ?? NOT_USED;
    const busy = typeof answer === "object" && "retryAfter" in answer;
    if (busy) {
      const wait = Math.min(
        Math.max(answer.retryAfter * 1000, MIN_RETRY_AFTER_MS),
        MAX_RETRY_AFTER_MS,
      );
      this.signatureRetries.set(accountId, now + wait);
    } else {
      this.signatureRetries.delete(accountId);
    }
    signature.organization =
      answer === "unreachable" || busy
        ? { ...before, stale: true }
        : {
            ...answer,
            html: answer.html && cleanSignature(answer.html),
            checkedAt: new Date().toISOString(),
            stale: false,
            // Worked out as it's asked for.
            checkableAt: null,
          };
    const after = signature.organization;
    if (
      after.state !== before.state ||
      after.html !== before.html ||
      after.onReplies !== before.onReplies ||
      after.stale !== before.stale
    ) {
      this.emit(["signatures"], accountId);
    }
    return this.publicSignature(accountId);
  }

  /**
   * Like the core: waits for an organization's first answer; asks one that
   * signs the account's mail again, for the next message, when its answer
   * is more than an hour old, but not one that doesn't (asked once a day).
   */
  async getComposeSignature(accountId: string, replyKind: ReplyKind | null) {
    await this.delay();
    let signature = this.accountSignature(accountId);
    const { state, checkedAt } = signature.organization;
    if (state === "unchecked") {
      signature = await this.checkOrganizationSignature(accountId);
    } else if (
      (state === "ready" || state === "addedOnSend") &&
      (!checkedAt || Date.now() - new Date(checkedAt).getTime() > COMPOSE_RECHECK_MS)
    ) {
      void this.checkOrganizationSignature(accountId);
    }
    const organization = signature.organization;
    const fresh = replyKind === null;
    // While the organization signs the account's mail, the own signature
    // isn't used, not even on the replies and forwards it leaves unsigned.
    const signed = fresh || organization.onReplies;
    let result: ComposeSignature = { kind: "none", html: null };
    if (organization.state === "ready" && organization.html) {
      if (signed) result = { kind: "organization", html: organization.html };
    } else if (organization.state === "addedOnSend") {
      if (signed) result = { kind: "addedOnSend", html: null };
    } else if (signature.ownHtml && (fresh || signature.ownOnReplies)) {
      result = { kind: "own", html: signature.ownHtml };
    }
    return result;
  }

  // --- calendar -------------------------------------------------------------------

  async listCalendars() {
    await this.delay();
    return clone(this.state.calendars);
  }

  async listEvents(start: string, end: string, calendarIds?: string[]) {
    await this.delay();
    return clone(
      this.state.events
        .filter((e) => e.start < end && e.end > start)
        .filter((e) => !calendarIds || calendarIds.includes(e.calendarId))
        .sort((a, b) => a.start.localeCompare(b.start) || b.end.localeCompare(a.end)),
    );
  }

  async refreshEvents() {
    await this.delay();
  }

  private eventFromDraft(id: string, draft: Parameters<Backend["createEvent"]>[0]): CalendarEvent {
    if (!draft.subject.trim()) fail("invalidInput", "Add a title for the event.");
    if (draft.end < draft.start) fail("invalidInput", "The event ends before it starts.");
    const calendar =
      this.state.calendars.find((c) => c.id === draft.calendarId) ??
      fail("notFound", "calendar not found");
    if (!calendar.canEdit) fail("invalidInput", "This calendar is read-only.");
    const account = this.account(calendar.accountId);
    return {
      id,
      accountId: calendar.accountId,
      calendarId: calendar.id,
      remoteId: id,
      subject: draft.subject.trim(),
      location: draft.location?.trim() || null,
      bodyHtml: draft.bodyHtml ?? null,
      bodyPreview: draft.bodyHtml
        ? draft.bodyHtml
            .replace(/<[^>]+>/g, " ")
            .trim()
            .slice(0, 200)
        : null,
      start: draft.start,
      end: draft.end,
      isAllDay: draft.isAllDay,
      organizer: { name: account.displayName, address: account.email },
      attendees: draft.attendees.map((email) => ({ email, kind: "required", response: "none" })),
      showAs: draft.showAs,
      responseStatus: "organizer",
      isOnlineMeeting: draft.isOnlineMeeting && account.kind === "microsoft",
      onlineMeetingUrl:
        draft.isOnlineMeeting && account.kind === "microsoft" ? `https://meet.example/${id}` : null,
      reminderMinutes: draft.reminderMinutes ?? null,
      isRecurring: false,
      isCancelled: false,
    };
  }

  async createEvent(draft: Parameters<Backend["createEvent"]>[0]) {
    await this.delay();
    const calendar = this.state.calendars.find((c) => c.id === draft.calendarId);
    if (calendar) this.checkFeature(calendar.accountId, "calendar");
    const event = this.eventFromDraft(this.id("evt"), draft);
    this.state.events.push(event);
    this.emit(["calendar"], event.accountId);
    return clone(event);
  }

  async updateEvent(eventId: string, draft: Parameters<Backend["updateEvent"]>[1]) {
    await this.delay();
    const index = this.state.events.findIndex((e) => e.id === eventId);
    if (index < 0) fail("notFound", "event not found");
    const existing = this.state.events[index]!;
    const updated = this.eventFromDraft(eventId, draft);
    const event: CalendarEvent = {
      ...updated,
      isRecurring: existing.isRecurring,
      isCancelled: existing.isCancelled,
      organizer: existing.organizer,
      // Editing keeps the user's own response and what attendees answered.
      responseStatus: existing.responseStatus,
      attendees: updated.attendees.map(
        (a) =>
          existing.attendees.find(
            (old) => old.email.address.toLowerCase() === a.email.address.toLowerCase(),
          ) ?? a,
      ),
      onlineMeetingUrl: updated.isOnlineMeeting
        ? (existing.onlineMeetingUrl ?? updated.onlineMeetingUrl)
        : null,
    };
    this.state.events[index] = event;
    this.emit(["calendar"], event.accountId);
    return clone(event);
  }

  async deleteEvent(eventId: string) {
    await this.delay();
    this.state.events = this.state.events.filter((e) => e.id !== eventId);
    this.emit(["calendar"]);
  }

  async respondToEvent(eventId: string, response: Parameters<Backend["respondToEvent"]>[1]) {
    await this.delay();
    const event =
      this.state.events.find((e) => e.id === eventId) ?? fail("notFound", "event not found");
    if (this.account(event.accountId).kind !== "microsoft") {
      fail("unsupported", "Meeting responses need a Microsoft account.");
    }
    if (response === "decline") {
      // Like Exchange: a declined meeting is removed from the calendar.
      this.state.events = this.state.events.filter((e) => e.id !== eventId);
    } else {
      event.responseStatus = response === "accept" ? "accepted" : "tentative";
      const me = this.account(event.accountId).email.toLowerCase();
      for (const a of event.attendees) {
        if (a.email.address.toLowerCase() === me) a.response = event.responseStatus;
      }
    }
    this.emit(["calendar"], event.accountId);
    return clone(event);
  }

  // --- contacts -------------------------------------------------------------------

  async listContacts(accountId?: string, text?: string) {
    await this.delay();
    const q = text?.trim().toLowerCase();
    return clone(
      this.state.contacts
        .filter((c) => !accountId || c.accountId === accountId)
        .filter(
          (c) =>
            !q ||
            [c.displayName, c.givenName, c.surname, c.company, ...c.emails.map((e) => e.address)]
              .join(" ")
              .toLowerCase()
              .includes(q),
        )
        .sort((a, b) =>
          a.displayName.localeCompare(b.displayName, undefined, { sensitivity: "base" }),
        ),
    );
  }

  private contactFromDraft(id: string, draft: Parameters<Backend["createContact"]>[0]): Contact {
    if (!draft.displayName.trim()) fail("invalidInput", "Add a name for the contact.");
    this.account(draft.accountId);
    return {
      id,
      accountId: draft.accountId,
      remoteId: id,
      displayName: draft.displayName.trim(),
      givenName: draft.givenName ?? null,
      surname: draft.surname ?? null,
      emails: draft.emails,
      phones: draft.phones,
      company: draft.company ?? null,
      jobTitle: draft.jobTitle ?? null,
      department: draft.department ?? null,
      address: draft.address ?? null,
      birthday: draft.birthday ?? null,
      notes: draft.notes ?? null,
    };
  }

  async createContact(draft: Parameters<Backend["createContact"]>[0]) {
    await this.delay();
    this.checkFeature(draft.accountId, "contacts");
    const contact = this.contactFromDraft(this.id("contact"), draft);
    this.state.contacts.push(contact);
    this.emit(["contacts"], contact.accountId);
    return clone(contact);
  }

  async updateContact(contactId: string, draft: Parameters<Backend["updateContact"]>[1]) {
    await this.delay();
    const index = this.state.contacts.findIndex((c) => c.id === contactId);
    if (index < 0) fail("notFound", "contact not found");
    if (this.state.contacts[index]!.accountId !== draft.accountId) {
      fail("unsupported", "Contacts cannot be moved between accounts.");
    }
    const contact = this.contactFromDraft(contactId, draft);
    this.state.contacts[index] = contact;
    this.emit(["contacts"], contact.accountId);
    return clone(contact);
  }

  async deleteContact(contactId: string) {
    await this.delay();
    this.state.contacts = this.state.contacts.filter((c) => c.id !== contactId);
    this.emit(["contacts"]);
  }

  // --- tasks ----------------------------------------------------------------------

  async listTaskLists() {
    await this.delay();
    return clone(this.state.taskLists);
  }

  async listTasks(listId: string | null, includeCompleted: boolean) {
    await this.delay();
    return clone(
      this.state.tasks
        .filter((t) => !listId || t.listId === listId)
        .filter((t) => includeCompleted || t.status !== "completed")
        .sort(
          (a, b) =>
            Number(a.status === "completed") - Number(b.status === "completed") ||
            Number(a.dueDate === null) - Number(b.dueDate === null) ||
            (a.dueDate ?? "").localeCompare(b.dueDate ?? "") ||
            b.createdAt.localeCompare(a.createdAt),
        ),
    );
  }

  async createTask(draft: Parameters<Backend["createTask"]>[0]) {
    await this.delay();
    if (!draft.title.trim()) fail("invalidInput", "Add a title for the task.");
    const list =
      this.state.taskLists.find((l) => l.id === draft.listId) ??
      fail("notFound", "task list not found");
    this.checkFeature(list.accountId, "tasks");
    const now = new Date().toISOString();
    const id = this.id("task");
    const task: Task = {
      id,
      accountId: list.accountId,
      listId: list.id,
      remoteId: id,
      title: draft.title.trim(),
      notes: draft.notes?.trim() || null,
      status: "notStarted",
      importance: draft.importance ?? "normal",
      dueDate: draft.dueDate ?? null,
      reminderAt: draft.reminderAt ?? null,
      completedAt: null,
      createdAt: now,
      updatedAt: now,
    };
    this.state.tasks.push(task);
    this.emit(["tasks"], task.accountId);
    return clone(task);
  }

  async updateTask(taskId: string, patch: Parameters<Backend["updateTask"]>[1]) {
    await this.delay();
    const task =
      this.state.tasks.find((t) => t.id === taskId) ?? fail("notFound", "task not found");
    if (patch.title !== undefined) {
      if (!patch.title.trim()) fail("invalidInput", "A task needs a title.");
      task.title = patch.title.trim();
    }
    if (patch.clearNotes) task.notes = null;
    else if (patch.notes !== undefined) task.notes = patch.notes;
    if (patch.importance) task.importance = patch.importance;
    if (patch.clearDueDate) task.dueDate = null;
    else if (patch.dueDate) task.dueDate = patch.dueDate;
    if (patch.clearReminder) task.reminderAt = null;
    else if (patch.reminderAt) task.reminderAt = new Date(patch.reminderAt).toISOString();
    if (patch.status) {
      if (patch.status === "completed" && task.status !== "completed") {
        task.completedAt = new Date().toISOString();
      } else if (patch.status !== "completed") {
        task.completedAt = null;
      }
      task.status = patch.status;
    }
    task.updatedAt = new Date().toISOString();
    this.emit(["tasks"], task.accountId);
    return clone(task);
  }

  async deleteTask(taskId: string) {
    await this.delay();
    this.state.tasks = this.state.tasks.filter((t) => t.id !== taskId);
    this.emit(["tasks"]);
  }

  // --- events ---------------------------------------------------------------------

  onDataChanged(handler: (event: DataChangedEvent) => void): Unsubscribe {
    this.dataHandlers.add(handler);
    return () => this.dataHandlers.delete(handler);
  }

  onSyncStatus(handler: (status: SyncStatus) => void): Unsubscribe {
    this.syncHandlers.add(handler);
    return () => this.syncHandlers.delete(handler);
  }

  onPushStatus(handler: (status: PushStatus) => void): Unsubscribe {
    this.pushHandlers.add(handler);
    return () => this.pushHandlers.delete(handler);
  }

  onNavigate(handler: (to: Navigate) => void): Unsubscribe {
    this.navigateHandlers.add(handler);
    return () => this.navigateHandlers.delete(handler);
  }

  onBackgroundState(handler: (state: BackgroundState) => void): Unsubscribe {
    this.backgroundHandlers.add(handler);
    return () => this.backgroundHandlers.delete(handler);
  }

  onQuitRequested(handler: () => void): Unsubscribe {
    this.quitHandlers.add(handler);
    return () => this.quitHandlers.delete(handler);
  }

  onUpdateState(handler: (state: UpdateState) => void): Unsubscribe {
    this.updateHandlers.add(handler);
    return () => this.updateHandlers.delete(handler);
  }

  onRestartRequested(handler: () => void): Unsubscribe {
    this.restartHandlers.add(handler);
    return () => this.restartHandlers.delete(handler);
  }

  onRemindersChanged(handler: (event: RemindersChangedEvent) => void): Unsubscribe {
    this.reminderHandlers.add(handler);
    return () => this.reminderHandlers.delete(handler);
  }
}

function emptyState(): MockState {
  return {
    accounts: [],
    folders: [],
    messages: [],
    calendars: [],
    events: [],
    contacts: [],
    taskLists: [],
    tasks: [],
  };
}
