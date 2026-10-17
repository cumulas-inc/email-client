import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { MockBackend, mockAdminConsentUrl } from "../../api/mock";
import { useAccounts } from "../../api/queries";
import type { Account, AdminConsentLink } from "../../api/types";
import { useAppStore } from "../../app/store";
import { useToasts } from "../../components/toast";
import { renderWithBackend } from "../../test/render";
import { AddAccountDialog } from "./AddAccountDialog";
import { useAddAccountRequest } from "./addAccountStore";
import { FeaturePermissionNotice } from "./FeaturePermission";
import type { ModuleFeature } from "./microsoft";

const EMAIL = "alex.morgan@acme.example";

function Notice({ feature }: { feature: ModuleFeature }) {
  const { data: accounts = [] } = useAccounts();
  return (
    <>
      <button id="module-new" type="button">
        New
      </button>
      <FeaturePermissionNotice feature={feature} accounts={accounts} returnFocusId="module-new" />
    </>
  );
}

function renderNotice(backend: MockBackend, feature: ModuleFeature = "calendar") {
  return renderWithBackend(
    <>
      <Notice feature={feature} />
      <AddAccountDialog />
    </>,
    { backend },
  );
}

/** Holds a backend call until `release()`. */
function hold<K extends "microsoftAdminConsent" | "addMicrosoftAccount" | "retryMicrosoftFeatures">(
  backend: MockBackend,
  method: K,
) {
  const original = backend[method].bind(backend) as (...args: unknown[]) => Promise<unknown>;
  let release!: () => void;
  const gate = new Promise<void>((resolve) => (release = resolve));
  const spy = vi.spyOn(backend, method).mockImplementation((async (...args: unknown[]) => {
    await gate;
    return original(...args);
  }) as never);
  return { spy, release };
}

function makePersonal(backend: MockBackend) {
  const state = (backend as unknown as { state: { accounts: Account[] } }).state;
  state.accounts[0]!.settings = {
    kind: "microsoft",
    tenantId: "9188040d-6c67-4c5b-b112-36a304b66dad",
    userId: "u",
  };
}

beforeEach(() => {
  useAppStore.setState({ addAccountOpen: false, settingsOpen: null });
  useAddAccountRequest.setState({ request: { step: "choose" } });
  useToasts.setState({ toasts: [] });
});

describe("FeaturePermissionNotice", () => {
  it("shows nothing while every account can use the feature", async () => {
    const backend = new MockBackend({ missing: ["tasks"] });
    const listAccounts = vi.spyOn(backend, "listAccounts");
    const { container } = renderNotice(backend, "contacts");
    await waitFor(() => expect(listAccounts).toHaveBeenCalled());
    await screen.findByRole("button", { name: "New" });
    expect(container.querySelector(".feature-notice")).toBeNull();
  });

  it("turns the feature on, giving the keyboard to the module's New button after", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ missing: ["calendar"] });
    const signIn = vi.spyOn(backend, "addMicrosoftAccount");
    renderNotice(backend);

    await user.click(await screen.findByRole("button", { name: `Turn on calendar for ${EMAIL}` }));
    expect(signIn).toHaveBeenCalledWith(EMAIL, { kind: "feature", feature: "calendar" });
    await waitFor(() => expect(useAppStore.getState().addAccountOpen).toBe(false));
    await waitFor(() => expect(screen.getByRole("button", { name: "New" })).toHaveFocus());
    expect(screen.queryByRole("region", { name: "Calendar needs permission" })).toBeNull();
  });

  it("only offers Turn on to a personal account", async () => {
    const backend = new MockBackend({ missing: ["calendar"] });
    makePersonal(backend);
    renderNotice(backend);

    const notice = await screen.findByRole("region", { name: "Calendar needs permission" });
    expect(within(notice).queryByRole("button", { name: /Ask IT/ })).toBeNull();
    expect(within(notice).getByText("Turn on asks Microsoft for permission.")).toBeInTheDocument();
  });

  it("checks a refused feature again, and emails IT about it", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ refused: ["contacts"] });
    const { spy: retry, release } = hold(backend, "retryMicrosoftFeatures");
    const openLink = vi.spyOn(backend, "openLink").mockResolvedValue(undefined);
    renderNotice(backend, "contacts");

    const check = await screen.findByRole("button", { name: `Check contacts again for ${EMAIL}` });
    await user.click(check);
    expect(retry).toHaveBeenCalledWith("acct-work");
    await waitFor(() => expect(check).toBeDisabled());
    expect(check).toHaveAttribute("aria-busy", "true");
    release();
    await waitFor(() => expect(check).toBeEnabled());

    await user.click(screen.getByRole("button", { name: `Email IT about contacts for ${EMAIL}` }));
    expect(decodeURIComponent(String(openLink.mock.lastCall?.[0]))).toMatch(
      /^mailto:\?subject=Cumulas Mail can't open my contacts&body=/,
    );
  });
});

