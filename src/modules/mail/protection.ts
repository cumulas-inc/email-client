// Pure helpers for protected mail: what the composer offers from an
// account's checks and labels, the label policy's rules, and the sender's
// name as protection notices show it.

import type {
  LabelOption,
  Message,
  MessageLabel,
  ProtectAction,
  Protection,
  ProtectionSetup,
} from "../../api/types";
import { formatDate } from "../../lib/datetime";
import type { ComposeDraft } from "./mailStore";

/** A check sends at most this many test messages (besides its control). Mirrors protection.rs. */
export const MAX_CHECK_ITEMS = 19;
/** A passed check is offered for this long. Mirrors protection.rs. */
export const CHECK_VALID_DAYS = 90;
/** A test message that hasn't arrived after this long isn't waited for. Mirrors protection.rs. */
export const CHECK_TIMEOUT_MINUTES = 15;

/** Its content is encrypted: replies and forwards go through Outlook on the web. */
export const isEncrypted = (p: Protection | null) =>
  p?.kind === "rightsManaged" || p?.kind === "smimeEncrypted";

/** Neither an HTML nor a text body with anything in it. */
export function hasBlankBody(message: Pick<Message, "bodyHtml" | "bodyText">): boolean {
  return !message.bodyHtml?.trim() && !message.bodyText?.trim();
}

/**
 * Whether the reading pane explains the message's protection in a notice:
 * it's encrypted or links to Microsoft's portal, or it's signed and its
 * content (or some of it) is inside the signature file. Other signed mail
 * gets a tag instead.
 */
export function hasProtectionNotice(
  message: Pick<Message, "protection" | "bodyHtml" | "bodyText">,
): boolean {
  const protection = message.protection;
  if (!protection) return false;
  if (protection.kind !== "smimeSigned") return true;
  return protection.contentHidden || hasBlankBody(message);
}

/** Microsoft's names for the actions, as the composer and Settings show them. */
export const ACTION_NAMES: Record<ProtectAction, string> = {
  encrypt: "Encrypt",
  doNotForward: "Do Not Forward",
};

/** "Parent › Child", or the label's name. */
export function labelTitle(l: Pick<LabelOption, "name" | "parentName">): string {
  return l.parentName ? `${l.parentName} › ${l.name}` : l.name;
}

/** "Oct 18", with the year when it isn't this one. */
export function checkDate(iso: string): string {
  const date = new Date(iso);
  return formatDate(date, date.getFullYear() === new Date().getFullYear() ? "short" : "shortYear");
}

/**
 * Encrypt and Do Not Forward, when a check found they work (and not too long
 * ago); a pass still counts while a new check waits.
 */
export function offeredActions(setup: ProtectionSetup | undefined): ProtectAction[] {
  if (!setup?.supported) return [];
  const actions: ProtectAction[] = [];
  if (setup.encrypt.passedAt !== null) actions.push("encrypt");
  if (setup.doNotForward.passedAt !== null) actions.push("doNotForward");
  return actions;
}

/** The labels a check found work (and not too long ago), in Microsoft 365's order. */
export function offeredLabels(setup: ProtectionSetup | undefined): LabelOption[] {
  if (!setup?.supported) return [];
  return setup.labels.filter((l) => l.check.passedAt !== null);
}

/**
 * What the draft asks for that isn't offered (any more): a check of it is
 * running again, expired, or didn't pass. Sending it would be refused, so
 * the composer says so and keeps the choice to clear it in view. A label
 * that encrypts makes Encrypt moot.
 */
export function unofferedChoice(
  draft: Pick<ComposeDraft, "protect" | "labelId">,
  setup: ProtectionSetup | undefined,
): { action: ProtectAction | null; label: LabelOption | { id: string } | null } {
  const offered = offeredLabels(setup);
  const label =
    draft.labelId && !offered.some((l) => l.id === draft.labelId)
      ? (setup?.labels.find((l) => l.id === draft.labelId) ?? { id: draft.labelId })
      : null;
  const encrypts = offered.find((l) => l.id === draft.labelId)?.hasProtection ?? false;
  const action =
    draft.protect && !encrypts && !offeredActions(setup).includes(draft.protect)
      ? draft.protect
      : null;
  return { action, label };
}

/** The organization requires a label, and Cumulas Mail can apply at least one. */
export function labelRequired(setup: ProtectionSetup | undefined): boolean {
  return !!setup?.policy.required && offeredLabels(setup).length > 0;
}

/**
 * The label a message starts with: a reply's or forward's original's when
 * it's offered (none when it isn't), else the policy's default when offered.
 */
