// The words of organization setup: the setup dialog, its status rows and
// their chips, and the notes under people's checks. The dialog, its tests and
// the demo backend share them. The core words the rows it sends, and its
// errors, the same way (crates/core/src/orgsetup, crates/core/src/service):
// keep the two in step.

import type {
  OrgSetupAction,
  OrgSetupFix,
  OrgSetupObject,
  OrgSetupRowState,
  OrgSetupSection,
  ProtectAction,
} from "../../../api/types";

// --- Names -----------------------------------------------------------------------

/**
 * The setup app's name, as its Microsoft Entra registration shows it. Keep in
 * step with SETUP_APP_NAME in crates/core/src/auth/admin_setup.rs.
 */
export const SETUP_APP_NAME = "Cumulas Mail - Admin Setup";

/** The start of the name of every rule and policy Cumulas Mail sets up. */
export const OWN_PREFIX = "Cumulas Mail: ";

/**
 * A value as a PowerShell string: in single quotes, with single quotes
 * (typographic ones too) doubled, as the core's script does.
 */
export function psQuote(value: string): string {
  return `'${value.replace(/['‘’‚‛]/g, (q) => q + q)}'`;
}

// --- Entry -----------------------------------------------------------------------

/** The button in Sensitivity labels and encryption that opens the dialog. */
export const ENTRY_BUTTON = "Set up or check for your organization…";
export const DIALOG_TITLE = "Set up or check for your organization";

// --- Buttons ---------------------------------------------------------------------

export const BUTTONS = {
  signInAsAdmin: "Sign in as an administrator",
  signIn: "Sign in",
  signInAgain: "Sign in again",
  signInWithAnother: "Sign in with another account",
  signOut: "Sign out",
  tryAgain: "Try again",
  cancel: "Cancel",
  showScript: "Show the PowerShell script",
  copyScript: "Copy script",
  copy: "Copy",
  openGuide: "Open the guide",
  copySetupDetails: "Copy setup details",
  readAgain: "Read again",
  setUp: "Set up",
  repair: "Repair",
  makeChanges: "Make these changes",
  back: "Back",
  review: "Review",
  close: "Close",
  approveForEveryone: "Approve again for everyone",
  openSignatures: "Open Cumulas Signatures",
  howToFix: "How to fix",
  showPowerShell: "Show the PowerShell",
  labelsAnyway: "Set up labels anyway",
  setUpActions: "Set up Encrypt and Do Not Forward",
  doneWithSetup: "Done with setup?",
  whatApprovingCantFix: "What approving can't fix",
} as const;

// --- Dialog states ---------------------------------------------------------------

export const INTRO = {
  /** With automatic setup: what it does. */
  what: "Create, check and repair the mail flow rules and auto-labeling policies that let people send encrypted and labeled mail from Cumulas Mail. Cumulas Mail changes only rules and policies whose names start with “Cumulas Mail:”, may move other rules down a place, and shows you every change first.",
  who: (org: string) =>
    `Sign in with an administrator account of ${org}: an Exchange Administrator for mail flow rules, a Compliance Administrator for auto-labeling policies, or a Global Administrator for both. If Microsoft shows “Consent on behalf of your organization”, leave it unchecked.`,
  /** Muted, under the two paragraphs. */
  undocumented:
    "This uses an interface of Microsoft 365 that Microsoft doesn't document for other apps, so it may stop working. If anything fails, you get a PowerShell script that does the same.",
  notAdmin: "Not an administrator? Copy the PowerShell script and send it to your IT team.",
  /** Without automatic setup: the script instead. */
  scriptOnly: (org: string) =>
    `This PowerShell script sets up the same rules and policies; it's filled in for ${org}.`,
};

export const SIGNING_IN = "Finish signing in in your browser.";

export const CONSENT = {
  title: `An administrator needs to approve ${SETUP_APP_NAME}`,
  message: `Microsoft 365 needs an administrator to approve ${SETUP_APP_NAME} before you can use it.`,
  canApprove:
    "If you're a Global Administrator or Privileged Role Administrator, you can approve it now: open the approval page and sign in, or select Sign in again and check “Consent on behalf of your organization”. Your browser then shows a page that can't connect; the approval is recorded.",
  warning: `Approving it for everyone lets anyone in your organization use it with their own Exchange rights. Afterwards, require assignment and assign only your IT admins (Enterprise apps → ${SETUP_APP_NAME} → Properties, then Users and groups). To limit it from the start, use the setup script on the IT page instead.`,
  /** Where the approval link ends up. */
  afterApproving:
    "Your browser then shows a page that can't connect. The approval is recorded: return to Cumulas Mail and select Sign in again.",
};

