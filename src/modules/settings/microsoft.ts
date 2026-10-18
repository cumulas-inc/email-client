// What Cumulas Mail asks Microsoft for, in plain language, and the request
// a user can forward to IT when their organization requires admin approval.
// Keep in step with CORE_SCOPES and OPTIONAL_SCOPES in crates/core/src/auth/microsoft.rs.

import type { Account, AccountFeature, SignInBlockedReason } from "../../api/types";

export interface Permission {
  /** Graph permission names, as IT will see them. */
  scopes: string[];
  /** "Read and write your mail" — shown in the app. */
  label: string;
  /** "Read and write my mail" — used in the request to IT. */
  request: string;
  /** The optional feature it's for; none for the permissions mail needs. */
  feature?: AccountFeature;
}

export const MICROSOFT_PERMISSIONS: Permission[] = [
  {
    scopes: ["Mail.ReadWrite"],
    label: "Read and write your mail",
    request: "Read and write my mail",
  },
  { scopes: ["Mail.Send"], label: "Send mail as you", request: "Send mail as me" },
  {
    scopes: ["User.Read", "openid", "profile", "email"],
    label: "Sign you in and read your basic profile",
    request: "Sign me in and read my basic profile",
  },
  {
    scopes: ["offline_access"],
    label: "Stay signed in (offline access)",
    request: "Stay signed in (offline access)",
  },
  {
    scopes: ["MailboxSettings.Read"],
    label: "Read your mailbox settings, such as your time zone",
    request: "Read my mailbox settings, such as my time zone",
    feature: "mailboxSettings",
  },
  {
    scopes: ["Calendars.ReadWrite"],
    label: "Read and write your calendars",
    request: "Read and write my calendars",
    feature: "calendar",
  },
  {
    scopes: ["Contacts.ReadWrite"],
    label: "Read and write your contacts",
    request: "Read and write my contacts",
    feature: "contacts",
  },
  {
    scopes: ["Tasks.ReadWrite"],
    label: "Read and write your tasks",
    request: "Read and write my tasks",
    feature: "tasks",
  },
  {
    scopes: ["InformationProtectionPolicy.Read"],
    label: "Read the sensitivity labels you can use (work and school accounts)",
    request: "Read the sensitivity labels I can use",
    feature: "sensitivityLabels",
  },
];

/** The optional features, in display order (as the backend lists them). */
export const ACCOUNT_FEATURES: readonly AccountFeature[] = [
  "calendar",
  "contacts",
  "tasks",
  "mailboxSettings",
  "sensitivityLabels",
];

/**
 * Features asked for only when turned on by themselves, never with the others
 * at sign-in: generic permission messages leave them out, and they have their
 * own place in Settings → Accounts.
 */
export const ON_REQUEST_FEATURES: readonly AccountFeature[] = ["sensitivityLabels"];

/** The features in `list` that generic permission messages are about. */
export function moduleFeatures(list: readonly AccountFeature[]): AccountFeature[] {
  return list.filter((f) => !ON_REQUEST_FEATURES.includes(f));
}

/** The admin guide's section on encrypting and labeling mail sent from Cumulas Mail. */
export const PROTECTION_GUIDE_URL =
  "https://github.com/cumulas-inc/email-client/blob/main/docs/microsoft-365.md#sensitivity-labels-and-encryption-it-admins";

/** Microsoft's tenant for personal accounts (Outlook.com, Hotmail…). */
export const PERSONAL_TENANT_ID = "9188040d-6c67-4c5b-b112-36a304b66dad";

/** Where Microsoft's own page explains an AADSTS code. */
export const MICROSOFT_ERROR_LOOKUP = (code: number) =>
  `https://login.microsoftonline.com/error?code=${code}`;

/** Where people tell Cumulas about problems. */
export const ISSUES_URL = "https://github.com/cumulas-inc/email-client/issues";

/** Like Rust's to_ascii_lowercase: other letters stay as they are. */
function asciiLower(text: string): string {
  return text.replace(/[A-Z]/g, (c) => c.toLowerCase());
}

/** A personal Microsoft account (Outlook.com, Hotmail…), which has no administrator. */
export function isPersonalAccount(account: Account): boolean {
  if (account.settings.kind !== "microsoft") return false;
  const tenant = asciiLower(account.settings.tenantId);
  return tenant === PERSONAL_TENANT_ID || tenant === "consumers";
}

const CONSUMER_DOMAINS = new Set([
  "outlook.com",
  "hotmail.com",
  "live.com",
  "msn.com",
  "passport.com",
  "windowslive.com",
]);
const CONSUMER_DOMAIN = /^(hotmail|live|outlook)\.((co|com)\.)?[a-z]{2}$/;