export function initialLabelId(
  setup: ProtectionSetup,
  original: MessageLabel | null,
): string | null {
  const offered = offeredLabels(setup);
  if (original) return offered.some((l) => l.id === original.id) ? original.id : null;
  const fallback = setup.policy.defaultLabelId;
  return fallback && offered.some((l) => l.id === fallback) ? fallback : null;
}

/**
 * Whether a reply may get `candidate` (null: no label). When lowering a
 * label needs a reason, which Cumulas Mail can't give, one less sensitive
 * than the original's isn't allowed; sensitivities Microsoft 365 doesn't
 * give don't restrict.
 */
export function canChooseLabel(
  setup: ProtectionSetup,
  original: MessageLabel | null,
  candidate: LabelOption | null,
): boolean {
  if (!setup.policy.justifyDowngrade || !original) return true;
  const from = setup.labels.find((l) => l.id === original.id)?.sensitivity;
  if (from === undefined || from === null) return true;
  if (!candidate) return false;
  return candidate.sensitivity === null || candidate.sensitivity >= from;
}

/**
 * What "Check all labels" sends: labels never checked first, then expired
 * ones, then the oldest checks; at most MAX_CHECK_ITEMS.
 */
export function labelsToCheck(setup: ProtectionSetup): LabelOption[] {
  const rank = (l: LabelOption) =>
    l.check.state === "notChecked" ? 0 : l.check.state === "expired" ? 1 : 2;
  const time = (l: LabelOption) => (l.check.at ? Date.parse(l.check.at) : 0);
  return setup.labels
    .filter((l) => l.check.state !== "waiting")
    .map((l, index) => ({ l, index }))
    .sort((a, b) => rank(a.l) - rank(b.l) || time(a.l) - time(b.l) || a.index - b.index)
    .slice(0, MAX_CHECK_ITEMS)
    .map(({ l }) => l);
}

/** U+061C, U+200E, U+200F, U+202A–U+202E and U+2066–U+2069: they can reorder what's around them. */
const BIDI_CONTROLS = /[\u061C\u200E\u200F\u202A-\u202E\u2066-\u2069]/g;

/**
 * A sender's display name as a protection notice shows it, beside the
 * address: bidi controls removed and trimmed; null when blank, when it
 * holds an "@" (it could pass for another address) or is the address.
 */
export function noticeName(name: string | null | undefined, address: string): string | null {
  const clean = (name ?? "").replace(BIDI_CONTROLS, "").trim();
  if (!clean || clean.includes("@")) return null;
  return clean.toLowerCase() === address.trim().toLowerCase() ? null : clean;
}

/**
 * Why Send waits: a label is required and none is chosen; the original's
 * label can't be applied and the organization requires a label, so one has
 * to be chosen; the same, but every label Cumulas Mail can apply is less
 * sensitive than the original's, which needs a reason Cumulas Mail can't
 * give, so only Outlook on the web can send it; or the original's label
 * can't be applied, which the person confirms.
 */
export type SendProblem =
  | { kind: "labelRequired" }
  | { kind: "chooseLabel"; name: string | null }
  | { kind: "outlookOnly"; name: string | null }
  | { kind: "originalLabelDropped"; name: string | null };

/**
 * The original's label (a reply's or forward's original, or an edited
 * draft's own), when the draft might drop it (only for people who use
 * labels here).
 */
export function droppedLabel(
  draft: Pick<ComposeDraft, "labelId" | "originalLabel">,
  setup: ProtectionSetup | undefined,
): MessageLabel | null {
  const original = draft.originalLabel;
  const offered = offeredLabels(setup);
  if (!original || offered.length === 0) return null;
  if (offered.some((l) => l.id === original.id) || draft.labelId === original.id) return null;
  return original;
}

export function sendProblem(
  draft: Pick<ComposeDraft, "labelId" | "labelChosen" | "originalLabel">,
  setup: ProtectionSetup | undefined,
): SendProblem | null {
  const dropped = droppedLabel(draft, setup);
  if (labelRequired(setup)) {
    if (dropped && !draft.labelChosen && setup) {
      const choosable = offeredLabels(setup).some((l) =>
        canChooseLabel(setup, draft.originalLabel, l),
      );
      return choosable
        ? { kind: "chooseLabel", name: dropped.name }
        : { kind: "outlookOnly", name: dropped.name };
    }
    return draft.labelId === null ? { kind: "labelRequired" } : null;
  }
  return dropped ? { kind: "originalLabelDropped", name: dropped.name } : null;
}
