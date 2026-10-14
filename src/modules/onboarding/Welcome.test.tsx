import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { MockBackend } from "../../api/mock";
import { useAppStore } from "../../app/store";
import { renderWithBackend } from "../../test/render";
import { AddAccountDialog } from "../settings/AddAccountDialog";
import { useAddAccountRequest } from "../settings/addAccountStore";
import { Welcome } from "./Welcome";

function renderOnboarding() {
  return renderWithBackend(
    <>
      <Welcome />
      <AddAccountDialog />
    </>,
    { backend: new MockBackend({ empty: true }) },
  );
}

beforeEach(() => {
  useAppStore.setState({ addAccountOpen: false, settingsOpen: null });
  useAddAccountRequest.setState({ request: { step: "choose" } });
});

describe("Welcome", () => {
  it("introduces the app and offers both kinds of account", () => {
    renderOnboarding();
    expect(screen.getByRole("heading", { name: "Welcome to Cumulas Mail" })).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Microsoft 365 or Outlook.com" }),
    ).toHaveAccessibleDescription(/Work, school or personal Microsoft account/);
    expect(screen.getByRole("button", { name: "Other email (IMAP)" })).toHaveAccessibleDescription(
      /Gmail.*Fastmail.*iCloud.*Yahoo.*Self-hosted/,
    );
  });

  it("opens Microsoft sign-in from the Microsoft choice", async () => {
    const user = userEvent.setup();
    renderOnboarding();
    await user.click(screen.getByRole("button", { name: "Microsoft 365 or Outlook.com" }));

    expect(screen.getByRole("dialog", { name: "Add a Microsoft account" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Sign in with Microsoft" })).toBeInTheDocument();
    await waitFor(() => expect(screen.getByLabelText("Email address (optional)")).toHaveFocus());

    // Cancelling returns to the welcome screen where it left off.
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Microsoft 365 or Outlook.com" })).toHaveFocus();
  });

  it("opens the IMAP form from the other-email choice", async () => {
    const user = userEvent.setup();
    renderOnboarding();
    await user.click(screen.getByRole("button", { name: "Other email (IMAP)" }));

    expect(screen.getByRole("dialog", { name: "Add an email account" })).toBeInTheDocument();
    await waitFor(() => expect(screen.getByLabelText("Email address")).toHaveFocus());
    expect(screen.getByLabelText("Password")).toHaveAttribute("type", "password");
  });

  it("starts on the choice step when opened without one, with Back to return", async () => {
    const user = userEvent.setup();
    renderOnboarding();
    act(() => useAppStore.getState().setAddAccountOpen(true));

    const chooser = screen.getByRole("dialog", { name: "Add an account" });
    await user.click(within(chooser).getByRole("button", { name: "Other email (IMAP)" }));
    expect(screen.getByRole("dialog", { name: "Add an email account" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Back" }));
    expect(screen.getByRole("dialog", { name: "Add an account" })).toBeInTheDocument();
  });
});
