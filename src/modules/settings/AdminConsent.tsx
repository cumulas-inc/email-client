import { Check, Copy, Mail, ShieldCheck } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";

import { useBackend } from "../../api/context";
import { toast } from "../../components/toast";
import { adminConsentMailto, organisationName } from "./microsoft";
import { PermissionList } from "./PermissionList";
import { ResultPanel } from "./StepLayout";

interface AdminConsentProps {
  adminConsentUrl: string;
  tenant?: string;
  /** The address the user tried to sign in with. */
  email?: string;
}

/**
 * Shown when the organisation only allows apps an administrator approved.
 * Explains that nothing is wrong and makes forwarding the approval link easy.
 */
export function AdminConsent({ adminConsentUrl, tenant, email }: AdminConsentProps) {
  const backend = useBackend();
  const linkId = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const [copied, setCopied] = useState(false);
  const org = organisationName(tenant);

  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 2500);
    return () => window.clearTimeout(timer);
  }, [copied]);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(adminConsentUrl);
      setCopied(true);
    } catch {
      inputRef.current?.focus();
      inputRef.current?.select();
      toast.info("Couldn't copy automatically. The link is selected — press Ctrl+C to copy it.");
    }
  };

  const emailIt = () => {
    backend
      .openLink(adminConsentMailto({ adminConsentUrl, email, tenant }))
      .catch((e: unknown) => toast.error(e));
  };

  return (
    <ResultPanel
      icon={<ShieldCheck size={28} />}
      tone="brand"
      align="start"
      title="Your organisation needs to approve Cumulas Mail"
    >
      <p>
        This isn&apos;t something you did wrong, and nothing is broken.{" "}
        {org ? (
          <>
            <strong>{org}</strong> only allows
          </>
        ) : (
          "Your organisation only allows"
        )}{" "}
        apps that its IT team has approved. An administrator can approve Cumulas Mail for everyone
        in a couple of minutes.
      </p>
      <ol className="consent-steps">
        <li>
          <span>Send the approval link below to your IT team.</span>
        </li>
        <li>
          <span>
            Once they&apos;ve approved it, come back and select <strong>Try again</strong>.
          </span>
        </li>
      </ol>

      <div className="consent-link">
        <label htmlFor={linkId} className="field__label">
          Approval link for your IT administrator
        </label>
        <div className="consent-link__row">
          <input
            ref={inputRef}
            id={linkId}
            className="input consent-link__input selectable"
            readOnly
            value={adminConsentUrl}
            onFocus={(e) => e.currentTarget.select()}
          />
          <button type="button" className="btn" onClick={() => void copy()}>
            {copied ? (
              <Check size={16} aria-hidden="true" />
            ) : (
              <Copy size={16} aria-hidden="true" />
            )}
            {copied ? "Copied" : "Copy"}
          </button>
        </div>
        <span role="status" className="visually-hidden">
          {copied ? "Link copied to the clipboard" : ""}
        </span>
      </div>

      <div className="consent-email">
        <button type="button" className="btn btn--primary" onClick={emailIt}>
          <Mail size={16} aria-hidden="true" />
          Email IT
        </button>
        <span className="consent-email__hint">
          Opens your email app with a ready-to-send request. Just add your IT team&apos;s address.
        </span>
      </div>

      <details className="permissions">
        <summary>What Cumulas Mail asks for</summary>
        <p className="permissions__intro">
          Only permissions that act as you. Your data stays on this computer and goes only to
          Microsoft.
        </p>
        <PermissionList showScopes />
      </details>
    </ResultPanel>
  );
}