/** A domain of Microsoft's personal accounts (outlook.com, hotmail.co.uk…). Mirrors is_consumer_domain. */
export function isConsumerDomain(domain: string): boolean {
  const d = asciiLower(domain.trim());
  return CONSUMER_DOMAINS.has(d) || CONSUMER_DOMAIN.test(d);
}

/** The text after the last "@", trimmed; undefined when there is none. */
export function emailDomain(email: string | undefined): string | undefined {
  if (!email) return undefined;
  const at = email.lastIndexOf("@");
  if (at < 0) return undefined;
  return email.slice(at + 1).trim() || undefined;
}

/** An address of Microsoft's personal accounts: no administrator to ask. */
export function isConsumerAddress(email: string | undefined): boolean {
  const domain = emailDomain(email);
  return domain !== undefined && isConsumerDomain(domain);
}

/** What an AADSTS code means for a sign-in; null for codes Cumulas Mail doesn't know. */
export type AadstsClass =
  | { kind: "blocked"; reason: SignInBlockedReason }
  | { kind: "adminConsent"; canRequestApproval: boolean }
  | { kind: "declined" }
  | null;

/** The codes that mean an administrator must approve the app (ADMIN_CONSENT_CODES). */
const ADMIN_CONSENT_CODES = [65001, 90093, 90094, 90095, 900941, 900981];
/** Microsoft's admin consent workflow is on: its page offers "Request approval". */
const ADMIN_CONSENT_WORKFLOW_CODE = 90095;

/** Mirrors the first three rows of sign_in_error_code's classification. */
export function classifyAadsts(code: number): AadstsClass {
  const blocked = (reason: SignInBlockedReason): AadstsClass => ({ kind: "blocked", reason });
  if (code === 50105) return blocked("notAssigned");
  if (code === 7000112) return blocked("appDisabled");
  if (code === 65005) return blocked("appNotSetUp");
  if (code === 650052) return blocked("serviceNotEnabled");
  if (code === 53004 || code === 53010) return blocked("securityInfo");
  if ([53000, 53001, 53002, 53003, 53011].includes(code) || (code >= 530000 && code <= 530999)) {
    return blocked("conditionalAccess");
  }
  if (ADMIN_CONSENT_CODES.includes(code)) {
    return { kind: "adminConsent", canRequestApproval: code === ADMIN_CONSENT_WORKFLOW_CODE };
  }
  if (code === 65004) return { kind: "declined" };
  return null;
}

/** Whether an approval link also covers the push service (its scope names Sync.Receive). */
export function linkIncludesPush(adminConsentUrl: string): boolean {
  try {
    return new URL(adminConsentUrl).searchParams.get("scope")?.includes("/Sync.Receive") ?? false;
  } catch {
    return false;
  }
}

export interface FeatureCopy {
  /** "Calendar": a row's name. */
  name: string;
  /** "calendar": in sentences and lists. */
  noun: string;
  /** "your calendar". */
  object: string;
  /** Its permission isn't granted. */
  title: string;
  /** Turning it on was declined. */
  notOn: string;
  /** It came back. */
  isOn: string;
  /** Microsoft 365 refused it. */
  refusedTitle: string;
}

export const FEATURE_COPY: Record<AccountFeature, FeatureCopy> = {
  calendar: {
    name: "Calendar",
    noun: "calendar",
    object: "your calendar",
    title: "Calendar needs permission",
    notOn: "Calendar wasn't turned on",
    isOn: "Calendar is on",
    refusedTitle: "Microsoft 365 refused access to your calendar",
  },
  contacts: {
    name: "Contacts",
    noun: "contacts",
    object: "your contacts",
    title: "Contacts need permission",
    notOn: "Contacts weren't turned on",
    isOn: "Contacts are on",
    refusedTitle: "Microsoft 365 refused access to your contacts",
  },
  tasks: {
    name: "Tasks",
    noun: "tasks",
    object: "your tasks",
    title: "Tasks need permission",
    notOn: "Tasks weren't turned on",
    isOn: "Tasks are on",
    refusedTitle: "Microsoft 365 refused access to your tasks",
  },
  mailboxSettings: {
    name: "Mailbox time zone",
    noun: "mailbox time zone",
    object: "your mailbox time zone",
    title: "Mailbox time zone needs permission",
    notOn: "Mailbox time zone wasn't turned on",
    isOn: "Mailbox time zone is on",
    refusedTitle: "Microsoft 365 refused access to your mailbox time zone",
  },
  sensitivityLabels: {
    name: "Sensitivity labels",
    noun: "sensitivity labels",
    object: "your sensitivity labels",
    title: "Sensitivity labels need permission",
    notOn: "Sensitivity labels weren't turned on",
    isOn: "Sensitivity labels are on",
    refusedTitle: "Microsoft 365 refused access to your sensitivity labels",
  },
};

