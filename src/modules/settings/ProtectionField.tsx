import clsx from "clsx";
import {
  Building2,
  ClipboardList,
  ExternalLink,
  LoaderCircle,
  Lock,
  Mail,
  RefreshCw,
} from "lucide-react";
import { type ReactNode, useId, useState } from "react";

import { useBackend } from "../../api/context";
import { errorMessage } from "../../api/errors";
import { useProtectionSetup } from "../../api/queries";
import type {
  Account,
  CheckItem,
  CheckResult,
  LabelOption,
  ProtectAction,
  ProtectionSetup,
} from "../../api/types";
import { Spinner } from "../../components/Spinner";
import { toast } from "../../components/toast";
import { copyLater } from "../../lib/clipboard";
import {
  ACTION_NAMES,
  CHECK_TIMEOUT_MINUTES,
  CHECK_VALID_DAYS,
  MAX_CHECK_ITEMS,
  checkDate,
  labelTitle,
  labelsToCheck,
} from "../mail/protection";
import { openTurnOn } from "./addAccountStore";
import { ApprovalLink } from "./AdminConsent";
import { REFUSED_HINT } from "./FeaturePermission";
import { noteFeatureTurnedOn } from "./featureToasts";
import { PROTECTION_GUIDE_URL, refusedFeatureMailto } from "./microsoft";
import {
  CHECK_NOTES,
  CHECK_ROWS,
  CHECK_WAIT,
  COPIED_DETAILS_FOR_IT,
  COPY_DETAILS_FOR_IT,
  ENTRY_BUTTON,
  IT_CAN_FIX,
  OTHER_LABEL_UNNAMED,
} from "./orgsetup/copy";
import { OrgSetupDialog } from "./orgsetup/OrgSetupDialog";
import {
  useAdminConsentLink,
  useCheckProtection,
  useProtectionCheckPolling,
  useRefreshLabels,
  useRetryMicrosoftFeatures,
} from "./queries";

const DNF_NOTE =
  "Cumulas Mail can't tell Do Not Forward from Encrypt in a test message. Open the test message in Outlook on the web: it should say that it can't be forwarded.";

type Tone = "neutral" | "busy" | "ok" | "attention";

/** How long before the check the organization's setup changed: "10 minutes", "3 hours". */
function howLongBefore(changedAt: string, at: string | null): string {
  const ms = Math.max(0, (at ? Date.parse(at) : Date.now()) - Date.parse(changedAt));
  const minutes = Math.round(ms / 60_000);
  if (minutes < 1) return "less than a minute";
  if (minutes < 60) return minutes === 1 ? "1 minute" : `${minutes} minutes`;
  const hours = Math.round(minutes / 60);
  if (hours < 48) return hours === 1 ? "1 hour" : `${hours} hours`;
  return `${Math.round(hours / 24)} days`;
}

/**
 * The note under a check that didn't pass, or couldn't tell (4.7): what the
 * check found, a recent setup change, and that IT can fix it.
 */
function checkNote(
  result: CheckResult,
  item: CheckItem,
  labels: readonly LabelOption[],
): string | null {
  const ago = result.changedAt ? howLongBefore(result.changedAt, result.at) : null;
  const changed = ago ? CHECK_NOTES.changedBefore(ago) : "";
  const kind = item.kind;
  switch (result.note) {
    case "requestIgnored":
      return (
        (item.kind === "action"
          ? CHECK_NOTES.requestIgnoredAction(item.action)
          : CHECK_NOTES.requestIgnoredLabel) +
        changed +
        IT_CAN_FIX
      );
    case "requestRemoved":
      return CHECK_NOTES.requestRemoved(kind) + changed + IT_CAN_FIX;
    case "relayNotInternal":
      return (
        (kind === "action"
          ? CHECK_NOTES.relayNotInternalAction
          : CHECK_NOTES.relayNotInternalLabel) +
        changed +
        IT_CAN_FIX
      );
    case "tooSoon":
      return CHECK_NOTES.tooSoon(ago ?? "shortly");
    case "otherLabel": {
      const other = labels.find((l) => l.id === result.otherLabelId);
      const title = other ? labelTitle(other) : OTHER_LABEL_UNNAMED;
      return CHECK_NOTES.otherLabel(title, result.otherLabelOnControl) + changed;
    }
    case "otherProtection":
      return CHECK_NOTES.otherProtection + changed;
    case "labelUnreadable":
      return CHECK_NOTES.labelUnreadable + changed;
    default:
      // Arrived without it, and nothing more to say about why.
      return result.state === "notApplied" ? CHECK_NOTES.notApplied + changed + IT_CAN_FIX : null;
  }
}

