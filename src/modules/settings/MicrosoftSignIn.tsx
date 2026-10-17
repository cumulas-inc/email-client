import {
  ArrowLeft,
  Ban,
  BriefcaseBusiness,
  CircleAlert,
  CircleX,
  ExternalLink,
  KeyRound,
  LockKeyhole,
  Mail,
  MonitorSmartphone,
  ServerOff,
  Settings,
  ShieldAlert,
  TimerOff,
  UserX,
  WifiOff,
  Wrench,
  Zap,
} from "lucide-react";
import {
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
} from "react";

import { useBackend } from "../../api/context";
import { errorMessage, hasErrorCode, isBackendError } from "../../api/errors";
import { useAppInfo, useSettings } from "../../api/queries";
import type {
  Account,
  BackendError,
  PermissionRequest,
  SignInBlockedReason,
} from "../../api/types";
import { Spinner } from "../../components/Spinner";
import { toast } from "../../components/toast";
import { AdminConsent, ApprovalLink } from "./AdminConsent";
import { TextField } from "./Fields";
import { FEATURE_COPY, ISSUES_URL, blockedSignInMailto, isConsumerAddress } from "./microsoft";
import { PermissionList } from "./PermissionList";
import { useAddMicrosoftAccount } from "./queries";
import { SignInHelp } from "./SignInHelp";
import { ResultPanel, StepIntro, StepLayout } from "./StepLayout";
import { validateEmail } from "./validation";

type Phase =
  | { kind: "form" }
  | { kind: "waiting" }
  /** `request` is what the failed sign-in asked for. */
  | { kind: "failed"; error: unknown; request: PermissionRequest };

type RetryLabel = "Try again" | "Turn on";

const BLOCKED_REASONS: readonly SignInBlockedReason[] = [
  "notAssigned",
  "appDisabled",
  "appNotSetUp",
  "serviceNotEnabled",
  "securityInfo",
  "conditionalAccess",
];

function isBlockedReason(reason: unknown): reason is SignInBlockedReason {
  return BLOCKED_REASONS.includes(reason as SignInBlockedReason);
}

/** What a blocked sign-in's panel says: Conditional Access by Microsoft's code. */
function blockedCopy(
  reason: SignInBlockedReason,
  aadsts: number | undefined,
  retry: RetryLabel,
): { icon: ReactNode; title: string; body: ReactNode } {
  switch (reason) {
    case "notAssigned":
      return {
        icon: <UserX size={28} />,
        title: "You haven't been given access to Cumulas Mail",
        body: (
          <>
            Your organization only lets people its IT team has assigned to Cumulas Mail sign in. Ask
            your IT team to assign you to the app (Microsoft Entra admin center → Enterprise apps →
            Cumulas Mail → Users and groups), then select <strong>{retry}</strong>.
          </>
        ),
      };
    case "appDisabled":
      return {
        icon: <Ban size={28} />,
        title: "Your organization has blocked Cumulas Mail",
        body: "An administrator has turned off or blocked Cumulas Mail in your organization. If you need it, ask your IT team whether they can allow it (Enterprise apps → Cumulas Mail → Properties → Enabled for users to sign in).",
      };
    case "appNotSetUp":
      return {
        icon: <Wrench size={28} />,
        title: "Your organization isn't set up for Cumulas Mail yet",
        body: "Microsoft 365 in your organization is missing something Cumulas Mail needs. An administrator can usually fix this by approving Cumulas Mail with the link below.",
      };
    case "serviceNotEnabled":
      return {
        icon: <ServerOff size={28} />,
        title: "Your organization's Microsoft 365 is missing a service",
        body: "Your organization's Microsoft 365 doesn't include a service Cumulas Mail needs, usually Exchange Online mail. Ask your IT team to check your subscription.",
      };
    case "securityInfo":
      return {
        icon: <KeyRound size={28} />,
        title: "Set up your sign-in security info first",
        body: (
          <>
            Your organization requires multifactor authentication or other security info, and
            Microsoft couldn&apos;t set it up during this sign-in. Ask your IT team for help, then
            select <strong>{retry}</strong>.
          </>
        ),
      };
    case "conditionalAccess":
      if (aadsts === 53000 || aadsts === 53001) {
        return {
          icon: <MonitorSmartphone size={28} />,
          title: "Your organization's sign-in rules require a managed device",
          body: "Your organization only allows sign-in from devices it manages, and Cumulas Mail on Linux may not be able to meet those rules. Ask your IT team whether you can use Cumulas Mail, or which app to use instead.",
        };
      }
      if (aadsts === 53002) {
        return {
          icon: <ShieldAlert size={28} />,
          title: "Your organization only allows apps it has approved",
          body: "Your organization's sign-in rules only allow certain apps, and Cumulas Mail isn't one of them yet. Ask your IT team whether they can allow it.",
        };
      }
      return {
        icon: <ShieldAlert size={28} />,
        title: "Your organization's sign-in rules blocked this sign-in",
        body: "Your organization's rules didn't allow this sign-in, for example because of the device, the app or where you're signing in from. Ask your IT team what's required and mention the error code below.",
      };
  }
}

