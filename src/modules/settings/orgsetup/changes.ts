// What the dialog says about changes: the confirmation sheet's and the
// result's titles, and whether the changes went through.

import type { OrgSetupChange, OrgSetupChangeResult, OrgSetupResult } from "../../../api/types";
import { ACTION_VERBS, OBJECT_KINDS } from "./copy";

/** "Create mail flow rule “Cumulas Mail: Encrypt”": what a change does, to what. */
export function changeTitle(change: OrgSetupChange): string {
  return `${ACTION_VERBS[change.action]} ${OBJECT_KINDS[change.object]} “${change.name}”`;
}

/** The change was made, whether or not Microsoft 365 lists it yet. */
export function made(result: OrgSetupChangeResult): boolean {
  return result.outcome === "done" || result.outcome === "notYetListed";
}

/** Every change went through: the result is titled Done. */
export function allDone(result: OrgSetupResult): boolean {
  return result.changes.every(made);
}