/** What a row says about its check, and the note under it. */
function describeCheck(
  result: CheckResult,
  item: CheckItem,
  labels: readonly LabelOption[],
): { text: string; note: string | null; tone: Tone } {
  const kind = item.kind;
  const on = result.at ? checkDate(result.at) : null;
  const dated = (text: string) => (on ? `${text} · ${on}` : text);
  const checked = (text: string) => (on ? `${text} · checked ${on}` : text);
  switch (result.state) {
    case "notChecked":
      return { text: "Not checked yet", note: null, tone: "neutral" };
    case "waiting":
      return {
        text: "Checking…",
        // The pass before it still counts until this one is decided.
        note: result.passedAt
          ? `Until this check is done, the one from ${checkDate(result.passedAt)} still applies.`
          : null,
        tone: "busy",
      };
    case "applied":
      return {
        text: checked(kind === "action" ? "Arrived encrypted" : "Arrived labeled"),
        note:
          result.note === "notArrived"
            ? `The latest test message didn't arrive in your Inbox within ${CHECK_TIMEOUT_MINUTES} minutes, so this earlier check still applies.`
            : null,
        tone: "ok",
      };
    case "expired":
      // The relay came into the path after the pass: only the 90-day case is about its age.
      return result.note === "relayChanged"
        ? { text: CHECK_ROWS.relayChanged, note: CHECK_NOTES.relayChanged, tone: "attention" }
        : {
            text: `Checked over ${CHECK_VALID_DAYS} days ago${on ? ` (${on})` : ""}. Check again to use it.`,
            note: null,
            tone: "attention",
          };
    case "notApplied":
      return result.note === "headerNotKept"
        ? {
            text: dated("Couldn't send the test"),
            note: "Microsoft 365 didn't keep the request on the test message, so Cumulas Mail can't ask for this.",
            tone: "attention",
          }
        : {
            text: checked(
              result.note === "otherLabel" ? CHECK_ROWS.otherLabel : "Arrived without it",
            ),
            note: checkNote(result, item, labels),
            tone: "attention",
          };
    case "unknown":
      switch (result.note) {
        case "otherRule":
          return {
            text: dated("Couldn't tell"),
            note: "Another rule in your organization already protects or labels this mail, so Cumulas Mail can't tell whether its own rule works.",
            tone: "attention",
          };
        case "headerKept":
          return {
            text: dated("Couldn't tell"),
            note: "The test message arrived encrypted, but your organization's rule didn't remove the x-cumulas-protect header, so another rule may have encrypted it. Ask your IT team to check the rule.",
            tone: "attention",
          };
        case "notArrived":
          return {
            text: dated("Couldn't check"),
            note: `The test message didn't arrive in your Inbox within ${CHECK_TIMEOUT_MINUTES} minutes.`,
            tone: "attention",
          };
        default:
          return {
            text: dated("Couldn't tell"),
            note: checkNote(result, item, labels),
            tone: "attention",
          };
      }
  }
}

interface CheckRowProps {
  /** The row's name, as shown. */
  name: ReactNode;
  /** The item in the buttons' accessible names: "Encrypt", "the label Confidential". */
  spoken: string;
  item: CheckItem;
  result: CheckResult;
  /** The account's labels, to name one that arrived instead. */
  labels: readonly LabelOption[];
  email: string;
  /** Checking can't start now: a check is waiting, or being sent. */
  busy: boolean;
  /** The button that started the check that's running, if one here did. */
  pressed: string | null;
  /** A note that always goes with the row. */
  extra?: string;
  title?: string;
  onCheck: (items: CheckItem[], button: string) => void;
  onOpen: (item: CheckItem) => void;
}

