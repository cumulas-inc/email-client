import { useEffect, useRef, useState } from "react";

import { useBackend } from "../../api/context";
import { useAppStore } from "../../app/store";
import { Dialog } from "../../components/Dialog";
import { toast } from "../../components/toast";
import type { Account } from "../../api/types";
import { AccountKindOptions } from "./AccountKindOptions";
import {
  closeAddAccount,
  resetAddAccountRequest,
  useAddAccountRequest,
  type AddAccountStep,
} from "./addAccountStore";
import { useFocusOnChange, useRestoreFocus } from "./focus";
import { ImapForm } from "./ImapForm";
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
  const [request] = useState(() => useAddAccountRequest.getState().request);
  const [step, setStep] = useState<AddAccountStep>(request.step);
  const [email, setEmail] = useState(request.email ?? "");
  const [phase, setPhase] = useState("form");
  const bodyRef = useRef<HTMLDivElement>(null);
  const reauth = request.autoStart === true;

  useEffect(() => resetAddAccountRequest(), []);
  useRestoreFocus();
  useFocusOnChange(bodyRef, `${step}:${phase}`);

  const goTo = (next: AddAccountStep, nextEmail?: string) => {
    if (nextEmail !== undefined) setEmail(nextEmail);
    setPhase("form");
    setStep(next);
  };
  const back = reauth ? undefined : () => goTo("choose");

  /** Closing while the browser sign-in is open abandons it. */
  const close = () => {
    if (step === "microsoft" && phase === "waiting") {
      backend.cancelMicrosoftSignIn().catch(() => undefined);
    }
    closeAddAccount();
  };

  const done = (account: Account, kind: "microsoft" | "imap") => {
    closeAddAccount();
    toast.success(
      reauth
        ? `You're signed in again as ${account.email}.`
        : kind === "microsoft"
          ? `${account.email} is ready. Your mail, calendar and contacts will appear in a moment.`
          : `${account.email} is ready. Your mail will appear in a moment.`,
    );
  };

  const openMicrosoftSettings = () => {
    closeAddAccount();
    useAppStore.getState().openSettings("microsoft");
  };

  const title = reauth && step === "microsoft" ? "Sign in again" : TITLES[step];

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
