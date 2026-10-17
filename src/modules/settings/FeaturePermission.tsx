import { LoaderCircle, Mail, RefreshCw } from "lucide-react";
import { useId, useState } from "react";

import { useBackend } from "../../api/context";
import type { Account } from "../../api/types";
import { AccountBadge } from "../../components/AccountBadge";
import { toast } from "../../components/toast";
import { openTurnOn } from "./addAccountStore";
import { ApprovalDialog } from "./ApprovalDialog";
import { noteFeatureTurnedOn } from "./featureToasts";
import {
  FEATURE_COPY,
  type ModuleFeature,
  accountsWithout,
  isPersonalAccount,
  noticeHeadingIds,
  refusedFeatureMailto,
} from "./microsoft";
import { useRetryMicrosoftFeatures } from "./queries";

import "./settings.css";

/** The refused text, shared with Settings → Accounts. */
export const REFUSED_HINT =
  "Your account has the permission, but Microsoft 365 refused access. This often means your mailbox doesn't include it, for example because of your license. Ask your IT team to check.";

interface FeaturePermissionNoticeProps {
  feature: ModuleFeature;
  accounts: Account[];
  /** Where the keyboard goes once a Turn on succeeds (the module's New button). */
  returnFocusId?: string;
  /** A base for the headings' ids (see noticeHeadingIds). */
  id?: string;
}

/**
 * Under a module's list: the Microsoft accounts whose calendar, contacts or
 * tasks can't be used, with what to do. Nothing when there are none.
 */
export function FeaturePermissionNotice({
  feature,
  accounts,
  returnFocusId,
  id,
}: FeaturePermissionNoticeProps) {
  const ownId = useId();
  const headings = noticeHeadingIds(id ?? ownId);
  const backend = useBackend();
  const retry = useRetryMicrosoftFeatures();
  const [askingFor, setAskingFor] = useState<Account | null>(null);
  const { needing, refused } = accountsWithout(accounts, feature);
  if (needing.length === 0 && refused.length === 0) return null;
  const copy = FEATURE_COPY[feature];
  const allPersonal = needing.every(isPersonalAccount);

  const checkAgain = (account: Account) => {
    // The notice going away says it's back; a sync that's refused again says so.
    for (const f of account.refusedFeatures) noteFeatureTurnedOn(account.id, f);
    retry.mutate(account.id, { onError: (e) => toast.error(e) });
  };

  const emailIt = (account: Account) => {
    backend
      .openLink(refusedFeatureMailto({ email: account.email, features: [feature] }))
      .catch((e: unknown) => toast.error(e));
  };

  return (
    <div className="feature-notice">
      {needing.length > 0 && (
        <section className="feature-notice__section" aria-labelledby={headings.needing}>
          <h2 id={headings.needing} className="feature-notice__title">
            {copy.title}
          </h2>
          <ul className="feature-notice__rows">
            {needing.map((account) => (
              <li key={account.id} className="feature-notice__row">
                <AccountBadge name={account.displayName} color={account.color} size={20} />
                <span className="feature-notice__email" title={account.email}>
                  {account.email}
                </span>
                <span className="feature-notice__actions">
                  <button
                    type="button"
                    className="btn btn--sm"
                    aria-label={`Turn on ${copy.noun} for ${account.email}`}
                    onClick={() => openTurnOn(account.email, feature, returnFocusId)}
                  >
                    Turn on
                  </button>
                  {!isPersonalAccount(account) && (
                    <button
                      type="button"
                      className="btn btn--sm"
                      aria-label={`Ask IT about ${copy.noun} for ${account.email}`}
                      onClick={() => setAskingFor(account)}
                    >
                      Ask IT
                    </button>
                  )}
                </span>
              </li>
            ))}
          </ul>
          <p className="feature-notice__hint">
            {allPersonal
              ? "Turn on asks Microsoft for permission."
              : "Turn on asks Microsoft for permission. If your organization requires an administrator to approve it, select Ask IT to send them a link."}
          </p>
        </section>
      )}
      {refused.length > 0 && (
        <section className="feature-notice__section" aria-labelledby={headings.refused}>
          <h2 id={headings.refused} className="feature-notice__title">
            {copy.refusedTitle}
          </h2>
          <p className="feature-notice__hint">{REFUSED_HINT}</p>
          <ul className="feature-notice__rows">
            {refused.map((account) => {
              const checking = retry.isPending && retry.variables === account.id;
              return (
                <li key={account.id} className="feature-notice__row">
                  <AccountBadge name={account.displayName} color={account.color} size={20} />
                  <span className="feature-notice__email" title={account.email}>
                    {account.email}
                  </span>
                  <span className="feature-notice__actions">
                    <button
                      type="button"
                      className="btn btn--sm"
                      aria-label={`Check ${copy.noun} again for ${account.email}`}
                      aria-busy={checking || undefined}
                      disabled={checking}
                      onClick={() => checkAgain(account)}
                    >
                      {checking ? (
                        <LoaderCircle size={14} className="spin" aria-hidden="true" />
                      ) : (
                        <RefreshCw size={14} aria-hidden="true" />
                      )}
                      Check again
                    </button>
                    <button
                      type="button"
                      className="btn btn--sm"
                      aria-label={`Email IT about ${copy.noun} for ${account.email}`}
                      onClick={() => emailIt(account)}
                    >
                      <Mail size={14} aria-hidden="true" />
                      Email IT
                    </button>
                  </span>
                </li>
              );
            })}
          </ul>
        </section>
      )}
      {askingFor && (
        <ApprovalDialog
          account={askingFor}
          feature={feature}
          returnFocusId={returnFocusId}
          onClose={() => setAskingFor(null)}
        />
      )}
    </div>
  );
}