/**
 * A button that starts a check. While one runs it stays focusable
 * (aria-disabled, not disabled), so the keyboard keeps its place in a long
 * dialog; the one that started it shows that it's busy.
 */
function CheckButton({
  id,
  busy,
  pressed,
  unavailable = false,
  className,
  label,
  onCheck,
  children,
}: {
  id: string;
  busy: boolean;
  pressed: string | null;
  /** Nothing to check (not a wait). */
  unavailable?: boolean;
  className?: string;
  label: string;
  onCheck: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      className={clsx("btn btn--sm", className, busy && pressed === id && "btn--busy")}
      aria-label={label}
      aria-disabled={busy || undefined}
      disabled={unavailable}
      onClick={() => {
        if (!busy) onCheck();
      }}
    >
      {children}
    </button>
  );
}

const checkButtonId = (item: CheckItem) =>
  item.kind === "action" ? `check:${item.action}` : `check:${item.labelId}`;

function CheckRow({
  name,
  spoken,
  item,
  result,
  labels,
  email,
  busy,
  pressed,
  extra,
  title,
  onCheck,
  onOpen,
}: CheckRowProps) {
  const { text, note, tone } = describeCheck(result, item, labels);
  const again = result.state !== "notChecked";
  return (
    <li className="protection-row">
      <span className="protection-row__text">
        <span className="protection-row__name" title={title}>
          {name}
        </span>
        <span className={clsx("protection-status", `protection-status--${tone}`)} role="status">
          {tone === "busy" ? (
            <LoaderCircle size={12} className="spin" aria-hidden="true" />
          ) : (
            <span className="protection-status__dot" aria-hidden="true" />
          )}
          {text}
        </span>
        {note && <span className="protection-row__note">{note}</span>}
        {extra && <span className="protection-row__note">{extra}</span>}
      </span>
      <span className="protection-row__actions">
        {result.testMessage && (
          <button
            type="button"
            className="btn btn--subtle btn--sm"
            aria-label={`Open the test message for ${spoken} in Outlook on the web`}
            onClick={() => onOpen(item)}
          >
            Open test message
          </button>
        )}
        <CheckButton
          id={checkButtonId(item)}
          busy={busy}
          pressed={pressed}
          label={again ? `Check ${spoken} again for ${email}` : `Check ${spoken} for ${email}`}
          onCheck={() => onCheck([item], checkButtonId(item))}
        >
          {again ? "Check again" : "Check"}
        </CheckButton>
      </span>
    </li>
  );
}

const actionItem = (action: ProtectAction): CheckItem => ({ kind: "action", action });
const labelItem = (label: LabelOption): CheckItem => ({ kind: "label", labelId: label.id });

interface LabelsGroupProps {
  account: Account;
  setup: ProtectionSetup;
  busy: boolean;
  pressed: string | null;
  returnFocusId: string;
  onCheck: (items: CheckItem[], button: string) => void;
  onOpen: (item: CheckItem) => void;
}

