import { ExternalLink, Lock, LogOut } from "lucide-react";
import { type ReactNode, useId, useState } from "react";

import { useBackend } from "../../../api/context";
import { errorMessage } from "../../../api/errors";
import type {
  Account,
  OrgSetupChoices,
  OrgSetupPlan,
  OrgSetupRow,
  OrgSetupSection,
  OrgSetupStatus,
} from "../../../api/types";
import { Spinner } from "../../../components/Spinner";
import { toast } from "../../../components/toast";
import { ApprovalLink } from "../AdminConsent";
import { Callout } from "../Fields";
import { useAdminConsentLink } from "../queries";
import {
  BUTTONS,
  DIALOG,
  FOOTER,
  LABEL_DISCLOSURE,
  NOT_IN_YOUR_LIST,
  NO_ROLE,
  PERMISSIONS,
  ROWS,
  ROW_TITLES,
  SECTION_TITLES,
  SIGNED_IN_AS,
} from "./copy";
import { Chip, DetailText } from "./parts";

/** A detail this long, or in paragraphs, goes behind a disclosure. */
const SHORT_DETAIL = 160;

interface RowActions {
  /** "Sign in with another account". */
  onSignIn: () => void;
  onShowScript: () => void;
}

/** One line of the status: title, chip, summary, detail and what fixes it (3.6.4). */
function StatusRow({
  row,
  title,
  onSignIn,
  onShowScript,
  children,
}: RowActions & { row: OrgSetupRow; title?: ReactNode; children?: ReactNode }) {
  const backend = useBackend();
  const detail = row.detail;
  const manual = row.fix === "manual";
  const long = detail !== null && (detail.length > SHORT_DETAIL || detail.includes("\n"));
  const open = (url: string) => backend.openLink(url).catch((e: unknown) => toast.error(e));

  return (
    <li className="orgsetup-row">
      <div className="orgsetup-row__head">
        <span className="orgsetup-row__title">{title ?? <bdi>{row.title}</bdi>}</span>
        <Chip state={row.state} />
      </div>
      {row.summary && <p className="orgsetup-row__summary selectable">{row.summary}</p>}
      {detail && !manual && !long && <p className="orgsetup-row__detail selectable">{detail}</p>}
      {detail && !manual && long && (
        <details className="orgsetup-disclosure">
          <summary>{DIALOG.moreDetail}</summary>
          <DetailText text={detail} />
        </details>
      )}
      {manual && (
        <details className="orgsetup-disclosure">
          <summary>{BUTTONS.howToFix}</summary>
          {detail && <DetailText text={detail} />}
          <button type="button" className="btn btn--sm" onClick={onShowScript}>
            {BUTTONS.showScript}
          </button>
        </details>
      )}
      {row.fix === "signatures" && row.link && (
        <div className="orgsetup-row__actions">
          <button type="button" className="btn btn--sm" onClick={() => open(row.link!)}>
            <ExternalLink size={14} aria-hidden="true" />
            {BUTTONS.openSignatures}
          </button>
        </div>
      )}
      {row.fix === "signIn" && (
        <div className="orgsetup-row__actions">
          <button type="button" className="btn btn--sm" onClick={onSignIn}>
            {BUTTONS.signInWithAnother}
          </button>
        </div>
      )}
      {children}
    </li>
  );
}

/**
 * The `approval` row as the work account says it, before any setup sign-in:
 * the same words the core's status uses (3.6.4).
 */
function accountApprovalRow(account: Account): OrgSetupRow {
  const base = {
    id: "approval",
    section: "approval" as const,
    title: ROW_TITLES.approval,
    detail: null,
    objectName: null,
    fix: "approval" as const,
    link: null,
  };
  if (account.needsPermission.includes("sensitivityLabels")) {
    return { ...base, state: "missing", summary: ROWS.approval.missing(account.email) };
  }
  if (account.refusedFeatures.includes("sensitivityLabels")) {
    return { ...base, state: "blocked", summary: ROWS.approval.refused(account.email) };
  }
  return { ...base, state: "ok", summary: ROWS.approval.ok(account.email) };
}