export const READING = (org: string) => `Reading ${org}'s setup…`;

export const SIGNED_IN_AS = (username: string) => `Signed in as ${username}`;

/** Both halves refused for lack of a role: this instead of the rows. */
export const NO_ROLE =
  "This account isn't an Exchange or Compliance administrator. Sign in with another account, or copy the PowerShell script for your IT team.";

export const SECTION_TITLES: Record<OrgSetupSection, string> = {
  approval: "Cumulas Mail's permissions",
  encryption: "Encryption",
  labels: "Sensitivity labels",
  routing: "Mail routing",
};

/** Above the label list, always (and under the first label change). */
export const LABEL_DISCLOSURE =
  "Anyone in your organization can apply a label you set up here, from Cumulas Mail or any mail app that can set the header, whoever the label is published to.";

/** The tag of a label Purview lists that isn't in the account's own list. */
export const NOT_IN_YOUR_LIST = "not in your list";

export const FOOTER = {
  changes: (n: number) => `${n} ${n === 1 ? "change" : "changes"} to make`,
  nothing: "Everything is set up.",
  blocked: (n: number) => `${n} can't be changed from here (see above).`,
};

/** Problems that keep the plan from being made (the core's words). */
export const PLAN_PROBLEMS = {
  unlicensed: (email: string) =>
    `${email}'s license doesn't include automatic labeling, which sending with a label needs. Select “Set up labels anyway” if your organization has it.`,
  stripFirst:
    "Label requests from outside must be blocked first: sign in as an Exchange Administrator or Global Administrator, or run Part 1 of the PowerShell script.",
};

export const CONFIRM = {
  title: "Make these changes in Microsoft 365?",
  footer:
    "New rules can take 30 minutes or more to start working. Auto-labeling policies turn themselves on after their simulation, which can take several hours, and then up to a day.",
  /** Under a change to an auto-labeling policy that is on. */
  backToSimulation:
    "Microsoft 365 may put it back into simulation for a few hours; it turns itself on again.",
};

export const ACTION_VERBS: Record<OrgSetupAction, string> = {
  create: "Create",
  change: "Change",
  turnOn: "Turn on",
  move: "Move",
  startSimulation: "Start simulation",
};

export const OBJECT_KINDS: Record<OrgSetupObject, string> = {
  mailFlowRule: "mail flow rule",
  autoLabelingPolicy: "auto-labeling policy",
  autoLabelingRule: "auto-labeling rule",
};

export const APPLYING = "Making changes… This can take a minute.";

export const STALE =
  "Something changed in Microsoft 365 since you looked, so nothing was changed. Review the changes again.";

export const DONE = {
  title: "Done",
  someFailed: "Some changes didn't go through",
  done: "Done",
  notYetListed: "Done; Microsoft 365 doesn't list it yet",
  failed: (error: string) => `Didn't go through: ${error}`,
  skipped: (why: string) => `Skipped: ${why}`,
  signedOut: "Some changes weren't made because your sign-in ended.",
  /** Next steps, naming only what was set up. */
  checkEncryption: "In about an hour, select Check both under Sensitivity labels and encryption.",
  checkLabels:
    "Auto-labeling policies turn themselves on after their simulation, which can take several hours, and then up to a day to start working: select Check all labels tomorrow.",
  everyoneElse: "Everyone else in your organization checks in their own Cumulas Mail.",
};

/** After Sign out, 60 idle minutes, or a call that found the sign-in ended (the core's words). */
export const SIGNED_OUT = "Your setup sign-in ended. Sign in again to continue.";

// --- Cumulas Mail's own permissions, and removing the setup app -------------------

export const PERMISSIONS = {
  title: "Cumulas Mail's permissions",
  approvesEverything:
    "This approves every permission Cumulas Mail asks for, for everyone in your organization. Permissions people approved for themselves stay. Use this link rather than Grant admin consent in the Entra admin center, which grants only what's listed on the app's registration.",
  cantFix:
    "Not assigned to the app (AADSTS50105): assign people under Enterprise apps → Cumulas Mail → Users and groups. App turned off (AADSTS7000112): turn it back on under Properties. Sign-in rules (AADSTS53000-53003, 53011, 530xxx): your Conditional Access policies. Missing service (AADSTS650052): your subscription. Admin roles in Exchange and Purview: role assignments, not approvals.",
};

