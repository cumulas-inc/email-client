import { create } from "zustand";

import { useAppStore } from "../../app/store";

export type AddAccountStep = "choose" | "microsoft" | "imap";

export interface AddAccountRequest {
  step: AddAccountStep;
  /** Email to prefill (Microsoft login hint or IMAP address). */
  email?: string;
  /** Microsoft: start signing in straight away ("Sign in again"). */
  autoStart?: boolean;
}

const DEFAULT_REQUEST: AddAccountRequest = { step: "choose" };

/**
 * Which step the add-account dialog opens on. The dialog itself is shown by
 * `addAccountOpen` in the app store, so other modules can keep calling
 * `setAddAccountOpen(true)` and get the first step.
 */
export const useAddAccountRequest = create<{ request: AddAccountRequest }>(() => ({
  request: DEFAULT_REQUEST,
}));

export function openAddAccount(request: AddAccountRequest = DEFAULT_REQUEST): void {
  useAddAccountRequest.setState({ request });
  useAppStore.getState().setAddAccountOpen(true);
}

/** Forgets the pending request once the dialog has read it. */
export function resetAddAccountRequest(): void {
  useAddAccountRequest.setState({ request: DEFAULT_REQUEST });
}

export function closeAddAccount(): void {
  useAddAccountRequest.setState({ request: DEFAULT_REQUEST });
  useAppStore.getState().setAddAccountOpen(false);
}