/** Approve again for everyone: the mail app's own approval link, behind its button. */
function ApproveAgain({ account, startOpen }: { account: Account; startOpen: boolean }) {
  const id = useId();
  const [open, setOpen] = useState(startOpen);
  const link = useAdminConsentLink(account.id, open);
  return (
    <div className="orgsetup-approve">
      <div className="orgsetup-row__actions">
        <button
          type="button"
          className="btn btn--sm"
          aria-expanded={open}
          aria-controls={id}
          onClick={() => setOpen((o) => !o)}
        >
          {BUTTONS.approveForEveryone}
        </button>
      </div>
      <div id={id} className="orgsetup-approve__body" hidden={!open}>
        {open && link.isPending && <Spinner label="Loading the approval link" />}
        {open && link.isError && (
          <p className="field__error" role="alert">
            {errorMessage(link.error)}
          </p>
        )}
        {open && link.data && (
          <>
            <ApprovalLink
              adminConsentUrl={link.data.adminConsentUrl}
              tenant={link.data.tenant}
              adminPageUrl={link.data.adminPageUrl}
              email={account.email}
              purpose="features"
              features={["sensitivityLabels"]}
              emphasis="compact"
            />
            <p className="orgsetup-note">{PERMISSIONS.approvesEverything}</p>
          </>
        )}
      </div>
    </div>
  );
}

interface PermissionsSectionProps extends RowActions {
  account: Account;
  /** The section's rows from the status; before a sign-in, the work account's own row. */
  rows?: OrgSetupRow[];
}

/**
 * "Cumulas Mail's permissions" (3.12): re-approving the mail app's own
 * permissions needs no setup sign-in, so it's there from the start.
 */
export function PermissionsSection({ account, rows, ...actions }: PermissionsSectionProps) {
  const headingId = useId();
  const shown = rows ?? [accountApprovalRow(account)];
  return (
    <section className="orgsetup-section" aria-labelledby={headingId}>
      <h3 id={headingId} className="orgsetup-section__title">
        {SECTION_TITLES.approval}
      </h3>
      <ul className="orgsetup-rows">
        {shown.map((row) => (
          <StatusRow key={row.id} row={row} {...actions}>
            {row.fix === "approval" && (
              <ApproveAgain account={account} startOpen={row.state === "missing"} />
            )}
          </StatusRow>
        ))}
      </ul>
      <details className="orgsetup-disclosure">
        <summary>{BUTTONS.whatApprovingCantFix}</summary>
        <p className="orgsetup-note selectable">{PERMISSIONS.cantFix}</p>
      </details>
    </section>
  );
}

function Section({
  section,
  rows,
  actions,
  children,
}: {
  section: OrgSetupSection;
  rows: OrgSetupRow[];
  actions: RowActions;
  children?: ReactNode;
}) {
  const headingId = useId();
  if (rows.length === 0 && !children) return null;
  return (
    <section className="orgsetup-section" aria-labelledby={headingId}>
      <h3 id={headingId} className="orgsetup-section__title">
        {SECTION_TITLES[section]}
      </h3>
      {rows.length > 0 && (
        <ul className="orgsetup-rows">
          {rows.map((row) => (
            <StatusRow key={row.id} row={row} {...actions} />
          ))}
        </ul>
      )}
      {children}
    </section>
  );
}

interface StatusViewProps extends RowActions {
  account: Account;
  status: OrgSetupStatus;
  choices: OrgSetupChoices;
  plan: OrgSetupPlan;
  /** The plan for changed choices is being worked out. */
  planning: boolean;
  onChoices: (choices: OrgSetupChoices) => void;
  onSignOut: () => void;
  /** Above the rows (a stale plan's message). */
  notice?: ReactNode;
}

