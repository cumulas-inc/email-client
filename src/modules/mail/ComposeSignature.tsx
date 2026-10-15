// The signature in the composer: editable in place, removable and
// restorable, or a note that the organization signs on send.

import { Signature, X } from "lucide-react";
import { useEffect, useId, useRef } from "react";

import { useMailStore } from "./mailStore";
import type { SignatureView } from "./signature";
import { SignatureEditor } from "./SignatureEditor";

/**
 * The signature between what's written and the quoted original. Its
 * buttons are `disabled` while Send waits for the signature.
 */
export function ComposeSignatureBlock({
  view,
  disabled = false,
}: {
  view: SignatureView;
  disabled?: boolean;
}) {
  const update = useMailStore((s) => s.updateCompose);
  const labelId = useId();
  const removeRef = useRef<HTMLButtonElement>(null);
  const addRef = useRef<HTMLButtonElement>(null);
  // The button pressed goes away; focus moves to the one in its place.
  const focusNext = useRef<"remove" | "add" | null>(null);
  useEffect(() => {
    const target = focusNext.current === "remove" ? removeRef : addRef;
    if (focusNext.current && target.current) {
      focusNext.current = null;
      target.current.focus();
    }
  });

  if (view.kind === "addedOnSend") {
    return (
      <p className="mail-signature-note">
        <Signature size={15} strokeWidth={1.75} aria-hidden="true" />
        Your organization adds your signature when you send.
      </p>
    );
  }

  if (view.kind === "removed") {
    return (
      <button
        ref={addRef}
        type="button"
        className="btn btn--subtle btn--sm mail-signature-add"
        disabled={disabled}
        onClick={() => {
          focusNext.current = "remove";
          update({ signature: "auto" });
        }}
      >
        Add signature
      </button>
    );
  }

  if (view.kind !== "editor") return null;
  const { own } = view;
  return (
    <div className="mail-signature" role="group" aria-labelledby={labelId}>
      <div className="mail-signature__bar">
        <span id={labelId} className="mail-signature__label">
          Signature
        </span>
        <button
          ref={removeRef}
          type="button"
          className="icon-btn icon-btn--sm mail-signature__remove"
          aria-label="Remove signature"
          title="Remove signature"
          disabled={disabled}
          onClick={() => {
            focusNext.current = "add";
            update({ signature: "removed" });
          }}
        >
          <X size={14} />
        </button>
        {view.edited && (
          <button
            type="button"
            className="btn btn--subtle btn--sm mail-signature__restore"
            disabled={disabled}
            onClick={() => {
              focusNext.current = "remove";
              update({ signature: "auto" });
            }}
          >
            Restore signature
          </button>
        )}
      </div>
      <SignatureEditor
        html={view.html}
        letter={own}
        placeholder="Signature"
        onChange={(html) => update({ signature: { edited: html, own } })}
      />
    </div>
  );
}
