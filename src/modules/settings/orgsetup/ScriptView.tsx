import type { UseQueryResult } from "@tanstack/react-query";
import { ExternalLink } from "lucide-react";
import { type RefObject, useId } from "react";

import { useBackend } from "../../../api/context";
import { errorMessage } from "../../../api/errors";
import type { LicenseHintState, OrgSetupScript, PortalSteps } from "../../../api/types";
import { Spinner } from "../../../components/Spinner";
import { toast } from "../../../components/toast";
import { BUTTONS, DIALOG, POWERSHELL_GUIDE_URL, SCRIPT_VIEW, portalSteps } from "./copy";
import { CopyButton } from "./parts";

/** The script view's checkboxes: what the script covers. */
export interface ScriptParts {
  actions: boolean;
  labels: boolean;
}

interface ScriptViewProps {
  parts: ScriptParts;
  onParts: (parts: ScriptParts) => void;
  script: UseQueryResult<OrgSetupScript>;
  /** The work account's license hint; null before a setup sign-in read it. */
  license: LicenseHintState | null;
  /** The script on screen, selected when the clipboard refuses Copy script. */
  scriptRef: RefObject<HTMLPreElement | null>;
}

/** One label's Microsoft Purview portal steps, with Copy for what's typed in (3.11.1). */
function PortalStepList({ steps }: { steps: PortalSteps }) {
  const headingId = useId();
  const lines = portalSteps(steps);
  return (
    <section className="orgsetup-portal" aria-labelledby={headingId}>
      <h4 id={headingId} className="orgsetup-portal__title">
        <bdi>{steps.labelTitle}</bdi>
      </h4>
      <ol className="consent-steps">
        {lines.map((line, i) => (
          <li key={i}>
            <span className="orgsetup-portal__step">
              <span className="selectable">{line}</span>
              {i === 2 && (
                <span className="orgsetup-portal__copies">
                  <CopyButton
                    text={steps.policyName}
                    label={DIALOG.copyPolicyName(steps.policyName)}
                    className="btn--sm"
                  />
                </span>
              )}
              {i === 4 && (
                <span className="orgsetup-portal__copies">
                  <CopyButton
                    text={steps.ruleName}
                    label={DIALOG.copyRuleName(steps.ruleName)}
                    className="btn--sm"
                  />
                  <CopyButton
                    text={steps.pattern}
                    label={DIALOG.copyPattern(steps.pattern)}
                    className="btn--sm"
                  />
                </span>
              )}
            </span>
          </li>
        ))}
      </ol>
    </section>
  );
}

/**
 * The PowerShell script that sets up the same rules and policies, filled in,
 * and the portal steps for administrators without Windows (3.11).
 */
export function ScriptView({ parts, onParts, script, license, scriptRef }: ScriptViewProps) {
  const backend = useBackend();
  const id = useId();
  const openGuide = () => {
    backend.openLink(POWERSHELL_GUIDE_URL).catch((e: unknown) => toast.error(e));
  };
  const steps = parts.labels ? (script.data?.portalSteps ?? []) : [];

  return (
    <div className="orgsetup-script">
      <div className="orgsetup-choices">
        <label className="checkbox">
          <input
            type="checkbox"
            checked={parts.actions}
            onChange={(e) => onParts({ ...parts, actions: e.currentTarget.checked })}
          />
          {SCRIPT_VIEW.encryption}
        </label>
        <label className="checkbox">
          <input
            type="checkbox"
            checked={parts.labels}
            aria-describedby={license !== "included" ? `${id}-plan` : undefined}
            onChange={(e) => onParts({ ...parts, labels: e.currentTarget.checked })}
          />
          {SCRIPT_VIEW.labels}
        </label>
        {license !== "included" && (
          <p id={`${id}-plan`} className="orgsetup-note">
            {SCRIPT_VIEW.labelsNeedPlan}
          </p>
        )}
      </div>
      <div className="orgsetup-notes">
        <p>{SCRIPT_VIEW.part1}</p>
        {parts.labels && <p>{SCRIPT_VIEW.part2}</p>}
      </div>
      {script.isError ? (
        <p className="field__error" role="alert">
          {errorMessage(script.error)}
        </p>
      ) : script.data ? (
        <pre
          ref={scriptRef}
          className="orgsetup-script__text selectable"
          // Focusable, so the keyboard can scroll it.
          tabIndex={0}
          aria-label={DIALOG.scriptLabel}
        >
          {script.data.script}
        </pre>
      ) : (
        <div className="orgsetup-wait">
          <Spinner label="Loading the script" />
        </div>
      )}
      {steps.length > 0 && (
        <section className="orgsetup-section" aria-labelledby={`${id}-portal`}>
          <h3 id={`${id}-portal`} className="orgsetup-section__title">
            {SCRIPT_VIEW.portalTitle}
          </h3>
          {steps.map((s) => (
            <PortalStepList key={s.policyName} steps={s} />
          ))}
        </section>
      )}
      <div>
        <button type="button" className="btn btn--subtle btn--sm" onClick={openGuide}>
          <ExternalLink size={14} aria-hidden="true" />
          {SCRIPT_VIEW.guideLink}
        </button>
      </div>
    </div>
  );
}

/** The script view's main button. */
export function CopyScriptButton({
  script,
  scriptRef,
}: {
  script: UseQueryResult<OrgSetupScript>;
  scriptRef: RefObject<HTMLPreElement | null>;
}) {
  if (!script.data) {
    return (
      <button type="button" className="btn btn--primary" disabled>
        {BUTTONS.copyScript}
      </button>
    );
  }
  return (
    <CopyButton text={script.data.script} source={scriptRef} className="btn--primary">
      {BUTTONS.copyScript}
    </CopyButton>
  );
}
