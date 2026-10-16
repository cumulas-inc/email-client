// IMAP/SMTP server settings while they are being edited, shared by the
// add-account form and the account "Fix" form: validation and how failed
// checks are explained.

import { errorMessage, hasErrorCode } from "../../api/errors";
import type { ImapSettings, TlsMode } from "../../api/types";
import { isLoopbackHost, type ProviderPreset } from "./presets";
import { validateHost, validatePort } from "./validation";

/** Server settings as typed (ports are strings until saved). */
export interface ServerDraft {
  imapHost: string;
  imapPort: string;
  imapTls: TlsMode;
  smtpHost: string;
  smtpPort: string;
  smtpTls: TlsMode;
  username: string;
}

export type ServerField = keyof ServerDraft;

export const SERVER_FIELDS: ServerField[] = [
  "imapHost",
  "imapPort",
  "imapTls",
  "smtpHost",
  "smtpPort",
  "smtpTls",
  "username",
];

/** Fields of either form that can carry an error. */
export type ImapFieldName = "email" | "password" | ServerField;
export type ImapErrors = Partial<Record<ImapFieldName, string>>;

export interface FormProblem {
  title: string;
  detail?: string;
  technical?: string;
}

/** Usual port for each security mode, used to follow a security change. */
const DEFAULT_PORTS: Record<"imap" | "smtp", Record<TlsMode, string>> = {
  imap: { tls: "993", startTls: "143", none: "143" },
  smtp: { tls: "465", startTls: "587", none: "587" },
};

export function toDraft(s: ImapSettings): ServerDraft {
  return {
    imapHost: s.imapHost,
    imapPort: String(s.imapPort),
    imapTls: s.imapTls,
    smtpHost: s.smtpHost,
    smtpPort: String(s.smtpPort),
    smtpTls: s.smtpTls,
    username: s.username,
  };
}

export function fromDraft(d: ServerDraft): ImapSettings {
  return {
    imapHost: d.imapHost.trim(),
    imapPort: Number(d.imapPort),
    imapTls: d.imapTls,
    smtpHost: d.smtpHost.trim(),
    smtpPort: Number(d.smtpPort),
    smtpTls: d.smtpTls,
    username: d.username.trim(),
  };
}

export function sameSettings(a: ImapSettings, b: ImapSettings): boolean {
  return (
    a.imapHost === b.imapHost &&
    a.imapPort === b.imapPort &&
    a.imapTls === b.imapTls &&
    a.smtpHost === b.smtpHost &&
    a.smtpPort === b.smtpPort &&
    a.smtpTls === b.smtpTls &&
    a.username === b.username
  );
}

/** The change for a new security mode; a standard port follows along. */
export function securityChange(
  draft: ServerDraft,
  kind: "imap" | "smtp",
  tls: TlsMode,
): Partial<ServerDraft> {
  const portField = kind === "imap" ? "imapPort" : "smtpPort";
  const tlsField = kind === "imap" ? "imapTls" : "smtpTls";
  const change: Partial<ServerDraft> = { [tlsField]: tls };
  if (draft[portField] === DEFAULT_PORTS[kind][draft[tlsField]]) {
    change[portField] = DEFAULT_PORTS[kind][tls];
  }
  return change;
}

export function validateServerDraft(d: ServerDraft): Partial<Record<ServerField, string>> {
  const found: Partial<Record<ServerField, string>> = {};
  for (const field of ["imapHost", "smtpHost"] as const) {
    const error = validateHost(d[field]);
    if (error) found[field] = error;
  }
  for (const field of ["imapPort", "smtpPort"] as const) {
    const error = validatePort(d[field]);
    if (error) found[field] = error;
  }
  for (const [tls, host] of [
    ["imapTls", d.imapHost],
    ["smtpTls", d.smtpHost],
  ] as const) {
    if (d[tls] === "none" && host.trim() && !isLoopbackHost(host)) {
      found[tls] =
        "Unencrypted connections are only allowed to this computer. Choose SSL/TLS or STARTTLS.";
    }
  }
  if (!d.username.trim()) found.username = "Enter your user name, usually your email address.";
  return found;
}

interface FailureContext {
  preset: ProviderPreset | null;
  email: string;
  server: ServerDraft;
  /** Where the server fields are, e.g. "under Advanced settings" or "below". */
  serversAre: string;
  /** Title for unexpected errors. */
  fallbackTitle: string;
}

/**
 * Explains a failed server check: field errors where the user can fix them
 * (a rejected password, an invalid address) or a problem for the whole form.
 */
export function describeImapFailure(
  error: unknown,
  { preset, email, server, serversAre, fallbackTitle }: FailureContext,
): { errors: ImapErrors; problem: FormProblem | null } {
  if (hasErrorCode(error, "authRequired")) {
    const usernameNote =
      server.username.trim().toLowerCase() !== email.trim().toLowerCase()
        ? ` Also check the user name ${serversAre}.`
        : "";
    const password = preset?.appPassword
      ? `${preset.name} didn't accept this password. Make sure you're using an ${preset.appPassword.term}, not your usual password.`
      : preset?.bridge
        ? "Proton Mail Bridge didn't accept this password. Use the password shown in Bridge, not your Proton password."
        : "The server didn't accept this email address and password. Check them and try again.";
    return { errors: { password: password + usernameNote }, problem: null };
  }
  if (hasErrorCode(error, "network")) {
    return {
      errors: {},
      problem: {
        title: preset?.bridge
          ? "Couldn't connect to Proton Mail Bridge"
          : "Couldn't connect to the mail server",
        detail: preset?.bridge
          ? "Make sure Bridge is running and signed in, and that the ports match the ones Bridge shows."
          : `Check the server names and ports ${serversAre}, and that you're online. Your provider's help pages list the right settings.`,
        technical: error.message,
      },
    };
  }
  if (hasErrorCode(error, "invalidInput")) {
    const message = error.message;
    if (/already been added|email address/i.test(message)) {
      return { errors: { email: message }, problem: null };
    }
    if (/unencrypted/i.test(message)) {
      return {
        errors: server.imapTls === "none" ? { imapTls: message } : { smtpTls: message },
        problem: null,
      };
    }
    if (/server/i.test(message)) return { errors: { imapHost: message }, problem: null };
    return { errors: {}, problem: { title: message } };
  }
  return {
    errors: {},
    problem: {
      title: fallbackTitle,
      detail:
        "The check didn't finish. Try again, and if it keeps happening, the details below can help your provider.",
      technical: errorMessage(error),
    },
  };
}
