import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { beforeEach, describe, expect, it, onTestFinished, vi } from "vitest";

import {
  type DemoOrgSetup,
  MockBackend,
  mockAdminConsentUrl,
  mockSetupConsentUrl,
} from "../../../api/mock";
import { DEMO_LABEL_IDS } from "../../../api/mock/data";
import type {
  Account,
  BackendError,
  OrgSetupAdmin,
  OrgSetupChange,
  OrgSetupChangeResult,
  OrgSetupChoices,
  OrgSetupLabel,
  OrgSetupPlan,
  OrgSetupResult,
  OrgSetupRow,
  OrgSetupRowState,
  OrgSetupSection,
  OrgSetupStatus,
} from "../../../api/types";
import { Dialog } from "../../../components/Dialog";
import { useToasts } from "../../../components/toast";
import { renderWithBackend } from "../../../test/render";
import {
  ACTION_VERBS,
  APPLYING,
  CHIPS,
  CONFIRM,
  CONSENT,
  DIALOG_TITLE,
  DONE,
  FOOTER,
  INTRO,
  LABEL_DISCLOSURE,
  NO_ROLE,
  OBJECT_KINDS,
  PERMISSIONS,
  PLAN_PROBLEMS,
  REMOVE_APPROVAL,
  ROWS,
  ROW_TITLES,
  SCRIPT_VIEW,
  SETUP_APP_IT_PAGE_URL,
  SIGNED_IN_AS,
  SIGNED_OUT,
  SKIPPED_SIGNED_OUT,
  STALE,
} from "./copy";
import { OrgSetupDialog } from "./OrgSetupDialog";

// The organization the dialog sets up is a fixture here, scenario by
// scenario as design 7.4 describes them, in the core's words (copy.ts).

const EMAIL = "alex.morgan@acme.example";
const ORG = "acme.example";
const ADMIN: OrgSetupAdmin = { username: "sam.rivera@acme.example", displayName: "Sam Rivera" };
const ADMIN_OID = "4d1f6b2a-0c3e-4f5a-9b8c-7d6e5f4a3b21";
const SETUP_CLIENT_ID = "b4f17c55-b9f6-4a84-958f-afe3842da327";
const SETUP_LINK = `https://login.microsoftonline.com/demo-tenant/v2.0/adminconsent?client_id=${SETUP_CLIENT_ID}&scope=openid+profile+offline_access+https%3A%2F%2Foutlook.office365.com%2FExchange.Manage&redirect_uri=http%3A%2F%2Flocalhost`;
const SIGNATURES_MANAGE = "https://signature.cumulas.co/manage";
const { general: GENERAL, allEmployees: ALL_EMPLOYEES } = DEMO_LABEL_IDS;
/** A label Purview lists that isn't in the work account's own list. */
const BOARD = "c3d4e5f6-0718-4a29-b3c4-d5e6f7a8b9c0";
const ENCRYPT = "Cumulas Mail: Encrypt";
const DNF = "Cumulas Mail: Do Not Forward";
const STRIP = "Cumulas Mail: ignore label requests from outside";
/** What the core says of one that was already there (shown with "doesn't list it yet" too). */
const ADOPTED = "It was already there; Cumulas Mail repaired it.";
const LICENSE_MISSING =
  "Microsoft 365 says a license is missing: both the administrator who signs in and the people who send need a plan that includes automatic labeling. Microsoft said: The tenant isn't licensed for auto-labeling.";

type Scenario =
  | "partial"
  | "unlicensed"
  | "consent"
  | "noRole"
  | "exchangeOnly"
  | "blocked"
  | "failing"
  | "signedOut"
  | "relay";

/** One organization's setup as a scenario has it; changes made from the dialog change it. */
interface Org {
  scenario: Scenario;
  signedIn: OrgSetupAdmin | null;
  /** The changes were made. */
  made: boolean;
  /** Someone changed a rule in Microsoft 365 meanwhile: the plan gains a change. */
  changedElsewhere: boolean;
}

function row(
  id: string,
  section: OrgSetupSection,
  title: string,
  state: OrgSetupRowState,
  summary: string,
  more: Partial<OrgSetupRow> = {},
): OrgSetupRow {
  return {
    id,
    section,
    title,
    state,
    summary,
    detail: null,
    objectName: null,
    fix: null,
    link: null,
    ...more,
  };
}

const exchangeReads = (org: Org) => org.scenario !== "noRole";
const labelsRead = (org: Org) => org.scenario !== "noRole" && org.scenario !== "exchangeOnly";
/** Nothing of Cumulas Mail's exists yet. */
const fresh = (org: Org) => org.scenario === "unlicensed";

function labelsOf(org: Org): OrgSetupLabel[] {
  if (!exchangeReads(org)) return [];
  const label = (id: string, title: string, inCatalog: boolean, encrypts: boolean) => ({
    id,
    title,
    inCatalog,
    encrypts,
    forEmail: labelsRead(org) ? true : null,
    state: (labelsRead(org) ? "missing" : "unknown") as OrgSetupRowState,
  });
  return [
    {
      ...label(GENERAL, "General", true, false),
      state: labelsRead(org) && !fresh(org) ? "off" : "missing",
    },
    label(ALL_EMPLOYEES, "Confidential › All Employees", true, true),
    label(BOARD, "Board", false, true),
  ];
}

