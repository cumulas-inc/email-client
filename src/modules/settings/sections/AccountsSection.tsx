import clsx from "clsx";
import { formatDistanceToNowStrict } from "date-fns";
import {
  ChevronDown,
  KeyRound,
  LoaderCircle,
  LogIn,
  Plus,
  RefreshCw,
  Trash,
  TriangleAlert,
  Wrench,
} from "lucide-react";
import { useEffect, useId, useState } from "react";

import { useAccounts, useSettings, useSyncStatuses } from "../../../api/queries";
import type { Account, SyncStatus } from "../../../api/types";
import { ColorPicker } from "../../../components/ColorPicker";
import { EmptyState } from "../../../components/EmptyState";
import { PaneSpinner } from "../../../components/Spinner";
import { toast } from "../../../components/toast";
import { openAddAccount } from "../addAccountStore";
import { ACCOUNT_COLORS, ADD_ACCOUNT_BUTTON_ID } from "../constants";
import { Callout, TextField } from "../Fields";
import { focusLater } from "../focus";
import { ImapAccountEditor } from "../ImapAccountEditor";
import { TLS_LABELS } from "../presets";
import { useSyncAccount, useUpdateAccount } from "../queries";
import { trackSave } from "../saveStatus";

/** Re-renders every half minute so "Synced 2 minutes ago" stays true. */
function useTick(ms = 30_000) {
  const [, setTick] = useState(0);
  useEffect(() => {
    const timer = window.setInterval(() => setTick((t) => t + 1), ms);
    return () => window.clearInterval(timer);
  }, [ms]);
}

function syncedLabel(iso: string | null): string {
  if (!iso) return "Not synced yet";
  const date = new Date(iso);
  if (Date.now() - date.getTime() < 60_000) return "Synced just now";
  return `Synced ${formatDistanceToNowStrict(date, { addSuffix: true })}`;
}

interface AccountsSectionProps {
  onRemove: (account: Account) => void;
}

export function AccountsSection({ onRemove }: AccountsSectionProps) {
  const { data: accounts, isLoading } = useAccounts();
  const { data: statuses = [] } = useSyncStatuses();
  const { data: settings } = useSettings();
  const [expanded, setExpanded] = useState<string | null>(null);
  useTick();

  if (isLoading || !accounts) return <PaneSpinner />;

  const toggle = (id: string) => setExpanded((current) => (current === id ? null : id));

  return (
    <>
      <p className="settings-intro">
        Accounts you&apos;ve added on this computer. Changes here don&apos;t affect your accounts on
        the server.
      </p>
      {accounts.length === 0 ? (
        <EmptyState
          title="No accounts yet"
          description="Add an account to start using Cumulas Mail."
        />
      ) : (
        <ul className="account-list">
          {accounts.map((account) => (
            <AccountItem
              key={account.id}
              account={account}
              status={statuses.find((s) => s.accountId === account.id)}
              intervalMinutes={settings?.syncIntervalMinutes}
              expanded={expanded === account.id || accounts.length === 1}
              canCollapse={accounts.length > 1}
              onToggle={() => toggle(account.id)}
              onExpand={() => setExpanded(account.id)}
              onRemove={() => onRemove(account)}
            />
          ))}
        </ul>
      )}
      <div className="settings-actions">
        <button
          id={ADD_ACCOUNT_BUTTON_ID}
          type="button"
          className="btn"
          onClick={() => openAddAccount()}
        >
          <Plus size={16} aria-hidden="true" />
          Add account
        </button>
      </div>
    </>
  );
}

interface AccountItemProps {
  account: Account;
  status: SyncStatus | undefined;
  intervalMinutes: number | undefined;
  expanded: boolean;
  canCollapse: boolean;
  onToggle: () => void;
  onExpand: () => void;
  onRemove: () => void;
}

