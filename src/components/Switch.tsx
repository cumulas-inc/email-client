import { useId, type ReactNode } from "react";

import "./controls.css";

interface SwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: ReactNode;
  description?: ReactNode;
  disabled?: boolean;
}

/** On/off toggle (a native checkbox with role="switch") with a label on the left. */
export function Switch({ checked, onChange, label, description, disabled }: SwitchProps) {
  const id = useId();
  const descriptionId = `${id}-description`;
  return (
    <div className="switch">
      <div className="switch__text">
        <label htmlFor={id} className="switch__label">
          {label}
        </label>
        {description && (
          <p id={descriptionId} className="switch__description">
            {description}
          </p>
        )}
      </div>
      <span className="switch__control">
        <span className="switch__state" aria-hidden="true">
          {checked ? "On" : "Off"}
        </span>
        <span className="switch__box">
          <input
            id={id}
            type="checkbox"
            role="switch"
            className="switch__input"
            checked={checked}
            disabled={disabled}
            aria-describedby={description ? descriptionId : undefined}
            onChange={(e) => onChange(e.target.checked)}
          />
          <span className="switch__track" aria-hidden="true">
            <span className="switch__thumb" />
          </span>
        </span>
      </span>
    </div>
  );
}
