import clsx from "clsx";
import { useId, type ReactNode } from "react";

import "./controls.css";

export interface SegmentedOption<T extends string> {
  value: T;
  label: string;
  icon?: ReactNode;
}

interface SegmentedControlProps<T extends string> {
  value: T;
  options: SegmentedOption<T>[];
  onChange: (value: T) => void;
  /** Accessible name, used when `labelledBy` is not given. */
  label?: string;
  labelledBy?: string;
  describedBy?: string;
  /** "segments": a compact row; "tiles": larger buttons with the icon on top. */
  variant?: "segments" | "tiles";
  disabled?: boolean;
}

/**
 * A single choice between a few options, drawn as joined buttons. Built on
 * native radio buttons, so arrow keys move the selection.
 */
export function SegmentedControl<T extends string>({
  value,
  options,
  onChange,
  label,
  labelledBy,
  describedBy,
  variant = "segments",
  disabled,
}: SegmentedControlProps<T>) {
  const name = useId();
  return (
    <div
      role="radiogroup"
      aria-label={labelledBy ? undefined : label}
      aria-labelledby={labelledBy}
      aria-describedby={describedBy}
      className={clsx("segmented", `segmented--${variant}`)}
    >
      {options.map((option) => {
        const selected = option.value === value;
        return (
          <label
            key={option.value}
            className={clsx("segmented__option", selected && "segmented__option--selected")}
          >
            <input
              type="radio"
              className="segmented__input"
              name={name}
              value={option.value}
              checked={selected}
              disabled={disabled}
              onChange={() => onChange(option.value)}
            />
            {option.icon && (
              <span className="segmented__icon" aria-hidden="true">
                {option.icon}
              </span>
            )}
            <span className="segmented__label">{option.label}</span>
          </label>
        );
      })}
    </div>
  );
}
