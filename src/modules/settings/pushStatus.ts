// How an account hears about new mail, in words for Settings → Accounts.

import type { AccountKind, PushStatus } from "../../api/types";

export interface PushSummary {
  /** One line: how new mail arrives. */
  label: string;
  /** More to say, if anything. */
  detail: string | null;
  /** "ok": instant; "attention": something the user can fix or should know. */
  tone: "ok" | "neutral" | "attention";
  /** What the user can do about it: sign in again, or get an administrator's approval. */
  action: "signIn" | "approve" | null;
}

/** "Every 30 seconds", "Every minute", "Every 2 minutes". */
export function everyLabel(seconds: number): string {
  if (seconds < 60) return `Every ${seconds} seconds`;
  const minutes = Math.round(seconds / 60);
  return minutes === 1 ? "Every minute" : `Every ${minutes} minutes`;
}

export function describePush(status: PushStatus, kind: AccountKind): PushSummary {
  const { mode, health } = status;
  const microsoft = kind === "microsoft";
  const every = mode.kind === "polling" ? everyLabel(mode.everySecs) : null;
  const summary = (
    label: string,
    tone: PushSummary["tone"],
    detail: string | null = status.detail,
    action: PushSummary["action"] = null,
  ): PushSummary => ({ label, detail, tone, action });

  if (mode.kind === "off") {
    return summary(
      "Instant updates are off",
      "neutral",
      "New mail arrives with the regular check for everything else.",
    );
  }
  switch (health.kind) {
    case "live":
      if (mode.kind === "imapIdle") return summary("Instant (IMAP IDLE)", "ok");
      if (mode.kind === "imapNotify") return summary("Instant (IMAP NOTIFY)", "ok");
      if (mode.kind === "graphRelay") return summary("Instant (Microsoft push)", "ok");
      return summary(
        every ?? "Checked regularly",
        "neutral",
        microsoft ? "The push service is off." : null,
      );
    case "connecting":
      return microsoft && every
        ? summary(`${every} — connecting to the push service`, "neutral")
        : summary("Connecting…", "neutral");
    case "degraded":
      if (every) {
        return summary(
          `${every} — ${microsoft ? "push service unreachable" : "reconnecting"}`,
          "attention",
          health.reason,
        );
      }
      return summary("Reconnecting…", "attention", health.reason);
    case "unsupported":
      return summary(
        `${every ?? "Checked regularly"} — ${
          microsoft ? "instant updates aren't available" : "this server can't push new mail"
        }`,
        "neutral",
      );
    case "authRequired":
      return summary("Paused until the account is signed in again", "attention", null);
    case "consentMissing":
      return summary(
        `${every ?? "Checked regularly"} — instant updates need permission`,
        "attention",
        status.detail,
        status.consentRefused ? (status.adminConsent ? "approve" : null) : "signIn",
      );
  }
}