/** What setup needs, as read, with the choices and the plan they make (3.13.2). */
export function StatusView({
  account,
  status,
  choices,
  plan,
  planning,
  onChoices,
  onSignOut,
  notice,
  ...actions
}: StatusViewProps) {
  const id = useId();
  const of = (section: OrgSetupSection) =>
    status.rows.filter((r) => r.section === section && !r.id.startsWith("label:"));
  const labelRows = new Map(
    status.rows.filter((r) => r.id.startsWith("label:")).map((r) => [r.id.slice(6), r]),
  );
  const templatesOk = status.rows.find((r) => r.id === "templates")?.state === "ok";
  const set = (patch: Partial<OrgSetupChoices>) => onChoices({ ...choices, ...patch });
  const toggleLabel = (labelId: string, on: boolean) =>
    set({
      labelIds: on
        ? [...choices.labelIds.filter((l) => l !== labelId), labelId]
        : choices.labelIds.filter((l) => l !== labelId),
    });
  const n = plan.changes.length;

  return (
    <div className="orgsetup-status">
      <div className="orgsetup-signed-in">
        <p className="selectable">{SIGNED_IN_AS(status.admin.username)}</p>
        <button type="button" className="btn btn--subtle btn--sm" onClick={onSignOut}>
          <LogOut size={14} aria-hidden="true" />
          {BUTTONS.signOut}
        </button>
      </div>
      {notice}
      <PermissionsSection account={account} rows={of("approval")} {...actions} />
      {status.noRole ? (
        <Callout tone="warning">
          <p>{NO_ROLE}</p>
          <div className="orgsetup-buttons">
            <button type="button" className="btn btn--sm" onClick={actions.onSignIn}>
              {BUTTONS.signInWithAnother}
            </button>
            <button type="button" className="btn btn--sm" onClick={actions.onShowScript}>
              {BUTTONS.showScript}
            </button>
          </div>
        </Callout>
      ) : (
        <>
          <Section section="encryption" rows={of("encryption")} actions={actions}>
            {status.exchangeReadable && (
              <div className="orgsetup-choices">
                <label className="checkbox">
                  <input
                    type="checkbox"
                    checked={choices.actions}
                    onChange={(e) => set({ actions: e.currentTarget.checked })}
                  />
                  {BUTTONS.setUpActions}
                </label>
                {choices.actions && !templatesOk && status.templates.length > 0 && (
                  <div className="orgsetup-templates">
                    <TemplateSelect
                      id={`${id}-encrypt`}
                      label={DIALOG.encryptTemplate}
                      value={choices.encryptTemplate}
                      names={status.templates.map((t) => t.name)}
                      onChange={(name) => set({ encryptTemplate: name })}
                    />
                    <TemplateSelect
                      id={`${id}-dnf`}
                      label={DIALOG.doNotForwardTemplate}
                      value={choices.doNotForwardTemplate}
                      names={status.templates.map((t) => t.name)}
                      onChange={(name) => set({ doNotForwardTemplate: name })}
                    />
                  </div>
                )}
              </div>
            )}
          </Section>
          <Section section="labels" rows={of("labels")} actions={actions}>
            {status.labels.length > 0 && (
              <fieldset className="orgsetup-labels">
                <legend className="orgsetup-labels__legend">{DIALOG.labelsToSetUp}</legend>
                <p className="orgsetup-note">{LABEL_DISCLOSURE}</p>
                <ul className="orgsetup-rows">
                  {status.labels.map((label) => {
                    const row = labelRows.get(label.id);
                    const checkboxId = `${id}-label-${label.id}`;
                    return (
                      <StatusRow
                        key={label.id}
                        row={
                          row ?? {
                            id: `label:${label.id}`,
                            section: "labels",
                            title: label.title,
                            state: label.state,
                            summary: "",
                            detail: null,
                            objectName: null,
                            fix: null,
                            link: null,
                          }
                        }
                        title={
                          <span className="orgsetup-label">
                            <input
                              id={checkboxId}
                              type="checkbox"
                              checked={choices.labelIds.includes(label.id)}
                              onChange={(e) => toggleLabel(label.id, e.currentTarget.checked)}
                            />
                            <label htmlFor={checkboxId}>
                              <bdi>{label.title}</bdi>
                            </label>
                            {label.encrypts && (
                              <Lock
                                size={13}
                                strokeWidth={2}
                                className="orgsetup-label__lock"
                                role="img"
                                aria-label="This label encrypts"
                              />
                            )}
                            {!label.inCatalog && (
                              <span className="orgsetup-tag">{NOT_IN_YOUR_LIST}</span>
                            )}
                          </span>
                        }
                        {...actions}
                      />
                    );
                  })}
                </ul>
                {status.license !== "included" && (
                  <label className="checkbox">
                    <input
                      type="checkbox"
                      checked={choices.labelsWithoutLicense}
                      onChange={(e) => set({ labelsWithoutLicense: e.currentTarget.checked })}
                    />
                    {BUTTONS.labelsAnyway}
                  </label>
                )}
              </fieldset>
            )}
          </Section>
          <Section section="routing" rows={of("routing")} actions={actions} />
        </>
      )}
      <div className="orgsetup-plan" aria-busy={planning || undefined}>
        <p className="orgsetup-plan__line" role="status">
          {planning
            ? DIALOG.planning
            : `${n > 0 ? FOOTER.changes(n) : FOOTER.nothing}${plan.blocked > 0 ? ` ${FOOTER.blocked(plan.blocked)}` : ""}`}
        </p>
        {!planning && plan.problems.length > 0 && (
          <ul className="orgsetup-problems" role="alert">
            {plan.problems.map((problem) => (
              <li key={problem}>{problem}</li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

function TemplateSelect({
  id,
  label,
  value,
  names,
  onChange,
}: {
  id: string;
  label: string;
  value: string | null;
  names: string[];
  onChange: (name: string | null) => void;
}) {
  return (
    <div className="field">
      <label htmlFor={id} className="field__label">
        {label}
      </label>
      <select
        id={id}
        className="select"
        value={value ?? ""}
        onChange={(e) => onChange(e.currentTarget.value || null)}
      >
        {value === null && <option value="">{DIALOG.chooseTemplate}</option>}
        {names.map((name) => (
          <option key={name} value={name}>
            {name}
          </option>
        ))}
      </select>
    </div>
  );
}
