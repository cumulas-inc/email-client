// The demo organization as Cumulas Mail's organization setup sees it
// (crates/core/src/orgsetup): what each `?orgsetup=` scenario has set up in
// Exchange Online and Microsoft Purview, the rows, defaults and changes the
// core makes of it, and what making the changes does. The words the dialog
// shows too come from orgsetup/copy.ts; the core's own (change lines,
// results, sign-in errors, the run log) are copied here.

import type {
  BackendError,
  LicenseHintState,
  OrgSetupAction,
  OrgSetupAdmin,
  OrgSetupChange,
  OrgSetupChangeResult,
  OrgSetupChoices,
  OrgSetupLabel,
  OrgSetupObject,
  OrgSetupPlan,
  OrgSetupRow,
  OrgSetupRowState,
  OrgSetupSection,
  OrgSetupStatus,
  RelayPath,
  RmsTemplate,
} from "../types";
import {
  ACTION_VERBS,
  CONSENT,
  CONFIRM,
  LABEL_DISCLOSURE,
  OWN_PREFIX,
  PLAN_PROBLEMS,
  REMOVE_APPROVAL,
  ROWS,
  ROW_TITLES,
  SETUP_APP_NAME,
  SIGNED_OUT,
  psQuote,
} from "../../modules/settings/orgsetup/copy";

/**
 * The demo organization's setup (`?orgsetup=`): what's set up when an
 * administrator signs in, and how signing in and making changes go.
 *
 * - `unconfigured`: a build without a setup app; only the script.
 * - `consent`: signing in needs an administrator's approval of the setup
 *   app, until its approval link is opened; then as `fresh`.
 * - `fresh` (the default): nothing of Cumulas Mail's is set up.
 * - `partial`: what the 0.4.0 guide set up: an Encrypt rule to repair, Do Not
 *   Forward in test mode, a policy left in simulation, a leftover rule.
 * - `relay`: `partial` with mail going through the Cumulas Signatures relay,
 *   which Microsoft 365 treats as outside mail when it comes back.
 * - `unlicensed`: `fresh`, with a license without automatic labeling.
 * - `noRole`: the administrator can read neither half.
 * - `exchangeOnly`: Security & Compliance refuses the administrator.
 * - `blocked`: `partial` with a setting Cumulas Mail doesn't set on Encrypt.
 * - `done`: everything set up.
 * - `failing`: `partial` with two policies; Purview refuses them for a
 *   missing license, after the mail flow rules are done.
 * - `signedOut`: `fresh`; the sign-in ends after the first change.
 */
export type DemoOrgSetup =
  | "unconfigured"
  | "consent"
  | "fresh"
  | "partial"
  | "relay"
  | "unlicensed"
  | "noRole"
  | "exchangeOnly"
  | "blocked"
  | "done"
  | "failing"
  | "signedOut";

/** How a setup sign-in can fail besides needing approval (`failOrgSetupSignIn`). */
export type OrgSetupSignInFailure =
  | "declined"
  | "notAssigned"
  | "otherTenant"
  | "notRegistered"
  | "noExchange"
  | "otherCloud"
  | "timeout";

// --- The setup app -----------------------------------------------------------------

/** Cumulas Mail - Admin Setup's client ID (crates/core/src/auth/admin_setup.rs). */
export const SETUP_CLIENT_ID = "b4f17c55-b9f6-4a84-958f-afe3842da327";
/** What the setup sign-in asks consent for, in order. */
const SETUP_SCOPES = "openid profile offline_access https://outlook.office365.com/Exchange.Manage";
const AUTHORITY = "login.microsoftonline.com";

/**
 * The setup app's approval link for `tenant`, as the core's
 * setup_admin_consent_url builds it (no state; `organizations` without one).
 */
export function mockSetupConsentUrl(tenant?: string): string {
  const query = new URLSearchParams({
    client_id: SETUP_CLIENT_ID,
    scope: SETUP_SCOPES,
    redirect_uri: "http://localhost",
  });
  return `https://${AUTHORITY}/${tenant || "organizations"}/v2.0/adminconsent?${query}`;
}

/** The setup sign-in's error for `failure`, in the core's words. */
export function orgSetupSignInError(failure: OrgSetupSignInFailure, org: string): BackendError {
  switch (failure) {
    case "declined":
      return {
        code: "consentDeclined",
        message: `You declined the permissions ${SETUP_APP_NAME} asks for, so it can't read or change your organization's setup. Sign in again to approve them, or use the PowerShell script.`,
      };
    case "notAssigned":
      return {
        code: "signInBlocked",
        message: `Your organization requires assignment to ${SETUP_APP_NAME}. An administrator assigns you under Enterprise apps → ${SETUP_APP_NAME} → Users and groups.`,
        details: { reason: "notAssigned", aadsts: 50105, app: "setup" },
      };
    case "otherTenant":
      return {
        code: "invalidInput",
        message: `You signed in with an account from another organization. Sign in with an administrator account of ${org}.`,
        details: { reason: "otherTenant" },
      };
    case "notRegistered":
      return {
        code: "provider",
        message: `${SETUP_APP_NAME} isn't fully registered with Microsoft (AADSTS700016). Use the PowerShell script for now, and send Copy setup details to Cumulas.`,
        details: { reason: "setupNotRegistered", aadsts: 700016 },
      };
    case "noExchange":
      return {
        code: "provider",
        message:
          "Microsoft says your organization doesn't have Exchange Online, which setting this up needs (AADSTS500011).",
        details: { reason: "noExchange", aadsts: 500011 },
      };
    case "otherCloud":
      return {
        code: "provider",
        message:
          "Microsoft couldn't find your organization in its public cloud. Organizations in government or other national clouds need to use the PowerShell script.",
        details: { reason: "otherCloud", aadsts: 90002 },
      };
    case "timeout":
      return {
        code: "signInCancelled",
        message: "Sign-in was not completed in time. Try again.",
        details: { reason: "timeout" },
      };
  }
}

/** Signing in needs an administrator's approval of the setup app (`consent`). */
export function setupConsentError(tenant: string): BackendError {
  return {
    code: "adminConsentRequired",
    message: CONSENT.message,
    details: {
      reason: "setupConsent",
      adminConsentUrl: mockSetupConsentUrl(tenant),
      tenant,
      canRequestApproval: false,
      aadsts: 65001,
    },
  };
}

// --- Cumulas Mail's objects --------------------------------------------------------

const ENCRYPT_RULE = `${OWN_PREFIX}Encrypt`;
const DNF_RULE = `${OWN_PREFIX}Do Not Forward`;
const STRIP_RULE = `${OWN_PREFIX}ignore label requests from outside`;
const STAMPED_STRIP_RULE = `${OWN_PREFIX}ignore stamped label requests from outside`;
const LEFTOVER_SUFFIX = " after the signature";
/** Cumulas Signatures' routing rule (read only). */
const RELAY_RULE = "Email signatures: add signature";
/** Where an object of Cumulas Mail's says who manages it (wanted::MARKER). */
const MARKER =
  "Managed by Cumulas Mail (cumulas-mail:v1). Check or repair it from Cumulas Mail: Settings > Accounts > Sensitivity labels and encryption > Set up or check for your organization.";
