import clsx from "clsx";
import {
  ArrowLeft,
  AtSign,
  ChevronRight,
  ExternalLink,
  Eye,
  EyeOff,
  KeyRound,
  RotateCcw,
} from "lucide-react";
import { useId, useState, type FormEvent } from "react";

import { useBackend } from "../../api/context";
import { errorMessage, hasErrorCode } from "../../api/errors";
import type { Account, ImapSettings, TlsMode } from "../../api/types";
import { Spinner } from "../../components/Spinner";
import { toast } from "../../components/toast";
import { Callout, SelectField, TextField } from "./Fields";
import { focusLater } from "./focus";
import {
  PROTON_BRIDGE_URL,
  TLS_LABELS,
  findPreset,
  isLoopbackHost,
  suggestedSettings,
  type ProviderPreset,
} from "./presets";
import { useAddImapAccount } from "./queries";
import { StepIntro, StepLayout } from "./StepLayout";
import { validateEmail, validateHost, validatePort } from "./validation";

/** Server settings as typed (ports are strings until submitted). */
interface ServerDraft {
  imapHost: string;
  imapPort: string;
  imapTls: TlsMode;
  smtpHost: string;
  smtpPort: string;
  smtpTls: TlsMode;
  username: string;
}

type FieldName = "email" | "displayName" | "password" | keyof ServerDraft;
type Errors = Partial<Record<FieldName, string>>;

const ADVANCED_FIELDS: FieldName[] = [
  "imapHost",
  "imapPort",
  "imapTls",
  "smtpHost",
  "smtpPort",
  "smtpTls",
  "username",
];
const FIELD_ORDER: FieldName[] = ["email", "displayName", "password", ...ADVANCED_FIELDS];

/** Usual port for each security mode, used to follow a security change. */
const DEFAULT_PORTS: Record<"imap" | "smtp", Record<TlsMode, string>> = {
  imap: { tls: "993", startTls: "143", none: "143" },
  smtp: { tls: "465", startTls: "587", none: "587" },
};

function toDraft(s: ImapSettings): ServerDraft {
  return { ...s, imapPort: String(s.imapPort), smtpPort: String(s.smtpPort) };
}

