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
  ComposeSignature,
  Contact,
  DataChangedEvent,
  DataKind,
  ErrorCode,
  Message,
  MessageSummary,
  Navigate,
  OrganizationSignature,
  PermissionRequest,
  PermissionRequiredReason,
  PushStatus,
  RemindersChangedEvent,
  ReplyKind,
  ServerSearchResult,
  ServerSearchStatus,
  Settings,
  SignInBlockedReason,
  SnoozeFor,
  SyncStatus,
  Task,
} from "../types";
import { cleanFamily } from "../../lib/fonts";
import {
  FEATURE_COPY,
  emailDomain,
  isConsumerDomain,
  isPersonalAccount,
} from "../../modules/settings/microsoft";
import { type MockState, createDemoState, recount } from "./data";
import { isEmptySearch, isNarrowEnoughForServers, matchesSearch, parseSearch } from "./searchQuery";
import { createWorstState } from "./worst";

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

/** The optional features of a Microsoft account, in the order the core lists them. */
const FEATURES: readonly AccountFeature[] = ["calendar", "contacts", "tasks", "mailboxSettings"];

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

export interface MockOptions {
  /** Artificial latency in ms for every call (default 0). */
  latency?: number;
  /** How long a server search takes, in ms (default: `latency`). */
  serverSearchLatency?: number;
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
  readonly calls = { quit: 0, quitCanceled: 0, testNotifications: 0 };
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

  constructor(options: MockOptions = {}) {
    this.latency = options.latency ?? 0;
    this.serverSearchLatency = options.serverSearchLatency ?? this.latency;
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
    const microsoft = this.state.accounts.find((a) => a.kind === "microsoft");
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
    };
  }

  async openLink(url: string) {
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
    if (existing) {
      const before = [...existing.needsPermission, ...existing.refusedFeatures];
      // Every sign-in checks refused features again.
      existing.refusedFeatures = [];
      if (permissions.kind === "all") existing.needsPermission = [];
      else if (permissions.kind === "feature") {
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
    const account: Account = {
      id: this.id("acct"),
      kind: "microsoft",
      displayName: email.split("@")[0] ?? email,
      email,
      color: "#2b7aa8",
      settings: { kind: "microsoft", tenantId: "demo", userId: this.id("user") },
      createdAt: new Date().toISOString(),
      lastSyncedAt: null,
      syncError: null,
      needsPermission:
        permissions.kind === "mailOnly"
          ? [...FEATURES]
          : permissions.kind === "feature"
            ? FEATURES.filter((f) => f !== permissions.feature)
            : [],
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
    this.emit(["accounts", "mail", "calendar", "contacts", "tasks", "signatures"]);
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
   * Like the core: the messages the index finds (word prefixes, and inside
   * subjects and senders), at most 200, plus the ones server searches found.
   */
  async searchMessages(text: string, accountId?: string, includeIds: string[] = []) {
    await this.delay();
    const query = parseSearch(text);
    const inAccount = (m: Message) => !accountId || m.accountId === accountId;
    // As the app's index: a message's text once it was opened, its preview
    // before.
    const indexed = (m: Message) => (this.opened.has(m.id) ? m : { ...m, bodyText: null });
    const found = isEmptySearch(query)
      ? []
      : this.state.messages
          .filter((m) => inAccount(m) && matchesSearch(query, indexed(m), { infix: true }))
          .sort(searchOrder)
          .slice(0, LOCAL_SEARCH_LIMIT);
    const ids = new Set(found.map((m) => m.id));
    const included = new Set(includeIds.slice(0, MAX_INCLUDED_IDS));
    for (const m of this.state.messages) {
      if (included.has(m.id) && !ids.has(m.id) && inAccount(m)) {
        ids.add(m.id);
        found.push(m);
      }
    }
    return found.sort(searchOrder).map(summary);
  }

  /**
   * Like the core: searches one account's mail on its "server" (what is on
   * the device and `serverMessages`, matched as Graph does by the start of
   * words, or as IMAP does anywhere in them), a page at a time, and brings
   * what it finds onto the device.
   */
  async searchServer(
    searchId: string,
    text: string,
    accountId: string,
    more = false,
  ): Promise<ServerSearchResult> {
    if (!searchId || searchId.length > 100) fail("invalidInput", "Invalid search.");
    this.account(accountId);
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
    // Like the core, before the search is started: it cancels nothing.
    if (!isNarrowEnoughForServers(query)) return answer("done");
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

    let matches: string[];
    if (more) {
      const rest = search.rest.get(accountId);
      if (!rest) return answer("done");
      matches = rest;
    } else {
      // Graph matches words by their start; IMAP's SEARCH, anywhere.
      const options = { infix: false, substrings: this.account(accountId).kind === "imap" };
      matches = [...this.state.messages, ...(this.state.serverMessages ?? [])]
        .filter((m) => m.accountId === accountId && matchesSearch(query, m, options))
        .sort(searchOrder)
        .map((m) => m.id);
    }
    const page = matches.slice(0, SERVER_SEARCH_PAGE);
    const rest = matches.slice(SERVER_SEARCH_PAGE);
    if (rest.length > 0) search.rest.set(accountId, rest);
    else search.rest.delete(accountId);

    const messageIds: string[] = [];
    let added = 0;
    const server = this.state.serverMessages ?? [];
    for (const id of page) {
      const index = server.findIndex((m) => m.id === id);
      if (index >= 0) {
        // Stored now; the folder's count already had it.
        this.state.messages.push(...server.splice(index, 1));
        added += 1;
        messageIds.push(id);
      } else if (this.state.messages.some((m) => m.id === id)) {
        messageIds.push(id);
      }
    }
    if (added > 0) this.emit(["mail"], accountId);
    return answer("done", { messageIds, added, hasMore: rest.length > 0 });
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
      });
      recount(this.state);
    }
    this.emit(["mail"], account.id);
  }

  async suggestAddresses(text: string) {
    await this.delay();
    const q = text.trim().toLowerCase();
    if (!q) return [];
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

  async saveAttachment(messageId: string, attachmentId: string) {
    await this.delay();
    const att =
      this.message(messageId).attachments.find((a) => a.id === attachmentId) ??
      fail("notFound", "attachment not found");
    return `/home/alex/Downloads/${att.name}`;
  }

  async openAttachment(messageId: string, attachmentId: string) {
    await this.delay();
    if (!this.message(messageId).attachments.some((a) => a.id === attachmentId)) {
      fail("notFound", "attachment not found");
    }
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