/** Transport rule names are at most 64 characters; the same cap for policies. */
const MAX_NAME = 64;
const LABEL_HEADER = "x-cumulas-label";
const ENCRYPT_TEMPLATE = "Encrypt";
const DNF_TEMPLATE = "Do Not Forward";

const MINUTE_MS = 60_000;
const HOUR_MS = 60 * MINUTE_MS;
const DAY_MS = 24 * HOUR_MS;

type OwnRule = "strip" | "encrypt" | "dnf";

/** Our three mail flow rules: their names, rows and what they're set to. */
const OWN_RULES: Record<
  OwnRule,
  {
    name: string;
    row: "stripRule" | "encryptRule" | "dnfRule";
    section: OrgSetupSection;
    exceptIfFromScope: string;
    /** The x-cumulas-protect value it acts on; none for the strip rule. */
    pattern: string | null;
    removeHeader: string;
  }
> = {
  strip: {
    name: STRIP_RULE,
    row: "stripRule",
    section: "labels",
    exceptIfFromScope: "InOrganization",
    pattern: null,
    removeHeader: LABEL_HEADER,
  },
  encrypt: {
    name: ENCRYPT_RULE,
    row: "encryptRule",
    section: "encryption",
    exceptIfFromScope: "NotInOrganization",
    pattern: "^encrypt$",
    removeHeader: "x-cumulas-protect",
  },
  dnf: {
    name: DNF_RULE,
    row: "dnfRule",
    section: "encryption",
    exceptIfFromScope: "NotInOrganization",
    pattern: "^do-not-forward$",
    removeHeader: "x-cumulas-protect",
  },
};

/** A mail flow rule of the demo organization: Cumulas Mail's, or another's. */
interface DemoRule {
  name: string;
  guid: string;
  /** Which of ours it is; null for other rules (and leftovers). */
  own: OwnRule | null;
  /** Stops the rules after it, or sends mail out through another service. */
  stops: boolean;
  routes: boolean;
  enabled: boolean;
  mode: string;
  // What Cumulas Mail sets, on ours:
  fromScope: string | null;
  exceptIfFromScope: string | null;
  /** Waits for X-Signature-Stamped, as the 0.4.0 guide's relay recipe did. */
  stamped: boolean;
  template: string | null;
  ruleErrorAction: string | null;
  /** Settings Cumulas Mail never sets (D12): parameter and value. */
  extras: [string, string][];
  /** When setup last changed it from this computer (ms). */
  changedAt: number | null;
}

/** An auto-labeling policy of ours and its rule. */
interface DemoPolicy {
  /** On (Enable), or in simulation (TestWithoutNotifications). */
  mode: "on" | "simulation";
  /** Turns itself on after its simulation (AutoEnableAfter). */
  autoEnable: boolean;
  /** When its simulation completed (ms), if it has. */
  simulated: number | null;
  /** Its rule: "ok", "missing", or the other pattern it matches (as the 0.4.0 guide's). */
  rule: string;
  /** The group whose members' mail it applies to (ExchangeSenderMemberOf). */
  group: string | null;
  /** Something Cumulas Mail doesn't set (D12): "another rule" and the like. */
  blocked: string | null;
  changedAt: number | null;
}

/** A sensitivity label as Purview lists it, with the ID people's Cumulas Mail sends. */
export interface PurviewLabel {
  id: string;
  /** Purview's Name, which names its policy. */
  name: string;
  title: string;
  encrypts: boolean;
}

/** The demo organization's Exchange Online and Purview, as setup reads them. */
export interface DemoTenant {
  kind: DemoOrgSetup;
  /** Whether each half can be read: false when the administrator has no role there. */
  exchange: boolean;
  compliance: boolean;
  templates: RmsTemplate[];
  /** Mail flow rules, by priority. */
  rules: DemoRule[];
  relay: RelayPath;
  /** Auto-labeling policies of ours, by their label's ID. */
  policies: Map<string, DemoPolicy>;
  labels: PurviewLabel[];
  license: LicenseHintState;
  /** The work account's SKUs the license hint names. */
  skus: string[];
}

/** What the service knows without Microsoft: the account and its administrator. */
export interface SetupContext {
  email: string;
  /** The account's labels, in order, once listed. */
  catalog: { id: string; title: string; encrypts: boolean }[] | null;
  needsPermission: boolean;
  refused: boolean;
  admin: OrgSetupAdmin;
  adminObjectId: string;
  /** Cumulas Signatures' page for administrators. */
  signaturesManageUrl: string;
  now: number;
}

let guids = 0;
function newGuid(): string {
  guids += 1;
  return `6e1f0c2a-4b3d-4c5e-8f9a-${String(guids).padStart(12, "0")}`;
}

function rule(name: string, fields: Partial<DemoRule> = {}): DemoRule {
  return {
    name,
    guid: newGuid(),
    own: null,
    stops: false,
    routes: false,
    enabled: true,
    mode: "Enforce",
    fromScope: null,
    exceptIfFromScope: null,
    stamped: false,
    template: null,
    ruleErrorAction: null,
    extras: [],
    changedAt: null,
    ...fields,
  };
}

/** One of ours, as Cumulas Mail sets it up. */
function ownRule(own: OwnRule, fields: Partial<DemoRule> = {}): DemoRule {
  const wanted = OWN_RULES[own];
  return rule(wanted.name, {
    own,
    exceptIfFromScope: wanted.exceptIfFromScope,
    template: own === "encrypt" ? ENCRYPT_TEMPLATE : own === "dnf" ? DNF_TEMPLATE : null,
    ruleErrorAction: own === "strip" ? null : "Defer",
    ...fields,
  });
}

/** A policy of ours, as Cumulas Mail sets it up and Microsoft 365 turned on. */
function onPolicy(fields: Partial<DemoPolicy> = {}): DemoPolicy {
  return {
    mode: "on",
    autoEnable: true,
    simulated: null,
    rule: "ok",
    group: null,
    blocked: null,
    changedAt: null,
    ...fields,
  };
}

/** The 0.4.0 guide's objects, three days old. */
function guideObjects(labels: PurviewLabel[], policyLabels: string[]): Partial<DemoTenant> {
  const confidential = labels.find((l) => l.name === "Confidential");
  const simulated = Date.now() - 3 * DAY_MS;
  return {
    rules: [
      ownRule("strip"),
      // As the guide created it: for mail Microsoft 365 authenticated, once
      // the relay stamped it, sent unencrypted if encryption fails.
      ownRule("encrypt", {
        fromScope: "InOrganization",
        exceptIfFromScope: null,
        stamped: true,
        ruleErrorAction: "Ignore",
      }),
      ownRule("dnf", { mode: "Audit" }),
      rule(`${OWN_PREFIX}${confidential?.name ?? "Confidential"}${LEFTOVER_SUFFIX}`),
    ],
    policies: new Map(
      policyLabels.map((id) => [
        id,
        onPolicy({
          mode: "simulation",
          autoEnable: false,
          simulated,
          group: "cumulas-mail-users@acme.example",
        }),
      ]),
    ),
  };
}

