// Server settings for well-known mail providers, picked from the email
// address's domain, plus a sensible guess for everyone else.

import type { ImapSettings, TlsMode } from "../../api/types";

export interface ServerPreset {
  host: string;
  port: number;
  tls: TlsMode;
}

export type PresetId = "gmail" | "outlook" | "icloud" | "yahoo" | "fastmail" | "proton";

export interface ProviderPreset {
  id: PresetId;
  /** Provider name as people know it, e.g. "Gmail". */
  name: string;
  domains: string[];
  imap: ServerPreset;
  smtp: ServerPreset;
  /** The provider only accepts a separately created password from mail apps. */
  appPassword?: {
    /** What the provider calls it. */
    term: string;
    /** One line on where to create one. */
    howTo: string;
    url?: string;
  };
  /** Microsoft sign-in works better than IMAP (Outlook.com). */
  preferMicrosoft?: boolean;
  /** Needs Proton Mail Bridge running on this computer. */
  bridge?: boolean;
}

export const PRESETS: ProviderPreset[] = [
  {
    id: "gmail",
    name: "Gmail",
    domains: ["gmail.com", "googlemail.com"],
    imap: { host: "imap.gmail.com", port: 993, tls: "tls" },
    smtp: { host: "smtp.gmail.com", port: 465, tls: "tls" },
    appPassword: {
      term: "app password",
      howTo: "Turn on 2-Step Verification, then create one in your Google Account.",
      url: "https://myaccount.google.com/apppasswords",
    },
  },
  {
    id: "outlook",
    name: "Outlook.com",
    domains: ["outlook.com", "hotmail.com", "live.com", "msn.com"],
    imap: { host: "outlook.office365.com", port: 993, tls: "tls" },
    smtp: { host: "smtp-mail.outlook.com", port: 587, tls: "startTls" },
    preferMicrosoft: true,
  },
  {
    id: "icloud",
    name: "iCloud Mail",
    domains: ["icloud.com", "me.com", "mac.com"],
    imap: { host: "imap.mail.me.com", port: 993, tls: "tls" },
    smtp: { host: "smtp.mail.me.com", port: 587, tls: "startTls" },
    appPassword: {
      term: "app-specific password",
      howTo: "Create one in your Apple Account under Sign-In and Security.",
      url: "https://account.apple.com",
    },
  },
  {
    id: "yahoo",
    name: "Yahoo Mail",
    domains: ["yahoo.com", "ymail.com"],
    imap: { host: "imap.mail.yahoo.com", port: 993, tls: "tls" },
    smtp: { host: "smtp.mail.yahoo.com", port: 465, tls: "tls" },
    appPassword: {
      term: "app password",
      howTo: "Create one in your Yahoo account under Account security.",
      url: "https://login.yahoo.com/account/security",
    },
  },
  {
    id: "fastmail",
    name: "Fastmail",
    domains: ["fastmail.com", "fastmail.fm"],
    imap: { host: "imap.fastmail.com", port: 993, tls: "tls" },
    smtp: { host: "smtp.fastmail.com", port: 465, tls: "tls" },
    appPassword: {
      term: "app password",
      howTo: "Create one in Fastmail under Settings → Privacy & Security.",
    },
  },
  {
    id: "proton",
    name: "Proton Mail",
    domains: ["proton.me", "protonmail.com", "protonmail.ch", "pm.me"],
    imap: { host: "127.0.0.1", port: 1143, tls: "startTls" },
    smtp: { host: "127.0.0.1", port: 1025, tls: "startTls" },
    bridge: true,
  },
];

export const PROTON_BRIDGE_URL = "https://proton.me/mail/bridge";

/** The lower-cased domain of an email address, or null if it has none yet. */
export function emailDomain(email: string): string | null {
  const trimmed = email.trim();
  const at = trimmed.lastIndexOf("@");
  if (at < 1) return null;
  const domain = trimmed.slice(at + 1).toLowerCase();
  return /^[\p{L}\p{N}-]+(\.[\p{L}\p{N}-]+)*\.[\p{L}]{2,}$/u.test(domain) ? domain : null;
}

export function findPreset(email: string): ProviderPreset | null {
  const domain = emailDomain(email);
  if (!domain) return null;
  return PRESETS.find((p) => p.domains.includes(domain)) ?? null;
}

/** Server settings to start from for an address. */
export function suggestedSettings(email: string): ImapSettings {
  const username = email.trim();
  const preset = findPreset(email);
  if (preset) {
    return {
      imapHost: preset.imap.host,
      imapPort: preset.imap.port,
      imapTls: preset.imap.tls,
      smtpHost: preset.smtp.host,
      smtpPort: preset.smtp.port,
      smtpTls: preset.smtp.tls,
      username,
    };
  }
  const domain = emailDomain(email);
  return {
    imapHost: domain ? `imap.${domain}` : "",
    imapPort: 993,
    imapTls: "tls",
    smtpHost: domain ? `smtp.${domain}` : "",
    smtpPort: 587,
    smtpTls: "startTls",
    username,
  };
}

/** Hosts on this computer, where unencrypted connections are allowed. */
export function isLoopbackHost(host: string): boolean {
  const h = host
    .trim()
    .toLowerCase()
    .replace(/^\[|\]$/g, "");
  return h === "localhost" || h === "::1" || /^127(\.\d{1,3}){3}$/.test(h);
}

export const TLS_LABELS: Record<TlsMode, string> = {
  tls: "SSL/TLS",
  startTls: "STARTTLS",
  none: "None",
};
