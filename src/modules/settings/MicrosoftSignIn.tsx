import {
  ArrowLeft,
  BriefcaseBusiness,
  CircleAlert,
  CircleX,
  ExternalLink,
  LockKeyhole,
  Settings,
  ShieldAlert,
  WifiOff,
} from "lucide-react";
import { useCallback, useEffect, useId, useRef, useState, type FormEvent } from "react";

import { useBackend } from "../../api/context";
import { errorMessage, isBackendError } from "../../api/errors";
import type { Account } from "../../api/types";
import { Spinner } from "../../components/Spinner";
import { toast } from "../../components/toast";
import { AdminConsent } from "./AdminConsent";
import { TextField } from "./Fields";
import { PermissionList } from "./PermissionList";
import { useAddMicrosoftAccount } from "./queries";
import { ResultPanel, StepIntro, StepLayout } from "./StepLayout";
import { validateEmail } from "./validation";

type Phase = { kind: "form" } | { kind: "waiting" } | { kind: "failed"; error: unknown };

interface MicrosoftSignInProps {
  initialEmail: string;
  /** Start signing in immediately (e.g. "Sign in again"). */
  autoStart: boolean;
  onBack?: () => void;
  onClose: () => void;
  onDone: (account: Account) => void;
  onOpenSettings: () => void;
  /** Lets the dialog follow the current phase (for its title and focus). */
  onPhaseChange?: (phase: Phase["kind"]) => void;
}