function rowsOf(org: Org): OrgSetupRow[] {
  const waiting = (id: string, section: OrgSetupSection, title: string, name: string) =>
    row(id, section, title, "waiting", ROWS.rule.waiting(1), { objectName: name });
  const rows: OrgSetupRow[] = [
    row("approval", "approval", ROW_TITLES.approval, "ok", ROWS.approval.ok(EMAIL), {
      fix: "approval",
    }),
    row("setupApp", "approval", ROW_TITLES.setupApp, "info", ROWS.setupApp(ADMIN.username), {
      detail: REMOVE_APPROVAL.detail(SETUP_CLIENT_ID, ADMIN_OID),
    }),
  ];
  if (!exchangeReads(org)) {
    rows.push(
      row(
        "exchangeAccess",
        "encryption",
        ROW_TITLES.exchangeAccess,
        "blocked",
        ROWS.exchangeAccess.noRole,
        {
          fix: "signIn",
        },
      ),
    );
  } else {
    rows.push(row("irm", "encryption", ROW_TITLES.irm, "ok", ROWS.irm.ok));
    rows.push(row("templates", "encryption", ROW_TITLES.templates, "ok", ROWS.templates.ok));
    if (org.made) {
      rows.push(waiting("encryptRule", "encryption", ROW_TITLES.encryptRule, ENCRYPT));
      rows.push(waiting("dnfRule", "encryption", ROW_TITLES.dnfRule, DNF));
    } else if (fresh(org)) {
      rows.push(
        row("encryptRule", "encryption", ROW_TITLES.encryptRule, "missing", ROWS.rule.missing, {
          fix: "setup",
        }),
      );
      rows.push(
        row("dnfRule", "encryption", ROW_TITLES.dnfRule, "missing", ROWS.rule.missing, {
          fix: "setup",
        }),
      );
    } else {
      rows.push(
        org.scenario === "blocked"
          ? row(
              "encryptRule",
              "encryption",
              ROW_TITLES.encryptRule,
              "blocked",
              ROWS.rule.extras("BlindCopyTo"),
              {
                objectName: ENCRYPT,
                detail: "BlindCopyTo: audit@acme.example",
                fix: "manual",
              },
            )
          : row(
              "encryptRule",
              "encryption",
              ROW_TITLES.encryptRule,
              "differs",
              ROWS.rule.differs("it acts only on mail from inside your organization"),
              { objectName: ENCRYPT, fix: "setup" },
            ),
      );
      rows.push(
        row("dnfRule", "encryption", ROW_TITLES.dnfRule, "off", ROWS.rule.testMode("Audit"), {
          objectName: DNF,
          fix: "setup",
        }),
      );
    }
  }
  if (!labelsRead(org)) {
    rows.push(
      row("labelsAccess", "labels", ROW_TITLES.labelsAccess, "blocked", ROWS.labelsAccess.noRole, {
        fix: "signIn",
      }),
    );
  }
  if (exchangeReads(org)) {
    rows.push(
      org.made
        ? waiting("stripRule", "labels", ROW_TITLES.stripRule, STRIP)
        : row("stripRule", "labels", ROW_TITLES.stripRule, "missing", ROWS.rule.missing, {
            fix: "setup",
          }),
    );
    rows.push(
      org.scenario === "unlicensed"
        ? row(
            "licensing",
            "labels",
            ROW_TITLES.licensing,
            "blocked",
            ROWS.licensing.notIncluded(EMAIL),
            {
              detail: ROWS.licensing.notIncludedDetail,
            },
          )
        : row("licensing", "labels", ROW_TITLES.licensing, "ok", ROWS.licensing.included(EMAIL), {
            detail: ROWS.licensing.includedDetail("SPE_E5"),
          }),
    );
  }
  if (labelsRead(org)) {
    for (const label of labelsOf(org)) {
      const name = `Cumulas Mail: ${label.title.replace("Confidential › ", "")}`;
      const general = label.id === GENERAL;
      rows.push(
        row(
          `label:${label.id}`,
          "labels",
          label.title,
          general && org.made ? "waiting" : label.state,
          general && org.made
            ? ROWS.label.simulating
            : label.state === "off"
              ? ROWS.label.simulationOnly
              : ROWS.label.missing,
          { objectName: name, fix: "setup", detail: ROWS.label.matchedBy("Guid", label.id) },
        ),
      );
    }
  }
  if (exchangeReads(org)) {
    rows.push(
      org.scenario === "relay"
        ? row(
            "routing",
            "routing",
            ROW_TITLES.routing,
            "differs",
            ROWS.routing.cumulasNotInternal,
            {
              fix: "signatures",
              link: SIGNATURES_MANAGE,
            },
          )
        : row("routing", "routing", ROW_TITLES.routing, "ok", ROWS.routing.none),
    );
  }
  if (exchangeReads(org) && !fresh(org)) {
    const leftover = "Cumulas Mail: Encrypt after the signature";
    rows.push(row(`leftover:${leftover}`, "routing", leftover, "info", ROWS.leftover.unused));
  }
  return rows;
}

function change(
  action: OrgSetupChange["action"],
  object: OrgSetupChange["object"],
  name: string,
  details: string[],
): OrgSetupChange {
  return {
    id: `${action}:${object}:${name.toLowerCase()}`,
    action,
    object,
    name,
    details,
    powershell: `${object === "mailFlowRule" ? "Set-TransportRule" : "Set-AutoSensitivityLabelPolicy"} -Identity '${name}'`,
  };
}

function planOf(org: Org, choices: OrgSetupChoices): OrgSetupPlan {
  const changes: OrgSetupChange[] = [];
  const problems: string[] = [];
  if (org.made) return { changes, problems, blocked: 0 };
  const verb = fresh(org) ? "create" : "change";
  if (exchangeReads(org) && choices.actions) {
    if (org.scenario !== "blocked") {
      changes.push(
        change(verb, "mailFlowRule", ENCRYPT, [
          org.scenario === "relay"
            ? "Moves above “Email signatures: add signature”"
            : "Acts on mail from anyone but senders outside your organization",
        ]),
      );
    }
    changes.push(
      change(fresh(org) ? "create" : "turnOn", "mailFlowRule", DNF, ["Mode: Enforce (was Audit)"]),
    );
  }
  if (org.changedElsewhere) {
    changes.push(change("move", "mailFlowRule", DNF, ["Moves above “Contoso: route to archive”"]));
  }
  if (labelsRead(org) && choices.labelIds.length > 0) {
    if (org.scenario === "unlicensed" && !choices.labelsWithoutLicense) {
      problems.push(PLAN_PROBLEMS.unlicensed(EMAIL));
    } else {
      changes.push(change("create", "mailFlowRule", STRIP, ["Goes to the top of the list"]));
      for (const id of choices.labelIds) {
        const label = labelsOf(org).find((l) => l.id === id)!;
        const name = `Cumulas Mail: ${label.title.replace("Confidential › ", "")}`;
        if (label.state === "off") {
          changes.push(
            change("startSimulation", "autoLabelingPolicy", name, ["Starts its simulation"]),
          );
        } else {
          changes.push(
            change("create", "autoLabelingPolicy", name, [
              `Applies the label ${label.title} to mail that asks for it`,
            ]),
            change("create", "autoLabelingRule", `${name} (header)`, [
              `Matches the header x-cumulas-label with ${id}`,
            ]),
          );
        }
      }
    }
  }
  // Like the core: the disclosure goes under the first label change.
  changes.find((c) => c.object !== "mailFlowRule")?.details.unshift(LABEL_DISCLOSURE);
  return { changes, problems, blocked: org.scenario === "blocked" ? 1 : 0 };
}

function defaultsOf(org: Org): OrgSetupChoices {
  return {
    actions: exchangeReads(org),
    encryptTemplate: exchangeReads(org) ? "Encrypt" : null,
    doNotForwardTemplate: exchangeReads(org) ? "Do Not Forward" : null,
    // Only the labels with a policy of Cumulas Mail's already.
    labelIds: labelsRead(org) && !fresh(org) ? [GENERAL] : [],
    labelsWithoutLicense: false,
  };
}

function statusOf(org: Org): OrgSetupStatus {
  const defaults = defaultsOf(org);
  return {
    readAt: "2026-10-19T09:00:00Z",
    admin: ADMIN,
    rows: rowsOf(org),
    labels: labelsOf(org),
    templates: exchangeReads(org)
      ? [
          { name: "Encrypt", id: null },
          { name: "Do Not Forward", id: null },
        ]
      : [],
    relay: org.scenario === "relay" ? "cumulasNotInternal" : "none",
    license: org.scenario === "unlicensed" ? "notIncluded" : "included",
    exchangeReadable: exchangeReads(org),
    labelsReadable: labelsRead(org),
    noRole: org.scenario === "noRole",
    anyOurs: !fresh(org) && exchangeReads(org),
    defaults,
    plan: planOf(org, defaults),
  };
}

function outcomes(
  org: Org,
  changes: OrgSetupChange[],
): { results: OrgSetupChangeResult[]; signedOut: boolean } {
  const done = (c: OrgSetupChange): OrgSetupChangeResult => ({
    change: c,
    outcome: "done",
    message: null,
  });
  if (org.scenario === "signedOut") {
    return {
      signedOut: true,
      results: changes.map((c, i) =>
        i === 0
          ? done(c)
          : {
              change: c,
              outcome: "skipped",
              message: "Your sign-in ended. Sign in again to continue.",
            },
      ),
    };
  }
  if (org.scenario === "failing") {
    let failed = false;
    return {
      signedOut: false,
      results: changes.map((c) => {
        if (c.object === "mailFlowRule") return done(c);
        if (!failed) {
          failed = true;
          return { change: c, outcome: "failed", message: LICENSE_MISSING };
        }
        return { change: c, outcome: "skipped", message: LICENSE_MISSING };
      }),
    };
  }
  return {
    signedOut: false,
    results: changes.map((c) =>
      c.object === "autoLabelingRule"
        ? { change: c, outcome: "notYetListed", message: ADOPTED }
        : done(c),
    ),
  };
}