export const REMOVE_APPROVAL = {
  intro: `Microsoft keeps your approval of ${SETUP_APP_NAME} until you remove it. If this was a one-off setup, remove it:`,
  /** The command that removes this administrator's approval (both values GUIDs). */
  command: (setupClientId: string, adminObjectId: string) =>
    [
      "Connect-MgGraph -Scopes 'DelegatedPermissionGrant.ReadWrite.All','Application.Read.All'",
      `$sp = Get-MgServicePrincipal -Filter "appId eq '${setupClientId}'"`,
      `Get-MgOauth2PermissionGrant -Filter "clientId eq '$($sp.Id)' and principalId eq '${adminObjectId}'" |`,
      "  ForEach-Object { Remove-MgOauth2PermissionGrant -OAuth2PermissionGrantId $_.Id }",
    ].join("\n"),
  orDelete: `Or delete it for everyone: Entra admin center → Enterprise apps → ${SETUP_APP_NAME} → Properties → Delete. To keep it for later repairs: set Assignment required to Yes, assign only your IT admins under Users and groups, and add a Conditional Access policy for it that requires phishing-resistant MFA or a compliant device.`,
};

// --- Status rows -----------------------------------------------------------------

/** Every chip has words: never color alone. */
export const CHIPS: Record<OrgSetupRowState, string> = {
  ok: "OK",
  missing: "Not set up",
  differs: "Needs repair",
  off: "Turned off",
  waiting: "Waiting",
  blocked: "Can't set up",
  unknown: "Couldn't check",
  info: "Note",
};

/** What a row's fix shows ("setup" rows rely on the footer's Set up or Repair). */
export const FIX_LABELS: Record<OrgSetupFix, string | null> = {
  setup: null,
  approval: BUTTONS.approveForEveryone,
  signatures: BUTTONS.openSignatures,
  manual: BUTTONS.howToFix,
  signIn: BUTTONS.signInWithAnother,
};

/** Row titles by row id (a label's row is titled with the label, a routing rule's with its name). */
export const ROW_TITLES = {
  approval: "Cumulas Mail's permissions",
  setupApp: SETUP_APP_NAME,
  exchangeAccess: "Exchange Online",
  irm: "Message Encryption",
  templates: "Encryption templates",
  encryptRule: "Encrypt rule",
  dnfRule: "Do Not Forward rule",
  labelsAccess: "Security & Compliance",
  stripRule: "Label requests from outside",
  licensing: "Automatic labeling license",
  routing: "Mail routing",
} as const;

