import { ExternalLink } from "lucide-react";
import { type ReactNode, useCallback, useEffect, useId, useReducer, useRef } from "react";

import { useBackend } from "../../../api/context";
import { errorMessage, hasErrorCode, isBackendError } from "../../../api/errors";
import type {
  Account,
  BackendError,
  OrgSetupChoices,
  OrgSetupPlan,
  OrgSetupResult,
  OrgSetupStatus,
} from "../../../api/types";
import { Dialog } from "../../../components/Dialog";
import { Spinner } from "../../../components/Spinner";
import { toast } from "../../../components/toast";
import { ApprovalLink } from "../AdminConsent";
import { Callout } from "../Fields";
import { focusLater, useRestoreFocus } from "../focus";
import { MICROSOFT_ERROR_LOOKUP, PROTECTION_GUIDE_URL } from "../microsoft";
import { ChangesSheet } from "./ChangesSheet";
import {
  APPLYING,
  BUTTONS,
  CONFIRM,
  CONSENT,
  DIALOG,
  DIALOG_TITLE,
  DONE,
  INTRO,
  READING,
  SETUP_APP_IT_PAGE_URL,
  SIGNED_OUT,
  SIGNING_IN,
  STALE,
} from "./copy";
import { CopySetupDetails } from "./parts";
import {
  useApplyOrgSetup,
  useOrgSetupInfo,
  useOrgSetupScript,
  useOrgSetupSignIn,
  usePlanOrgSetup,
  useReadOrgSetup,
} from "./queries";
import { allDone } from "./changes";
import { ResultView } from "./ResultView";
import { CopyScriptButton, type ScriptParts, ScriptView } from "./ScriptView";
import { PermissionsSection, StatusView } from "./StatusView";

import "../settings.css";
import "./orgsetup.css";

/** Where the dialog is (3.13.2). The script view goes over any of them. */
type View =
  | { kind: "intro" }
  | { kind: "signingIn"; from: View }
  | { kind: "consent"; error: BackendError }
  /** A sign-in, a read or the changes failed as a whole. */
  | { kind: "failed"; error: unknown; retry: "signIn" | "read" }
  | { kind: "reading" }
  | { kind: "status"; stale: boolean }
  | { kind: "confirm" }
  | { kind: "applying" }
  | { kind: "done"; result: OrgSetupResult }
  | { kind: "signedOut" };

interface State {
  view: View;
  /** The script view's parts while it's shown over `view`; null when it isn't. */
  script: ScriptParts | null;
  /** A sign-in was tried: Copy setup details has something to say from now on. */
  attempted: boolean;
  /** The last read, the choices made on it, and the plan they make. */
  status: OrgSetupStatus | null;
  choices: OrgSetupChoices | null;
  plan: OrgSetupPlan | null;
  /** A plan for changed choices is being worked out. */
  planning: boolean;
}

type Action =
  | { type: "signingIn" }
  | { type: "signInFailed"; error: unknown }
  | { type: "reading" }
  | { type: "read"; status: OrgSetupStatus }
  | { type: "failed"; error: unknown; retry: "signIn" | "read" }
  | { type: "signedOut" }
  | { type: "choices"; choices: OrgSetupChoices }
  | { type: "planned"; plan: OrgSetupPlan }
  | { type: "confirm" }
  | { type: "back" }
  | { type: "applying" }
  | { type: "stale"; status: OrgSetupStatus; choices: OrgSetupChoices }
  | { type: "applied"; result: OrgSetupResult }
  | { type: "showScript"; parts: ScriptParts }
  | { type: "scriptParts"; parts: ScriptParts }
  | { type: "hideScript" };

/** The sign-in ended (Sign out, 60 idle minutes, a new sign-in elsewhere): sign in again. */
function signedOut(e: unknown): boolean {
  return hasErrorCode(e, "authRequired") && e.details?.reason === "setupSignedOut";
}

