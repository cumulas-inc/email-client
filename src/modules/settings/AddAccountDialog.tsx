import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";

import { useBackend } from "../../api/context";
import { qk } from "../../api/queries";
import { offerStartAtLoginOnce } from "../../app/offers";
import { useAppStore } from "../../app/store";
import { Dialog } from "../../components/Dialog";
import { toast } from "../../components/toast";
import type { Account, PermissionRequest } from "../../api/types";
import { AccountKindOptions } from "./AccountKindOptions";
import {
  closeAddAccount,
  resetAddAccountRequest,
  useAddAccountRequest,
  type AddAccountStep,
} from "./addAccountStore";
import { useFocusOnChange, useRestoreFocus } from "./focus";
import { ImapForm } from "./ImapForm";
import { FEATURE_COPY, featureList } from "./microsoft";
import { MicrosoftSignIn } from "./MicrosoftSignIn";
import { StepLayout } from "./StepLayout";

import "./settings.css";

/**
 * Add an account: choose a kind, then Microsoft sign-in or the IMAP form.
 * Shown while `addAccountOpen` is set; `openAddAccount()` picks the step.
 */
export function AddAccountDialog() {
  const open = useAppStore((s) => s.addAccountOpen);
  return open ? <AddAccountFlow /> : null;
}

const TITLES: Record<AddAccountStep, string> = {
  choose: "Add an account",
  microsoft: "Add a Microsoft account",
  imap: "Add an email account",
};

function AddAccountFlow() {
  const backend = useBackend();
  const client = useQueryClient();
  const [request] = useState(() => useAddAccountRequest.getState().request);
  const [step, setStep] = useState<AddAccountStep>(request.step);
  const [email, setEmail] = useState(request.email ?? "");
  const [phase, setPhase] = useState("form");
  const bodyRef = useRef<HTMLDivElement>(null);
  const reauth = request.autoStart === true;
  const [permissions] = useState<PermissionRequest>(
    () => request.permissions ?? (reauth ? { kind: "current" } : { kind: "all" }),
  );
  const feature = permissions.kind === "feature" ? permissions.feature : null;

  useEffect(() => resetAddAccountRequest(), []);
  useRestoreFocus(request.returnFocusTo);
  useFocusOnChange(bodyRef, `${step}:${phase}`);

  const goTo = (next: AddAccountStep, nextEmail?: string) => {
    if (nextEmail !== undefined) setEmail(nextEmail);
    setPhase("form");
    setStep(next);
  };
  const back = reauth || feature ? undefined : () => goTo("choose");

  /** Closing while the browser sign-in is open abandons it. */
  const close = () => {
    if (step === "microsoft" && phase === "waiting") {
      backend.cancelMicrosoftSignIn().catch(() => undefined);
    }
    closeAddAccount();
  };

  const done = (account: Account, kind: "microsoft" | "imap") => {
    closeAddAccount();
    // After the first account: would they like it to start at login?
    const accounts = client.getQueryData<Account[]>(qk.accounts());
    if (!reauth && accounts?.length === 1 && accounts[0]?.id === account.id) {
      offerStartAtLoginOnce();
    }
    const missing = account.needsPermission.length
      ? ` Cumulas Mail needs permission for your ${featureList(account.needsPermission)} (Settings → Accounts).`
      : "";
    toast.success(
      feature
        ? `Turned on ${FEATURE_COPY[feature].noun} for ${account.email}.`
        : reauth
          ? `You're signed in again as ${account.email}.${missing}`
          : kind === "microsoft"
            ? missing
              ? `${account.email} is ready. Your mail will appear in a moment.${missing}`
              : `${account.email} is ready. Your mail, calendar and contacts will appear in a moment.`
            : `${account.email} is ready. Your mail will appear in a moment.`,
    );
  };

  const openMicrosoftSettings = () => {
    closeAddAccount();
    useAppStore.getState().openSettings("microsoft");
  };

  const title =
    step !== "microsoft"
      ? TITLES[step]
      : feature
        ? `Turn on ${FEATURE_COPY[feature].noun}`
        : reauth
          ? "Sign in again"
          : TITLES[step];

  return (
    <Dialog open onClose={close} title={title} width={560}>
      <div className="add-account" ref={bodyRef}>
        {step === "choose" && (
          <StepLayout
            footer={
              <button type="button" className="btn" onClick={close}>
                Cancel
              </button>
            }
          >
            <p className="add-account__lead">What kind of account would you like to add?</p>
            <AccountKindOptions variant="list" autoFocus onChoose={(kind) => goTo(kind)} />
          </StepLayout>
        )}
        {step === "microsoft" && (
          <MicrosoftSignIn
            initialEmail={email}
            autoStart={reauth}
            permissions={permissions}
            onBack={back}
            onClose={close}
            onDone={(account) => done(account, "microsoft")}
            onOpenSettings={openMicrosoftSettings}
            onPhaseChange={setPhase}
          />
        )}
        {step === "imap" && (
          <ImapForm
            initialEmail={email}
            onBack={back}
            onClose={close}
            onDone={(account) => done(account, "imap")}
            onUseMicrosoft={(address) => goTo("microsoft", address.trim())}
          />
        )}
      </div>
    </Dialog>
  );
}