/** The demo organization `kind` names, with Purview listing `labels`. */
export function demoTenant(kind: DemoOrgSetup, labels: PurviewLabel[]): DemoTenant {
  const tenant: DemoTenant = {
    kind,
    exchange: kind !== "noRole",
    compliance: kind !== "noRole" && kind !== "exchangeOnly",
    templates: [
      { name: ENCRYPT_TEMPLATE, id: "1c2d3e4f-5a6b-4c7d-8e9f-0a1b2c3d4e5f" },
      { name: DNF_TEMPLATE, id: "cf5cf348-a8d7-40d5-91ef-a600b88a395d" },
      { name: "Acme - Confidential View Only", id: "7d8e9f0a-1b2c-4d3e-8f4a-5b6c7d8e9f0a" },
    ],
    rules: [],
    relay: "none",
    policies: new Map(),
    labels,
    license: kind === "unlicensed" ? "notIncluded" : "included",
    skus: kind === "unlicensed" ? ["SPE_E3"] : ["SPE_E5"],
  };
  const byName = (name: string) => labels.find((l) => l.name === name)?.id ?? "";
  switch (kind) {
    case "partial":
      return { ...tenant, ...guideObjects(labels, [byName("Confidential")]) };
    case "failing":
      return {
        ...tenant,
        ...guideObjects(labels, [byName("General"), byName("Confidential")]),
      };
    case "blocked": {
      const partial = { ...tenant, ...guideObjects(labels, [byName("Confidential")]) };
      const encrypt = partial.rules.find((r) => r.own === "encrypt")!;
      encrypt.extras = [["BlindCopyTo", "archive@acme.example"]];
      return partial;
    }
    case "relay": {
      const partial = { ...tenant, ...guideObjects(labels, [byName("Confidential")]) };
      const [strip, encrypt, dnf, leftover] = partial.rules as [
        DemoRule,
        DemoRule,
        DemoRule,
        DemoRule,
      ];
      // Encrypt below the relay's rule, which routes mail to the relay.
      partial.rules = [
        strip,
        dnf,
        rule(RELAY_RULE, { stops: true, routes: true }),
        encrypt,
        leftover,
      ];
      partial.relay = "cumulasNotInternal";
      return partial;
    }
    case "done":
      return {
        ...tenant,
        rules: [ownRule("strip"), ownRule("encrypt"), ownRule("dnf")],
        policies: new Map(labels.map((l) => [l.id, onPolicy()])),
      };
    default:
      return tenant;
  }
}

// --- Reading -------------------------------------------------------------------------

/** "Confidential" → "Cumulas Mail: Confidential", by the label's ID when that's too long. */
export function policyNames(label: { id: string; name: string }): {
  policyName: string;
  ruleName: string;
} {
  const fits = (name: string, fallback: string) => (name.length > MAX_NAME ? fallback : name);
  const policyName = fits(`${OWN_PREFIX}${label.name}`, `${OWN_PREFIX}label ${label.id}`);
  return {
    policyName,
    ruleName: fits(`${policyName} (header)`, `${OWN_PREFIX}${label.id} (header)`),
  };
}

/** The header pattern an auto-labeling rule of ours matches. */
export function labelPattern(id: string): string {
  return `(?i)^(${id})$`;
}

function isLeftover(name: string): boolean {
  const lower = name.toLowerCase();
  return (
    lower.startsWith(OWN_PREFIX.toLowerCase()) &&
    (lower.endsWith(LEFTOVER_SUFFIX) || lower === STAMPED_STRIP_RULE.toLowerCase())
  );
}

/** Stops later rules from acting on the mail it matches. */
function blocks(r: DemoRule): boolean {
  return r.enabled && r.mode === "Enforce" && (r.stops || r.routes);
}

/** A line as the middle of a row's sentence: lowercase first, unless it starts with a parameter. */
function rowText(line: string): string {
  const first = line.split(/[ :]/)[0] ?? "";
  if (/[A-Z]/.test(first.slice(1))) return line;
  return line.charAt(0).toLowerCase() + line.slice(1);
}

function minutesSince(at: number, now: number): number {
  return Math.max(1, Math.floor((now - at) / MINUTE_MS));
}

/** "Oct 19, 15:04 UTC". */
function shortTime(at: number): string {
  const text = new Intl.DateTimeFormat("en-US", {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
    timeZone: "UTC",
  }).format(at);
  return `${text} UTC`;
}

type PsValue = string | number | boolean | null | string[] | Record<string, string>;

/** One setting Cumulas Mail makes on a rule, and its line in the confirmation sheet. */
interface Setting {
  param: string;
  value: PsValue;
  line: string;
  /** Only the mode: the rule is in test mode. */
  mode?: boolean;
}

/** What differs on one of our rules from what's wanted (3.7.3), in the core's order. */
function diffRule(r: DemoRule, own: OwnRule, template: string | null): Setting[] {
  const wanted = OWN_RULES[own];
  const protect = own !== "strip";
  const out: Setting[] = [];
  if (r.fromScope) {
    out.push({
      param: "FromScope",
      value: null,
      line: protect
        ? "Applies to mail from your organization's addresses (it applied only to mail Microsoft 365 had authenticated)"
        : `FromScope: none (was ${r.fromScope})`,
    });
  }
  if (r.exceptIfFromScope?.toLowerCase() !== wanted.exceptIfFromScope.toLowerCase()) {
    out.push({
      param: "ExceptIfFromScope",
      value: wanted.exceptIfFromScope,
      line: `ExceptIfFromScope: ${wanted.exceptIfFromScope} (was ${r.exceptIfFromScope ?? "none"})`,
    });
  }
  if (r.stamped) {
    out.push({ param: "HeaderContainsMessageHeader", value: null, line: "" });
    out.push({
      param: "HeaderContainsWords",
      value: null,
      line: "No longer waits for X-Signature-Stamped (from the earlier Cumulas Signatures recipe)",
    });
  }
  if (protect && template && r.template?.toLowerCase() !== template.toLowerCase()) {
    out.push({
      param: "ApplyRightsProtectionTemplate",
      value: template,
      line: `Template: ${template} (was ${r.template ?? "none"})`,
    });
  }
  if (protect && r.ruleErrorAction?.toLowerCase() !== "defer") {
    out.push({
      param: "RuleErrorAction",
      value: "Defer",
      line: "Holds the message and tries again if encryption fails, instead of sending it unencrypted",
    });
  }
  if (r.mode !== "Enforce") {
    out.push({
      param: "Mode",
      value: "Enforce",
      line: `Turns from test mode (${r.mode}) to Enforce`,
      mode: true,
    });
  }
  return out;
}

const ABOVE_RELAY = `Moves above “${RELAY_RULE}”, because mail coming back from the relay doesn't count as internal: encrypted mail then goes out without a signature`;
const AFTER_RELAY = `Moves after “${RELAY_RULE}”, so mail is signed before it's encrypted (mail coming back from the relay now counts as internal)`;
const AFTER_RELAY_ROW = `moves after “${RELAY_RULE}”, so mail is signed before it's encrypted`;
const TO_THE_TOP = "Moves to the top, so no other rule can stop it first";

/** Where one of ours moves (2.5.3): its new priority, its lines and its row's words. */
interface Move {
  to: number;
  lines: string[];
  row: string;
}

/** "Moves above “X”, which stops later rules for mail it matches, so it acts on that mail first". */
function aboveLine(passed: DemoRule[]): string {
  const names = passed.map((r) => `“${r.name}”`);
  const quoted =
    names.length > 1 ? `${names.slice(0, -1).join(", ")} and ${names.at(-1)}` : names[0];
  const stops = passed.every((r) => r.stops);
  const [one, many] = stops
    ? ["stops later rules", "stop later rules"]
    : ["sends mail out through another service", "send mail out through another service"];
  return passed.length === 1
    ? `Moves above ${quoted}, which ${one} for mail it matches, so it acts on that mail first`
    : `Moves above ${quoted}, which ${many} for mail they match, so it acts on that mail first`;
}