/** What the rows say (the core's words). */
export const ROWS = {
  approval: {
    missing: (email: string) => `Reading sensitivity labels isn't approved for ${email}.`,
    refused: (email: string) =>
      `Microsoft 365 refused the label list for ${email}, often because of licensing.`,
    ok: (email: string) =>
      `Approved for ${email}. Cumulas Mail can't see whether it's approved for everyone.`,
  },
  setupApp: (username: string) =>
    `Signed in as ${username}. Cumulas Mail forgets this sign-in when you close this window. Microsoft keeps your approval of ${SETUP_APP_NAME} until you remove it.`,
  /** The access rows, when a half couldn't be read. */
  exchangeAccess: {
    noRole:
      "Your account can't read mail flow rules. Sign in as an Exchange Administrator or Global Administrator.",
    unreachable: (message: string) => `Couldn't reach Exchange Online (${message}).`,
    other: (message: string) => `Couldn't read mail flow rules: ${message}.`,
  },
  labelsAccess: {
    noRole:
      "Your account can't read auto-labeling policies. Sign in as a Compliance Administrator or Global Administrator.",
    unreachable: (message: string) => `Couldn't reach Security & Compliance (${message}).`,
    other: (message: string) => `Couldn't read auto-labeling policies: ${message}.`,
  },
  notAccepted: `Microsoft 365 didn't accept the sign-in for ${SETUP_APP_NAME}. Use the PowerShell script, and send Copy setup details to Cumulas.`,
  irm: {
    ok: "Microsoft Purview Message Encryption is on.",
    off: "Message Encryption is off in Exchange Online, so nothing can be encrypted.",
    offDetail: (email: string) =>
      `Cumulas Mail doesn't change this organization-wide setting. In Exchange Online PowerShell: Set-IRMConfiguration -AzureRMSLicensingEnabled $true -InternalLicensingEnabled $true, then Test-IRMConfiguration -Sender ${psQuote(email)} -Recipient ${psQuote(email)} must end with OVERALL RESULT: PASS.`,
    unknown: (message: string) => `Couldn't read the encryption settings (${message}).`,
  },
  templates: {
    ok: "Templates: Encrypt and Do Not Forward.",
    choose: (missing: string) => `Choose the template for ${missing}.`,
    none: "Exchange lists no rights protection templates.",
  },
  /** Encrypt, Do Not Forward and the strip rule. */
  rule: {
    extras: (parameters: string) =>
      `It has settings Cumulas Mail doesn't set: ${parameters}. Remove them in the Exchange admin center (or delete the rule), then select Read again.`,
    missing: "Not set up.",
    differs: (difference: string) => `Needs repair: ${difference}.`,
    off: "Turned off.",
    testMode: (mode: string) => `In test mode (${mode}): it doesn't encrypt.`,
    conflict: (rule: string, conditions: string) =>
      `“${rule}” comes first and stops later rules for mail it matches (${conditions}). Mail it matches isn't encrypted.`,
    waiting: (minutes: number) =>
      `Set up ${minutes} minutes ago. New rules can take 30 minutes or more to start working.`,
    ok: "Set up.",
    /** The strip rule's placement difference. */
    movesFirst: "moves to the top",
    stripDetail:
      "Removes x-cumulas-label from mail that isn't authenticated mail from inside your organization, so nobody outside can ask for a label.",
  },
  licensing: {
    included: (email: string) => `${email}'s license includes automatic labeling.`,
    pending: (email: string, status: string) =>
      `${email}'s license includes automatic labeling, but Microsoft 365 hasn't finished turning it on (${status}).`,
    includedDetail: (skus: string) =>
      `From: ${skus}. Everyone who sends labeled mail, and the administrator who manages auto-labeling policies, needs a license like it.`,
    notIncluded: (email: string) =>
      `${email}'s license doesn't include automatic labeling, which sending with a label needs.`,
    notIncludedDetail:
      "Microsoft 365 E3 and Business Premium don't include it. It comes with Microsoft 365 E5, or as an add-on such as Microsoft Purview Suite (formerly Microsoft 365 E5 Compliance). Everyone who sends labeled mail, and the administrator who manages auto-labeling policies, needs it. Encrypt and Do Not Forward work with E3 and Business Premium.",
    unknown: (email: string, message: string) => `Couldn't check ${email}'s license (${message}).`,
    unknownDetail:
      "Sending with a label needs automatic labeling: Microsoft 365 E5, or an add-on such as Microsoft Purview Suite.",
  },
  label: {
    notListed: "Purview doesn't list this label.",
    notForEmail: "This label isn't for email.",
    missing: "Not set up.",
    /** `what`: "another rule", "other conditions" or "SharePoint or OneDrive locations". */
    extras: (what: string) =>
      `Its auto-labeling policy has ${what} that Cumulas Mail doesn't set. Remove them in the Microsoft Purview portal, then select Read again.`,
    pendingDeletion: "Microsoft 365 is still deleting its auto-labeling policy. Try again later.",
    off: "Turned off.",
    distributionError: (message: string) =>
      `Microsoft 365 couldn't apply its auto-labeling policy: ${message}.`,
    simulating:
      "In simulation; it turns on by itself an hour after the simulation completes. Simulations can take several hours.",
    simulated: (time: string) => `Simulation complete; it turns on by itself around ${time}.`,
    simulationOnly: "In simulation: it doesn't label anything.",
    distributing: "Microsoft 365 is still applying it.",
    settling: (hours: number) =>
      `Turned on ${hours} hours ago. Auto-labeling policies can take up to a day to start working.`,
    ok: "On.",
    /** `by`: "Guid", "ImmutableId" or "the ID Cumulas Mail sends". */
    matchedBy: (by: string, id: string) => `Matched by ${by} (header value ${id}).`,
    group: (group: string) => `Applies only to mail from members of ${group}.`,
  },
  routing: {
    none: "Mail doesn't go through the Cumulas Signatures relay.",
    cumulasInternal:
      "Mail goes through the Cumulas Signatures relay and counts as internal when it comes back, so mail is signed before it's encrypted.",
    cumulasNotInternal:
      "Mail goes through the Cumulas Signatures relay, and Microsoft 365 treats it as outside mail when it comes back. Encrypt and Do Not Forward act before the relay's rule, so encrypted mail goes out without a signature. Sensitivity labels can't work until mail coming back counts as internal.",
    cumulasUnknown:
      "Mail goes through the Cumulas Signatures relay, but its connector couldn't be read. Encrypt and Do Not Forward act before the relay's rule to be safe.",
  },
  otherRouting: (connector: string, conditions: string | null) =>
    `Mail this rule matches goes out through “${connector}”${conditions ? `, for mail to ${conditions}` : ""}. Cumulas Mail's encryption rules come before it, so that service gets such mail already encrypted and can't add a signature to it. People's checks send mail to themselves, so they don't show whether encryption and labels work for mail this rule routes: send a test to an address it covers and look at what arrives.`,
  leftover: {
    unused: "Left from an earlier setup and not used any more. You can delete it.",
    stillNeeded: (policies: string) =>
      `Still needed: auto-labeling policies from an earlier setup (${policies}) use x-cumulas-label-stamped. Select those labels and Repair them first.`,
  },
};

