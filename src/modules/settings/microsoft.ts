// What Cumulas Mail asks Microsoft for, in plain language, and the request
// a user can forward to IT when their organization requires admin approval.
// Keep in step with SCOPES in crates/core/src/auth/microsoft.rs.

export interface Permission {
  /** Graph permission names, as IT will see them. */
  scopes: string[];
  /** "Read and write your mail" — shown in the app. */
  label: string;
  /** "Read and write my mail" — used in the request to IT. */
  request: string;
}

export const MICROSOFT_PERMISSIONS: Permission[] = [
  {
    scopes: ["Mail.ReadWrite"],
    label: "Read and write your mail",
    request: "Read and write my mail",
  },
  { scopes: ["Mail.Send"], label: "Send mail as you", request: "Send mail as me" },
  {
    scopes: ["MailboxSettings.Read"],
    label: "Read your mailbox settings, such as your time zone",
    request: "Read my mailbox settings, such as my time zone",
  },
  {
    scopes: ["Calendars.ReadWrite"],
    label: "Read and write your calendars",
    request: "Read and write my calendars",
  },
  {
    scopes: ["Contacts.ReadWrite"],
    label: "Read and write your contacts",
    request: "Read and write my contacts",
  },
  {
    scopes: ["Tasks.ReadWrite"],
    label: "Read and write your tasks",
    request: "Read and write my tasks",
  },
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
];

/** A readable organization name from the tenant hint, if it is a domain. */
export function organizationName(tenant: string | undefined): string | null {
  if (!tenant) return null;
  const t = tenant.trim();
  if (["common", "organizations", "consumers"].includes(t.toLowerCase())) return null;
  return t.includes(".") ? t : null;
}

/** What an administrator is asked to approve: signing in, or instant updates. */
export type ApprovalPurpose = "signIn" | "push";

interface AdminConsentRequest {
  adminConsentUrl: string;
  /** The address the user tried to sign in with, if known. */
  email?: string;
  tenant?: string;
  purpose?: ApprovalPurpose;
}

/** The push service's permission, as IT sees it (Settings → Accounts). */
const PUSH_PERMISSION =
  "Receive new-mail alerts through the Cumulas push service (Sync.Receive, on Cumulas Mail's own API)";

export function adminConsentEmail({
  adminConsentUrl,
  email,
  tenant,
  purpose = "signIn",
}: AdminConsentRequest) {
  const org = organizationName(tenant);
  const account = email ? ` (${email})` : "";
  const push = purpose === "push";
  const subject = push
    ? "Please approve instant updates for Cumulas Mail"
    : "Please approve Cumulas Mail for Microsoft 365";
  const situation = push
    ? `I use Cumulas Mail, an email and calendar app for Linux, with my Microsoft 365 account${account}. To show new mail as it arrives, it needs one more permission, which an administrator needs to approve.`
    : `I'd like to use Cumulas Mail, an email and calendar app for Linux, with my Microsoft 365 account${account}. When I sign in, Microsoft says an administrator needs to approve the app first.`;
  const lines = [
    "Hello,",
    "",
    situation,
    "",
    `Could you please review and approve it${org ? ` for ${org}` : ""}? An administrator can do this by opening this link and signing in:`,
    "",
    adminConsentUrl,
    "",
    "Cumulas Mail only asks for delegated permissions, so it acts as the signed-in person and can only reach their own data:",
    ...MICROSOFT_PERMISSIONS.map((p) => `- ${p.request} (${p.scopes.join(", ")})`),
    ...(push ? [`- ${PUSH_PERMISSION}`] : []),
    "",
    "The app runs on my computer and keeps mail there; it sends no telemetry. Mail goes only between my computer and Microsoft 365. For instant updates, Microsoft tells the Cumulas push service when my mailbox changes, but never what's in it. Approving it once makes it available to everyone in the organization, so nobody else will need to ask.",
    "",
    "Thank you!",
  ];
  return { subject, body: lines.join("\r\n") };
}

/** A mailto: link with the request to IT ready to send (no recipient yet). */
export function adminConsentMailto(request: AdminConsentRequest): string {
  const { subject, body } = adminConsentEmail(request);
  return `mailto:?subject=${encodeURIComponent(subject)}&body=${encodeURIComponent(body)}`;
}