interface MicrosoftSignInProps {
  initialEmail: string;
  /** Start signing in immediately (e.g. "Sign in again"). */
  autoStart: boolean;
  /** What the sign-in asks permission for. */
  permissions: PermissionRequest;
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
  permissions,
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
  /** What the next sign-in asks for: "Try with mail only" changes it. */
  const [request, setRequest] = useState<PermissionRequest>(permissions);
  /** The organization's Microsoft page offered Request approval. */
  const [prevCanRequestApproval, setPrevCanRequestApproval] = useState(false);
  const backend = useBackend();
  const { mutateAsync } = useAddMicrosoftAccount();
  const { data: info } = useAppInfo();
  const { data: settings } = useSettings();
  // The push service, unless it's turned off.
  const pushService =
    info && settings?.instantPush !== false && settings?.usePushRelay !== false
      ? info.syncServer
      : null;
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

  /** Shows the panel for a failed sign-in (or for what Microsoft's page said). */
  const showFailure = useCallback((error: unknown, req: PermissionRequest) => {
    if (isBackendError(error)) {
      const can = error.details?.canRequestApproval;
      // A link asked for by hand can't tell whether the workflow is on.
      if (can === true || (can === false && error.details?.manual !== true)) {
        setPrevCanRequestApproval(can);
      }
    }
    setPhase({ kind: "failed", error, request: req });
  }, []);

  /** Runs the browser sign-in; the caller has already shown the waiting state. */
  const signIn = useCallback(
    async (hint: string, req: PermissionRequest) => {
      const id = ++attempt.current;
      try {
        const account = await mutateAsync({ loginHint: hint || undefined, permissions: req });
        // Even a sign-in the user gave up on may have finished: say so.
        if (mounted.current) onDone(account);
        else toast.success(`Signed in as ${account.email}.`);
      } catch (error) {
        if (!mounted.current || id !== attempt.current) return;
        // Mail needs approval too: what was asked for at first is what to
        // try again.
        if (req.kind === "mailOnly" && hasErrorCode(error, "adminConsentRequired")) {
          setRequest(permissions);
        }
        showFailure(error, req);
      }
    },
    [mutateAsync, onDone, permissions, showFailure],
  );

  /** Stops waiting for the browser; the sign-in rejects, unseen. */
  const stopWaiting = () => {
    attempt.current += 1;
    backend.cancelMicrosoftSignIn().catch((e: unknown) => toast.error(e));
  };

  /** Stops waiting for the browser and returns to the form. */
  const cancel = () => {
    stopWaiting();
    setPhase({ kind: "form" });
  };