function reduce(state: State, action: Action): State {
  switch (action.type) {
    case "signingIn":
      return { ...state, view: { kind: "signingIn", from: state.view }, attempted: true };
    case "signInFailed": {
      const { error } = action;
      const from = state.view.kind === "signingIn" ? state.view.from : state.view;
      if (hasErrorCode(error, "signInCancelled") && error.details?.reason !== "timeout") {
        // A new sign-in ends the one there was: back to the start of it, signed out.
        const was = ["status", "confirm", "done"].includes(from.kind);
        return { ...state, view: was ? { kind: "signedOut" } : from };
      }
      if (hasErrorCode(error, "adminConsentRequired") && error.details?.reason === "setupConsent") {
        return { ...state, view: { kind: "consent", error } };
      }
      return { ...state, view: { kind: "failed", error, retry: "signIn" } };
    }
    case "reading":
      return { ...state, view: { kind: "reading" } };
    case "read":
      return {
        ...state,
        view: { kind: "status", stale: false },
        status: action.status,
        choices: action.status.defaults,
        plan: action.status.plan,
        planning: false,
      };
    case "failed":
      return { ...state, view: { kind: "failed", error: action.error, retry: action.retry } };
    case "signedOut":
      return { ...state, view: { kind: "signedOut" }, script: null, planning: false };
    case "choices":
      return { ...state, choices: action.choices, planning: true };
    case "planned":
      return { ...state, plan: action.plan, planning: false };
    case "confirm":
      return { ...state, view: { kind: "confirm" } };
    case "back":
      return { ...state, view: { kind: "status", stale: false } };
    case "applying":
      return { ...state, view: { kind: "applying" } };
    case "stale":
      return {
        ...state,
        view: { kind: "status", stale: true },
        status: action.status,
        choices: action.choices,
        plan: action.status.plan,
        planning: true,
      };
    case "applied":
      // The choices stay: Show the PowerShell script covers what was asked for.
      return {
        ...state,
        view: { kind: "done", result: action.result },
        status: action.result.status,
      };
    case "showScript":
    case "scriptParts":
      return { ...state, script: action.parts };
    case "hideScript":
      return { ...state, script: null };
  }
}

const INITIAL: State = {
  view: { kind: "intro" },
  script: null,
  attempted: false,
  status: null,
  choices: null,
  plan: null,
  planning: false,
};

/** The organization's name in the dialog's words: the work account's domain. */
function organization(account: Account): string {
  const at = account.email.lastIndexOf("@");
  return at >= 0 ? account.email.slice(at + 1) : account.email;
}

interface OrgSetupDialogProps {
  account: Account;
  onClose: () => void;
}

/**
 * "Set up or check for your organization": an administrator signs in with
 * the setup app, reads what setup needs, confirms the changes and makes
 * them; or copies the PowerShell script that does the same (3.13.2).
 * Closing it, or leaving it any other way, forgets the setup sign-in.
 */
