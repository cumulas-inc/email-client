import clsx from "clsx";
import { Check, ClipboardList, Copy } from "lucide-react";
import { type ReactNode, type RefObject, useEffect, useRef, useState } from "react";

import { useBackend } from "../../../api/context";
import type { OrgSetupRowState } from "../../../api/types";
import { toast } from "../../../components/toast";
import { copyLater } from "../../../lib/clipboard";
import { BUTTONS, CHIPS, DIALOG } from "./copy";

type ChipTone = "ok" | "neutral" | "attention" | "problem" | "busy";

/** How a state looks; its words (CHIPS) always go with it, never the color alone. */
const CHIP_TONES: Record<OrgSetupRowState, ChipTone> = {
  ok: "ok",
  // Not set up yet: Set up does it, nothing is wrong.
  missing: "neutral",
  differs: "attention",
  off: "attention",
  waiting: "busy",
  blocked: "problem",
  unknown: "neutral",
  info: "neutral",
};

/** A row's state, in words. */
export function Chip({ state }: { state: OrgSetupRowState }) {
  return (
    <span className={clsx("orgsetup-chip", `orgsetup-chip--${CHIP_TONES[state]}`)}>
      {CHIPS[state]}
    </span>
  );
}

/** Selects the text of `element`, for Ctrl+C when the clipboard refused. */
function selectText(element: HTMLElement | null) {
  if (!element) return;
  if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) {
    element.focus();
    element.select();
    return;
  }
  element.focus();
  window.getSelection()?.selectAllChildren(element);
}

interface CopyButtonProps {
  text: string;
  /** What it copies, read out ("Copy the pattern …"); the button shows `children`. */
  label?: string;
  /** Where the text is on screen, selected when the clipboard refuses. */
  source?: RefObject<HTMLElement | null>;
  className?: string;
  children?: ReactNode;
}

/** Copies a text that's already here, and says so for a moment (like the approval link's Copy). */
export function CopyButton({ text, label, source, className, children }: CopyButtonProps) {
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 2500);
    return () => window.clearTimeout(timer);
  }, [copied]);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
    } catch {
      selectText(source?.current ?? null);
      toast.info(DIALOG.copyFailed);
    }
  };

  return (
    <>
      <button
        type="button"
        className={clsx("btn", className)}
        aria-label={label}
        onClick={() => void copy()}
      >
        {copied ? <Check size={14} aria-hidden="true" /> : <Copy size={14} aria-hidden="true" />}
        {copied ? DIALOG.copied : (children ?? BUTTONS.copy)}
      </button>
      <span role="status" className="visually-hidden">
        {copied ? DIALOG.copied : ""}
      </span>
    </>
  );
}

/** "Copy setup details": what the setup sign-in and its calls did, for support. */
export function CopySetupDetails() {
  const backend = useBackend();
  const copy = () => {
    copyLater(backend.getOrgSetupDetails())
      .then(() => toast.success(DIALOG.copiedSetupDetails))
      .catch((e: unknown) => toast.error(e));
  };
  return (
    <button type="button" className="btn" onClick={copy}>
      <ClipboardList size={16} aria-hidden="true" />
      {BUTTONS.copySetupDetails}
    </button>
  );
}

/** A paragraph that is PowerShell to run, such as the command that removes an approval. */
const COMMAND = /^(\$\w+ ?=|[A-Z][a-z]+-[A-Z]\w+)/;

/**
 * A row's detail: paragraphs, separated by blank lines as the core writes
 * them; a paragraph of PowerShell is shown as code, with Copy.
 */
export function DetailText({ text }: { text: string }) {
  const paragraphs = text.split(/\n\s*\n/).filter((p) => p.trim() !== "");
  return (
    <div className="orgsetup-detail">
      {paragraphs.map((paragraph, i) =>
        COMMAND.test(paragraph) ? (
          <CommandBlock key={i} command={paragraph} />
        ) : (
          <p key={i} className="orgsetup-detail__text selectable">
            {paragraph}
          </p>
        ),
      )}
    </div>
  );
}

function CommandBlock({ command }: { command: string }) {
  const ref = useRef<HTMLPreElement>(null);
  return (
    <div className="orgsetup-code">
      <pre ref={ref} className="orgsetup-code__text selectable" tabIndex={-1}>
        {command}
      </pre>
      <CopyButton text={command} source={ref} className="btn--sm" />
    </div>
  );
}