/** The features a module of its own shows: calendar, contacts and tasks. */
export type ModuleFeature = Exclude<AccountFeature, "mailboxSettings" | "sensitivityLabels">;

/** The Microsoft accounts that can't use the feature: its permission missing, or refused. */
export function accountsWithout(accounts: Account[], feature: AccountFeature) {
  const microsoft = accounts.filter((a) => a.kind === "microsoft");
  return {
    needing: microsoft.filter((a) => a.needsPermission.includes(feature)),
    refused: microsoft.filter((a) => a.refusedFeatures.includes(feature)),
  };
}

/** The ids of a FeaturePermissionNotice's headings, for a button they explain. */
export function noticeHeadingIds(id: string) {
  return { needing: `${id}-needs-permission`, refused: `${id}-refused` };
}

/** The features' nouns as a list: "calendar, contacts and tasks". */
export function featureList(features: readonly AccountFeature[]): string {
  const nouns = features.map((f) => FEATURE_COPY[f].noun);
  if (nouns.length <= 1) return nouns.join("");
  return `${nouns.slice(0, -1).join(", ")} and ${nouns[nouns.length - 1]}`;
}

/** A readable organization name from the tenant hint, if it is a domain. */
export function organizationName(tenant: string | undefined): string | null {
  if (!tenant) return null;
  const t = tenant.trim();
  if (["common", "organizations", "consumers"].includes(t.toLowerCase())) return null;
  return t.includes(".") ? t : null;
}

/**
 * What an administrator is asked to approve: signing in, instant updates,
 * or an account's optional features (calendar, contacts…).
 */
export type ApprovalPurpose = "signIn" | "push" | "features";

interface AdminConsentRequest {
  adminConsentUrl: string;
  /** The address the user tried to sign in with, if known. */
  email?: string;
  tenant?: string;
  purpose?: ApprovalPurpose;
  /** The page for IT administrators; only for the published app registration. */
  adminPageUrl?: string | null;
  /** The features to approve, for `features`. */
  features?: AccountFeature[];
}

/** The push service's permission, as IT sees it (Settings → Accounts). */
const PUSH_PERMISSION =
  "Receive notices that my mail or calendar changed, through the Cumulas push service (Sync.Receive, on Cumulas Mail's own API)";

/** Who may approve an app for an organization. */
const ADMIN_ROLES =
  "a Global Administrator, Privileged Role Administrator, Cloud Application Administrator or Application Administrator";

const DELEGATED_ONLY =
  "It only asks for delegated permissions: it acts as the signed-in person and can only reach their own mail, calendar, contacts and tasks, never anyone else's.";

export function adminConsentEmail({
  adminConsentUrl,
  email,
  tenant,
  purpose = "signIn",
  adminPageUrl,
  features = [],
}: AdminConsentRequest) {
  const org = organizationName(tenant);
  const account = email ? ` (${email})` : "";
  const push = purpose === "push";
  const subject = push
    ? "Please approve instant updates for Cumulas Mail"
    : "Please approve Cumulas Mail for Microsoft 365";
  const situation = push
    ? `I use Cumulas Mail, an email and calendar app for Linux, with my Microsoft 365 account${account}. To show new mail as it arrives, it needs one more permission, which an administrator needs to approve.`
    : purpose === "features"
      ? `I use Cumulas Mail, an email and calendar app for Linux, with my Microsoft 365 account${account}. My mail works, but Cumulas Mail needs an administrator's approval to use my ${featureList(features)}.`
      : `I'd like to use Cumulas Mail, an email and calendar app for Linux, with my Microsoft 365 account${account}. When I sign in, Microsoft says an administrator needs to approve the app first.`;
  const lines = [
    "Hello,",
    "",
    situation,
    "",
    adminPageUrl
      ? `Cumulas Mail is published by Cumulas Inc, a Microsoft-verified publisher. ${DELEGATED_ONLY}`
      : DELEGATED_ONLY,
    "",
    `To approve it${org ? ` for ${org}` : ""}, ${ADMIN_ROLES} can either:`,
    `- open this link and sign in: ${adminConsentUrl}`,
    "- or, if Cumulas Mail is already listed in the Microsoft Entra admin center, go to Enterprise apps > Cumulas Mail > Permissions and select Grant admin consent.",
    "",
    "What it asks for:",
    ...MICROSOFT_PERMISSIONS.map((p) => `- ${p.request} (${p.scopes.join(", ")})`),
    ...(push || linkIncludesPush(adminConsentUrl) ? [`- ${PUSH_PERMISSION}`] : []),
    "",
    "The app runs on my computer and keeps mail there; it sends no telemetry. Mail goes only between my computer and Microsoft 365. For instant updates, Microsoft tells the Cumulas push service when my Inbox or calendar changes, but never what's in it. Approving it once makes it available to everyone in the organization, so nobody else will need to ask.",
    "",
    ...(adminPageUrl
      ? [
          `More for administrators, including how to let people approve Cumulas Mail for themselves: ${adminPageUrl}`,
          "",
        ]
      : []),
    "Thank you!",
  ];
  return { subject, body: lines.join("\r\n") };
}

