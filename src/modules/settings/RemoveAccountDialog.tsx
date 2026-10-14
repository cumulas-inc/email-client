import clsx from "clsx";
import { useEffect, useRef, useState } from "react";

import { errorMessage } from "../../api/errors";
import type { Account } from "../../api/types";
import { Dialog } from "../../components/Dialog";
import { toast } from "../../components/toast";
import { Callout } from "./Fields";
import { useRestoreFocus } from "./focus";
import { useRemoveAccount } from "./queries";

interface RemoveAccountDialogProps {
  account: Account | null;
  onClose: () => void;
  /** Called after the account was removed (e.g. to move focus). */
  onRemoved?: () => void;
}

/** Confirms removing an account and explains what is (and isn't) deleted. */
export function RemoveAccountDialog({ account, onClose, onRemoved }: RemoveAccountDialogProps) {
  if (!account) return null;
  return <ConfirmRemove account={account} onClose={onClose} onRemoved={onRemoved} />;
}

function ConfirmRemove({
  account,
  onClose,
  onRemoved,
}: RemoveAccountDialogProps & { account: Account }) {
  const remove = useRemoveAccount();
  const cancelRef = useRef<HTMLButtonElement>(null);
  const [error, setError] = useState<string | null>(null);
  useRestoreFocus();

  useEffect(() => {
    // Start on the safe choice; deferred until after showModal().
    const timer = window.setTimeout(() => cancelRef.current?.focus(), 0);
    return () => window.clearTimeout(timer);
  }, []);

  const confirm = async () => {
    setError(null);
    try {
      await remove.mutateAsync(account.id);
      toast.success(`${account.email} was removed from this computer.`);
      onClose();
      onRemoved?.();
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const imap = account.kind === "imap";

  return (
    <Dialog
      open
      onClose={onClose}
      title="Remove this account?"
      width={480}
      footer={
        <>
          {/* Cancel first and the confirming button last, as in every confirmation. */}
          <button ref={cancelRef} type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className={clsx("btn btn--destructive", remove.isPending && "btn--busy")}
            onClick={() => void confirm()}
            disabled={remove.isPending}
            aria-busy={remove.isPending || undefined}
          >
            {remove.isPending ? "Removing…" : "Remove account"}
          </button>
        </>
      }
    >
      <div className="remove-account">
        <p>
          <strong className="remove-account__address">{account.email}</strong> will be removed from
          Cumulas Mail on this computer.
        </p>
        <ul className="remove-account__list">
          <li>
            Its downloaded mail, calendars, contacts and tasks are deleted from this computer.
          </li>
          <li>
            {imap
              ? "Its saved password is removed from your keyring."
              : "Its Microsoft sign-in is removed from your keyring."}
          </li>
          <li>
            Nothing changes on the server: your mail stays there, and you can add the account again
            at any time.
          </li>
        </ul>
        {imap && (
          <Callout tone="warning">
            <p>
              Calendar events, contacts and tasks you created in this account are kept only on this
              computer, so they&apos;ll be deleted for good.
            </p>
          </Callout>
        )}
        {error && (
          <Callout tone="danger" role="alert" title="Couldn't remove the account">
            <p>{error}</p>
          </Callout>
        )}
      </div>
    </Dialog>
  );
}