const ended = (message = "Your setup sign-in ended. Sign in again to continue."): BackendError => ({
  code: "authRequired",
  message,
  details: { reason: "setupSignedOut" },
});

/** A demo backend whose organization setup is the scenario's (the rest is the demo's). */
function setUpBackend(scenario: Scenario, backend = new MockBackend()) {
  const org: Org = { scenario, signedIn: null, made: false, changedElsewhere: false };
  /** Holds Make these changes until the test lets it go. */
  let gate: Promise<void> | null = null;
  vi.spyOn(backend, "getOrgSetupInfo").mockImplementation(async () => ({
    automatic: true,
    tenantId: "demo-tenant",
    signedIn: org.signedIn,
    approvalUrl: SETUP_LINK,
    rulesChangedAt: null,
    policiesChangedAt: null,
  }));
  const signIn = vi.spyOn(backend, "orgSetupSignIn").mockImplementation(async () => {
    if (scenario === "consent") {
      throw {
        code: "adminConsentRequired",
        message: CONSENT.message,
        details: {
          reason: "setupConsent",
          adminConsentUrl: SETUP_LINK,
          tenant: "demo-tenant",
          aadsts: 65001,
        },
      };
    }
    org.signedIn = ADMIN;
    return ADMIN;
  });
  vi.spyOn(backend, "readOrgSetup").mockImplementation(async () => {
    if (!org.signedIn) throw ended();
    return statusOf(org);
  });
  vi.spyOn(backend, "planOrgSetup").mockImplementation(async (_id, choices) => {
    if (!org.signedIn) throw ended();
    return planOf(org, choices);
  });
  const apply = vi
    .spyOn(backend, "applyOrgSetup")
    .mockImplementation(async (_id, choices, confirmed) => {
      if (gate) await gate;
      if (!org.signedIn) throw ended();
      const plan = planOf(org, choices);
      const now = plan.changes.map((c) => c.id);
      if (now.length !== confirmed.length || now.some((id, i) => id !== confirmed[i])) {
        const result: OrgSetupResult = {
          stale: true,
          signedOut: false,
          changes: [],
          status: statusOf(org),
        };
        return result;
      }
      const { results, signedOut } = outcomes(org, plan.changes);
      org.made = true;
      if (signedOut) org.signedIn = null;
      return { stale: false, signedOut, changes: results, status: statusOf(org) };
    });
  vi.spyOn(backend, "getOrgSetupDetails").mockResolvedValue(
    "Cumulas Mail 0.5.0: organization setup details",
  );
  const end = backend.endOrgSetup.bind(backend);
  vi.spyOn(backend, "endOrgSetup").mockImplementation(async () => {
    org.signedIn = null;
    await end();
  });
  const cancel = vi.spyOn(backend, "cancelOrgSetupSignIn");
  return {
    backend,
    org,
    signIn,
    apply,
    cancel,
    /** Holds the next Make these changes; returns what lets it go. */
    hold: () => {
      let release!: () => void;
      gate = new Promise<void>((resolve) => (release = resolve));
      return () => {
        gate = null;
        release();
      };
    },
  };
}

/** Settings' Sensitivity labels and encryption, as far as the dialog is concerned. */
function Opener({ account }: { account: Account }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button type="button" aria-haspopup="dialog" onClick={() => setOpen(true)}>
        Set up or check for your organization…
      </button>
      {open && <OrgSetupDialog account={account} onClose={() => setOpen(false)} />}
    </>
  );
}

async function openDialog(backend: MockBackend) {
  const user = userEvent.setup();
  const account = (await backend.listAccounts()).find((a) => a.id === "acct-work")!;
  const utils = renderWithBackend(<Opener account={account} />, { backend });
  const opener = screen.getByRole("button", { name: "Set up or check for your organization…" });
  await user.click(opener);
  const dialog = await screen.findByRole("dialog");
  return { ...utils, user, dialog, opener, account };
}

const title = (name: string) => screen.getByRole("heading", { level: 2, name });

/** The status row titled `name`, in the section titled `section`. */
function statusRow(dialog: HTMLElement, section: string, name: string) {
  const region = within(dialog).getByRole("region", { name: section });
  const found = within(region)
    .getAllByRole("listitem")
    .find((li) => li.querySelector(".orgsetup-row__title")?.textContent === name);
  if (!found) throw new Error(`No row ${name} in ${section}`);
  return found;
}

async function signIn(user: ReturnType<typeof userEvent.setup>, dialog: HTMLElement) {
  await user.click(within(dialog).getByRole("button", { name: "Sign in as an administrator" }));
  await within(dialog).findByText(SIGNED_IN_AS(ADMIN.username));
}

const toasts = () => useToasts.getState().toasts.map((t) => t.message);

beforeEach(() => {
  useToasts.setState({ toasts: [] });
});