function mailto({ subject, body }: { subject: string; body: string }): string {
  return `mailto:?subject=${encodeURIComponent(subject)}&body=${encodeURIComponent(body)}`;
}

/** A mailto: link with the request to IT ready to send (no recipient yet). */
export function adminConsentMailto(request: AdminConsentRequest): string {
  return mailto(adminConsentEmail(request));
}

interface BlockedSignInRequest {
  reason: SignInBlockedReason;
  aadsts?: number;
  /** The address the user tried to sign in with, if known. */
  email?: string;
  tenant?: string;
  adminPageUrl?: string | null;
  /** appNotSetUp: approving the app may fix it. */
  adminConsentUrl?: string;
}

/** The request to IT when the organization's settings block the sign-in. */
export function blockedSignInEmail({
  reason,
  aadsts,
  email,
  adminPageUrl,
  adminConsentUrl,
}: BlockedSignInRequest) {
  const account = email ? ` (${email})` : "";
  const code = aadsts === undefined ? "" : ` (AADSTS${aadsts})`;
  const why: Record<SignInBlockedReason, string> = {
    notAssigned: `Microsoft says I haven't been assigned to the app${code}. Could you assign me? In the Microsoft Entra admin center: Enterprise apps > Cumulas Mail > Users and groups > Add user/group.`,
    appDisabled: `Microsoft says the app is disabled in our organization${code}. If we may use it, it can be turned back on in the Microsoft Entra admin center: Enterprise apps > Cumulas Mail > Properties > Enabled for users to sign in.`,
    appNotSetUp: adminConsentUrl
      ? `Microsoft says the app isn't set up in our organization${code}. Approving it may fix this; an administrator can open this link and sign in: ${adminConsentUrl}`
      : `Microsoft says the app isn't set up in our organization${code}. Approving it may fix this.`,
    serviceNotEnabled: `Microsoft says our organization hasn't subscribed to or enabled a service the app needs${code}, usually Exchange Online. Could you check our subscription?`,
    securityInfo: `Microsoft says I need to finish setting up my sign-in security info, such as multifactor authentication${code}, and I couldn't do that during sign-in. Could you help me set it up?`,
    conditionalAccess: `Our sign-in rules blocked the sign-in${code}. Could you tell me what's required, or whether Cumulas Mail can be allowed? The Entra sign-in logs show which policy applied.`,
  };
  const lines = [
    "Hello,",
    "",
    `I'd like to use Cumulas Mail, an email and calendar app for Linux, with my Microsoft 365 account${account}. ${why[reason]}`,
    "",
    ...(adminPageUrl ? [`More for administrators: ${adminPageUrl}`, ""] : []),
    "Thank you!",
  ];
  return { subject: `Cumulas Mail sign-in is blocked${code}`, body: lines.join("\r\n") };
}

export function blockedSignInMailto(request: BlockedSignInRequest): string {
  return mailto(blockedSignInEmail(request));
}

interface RefusedFeatureRequest {
  email?: string;
  features: AccountFeature[];
}

/** The request to IT when Microsoft 365 refuses a feature the account has permission for. */
export function refusedFeatureEmail({ email, features }: RefusedFeatureRequest) {
  const account = email ? ` (${email})` : "";
  const list = featureList(features);
  const lines = [
    "Hello,",
    "",
    `I use Cumulas Mail, an email and calendar app for Linux, with my Microsoft 365 account${account}. It has permission to use my ${list}, but Microsoft 365 refuses access (HTTP 403). This often means my mailbox doesn't include that feature, for example because of my license, or that a policy blocks it.`,
    "",
    "Could you check? Thank you!",
  ];
  return { subject: `Cumulas Mail can't open my ${list}`, body: lines.join("\r\n") };
}

export function refusedFeatureMailto(request: RefusedFeatureRequest): string {
  return mailto(refusedFeatureEmail(request));
}
