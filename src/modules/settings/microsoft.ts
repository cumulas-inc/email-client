// What Cumulas Mail asks Microsoft for, in plain language, and the request
// a user can forward to IT when their organisation requires admin approval.
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

/** A readable organisation name from the tenant hint, if it is a domain. */
export function organisationName(tenant: string | undefined): string | null {
  if (!tenant) return null;
  const t = tenant.trim();
  if (["common", "organizations", "consumers"].includes(t.toLowerCase())) return null;
  return t.includes(".") ? t : null;
}

interface AdminConsentRequest {
  adminConsentUrl: string;
  /** The address the user tried to sign in with, if known. */
  email?: string;
  tenant?: string;
}

export function adminConsentEmail({ adminConsentUrl, email, tenant }: AdminConsentRequest) {
  const org = organisationName(tenant);
  const subject = "Please approve Cumulas Mail for Microsoft 365";
  const account = email ? ` (${email})` : "";
  const lines = [
    "Hello,",
    "",
    `I'd like to use Cumulas Mail, an email and calendar app for Linux, with my Microsoft 365 account${account}. When I sign in, Microsoft says an administrator needs to approve the app first.`,
    "",
    `Could you please review and approve it${org ? ` for ${org}` : ""}? An administrator can do this by opening this link and signing in:`,
    "",
    adminConsentUrl,
    "",
    "Cumulas Mail only asks for delegated permissions, so it acts as the signed-in person and can only reach their own data:",
    ...MICROSOFT_PERMISSIONS.map((p) => `- ${p.request} (${p.scopes.join(", ")})`),
    "",
    "The app runs on my computer and keeps mail there. It connects only to Microsoft 365, has no servers of its own and sends no telemetry. Approving it once makes it available to everyone in the organisation, so nobody else will need to ask.",
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
