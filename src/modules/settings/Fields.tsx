import clsx from "clsx";
import { CircleAlert, Info, TriangleAlert } from "lucide-react";
import type { InputHTMLAttributes, ReactNode, SelectHTMLAttributes } from "react";

import "./settings.css";

function describedBy(...ids: (string | false | null | undefined)[]): string | undefined {
  return ids.filter(Boolean).join(" ") || undefined;
}

export function FieldError({ id, children }: { id: string; children: ReactNode }) {
  return (
    <p id={id} className="field__error form-error">
      <CircleAlert size={14} aria-hidden="true" />
      <span>{children}</span>
    </p>
  );
}

interface TextFieldProps extends Omit<InputHTMLAttributes<HTMLInputElement>, "id"> {
  id: string;
  label: ReactNode;
  hint?: ReactNode;
  error?: string | null;
  /** Rendered inside the input box, on the right (e.g. a show/hide button). */
  trailing?: ReactNode;
  fieldClassName?: string;
}

/** Labelled input with its hint and error linked through aria-describedby. */
export function TextField({
  id,
  label,
  hint,
  error,
  trailing,
  fieldClassName,
  className,
  ...input
}: TextFieldProps) {
  const hintId = `${id}-hint`;
  const errorId = `${id}-error`;
  return (
    <div className={clsx("field", fieldClassName)}>
      <label htmlFor={id} className="field__label">
        {label}
      </label>
      <div className={clsx("input-wrap", trailing != null && "input-wrap--trailing")}>
        <input
          id={id}
          className={clsx("input", className)}
          aria-invalid={error ? true : undefined}
          aria-describedby={describedBy(error && errorId, hint != null && hintId)}
          {...input}
        />
        {trailing}
      </div>
      {error && <FieldError id={errorId}>{error}</FieldError>}
      {hint != null && (
        <p id={hintId} className="field__hint">
          {hint}
        </p>
      )}
    </div>
  );
}

interface SelectFieldProps extends Omit<SelectHTMLAttributes<HTMLSelectElement>, "id"> {
  id: string;
  label: ReactNode;
  hint?: ReactNode;
  error?: string | null;
  fieldClassName?: string;
}

export function SelectField({
  id,
  label,
  hint,
  error,
  fieldClassName,
  className,
  children,
  ...select
}: SelectFieldProps) {
  const hintId = `${id}-hint`;
  const errorId = `${id}-error`;
  return (
    <div className={clsx("field", fieldClassName)}>
      <label htmlFor={id} className="field__label">
        {label}
      </label>
      <select
        id={id}
        className={clsx("select", className)}
        aria-invalid={error ? true : undefined}
        aria-describedby={describedBy(error && errorId, hint != null && hintId)}
        {...select}
      >
        {children}
      </select>
      {error && <FieldError id={errorId}>{error}</FieldError>}
      {hint != null && (
        <p id={hintId} className="field__hint">
          {hint}
        </p>
      )}
    </div>
  );
}

interface CalloutProps {
  tone?: "info" | "warning" | "danger";
  title?: ReactNode;
  children?: ReactNode;
  action?: ReactNode;
  icon?: ReactNode;
  id?: string;
  role?: "alert" | "status";
  className?: string;
}

/** A tinted box for tips, warnings and errors. */
export function Callout({
  tone = "info",
  title,
  children,
  action,
  icon,
  id,
  role,
  className,
}: CalloutProps) {
  const defaultIcon =
    tone === "info" ? (
      <Info size={16} />
    ) : tone === "warning" ? (
      <TriangleAlert size={16} />
    ) : (
      <CircleAlert size={16} />
    );
  return (
    <div id={id} role={role} className={clsx("callout", `callout--${tone}`, className)}>
      <span className="callout__icon" aria-hidden="true">
        {icon ?? defaultIcon}
      </span>
      <div className="callout__content">
        {title && <p className="callout__title">{title}</p>}
        {children && <div className="callout__body">{children}</div>}
        {action && <div className="callout__action">{action}</div>}
      </div>
    </div>
  );
}