describe("Set up or check for your organization", () => {
  it("re-approves, signs in, reads, confirms the changes and makes them", async () => {
    const { backend, signIn: signInSpy, apply, hold } = setUpBackend("partial");
    const { user, dialog, opener } = await openDialog(backend);
    await waitFor(() => expect(title(DIALOG_TITLE)).toHaveFocus());

    // Before any sign-in: what setup does, and Cumulas Mail's own permissions.
    expect(await within(dialog).findByText(INTRO.what)).toBeInTheDocument();
    expect(within(dialog).getByText(INTRO.who(ORG))).toBeInTheDocument();
    expect(within(dialog).getByText(INTRO.undocumented)).toBeInTheDocument();
    expect(within(dialog).getByText(INTRO.notAdmin)).toBeInTheDocument();
    const permissions = within(dialog).getByRole("region", { name: "Cumulas Mail's permissions" });
    const approval = statusRow(dialog, "Cumulas Mail's permissions", "Cumulas Mail's permissions");
    expect(within(approval).getByText(CHIPS.ok)).toBeInTheDocument();
    expect(within(approval).getByText(ROWS.approval.ok(EMAIL))).toBeInTheDocument();
    const again = within(permissions).getByRole("button", { name: "Approve again for everyone" });
    expect(again).toHaveAttribute("aria-expanded", "false");
    await user.click(again);
    expect(again).toHaveAttribute("aria-expanded", "true");
    expect(
      await within(permissions).findByLabelText("Approval link for your IT administrator"),
    ).toHaveValue(mockAdminConsentUrl("demo-tenant"));
    expect(within(permissions).getByRole("button", { name: "Email IT" })).toBeInTheDocument();
    expect(within(permissions).getByText(PERMISSIONS.approvesEverything)).toBeInTheDocument();
    expect(within(permissions).getByText("What approving can't fix")).toBeInTheDocument();
    expect(within(permissions).getByText(PERMISSIONS.cantFix)).not.toBeVisible();
    expect(signInSpy).not.toHaveBeenCalled();
    // Nothing to copy before a sign-in was tried.
    expect(within(dialog).queryByRole("button", { name: "Copy setup details" })).toBeNull();

    // Sign in, and the status, section by section, every chip in words.
    await signIn(user, dialog);
    expect(signInSpy).toHaveBeenCalledWith("acct-work");
    await waitFor(() => expect(title(DIALOG_TITLE)).toHaveFocus());
    expect(
      within(statusRow(dialog, "Cumulas Mail's permissions", ROW_TITLES.setupApp)).getByText(
        ROWS.setupApp(ADMIN.username),
      ),
    ).toBeInTheDocument();
    const encrypt = statusRow(dialog, "Encryption", "Encrypt rule");
    expect(within(encrypt).getByText(CHIPS.differs)).toBeInTheDocument();
    expect(
      within(encrypt).getByText(
        ROWS.rule.differs("it acts only on mail from inside your organization"),
      ),
    ).toBeInTheDocument();
    expect(
      within(statusRow(dialog, "Encryption", "Do Not Forward rule")).getByText(CHIPS.off),
    ).toBeInTheDocument();
    expect(
      within(statusRow(dialog, "Encryption", "Message Encryption")).getByText(CHIPS.ok),
    ).toBeInTheDocument();
    expect(
      within(statusRow(dialog, "Sensitivity labels", "Label requests from outside")).getByText(
        CHIPS.missing,
      ),
    ).toBeInTheDocument();
    expect(
      within(
        statusRow(dialog, "Mail routing", "Cumulas Mail: Encrypt after the signature"),
      ).getByText(CHIPS.info),
    ).toBeInTheDocument();

    // The labels: the disclosure above them, the one with a policy already chosen.
    const labels = within(dialog).getByRole("group", { name: "Labels to set up or repair" });
    expect(within(labels).getByText(LABEL_DISCLOSURE)).toBeInTheDocument();
    expect(within(labels).getByRole("checkbox", { name: "General" })).toBeChecked();
    expect(
      within(labels).getByRole("checkbox", { name: "Confidential › All Employees" }),
    ).not.toBeChecked();
    expect(within(labels).getByRole("checkbox", { name: "Board" })).not.toBeChecked();
    expect(within(labels).getAllByRole("img", { name: "This label encrypts" })).toHaveLength(2);
    expect(within(labels).getByText("not in your list")).toBeInTheDocument();
    expect(within(dialog).getByText(FOOTER.changes(4))).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Copy setup details" })).toBeInTheDocument();

    // Choices change the plan.
    await user.click(
      within(dialog).getByRole("checkbox", { name: "Set up Encrypt and Do Not Forward" }),
    );
    expect(await within(dialog).findByText(FOOTER.changes(2))).toBeInTheDocument();
    await user.click(
      within(labels).getByRole("checkbox", { name: "Confidential › All Employees" }),
    );
    expect(await within(dialog).findByText(FOOTER.changes(4))).toBeInTheDocument();
    expect(backend.planOrgSetup).toHaveBeenLastCalledWith("acct-work", {
      actions: false,
      encryptTemplate: "Encrypt",
      doNotForwardTemplate: "Do Not Forward",
      labelIds: [GENERAL, ALL_EMPLOYEES],
      labelsWithoutLicense: false,
    });

    // Repair: the confirmation sheet, each change and its PowerShell.
    await user.click(within(dialog).getByRole("button", { name: "Repair" }));
    await waitFor(() => expect(title(CONFIRM.title)).toHaveFocus());
    const items = within(dialog)
      .getAllByRole("listitem")
      .filter((li) => li.classList.contains("orgsetup-change"));
    expect(items.map((li) => li.querySelector(".orgsetup-change__title")?.textContent)).toEqual([
      `Create mail flow rule “${STRIP}”`,
      "Start simulation auto-labeling policy “Cumulas Mail: General”",
      "Create auto-labeling policy “Cumulas Mail: All Employees”",
      "Create auto-labeling rule “Cumulas Mail: All Employees (header)”",
    ]);
    expect(within(items[1]!).getByText(LABEL_DISCLOSURE)).toBeInTheDocument();
    expect(within(items[2]!).queryByText(LABEL_DISCLOSURE)).toBeNull();
    const powershell = within(items[0]!).getByText(`Set-TransportRule -Identity '${STRIP}'`);
    expect(powershell).not.toBeVisible();
    await user.click(within(items[0]!).getByText("Show the PowerShell"));
    expect(powershell).toBeVisible();
    expect(within(dialog).getByText(CONFIRM.footer)).toBeInTheDocument();

    // Make these changes: a spinner that Escape doesn't close.
    const release = hold();
    await user.click(within(dialog).getByRole("button", { name: "Make these changes" }));
    expect(await within(dialog).findByText(APPLYING)).toBeInTheDocument();
    // What the browser does with Escape.
    fireEvent(dialog, new Event("cancel", { cancelable: true }));
    expect(within(dialog).getByRole("button", { name: "Close" })).toBeDisabled();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(apply).toHaveBeenCalledWith(
      "acct-work",
      expect.objectContaining({ labelIds: [GENERAL, ALL_EMPLOYEES] }),
      [
        `create:mailFlowRule:${STRIP.toLowerCase()}`,
        "startSimulation:autoLabelingPolicy:cumulas mail: general",
        "create:autoLabelingPolicy:cumulas mail: all employees",
        "create:autoLabelingRule:cumulas mail: all employees (header)",
      ],
    );
    await act(async () => release());

    // Done, naming only what was set up: labels, not encryption.
    await waitFor(() => expect(title(DONE.title)).toHaveFocus());
    expect(
      within(dialog).getAllByText(DONE.done, { selector: ".orgsetup-outcome__line" }),
    ).toHaveLength(3);
    expect(within(dialog).getByText(DONE.notYetListed)).toBeInTheDocument();
    expect(within(dialog).getByText(ADOPTED)).toBeInTheDocument();
    expect(within(dialog).getByText(DONE.checkLabels)).toBeInTheDocument();
    expect(within(dialog).queryByText(DONE.checkEncryption)).toBeNull();
    expect(within(dialog).getByText(DONE.everyoneElse)).toBeInTheDocument();
    expect(within(dialog).queryByRole("button", { name: "Show the PowerShell script" })).toBeNull();
    await user.click(within(dialog).getByText("Done with setup?"));
    expect(within(dialog).getByText(REMOVE_APPROVAL.intro)).toBeVisible();
    const commands = within(dialog).getAllByText(/Remove-MgOauth2PermissionGrant/, {
      selector: "pre",
    });
    // The administrator's own approval, then an approval for everyone, which
    // has no principal.
    expect(commands).toHaveLength(2);
    expect(commands[0]).toHaveTextContent(`principalId eq '${ADMIN_OID}'`);
    expect(within(dialog).getByText(REMOVE_APPROVAL.forEveryone)).toBeVisible();
    expect(commands[1]).toHaveTextContent("consentType eq 'AllPrincipals'");
    expect(within(dialog).getByText(REMOVE_APPROVAL.orDelete)).toBeVisible();

    // Closing forgets the sign-in, and the keyboard goes back.
    const endedBefore = backend.calls.orgSetupEnded;
    await user.click(within(dialog).getAllByRole("button", { name: "Close" }).at(-1)!);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(backend.calls.orgSetupEnded).toBe(endedBefore + 1);
    await waitFor(() => expect(opener).toHaveFocus());
  });

  it("offers only the script without a setup app, with its parts and the portal steps", async () => {
    const backend = new MockBackend();
    // A build without a setup app.
    vi.spyOn(backend, "getOrgSetupInfo").mockResolvedValue({
      automatic: false,
      tenantId: "demo-tenant",
      signedIn: null,
      approvalUrl: null,
      rulesChangedAt: null,
      policiesChangedAt: null,
    });
    const { user, dialog } = await openDialog(backend);
    const writeText = vi.fn(async (_text: string) => undefined);
    vi.stubGlobal("navigator", { ...navigator, clipboard: { writeText } });
    onTestFinished(() => void vi.unstubAllGlobals());

    expect(
      await within(dialog).findByText(
        "This PowerShell script sets up the same rules and policies; it's filled in for acme.example.",
      ),
    ).toBeInTheDocument();
    expect(
      within(dialog).getByRole("region", { name: "Cumulas Mail's permissions" }),
    ).toBeInTheDocument();
    expect(
      within(dialog).queryByRole("button", { name: "Sign in as an administrator" }),
    ).toBeNull();
    const encryption = within(dialog).getByRole("checkbox", { name: "Encryption" });
    const labels = within(dialog).getByRole("checkbox", { name: "Sensitivity labels" });
    expect(encryption).toBeChecked();
    // Without a sign-in, nothing says the license includes automatic labeling.
    expect(labels).not.toBeChecked();
    expect(labels).toHaveAccessibleDescription(SCRIPT_VIEW.labelsNeedPlan);
    expect(within(dialog).getByText(SCRIPT_VIEW.part1)).toBeInTheDocument();
    const pre = await within(dialog).findByLabelText("PowerShell script");
    expect(pre).toHaveTextContent(
      "# Cumulas Mail: encryption and sensitivity labels for acme.example",
    );
    expect(pre).toHaveTextContent(`New-TransportRule -Name '${ENCRYPT}'`);
    expect(pre).not.toHaveTextContent("Connect-IPPSSession");
    expect(within(dialog).queryByRole("heading", { name: SCRIPT_VIEW.portalTitle })).toBeNull();

    await user.click(labels);
    const filled = await within(dialog).findByText(/Connect-IPPSSession/, { selector: "pre" });
    expect(filled).toHaveTextContent(
      `New-AutoSensitivityLabelPolicy -Name 'Cumulas Mail: General'`,
    );
    expect(within(dialog).getByText(SCRIPT_VIEW.part2)).toBeInTheDocument();
    expect(
      within(dialog).getByRole("heading", { name: SCRIPT_VIEW.portalTitle }),
    ).toBeInTheDocument();
    const general = within(dialog).getByRole("region", { name: "General" });
    expect(within(general).getAllByRole("listitem")).toHaveLength(7);
    expect(within(general).getByText("Name: Cumulas Mail: General.")).toBeInTheDocument();
    await user.click(
      within(general).getByRole("button", {
        name: "Copy the pattern (?i)^(2a7f3c10-8b4e-4f6a-a1d2-3b4c5d6e7f80)$",
      }),
    );
    expect(writeText).toHaveBeenLastCalledWith(`(?i)^(${GENERAL})$`);

    await user.click(within(dialog).getByRole("button", { name: "Copy script" }));
    expect(writeText).toHaveBeenLastCalledWith(filled.textContent);
    expect(within(dialog).getByRole("button", { name: "Copied" })).toBeInTheDocument();

    const openLink = vi.spyOn(backend, "openLink").mockResolvedValue(undefined);
    await user.click(within(dialog).getByRole("button", { name: "PowerShell setup in the guide" }));
    expect(openLink).toHaveBeenCalledWith(
      "https://github.com/cumulas-inc/email-client/blob/main/docs/microsoft-365.md#encrypt-and-do-not-forward",
    );
    await user.click(within(dialog).getByRole("button", { name: "Open the guide" }));
    expect(openLink).toHaveBeenLastCalledWith(
      "https://github.com/cumulas-inc/email-client/blob/main/docs/microsoft-365.md#sensitivity-labels-and-encryption-it-admins",
    );
  });

  it("asks for an approval of the setup app, without Email IT", async () => {
    const { backend } = setUpBackend("consent");
    const openLink = vi.spyOn(backend, "openLink").mockResolvedValue(undefined);
    const { user, dialog } = await openDialog(backend);
    await user.click(
      await within(dialog).findByRole("button", { name: "Sign in as an administrator" }),
    );

    await waitFor(() => expect(title(CONSENT.title)).toHaveFocus());
    expect(within(dialog).getByText(CONSENT.message)).toBeInTheDocument();
    expect(within(dialog).getByText(CONSENT.canApprove)).toBeInTheDocument();
    expect(within(dialog).getByText(CONSENT.warning)).toBeInTheDocument();
    expect(within(dialog).getByLabelText("Approval link for your IT administrator")).toHaveValue(
      SETUP_LINK,
    );
    expect(within(dialog).queryByRole("button", { name: "Email IT" })).toBeNull();
    await user.click(within(dialog).getByRole("button", { name: "Open approval page" }));
    expect(openLink).toHaveBeenLastCalledWith(SETUP_LINK);
    await user.click(within(dialog).getByRole("button", { name: "Setup script on the IT page" }));
    expect(openLink).toHaveBeenLastCalledWith(SETUP_APP_IT_PAGE_URL);
    expect(within(dialog).getByRole("button", { name: "Sign in again" })).toBeInTheDocument();
    expect(
      within(dialog).getByRole("button", { name: "Show the PowerShell script" }),
    ).toBeInTheDocument();

    const writeText = vi.fn(async (_text: string) => undefined);
    vi.stubGlobal("navigator", { ...navigator, clipboard: { writeText } });
    onTestFinished(() => void vi.unstubAllGlobals());
    await user.click(within(dialog).getByRole("button", { name: "Copy setup details" }));
    await waitFor(() =>
      expect(writeText).toHaveBeenCalledWith("Cumulas Mail 0.5.0: organization setup details"),
    );
    await waitFor(() => expect(toasts()).toEqual(["Copied the setup details"]));
  });

  it("says why a sign-in failed, with Microsoft's code, and copies the details", async () => {
    const { backend, signIn: signInSpy } = setUpBackend("partial");
    signInSpy.mockRejectedValueOnce({
      code: "provider",
      message:
        "Cumulas Mail - Admin Setup isn't fully registered with Microsoft (AADSTS700016). Use the PowerShell script for now, and send Copy setup details to Cumulas.",
      details: { reason: "setupNotRegistered", aadsts: 700016 },
    });
    const openLink = vi.spyOn(backend, "openLink").mockResolvedValue(undefined);
    const { user, dialog } = await openDialog(backend);
    await user.click(
      await within(dialog).findByRole("button", { name: "Sign in as an administrator" }),
    );

    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "Cumulas Mail - Admin Setup isn't fully registered with Microsoft (AADSTS700016).",
    );
    await user.click(within(dialog).getByRole("button", { name: "What AADSTS700016 means" }));
    expect(openLink).toHaveBeenCalledWith("https://login.microsoftonline.com/error?code=700016");
    expect(within(dialog).getByRole("button", { name: "Copy setup details" })).toBeInTheDocument();
    expect(
      within(dialog).getByRole("button", { name: "Show the PowerShell script" }),
    ).toBeInTheDocument();

    await user.click(within(dialog).getByRole("button", { name: "Try again" }));
    expect(await within(dialog).findByText(SIGNED_IN_AS(ADMIN.username))).toBeInTheDocument();
  });

  it("goes back when the sign-in is cancelled, and stops it when the dialog closes", async () => {
    const { backend, signIn: signInSpy, cancel } = setUpBackend("partial");
    let reject!: (e: unknown) => void;
    signInSpy.mockImplementationOnce(
      () => new Promise((_resolve, rejectWith) => (reject = rejectWith)),
    );
    cancel.mockImplementation(async () => {
      reject({ code: "signInCancelled", message: "Sign-in was cancelled." });
    });
    const { user, dialog } = await openDialog(backend);
    await user.click(
      await within(dialog).findByRole("button", { name: "Sign in as an administrator" }),
    );
    expect(within(dialog).getByText("Finish signing in in your browser.")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(cancel).toHaveBeenCalledTimes(1);
    expect(await within(dialog).findByText(INTRO.what)).toBeInTheDocument();

    signInSpy.mockImplementationOnce(() => new Promise(() => undefined));
    await user.click(within(dialog).getByRole("button", { name: "Sign in as an administrator" }));
    await within(dialog).findByText("Finish signing in in your browser.");
    await user.click(within(dialog).getAllByRole("button", { name: "Close" })[0]!);
    expect(cancel).toHaveBeenCalledTimes(2);
    expect(backend.calls.orgSetupEnded).toBe(1);
  });

  it("starts with the labels unchecked, and offers them anyway, when the license doesn't include them", async () => {
    const { backend } = setUpBackend("unlicensed");
    const { user, dialog } = await openDialog(backend);
    await signIn(user, dialog);

    expect(
      within(statusRow(dialog, "Sensitivity labels", ROW_TITLES.licensing)).getByText(
        ROWS.licensing.notIncluded(EMAIL),
      ),
    ).toBeInTheDocument();
    const labels = within(dialog).getByRole("group", { name: "Labels to set up or repair" });
    for (const box of within(labels).getAllByRole("checkbox", {
      name: /General|Employees|Board/,
    })) {
      expect(box).not.toBeChecked();
    }
    // Nothing of Cumulas Mail's yet: Set up, not Repair.
    expect(within(dialog).getByRole("button", { name: "Set up" })).toBeEnabled();

    await user.click(within(labels).getByRole("checkbox", { name: "General" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      PLAN_PROBLEMS.unlicensed(EMAIL),
    );
    expect(within(dialog).getByRole("button", { name: "Set up" })).toBeDisabled();
    await user.click(within(labels).getByRole("checkbox", { name: "Set up labels anyway" }));
    await waitFor(() => expect(within(dialog).queryByRole("alert")).toBeNull());
    expect(within(dialog).getByRole("button", { name: "Set up" })).toBeEnabled();
  });

  it("says once that the account isn't an administrator of either half", async () => {
    const { backend } = setUpBackend("noRole");
    const { user, dialog } = await openDialog(backend);
    await signIn(user, dialog);

    expect(within(dialog).getByText(NO_ROLE)).toBeInTheDocument();
    expect(within(dialog).queryByRole("region", { name: "Encryption" })).toBeNull();
    expect(
      within(dialog).getByRole("button", { name: "Sign in with another account" }),
    ).toBeInTheDocument();
    expect(within(dialog).getByText(FOOTER.nothing)).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Set up" })).toBeDisabled();
  });

  it("sets up no labels when only Exchange could be read", async () => {
    const { backend } = setUpBackend("exchangeOnly");
    const { user, dialog } = await openDialog(backend);
    await signIn(user, dialog);

    const access = statusRow(dialog, "Sensitivity labels", ROW_TITLES.labelsAccess);
    expect(within(access).getByText(ROWS.labelsAccess.noRole)).toBeInTheDocument();
    expect(
      within(access).getByRole("button", { name: "Sign in with another account" }),
    ).toBeInTheDocument();
    expect(within(dialog).getByText(FOOTER.changes(2))).toBeInTheDocument();
    const labels = within(dialog).getByRole("group", { name: "Labels to set up or repair" });
    await user.click(within(labels).getByRole("checkbox", { name: "General" }));
    await waitFor(() => expect(backend.planOrgSetup).toHaveBeenCalled());
    expect(await within(dialog).findByText(FOOTER.changes(2))).toBeInTheDocument();
  });

  it("counts what can't be changed from here, and says how to fix it", async () => {
    const { backend } = setUpBackend("blocked");
    const { user, dialog } = await openDialog(backend);
    await signIn(user, dialog);

    // Do Not Forward, and the label with a policy already: Encrypt is left out.
    expect(
      within(dialog).getByText(`${FOOTER.changes(3)} ${FOOTER.blocked(1)}`),
    ).toBeInTheDocument();
    const encrypt = statusRow(dialog, "Encryption", "Encrypt rule");
    expect(within(encrypt).getByText(CHIPS.blocked)).toBeInTheDocument();
    const fix = within(encrypt).getByText("How to fix");
    await user.click(fix);
    expect(within(encrypt).getByText("BlindCopyTo: audit@acme.example")).toBeVisible();
    await user.click(within(encrypt).getByRole("button", { name: "Show the PowerShell script" }));
    expect(await within(dialog).findByRole("checkbox", { name: "Encryption" })).toBeChecked();
    // Once signed in, the script covers what was chosen.
    expect(within(dialog).getByRole("checkbox", { name: "Sensitivity labels" })).toBeChecked();
    await user.click(within(dialog).getByRole("button", { name: "Back" }));
    expect(await within(dialog).findByRole("region", { name: "Encryption" })).toBeInTheDocument();
  });

  it("says which changes didn't go through, and offers the script", async () => {
    const { backend } = setUpBackend("failing");
    const { user, dialog } = await openDialog(backend);
    await signIn(user, dialog);
    await user.click(within(dialog).getByRole("button", { name: "Repair" }));
    await user.click(within(dialog).getByRole("button", { name: "Make these changes" }));

    await waitFor(() => expect(title(DONE.someFailed)).toHaveFocus());
    expect(within(dialog).getByText(DONE.failed(LICENSE_MISSING))).toBeInTheDocument();
    expect(
      within(dialog).getAllByText(DONE.done, { selector: ".orgsetup-outcome__line" }),
    ).toHaveLength(3);
    // Encryption went through, so it's in the next steps; labels didn't.
    expect(within(dialog).getByText(DONE.checkEncryption)).toBeInTheDocument();
    expect(within(dialog).queryByText(DONE.checkLabels)).toBeNull();
    await user.click(within(dialog).getByRole("button", { name: "Show the PowerShell script" }));
    expect(await within(dialog).findByLabelText("PowerShell script")).toBeInTheDocument();
  });

  it("keeps what was done when the sign-in ends halfway, and signs in again", async () => {
    const { backend } = setUpBackend("signedOut");
    const { user, dialog } = await openDialog(backend);
    await signIn(user, dialog);
    await user.click(within(dialog).getByRole("button", { name: "Repair" }));
    await user.click(within(dialog).getByRole("button", { name: "Make these changes" }));

    expect(await within(dialog).findByText(DONE.signedOut)).toBeInTheDocument();
    expect(
      within(dialog).getAllByText(DONE.skipped("Your sign-in ended. Sign in again to continue.")),
    ).toHaveLength(3);
    await user.click(within(dialog).getByRole("button", { name: "Sign in" }));
    expect(await within(dialog).findByText(SIGNED_IN_AS(ADMIN.username))).toBeInTheDocument();
  });

  it("signs out, and asks to sign in again when the sign-in ended", async () => {
    const { backend } = setUpBackend("partial");
    const { user, dialog } = await openDialog(backend);
    await signIn(user, dialog);
    await user.click(within(dialog).getByRole("button", { name: "Sign out" }));
    await waitFor(() => expect(title(DIALOG_TITLE)).toHaveFocus());
    expect(within(dialog).getByText(SIGNED_OUT)).toBeInTheDocument();
    expect(backend.calls.orgSetupEnded).toBe(1);

    // Read again after the sign-in ended elsewhere (60 idle minutes).
    await user.click(within(dialog).getByRole("button", { name: "Sign in" }));
    await within(dialog).findByText(SIGNED_IN_AS(ADMIN.username));
    vi.mocked(backend.readOrgSetup).mockRejectedValueOnce(ended());
    await user.click(within(dialog).getByRole("button", { name: "Read again" }));
    expect(await within(dialog).findByText(SIGNED_OUT)).toBeInTheDocument();
  });

  it("links the relay's routing row to Cumulas Signatures", async () => {
    const { backend } = setUpBackend("relay");
    const openLink = vi.spyOn(backend, "openLink").mockResolvedValue(undefined);
    const { user, dialog } = await openDialog(backend);
    await signIn(user, dialog);

    const routing = statusRow(dialog, "Mail routing", "Mail routing");
    expect(within(routing).getByText(CHIPS.differs)).toBeInTheDocument();
    expect(within(routing).getByText(ROWS.routing.cumulasNotInternal)).toBeInTheDocument();
    await user.click(within(routing).getByRole("button", { name: "Open Cumulas Signatures" }));
    expect(openLink).toHaveBeenCalledWith(SIGNATURES_MANAGE);
  });

  it("refuses a plan that went stale, and reviews the fresh one", async () => {
    const { backend, org, apply } = setUpBackend("partial");
    const { user, dialog } = await openDialog(backend);
    await signIn(user, dialog);
    await user.click(within(dialog).getByRole("button", { name: "Repair" }));
    org.changedElsewhere = true;
    await user.click(within(dialog).getByRole("button", { name: "Make these changes" }));

    expect(await within(dialog).findByText(STALE)).toBeInTheDocument();
    expect(await within(dialog).findByText(FOOTER.changes(5))).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Review" }));
    expect(await within(dialog).findByText(`Move mail flow rule “${DNF}”`)).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Make these changes" }));
    await waitFor(() => expect(title(DONE.title)).toBeInTheDocument());
    expect(apply).toHaveBeenCalledTimes(2);
  });

  it("keeps the plan of a fresh read when an older plan answers late", async () => {
    const { backend } = setUpBackend("partial");
    const { user, dialog } = await openDialog(backend);
    await signIn(user, dialog);
    let answer!: (plan: OrgSetupPlan) => void;
    vi.mocked(backend.planOrgSetup).mockImplementationOnce(
      () => new Promise((resolve) => (answer = resolve)),
    );
    await user.click(within(dialog).getByRole("checkbox", { name: "Board" }));
    expect(await within(dialog).findByText("Working out the changes…")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Read again" }));
    expect(await within(dialog).findByText(FOOTER.changes(4))).toBeInTheDocument();
    await act(async () => answer({ changes: [], problems: [], blocked: 0 }));
    expect(within(dialog).getByText(FOOTER.changes(4))).toBeInTheDocument();
    expect(within(dialog).getByRole("checkbox", { name: "Board" })).not.toBeChecked();
  });

  it("closes by itself on Escape, not Settings around it", async () => {
    const user = userEvent.setup();
    const { backend } = setUpBackend("partial");
    const account = (await backend.listAccounts()).find((a) => a.id === "acct-work")!;
    const closeSettings = vi.fn();
    renderWithBackend(
      <Dialog open title="Settings" onClose={closeSettings}>
        <Opener account={account} />
      </Dialog>,
      { backend },
    );
    await user.click(
      screen.getByRole("button", { name: "Set up or check for your organization…" }),
    );
    const dialog = await screen.findByRole("dialog", { name: DIALOG_TITLE });
    fireEvent(dialog, new Event("cancel", { cancelable: true }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: DIALOG_TITLE })).toBeNull());
    expect(closeSettings).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog", { name: "Settings" })).toBeInTheDocument();
    expect(backend.calls.orgSetupEnded).toBe(1);
  });

  it("forgets the setup sign-in when it's left without closing", async () => {
    const { backend } = setUpBackend("partial");
    const { user, dialog, unmount } = await openDialog(backend);
    await signIn(user, dialog);
    unmount();
    expect(backend.calls.orgSetupEnded).toBe(1);
  });
});

// The same dialog against the demo backend's own organizations (?orgsetup=),
// so the dialog and the demo agree on what each scenario reads and does.
describe("Set up or check for your organization, against the demo", () => {
  const CONFIDENTIAL = "Cumulas Mail: Confidential";
  const changeLine = (
    action: keyof typeof ACTION_VERBS,
    object: keyof typeof OBJECT_KINDS,
    name: string,
  ) => `${ACTION_VERBS[action]} ${OBJECT_KINDS[object]} “${name}”`;
  const sheet = (dialog: HTMLElement) =>
    within(dialog)
      .getAllByRole("listitem")
      .filter((li) => li.classList.contains("orgsetup-change"))
      .map((li) => li.querySelector(".orgsetup-change__title")?.textContent);

  /** The dialog on the demo organization `scenario`, signed in when it can be. */
  async function demo(scenario: DemoOrgSetup) {
    const backend = new MockBackend({ orgSetup: scenario });
    const opened = await openDialog(backend);
    return { ...opened, backend };
  }

  it("repairs what the 0.4.0 guide set up, from the sign-in to Done", async () => {
    const { backend, user, dialog, opener } = await demo("partial");
    await signIn(user, dialog);

    const encrypt = statusRow(dialog, "Encryption", ROW_TITLES.encryptRule);
    expect(within(encrypt).getByText(CHIPS.differs)).toBeInTheDocument();
    expect(
      within(encrypt).getByText(
        ROWS.rule.differs(
          "applies to mail from your organization's addresses (it applied only to mail Microsoft 365 had authenticated)",
        ),
      ),
    ).toBeInTheDocument();
    expect(
      within(statusRow(dialog, "Encryption", ROW_TITLES.dnfRule)).getByText(
        ROWS.rule.testMode("Audit"),
      ),
    ).toBeInTheDocument();
    expect(
      within(
        statusRow(dialog, "Mail routing", "Cumulas Mail: Confidential after the signature"),
      ).getByText(ROWS.leftover.unused),
    ).toBeInTheDocument();
    const labels = within(dialog).getByRole("group", { name: "Labels to set up or repair" });
    expect(within(labels).getByRole("checkbox", { name: "Confidential" })).toBeChecked();
    expect(within(labels).getByRole("checkbox", { name: "General" })).not.toBeChecked();
    expect(within(dialog).getByText(FOOTER.changes(3))).toBeInTheDocument();

    await user.click(within(dialog).getByRole("button", { name: "Repair" }));
    await waitFor(() =>
      expect(screen.getByRole("heading", { level: 2, name: CONFIRM.title })).toHaveFocus(),
    );
    expect(sheet(dialog)).toEqual([
      changeLine("change", "mailFlowRule", ENCRYPT),
      changeLine("change", "mailFlowRule", DNF),
      changeLine("turnOn", "autoLabelingPolicy", CONFIDENTIAL),
    ]);
    expect(
      within(dialog).getByText(
        `Set-AutoSensitivityLabelPolicy -Identity '${CONFIDENTIAL}' -Mode 'Enable'`,
      ),
    ).not.toBeVisible();

    await user.click(within(dialog).getByRole("button", { name: "Make these changes" }));
    await waitFor(() =>
      expect(screen.getByRole("heading", { level: 2, name: DONE.title })).toHaveFocus(),
    );
    expect(
      within(dialog).getAllByText(DONE.done, { selector: ".orgsetup-outcome__line" }),
    ).toHaveLength(3);
    expect(within(dialog).getByText(DONE.checkEncryption)).toBeInTheDocument();
    expect(within(dialog).getByText(DONE.checkLabels)).toBeInTheDocument();
    // The organization now has what was confirmed, and the time it changed.
    expect(await backend.getOrgSetupInfo("acct-work")).toMatchObject({
      rulesChangedAt: expect.any(String),
      policiesChangedAt: expect.any(String),
    });
    expect((await backend.readOrgSetup("acct-work")).plan.changes).toEqual([]);

    const writeText = vi.fn(async (_text: string) => undefined);
    vi.stubGlobal("navigator", { ...navigator, clipboard: { writeText } });
    onTestFinished(() => void vi.unstubAllGlobals());
    await user.click(within(dialog).getByRole("button", { name: "Copy setup details" }));
    await waitFor(() =>
      expect(writeText).toHaveBeenCalledWith(
        expect.stringContaining(`Set-AutoSensitivityLabelPolicy`),
      ),
    );

    await user.click(within(dialog).getAllByRole("button", { name: "Close" }).at(-1)!);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(backend.calls.orgSetupEnded).toBe(1);
    await waitFor(() => expect(opener).toHaveFocus());
  });

  it("offers only the script in a build without a setup app", async () => {
    const { dialog } = await demo("unconfigured");
    expect(await within(dialog).findByText(INTRO.scriptOnly(ORG))).toBeInTheDocument();
    expect(
      within(dialog).queryByRole("button", { name: "Sign in as an administrator" }),
    ).toBeNull();
    expect(await within(dialog).findByLabelText("PowerShell script")).toHaveTextContent(
      `# Cumulas Mail: encryption and sensitivity labels for ${ORG}`,
    );
  });

  it("signs in once the setup app's approval page was opened", async () => {
    const { user, dialog } = await demo("consent");
    await user.click(
      await within(dialog).findByRole("button", { name: "Sign in as an administrator" }),
    );
    await waitFor(() =>
      expect(screen.getByRole("heading", { level: 2, name: CONSENT.title })).toHaveFocus(),
    );
    expect(within(dialog).getByLabelText("Approval link for your IT administrator")).toHaveValue(
      mockSetupConsentUrl("demo-tenant"),
    );
    expect(within(dialog).queryByRole("button", { name: "Email IT" })).toBeNull();
    await user.click(within(dialog).getByRole("button", { name: "Open approval page" }));
    await user.click(within(dialog).getByRole("button", { name: "Sign in again" }));
    expect(await within(dialog).findByText(SIGNED_IN_AS(ADMIN.username))).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Set up" })).toBeEnabled();
  });

  it("holds labels back without the license, until Set up labels anyway", async () => {
    const { user, dialog } = await demo("unlicensed");
    await signIn(user, dialog);
    expect(
      within(statusRow(dialog, "Sensitivity labels", ROW_TITLES.licensing)).getByText(
        ROWS.licensing.notIncluded(EMAIL),
      ),
    ).toBeInTheDocument();
    const labels = within(dialog).getByRole("group", { name: "Labels to set up or repair" });
    await user.click(within(labels).getByRole("checkbox", { name: "General" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      PLAN_PROBLEMS.unlicensed(EMAIL),
    );
    await user.click(within(labels).getByRole("checkbox", { name: "Set up labels anyway" }));
    await waitFor(() => expect(within(dialog).queryByRole("alert")).toBeNull());
    expect(within(dialog).getByRole("button", { name: "Set up" })).toBeEnabled();
  });

  it("says once that the account can read neither half", async () => {
    const { user, dialog } = await demo("noRole");
    await user.click(
      await within(dialog).findByRole("button", { name: "Sign in as an administrator" }),
    );
    expect(await within(dialog).findByText(NO_ROLE)).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Set up" })).toBeDisabled();
  });

  it("plans no labels when only Exchange could be read", async () => {
    const { user, dialog } = await demo("exchangeOnly");
    await signIn(user, dialog);
    expect(
      within(statusRow(dialog, "Sensitivity labels", ROW_TITLES.labelsAccess)).getByText(
        ROWS.labelsAccess.noRole,
      ),
    ).toBeInTheDocument();
    expect(within(dialog).getByText(FOOTER.changes(2))).toBeInTheDocument();
    const labels = within(dialog).getByRole("group", { name: "Labels to set up or repair" });
    await user.click(within(labels).getByRole("checkbox", { name: "General" }));
    // Only the rule that blocks label requests from outside joins them.
    expect(await within(dialog).findByText(FOOTER.changes(3))).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Set up" }));
    expect(sheet(dialog)).toEqual([
      changeLine("create", "mailFlowRule", STRIP),
      changeLine("create", "mailFlowRule", ENCRYPT),
      changeLine("create", "mailFlowRule", DNF),
    ]);
  });

  it("counts a rule it can't change from here", async () => {
    const { user, dialog } = await demo("blocked");
    await signIn(user, dialog);
    expect(
      within(dialog).getByText(`${FOOTER.changes(2)} ${FOOTER.blocked(1)}`),
    ).toBeInTheDocument();
    expect(
      within(statusRow(dialog, "Encryption", ROW_TITLES.encryptRule)).getByText(CHIPS.blocked),
    ).toBeInTheDocument();
  });

  it("finds nothing to do in an organization that's set up", async () => {
    const { user, dialog } = await demo("done");
    await signIn(user, dialog);
    expect(within(dialog).getByText(FOOTER.nothing)).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Repair" })).toBeDisabled();
  });

  it("says which changes Purview refused, and offers the script", async () => {
    const { user, dialog } = await demo("failing");
    await signIn(user, dialog);
    await user.click(within(dialog).getByRole("button", { name: "Repair" }));
    await user.click(within(dialog).getByRole("button", { name: "Make these changes" }));
    await waitFor(() =>
      expect(screen.getByRole("heading", { level: 2, name: DONE.someFailed })).toHaveFocus(),
    );
    expect(within(dialog).getByText(DONE.checkEncryption)).toBeInTheDocument();
    expect(within(dialog).queryByText(DONE.checkLabels)).toBeNull();
    expect(
      within(dialog).getByRole("button", { name: "Show the PowerShell script" }),
    ).toBeInTheDocument();
  });

  it("keeps what was made when the sign-in ends halfway", async () => {
    const { user, dialog } = await demo("signedOut");
    await signIn(user, dialog);
    await user.click(within(dialog).getByRole("button", { name: "Set up" }));
    await user.click(within(dialog).getByRole("button", { name: "Make these changes" }));
    expect(await within(dialog).findByText(DONE.signedOut)).toBeInTheDocument();
    expect(within(dialog).getByText(DONE.skipped(SKIPPED_SIGNED_OUT))).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Sign in" }));
    expect(await within(dialog).findByText(FOOTER.changes(1))).toBeInTheDocument();
  });

  it("links the relay's routing row to Cumulas Signatures", async () => {
    const { backend, user, dialog } = await demo("relay");
    const openLink = vi.spyOn(backend, "openLink").mockResolvedValue(undefined);
    await signIn(user, dialog);
    const routing = statusRow(dialog, "Mail routing", ROW_TITLES.routing);
    expect(within(routing).getByText(ROWS.routing.cumulasNotInternal)).toBeInTheDocument();
    await user.click(within(routing).getByRole("button", { name: "Open Cumulas Signatures" }));
    expect(openLink).toHaveBeenCalledWith(SIGNATURES_MANAGE);
  });
});
