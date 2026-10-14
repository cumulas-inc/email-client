import clsx from "clsx";
import { ArrowLeft, AtSign, ChevronRight, ExternalLink, KeyRound } from "lucide-react";
import { useId, useState, type FormEvent } from "react";

import { useBackend } from "../../api/context";
import type { Account } from "../../api/types";
import { Spinner } from "../../components/Spinner";
import { toast } from "../../components/toast";
import { Callout, PasswordField, ProblemCallout, TextField } from "./Fields";
import { focusLater } from "./focus";
import { PROTON_BRIDGE_URL, findPreset, suggestedSettings, type ProviderPreset } from "./presets";
import { useAddImapAccount } from "./queries";
import {
  SERVER_FIELDS,
  describeImapFailure,
  fromDraft,
  toDraft,
  validateServerDraft,
  type FormProblem,
  type ImapErrors,
  type ImapFieldName,
  type ServerDraft,
} from "./serverDraft";
import { ServerSettingsFields } from "./ServerSettingsFields";
import { StepIntro, StepLayout } from "./StepLayout";
import { validateEmail } from "./validation";

const FIELD_ORDER: ImapFieldName[] = ["email", "password", ...SERVER_FIELDS];

interface ImapFormProps {
  initialEmail: string;
  onBack?: () => void;
  onClose: () => void;
  onDone: (account: Account) => void;
  /** Switch to Microsoft sign-in (suggested for Outlook.com addresses). */
  onUseMicrosoft: (email: string) => void;
}

export function ImapForm({ initialEmail, onBack, onClose, onDone, onUseMicrosoft }: ImapFormProps) {
  const backend = useBackend();
  const uid = useId();
  const ids = Object.fromEntries(
    [...FIELD_ORDER, "displayName"].map((f) => [f, `${uid}-${f}`]),
  ) as Record<ImapFieldName | "displayName", string>;
  const formId = `${uid}-form`;
  const advancedId = `${uid}-advanced`;
  const problemId = `${uid}-problem`;

  const [email, setEmail] = useState(initialEmail);
  const [displayName, setDisplayName] = useState("");
  const [password, setPassword] = useState("");
  const [overrides, setOverrides] = useState<Partial<ServerDraft>>({});
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [errors, setErrors] = useState<ImapErrors>({});
  const [problem, setProblem] = useState<FormProblem | null>(null);
  const { mutateAsync, isPending } = useAddImapAccount();

  const preset = findPreset(email);
  const server: ServerDraft = { ...toDraft(suggestedSettings(email)), ...overrides };
  const customized = Object.keys(overrides).length > 0;

  const clearErrors = (fields: string[]) =>
    setErrors((e) => {
      if (!fields.some((f) => e[f as ImapFieldName])) return e;
      const next = { ...e };
      for (const f of fields) delete next[f as ImapFieldName];
      return next;
    });

  const changeServer = (change: Partial<ServerDraft>) => {
    setOverrides((o) => ({ ...o, ...change }));
    clearErrors(Object.keys(change));
  };

  const validate = (): ImapErrors => {
    const found: ImapErrors = validateServerDraft(server);
    const emailError = validateEmail(email);
    if (emailError) found.email = emailError;
    if (!password) {
      found.password = preset?.appPassword
        ? `Enter your ${preset.name} ${preset.appPassword.term}.`
        : "Enter your password.";
    }
    return found;
  };

  const showErrors = (found: ImapErrors) => {
    setErrors(found);
    const first = FIELD_ORDER.find((f) => found[f]);
    if (!first) return;
    if ((SERVER_FIELDS as string[]).includes(first)) setAdvancedOpen(true);
    focusLater(ids[first]);
  };

  const onSubmit = async (e: FormEvent) => {
    e.preventDefault();
    if (isPending) return;
    setProblem(null);
    const found = validate();
    if (Object.keys(found).length > 0) {
      showErrors(found);
      return;
    }
    setErrors({});
    try {
      const account = await mutateAsync({
        displayName: displayName.trim(),
        email: email.trim(),
        password,
        settings: fromDraft(server),
      });
      onDone(account);
    } catch (error) {
      const failure = describeImapFailure(error, {
        preset,
        email,
        server,
        serversAre: "under Advanced settings",
        fallbackTitle: "Couldn't add this account",
      });
      if (failure.problem) {
        setAdvancedOpen(true);
        setProblem(failure.problem);
        focusLater(problemId);
      }
      showErrors(failure.errors);
    }
  };

  const openLink = (url: string) => {
    backend.openLink(url).catch((e: unknown) => toast.error(e));
  };

  const passwordHint = preset?.appPassword
    ? `Use an ${preset.appPassword.term}, not your usual ${preset.name} password.`
    : preset?.bridge
      ? "The password shown in Proton Mail Bridge, not your Proton password."
      : undefined;

  return (
    <StepLayout
      formId={formId}
      onSubmit={(e) => void onSubmit(e)}
      footer={
        <>
          {onBack && (
            <button
              type="button"
              className="btn btn--subtle add-account__back"
              onClick={onBack}
              disabled={isPending}
            >
              <ArrowLeft size={16} aria-hidden="true" />
              Back
            </button>
          )}
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="btn btn--primary" form={formId} disabled={isPending}>
            {isPending && <Spinner size={16} label="Checking" />}
            {isPending ? "Checking your settings…" : "Add account"}
          </button>
        </>
      }
    >
      <fieldset className="imap-form" disabled={isPending}>
        <legend className="visually-hidden">Account details</legend>
        <StepIntro icon={<AtSign size={20} />}>
          Works with Gmail, iCloud, Fastmail, Yahoo and most other providers. Enter your email
          address and we&apos;ll suggest the server settings.
        </StepIntro>
        <TextField
          id={ids.email}
          label="Email address"
          type="email"
          autoComplete="email"
          spellCheck={false}
          autoCapitalize="none"
          placeholder="name@example.com"
          value={email}
          error={errors.email}
          onChange={(e) => {
            setEmail(e.target.value);
            clearErrors(["email"]);
          }}
          data-autofocus
        />

        <div aria-live="polite">
          {preset && (
            <PresetNote
              preset={preset}
              onUseMicrosoft={() => onUseMicrosoft(email)}
              onOpenLink={openLink}
            />
          )}
        </div>

        <TextField
          id={ids.displayName}
          label="Your name"
          autoComplete="name"
          value={displayName}
          hint="Shown to people who receive your email."
          onChange={(e) => setDisplayName(e.target.value)}
        />

        <PasswordField
          id={ids.password}
          value={password}
          error={errors.password}
          hint={passwordHint}
          onChange={(value) => {
            setPassword(value);
            clearErrors(["password"]);
          }}
        />

        <div className={clsx("advanced", advancedOpen && "advanced--open")}>
          <button
            type="button"
            className="advanced__toggle"
            aria-expanded={advancedOpen}
            aria-controls={advancedId}
            onClick={() => setAdvancedOpen((o) => !o)}
          >
            <ChevronRight size={16} className="advanced__chevron" aria-hidden="true" />
            <span className="advanced__title">Advanced settings</span>
            {!advancedOpen && server.imapHost && (
              <span className="advanced__summary">
                {server.imapHost}:{server.imapPort} · {server.smtpHost}:{server.smtpPort}
              </span>
            )}
          </button>
          {advancedOpen && (
            <div id={advancedId} className="advanced__body">
              <ServerSettingsFields
                ids={ids}
                value={server}
                errors={errors}
                onChange={changeServer}
                onReset={
                  customized
                    ? () => {
                        setOverrides({});
                        clearErrors(SERVER_FIELDS);
                      }
                    : undefined
                }
              />
            </div>
          )}
        </div>

        {problem && <ProblemCallout id={problemId} problem={problem} />}

        {isPending && (
          <p className="visually-hidden" role="status">
            Checking your settings…
          </p>
        )}
      </fieldset>
    </StepLayout>
  );
}

