import { useEffect, useId } from "react";

import { errorMessage } from "../../api/errors";
import type { Account, AccountFeature } from "../../api/types";
import { Dialog } from "../../components/Dialog";
import { Spinner } from "../../components/Spinner";
import { AdminConsent } from "./AdminConsent";
import { openTurnOn } from "./addAccountStore";
import { focusLater, useRestoreFocus } from "./focus";
import { useAdminConsentLink } from "./queries";

import "./settings.css";

interface ApprovalDialogProps {
  account: Account;
  feature: AccountFeature;
  /** Where the keyboard goes after a Turn on started here. */
  returnFocusId?: string;
  onClose: () => void;
}

/** "Ask IT": the approval link for a feature of an account, to send to IT. */
export function ApprovalDialog({ account, feature, returnFocusId, onClose }: ApprovalDialogProps) {
  const titleId = useId();
  const link = useAdminConsentLink(account.id);
  useRestoreFocus();

  // Once the link is in, the keyboard goes to what it's about.
  const loaded = link.data !== undefined;
  useEffect(() => {
    if (loaded) focusLater(titleId);
  }, [loaded, titleId]);

  const turnOn = () => {
    onClose();
    openTurnOn(account.email, feature, returnFocusId);
  };

  return (
    <Dialog
      open
      onClose={onClose}
      title="Ask IT for approval"
      width={560}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            Close
          </button>
          <button type="button" className="btn btn--primary" onClick={turnOn}>
            Turn on
          </button>
        </>
      }
    >
      <div className="approval-dialog" aria-busy={link.isPending ? "true" : undefined}>
        {link.isPending && (
          <div className="approval-dialog__loading">
            <Spinner size={24} label="Loading the approval link" />
          </div>
        )}
        {link.isError && (
          <p className="field__error" role="alert">
            {errorMessage(link.error)}
          </p>
        )}
        {link.data && (
          <AdminConsent
            adminConsentUrl={link.data.adminConsentUrl}
            tenant={link.data.tenant}
            adminPageUrl={link.data.adminPageUrl}
            email={account.email}
            feature={feature}
            retryLabel="Turn on"
            titleId={titleId}
          />
        )}
      </div>
    </Dialog>
  );
}
