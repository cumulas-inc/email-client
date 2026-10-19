import type { OrgSetupChange, OrgSetupPlan, OrgSetupStatus } from "../../../api/types";
import { changeTitle } from "./changes";
import { BUTTONS, CONFIRM } from "./copy";

/**
 * Whether changing this auto-labeling policy may send it back into
 * simulation: its label's row says the policy is on now.
 */
function backToSimulation(change: OrgSetupChange, status: OrgSetupStatus): boolean {
  if (change.action !== "change" || change.object !== "autoLabelingPolicy") return false;
  const name = change.name.toLowerCase();
  return status.rows.some(
    (r) => r.id.startsWith("label:") && r.state === "ok" && r.objectName?.toLowerCase() === name,
  );
}

/** The confirmation sheet: every change, in order, with its PowerShell (3.13.2). */
export function ChangesSheet({ plan, status }: { plan: OrgSetupPlan; status: OrgSetupStatus }) {
  return (
    <div className="orgsetup-changes">
      <ol className="orgsetup-change-list">
        {plan.changes.map((change) => (
          <li key={change.id} className="orgsetup-change">
            <p className="orgsetup-change__title selectable">{changeTitle(change)}</p>
            {change.details.length > 0 && (
              <ul className="orgsetup-change__details">
                {change.details.map((line, i) => (
                  <li key={i} className="selectable">
                    {line}
                  </li>
                ))}
              </ul>
            )}
            {backToSimulation(change, status) && (
              <p className="orgsetup-note">{CONFIRM.backToSimulation}</p>
            )}
            <details className="orgsetup-disclosure">
              <summary>{BUTTONS.showPowerShell}</summary>
              <pre className="orgsetup-code__text selectable">{change.powershell}</pre>
            </details>
          </li>
        ))}
      </ol>
      <p className="orgsetup-note">{CONFIRM.footer}</p>
    </div>
  );
}
