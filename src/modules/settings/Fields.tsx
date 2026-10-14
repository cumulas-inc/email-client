import clsx from "clsx";
import { CircleAlert, Eye, EyeOff, Info, TriangleAlert } from "lucide-react";
import {
  useState,
  type InputHTMLAttributes,
  type ReactNode,
  type SelectHTMLAttributes,
} from "react";

import type { FormProblem } from "./serverDraft";

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

interface PasswordFieldProps {
  id: string;
  value: string;
  onChange: (value: string) => void;
  label?: string;
  hint?: ReactNode;
  error?: string | null;
  placeholder?: string;
  autoComplete?: string;
}

/** Password input with a show/hide button. */
export function PasswordField({
  id,
  value,
  onChange,
  label = "Password",
  hint,
  error,
  placeholder,
  autoComplete = "current-password",
}: PasswordFieldProps) {
  const [visible, setVisible] = useState(false);
  return (
    <TextField
      id={id}
      label={label}
      type={visible ? "text" : "password"}
      autoComplete={autoComplete}
      spellCheck={false}
      placeholder={placeholder}
      value={value}
      error={error}
      hint={hint}
      onChange={(e) => onChange(e.target.value)}
      trailing={
        <button
          type="button"
          className="icon-btn icon-btn--sm input-wrap__button"
          aria-label={visible ? "Hide password" : "Show password"}
          aria-pressed={visible}
          aria-controls={id}
          onClick={() => setVisible((v) => !v)}
        >
          {visible ? <EyeOff size={16} /> : <Eye size={16} />}
        </button>
      }
    />
  );
}

/** A failed check that isn't about one field; focusable so it can be announced. */
export function ProblemCallout({ id, problem }: { id: string; problem: FormProblem }) {
  return (
    <div id={id} tabIndex={-1} className="imap-form__problem">
      <Callout tone="danger" role="alert" title={problem.title}>
        {problem.detail && <p>{problem.detail}</p>}
        {problem.technical && <p className="callout__technical selectable">{problem.technical}</p>}
      </Callout>
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