describe("ApprovalDialog", () => {
  it("loads the approval link, then explains it; Turn on starts the feature's sign-in", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ missing: ["calendar"] });
    const { spy: ask, release } = hold(backend, "microsoftAdminConsent");
    const { spy: signIn } = hold(backend, "addMicrosoftAccount");
    renderNotice(backend);

    const askIt = await screen.findByRole("button", { name: `Ask IT about calendar for ${EMAIL}` });
    await user.click(askIt);
    const dialog = screen.getByRole("dialog", { name: "Ask IT for approval" });
    expect(ask).toHaveBeenCalledWith("acct-work");
    expect(within(dialog).getByRole("status", { name: "Loading the approval link" })).toBeVisible();
    expect(dialog.querySelector('[aria-busy="true"]')).not.toBeNull();

    release();
    const heading = await within(dialog).findByRole("heading", {
      name: "Your organization needs to approve access to your calendar",
    });
    await waitFor(() => expect(heading).toHaveFocus());
    expect(dialog.querySelector('[aria-busy="true"]')).toBeNull();
    expect(within(dialog).getByLabelText("Approval link for your IT administrator")).toHaveValue(
      mockAdminConsentUrl("demo-tenant"),
    );

    await user.click(within(dialog).getByRole("button", { name: "Turn on" }));
    expect(screen.queryByRole("dialog", { name: "Ask IT for approval" })).toBeNull();
    expect(screen.getByRole("dialog", { name: "Turn on calendar" })).toBeInTheDocument();
    expect(signIn).toHaveBeenCalledWith(EMAIL, { kind: "feature", feature: "calendar" });
  });

  it("says why the link couldn't be had, and closes back to Ask IT", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ missing: ["calendar"] });
    vi.spyOn(backend, "microsoftAdminConsent").mockRejectedValue({
      code: "notConfigured",
      message: "Microsoft sign-in isn't available.",
    });
    renderNotice(backend);

    const askIt = await screen.findByRole("button", { name: `Ask IT about calendar for ${EMAIL}` });
    await user.click(askIt);
    const dialog = screen.getByRole("dialog", { name: "Ask IT for approval" });
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "Microsoft sign-in isn't available.",
    );
    // The footer's, not the header's icon.
    await user.click(within(dialog).getAllByRole("button", { name: "Close" }).at(-1)!);
    expect(screen.queryByRole("dialog", { name: "Ask IT for approval" })).toBeNull();
    await waitFor(() => expect(askIt).toHaveFocus());
  });

  it("asks only once for the same account's link", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ missing: ["calendar"] });
    const link: AdminConsentLink = {
      adminConsentUrl: mockAdminConsentUrl("acme.example"),
      tenant: "acme.example",
      adminPageUrl: null,
    };
    const ask = vi.spyOn(backend, "microsoftAdminConsent").mockResolvedValue(link);
    renderNotice(backend);

    for (let i = 0; i < 2; i++) {
      await user.click(
        await screen.findByRole("button", { name: `Ask IT about calendar for ${EMAIL}` }),
      );
      const dialog = screen.getByRole("dialog", { name: "Ask IT for approval" });
      expect(await within(dialog).findByText(/can't be reached/)).toBeInTheDocument();
      await user.click(within(dialog).getAllByRole("button", { name: "Close" }).at(-1)!);
    }
    expect(ask).toHaveBeenCalledTimes(1);
  });
});