// --- The script and the portal steps ----------------------------------------------

export const SCRIPT_VIEW = {
  encryption: "Encryption",
  labels: "Sensitivity labels",
  /** Next to Sensitivity labels when the license hint isn't "included". */
  labelsNeedPlan:
    "Sensitivity labels need a plan that includes automatic labeling: Microsoft 365 E5, or an add-on such as Microsoft Purview Suite.",
  part1:
    "Part 1 runs in PowerShell 7.4 or later on Linux, macOS or Windows, or in Windows PowerShell 5.1. On Linux, install PowerShell from Microsoft's packages, run pwsh, then paste.",
  part2:
    "Part 2 (auto-labeling policies) needs Windows: Microsoft doesn't offer Security & Compliance PowerShell on Linux or macOS, and Azure Cloud Shell runs Linux. Without a Windows computer, run Part 1, then follow the Microsoft Purview portal steps below.",
  portalTitle: "Part 2 needs Windows",
  guideLink: "PowerShell setup in the guide",
};

/** One label's Microsoft Purview portal steps, filled in. */
export function portalSteps(steps: {
  labelTitle: string;
  policyName: string;
  ruleName: string;
  pattern: string;
}): string[] {
  return [
    "First run Part 1 of the script (it runs on Linux and macOS): auto-labeling acts on mail by sender address, so label requests from outside must be blocked first.",
    "In the Microsoft Purview portal: Solutions → Information Protection → Policies → Auto-labeling policies → Create auto-labeling policy → Custom policy.",
    `Name: ${steps.policyName}.`,
    "Locations: Exchange email only. Included: All users (or your group: a distribution group or mail-enabled security group).",
    `Rules: Advanced rules → Create rule, name ${steps.ruleName}, condition Header matches patterns: header x-cumulas-label, pattern ${steps.pattern}. No other conditions.`,
    `Label: ${steps.labelTitle}. Under Additional settings for email, select Replace existing labels that have the same or lower priority.`,
    "Policy mode: Run policy in simulation mode, with Automatically turn on policy if not edited after the shortest time offered, or turn it on yourself once the simulation is complete.",
  ];
}

// --- People's checks (Sensitivity labels and encryption) ---------------------------

/** Added to the intro of Sensitivity labels and encryption. */
export const CHECK_WAIT =
  "New rules can take 30 minutes or more to start working, and auto-labeling policies up to a day.";

/** Row texts that differ from the plain ones ("· checked {date}" follows the first). */
export const CHECK_ROWS = {
  otherLabel: "Arrived with another label",
  relayChanged: "Check again to use it",
};

/** Ends the plain, requestIgnored, requestRemoved and relayNotInternal notes. */
export const IT_CAN_FIX =
  " Your IT team can fix this. Copy details for IT gives them what the check found.";