/** Where the strip rule goes: above every rule that stops later ones. */
function placeStrip(order: DemoRule[], own: number): Move | null {
  return own > 0 && order.slice(0, own).some(blocks)
    ? { to: 0, lines: [TO_THE_TOP], row: ROWS.rule.movesFirst }
    : null;
}

/**
 * Where Encrypt or Do Not Forward goes (D3): above every rule that stops
 * later ones or routes mail out, except the relay's rule when mail coming
 * back from it counts as internal, which ours then come right after. `own`
 * is its index (`order.length` for one to create).
 */
function placeProtect(order: DemoRule[], own: number, relayInternal: boolean): Move | null {
  const relay = order.findIndex((r) => r.name === RELAY_RULE && blocks(r));
  const useRelay = relay >= 0 && relayInternal;
  const above = order
    .slice(0, own)
    .map((r, i) => [r, i] as const)
    .filter(([r, i]) => blocks(r) && !(useRelay && i === relay));
  const relayBelow = useRelay && relay > own;
  if (!above.length && !relayBelow) return null;
  if (useRelay) {
    const to = relay > own ? relay : relay + 1;
    const lines = relayBelow ? [AFTER_RELAY] : [aboveLine(above.map(([r]) => r))];
    return { to, lines, row: relayBelow ? AFTER_RELAY_ROW : rowText(lines[0]!) };
  }
  const lines: string[] = [];
  if (above.some(([r]) => r.name === RELAY_RULE)) lines.push(ABOVE_RELAY);
  const others = above.filter(([r]) => r.name !== RELAY_RULE).map(([r]) => r);
  if (others.length) lines.push(aboveLine(others));
  return { to: above[0]![1], lines, row: rowText(lines[0]!) };
}

function stripOk(tenant: DemoTenant): boolean {
  const order = tenant.rules;
  const index = order.findIndex((r) => r.own === "strip");
  const strip = order[index];
  return (
    !!strip &&
    !strip.extras.length &&
    strip.enabled &&
    !diffRule(strip, "strip", null).length &&
    !placeStrip(order, index)
  );
}

/** What the status says of a label's policy, after what differs (WP1's policy_state). */
function policyState(
  policy: DemoPolicy,
  now: number,
): { state: OrgSetupRowState; summary: string; fix: OrgSetupRow["fix"] } {
  if (policy.mode === "simulation") {
    if (!policy.autoEnable)
      return { state: "off", summary: ROWS.label.simulationOnly, fix: "setup" };
    return policy.simulated
      ? {
          state: "waiting",
          summary: ROWS.label.simulated(shortTime(policy.simulated + HOUR_MS)),
          fix: null,
        }
      : { state: "waiting", summary: ROWS.label.simulating, fix: null };
  }
  if (policy.changedAt && now - policy.changedAt < DAY_MS) {
    const hours = Math.max(1, Math.floor((now - policy.changedAt) / HOUR_MS));
    return { state: "waiting", summary: ROWS.label.settling(hours), fix: null };
  }
  return { state: "ok", summary: ROWS.label.ok, fix: null };
}

