// "Copy details for IT" for the demo: what a work account's last checks
// found, in the shape of the core's text (protection::check_details), with
// the evidence the demo's test messages would have carried.

import type { CheckItem, CheckResult } from "../types";
import { CHECK_TIMEOUT_MINUTES, CHECK_VALID_DAYS } from "../../modules/mail/protection";
import { CHECK_NOTES, OTHER_LABEL_UNNAMED, psQuote } from "../../modules/settings/orgsetup/copy";

/** Where the guide says what the check details mean. */
const CHECK_GUIDE =
  "https://github.com/cumulas-inc/email-client/blob/main/docs/microsoft-365.md#troubleshooting-the-check";
const CHECK_SUBJECT = "Cumulas Mail setup check";
/** How long after it was sent the demo finds a test message. */
const FOUND_AFTER_SECS = 20;

/** One option's check, as Settings shows it now. */
export interface DetailsItem {
  /** "Encrypt", "Do Not Forward", or "Label Confidential (<id>)". */
  title: string;
  item: CheckItem;
  result: CheckResult;
  /** What the last decided check kept. */
  record: Omit<CheckResult, "passedAt">;
}

export interface DetailsInput {
  version: string;
  email: string;
  tenant: string;
  /** The demo's mail goes through the Cumulas Signatures relay, and comes back as outside mail. */
  relay: boolean;
  items: DetailsItem[];
  /** The account's labels, for their titles. */
  labels: { id: string; title: string; encrypts: boolean }[];
  rulesChangedAt: string | null;
  policiesChangedAt: string | null;
}

/** "2026-10-19 14:03:00 UTC". */
function utc(at: number): string {
  return `${new Date(at).toISOString().slice(0, 19).replace("T", " ")} UTC`;
}

/** "2026-10-19 14:03 UTC". */
function utcDay(at: string): string {
  return `${at.slice(0, 16).replace("T", " ")} UTC`;
}

/** "10 minutes", "3 hours". */
function span(ms: number): string {
  const minutes = Math.max(0, Math.floor(ms / 60_000));
  const plural = (n: number, unit: string) => (n === 1 ? `1 ${unit}` : `${n} ${unit}s`);
  if (minutes === 0) return "less than a minute";
  if (minutes < 60) return plural(minutes, "minute");
  if (minutes < 2880) return plural(Math.floor(minutes / 60), "hour");
  return plural(Math.floor(minutes / 1440), "day");
}

/** A check's row in Settings, with times in UTC. */
function rowWords(result: CheckResult, label: boolean): string {
  const on = result.at ? utcDay(result.at) : null;
  const dated = (text: string) => (on ? `${text} · ${on}` : text);
  const checked = (text: string) => (on ? `${text} · checked ${on}` : text);
  switch (result.state) {
    case "notChecked":
      return "Not checked yet";
    case "waiting":
      return "Checking…";
    case "applied":
      return checked(label ? "Arrived labeled" : "Arrived encrypted");
    case "expired":
      return result.note === "relayChanged"
        ? "Check again to use it"
        : `Checked over ${CHECK_VALID_DAYS} days ago${on ? ` (${on})` : ""}. Check again to use it.`;
    case "notApplied":
      if (result.note === "headerNotKept") return dated("Couldn't send the test");
      if (result.note === "otherLabel") return checked("Arrived with another label");
      return checked("Arrived without it");
    case "unknown":
      return dated(result.note === "notArrived" ? "Couldn't check" : "Couldn't tell");
  }
}