  const start = (req: PermissionRequest = request) => {
    const hint = email.trim();
    const invalid = hint ? validateEmail(hint) : null;
    setEmailError(invalid);
    if (invalid) {
      document.getElementById(emailId)?.focus();
      return;
    }
    setPhase({ kind: "waiting" });
    void signIn(hint, req);
  };

  const tryMailOnly = () => {
    const mailOnly: PermissionRequest = { kind: "mailOnly" };
    setRequest(mailOnly);
    start(mailOnly);
  };

  /** What Microsoft's page said, entered by hand, as the current request's failure. */
  const showResult = (error: BackendError) => showFailure(error, request);

  const autoStarted = useRef(false);
  useEffect(() => {
    if (!autoStart || autoStarted.current) return;
    autoStarted.current = true;
    void signIn(initialEmail.trim(), permissions);
  }, [autoStart, initialEmail, permissions, signIn]);

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    start();
  };

  const retryLabel: RetryLabel = request.kind === "feature" ? "Turn on" : "Try again";
  const hintEmail = email.trim() || undefined;

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
            title="Continue in your browser"
          >
            <p>
              We&apos;ve opened Microsoft&apos;s sign-in page in your web browser. When you&apos;re
              done there, come back here — this window updates by itself.
            </p>
          </ResultPanel>
        </div>
        <SignInHelp email={email} onCancelFirst={stopWaiting} onResult={showResult} />
      </StepLayout>
    );
  }

  if (phase.kind === "failed") {
    const retry = (
      <button type="button" className="btn btn--primary" onClick={() => start()}>
        {retryLabel}
      </button>
    );
    const close = (
      <button type="button" className="btn" onClick={onClose}>
        Close
      </button>
    );
    const { error, request: failed } = phase;
    const code = isBackendError(error) ? error.code : "internal";
    const details = isBackendError(error) ? error.details : undefined;
    const feature = failed.kind === "feature" ? failed.feature : undefined;

    if (code === "adminConsentRequired" && details?.adminConsentUrl) {
      const canRequestApproval = details.canRequestApproval === true;
      return (
        <StepLayout
          footer={
            <>
              {backButton}
              {close}
              <button type="button" className="btn" onClick={() => start()}>
                {retryLabel}
              </button>
              {canRequestApproval && (
                <button type="button" className="btn btn--primary" onClick={() => start()}>
                  Request approval
                </button>
              )}
            </>
          }
        >
          <AdminConsent
            adminConsentUrl={details.adminConsentUrl}
            tenant={details.tenant}
            email={hintEmail}
            canRequestApproval={canRequestApproval}
            adminPageUrl={details.adminPageUrl}
            retryLabel={retryLabel}
            feature={feature}
            onMailOnly={
              failed.kind === "all" || failed.kind === "current" ? tryMailOnly : undefined
            }
            mailOnlyFailed={failed.kind === "mailOnly"}
            manual={details.manual === true}
          />
        </StepLayout>
      );
    }

    if (code === "signInCancelled") {
      const timedOut = details?.reason === "timeout";
      return (
        <StepLayout
          footer={
            <>
              {backButton}
              {close}
              {retry}
            </>
          }
        >
          <ResultPanel
            icon={timedOut ? <TimerOff size={28} /> : <CircleX size={28} />}
            tone="neutral"
            title={timedOut ? "Sign-in timed out" : "Sign-in was canceled"}
          >
            <p>
              {timedOut
                ? "Microsoft's page didn't send you back to Cumulas Mail within 5 minutes. Nothing was changed."
                : "Nothing was changed. You can try again whenever you're ready."}
            </p>
          </ResultPanel>
          <SignInHelp email={email} onResult={showResult} />
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
              {retry}
            </>
          }
        >
          <ResultPanel
            icon={<ShieldAlert size={28} />}
            tone="warning"
            align="start"
            title={
              feature ? FEATURE_COPY[feature].notOn : "Cumulas Mail doesn't have permission yet"
            }
          >
            <p>
              {feature
                ? `Your mail keeps working. Microsoft didn't give Cumulas Mail permission to use ${FEATURE_COPY[feature].object}.`
                : "Microsoft didn't give Cumulas Mail permission to open your mailbox. It only ever acts as you, and your data stays on this computer."}
            </p>
            {!isConsumerAddress(email) && (
              <p>
                <strong>If you sent a request to your IT team:</strong> Microsoft usually emails you
                when they respond. Once they approve it, select <strong>{retryLabel}</strong>.
              </p>
            )}
            {prevCanRequestApproval ? (
              <p>
                <strong>If you haven&apos;t sent a request yet:</strong> select{" "}
                <strong>{retryLabel}</strong>, then <strong>Request approval</strong> on
                Microsoft&apos;s page.
              </p>
            ) : (
              <p>
                <strong>If you selected Cancel or didn&apos;t accept:</strong> select{" "}
                <strong>{retryLabel}</strong>, then <strong>Accept</strong> on Microsoft&apos;s
                page.
              </p>
            )}
            {failed.kind === "mailOnly" ? (
              <PermissionList only="core" />
            ) : (
              !feature && <PermissionList />
            )}
          </ResultPanel>
        </StepLayout>
      );
    }

    if (code === "signInBlocked" && details && isBlockedReason(details.reason)) {
      const reason = details.reason;
      const aadsts = typeof details.aadsts === "number" ? details.aadsts : undefined;
      const { icon, title, body } = blockedCopy(reason, aadsts, retryLabel);
      const link = reason === "appNotSetUp" ? details.adminConsentUrl : undefined;
      const mailto = blockedSignInMailto({
        reason,
        aadsts,
        email: hintEmail,
        tenant: details.tenant,
        adminPageUrl: details.adminPageUrl,
        adminConsentUrl: details.adminConsentUrl,
      });
      const open = (url: string) => backend.openLink(url).catch((e: unknown) => toast.error(e));
      return (
        <StepLayout
          footer={
            <>
              {backButton}
              {close}
              {retry}
            </>
          }
        >
          <ResultPanel icon={icon} tone="warning" align="start" title={title}>
            <p>{body}</p>
            {link && (
              // Its Email IT sends the blocked sign-in's request, which has the link.
              <ApprovalLink
                adminConsentUrl={link}
                tenant={details.tenant}
                email={hintEmail}
                adminPageUrl={details.adminPageUrl}
                emphasis="secondary"
                mailto={mailto}
              />
            )}
            {reason === "appNotSetUp" && (
              <p className="blocked-note">
                <span>If approving doesn&apos;t help, let Cumulas know.</span>
                <button
                  type="button"
                  className="btn btn--subtle btn--sm"
                  onClick={() => open(ISSUES_URL)}
                >
                  <ExternalLink size={14} aria-hidden="true" />
                  Let Cumulas know
                </button>
              </p>
            )}
            {aadsts !== undefined && (
              <p className="result-panel__detail selectable">Error code: AADSTS{aadsts}</p>
            )}
            {!link && (
              <div className="blocked-actions">
                <button type="button" className="btn" onClick={() => open(mailto)}>
                  <Mail size={16} aria-hidden="true" />
                  Email IT
                </button>
              </div>
            )}
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
            {retry}
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
              : "The sign-in didn't finish. Try again, and if it keeps happening, share the details below with your IT team."}
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
        Mail, calendar, contacts and tasks all stay in sync.
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
      {pushService && (
        <div className="secure-note">
          <Zap size={16} aria-hidden="true" />
          <p>
            For instant updates, Microsoft tells Cumulas&apos;s push service ({pushService}) when
            your Inbox or calendar changes, but never what&apos;s in it.
          </p>
        </div>
      )}
      <details className="permissions">
        <summary>What Cumulas Mail asks for</summary>
        <PermissionList />
      </details>
    </StepLayout>
  );
}