interface FormProblem {
  title: string;
  detail?: string;
  technical?: string;
}

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
  const ids = Object.fromEntries(FIELD_ORDER.map((f) => [f, `${uid}-${f}`])) as Record<
    FieldName,
    string
  >;
  const formId = `${uid}-form`;
  const advancedId = `${uid}-advanced`;
  const problemId = `${uid}-problem`;

  const [email, setEmail] = useState(initialEmail);
  const [displayName, setDisplayName] = useState("");
  const [password, setPassword] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [overrides, setOverrides] = useState<Partial<ServerDraft>>({});
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [errors, setErrors] = useState<Errors>({});
  const [problem, setProblem] = useState<FormProblem | null>(null);
  const { mutateAsync, isPending } = useAddImapAccount();

  const preset = findPreset(email);
  const server: ServerDraft = { ...toDraft(suggestedSettings(email)), ...overrides };
  const customised = Object.keys(overrides).length > 0;

  const clearError = (field: FieldName) =>
    setErrors((e) => {
      if (!e[field]) return e;
      const next = { ...e };
      delete next[field];
      return next;
    });

  const setServer = <K extends keyof ServerDraft>(field: K, value: ServerDraft[K]) => {
    setOverrides((o) => ({ ...o, [field]: value }));
    clearError(field);
  };

  const setSecurity = (kind: "imap" | "smtp", tls: TlsMode) => {
    const portField = kind === "imap" ? "imapPort" : "smtpPort";
    const tlsField = kind === "imap" ? "imapTls" : "smtpTls";
    const previousDefault = DEFAULT_PORTS[kind][server[tlsField]];
    const next: Partial<ServerDraft> = { [tlsField]: tls };
    if (server[portField] === previousDefault) next[portField] = DEFAULT_PORTS[kind][tls];
    setOverrides((o) => ({ ...o, ...next }));
    clearError(tlsField);
    clearError(portField);
  };

  const validate = (): Errors => {
    const found: Errors = {};
    const emailError = validateEmail(email);
    if (emailError) found.email = emailError;
    if (!password) {
      found.password = preset?.appPassword
        ? `Enter your ${preset.name} ${preset.appPassword.term}.`
        : "Enter your password.";
    }
    for (const field of ["imapHost", "smtpHost"] as const) {
      const error = validateHost(server[field]);
      if (error) found[field] = error;
    }
    for (const field of ["imapPort", "smtpPort"] as const) {
      const error = validatePort(server[field]);
      if (error) found[field] = error;
    }
    for (const [tls, host] of [
      ["imapTls", server.imapHost],
      ["smtpTls", server.smtpHost],
    ] as const) {
      if (server[tls] === "none" && host.trim() && !isLoopbackHost(host)) {
        found[tls] =
          "Unencrypted connections are only allowed to this computer. Choose SSL/TLS or STARTTLS.";
      }
    }
    if (!server.username.trim())
      found.username = "Enter your user name, usually your email address.";
    return found;
  };

  const showErrors = (found: Errors) => {
    setErrors(found);
    const first = FIELD_ORDER.find((f) => found[f]);
    if (!first) return;
    if (ADVANCED_FIELDS.includes(first)) setAdvancedOpen(true);
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
        settings: {
          imapHost: server.imapHost.trim(),
          imapPort: Number(server.imapPort),
          imapTls: server.imapTls,
          smtpHost: server.smtpHost.trim(),
          smtpPort: Number(server.smtpPort),
          smtpTls: server.smtpTls,
          username: server.username.trim(),
        },
      });
      onDone(account);
    } catch (error) {
      handleFailure(error);
    }
  };

  const handleFailure = (error: unknown) => {
    if (hasErrorCode(error, "authRequired")) {
      const usernameNote =
        server.username.trim().toLowerCase() !== email.trim().toLowerCase()
          ? " Also check the user name under Advanced settings."
          : "";
      showErrors({
        password:
          (preset?.appPassword
            ? `${preset.name} didn't accept this password. Make sure you're using an ${preset.appPassword.term}, not your usual password.`
            : preset?.bridge
              ? "Proton Mail Bridge didn't accept this password. Use the password shown in Bridge, not your Proton password."
              : "The server didn't accept this email address and password. Check them and try again.") +
          usernameNote,
      });
      return;
    }
    if (hasErrorCode(error, "network")) {
      setAdvancedOpen(true);
      setProblem({
        title: preset?.bridge
          ? "Couldn't connect to Proton Mail Bridge"
          : "Couldn't connect to the mail server",
        detail: preset?.bridge
          ? "Make sure Bridge is running and signed in, and that the ports match the ones Bridge shows."
          : "Check the server names and ports under Advanced settings, and that you're online. Your provider's help pages list the right settings.",
        technical: error.message,
      });
      focusLater(problemId);
      return;
    }
    if (hasErrorCode(error, "invalidInput")) {
      const message = error.message;
      if (/already been added|email address/i.test(message)) {
        showErrors({ email: message });
      } else if (/unencrypted/i.test(message)) {
        showErrors(server.imapTls === "none" ? { imapTls: message } : { smtpTls: message });
      } else if (/server/i.test(message)) {
        showErrors({ imapHost: message });
      } else {
        setProblem({ title: message });
        focusLater(problemId);
      }
      return;
    }
    setProblem({
      title: "Couldn't add this account",
      detail: "Something went wrong while checking your settings. Please try again.",
      technical: errorMessage(error),
    });
    focusLater(problemId);
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
            clearError("email");
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

        <TextField
          id={ids.password}
          label="Password"
          type={showPassword ? "text" : "password"}
          autoComplete="current-password"
          spellCheck={false}
          value={password}
          error={errors.password}
          hint={passwordHint}
          onChange={(e) => {
            setPassword(e.target.value);
            clearError("password");
          }}
          trailing={
            <button
              type="button"
              className="icon-btn icon-btn--sm input-wrap__button"
              aria-label={showPassword ? "Hide password" : "Show password"}
              aria-pressed={showPassword}
              aria-controls={ids.password}
              onClick={() => setShowPassword((s) => !s)}
            >
              {showPassword ? <EyeOff size={16} /> : <Eye size={16} />}
            </button>
          }
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
              <ServerGroup
                kind="imap"
                title="Incoming mail (IMAP)"
                ids={ids}
                host={server.imapHost}
                port={server.imapPort}
                tls={server.imapTls}
                errors={errors}
                onHost={(v) => setServer("imapHost", v)}
                onPort={(v) => setServer("imapPort", v)}
                onTls={(v) => setSecurity("imap", v)}
              />
              <ServerGroup
                kind="smtp"
                title="Outgoing mail (SMTP)"
                ids={ids}
                host={server.smtpHost}
                port={server.smtpPort}
                tls={server.smtpTls}
                errors={errors}
                onHost={(v) => setServer("smtpHost", v)}
                onPort={(v) => setServer("smtpPort", v)}
                onTls={(v) => setSecurity("smtp", v)}
              />
              <TextField
                id={ids.username}
                label="User name"
                autoComplete="username"
                spellCheck={false}
                autoCapitalize="none"
                value={server.username}
                error={errors.username}
                hint="Used for both servers. Usually your email address."
                onChange={(e) => setServer("username", e.target.value)}
              />
              {customised && (
                <button
                  type="button"
                  className="btn btn--subtle btn--sm advanced__reset"
                  onClick={() => {
                    setOverrides({});
                    setErrors((e) => {
                      const next = { ...e };
                      for (const f of ADVANCED_FIELDS) delete next[f];
                      return next;
                    });
                  }}
                >
                  <RotateCcw size={14} aria-hidden="true" />
                  Use suggested settings
                </button>
              )}
            </div>
          )}
        </div>

        {problem && (
          <div id={problemId} tabIndex={-1} className="imap-form__problem">
            <Callout tone="danger" role="alert" title={problem.title}>
              {problem.detail && <p>{problem.detail}</p>}
              {problem.technical && (
                <p className="callout__technical selectable">{problem.technical}</p>
              )}
            </Callout>
          </div>
        )}

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
          Your calendar, contacts and To Do sync too, and {preset.name} usually doesn&apos;t accept
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