/** The note under a check's row, without the pointer to these details. */
function noteWords(input: DetailsInput, detail: DetailsItem): string | null {
  const { result, item } = detail;
  const action = item.kind === "action" ? item.action : null;
  const ago =
    result.changedAt && result.at
      ? span(Date.parse(result.at) - Date.parse(result.changedAt))
      : null;
  let note: string;
  switch (result.state) {
    case "waiting":
      return result.passedAt
        ? `Until this check is done, the one from ${utcDay(result.passedAt)} still applies.`
        : null;
    case "applied":
      if (result.note !== "notArrived") return null;
      note = `The latest test message didn't arrive in your Inbox within ${CHECK_TIMEOUT_MINUTES} minutes, so this earlier check still applies.`;
      break;
    case "expired":
      return result.note === "relayChanged" ? CHECK_NOTES.relayChanged : null;
    case "notApplied":
      switch (result.note) {
        case "headerNotKept":
          note =
            "Microsoft 365 didn't keep the request on the test message, so Cumulas Mail can't ask for this.";
          break;
        case "requestIgnored":
          note = action
            ? CHECK_NOTES.requestIgnoredAction(action)
            : CHECK_NOTES.requestIgnoredLabel;
          break;
        case "requestRemoved":
          note = CHECK_NOTES.requestRemoved(action ? "action" : "label");
          break;
        case "relayNotInternal":
          note = action ? CHECK_NOTES.relayNotInternalAction : CHECK_NOTES.relayNotInternalLabel;
          break;
        case "tooSoon":
          return CHECK_NOTES.tooSoon(ago ?? "shortly");
        case "otherLabel": {
          const other = input.labels.find((l) => l.id === result.otherLabelId);
          note = CHECK_NOTES.otherLabel(
            other?.title ?? OTHER_LABEL_UNNAMED,
            result.otherLabelOnControl,
          );
          break;
        }
        default:
          note = CHECK_NOTES.notApplied;
      }
      break;
    case "unknown":
      switch (result.note) {
        case "otherRule":
          note =
            "Another rule in your organization already protects or labels this mail, so Cumulas Mail can't tell whether its own rule works.";
          break;
        case "headerKept":
          note =
            "The test message arrived encrypted, but your organization's rule didn't remove the x-cumulas-protect header, so another rule may have encrypted it. Ask your IT team to check the rule.";
          break;
        case "notArrived":
          note = `The test message didn't arrive in your Inbox within ${CHECK_TIMEOUT_MINUTES} minutes.`;
          break;
        case "otherProtection":
          note = CHECK_NOTES.otherProtection;
          break;
        case "labelUnreadable":
          note = CHECK_NOTES.labelUnreadable;
          break;
        default:
          return null;
      }
      break;
    default:
      return null;
  }
  return ago && (result.state === "notApplied" || result.state === "unknown")
    ? note + CHECK_NOTES.changedBefore(ago)
    : note;
}

/** A label of the organization: "Title (id)" when it's listed, else its id. */
function labelWords(input: DetailsInput, id: string): string {
  const label = input.labels.find((l) => l.id === id);
  return label ? `${label.title} (${id})` : id;
}

/**
 * "Copy details for IT": the control and each check's test message as the
 * demo would have found them, and the commands IT runs to look them up.
 */