export function OrgSetupDialog({ account, onClose }: OrgSetupDialogProps) {
  const backend = useBackend();
  const titleId = useId();
  const [state, dispatch] = useReducer(reduce, INITIAL);
  const { view, status, choices, plan } = state;
  const info = useOrgSetupInfo(account.id);
  const signIn = useOrgSetupSignIn();
  const readSetup = useReadOrgSetup();
  const planSetup = usePlanOrgSetup();
  const applySetup = useApplyOrgSetup();
  const scriptRef = useRef<HTMLPreElement>(null);
  const org = organization(account);
  const automatic = info.data?.automatic ?? false;
  useRestoreFocus();

  // What closing and unmounting need to know: whether a sign-in waits for
  // the browser, and whether the dialog is still there to show answers.
  const waiting = useRef(false);
  const gone = useRef(false);
  const closed = useRef(false);
  useEffect(() => {
    waiting.current = view.kind === "signingIn";
  }, [view.kind]);

  // The script shows over a state, or is the whole intro without automatic setup.
  const scriptShown = state.script !== null || (info.data !== undefined && !automatic);
  const parts = state.script ?? { actions: true, labels: false };
  const script = useOrgSetupScript(
    account.id,
    { choices: status ? choices : null, actions: parts.actions, labels: parts.labels },
    scriptShown,
  );

  /** Forgets the setup sign-in, and stops one that's waiting for the browser. */
  const end = useCallback(() => {
    if (waiting.current) backend.cancelOrgSetupSignIn().catch(() => undefined);
    backend.endOrgSetup().catch(() => undefined);
  }, [backend]);

  useEffect(() => {
    gone.current = false;
    return () => {
      gone.current = true;
      // Left some other way (a reload, another part of Settings): closing already ended it.
      if (!closed.current) end();
    };
  }, [end]);

  const close = () => {
    // Nothing stops changes halfway: they finish, then say how they went.
    if (view.kind === "applying") return;
    closed.current = true;
    end();
    onClose();
  };

  // The keyboard goes to the title whenever what the dialog shows changes.
  const shown = `${view.kind}:${state.script ? "script" : ""}:${info.data ? "info" : ""}`;
  useEffect(() => {
    focusLater(titleId);
  }, [shown, titleId]);

  // Each plan asked for has a number; an answer counts only while it's the latest.
  const planSeq = useRef(0);

  const read = async () => {
    // A plan still on its way is for choices on the read this one replaces.
    planSeq.current += 1;
    dispatch({ type: "reading" });
    try {
      const found = await readSetup.mutateAsync(account.id);
      if (!gone.current) dispatch({ type: "read", status: found });
    } catch (e) {
      if (gone.current) return;
      dispatch(signedOut(e) ? { type: "signedOut" } : { type: "failed", error: e, retry: "read" });
    }
  };

  const startSignIn = async () => {
    dispatch({ type: "signingIn" });
    try {
      await signIn.mutateAsync(account.id);
    } catch (e) {
      if (!gone.current) dispatch({ type: "signInFailed", error: e });
      return;
    }
    if (!gone.current) await read();
  };

  const replan = (next: OrgSetupChoices) => {
    const seq = ++planSeq.current;
    planSetup.mutateAsync({ accountId: account.id, choices: next }).then(
      (planned) => {
        if (seq === planSeq.current && !gone.current) {
          dispatch({ type: "planned", plan: planned });
        }
      },
      (e: unknown) => {
        if (seq !== planSeq.current || gone.current) return;
        if (signedOut(e)) dispatch({ type: "signedOut" });
        // Said where the plan's own problems are; nothing can be set up meanwhile.
        else {
          const problems = [errorMessage(e)];
          dispatch({ type: "planned", plan: { changes: [], problems, blocked: 0 } });
        }
      },
    );
  };

  const changeChoices = (next: OrgSetupChoices) => {
    dispatch({ type: "choices", choices: next });
    replan(next);
  };

  const apply = async () => {
    if (!choices || !plan) return;
    dispatch({ type: "applying" });
    try {
      const result = await applySetup.mutateAsync({
        accountId: account.id,
        choices,
        confirmed: plan.changes.map((c) => c.id),
      });
      if (gone.current) return;
      if (result.stale) {
        // The same choices, on what's there now.
        const known = new Set(result.status.labels.map((l) => l.id));
        const kept = { ...choices, labelIds: choices.labelIds.filter((id) => known.has(id)) };
        dispatch({ type: "stale", status: result.status, choices: kept });
        replan(kept);
      } else {
        dispatch({ type: "applied", result });
      }
    } catch (e) {
      if (gone.current) return;
      dispatch(signedOut(e) ? { type: "signedOut" } : { type: "failed", error: e, retry: "read" });
    }
  };

  const signOut = () => {
    planSeq.current += 1;
    backend.endOrgSetup().catch(() => undefined);
    dispatch({ type: "signedOut" });
  };

  const showScript = () =>
    dispatch({
      type: "showScript",
      // Once signed in, the script covers what was chosen.
      parts:
        status && choices
          ? { actions: choices.actions, labels: choices.labelIds.length > 0 }
          : { actions: true, labels: false },
    });

  const openLink = (url: string) => {
    backend.openLink(url).catch((e: unknown) => toast.error(e));
  };

  const copyDetails = state.attempted || Boolean(info.data?.signedIn);
  const rowActions = { onSignIn: () => void startSignIn(), onShowScript: showScript };

  // --- What each state shows --------------------------------------------------------

  let title = DIALOG_TITLE;
  let body: ReactNode;
  let footer: ReactNode;
  const button = (label: string, onClick: () => void, primary = false, disabled = false) => (
    <button
      type="button"
      className={primary ? "btn btn--primary" : "btn"}
      onClick={onClick}
      disabled={disabled}
    >
      {label}
    </button>
  );
  const scriptButton = button(BUTTONS.showScript, showScript);
  const detailsButton = copyDetails ? <CopySetupDetails /> : null;
  const scriptView = (
    <ScriptView
      parts={parts}
      onParts={(next) => dispatch({ type: "scriptParts", parts: next })}
      script={script}
      license={status?.license ?? null}
      scriptRef={scriptRef}
    />
  );
  const permissions = <PermissionsSection account={account} {...rowActions} />;

  if (!info.data) {
    body = info.isError ? (
      <p className="field__error" role="alert">
        {errorMessage(info.error)}
      </p>
    ) : (
      <div className="orgsetup-wait">
        <Spinner size={24} label="Loading" />
      </div>
    );
    footer = button(BUTTONS.close, close);
  } else if (!automatic) {
    body = (
      <>
        <p>{INTRO.scriptOnly(org)}</p>
        {permissions}
        {scriptView}
      </>
    );
    footer = (
      <>
        {button(BUTTONS.openGuide, () => openLink(PROTECTION_GUIDE_URL))}
        <CopyScriptButton script={script} scriptRef={scriptRef} />
      </>
    );
  } else if (state.script) {
    body = scriptView;
    footer = (
      <>
        {button(BUTTONS.back, () => dispatch({ type: "hideScript" }))}
        <CopyScriptButton script={script} scriptRef={scriptRef} />
      </>
    );
  } else {
    switch (view.kind) {
      case "intro":
        body = (
          <>
            <p>{INTRO.what}</p>
            <p>{INTRO.who(org)}</p>
            <p className="orgsetup-note">{INTRO.undocumented}</p>
            <p className="orgsetup-note">{INTRO.notAdmin}</p>
            {permissions}
          </>
        );
        footer = (
          <>
            {detailsButton}
            {scriptButton}
            {button(BUTTONS.signInAsAdmin, () => void startSignIn(), true)}
          </>
        );
        break;
      case "signingIn":
        body = (
          <div className="orgsetup-wait" role="status">
            <Spinner size={24} label="Waiting for the browser" />
            <p>{SIGNING_IN}</p>
          </div>
        );
        footer = button(BUTTONS.cancel, () => {
          backend.cancelOrgSetupSignIn().catch(() => undefined);
        });
        break;
      case "consent": {
        title = CONSENT.title;
        const link =
          (typeof view.error.details?.adminConsentUrl === "string"
            ? view.error.details.adminConsentUrl
            : null) ?? info.data.approvalUrl;
        body = (
          <>
            <p>{CONSENT.message}</p>
            <p>{CONSENT.canApprove}</p>
            <Callout tone="warning">
              <p>{CONSENT.warning}</p>
            </Callout>
            {link && <ApprovalLink adminConsentUrl={link} emailIt={false} emphasis="secondary" />}
            <div>
              <button
                type="button"
                className="btn btn--subtle btn--sm"
                onClick={() => openLink(SETUP_APP_IT_PAGE_URL)}
              >
                <ExternalLink size={14} aria-hidden="true" />
                {DIALOG.itPage}
              </button>
            </div>
          </>
        );
        footer = (
          <>
            {detailsButton}
            {scriptButton}
            {button(BUTTONS.signInAgain, () => void startSignIn(), true)}
          </>
        );
        break;
      }
      case "failed": {
        const error = view.error;
        const aadsts =
          isBackendError(error) && typeof error.details?.aadsts === "number"
            ? error.details.aadsts
            : undefined;
        body = (
          <Callout
            tone="danger"
            role="alert"
            title={view.retry === "signIn" ? undefined : DIALOG.readFailed}
          >
            <p className="selectable">{errorMessage(error)}</p>
            {aadsts !== undefined && (
              <button
                type="button"
                className="btn btn--subtle btn--sm"
                onClick={() => openLink(MICROSOFT_ERROR_LOOKUP(aadsts))}
              >
                <ExternalLink size={14} aria-hidden="true" />
                {DIALOG.lookUpCode(aadsts)}
              </button>
            )}
          </Callout>
        );
        footer = (
          <>
            {detailsButton}
            {scriptButton}
            {view.retry === "signIn"
              ? button(BUTTONS.tryAgain, () => void startSignIn(), true)
              : button(BUTTONS.readAgain, () => void read(), true)}
          </>
        );
        break;
      }
      case "reading":
        body = (
          <div className="orgsetup-wait" role="status">
            <Spinner size={24} label="Reading" />
            <p>{READING(org)}</p>
          </div>
        );
        footer = button(BUTTONS.cancel, close);
        break;
      case "status":
        if (!status || !choices || !plan) break;
        body = (
          <StatusView
            account={account}
            status={status}
            choices={choices}
            plan={plan}
            planning={state.planning}
            onChoices={changeChoices}
            onSignOut={signOut}
            notice={
              view.stale && (
                <Callout tone="warning">
                  <p>{STALE}</p>
                </Callout>
              )
            }
            {...rowActions}
          />
        );
        footer = (
          <>
            {button(BUTTONS.readAgain, () => void read())}
            {detailsButton}
            {scriptButton}
            {button(
              view.stale ? BUTTONS.review : status.anyOurs ? BUTTONS.repair : BUTTONS.setUp,
              () => dispatch({ type: "confirm" }),
              true,
              state.planning || plan.changes.length === 0 || plan.problems.length > 0,
            )}
          </>
        );
        break;
      case "confirm":
        if (!status || !plan) break;
        title = CONFIRM.title;
        body = <ChangesSheet plan={plan} status={status} />;
        footer = (
          <>
            {button(BUTTONS.back, () => dispatch({ type: "back" }))}
            {button(BUTTONS.makeChanges, () => void apply(), true)}
          </>
        );
        break;
      case "applying":
        title = CONFIRM.title;
        body = (
          <div className="orgsetup-wait" role="status">
            <Spinner size={24} label="Making changes" />
            <p>{APPLYING}</p>
          </div>
        );
        footer = null;
        break;
      case "done": {
        const result = view.result;
        const finished = allDone(result);
        title = finished ? DONE.title : DONE.someFailed;
        body = <ResultView result={result} />;
        footer = (
          <>
            {detailsButton}
            {!finished && scriptButton}
            {result.signedOut && button(BUTTONS.signIn, () => void startSignIn())}
            {button(BUTTONS.close, close, true)}
          </>
        );
        break;
      }
      case "signedOut":
        body = <p>{SIGNED_OUT}</p>;
        footer = (
          <>
            {detailsButton}
            {scriptButton}
            {button(BUTTONS.signIn, () => void startSignIn(), true)}
          </>
        );
        break;
    }
  }

  return (
    <Dialog
      open
      onClose={close}
      title={title}
      titleId={titleId}
      width={720}
      canClose={view.kind !== "applying"}
      footer={footer && <div className="orgsetup-footer">{footer}</div>}
    >
      <div className="orgsetup" aria-busy={view.kind === "applying" || undefined}>
        {body}
      </div>
    </Dialog>
  );
}
