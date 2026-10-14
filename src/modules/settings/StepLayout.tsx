import clsx from "clsx";
import type { FormEvent, ReactNode } from "react";

interface StepLayoutProps {
  children: ReactNode;
  footer: ReactNode;
  /** Wraps the step in a <form> so Enter submits it. */
  onSubmit?: (event: FormEvent<HTMLFormElement>) => void;
  formId?: string;
  className?: string;
}

/** A step of the add-account dialog: scrolling content and a fixed footer. */
export function StepLayout({ children, footer, onSubmit, formId, className }: StepLayoutProps) {
  const inner = (
    <>
      <div className={clsx("add-account__content", className)}>{children}</div>
      <div className="add-account__footer">{footer}</div>
    </>
  );
  if (!onSubmit) return <div className="add-account__step">{inner}</div>;
  return (
    <form id={formId} className="add-account__step" noValidate onSubmit={onSubmit}>
      {inner}
    </form>
  );
}

/** Icon and a sentence introducing a form step. */
export function StepIntro({ icon, children }: { icon: ReactNode; children: ReactNode }) {
  return (
    <div className="step-intro">
      <span className="step-intro__icon" aria-hidden="true">
        {icon}
      </span>
      <p className="step-intro__text">{children}</p>
    </div>
  );
}

interface ResultPanelProps {
  icon: ReactNode;
  tone: "brand" | "neutral" | "warning" | "danger";
  title: string;
  children?: ReactNode;
  /** Center short messages; align longer explanations to the start. */
  align?: "center" | "start";
}

/** Icon, heading and text for the waiting and sign-in problem screens. */
export function ResultPanel({ icon, tone, title, children, align = "center" }: ResultPanelProps) {
  return (
    <div className={clsx("result-panel", `result-panel--${align}`)}>
      <div className={clsx("result-panel__icon", `result-panel__icon--${tone}`)} aria-hidden="true">
        {icon}
      </div>
      <h3 className="result-panel__title" tabIndex={-1} data-autofocus>
        {title}
      </h3>
      {children && <div className="result-panel__body">{children}</div>}
    </div>
  );
}