function AccountItem({
  account,
  status,
  intervalMinutes,
  expanded,
  canCollapse,
  onToggle,
  onExpand,
  onRemove,
}: AccountItemProps) {
  const id = useId();
  const bodyId = `${id}-body`;
  const editButtonId = `${id}-edit-servers`;
  const sync = useSyncAccount();
  const update = useUpdateAccount();
  const [editing, setEditing] = useState(false);

  const state = status?.state ?? (account.syncError ? "error" : "idle");
  const lastSyncedAt = status?.lastSyncedAt ?? account.lastSyncedAt;
  const syncError =
    state === "error" ? (status?.message ?? account.syncError ?? "Sync failed.") : null;
  const syncing = state === "syncing";
  const microsoft = account.kind === "microsoft";

  const syncNow = () => {
    sync.mutate(account.id, { onError: (e) => toast.error(e) });
  };

  const signInAgain = () =>
    openAddAccount({ step: "microsoft", email: account.email, autoStart: true });

  // Opening the editor focuses its password field.
  const fix = () => {
    onExpand();
    setEditing(true);
  };

  const stopEditing = () => {
    setEditing(false);
    focusLater(editButtonId);
  };

  const imap = account.settings.kind === "imap" ? account.settings : null;
  let servers = null;
  if (imap && editing) {
    servers = (
      <div className="account-field">
        <span className="field__label">Password and servers</span>
        <ImapAccountEditor
          account={account}
          settings={imap}
          onDone={stopEditing}
          onCancel={stopEditing}
        />
      </div>
    );
  } else if (imap) {
    servers = (
      <div className="account-field">
        <div className="account-field__head">
          <span className="field__label">Password and servers</span>
          <button
            id={editButtonId}
            type="button"
            className="btn btn--sm"
            onClick={() => setEditing(true)}
          >
            <KeyRound size={14} aria-hidden="true" />
            Change password or servers
          </button>
        </div>
        <dl className="server-summary selectable">
          <dt>Incoming</dt>
          <dd>
            {imap.imapHost}:{imap.imapPort} · {TLS_LABELS[imap.imapTls]}
          </dd>
          <dt>Outgoing</dt>
          <dd>
            {imap.smtpHost}:{imap.smtpPort} · {TLS_LABELS[imap.smtpTls]}
          </dd>
          <dt>User name</dt>
          <dd>{imap.username}</dd>
        </dl>
      </div>
    );
  }

  let statusNode;
  if (syncing) {
    statusNode = (
      <span className="account-status">
        <LoaderCircle size={14} className="spin" aria-hidden="true" />
        {status?.message ?? "Syncing…"}
      </span>
    );
  } else if (syncError) {
    statusNode = (
      <span className="account-status account-status--error">
        <TriangleAlert size={14} aria-hidden="true" />
        Couldn&apos;t sync
      </span>
    );
  } else {
    statusNode = <span className="account-status">{syncedLabel(lastSyncedAt)}</span>;
  }

  return (
    <li className={clsx("account-item", expanded && "account-item--expanded")}>
      <div className="account-item__header">
        <span
          className="account-item__dot"
          style={{ background: account.color }}
          aria-hidden="true"
        />
        <div className="account-item__identity">
          <div className="account-item__line">
            <span className="account-item__name">{account.displayName}</span>
            <span className={`kind-badge kind-badge--${account.kind}`}>
              {microsoft ? "Microsoft 365" : "IMAP"}
            </span>
          </div>
          <div className="account-item__email selectable">{account.email}</div>
          <div className="account-item__status">{statusNode}</div>
        </div>
        <div className="account-item__actions">
          {syncError &&
            (microsoft ? (
              <button type="button" className="btn btn--sm" onClick={signInAgain}>
                <LogIn size={14} aria-hidden="true" />
                Sign in again
              </button>
            ) : (
              <button
                type="button"
                className="btn btn--sm"
                onClick={fix}
                aria-label={`Fix ${account.displayName}`}
              >
                <Wrench size={14} aria-hidden="true" />
                Fix
              </button>
            ))}
          {canCollapse && (
            <button
              type="button"
              className="icon-btn account-item__toggle"
              aria-expanded={expanded}
              aria-controls={bodyId}
              aria-label={`Manage ${account.displayName}`}
              onClick={onToggle}
            >
              <ChevronDown size={18} />
            </button>
          )}
        </div>
      </div>

      {/* When an IMAP account is open, its callout below shows the error instead. */}
      {syncError && (microsoft || !expanded) && (
        <p className="account-item__error selectable">
          <span className="visually-hidden">Sync problem: </span>
          {syncError}
        </p>
      )}

      {expanded && (
        <div id={bodyId} className="account-item__body">
          {syncError && !microsoft && (
            <Callout
              tone="danger"
              title="This account can't sync right now"
              action={
                <button type="button" className="btn btn--sm" onClick={syncNow}>
                  <RefreshCw size={14} aria-hidden="true" />
                  Try again
                </button>
              }
            >
              <p className="selectable">{syncError}</p>
              <p>
                Check that you&apos;re online. If your password or server settings changed, update
                them below.
              </p>
            </Callout>
          )}

          {/* With a sync error, the way to fix it comes first. */}
          {syncError && servers}

          <AccountNameField account={account} />

          <div className="account-field">
            <span id={`${id}-color`} className="field__label">
              Color
            </span>
            <ColorPicker
              labelledBy={`${id}-color`}
              value={account.color}
              options={ACCOUNT_COLORS}
              onChange={(color) =>
                void trackSave(update.mutateAsync({ accountId: account.id, color })).catch(
                  () => undefined,
                )
              }
            />
          </div>

          <div className="account-field account-field--row">
            <div>
              <span className="field__label">Sync</span>
              <p className="field__hint">
                {syncing ? "Syncing now…" : syncedLabel(lastSyncedAt)}
                {intervalMinutes !== undefined &&
                  ` · checks every ${intervalMinutes === 1 ? "minute" : `${intervalMinutes} minutes`}`}
              </p>
            </div>
            <button type="button" className="btn btn--sm" onClick={syncNow} disabled={syncing}>
              <RefreshCw size={14} aria-hidden="true" className={syncing ? "spin" : undefined} />
              Sync now
            </button>
          </div>

          {!syncError && servers}

          <div className="account-field account-field--row account-field--danger">
            <div>
              <span className="field__label">Remove account</span>
              <p className="field__hint">
                Removes it and its data from this computer. Nothing changes on the server.
              </p>
            </div>
            <button type="button" className="btn btn--sm btn--danger" onClick={onRemove}>
              <Trash size={14} aria-hidden="true" />
              Remove…
            </button>
          </div>
        </div>
      )}
    </li>
  );
}

function AccountNameField({ account }: { account: Account }) {
  const id = useId();
  const update = useUpdateAccount();
  const [name, setName] = useState(account.displayName);
  const [error, setError] = useState<string | null>(null);

  const commit = () => {
    const value = name.trim();
    if (!value) {
      setError("Enter a name for this account.");
      return;
    }
    setError(null);
    if (value === account.displayName) return;
    void trackSave(update.mutateAsync({ accountId: account.id, displayName: value })).catch(() =>
      setName(account.displayName),
    );
  };

  return (
    <TextField
      id={`${id}-name`}
      fieldClassName="account-field"
      label="Account name"
      value={name}
      error={error}
      hint="Shown in the folder list and when you choose which account to send from."
      onChange={(e) => {
        setName(e.target.value);
        if (error && e.target.value.trim()) setError(null);
      }}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          commit();
        } else if (e.key === "Escape" && name !== account.displayName) {
          // Undo the edit instead of closing the dialog.
          e.preventDefault();
          e.stopPropagation();
          setName(account.displayName);
          setError(null);
        }
      }}
    />
  );
}