/** The labels a row is shown for: the account's list, then labels only Purview lists. */
function setupLabels(
  tenant: DemoTenant,
  ctx: SetupContext,
): (PurviewLabel & { inCatalog: boolean })[] {
  const purview = new Map(tenant.labels.map((l) => [l.id, l]));
  const listed = (ctx.catalog ?? []).map((l) => ({
    id: l.id,
    name: purview.get(l.id)?.name ?? l.title,
    title: l.title,
    encrypts: l.encrypts,
    inCatalog: true,
  }));
  const others = tenant.labels
    .filter((l) => !listed.some((c) => c.id === l.id))
    .map((l) => ({ ...l, inCatalog: false }));
  return [...listed, ...others];
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

/** The rows of a read (3.6.4), in the core's order. */
function rows(tenant: DemoTenant, ctx: SetupContext, choices: OrgSetupChoices): OrgSetupRow[] {
  const out: OrgSetupRow[] = [];
  const email = ctx.email;

  // Cumulas Mail's permissions.
  out.push(
    ctx.needsPermission
      ? row("approval", "approval", ROW_TITLES.approval, "missing", ROWS.approval.missing(email), {
          fix: "approval",
        })
      : ctx.refused
        ? row(
            "approval",
            "approval",
            ROW_TITLES.approval,
            "blocked",
            ROWS.approval.refused(email),
            {
              fix: "approval",
            },
          )
        : row("approval", "approval", ROW_TITLES.approval, "ok", ROWS.approval.ok(email), {
            fix: "approval",
          }),
  );
  out.push(
    row("setupApp", "approval", ROW_TITLES.setupApp, "info", ROWS.setupApp(ctx.admin.username), {
      detail: REMOVE_APPROVAL.detail(SETUP_CLIENT_ID, ctx.adminObjectId),
    }),
  );

  // Encryption.
  if (!tenant.exchange) {
    out.push(
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
    out.push(row("irm", "encryption", ROW_TITLES.irm, "ok", ROWS.irm.ok));
    out.push(row("templates", "encryption", ROW_TITLES.templates, "ok", ROWS.templates.ok));
  }
  const ruleRow = (own: OwnRule, template: string | null): OrgSetupRow | null => {
    if (!tenant.exchange) return null;
    const wanted = OWN_RULES[own];
    const make = (state: OrgSetupRowState, summary: string, more: Partial<OrgSetupRow> = {}) =>
      row(wanted.row, wanted.section, ROW_TITLES[wanted.row], state, summary, {
        objectName: wanted.name,
        ...more,
      });
    const index = tenant.rules.findIndex((r) => r.own === own);
    const r = tenant.rules[index];
    if (!r) return make("missing", ROWS.rule.missing, { fix: "setup" });
    if (r.extras.length) {
      return make("blocked", ROWS.rule.extras(r.extras.map(([p]) => p).join(", ")), {
        detail: r.extras.map(([p, v]) => `${p}: ${v}`).join("\n"),
        fix: "manual",
      });
    }
    const first = diffRule(r, own, template).find((s) => !s.mode && s.line);
    const move =
      own === "strip"
        ? placeStrip(tenant.rules, index)
        : placeProtect(tenant.rules, index, tenant.relay === "cumulasInternal");
    if (first || move) {
      return make("differs", ROWS.rule.differs(first ? rowText(first.line) : move!.row), {
        fix: "setup",
      });
    }
    if (!r.enabled) return make("off", ROWS.rule.off, { fix: "setup" });
    if (r.mode !== "Enforce") return make("off", ROWS.rule.testMode(r.mode), { fix: "setup" });
    if (r.changedAt && ctx.now - r.changedAt < 30 * MINUTE_MS) {
      return make("waiting", ROWS.rule.waiting(minutesSince(r.changedAt, ctx.now)));
    }
    return make("ok", ROWS.rule.ok, own === "strip" ? { detail: ROWS.rule.stripDetail } : {});
  };
  for (const r of [
    ruleRow("encrypt", choices.encryptTemplate),
    ruleRow("dnf", choices.doNotForwardTemplate),
  ]) {
    if (r) out.push(r);
  }

  // Sensitivity labels.
  if (!tenant.compliance) {
    out.push(
      row("labelsAccess", "labels", ROW_TITLES.labelsAccess, "blocked", ROWS.labelsAccess.noRole, {
        fix: "signIn",
      }),
    );
  }
  const strip = ruleRow("strip", null);
  if (strip) out.push(strip);
  out.push(licensingRow(tenant, email));
  if (tenant.compliance) out.push(...labelRows(tenant, ctx).map(({ row: r }) => r));

  // Mail routing.
  if (tenant.exchange) {
    out.push(
      tenant.relay === "cumulasNotInternal"
        ? row(
            "routing",
            "routing",
            ROW_TITLES.routing,
            "differs",
            ROWS.routing.cumulasNotInternal,
            {
              fix: "signatures",
              link: ctx.signaturesManageUrl,
            },
          )
        : tenant.relay === "cumulasUnknown"
          ? row("routing", "routing", ROW_TITLES.routing, "unknown", ROWS.routing.cumulasUnknown)
          : row("routing", "routing", ROW_TITLES.routing, "ok", ROWS.routing[tenant.relay]),
    );
    for (const leftover of tenant.rules.filter((r) => isLeftover(r.name))) {
      const stamped = leftover.name.toLowerCase() === STAMPED_STRIP_RULE.toLowerCase();
      // Whether the stamped one is still needed takes the auto-labeling policies.
      const unread = stamped && !tenant.compliance;
      out.push(
        row(
          `leftover:${leftover.name}`,
          stamped ? "labels" : "routing",
          leftover.name,
          unread ? "unknown" : "info",
          unread ? ROWS.leftover.unread : ROWS.leftover.unused,
          { objectName: leftover.name },
        ),
      );
    }
  }
  return out;
}

/** What the work account's own licenses say (section 5). */
function licensingRow(tenant: DemoTenant, email: string): OrgSetupRow {
  const make = (state: OrgSetupRowState, summary: string, detail: string) =>
    row("licensing", "labels", ROW_TITLES.licensing, state, summary, { detail });
  const skus = tenant.skus.join(", ");
  switch (tenant.license) {
    case "included":
      return make("ok", ROWS.licensing.included(email), ROWS.licensing.includedDetail(skus));
    case "pending":
      return make(
        "waiting",
        ROWS.licensing.pending(email, "PendingActivation"),
        ROWS.licensing.includedDetail(skus),
      );
    case "notIncluded":
      return make("blocked", ROWS.licensing.notIncluded(email), ROWS.licensing.notIncludedDetail);
    case "unknown":
      return make(
        "unknown",
        ROWS.licensing.unknown(email, "Microsoft 365 didn't say"),
        ROWS.licensing.unknownDetail,
      );
  }
}

/** Each label's row and its state (3.6.4); only when Security & Compliance was read. */
function labelRows(
  tenant: DemoTenant,
  ctx: SetupContext,
): { label: PurviewLabel & { inCatalog: boolean }; row: OrgSetupRow }[] {
  return setupLabels(tenant, ctx).map((label) => {
    const policy = tenant.policies.get(label.id);
    const { policyName } = policyNames(label);
    const make = (state: OrgSetupRowState, summary: string, fix: OrgSetupRow["fix"]) => ({
      label,
      row: row(`label:${label.id}`, "labels", label.title, state, summary, {
        detail:
          ROWS.label.matchedBy("Guid", label.id) +
          (policy?.group ? ` ${ROWS.label.group(policy.group)}` : ""),
        objectName: policyName,
        fix,
      }),
    });
    if (!tenant.labels.some((l) => l.id === label.id)) {
      return make("blocked", ROWS.label.notListed, "manual");
    }
    if (!policy) return make("missing", ROWS.label.missing, "setup");
    if (policy.blocked) return make("blocked", ROWS.label.extras(policy.blocked), "manual");
    const first = labelDiff(label, policy)[0];
    if (first) return make("differs", ROWS.rule.differs(rowText(first)), "setup");
    const { state, summary, fix } = policyState(policy, ctx.now);
    return make(state, summary, fix);
  });
}

/** What differs on a label's policy and its rule, a line each. */
function labelDiff(label: PurviewLabel, policy: DemoPolicy): string[] {
  const pattern = labelPattern(label.id);
  if (policy.rule === "missing") return [`Adds its rule for the header ${LABEL_HEADER}`];
  if (policy.rule !== "ok") {
    return [`Header pattern: ${LABEL_HEADER}: ${pattern} (was ${LABEL_HEADER}: ${policy.rule})`];
  }
  return [];
}

/** The choices a read starts with (3.6.4). */
function defaults(tenant: DemoTenant, ctx: SetupContext): OrgSetupChoices {
  const template = (usual: string, own: OwnRule) =>
    tenant.templates.some((t) => t.name === usual)
      ? usual
      : (tenant.rules.find((r) => r.own === own)?.template ?? null);
  return {
    actions: tenant.exchange,
    encryptTemplate: tenant.exchange ? template(ENCRYPT_TEMPLATE, "encrypt") : null,
    doNotForwardTemplate: tenant.exchange ? template(DNF_TEMPLATE, "dnf") : null,
    labelIds: tenant.compliance
      ? setupLabels(tenant, ctx)
          .filter((l) => tenant.policies.has(l.id))
          .map((l) => l.id)
      : [],
    labelsWithoutLicense: false,
  };
}

/** What a read gives: the rows, the labels, the defaults and the plan for them. */
export function orgSetupStatus(tenant: DemoTenant, ctx: SetupContext): OrgSetupStatus {
  const choices = defaults(tenant, ctx);
  const states = new Map(labelRows(tenant, ctx).map(({ label, row }) => [label.id, row.state]));
  return {
    readAt: new Date(ctx.now).toISOString(),
    admin: { ...ctx.admin },
    rows: rows(tenant, ctx, choices),
    labels: setupLabels(tenant, ctx).map((l): OrgSetupLabel => ({
      id: l.id,
      title: l.title,
      inCatalog: l.inCatalog,
      encrypts: l.encrypts,
      forEmail: tenant.compliance ? true : null,
      state: (tenant.compliance && states.get(l.id)) || "unknown",
    })),
    templates: tenant.exchange ? tenant.templates.map((t) => ({ ...t })) : [],
    relay: tenant.exchange ? tenant.relay : "none",
    license: tenant.license,
    exchangeReadable: tenant.exchange,
    labelsReadable: tenant.compliance,
    noRole: !tenant.exchange && !tenant.compliance,
    anyOurs:
      (tenant.exchange && tenant.rules.some((r) => r.name.startsWith(OWN_PREFIX))) ||
      (tenant.compliance && tenant.policies.size > 0),
    defaults: choices,
    plan: planOrgSetup(tenant, ctx, choices).plan,
  };
}

// --- Planning ------------------------------------------------------------------------

/** One call a change makes, as Microsoft 365's cmdlet takes it. */
export interface PlannedCall {
  cmdlet: string;
  params: [string, PsValue][];
  /** The host it goes to. */
  host: "outlook.office365.com" | "eur03b.ps.compliance.protection.outlook.com";
}

/** A change as the core plans it, and what making it does to the demo organization. */
export interface Planned {
  change: OrgSetupChange;
  calls: PlannedCall[];
  /** The label it's for: it needs the strip rule first (D11). */
  labelId: string | null;
  make: (tenant: DemoTenant, now: number) => void;
}

function psValue(value: PsValue): string {
  if (value === null) return "$null";
  if (typeof value === "boolean") return value ? "$true" : "$false";
  if (typeof value === "number") return String(value);
  if (typeof value === "string") return psQuote(value);
  if (Array.isArray(value)) return `@(${value.map(psQuote).join(", ")})`;
  const entries = Object.entries(value).filter(([key]) => !key.startsWith("@"));
  return `@{ ${entries.map(([k, v]) => `${psQuote(k)} = ${psQuote(v)}`).join("; ")} }`;
}

/**
 * A call's parameters in the core's order: Identity and Name first, then the
 * others by name (invoke::ordered_params over its sorted map).
 */
function orderedParams(call: PlannedCall): [string, PsValue][] {
  const first = ["Identity", "Name"];
  const rest = call.params
    .filter(([key]) => !first.includes(key))
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
  return [...first.flatMap((name) => call.params.filter(([key]) => key === name)), ...rest];
}

/** One call as PowerShell, Identity or Name first (script::render_call). */
export function renderCall(call: PlannedCall): string {
  return [
    call.cmdlet,
    ...orderedParams(call).map(([key, value]) => `-${key} ${psValue(value)}`),
  ].join(" ");
}

const EXCHANGE = "outlook.office365.com" as const;
const COMPLIANCE = "eur03b.ps.compliance.protection.outlook.com" as const;

function planned(
  action: OrgSetupAction,
  object: OrgSetupObject,
  name: string,
  details: string[],
  calls: PlannedCall[],
  make: Planned["make"],
  labelId: string | null = null,
): Planned {
  return {
    change: {
      id: `${action}:${object}:${name.toLowerCase()}`,
      action,
      object,
      name,
      details,
      powershell: calls.map(renderCall).join("\n"),
    },
    calls,
    labelId,
    make,
  };
}

/** The changes for one of our mail flow rules; `order` is the order as planned so far. */
function planRule(
  own: OwnRule,
  order: DemoRule[],
  relayInternal: boolean,
  template: string,
): Planned[] {
  const wanted = OWN_RULES[own];
  const index = order.findIndex((r) => r.own === own);
  const existing = order[index];
  if (!existing) {
    const move =
      own === "strip"
        ? { to: 0, lines: ["Goes to the top of the list"], row: "" }
        : placeProtect(order, order.length, relayInternal);
    const params: [string, PsValue][] = [
      ["Name", wanted.name],
      ["ExceptIfFromScope", wanted.exceptIfFromScope],
    ];
    if (wanted.pattern) {
      params.push(["HeaderMatchesMessageHeader", "x-cumulas-protect"]);
      params.push(["HeaderMatchesPatterns", [wanted.pattern]]);
      params.push(["ApplyRightsProtectionTemplate", template]);
    }
    params.push(["RemoveHeader", wanted.removeHeader]);
    if (own !== "strip") params.push(["RuleErrorAction", "Defer"]);
    if (move) params.push(["Priority", move.to]);
    params.push(["Mode", "Enforce"], ["Comments", MARKER]);
    const details = [
      own === "strip"
        ? "Removes x-cumulas-label from mail that isn't authenticated mail from inside your organization"
        : own === "encrypt"
          ? `Encrypts mail that asks for it with the template “${template}”`
          : `Applies the template “${template}” to mail that asks for Do Not Forward`,
    ];
    if (own !== "strip") {
      details.push(
        "Holds the message and tries again if encryption fails, instead of sending it unencrypted",
      );
    }
    details.push(...(move?.lines ?? []).map((l) => l.replace(/^Moves/, "Goes")));
    // Planned in its place, so the next rule's place counts it.
    const placeholder = ownRule(own, { template: own === "strip" ? null : template });
    order.splice(move ? move.to : order.length, 0, placeholder);
    return [
      planned(
        "create",
        "mailFlowRule",
        wanted.name,
        details,
        [{ cmdlet: "New-TransportRule", params, host: EXCHANGE }],
        (tenant, now) => {
          const created = { ...placeholder, guid: newGuid(), changedAt: now };
          tenant.rules.splice(move ? move.to : tenant.rules.length, 0, created);
        },
      ),
    ];
  }

  const out: Planned[] = [];
  const settings = diffRule(existing, own, own === "strip" ? null : template);
  const find = (tenant: DemoTenant) => tenant.rules.find((r) => r.guid === existing.guid)!;
  if (settings.length) {
    out.push(
      planned(
        "change",
        "mailFlowRule",
        wanted.name,
        settings.filter((s) => s.line).map((s) => s.line),
        [
          {
            cmdlet: "Set-TransportRule",
            params: [
              ["Identity", existing.guid],
              ...settings.map((s): [string, PsValue] => [s.param, s.value]),
              ["Comments", MARKER],
            ],
            host: EXCHANGE,
          },
        ],
        (tenant, now) => {
          Object.assign(find(tenant), {
            fromScope: null,
            exceptIfFromScope: wanted.exceptIfFromScope,
            stamped: false,
            template: own === "strip" ? null : template,
            ruleErrorAction: own === "strip" ? null : "Defer",
            mode: "Enforce",
            changedAt: now,
          });
        },
      ),
    );
  }
  if (!existing.enabled) {
    out.push(
      planned(
        "turnOn",
        "mailFlowRule",
        wanted.name,
        ["Turns it on"],
        [{ cmdlet: "Enable-TransportRule", params: [["Identity", existing.guid]], host: EXCHANGE }],
        (tenant, now) => Object.assign(find(tenant), { enabled: true, changedAt: now }),
      ),
    );
  }
  const move =
    own === "strip" ? placeStrip(order, index) : placeProtect(order, index, relayInternal);
  if (move) {
    order.splice(index, 1);
    order.splice(move.to, 0, existing);
    out.push(
      planned(
        "move",
        "mailFlowRule",
        wanted.name,
        move.lines,
        [
          {
            cmdlet: "Set-TransportRule",
            params: [
              ["Identity", existing.guid],
              ["Priority", move.to],
            ],
            host: EXCHANGE,
          },
        ],
        (tenant, now) => {
          const moved = find(tenant);
          tenant.rules.splice(tenant.rules.indexOf(moved), 1);
          tenant.rules.splice(move.to, 0, moved);
          moved.changedAt = now;
        },
      ),
    );
  }
  return out;
}

/** The changes for one chosen label's auto-labeling policy and its rule (3.7.4). */
function planLabel(label: PurviewLabel, policy: DemoPolicy | undefined): Planned[] {
  const { policyName, ruleName } = policyNames(label);
  const pattern = labelPattern(label.id);
  const header: Record<string, string> = {
    "@odata.type": "#Exchange.GenericHashTable",
    [LABEL_HEADER]: pattern,
  };
  const newRule = (): PlannedCall => ({
    cmdlet: "New-AutoSensitivityLabelRule",
    params: [
      ["Name", ruleName],
      ["Policy", policyName],
      ["Workload", "Exchange"],
      ["HeaderMatchesPatterns", header],
    ],
    host: COMPLIANCE,
  });
  const simulate = (): PlannedCall => ({
    cmdlet: "Set-AutoSensitivityLabelPolicy",
    params: [
      ["Identity", policyName],
      ["StartSimulation", true],
      ["AutoEnableAfter", "01:00:00"],
    ],
    host: COMPLIANCE,
  });
  const simulationLine =
    "Starts its simulation; it turns itself on an hour after the simulation completes";
  const id = label.id;
  const update = (fields: Partial<DemoPolicy>) => (tenant: DemoTenant, now: number) => {
    const current = tenant.policies.get(id);
    if (current) Object.assign(current, { ...fields, changedAt: now });
  };

  if (!policy) {
    return [
      planned(
        "create",
        "autoLabelingPolicy",
        policyName,
        [
          `Applies the label ${label.title} to mail that asks for it`,
          "Covers email from everyone in your organization",
          "Starts in simulation",
        ],
        [
          {
            cmdlet: "New-AutoSensitivityLabelPolicy",
            params: [
              ["Name", policyName],
              ["ApplySensitivityLabel", id],
              ["ExchangeLocation", ["All"]],
              ["OverwriteLabel", true],
              ["Mode", "TestWithoutNotifications"],
              ["Comment", MARKER],
            ],
            host: COMPLIANCE,
          },
        ],
        (tenant, now) =>
          tenant.policies.set(
            id,
            onPolicy({ mode: "simulation", autoEnable: false, rule: "missing", changedAt: now }),
          ),
        id,
      ),
      planned(
        "create",
        "autoLabelingRule",
        ruleName,
        [`Matches the header ${LABEL_HEADER} with ${pattern}`],
        [newRule()],
        update({ rule: "ok" }),
        id,
      ),
      planned(
        "startSimulation",
        "autoLabelingPolicy",
        policyName,
        [simulationLine],
        [simulate()],
        update({ autoEnable: true, simulated: null }),
        id,
      ),
    ];
  }

  const out: Planned[] = [];
  const on = policy.mode === "on";
  const backToSimulation = on ? [CONFIRM.backToSimulation] : [];
  if (policy.rule === "missing") {
    out.push(
      planned(
        "create",
        "autoLabelingRule",
        ruleName,
        [`Matches the header ${LABEL_HEADER} with ${pattern}`, ...backToSimulation],
        [newRule()],
        update({ rule: "ok" }),
        id,
      ),
    );
  } else if (policy.rule !== "ok") {
    out.push(
      planned(
        "change",
        "autoLabelingRule",
        ruleName,
        [...labelDiff(label, policy), ...backToSimulation],
        [
          {
            cmdlet: "Set-AutoSensitivityLabelRule",
            params: [
              ["Identity", ruleName],
              ["HeaderMatchesPatterns", header],
            ],
            host: COMPLIANCE,
          },
        ],
        update({ rule: "ok" }),
        id,
      ),
    );
  }
  const edited = out.length > 0;
  // A policy that's on stays on (Microsoft 365 may put an edited one back
  // into simulation); one turning itself on after its simulation waits.
  if (on || (!edited && policy.autoEnable)) return out;
  if (!edited && policy.simulated) {
    out.push(
      planned(
        "turnOn",
        "autoLabelingPolicy",
        policyName,
        ["Turns it on: its simulation is complete"],
        [
          {
            cmdlet: "Set-AutoSensitivityLabelPolicy",
            params: [
              ["Identity", policyName],
              ["Mode", "Enable"],
            ],
            host: COMPLIANCE,
          },
        ],
        update({ mode: "on" }),
        id,
      ),
    );
  } else {
    out.push(
      planned(
        "startSimulation",
        "autoLabelingPolicy",
        policyName,
        [simulationLine],
        [simulate()],
        update({ autoEnable: true, simulated: null }),
        id,
      ),
    );
  }
  return out;
}

/**
 * Why these choices can't be planned, in the core's words (wanted::validate):
 * a label Purview doesn't list, a template Exchange doesn't. Null when they can.
 */
export function invalidChoice(
  tenant: DemoTenant,
  ctx: SetupContext,
  choices: OrgSetupChoices,
): string | null {
  for (const id of choices.labelIds) {
    const lower = id.trim().toLowerCase();
    const guid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(lower);
    const listed =
      ctx.catalog?.some((l) => l.id === lower) ||
      (tenant.compliance && tenant.labels.some((l) => l.id === lower));
    if (!guid || !listed) return "That label isn't one Purview lists.";
  }
  for (const template of [choices.encryptTemplate, choices.doNotForwardTemplate]) {
    if (template && tenant.exchange && !tenant.templates.some((t) => t.name === template.trim())) {
      return "That template isn't one Exchange lists.";
    }
  }
  return null;
}

/**
 * The changes `choices` make (3.7.5): the strip rule, Encrypt, Do Not
 * Forward, then each chosen label in the list's order; problems that keep
 * them from being made; and how many objects can't be changed from here.
 */
export function planOrgSetup(
  tenant: DemoTenant,
  ctx: SetupContext,
  choices: OrgSetupChoices,
): { plan: OrgSetupPlan; planned: Planned[] } {
  const problems: string[] = [];
  const out: Planned[] = [];
  let blocked = 0;
  const labelIds = [...new Set(choices.labelIds.map((id) => id.trim().toLowerCase()))];
  let labelsAllowed = labelIds.length > 0;
  if (labelsAllowed && tenant.license === "notIncluded" && !choices.labelsWithoutLicense) {
    problems.push(PLAN_PROBLEMS.unlicensed(ctx.email));
    labelsAllowed = false;
  }
  const encryptTemplate = choices.encryptTemplate?.trim() || null;
  const dnfTemplate = choices.doNotForwardTemplate?.trim() || null;
  if (choices.actions && tenant.exchange) {
    if (!encryptTemplate) problems.push(ROWS.templates.choose("Encrypt"));
    if (!dnfTemplate) problems.push(ROWS.templates.choose("Do Not Forward"));
  }

  if (tenant.exchange) {
    // As the rules are planned to stand: moves and new rules shift the rest.
    const order = tenant.rules.slice();
    const relayInternal = tenant.relay === "cumulasInternal";
    for (const own of ["strip", "encrypt", "dnf"] as const) {
      const covered =
        own === "strip" ? labelsAllowed : choices.actions && !!encryptTemplate && !!dnfTemplate;
      if (!covered) continue;
      if (order.find((r) => r.own === own)?.extras.length) {
        blocked += 1;
        continue;
      }
      const template = own === "encrypt" ? encryptTemplate : dnfTemplate;
      out.push(...planRule(own, order, relayInternal, template ?? ""));
    }
  }

  const strip = tenant.rules.find((r) => r.own === "strip");
  if (labelsAllowed && (!tenant.exchange || !!strip?.extras.length)) {
    problems.push(PLAN_PROBLEMS.stripFirst);
    labelsAllowed = false;
  }
  const stripPlanned = out.some((p) => p.change.name === STRIP_RULE);
  const labelsPlannable = labelsAllowed && (stripOk(tenant) || stripPlanned);
  if (tenant.compliance) {
    let first = true;
    for (const label of setupLabels(tenant, ctx).filter((l) => labelIds.includes(l.id))) {
      const policy = tenant.policies.get(label.id);
      if (policy?.blocked) {
        blocked += 1;
        continue;
      }
      if (!labelsPlannable) continue;
      const changes = planLabel(label, policy);
      if (first && changes[0]) {
        changes[0].change.details.unshift(LABEL_DISCLOSURE);
        first = false;
      }
      out.push(...changes);
    }
  }
  return { plan: { changes: out.map((p) => p.change), problems, blocked }, planned: out };
}

// --- Applying ------------------------------------------------------------------------

/** What Purview says to an organization without automatic labeling (`failing`). */
const LICENSE_REFUSED =
  "The tenant doesn't have the license required to create or change auto-labeling policies";
const LICENSE_REQUIRED = `Microsoft 365 says a license is missing: both the administrator who signs in and the people who send need a plan that includes automatic labeling. Microsoft said: ${LICENSE_REFUSED}.`;

/** What making the planned changes did, as apply reports it before the read-back. */
export interface MadeChanges {
  changes: OrgSetupChangeResult[];
  signedOut: boolean;
  /** Whether mail flow rules or auto-labeling objects changed (`orgsetup.changed`). */
  rulesChanged: boolean;
  policiesChanged: boolean;
}

/**
 * Makes `planned` in order (3.8): `failing` refuses its auto-labeling
 * changes for a missing license, `signedOut` ends the sign-in after the
 * first change. A label change waits for a working strip rule (D11).
 */
export function makeChanges(
  tenant: DemoTenant,
  planned: Planned[],
  now: number,
  log: SetupLog,
): MadeChanges {
  const made: MadeChanges = {
    changes: [],
    signedOut: false,
    rulesChanged: false,
    policiesChanged: false,
  };
  let labelsStopped: string | null = null;
  for (const [i, p] of planned.entries()) {
    const result = (outcome: OrgSetupChangeResult["outcome"], message: string | null = null) =>
      made.changes.push({ change: p.change, outcome, message });
    if (made.signedOut) {
      result("skipped", SIGNED_OUT);
      continue;
    }
    if (p.labelId && labelsStopped) {
      result("skipped", labelsStopped);
      continue;
    }
    if (p.labelId && !stripOk(tenant)) {
      result("skipped", PLAN_PROBLEMS.stripFirst);
      continue;
    }
    if (p.labelId && tenant.kind === "failing") {
      log.call(now, p, 400, `LicenseRequired · "${LICENSE_REFUSED}"`);
      result("failed", LICENSE_REQUIRED);
      labelsStopped = LICENSE_REQUIRED;
      continue;
    }
    p.make(tenant, now);
    log.call(now, p, 200);
    if (p.change.object === "mailFlowRule") made.rulesChanged = true;
    else made.policiesChanged = true;
    // Security & Compliance lists a new auto-labeling rule only after a while.
    result(
      p.change.object === "autoLabelingRule" && p.change.action === "create"
        ? "notYetListed"
        : "done",
    );
    if (tenant.kind === "signedOut" && i === 0) made.signedOut = true;
  }
  return made;
}

// --- The run log ("Copy setup details") ----------------------------------------------

function clock(at: number): string {
  return `${new Date(at).toISOString().slice(11, 19)} UTC`;
}

/**
 * What the core's run log keeps, in the shape of its "Copy setup details"
 * text (3.9): its header stays after the sign-in ends; the entries start
 * again at the next sign-in.
 */
export class SetupLog {
  private header: { tenant: string; admin: string | null } | null = null;
  private entries: string[] = [];

  /** A sign-in attempt: clears what the last session logged. */
  signIn(at: number, tenant: string, outcome: string): void {
    this.header = { tenant, admin: null };
    this.entries = [
      `${clock(at)}  Sign-in: ${AUTHORITY}/${tenant}, scope "${SETUP_SCOPES}"`,
      `  ${outcome}`,
    ];
  }

  signedIn(admin: string): void {
    if (this.header) this.header.admin = admin;
  }

  note(at: number, text: string): void {
    this.entries.push(`${clock(at)}  ${text}`);
  }

  /** What a read of `tenant` asked and got. */
  read(at: number, tenant: DemoTenant, email: string): void {
    const exchange = tenant.exchange
      ? `200 · 412 ms · ${tenant.rules.length} ${tenant.rules.length === 1 ? "result" : "results"}`
      : "403 · 96 ms";
    this.entries.push(
      `${clock(at)}  Read mail flow rules`,
      `  Get-TransportRule (ResultSize) on ${EXCHANGE} · anchor UPN · ${exchange}`,
    );
    if (!tenant.exchange) {
      this.entries.push(
        `  Error: Forbidden · "The term 'Get-TransportRule' is not recognized as a name of a cmdlet."`,
      );
    }
    const policies = tenant.policies.size;
    this.entries.push(
      `${clock(at)}  Read auto-labeling policies`,
      tenant.compliance
        ? `  Get-AutoSensitivityLabelPolicy on ${COMPLIANCE} · anchor UPN · 200 · 655 ms · ${policies} ${policies === 1 ? "result" : "results"}`
        : `  Get-AutoSensitivityLabelPolicy on ${COMPLIANCE} · anchor UPN · 403 · 120 ms`,
    );
    if (!tenant.compliance) {
      this.entries.push(
        `  Error: Forbidden · "The term 'Get-AutoSensitivityLabelPolicy' is not recognized as a name of a cmdlet."`,
      );
    }
    this.entries.push(
      `Licensing (from ${email}): ${tenant.skus.join(", ")} · automatic labeling: ${tenant.license === "included" ? "included" : "not included"}`,
    );
    if (tenant.exchange) {
      this.entries.push(
        "",
        "Mail flow rules, by priority:",
        ...(tenant.rules.length ? [] : ["  none"]),
        ...tenant.rules.map((r, i) => {
          const flags = [
            r.enabled ? "Enabled" : "Disabled",
            r.mode,
            ...(r.stops ? ["stops later rules"] : []),
            ...(r.routes ? ["routes mail out"] : []),
          ];
          return `  ${i}  ${r.name} · ${flags.join(" · ")}`;
        }),
        "",
      );
    }
  }

  /** One change's call, with the values of ours (every call apply makes is on ours). */
  call(at: number, p: Planned, status: number, error?: string): void {
    for (const call of p.calls) {
      const params = orderedParams(call)
        .map(([key, value]) => `${key}=${psValue(value)}`)
        .join(", ");
      this.entries.push(
        `${clock(at)}  ${ACTION_VERBS[p.change.action]} “${p.change.name}”`,
        `  ${call.cmdlet} (${params}) on ${call.host} · anchor UPN · ${status} · 380 ms`,
      );
      if (error) this.entries.push(`  Error: ${error}`);
    }
  }

  /** "Copy setup details": empty until a sign-in was tried. */
  text(version: string): string {
    if (!this.header) return "";
    return [
      `Cumulas Mail ${version}: organization setup details`,
      "These details include no passwords or tokens. They name your organization's tenant, the administrator who signed in, and the names and settings of Cumulas Mail's own rules and policies. Other rules appear by name and order only.",
      "",
      `Tenant: ${this.header.tenant}`,
      `Setup app: ${SETUP_APP_NAME} (${SETUP_CLIENT_ID})`,
      `Signed in as: ${this.header.admin ?? "nobody yet"}`,
      "",
      ...this.entries,
      "",
    ].join("\n");
  }
}
