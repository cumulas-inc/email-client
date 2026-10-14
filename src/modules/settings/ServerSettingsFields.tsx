import { RotateCcw } from "lucide-react";

import type { TlsMode } from "../../api/types";
import { Callout, SelectField, TextField } from "./Fields";
import { TLS_LABELS } from "./presets";
import { securityChange, type ServerDraft, type ServerField } from "./serverDraft";

interface ServerSettingsFieldsProps {
  /** Element ids for each field (for labels, errors and focus). */
  ids: Record<ServerField, string>;
  value: ServerDraft;
  errors: Partial<Record<ServerField, string>>;
  /** Called with the fields that changed. */
  onChange: (change: Partial<ServerDraft>) => void;
  /** Shows a "Use suggested settings" button when given. */
  onReset?: () => void;
}

/** Incoming and outgoing server, port and security, and the user name. */
export function ServerSettingsFields({
  ids,
  value,
  errors,
  onChange,
  onReset,
}: ServerSettingsFieldsProps) {
  return (
    <>
      <ServerGroup
        kind="imap"
        title="Incoming mail (IMAP)"
        ids={ids}
        value={value}
        errors={errors}
        onChange={onChange}
      />
      <ServerGroup
        kind="smtp"
        title="Outgoing mail (SMTP)"
        ids={ids}
        value={value}
        errors={errors}
        onChange={onChange}
      />
      <TextField
        id={ids.username}
        label="User name"
        autoComplete="username"
        spellCheck={false}
        autoCapitalize="none"
        value={value.username}
        error={errors.username}
        hint="Used for both servers. Usually your email address."
        onChange={(e) => onChange({ username: e.target.value })}
      />
      {onReset && (
        <button type="button" className="btn btn--subtle btn--sm advanced__reset" onClick={onReset}>
          <RotateCcw size={14} aria-hidden="true" />
          Use suggested settings
        </button>
      )}
    </>
  );
}

interface ServerGroupProps {
  kind: "imap" | "smtp";
  title: string;
  ids: Record<ServerField, string>;
  value: ServerDraft;
  errors: Partial<Record<ServerField, string>>;
  onChange: (change: Partial<ServerDraft>) => void;
}

function ServerGroup({ kind, title, ids, value, errors, onChange }: ServerGroupProps) {
  const hostField = kind === "imap" ? "imapHost" : "smtpHost";
  const portField = kind === "imap" ? "imapPort" : "smtpPort";
  const tlsField = kind === "imap" ? "imapTls" : "smtpTls";
  const label = kind === "imap" ? "IMAP" : "SMTP";
  const tls = value[tlsField];
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
          value={value[hostField]}
          error={errors[hostField]}
          onChange={(e) => onChange({ [hostField]: e.target.value })}
        />
        <TextField
          id={ids[portField]}
          fieldClassName="server-group__port"
          label="Port"
          inputMode="numeric"
          value={value[portField]}
          error={errors[portField]}
          onChange={(e) => onChange({ [portField]: e.target.value.replace(/[^\d]/g, "") })}
        />
        <SelectField
          id={ids[tlsField]}
          fieldClassName="server-group__tls"
          label="Security"
          value={tls}
          error={errors[tlsField]}
          onChange={(e) => onChange(securityChange(value, kind, e.target.value as TlsMode))}
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