interface ServerGroupProps {
  kind: "imap" | "smtp";
  title: string;
  ids: Record<FieldName, string>;
  host: string;
  port: string;
  tls: TlsMode;
  errors: Errors;
  onHost: (value: string) => void;
  onPort: (value: string) => void;
  onTls: (value: TlsMode) => void;
}

function ServerGroup({
  kind,
  title,
  ids,
  host,
  port,
  tls,
  errors,
  onHost,
  onPort,
  onTls,
}: ServerGroupProps) {
  const hostField = kind === "imap" ? "imapHost" : "smtpHost";
  const portField = kind === "imap" ? "imapPort" : "smtpPort";
  const tlsField = kind === "imap" ? "imapTls" : "smtpTls";
  const label = kind === "imap" ? "IMAP" : "SMTP";
  return (
    <fieldset className="server-group">
      <legend className="server-group__title">{title}</legend>
      <div className="server-group__row">
        <TextField
          id={ids[hostField]}
          fieldClassName="server-group__host"
          label={`${label} server`}
          spellCheck={false}
          autoCapitalize="none"
          value={host}
          error={errors[hostField]}
          onChange={(e) => onHost(e.target.value)}
        />
        <TextField
          id={ids[portField]}
          fieldClassName="server-group__port"
          label="Port"
          inputMode="numeric"
          value={port}
          error={errors[portField]}
          onChange={(e) => onPort(e.target.value.replace(/[^\d]/g, ""))}
        />
        <SelectField
          id={ids[tlsField]}
          fieldClassName="server-group__tls"
          label="Security"
          value={tls}
          error={errors[tlsField]}
          onChange={(e) => onTls(e.target.value as TlsMode)}
        >
          {(Object.keys(TLS_LABELS) as TlsMode[]).map((mode) => (
            <option key={mode} value={mode}>
              {TLS_LABELS[mode]}
            </option>
          ))}
        </SelectField>
      </div>
      {tls === "none" && !errors[tlsField] && (
        <Callout tone="warning" className="server-group__warning">
          <p>
            Without encryption your password is sent as plain text, so this is only allowed for
            servers on this computer, like Proton Mail Bridge.
          </p>
        </Callout>
      )}
    </fieldset>
  );
}
