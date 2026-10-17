import { ExternalLink, LoaderCircle } from "lucide-react";
import { useId, useState, type FormEvent } from "react";

import { useBackend } from "../../api/context";
import { errorMessage } from "../../api/errors";
import type { AdminConsentLink, BackendError } from "../../api/types";
import { toast } from "../../components/toast";
import { TextField } from "./Fields";
import { focusLater } from "./focus";
import { MICROSOFT_ERROR_LOOKUP, classifyAadsts, isConsumerAddress } from "./microsoft";

interface SignInHelpProps {
  /** The address being signed in with, if any. */
  email: string;
  /** Called before asking anything: the waiting panel stops waiting for the browser. */
  onCancelFirst?: () => void;
  /** Shows the panel for what Microsoft said, as if the sign-in had failed with it. */
  onResult: (error: BackendError) => void;
}

/** The approval panel's error, for a link the person asked for themselves. */
function needsApproval(link: AdminConsentLink, canRequestApproval: boolean): BackendError {
  return {
    code: "adminConsentRequired",
    message: "",
    details: {
      adminConsentUrl: link.adminConsentUrl,
      tenant: link.tenant,
      adminPageUrl: link.adminPageUrl,
      canRequestApproval,
      manual: true,
    },
  };
}

/**
 * For when Microsoft shows its own error page and never sends the browser
 * back: the person says what Microsoft said, and gets the panel for it.
 */
export function SignInHelp({ email, onCancelFirst, onResult }: SignInHelpProps) {
  const backend = useBackend();
  const id = useId();
  const formId = `${id}-code`;
  const fieldId = `${id}-field`;
  const [asking, setAsking] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const [codeOpen, setCodeOpen] = useState(false);
  const [code, setCode] = useState("");
  const [codeError, setCodeError] = useState<string | null>(null);
  const [unknown, setUnknown] = useState<number | null>(null);
  const hint = email.trim() || undefined;

  const askForLink = async (canRequestApproval: boolean) => {
    setAsking(true);
    setProblem(null);
    try {
      const link = await backend.microsoftAdminConsent(undefined, hint);
      onResult(needsApproval(link, canRequestApproval));
    } catch (e) {
      setProblem(errorMessage(e));
    } finally {
      setAsking(false);
    }
  };

  const adminApproval = () => {
    onCancelFirst?.();
    void askForLink(false);
  };

  const showWhatToDo = async (e: FormEvent) => {
    e.preventDefault();
    const digits = code.replace(/\D/g, "");
    setUnknown(null);
    if (!digits) {
      setCodeError("Type the number after AADSTS.");
      document.getElementById(fieldId)?.focus();
      return;
    }
    setCodeError(null);
    const aadsts = Number(digits);
    const kind = classifyAadsts(aadsts);
    if (!kind) {
      setUnknown(aadsts);
      return;
    }
    onCancelFirst?.();
    if (kind.kind === "declined") {
      onResult({ code: "consentDeclined", message: "" });
    } else if (kind.kind === "adminConsent") {
      await askForLink(kind.canRequestApproval);
    } else {
      // The panel works without the link: only appNotSetUp offers one.
      const link = await backend.microsoftAdminConsent(undefined, hint).catch(() => null);
      onResult({
        code: "signInBlocked",
        message: "",
        details: {
          reason: kind.reason,
          aadsts,
          tenant: link?.tenant,
          adminPageUrl: link?.adminPageUrl,
          adminConsentUrl: kind.reason === "appNotSetUp" ? link?.adminConsentUrl : undefined,
        },
      });
    }
  };

  const toggleCode = () => {
    if (!codeOpen) focusLater(fieldId);
    setCodeOpen(!codeOpen);
  };

  const lookUp = (aadsts: number) => {
    backend.openLink(MICROSOFT_ERROR_LOOKUP(aadsts)).catch((e: unknown) => toast.error(e));
  };

  return (
    <div className="sign-in-help">
      <p className="sign-in-help__lead">Stuck on Microsoft&apos;s page?</p>
      <div className="sign-in-help__actions">
        {!isConsumerAddress(email) && (
          <button
            type="button"
            className="btn btn--subtle btn--sm"
            onClick={adminApproval}
            disabled={asking}
            aria-busy={asking || undefined}
          >
            {asking && <LoaderCircle size={14} className="spin" aria-hidden="true" />}
            Microsoft said I need admin approval
          </button>
        )}
        <button
          type="button"
          className="btn btn--subtle btn--sm"
          aria-expanded={codeOpen}
          aria-controls={formId}
          onClick={toggleCode}
        >
          Microsoft showed an error code
        </button>
      </div>
      {problem && (
        <p className="sign-in-help__problem field__error" role="alert">
          {problem}
        </p>
      )}
      <form
        id={formId}
        className="sign-in-help__code"
        hidden={!codeOpen}
        noValidate
        onSubmit={showWhatToDo}
      >
        <TextField
          id={fieldId}
          label="Error code"
          hint="The number after AADSTS on Microsoft's page, for example 50105."
          inputMode="numeric"
          autoComplete="off"
          spellCheck={false}
          maxLength={20}
          value={code}
          error={codeError}
          onChange={(e) => {
            setCode(e.target.value);
            setCodeError(null);
          }}
        />
        <button type="submit" className="btn btn--sm" disabled={asking}>
          Show what to do
        </button>
      </form>
      <div aria-live="polite">
        {unknown !== null && (
          <div className="sign-in-help__unknown">
            <p>Cumulas Mail doesn&apos;t know AADSTS{unknown}.</p>
            <button type="button" className="btn btn--sm" onClick={() => lookUp(unknown)}>
              <ExternalLink size={14} aria-hidden="true" />
              Look it up on Microsoft&apos;s site
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