function PresetNote({
  preset,
  onUseMicrosoft,
  onOpenLink,
}: {
  preset: ProviderPreset;
  onUseMicrosoft: () => void;
  onOpenLink: (url: string) => void;
}) {
  if (preset.preferMicrosoft) {
    return (
      <Callout
        tone="info"
        title={`${preset.name} works best with Microsoft sign-in`}
        action={
          <button type="button" className="btn btn--sm" onClick={onUseMicrosoft}>
            Use Microsoft sign-in
          </button>
        }
      >
        <p>
          Your calendar, contacts and tasks sync too, and {preset.name} usually doesn&apos;t accept
          passwords from other apps.
        </p>
      </Callout>
    );
  }
  if (preset.bridge) {
    return (
      <Callout
        tone="info"
        title="Proton Mail works through Proton Mail Bridge"
        action={
          <button
            type="button"
            className="btn btn--sm"
            onClick={() => onOpenLink(PROTON_BRIDGE_URL)}
          >
            <ExternalLink size={14} aria-hidden="true" />
            Get Proton Mail Bridge
          </button>
        }
      >
        <p>
          Bridge is an app from Proton (for paid plans) that runs on this computer and lets mail
          apps reach your encrypted mailbox. Start Bridge, then enter the password it shows you.
        </p>
      </Callout>
    );
  }
  if (preset.appPassword) {
    const { term, howTo, url } = preset.appPassword;
    return (
      <Callout
        tone="info"
        icon={<KeyRound size={16} />}
        title={`${preset.name} needs an ${term}`}
        action={
          url && (
            <button type="button" className="btn btn--sm" onClick={() => onOpenLink(url)}>
              <ExternalLink size={14} aria-hidden="true" />
              Create an {term}
            </button>
          )
        }
      >
        <p>
          An {term} is a separate password just for mail apps, so your main password stays private.{" "}
          {howTo}
        </p>
      </Callout>
    );
  }
  return null;
}
