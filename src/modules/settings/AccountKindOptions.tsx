import clsx from "clsx";
import { ArrowRight, AtSign, BriefcaseBusiness, ChevronRight } from "lucide-react";
import { useId, type ReactNode } from "react";

import "./settings.css";

export type AccountKindChoice = "microsoft" | "imap";

interface Option {
  kind: AccountKindChoice;
  title: string;
  description: string;
  icon: ReactNode;
  examples: string[];
}

const OPTIONS: Option[] = [
  {
    kind: "microsoft",
    title: "Microsoft 365 or Outlook.com",
    description:
      "Work, school or personal Microsoft account. Mail, calendar, contacts and tasks all stay in sync.",
    icon: <BriefcaseBusiness size={22} strokeWidth={1.75} />,
    examples: ["Microsoft 365", "Outlook.com", "Hotmail", "Live"],
  },
  {
    kind: "imap",
    title: "Other email (IMAP)",
    description:
      "Any provider that supports IMAP. Your calendar, contacts and tasks are kept on this computer.",
    icon: <AtSign size={22} strokeWidth={1.75} />,
    examples: ["Gmail", "Fastmail", "iCloud", "Yahoo", "Self-hosted"],
  },
];

interface AccountKindOptionsProps {
  /** "cards": side-by-side tiles (welcome); "list": stacked tiles (dialog). */
  variant: "cards" | "list";
  onChoose: (kind: AccountKindChoice) => void;
  /** Mark the first option for initial focus. */
  autoFocus?: boolean;
}

/** The two ways to add an account: Microsoft sign-in or IMAP. */
export function AccountKindOptions({ variant, onChoose, autoFocus }: AccountKindOptionsProps) {
  const id = useId();
  return (
    <div className={clsx("kind-options", `kind-options--${variant}`)}>
      {OPTIONS.map((option, index) => (
        <button
          key={option.kind}
          type="button"
          className={clsx("kind-option", `kind-option--${option.kind}`)}
          aria-labelledby={`${id}-${option.kind}-title`}
          aria-describedby={`${id}-${option.kind} ${id}-${option.kind}-examples`}
          data-autofocus={autoFocus && index === 0 ? true : undefined}
          onClick={() => onChoose(option.kind)}
        >
          <span className="kind-option__icon" aria-hidden="true">
            {option.icon}
          </span>
          <span className="kind-option__text">
            <span id={`${id}-${option.kind}-title`} className="kind-option__title">
              {option.title}
            </span>
            <span id={`${id}-${option.kind}`} className="kind-option__description">
              {option.description}
            </span>
            <span id={`${id}-${option.kind}-examples`} className="kind-option__examples">
              <span className="visually-hidden">For example: </span>
              {option.examples.map((e) => (
                <span key={e} className="kind-option__example">
                  {e}
                </span>
              ))}
            </span>
          </span>
          <span className="kind-option__arrow" aria-hidden="true">
            {variant === "cards" ? (
              <ArrowRight size={18} strokeWidth={1.75} />
            ) : (
              <ChevronRight size={18} strokeWidth={1.75} />
            )}
          </span>
        </button>
      ))}
    </div>
  );
}