export function MicrosoftSignIn({
  initialEmail,
  autoStart,
  onBack,
  onClose,
  onDone,
  onOpenSettings,
  onPhaseChange,
}: MicrosoftSignInProps) {
  const formId = useId();
  const emailId = useId();
  const [email, setEmail] = useState(initialEmail);
  const [emailError, setEmailError] = useState<string | null>(null);
  const [phase, setPhase] = useState<Phase>(autoStart ? { kind: "waiting" } : { kind: "form" });
  const backend = useBackend();
  const { mutateAsync } = useAddMicrosoftAccount();
  /** Bumped for each sign-in, and on cancel, so stale results are ignored. */
  const attempt = useRef(0);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  useEffect(() => onPhaseChange?.(phase.kind), [phase.kind, onPhaseChange]);

  /** Runs the browser sign-in; the caller has already shown the waiting state. */
  const signIn = useCallback(
    async (hint: string) => {
      const id = ++attempt.current;
      try {
        const account = await mutateAsync(hint || undefined);
        // Even a sign-in the user gave up on may have finished: say so.
        if (mounted.current) onDone(account);
        else toast.success(`Signed in as ${account.email}.`);
      } catch (error) {
        if (!mounted.current || id !== attempt.current) return;
        setPhase({ kind: "failed", error });
      }
    },
    [mutateAsync, onDone],
  );

  /** Stops waiting for the browser and returns to the form. */
  const cancel = () => {
    attempt.current += 1;
    setPhase({ kind: "form" });
    backend.cancelMicrosoftSignIn().catch((e: unknown) => toast.error(e));
  };

  const start = () => {
    const hint = email.trim();
    const invalid = hint ? validateEmail(hint) : null;
    setEmailError(invalid);
    if (invalid) {
      document.getElementById(emailId)?.focus();
      return;
    }
    setPhase({ kind: "waiting" });
    void signIn(hint);
  };

  const autoStarted = useRef(false);
  useEffect(() => {
    if (!autoStart || autoStarted.current) return;
    autoStarted.current = true;
    void signIn(initialEmail.trim());
  }, [autoStart, initialEmail, signIn]);

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    start();
  };

  const backButton = onBack && (
    <button type="button" className="btn btn--subtle add-account__back" onClick={onBack}>
      <ArrowLeft size={16} aria-hidden="true" />
      Back
    </button>
  );

  if (phase.kind === "waiting") {
    return (
      <StepLayout
        footer={
          <button type="button" className="btn" onClick={cancel}>
            Cancel
          </button>
        }
      >
        <div aria-live="polite">
          <ResultPanel
            icon={<Spinner size={30} label="Waiting for sign-in" />}
            tone="brand"
            title="Finish signing in in your browser…"
          >
            <p>
              We&apos;ve opened Microsoft&apos;s sign-in page in your web browser. When you&apos;re
              done there, come back here — this window updates by itself.
            </p>
          </ResultPanel>
        </div>
      </StepLayout>
    );
  }

  if (phase.kind === "failed") {
    const tryAgain = (
      <button type="button" className="btn btn--primary" onClick={start}>
        Try again
      </button>
    );
    const close = (
      <button type="button" className="btn" onClick={onClose}>
        Close
      </button>
    );
    const error = phase.error;
    const code = isBackendError(error) ? error.code : "internal";

    if (
      code === "adminConsentRequired" &&
      isBackendError(error) &&
      error.details?.adminConsentUrl
    ) {
      return (
        <StepLayout
          footer={
            <>
              {backButton}
              {close}
              <button type="button" className="btn" onClick={start}>
                Try again
              </button>
            </>
          }
        >
          <AdminConsent
            adminConsentUrl={error.details.adminConsentUrl}
            tenant={error.details.tenant}
            email={email.trim() || undefined}
            canRequestApproval={error.details.canRequestApproval === true}
          />
        </StepLayout>
      );
    }

    if (code === "signInCancelled") {
      return (
        <StepLayout
          footer={
            <>
              {backButton}
              {close}
              {tryAgain}
            </>
          }
        >
          <ResultPanel icon={<CircleX size={28} />} tone="neutral" title="Sign-in was canceled">
            <p>Nothing was changed. You can try again whenever you&apos;re ready.</p>
          </ResultPanel>
        </StepLayout>
      );
    }

    if (code === "consentDeclined") {
      return (
        <StepLayout
          footer={
            <>
              {backButton}
              {close}
              {tryAgain}
            </>
          }
        >
          <ResultPanel
            icon={<ShieldAlert size={28} />}
            tone="warning"
            align="start"
            title="Cumulas Mail needs your permission"
          >
            <p>
              The permission request was declined, so Cumulas Mail can&apos;t open your mailbox. It
              needs these permissions to show your mail, calendar, contacts and tasks. It only ever
              acts as you, and your data stays on this computer.
            </p>
            <PermissionList />
            <p>
              When you try again, select <strong>Accept</strong> on Microsoft&apos;s permission
              page.
            </p>
          </ResultPanel>
        </StepLayout>
      );
    }

    if (code === "notConfigured") {
      return (
        <StepLayout
          footer={
            <>
              {backButton}
              {close}
              <button type="button" className="btn btn--primary" onClick={onOpenSettings}>
                <Settings size={16} aria-hidden="true" />
                Open Microsoft settings
              </button>
            </>
          }
        >
          <ResultPanel
            icon={<Settings size={28} />}
            tone="warning"
            align="start"
            title="Microsoft sign-in isn't set up yet"
          >
            <p>
              This copy of Cumulas Mail doesn&apos;t include a Microsoft application ID, which
              Microsoft needs to know which app is signing in.
            </p>
            <p>
              Add one in <strong>Settings → Microsoft app registration</strong>, then try again.
              Your IT team can tell you which ID to use, or you can register your own app (see
              docs/microsoft-365.md).
            </p>
          </ResultPanel>
        </StepLayout>
      );
    }

    const network = code === "network";
    return (
      <StepLayout
        footer={
          <>
            {backButton}
            {close}
            {tryAgain}
          </>
        }
      >
        <ResultPanel
          icon={network ? <WifiOff size={28} /> : <CircleAlert size={28} />}
          tone="danger"
          title={network ? "Couldn't reach Microsoft" : "Couldn't sign in"}
        >
          <p>
            {network
              ? "Check that you're connected to the internet, then try again."
              : "Something went wrong while signing in. Please try again."}
          </p>
          <p className="result-panel__detail selectable">{errorMessage(error)}</p>
        </ResultPanel>
      </StepLayout>
    );
  }

  return (
    <StepLayout
      formId={formId}
      onSubmit={onSubmit}
      footer={
        <>
          {backButton}
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="btn btn--primary" form={formId}>
            <ExternalLink size={16} aria-hidden="true" />
            Sign in with Microsoft
          </button>
        </>
      }
    >
      <StepIntro icon={<BriefcaseBusiness size={20} />}>
        Use your work, school or personal Microsoft account, including Outlook.com and Hotmail.
        Mail, calendar, contacts and To Do all stay in sync.
      </StepIntro>
      <TextField
        id={emailId}
        label="Email address (optional)"
        type="email"
        autoComplete="email"
        spellCheck={false}
        autoCapitalize="none"
        placeholder="name@example.com"
        value={email}
        error={emailError}
        hint="Helps Microsoft pick the right account if you have more than one."
        onChange={(e) => {
          setEmail(e.target.value);
          setEmailError(null);
        }}
        data-autofocus
      />
      <div className="secure-note">
        <LockKeyhole size={16} aria-hidden="true" />
        <p>
          Your web browser opens so you can sign in on Microsoft&apos;s own page. Cumulas Mail never
          sees your password.
        </p>
      </div>
      <details className="permissions">
        <summary>What Cumulas Mail will ask for</summary>
        <PermissionList />
      </details>
    </StepLayout>
  );
}
