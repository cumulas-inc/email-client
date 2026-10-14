import { Building, RotateCcw } from "lucide-react";
import { useId, useState, type KeyboardEvent } from "react";

import { useAppInfo, useSettings } from "../../../api/queries";
import type { Settings } from "../../../api/types";
import { PaneSpinner } from "../../../components/Spinner";
import { Callout, SelectField, TextField } from "../Fields";
import { useSaveSettings } from "../saveStatus";
import { SettingsGroup } from "../SettingsLayout";
import { isGuid, validateClientId, validateTenant } from "../validation";

type TenantMode = "common" | "organizations" | "consumers" | "custom";

const TENANT_OPTIONS: { value: TenantMode; label: string }[] = [
  { value: "common", label: "Any Microsoft account (common)" },
  { value: "organizations", label: "Work or school accounts only (organizations)" },
  { value: "consumers", label: "Personal accounts only (consumers)" },
  { value: "custom", label: "One organization (tenant ID or domain)" },
];

function tenantMode(tenant: string | null): TenantMode {
  const t = tenant?.trim().toLowerCase() ?? "";
  if (!t || t === "common") return "common";
  if (t === "organizations" || t === "consumers") return t;
  return "custom";
}

export function MicrosoftSection() {
  const { data: settings } = useSettings();
  const { data: info } = useAppInfo();
  if (!settings || !info) return <PaneSpinner />;
  return <MicrosoftForm settings={settings} builtIn={info.microsoftClientIdBuiltIn} />;
}

function MicrosoftForm({ settings, builtIn }: { settings: Settings; builtIn: boolean }) {
  const save = useSaveSettings();
  const clientIdId = useId();
  const tenantModeId = useId();
  const tenantId = useId();

  const [clientId, setClientId] = useState(settings.microsoftClientId ?? "");
  const [clientIdError, setClientIdError] = useState<string | null>(null);
  const [mode, setMode] = useState<TenantMode>(tenantMode(settings.microsoftTenant));
  const [customTenant, setCustomTenant] = useState(
    tenantMode(settings.microsoftTenant) === "custom" ? (settings.microsoftTenant ?? "") : "",
  );
  const [tenantError, setTenantError] = useState<string | null>(null);

  const saveClientId = (value: string) => {
    const next = value.trim() ? value.trim().toLowerCase() : null;
    if (next !== settings.microsoftClientId) void save({ microsoftClientId: next });
  };

  const commitClientId = () => {
    const error = validateClientId(clientId);
    setClientIdError(error);
    if (!error) saveClientId(clientId);
  };

  const commitTenant = () => {
    const error = validateTenant(customTenant);
    setTenantError(error);
    if (!error && customTenant.trim() !== settings.microsoftTenant) {
      void save({ microsoftTenant: customTenant.trim() });
    }
  };

  const onEnter = (commit: () => void) => (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      e.preventDefault();
      commit();
    }
  };

  const overridden = Boolean(settings.microsoftClientId || settings.microsoftTenant);
  const missing = !builtIn && !settings.microsoftClientId;

  return (
    <>
      <p className="settings-intro">
        Cumulas Mail signs in to Microsoft 365 and Outlook.com as a registered app. Most people
        never need to change this. Your IT team may ask you to use your organization&apos;s own
        registration.
      </p>

      {missing && (
        <Callout tone="warning" title="Microsoft sign-in needs an application ID">
          <p>
            This build of Cumulas Mail doesn&apos;t include one, so add your organization&apos;s
            application ID below before signing in.
          </p>
        </Callout>
      )}

      <SettingsGroup title="App registration">
        <div className="settings-form">
          <TextField
            id={clientIdId}
            label="Application (client) ID"
            spellCheck={false}
            autoCapitalize="none"
            autoComplete="off"
            className="mono-input"
            placeholder={builtIn ? "Using the built-in ID" : "00000000-0000-0000-0000-000000000000"}
            value={clientId}
            error={clientIdError}
            hint={
              builtIn
                ? "Leave empty to use the ID built into Cumulas Mail. You'll find your own on the app's Overview page in the Microsoft Entra admin center."
                : "Find it on the app's Overview page in the Microsoft Entra admin center."
            }
            onChange={(e) => {
              const value = e.target.value;
              setClientId(value);
              if (clientIdError) setClientIdError(validateClientId(value));
              // A complete ID can be saved straight away.
              if (isGuid(value)) {
                setClientIdError(null);
                saveClientId(value);
              }
            }}
            onBlur={commitClientId}
            onKeyDown={onEnter(commitClientId)}
          />

          <SelectField
            id={tenantModeId}
            label="Who can sign in"
            value={mode}
            hint="Leave this on “Any Microsoft account” unless your organization tells you otherwise."
            onChange={(e) => {
              const next = e.target.value as TenantMode;
              setMode(next);
              setTenantError(null);
              if (next === "custom") {
                window.setTimeout(() => document.getElementById(tenantId)?.focus(), 0);
                if (customTenant.trim() && !validateTenant(customTenant)) {
                  void save({ microsoftTenant: customTenant.trim() });
                }
              } else {
                void save({ microsoftTenant: next === "common" ? null : next });
              }
            }}
          >
            {TENANT_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </SelectField>

          {mode === "custom" && (
            <TextField
              id={tenantId}
              label="Tenant ID or domain"
              spellCheck={false}
              autoCapitalize="none"
              autoComplete="off"
              className="mono-input"
              placeholder="contoso.com"
              value={customTenant}
              error={tenantError}
              hint="For example contoso.onmicrosoft.com, or the directory (tenant) ID from Entra."
              onChange={(e) => {
                setCustomTenant(e.target.value);
                if (tenantError) setTenantError(validateTenant(e.target.value));
              }}
              onBlur={commitTenant}
              onKeyDown={onEnter(commitTenant)}
            />
          )}

          <p className="settings-form__note">
            Changes apply the next time you sign in. Microsoft accounts you&apos;ve already added
            may need to sign in again.
          </p>

          {builtIn && overridden && (
            <div>
              <button
                type="button"
                className="btn btn--sm"
                onClick={() => {
                  setClientId("");
                  setClientIdError(null);
                  setMode("common");
                  setCustomTenant("");
                  setTenantError(null);
                  void save({ microsoftClientId: null, microsoftTenant: null });
                }}
              >
                <RotateCcw size={14} aria-hidden="true" />
                Use the built-in registration
              </button>
            </div>
          )}
        </div>
      </SettingsGroup>

      <Callout tone="info" icon={<Building size={16} />} title="For IT administrators">
        <p>
          You can register Cumulas Mail in your own Microsoft Entra ID tenant and grant admin
          consent once for everyone, so nobody sees an approval request. Step-by-step instructions
          are in docs/microsoft-365.md.
        </p>
      </Callout>
    </>
  );
}
