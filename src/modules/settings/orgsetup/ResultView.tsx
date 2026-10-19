import clsx from "clsx";
import { CircleAlert, CircleCheck, CircleMinus } from "lucide-react";

import type { OrgSetupChangeResult, OrgSetupResult } from "../../../api/types";
import { Callout } from "../Fields";
import { changeTitle, made } from "./changes";
import { BUTTONS, DONE, OWN_PREFIX } from "./copy";
import { DetailText } from "./parts";

/** The rules that encrypt (not the one that blocks label requests from outside). */
const ENCRYPTION_RULES = [`${OWN_PREFIX}Encrypt`, `${OWN_PREFIX}Do Not Forward`].map((name) =>
  name.toLowerCase(),
);

function outcomeText(r: OrgSetupChangeResult): string {
  switch (r.outcome) {
    case "done":
      return DONE.done;
    case "notYetListed":
      return DONE.notYetListed;
    case "failed":
      return DONE.failed(r.message ?? "");
    case "skipped":
      return DONE.skipped(r.message ?? "");
  }
}

function OutcomeIcon({ result }: { result: OrgSetupChangeResult }) {
  if (made(result)) return <CircleCheck size={16} aria-hidden="true" />;
  if (result.outcome === "failed") return <CircleAlert size={16} aria-hidden="true" />;
  return <CircleMinus size={16} aria-hidden="true" />;
}

/** What the changes came to, and what to do next (3.13.2). */
export function ResultView({ result }: { result: OrgSetupResult }) {
  const encryption = result.changes.some(
    (r) =>
      made(r) &&
      r.change.object === "mailFlowRule" &&
      ENCRYPTION_RULES.includes(r.change.name.toLowerCase()),
  );
  const labels = result.changes.some((r) => made(r) && r.change.object !== "mailFlowRule");
  const removal = result.status.rows.find((r) => r.id === "setupApp")?.detail;

  return (
    <div className="orgsetup-result">
      {result.signedOut && (
        <Callout tone="warning">
          <p>{DONE.signedOut}</p>
        </Callout>
      )}
      <ul className="orgsetup-outcomes">
        {result.changes.map((r) => (
          <li
            key={r.change.id}
            className={clsx("orgsetup-outcome", `orgsetup-outcome--${r.outcome}`)}
          >
            <OutcomeIcon result={r} />
            <span className="orgsetup-outcome__text">
              <span className="orgsetup-outcome__title selectable">{changeTitle(r.change)}</span>
              <span className="orgsetup-outcome__line selectable">{outcomeText(r)}</span>
              {r.outcome === "done" && r.message && (
                <span className="orgsetup-outcome__line selectable">{r.message}</span>
              )}
            </span>
          </li>
        ))}
      </ul>
      <div className="orgsetup-next">
        {encryption && <p>{DONE.checkEncryption}</p>}
        {labels && <p>{DONE.checkLabels}</p>}
        <p>{DONE.everyoneElse}</p>
      </div>
      {removal && (
        <details className="orgsetup-disclosure">
          <summary>{BUTTONS.doneWithSetup}</summary>
          <DetailText text={removal} />
        </details>
      )}
    </div>
  );
}