/** The labels part: its permission, the list Microsoft 365 gave, and their checks. */
function LabelsGroup({
  account,
  setup,
  busy,
  pressed,
  returnFocusId,
  onCheck,
  onOpen,
}: LabelsGroupProps) {
  const id = useId();
  const backend = useBackend();
  const refresh = useRefreshLabels();
  const retry = useRetryMicrosoftFeatures();
  const [asking, setAsking] = useState(false);
  const link = useAdminConsentLink(account.id, asking);
  const email = account.email;
  const headingId = `${id}-labels`;
  const askId = `${id}-ask`;

  const heading = (
    <h3 id={headingId} className="protection-group__title">
      Sensitivity labels
    </h3>
  );

  const refreshNow = () => refresh.mutate(account.id, { onError: (e: unknown) => toast.error(e) });

  if (account.needsPermission.includes("sensitivityLabels")) {
    return (
      <section className="protection-group" aria-labelledby={headingId}>
        {heading}
        <p className="account-field__hint">
          Cumulas Mail needs permission to read the sensitivity labels you can use.
        </p>
        <div className="protection-group__buttons">
          <button
            type="button"
            className="btn btn--sm"
            aria-label={`Turn on sensitivity labels for ${email}`}
            onClick={() => openTurnOn(email, "sensitivityLabels", returnFocusId)}
          >
            Turn on
          </button>
          <button
            type="button"
            className="btn btn--sm"
            aria-label={`Ask IT about sensitivity labels for ${email}`}
            aria-expanded={asking}
            aria-controls={askId}
            onClick={() => setAsking((open) => !open)}
          >
            Ask IT
          </button>
        </div>
        <div id={askId} className="permission-ask__body" hidden={!asking}>
          {asking && link.isPending && <Spinner label="Loading the approval link" />}
          {asking && link.isError && (
            <p className="field__error" role="alert">
              {errorMessage(link.error)}
            </p>
          )}
          {asking && link.data && (
            <ApprovalLink
              adminConsentUrl={link.data.adminConsentUrl}
              tenant={link.data.tenant}
              adminPageUrl={link.data.adminPageUrl}
              email={email}
              purpose="features"
              features={["sensitivityLabels"]}
              emphasis="compact"
            />
          )}
        </div>
      </section>
    );
  }

  if (account.refusedFeatures.includes("sensitivityLabels")) {
    const checkAgain = () => {
      // The refusal going away says it's back; a sync that's refused again says so.
      for (const f of account.refusedFeatures) noteFeatureTurnedOn(account.id, f);
      retry.mutate(account.id, { onError: (e: unknown) => toast.error(e) });
    };
    const emailIt = () => {
      backend
        .openLink(refusedFeatureMailto({ email, features: ["sensitivityLabels"] }))
        .catch((e: unknown) => toast.error(e));
    };
    return (
      <section className="protection-group" aria-labelledby={headingId}>
        {heading}
        <p className="protection-group__problem">
          Microsoft 365 refused access to your sensitivity labels
        </p>
        <p className="account-field__hint">{REFUSED_HINT}</p>
        <div className="protection-group__buttons">
          <button
            type="button"
            className="btn btn--sm"
            aria-label={`Check sensitivity labels again for ${email}`}
            aria-busy={retry.isPending || undefined}
            disabled={retry.isPending}
            onClick={checkAgain}
          >
            {retry.isPending ? (
              <LoaderCircle size={14} className="spin" aria-hidden="true" />
            ) : (
              <RefreshCw size={14} aria-hidden="true" />
            )}
            Check again
          </button>
          <button
            type="button"
            className="btn btn--sm"
            aria-label={`Email IT about sensitivity labels for ${email}`}
            onClick={emailIt}
          >
            <Mail size={14} aria-hidden="true" />
            Email IT
          </button>
        </div>
      </section>
    );
  }

  const { labels, policy, labelsListedAt, labelsError } = setup;
  const refreshButton = (label: string) => (
    <button
      type="button"
      className="btn btn--subtle btn--sm"
      aria-label={label === "Refresh" ? `Refresh sensitivity labels for ${email}` : undefined}
      aria-busy={refresh.isPending || undefined}
      disabled={refresh.isPending}
      onClick={refreshNow}
    >
      {refresh.isPending ? (
        <LoaderCircle size={14} className="spin" aria-hidden="true" />
      ) : (
        <RefreshCw size={14} aria-hidden="true" />
      )}
      {label}
    </button>
  );

  if (!labelsListedAt) {
    return (
      <section className="protection-group" aria-labelledby={headingId}>
        {heading}
        {labelsError ? (
          <div className="protection-group__line">
            <p className="account-field__hint selectable">
              Couldn&apos;t get your labels from Microsoft 365: {labelsError}
            </p>
            {refreshButton("Try again")}
          </div>
        ) : (
          <p className="account-field__hint protection-group__loading" role="status">
            <LoaderCircle size={14} className="spin" aria-hidden="true" />
            Getting your labels from Microsoft 365…
          </p>
        )}
      </section>
    );
  }

  const defaultLabel = labels.find((l) => l.id === policy.defaultLabelId);
  const toCheck = labelsToCheck(setup);
  const summary = [
    labels.length === 1 ? "1 label" : `${labels.length} labels`,
    ...(policy.required ? ["A label is required"] : []),
    ...(defaultLabel ? [`Default: ${labelTitle(defaultLabel)}`] : []),
  ].join(" · ");

  return (
    <section className="protection-group" aria-labelledby={headingId}>
      <div className="protection-group__head">
        {heading}
        {labels.length > 0 && (
          <CheckButton
            id="check:labels"
            busy={busy}
            pressed={pressed}
            // While a check runs, every label may be in it: still focusable.
            unavailable={!busy && toCheck.length === 0}
            label={`Check labels for ${email}`}
            onCheck={() => onCheck(toCheck.map(labelItem), "check:labels")}
          >
            {labels.length <= MAX_CHECK_ITEMS
              ? "Check all labels"
              : `Check ${MAX_CHECK_ITEMS} labels`}
          </CheckButton>
        )}
      </div>
      {labelsError && (
        <p className="account-field__hint protection-group__problem-text selectable">
          Couldn&apos;t update your labels ({labelsError}). These are from{" "}
          {checkDate(labelsListedAt)}.
        </p>
      )}
      <div className="protection-group__line">
        <p className="account-field__hint">
          {labels.length === 0
            ? "Microsoft 365 lists no sensitivity labels for email for you."
            : summary}
          {" · "}Updated {checkDate(labelsListedAt)}
        </p>
        {refreshButton("Refresh")}
      </div>
      {labels.length > MAX_CHECK_ITEMS && (
        <p className="account-field__hint">
          Cumulas Mail checks {MAX_CHECK_ITEMS} labels at a time. Check again for the rest.
        </p>
      )}
      {labels.length > 0 && (
        <ul className="protection-rows">
          {labels.map((label) => {
            const title = labelTitle(label);
            return (
              <CheckRow
                key={label.id}
                name={
                  <>
                    {label.color && (
                      <span
                        className="protection-row__dot"
                        style={{ background: label.color }}
                        aria-hidden="true"
                      />
                    )}
                    <bdi>{title}</bdi>
                    {label.hasProtection && (
                      <Lock
                        size={13}
                        strokeWidth={2}
                        className="protection-row__lock"
                        role="img"
                        aria-label="This label encrypts"
                      />
                    )}
                  </>
                }
                title={label.tooltip ?? undefined}
                spoken={`the label ${title}`}
                item={labelItem(label)}
                result={label.check}
                labels={labels}
                email={email}
                busy={busy}
                pressed={pressed}
                onCheck={onCheck}
                onOpen={onOpen}
              />
            );
          })}
        </ul>
      )}
    </section>
  );
}

