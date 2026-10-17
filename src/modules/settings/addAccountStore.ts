import { create } from "zustand";

import type { AccountFeature, PermissionRequest } from "../../api/types";
import { useAppStore } from "../../app/store";

export type AddAccountStep = "choose" | "microsoft" | "imap";

export interface AddAccountRequest {
  step: AddAccountStep;
  /** Email to prefill (Microsoft login hint or IMAP address). */
  email?: string;
  /** Microsoft: start signing in straight away ("Sign in again"). */
  autoStart?: boolean;
  /**
   * Microsoft: what to ask permission for; by default everything for a new
   * account, and what the account has for "Sign in again".
   */
  permissions?: PermissionRequest;
  /**
   * The id of an element to focus once the sign-in succeeds, if it's there;
   * otherwise the keyboard goes back to what opened the dialog.
   */
  returnFocusTo?: string;
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

/**
 * "Turn on": signs in again, asking for what the account has plus the
 * feature. `returnFocusTo` gets the keyboard once it's on (the notice with
 * the button goes then), or the region around it until it's enabled.
 */
export function openTurnOn(email: string, feature: AccountFeature, returnFocusTo?: string): void {
  openAddAccount({
    step: "microsoft",
    email,
    autoStart: true,
    permissions: { kind: "feature", feature },
    returnFocusTo,
  });
}

/** Forgets the pending request once the dialog has read it. */
export function resetAddAccountRequest(): void {
  useAddAccountRequest.setState({ request: DEFAULT_REQUEST });
}

export function closeAddAccount(): void {
  useAddAccountRequest.setState({ request: DEFAULT_REQUEST });
  useAppStore.getState().setAddAccountOpen(false);
}