/** The notes under a check's row (before IT_CAN_FIX and the setup-change sentence). */
export const CHECK_NOTES = {
  notApplied:
    "The test message arrived without it, as far as Cumulas Mail can tell. New mail flow rules can take 30 minutes or more to start working, and auto-labeling policies up to a day.",
  requestIgnoredAction: (action: ProtectAction) =>
    `Your organization's mail server got the request (x-cumulas-protect was still on the test message), but no rule acted on it. The rule “Cumulas Mail: ${action === "encrypt" ? "Encrypt" : "Do Not Forward"}” may be missing, turned off, in test mode, or new: rules can take 30 minutes or more to start working.`,
  requestIgnoredLabel:
    "Your organization's mail server got the request (x-cumulas-label was still on the test message), but no auto-labeling policy applied the label. The policy may be missing, still in simulation, not include you, not be in effect yet (allow up to a day), or need a license your plan doesn't have.",
  requestRemoved: (kind: "action" | "label") =>
    `The request was removed on the way, but the message wasn't ${kind === "action" ? "encrypted" : "labeled"}. Usually a rule removed it, such as “Cumulas Mail: ignore label requests from outside” when the message didn't count as from inside your organization.`,
  relayNotInternalAction:
    "Your mail goes through the Cumulas Signatures relay, and Microsoft 365 treats it as outside mail when it comes back. Cumulas Mail's encryption rules then need to come before the relay's rule: Set up or check for your organization does that.",
  relayNotInternalLabel:
    "Your mail goes through the Cumulas Signatures relay, and Microsoft 365 treats it as outside mail when it comes back, so labels can't work yet. Your IT team needs to finish setting up “Count mail coming back from the signature server as mail from inside your organization” in Cumulas Signatures.",
  /** `ago`: how long before the check, such as "10 minutes". */
  tooSoon: (ago: string) =>
    `Your organization's setup changed ${ago} before this check. New rules can take 30 minutes or more to start working, and auto-labeling policies up to a day. Check again later.`,
  /** `title`: the label's title, else OTHER_LABEL_UNNAMED. */
  otherLabel: (title: string, onControl: boolean) =>
    `It arrived labeled ${title} instead.${onControl ? " Your organization puts that label on all mail." : ""}`,
  otherProtection:
    "The test message arrived protected another way (S/MIME, or as a link to Microsoft's encrypted message portal), so Cumulas Mail can't tell whether your organization's rule did it. Open the test message to see.",
  labelUnreadable:
    "The test message arrived encrypted, but Cumulas Mail couldn't read its label. Open the test message to see which label it has.",
  relayChanged:
    "Your organization's mail now goes through the Cumulas Signatures relay, which it didn't when this was checked.",
  /** Added when changedAt is set and the note isn't tooSoon. */
  changedBefore: (ago: string) =>
    ` Your organization's setup changed ${ago} before this check; if that's under an hour (a day for labels), check again later.`,
};

/** The label that arrived instead, when it isn't in the account's list. */
export const OTHER_LABEL_UNNAMED = "a different label of your organization";

/** "Copy details for IT" in Sensitivity labels and encryption, and its toast. */
export const COPY_DETAILS_FOR_IT = "Copy details for IT";
export const COPIED_DETAILS_FOR_IT = "Copied the check's details";

// --- The dialog's own words (no core counterpart) ---------------------------------

export const DIALOG = {
  copiedSetupDetails: "Copied the setup details",
  /** Under a failed sign-in, with the code Microsoft gave. */
  lookUpCode: (aadsts: number) => `What AADSTS${aadsts} means`,
  /** Where the consent state points for approving with assignment first. */
  itPage: "Setup script on the IT page",
  /** Shown while the plan for changed choices is worked out. */
  planning: "Working out the changes…",
  /** Over the label checkboxes. */
  labelsToSetUp: "Labels to set up or repair",
  encryptTemplate: "Template for Encrypt",
  doNotForwardTemplate: "Template for Do Not Forward",
  chooseTemplate: "Choose a template",
  /** The disclosure of a long row detail. */
  moreDetail: "Details",
  /** The script view's read-only script. */
  scriptLabel: "PowerShell script",
  copied: "Copied",
  copyFailed: "Couldn't copy automatically. The text is selected — press Ctrl+C to copy it.",
  /** The portal steps' Copy buttons, read out with what they copy. */
  copyPolicyName: (name: string) => `Copy the policy name ${name}`,
  copyRuleName: (name: string) => `Copy the rule name ${name}`,
  copyPattern: (pattern: string) => `Copy the pattern ${pattern}`,
  /** A read or a change that failed as a whole (not one half of it). */
  readFailed: "Cumulas Mail couldn't read your organization's setup.",
  applyFailed: "Cumulas Mail couldn't make the changes.",
};

/** The IT page's section on the setup app, with the script that approves it with assignment. */
export const SETUP_APP_IT_PAGE_URL = "https://flatpak.cumulas.co/admin-consent/#setup-app";

/** The guide's PowerShell for the rules and auto-labeling policies. */
export const POWERSHELL_GUIDE_URL =
  "https://github.com/cumulas-inc/email-client/blob/main/docs/microsoft-365.md#encrypt-and-do-not-forward";
