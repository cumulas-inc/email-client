import clsx from "clsx";
import { useEffect, useId, useState, type FormEvent, type KeyboardEvent } from "react";

import type { Account, ImapAccountUpdate, ImapSettings } from "../../api/types";
import { Spinner } from "../../components/Spinner";
import { toast } from "../../components/toast";
import { PasswordField, ProblemCallout } from "./Fields";
import { focusLater } from "./focus";
import { findPreset, suggestedSettings } from "./presets";
import { useUpdateImapAccount } from "./queries";
import {
  SERVER_FIELDS,
  describeImapFailure,
  fromDraft,
  sameSettings,
  toDraft,
  validateServerDraft,
  type FormProblem,
  type ImapErrors,
  type ImapFieldName,
  type ServerDraft,
} from "./serverDraft";
import { ServerSettingsFields } from "./ServerSettingsFields";

const FIELD_ORDER: ImapFieldName[] = ["password", ...SERVER_FIELDS];

interface ImapAccountEditorProps {
  account: Account;
  settings: ImapSettings;
  /** Saved and checked; the form can close. */
  onDone: () => void;
  onCancel: () => void;
}

/**
 * Change an IMAP account's password and/or server settings. Saving checks
 * them against the servers first, like adding an account.
 */
export function ImapAccountEditor({ account, settings, onDone, onCancel }: ImapAccountEditorProps) {
  const uid = useId();
  const ids = Object.fromEntries(FIELD_ORDER.map((f) => [f, `${uid}-${f}`])) as Record<
    ImapFieldName,
    string
  >;
  const problemId = `${uid}-problem`;
  const passwordId = ids.password;

  const [password, setPassword] = useState("");
  const [draft, setDraft] = useState<ServerDraft>(() => toDraft(settings));
  const [errors, setErrors] = useState<ImapErrors>({});
  const [problem, setProblem] = useState<FormProblem | null>(null);
  const { mutateAsync, isPending } = useUpdateImapAccount();

  const preset = findPreset(account.email);
  const suggested = suggestedSettings(account.email);

  useEffect(() => {
    document.getElementById(passwordId)?.focus();
  }, [passwordId]);

  const clearErrors = (fields: string[]) =>
    setErrors((e) => {
      if (!fields.some((f) => e[f as ImapFieldName])) return e;
      const next = { ...e };
      for (const f of fields) delete next[f as ImapFieldName];
      return next;
    });

  const changeServer = (change: Partial<ServerDraft>) => {
    setDraft((d) => ({ ...d, ...change }));
    clearErrors(Object.keys(change));
  };

  const showErrors = (found: ImapErrors) => {
    setErrors(found);
    const first = FIELD_ORDER.find((f) => found[f]);
    if (first) focusLater(ids[first]);
  };

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (isPending) return;
    setProblem(null);
    const found = validateServerDraft(draft);
    if (Object.keys(found).length > 0) {
      showErrors(found);
      return;
    }
    setErrors({});

    // Only send what changed; the backend keeps the rest.
    const next = fromDraft(draft);
    const update: ImapAccountUpdate = {};
    if (password) update.password = password;
    if (!sameSettings(next, settings)) update.settings = next;

    try {
      await mutateAsync({ accountId: account.id, update });
      toast.success(`Saved. ${account.email} is connected.`);
      onDone();
    } catch (error) {
      const failure = describeImapFailure(error, {
        preset,
        email: account.email,
        server: draft,
        serversAre: "below",
        fallbackTitle: "Couldn't save these settings",
      });
      const { email: emailError, ...fieldErrors } = failure.errors;
      const formProblem = failure.problem ?? (emailError ? { title: emailError } : null);
      if (formProblem) {
        setProblem(formProblem);
        focusLater(problemId);
      }
      showErrors(fieldErrors);
    }
  };

  const onKeyDown = (e: KeyboardEvent<HTMLFormElement>) => {
    // Escape leaves the form instead of closing Settings.
    if (e.key === "Escape" && !isPending) {
      e.preventDefault();
      e.stopPropagation();
      onCancel();
    }
  };

  const passwordHint = preset?.appPassword
    ? `Leave blank to keep the current password. ${preset.name} needs an ${preset.appPassword.term}, not your usual password.`
    : "Leave blank to keep the current password.";

  return (
    <form
      className="imap-editor"
      noValidate
      aria-label={`Password and servers for ${account.displayName}`}
      onSubmit={(e) => void submit(e)}
      onKeyDown={onKeyDown}
    >
      <fieldset className="imap-editor__fields" disabled={isPending}>
        <legend className="visually-hidden">Password and servers</legend>
        <PasswordField
          id={passwordId}
          label="New password"
          autoComplete="new-password"
          placeholder="Unchanged"
          value={password}
          error={errors.password}
          hint={passwordHint}
          onChange={(value) => {
            setPassword(value);
            clearErrors(["password"]);
          }}
        />
        <ServerSettingsFields
          ids={ids}
          value={draft}
          errors={errors}
          onChange={changeServer}
          onReset={
            sameSettings(fromDraft(draft), suggested)
              ? undefined
              : () => {
                  setDraft(toDraft(suggested));
                  clearErrors(SERVER_FIELDS);
                }
          }
        />
        {problem && <ProblemCallout id={problemId} problem={problem} />}
      </fieldset>
      <div className="imap-editor__actions">
        <button
          type="submit"
          className={clsx("btn btn--primary", isPending && "btn--busy")}
          disabled={isPending}
          aria-busy={isPending || undefined}
        >
          {isPending && <Spinner size={16} label="Checking" />}
          {isPending ? "Checking your settings…" : "Save changes"}
        </button>
        <button type="button" className="btn" onClick={onCancel} disabled={isPending}>
          Cancel
        </button>
      </div>
      {isPending && (
        <p className="visually-hidden" role="status">
          Checking your settings…
        </p>
      )}
    </form>
  );
}
