import { isTauri } from "@tauri-apps/api/core";

import type { Backend } from "./backend";
import {
  type DemoOrgSetup,
  type DemoProtection,
  type DemoPush,
  type DemoSignature,
  type DemoUpdate,
  MockBackend,
} from "./mock";
import { tauriBackend } from "./tauri";
import type { AccountFeature } from "./types";

export type { Backend } from "./backend";
export * from "./types";
export { errorMessage, hasErrorCode, isBackendError } from "./errors";

const DEMO_PUSH: readonly DemoPush[] = ["consent", "refused", "unreachable"];
const DEMO_SIGNATURE: readonly DemoSignature[] = [
  "addedOnSend",
  "notAllowed",
  "stale",
  "notUsed",
  "unreachable",
];

const DEMO_UPDATE: readonly DemoUpdate[] = ["installed", "restartfails"];

const DEMO_PROTECTION: readonly DemoProtection[] = ["setup", "checking", "expired", "relay"];

const DEMO_ORG_SETUP: readonly DemoOrgSetup[] = [
  "unconfigured",
  "consent",
  "fresh",
  "partial",
  "relay",
  "unlicensed",
  "noRole",
  "exchangeOnly",
  "blocked",
  "done",
  "failing",
  "signedOut",
];

const DEMO_FEATURES: readonly AccountFeature[] = [
  "calendar",
  "contacts",
  "tasks",
  "mailboxSettings",
  "sensitivityLabels",
];

/** The features a `?missing=` or `?refused=` parameter names, comma-separated. */
function demoFeatures(value: string | null): AccountFeature[] {
  const names = value?.split(",").map((name) => name.trim()) ?? [];
  return DEMO_FEATURES.filter((f) => names.includes(f));
}

/**
 * The Rust backend inside the desktop app; the in-memory demo backend in a
 * plain browser (`pnpm dev`). Add `?empty` to the URL to start the demo with
 * no accounts, or `?worst` to start it with worst-case data for stress-testing
 * the UI. `?push=consent|refused|unreachable` shows the Microsoft account's
 * push in that state, `?notray` a desktop without tray icons, and
 * `?reminders` a task reminder 12 minutes overdue besides the events' own.
 * `?signature=addedOnSend|notAllowed|stale|notUsed|unreachable` shows its
 * company signature in that state. `?searchfail` makes searches of the IMAP
 * account's server fail, as when it can't be reached, and `?slowsearch`
 * makes the servers take seconds, to show their hits joining the results.
 * `?missing=calendar,contacts,tasks,mailboxSettings,sensitivityLabels` starts
 * the Microsoft account without those features' permissions,
 * `?refused=calendar,contacts,tasks,sensitivityLabels` with Microsoft 365
 * refusing them, and `?customclient` signs in with an
 * organization's own app registration (approval links return to localhost).
 * Login hints such as admin@, partial@ or unassigned@ show how organizations
 * answer a sign-in (see the mock's addMicrosoftAccount).
 * `?update=installed` has an update of the Flatpak installed, whose Restart
 * reloads the demo as restarted into it (`?updated`, which says so), and
 * `?update=restartfails` one whose restart fails. `?protected` adds protected
 * and labeled mail (encrypted, signed, a link to Microsoft's portal, a Sent
 * copy that asked for encryption), and `?protection=setup|checking|expired|relay`
 * starts the work account's checks of encryption and sensitivity labels
 * passed, waiting, with Encrypt's too old to use, or with labels failing
 * because mail comes back from the Cumulas Signatures relay as outside mail.
 * `?orgsetup=` sets up the work account's organization for Set up or check
 * for your organization: `fresh` (the default) has nothing set up,
 * `partial` what the 0.4.0 guide set up, `relay` that with the Cumulas
 * Signatures relay, `done` everything; `unconfigured` is a build without
 * the setup app (only the script), `consent` needs the setup app approved
 * (open its approval link, then sign in again), `unlicensed` a license
 * without automatic labeling, `noRole` and `exchangeOnly` an administrator
 * without the roles, `blocked` a rule with settings Cumulas Mail doesn't
 * set, and `failing` and `signedOut` changes that don't all go through.
 */
export function createBackend(): Backend {
  if (isTauri()) return tauriBackend;
  const params = new URLSearchParams(window.location.search);
  const push = params.get("push") as DemoPush | null;
  const signature = params.get("signature") as DemoSignature | null;
  const update = params.get("update") as DemoUpdate | null;
  const protection = params.get("protection") as DemoProtection | null;
  const orgSetup = params.get("orgsetup") as DemoOrgSetup | null;
  return new MockBackend({
    latency: 80,
    // About as long as a real server takes to search a mailbox.
    serverSearchLatency: params.has("slowsearch") ? 2500 : 900,
    empty: params.has("empty"),
    worst: params.has("worst"),
    push: push && DEMO_PUSH.includes(push) ? push : undefined,
    signature: signature && DEMO_SIGNATURE.includes(signature) ? signature : undefined,
    overdueReminder: params.has("reminders"),
    searchFail: params.has("searchfail"),
    background: params.has("notray") ? { trayAvailable: false } : undefined,
    missing: demoFeatures(params.get("missing")),
    refused: demoFeatures(params.get("refused")),
    customClient: params.has("customclient"),
    update: update && DEMO_UPDATE.includes(update) ? update : undefined,
    protectedMail: params.has("protected"),
    protection: protection && DEMO_PROTECTION.includes(protection) ? protection : undefined,
    orgSetup: orgSetup && DEMO_ORG_SETUP.includes(orgSetup) ? orgSetup : undefined,
    restartedToUpdate: params.has("updated"),
    onRestart: () => {
      // As the desktop app comes back: the new version, saying so.
      const restarted = new URLSearchParams(params);
      restarted.delete("update");
      restarted.set("updated", "");
      window.location.search = restarted.toString();
    },
  });
}
