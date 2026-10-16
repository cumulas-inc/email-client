import clsx from "clsx";
import { Check, CircleAlert, LoaderCircle } from "lucide-react";
import { useId, type ReactNode } from "react";

import { useSaveStatus } from "./saveStatus";

interface SettingsGroupProps {
  title?: string;
  description?: ReactNode;
  children: ReactNode;
}

/** A titled card of setting rows. */
export function SettingsGroup({ title, description, children }: SettingsGroupProps) {
  const id = useId();
  return (
    <section className="settings-group" aria-labelledby={title ? id : undefined}>
      {title && (
        <h4 id={id} className="settings-group__title">
          {title}
        </h4>
      )}
      {description && <div className="settings-group__description">{description}</div>}
      <div className="settings-card">{children}</div>
    </section>
  );
}

export interface SettingIds {
  labelId: string;
  descriptionId: string | undefined;
  controlId: string;
}

interface SettingRowProps {
  label: ReactNode;
  description?: ReactNode;
  /** Put the control under the text instead of on the right. */
  stacked?: boolean;
  /** Render the label as a <label for=…> (inputs, selects) or plain text (groups). */
  labelFor?: boolean;
  children: (ids: SettingIds) => ReactNode;
}

/** One setting: label and description on the left, its control on the right. */
export function SettingRow({
  label,
  description,
  stacked,
  labelFor = true,
  children,
}: SettingRowProps) {
  const id = useId();
  const ids: SettingIds = {
    labelId: `${id}-label`,
    descriptionId: description ? `${id}-description` : undefined,
    controlId: `${id}-control`,
  };
  return (
    <div className={clsx("setting-row", stacked && "setting-row--stacked")}>
      <div className="setting-row__text">
        {labelFor ? (
          <label id={ids.labelId} htmlFor={ids.controlId} className="setting-row__label">
            {label}
          </label>
        ) : (
          <span id={ids.labelId} className="setting-row__label">
            {label}
          </span>
        )}
        {description && (
          <div id={ids.descriptionId} className="setting-row__description">
            {description}
          </div>
        )}
      </div>
      <div className="setting-row__control">{children(ids)}</div>
    </div>
  );
}

/** "Saving…" / "Saved" next to the section title; errors are announced. */
export function SaveIndicator() {
  const { state, error } = useSaveStatus();
  return (
    <>
      <div className="save-indicator" role="status" aria-live="polite">
        {state === "saving" && (
          <span className="save-indicator__saving">
            <LoaderCircle size={14} className="spin" aria-hidden="true" />
            Saving…
          </span>
        )}
        {state === "saved" && (
          <span className="save-indicator__saved">
            <Check size={14} aria-hidden="true" />
            Saved
          </span>
        )}
      </div>
      {state === "error" && (
        <div className="save-indicator save-indicator--error" role="alert">
          <CircleAlert size={14} aria-hidden="true" />
          {error}
        </div>
      )}
    </>
  );
}