interface ProtectionFieldProps {
  account: Account;
  /** Where the keyboard goes once a Turn on succeeds (the Sync now button). */
  returnFocusId: string;
}

/**
 * A work account's "Sensitivity labels and encryption": what the
 * organization's mail server applies to mail sent from Cumulas Mail, as
 * checks with test messages showed, and the labels Microsoft 365 lists.
 */
export function ProtectionField({ account, returnFocusId }: ProtectionFieldProps) {
  const id = useId();
  const backend = useBackend();
  const { data: setup, error } = useProtectionSetup(account.id);
  const check = useCheckProtection();
  const checking = setup?.checking ?? false;
  useProtectionCheckPolling(account.id, checking);
  // "Done" once a check seen here has stopped waiting.
  const [seenChecking, setSeenChecking] = useState(checking);
  const [finished, setFinished] = useState(false);
  if (checking !== seenChecking) {
    setSeenChecking(checking);
    setFinished(!checking);
  }

  const email = account.email;
  const busy = checking || check.isPending;
  const encryptionId = `${id}-encryption`;
  // The button that started the check that's running, shown busy.
  const [pressed, setPressed] = useState<string | null>(null);

  const runCheck = (items: CheckItem[], button: string) => {
    setPressed(button);
    check.mutate({ accountId: account.id, items }, { onError: (e: unknown) => toast.error(e) });
  };

  const openTestMessage = (item: CheckItem) => {
    backend.openProtectionCheckMessage(account.id, item).catch((e: unknown) => toast.error(e));
  };

  const openGuide = () => {
    backend.openLink(PROTECTION_GUIDE_URL).catch((e: unknown) => toast.error(e));
  };

  // What the checks found, for the organization's IT team.
  const anyChecked =
    setup !== undefined &&
    [setup.encrypt, setup.doNotForward, ...setup.labels.map((l) => l.check)].some(
      (c) => c.at !== null,
    );
  const copyDetails = () => {
    copyLater(backend.getProtectionCheckDetails(account.id))
      .then(() => toast.success(COPIED_DETAILS_FOR_IT))
      .catch((e: unknown) => toast.error(e));
  };
  const [settingUp, setSettingUp] = useState(false);

  return (
    <div className="account-field protection-field">
      <div className="account-field__head">
        <span className="account-field__label">Sensitivity labels and encryption</span>
        <span className="protection-field__links">
          <button type="button" className="btn btn--subtle btn--sm" onClick={openGuide}>
            <ExternalLink size={14} aria-hidden="true" />
            How IT sets this up
          </button>
          <button
            type="button"
            className="btn btn--subtle btn--sm"
            aria-haspopup="dialog"
            onClick={() => setSettingUp(true)}
          >
            <Building2 size={14} aria-hidden="true" />
            {ENTRY_BUTTON}
          </button>
          {anyChecked && (
            <button type="button" className="btn btn--subtle btn--sm" onClick={copyDetails}>
              <ClipboardList size={14} aria-hidden="true" />
              {COPY_DETAILS_FOR_IT}
            </button>
          )}
        </span>
      </div>
      {settingUp && <OrgSetupDialog account={account} onClose={() => setSettingUp(false)} />}
      <div>
        <p className="account-field__hint">
          Your organization&apos;s mail server can encrypt or label the mail you send from Cumulas
          Mail, once your IT team has set that up. Each option appears in new messages for{" "}
          {CHECK_VALID_DAYS} days after a test message here arrives encrypted or labeled.{" "}
          {CHECK_WAIT}
        </p>
        <p className="protection-field__progress" role="status">
          {checking
            ? `Cumulas Mail sent test messages to ${email} and is waiting for them to arrive. This usually takes a minute or two.`
            : finished
              ? "Done. Cumulas Mail moves the test messages it reads to Deleted Items."
              : ""}
        </p>
      </div>
      {!setup ? (
        error ? (
          <p className="field__error" role="alert">
            {errorMessage(error)}
          </p>
        ) : (
          <Spinner label="Loading" />
        )
      ) : (
        <>
          <section className="protection-group" aria-labelledby={encryptionId}>
            <div className="protection-group__head">
              <h3 id={encryptionId} className="protection-group__title">
                Encryption
              </h3>
              <CheckButton
                id="check:both"
                busy={busy}
                pressed={pressed}
                label={`Check Encrypt and Do Not Forward for ${email}`}
                onCheck={() =>
                  runCheck([actionItem("encrypt"), actionItem("doNotForward")], "check:both")
                }
              >
                Check both
              </CheckButton>
            </div>
            <ul className="protection-rows">
              {(["encrypt", "doNotForward"] as const).map((action) => (
                <CheckRow
                  key={action}
                  name={ACTION_NAMES[action]}
                  spoken={ACTION_NAMES[action]}
                  item={actionItem(action)}
                  result={action === "encrypt" ? setup.encrypt : setup.doNotForward}
                  labels={setup.labels}
                  email={email}
                  busy={busy}
                  pressed={pressed}
                  extra={action === "doNotForward" ? DNF_NOTE : undefined}
                  onCheck={runCheck}
                  onOpen={openTestMessage}
                />
              ))}
            </ul>
          </section>
          <LabelsGroup
            account={account}
            setup={setup}
            busy={busy}
            pressed={pressed}
            returnFocusId={returnFocusId}
            onCheck={runCheck}
            onOpen={openTestMessage}
          />
        </>
      )}
    </div>
  );
}