export function checkDetails(input: DetailsInput): string {
  const domain = input.email.split("@")[1] ?? "demo.example";
  const authAs = input.relay ? "Anonymous" : "Internal";
  const direction = input.relay ? "Incoming" : "Originating";
  const decided = input.items.filter((d) => d.record.at);
  const last = Math.max(0, ...decided.map((d) => Date.parse(d.record.at!)));
  const messageIds: string[] = [];
  const out = [
    `Cumulas Mail ${input.version}: setup check details for ${input.email}`,
    `Organization (tenant): ${input.tenant}`,
    "These details include no message text. They include the test messages' Message-IDs and what Microsoft 365 said about how it handled them.",
    "",
  ];
  if (last) {
    // The control arrives with the organization's default label when a
    // label's check couldn't tell for it.
    const defaultLabel = decided.find(
      (d) => d.record.note === "otherRule" && d.item.kind === "label",
    );
    const id = `<check-control.${last}@${domain}>`;
    messageIds.push(id);
    out.push(
      `Control message (no request), sent ${utc(last - FOUND_AFTER_SECS * 1000)}, found within ${FOUND_AFTER_SECS} s`,
      `  Message-ID: ${id}`,
      `  Went through the Cumulas Signatures relay (X-Signature-Stamped): ${input.relay ? "yes" : "no"}`,
      `  Counted as from inside your organization (X-MS-Exchange-Organization-AuthAs): ${authAs}`,
      `  Direction (X-MS-Exchange-Organization-MessageDirectionality): ${direction}`,
      `  Arrived: nothing; label of your organization: ${
        defaultLabel?.item.kind === "label" ? labelWords(input, defaultLabel.item.labelId) : "none"
      }`,
      "  Headers read from: the message's transport headers",
      `Path: ${input.relay ? "relay present, return not internal" : "no relay"}`,
    );
  } else {
    out.push("Control message (no request): not found", "Path: not known");
  }
  out.push("");

  for (const detail of input.items) {
    const { item, result, record } = detail;
    out.push(`${detail.title}: ${rowWords(result, item.kind === "label")}`);
    const note = noteWords(input, detail);
    if (note) out.push(`  Note: ${note}`);
    // A check on its way: what the one before it kept isn't its.
    if (result.state === "waiting" || !record.at) continue;
    const at = Date.parse(record.at);
    const sent = utc(at - FOUND_AFTER_SECS * 1000);
    out.push(
      item.kind === "action"
        ? `  Sent: x-cumulas-protect: ${item.action === "encrypt" ? "encrypt" : "do-not-forward"}, ${sent}`
        : `  Sent: x-cumulas-label: ${item.labelId}, ${sent}`,
    );
    const key = item.kind === "action" ? item.action : item.labelId;
    const id = `<check-${key}.${at}@${domain}>`;
    messageIds.push(id);
    const applied = record.state === "applied";
    const label = item.kind === "label" ? input.labels.find((l) => l.id === item.labelId) : null;
    // Encrypt and Do Not Forward, or a label that encrypts, arrive rights-managed.
    const encrypted = applied && (item.kind === "action" || !!label?.encrypts);
    // Rules remove the request; mail coming back from the relay as outside
    // mail loses a label request to the strip rule.
    const kept =
      record.state === "notApplied" &&
      !(record.note === "relayNotInternal" && item.kind === "label") &&
      record.note !== "requestRemoved";
    const seen =
      item.kind === "label" && (applied || record.note === "otherRule")
        ? `${labelWords(input, item.labelId)}, enabled`
        : "none";
    out.push(
      `  Message-ID: ${id}`,
      `  Request still on the message when it arrived: ${kept ? "yes" : "no"}`,
      `  Relay: ${input.relay ? "yes" : "no"} · AuthAs: ${authAs} · Direction: ${direction}`,
      encrypted
        ? "  Arrived: Microsoft Purview encryption · Content-Class: rpmsg.message · Message class: IPM.Note.rpmsg.Microsoft.Voting.Restricted · message.rpmsg: yes"
        : "  Arrived: nothing · Content-Class: not said · Message class: IPM.Note · message.rpmsg: no",
      `  Labels on it: ${seen} · other organizations' labels: 0`,
    );
  }

  out.push(
    "",
    `Setup changed on this computer: rules ${
      input.rulesChangedAt ? utc(Date.parse(input.rulesChangedAt)) : "never"
    }, auto-labeling policies ${
      input.policiesChangedAt ? utc(Date.parse(input.policiesChangedAt)) : "never"
    }`,
    "",
    "For your IT team, in Exchange Online PowerShell:",
    `  Get-MessageTraceV2 -SenderAddress ${psQuote(input.email)} -RecipientAddress ${psQuote(input.email)} -Subject ${psQuote(CHECK_SUBJECT)} -SubjectFilterType StartsWith -StartDate (Get-Date).AddDays(-2) -EndDate (Get-Date) | ForEach-Object { Get-MessageTraceDetailV2 -MessageTraceId $_.MessageTraceId -RecipientAddress ${psQuote(input.email)} } | Format-List Date,Event,Detail`,
    ...messageIds.map(
      (id) =>
        `  Get-MailDetailTransportRuleReport -MessageId ${psQuote(id)} -EventType TransportRuleHits,TransportRuleActionHits -StartDate (Get-Date).AddDays(-2) -EndDate (Get-Date) | Format-List`,
    ),
    `Guide: ${CHECK_GUIDE}`,
    "",
  );
  return out.join("\n");
}
