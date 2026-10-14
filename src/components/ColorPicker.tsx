import { Check } from "lucide-react";
import { useId, type CSSProperties } from "react";

import "./controls.css";

export interface ColorOption {
  /** CSS colour, e.g. "#0f6cbd". */
  value: string;
  /** Accessible name, e.g. "Blue". */
  name: string;
}

interface ColorPickerProps {
  value: string;
  options: ColorOption[];
  onChange: (value: string) => void;
  label?: string;
  labelledBy?: string;
  disabled?: boolean;
}

/** A row of colour swatches; native radios, so arrow keys work. */
export function ColorPicker({
  value,
  options,
  onChange,
  label,
  labelledBy,
  disabled,
}: ColorPickerProps) {
  const name = useId();
  const current = value.toLowerCase();
  const known = options.some((o) => o.value.toLowerCase() === current);
  const all = known || !value ? options : [{ value, name: "Current colour" }, ...options];
  return (
    <div
      role="radiogroup"
      aria-label={labelledBy ? undefined : label}
      aria-labelledby={labelledBy}
      className="color-picker"
    >
      {all.map((option) => {
        const selected = option.value.toLowerCase() === current;
        return (
          <label
            key={option.value}
            className="color-picker__swatch"
            title={option.name}
            style={{ "--swatch": option.value } as CSSProperties}
          >
            <input
              type="radio"
              className="color-picker__input"
              name={name}
              value={option.value}
              checked={selected}
              disabled={disabled}
              aria-label={option.name}
              onChange={() => onChange(option.value)}
            />
            <span className="color-picker__dot" aria-hidden="true">
              {selected && <Check size={14} strokeWidth={3} />}
            </span>
          </label>
        );
      })}
    </div>
  );
}
