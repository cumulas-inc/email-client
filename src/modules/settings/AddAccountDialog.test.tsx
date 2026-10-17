import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { MockBackend, mockAdminConsentUrl } from "../../api/mock";
import { qk } from "../../api/queries";
import type { Account, BackendError } from "../../api/types";
import { useAppStore } from "../../app/store";
import { useToasts } from "../../components/toast";
import { renderWithBackend } from "../../test/render";
import { AddAccountDialog } from "./AddAccountDialog";
import {
  type AddAccountRequest,
  closeAddAccount,
  openAddAccount,
  useAddAccountRequest,
} from "./addAccountStore";

const ADMIN_URL = mockAdminConsentUrl("contoso.example");

function openFlow(request: AddAccountRequest, backend = new MockBackend()) {
  const utils = renderWithBackend(<AddAccountDialog />, { backend });
  act(() => openAddAccount(request));
  return { ...utils, backend };
}

function toastMessages() {
  return useToasts.getState().toasts.map((t) => t.message);
}

/** The paragraph that starts with `text`, though <strong> splits its words. */
function paragraph(text: string) {
  return screen.getByText(
    (_, el) => el?.tagName === "P" && (el.textContent ?? "").startsWith(text),
  );
}

function rejection(code: BackendError["code"], message: string): BackendError {
  return { code, message };
}

/** Holds a backend call until `release()`, to look at the waiting state. */
function holdCalls<K extends "addMicrosoftAccount" | "addImapAccount">(
  backend: MockBackend,
  method: K,
) {
  const original = backend[method].bind(backend) as (...args: unknown[]) => Promise<Account>;
  let release!: () => void;
  const gate = new Promise<void>((resolve) => (release = resolve));
  const spy = vi.spyOn(backend, method).mockImplementation((async (...args: unknown[]) => {
    await gate;
    return original(...args);
  }) as never);
  return { spy, release };
}

beforeEach(() => {
  useAppStore.setState({ addAccountOpen: false, settingsOpen: null });
  useAddAccountRequest.setState({ request: { step: "choose" } });
  useToasts.setState({ toasts: [] });
});

describe("Microsoft sign-in", () => {
  it("waits for the browser, then adds the account and closes", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const { spy: signIn, release } = holdCalls(backend, "addMicrosoftAccount");
    const { client } = openFlow({ step: "microsoft" }, backend);

    await user.type(screen.getByLabelText("Email address (optional)"), "sam@contoso.example");
    await user.click(screen.getByRole("button", { name: "Sign in with Microsoft" }));

    expect(
      await screen.findByRole("heading", { name: "Continue in your browser" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();

    release();
    await waitFor(() => expect(useAppStore.getState().addAccountOpen).toBe(false));
    expect(signIn).toHaveBeenCalledWith("sam@contoso.example", { kind: "all" });
    expect(toastMessages()).toContainEqual(expect.stringMatching(/^sam@contoso.example is ready/));
    expect(client.getQueryData<Account[]>(qk.accounts())).toContainEqual(
      expect.objectContaining({ email: "sam@contoso.example", kind: "microsoft" }),
    );
  });

  it("says that the push service hears when the Inbox or calendar changes", async () => {
    openFlow({ step: "microsoft" });
    expect(
      await screen.findByText(
        /Microsoft tells Cumulas's push service \(sync\.cumulas\.co\) when your Inbox or calendar changes, but never what's in it/,
      ),
    ).toBeInTheDocument();
  });

  it("leaves the push service out when it's turned off", async () => {
    const backend = new MockBackend();
    const settings = await backend.getSettings();
    await backend.updateSettings({ ...settings, usePushRelay: false });
    openFlow({ step: "microsoft" }, backend);
    await screen.findByRole("button", { name: "Sign in with Microsoft" });
    await screen.findByText(/never sees your password/);
    expect(screen.queryByText(/push service/)).not.toBeInTheDocument();
  });

  it("cancels a pending sign-in and goes back to the form", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const signIn = vi.spyOn(backend, "addMicrosoftAccount");
    const cancel = vi.spyOn(backend, "cancelMicrosoftSignIn");
    openFlow({ step: "microsoft" }, backend);

    // "slow@…" waits in the demo backend until the sign-in is canceled.
    await user.type(screen.getByLabelText("Email address (optional)"), "slow@contoso.example");
    await user.click(screen.getByRole("button", { name: "Sign in with Microsoft" }));
    expect(
      await screen.findByRole("heading", { name: "Continue in your browser" }),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(cancel).toHaveBeenCalledTimes(1);
    // The backend reports the cancellation; that isn't shown as a problem.
    await expect(signIn.mock.results[0]!.value).rejects.toMatchObject({
      code: "signInCancelled",
    });

    expect(useAppStore.getState().addAccountOpen).toBe(true);
    expect(screen.queryByRole("heading", { name: "Sign-in was canceled" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Sign in with Microsoft" })).toBeInTheDocument();
    const email = screen.getByLabelText("Email address (optional)");
    expect(email).toHaveValue("slow@contoso.example");
    await waitFor(() => expect(email).toHaveFocus());
  });

  it("cancels the sign-in when the dialog is closed while waiting", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const signIn = vi.spyOn(backend, "addMicrosoftAccount");
    const cancel = vi.spyOn(backend, "cancelMicrosoftSignIn");
    openFlow({ step: "microsoft" }, backend);

    await user.type(screen.getByLabelText("Email address (optional)"), "slow@contoso.example");
    await user.click(screen.getByRole("button", { name: "Sign in with Microsoft" }));
    const dialog = await screen.findByRole("dialog", { name: "Add a Microsoft account" });
    await screen.findByRole("heading", { name: "Continue in your browser" });

    // Escape on a <dialog> fires "cancel".
    fireEvent(dialog, new Event("cancel", { cancelable: true }));
    expect(cancel).toHaveBeenCalledTimes(1);
    expect(useAppStore.getState().addAccountOpen).toBe(false);
    await expect(signIn.mock.results[0]!.value).rejects.toMatchObject({
      code: "signInCancelled",
    });
    expect(toastMessages()).toEqual([]);
  });

  it("cancels the sign-in from the close button too, but not after it ended", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const cancel = vi.spyOn(backend, "cancelMicrosoftSignIn");
    openFlow({ step: "microsoft" }, backend);

    await user.type(screen.getByLabelText("Email address (optional)"), "slow@contoso.example");
    await user.click(screen.getByRole("button", { name: "Sign in with Microsoft" }));
    await screen.findByRole("heading", { name: "Continue in your browser" });
    await user.click(screen.getByRole("button", { name: "Close" }));
    expect(cancel).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    // Closing from the form doesn't cancel anything.
    act(() => openAddAccount({ step: "microsoft" }));
    await user.click(screen.getByRole("button", { name: "Close" }));
    expect(cancel).toHaveBeenCalledTimes(1);
  });

  it("lets a canceled 'Sign in again' start over", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const signIn = vi.spyOn(backend, "addMicrosoftAccount");
    openFlow({ step: "microsoft", email: "slow@acme.example", autoStart: true }, backend);

    expect(screen.getByRole("dialog", { name: "Sign in again" })).toBeInTheDocument();
    await user.click(await screen.findByRole("button", { name: "Cancel" }));
    expect(await screen.findByRole("button", { name: "Sign in with Microsoft" })).toBeVisible();
    expect(screen.getByLabelText("Email address (optional)")).toHaveValue("slow@acme.example");
    expect(signIn).toHaveBeenCalledTimes(1);
  });

  it("checks the optional email before opening the browser", async () => {
    const user = userEvent.setup();
    const { backend } = openFlow({ step: "microsoft" });
    const signIn = vi.spyOn(backend, "addMicrosoftAccount");

    await user.type(screen.getByLabelText("Email address (optional)"), "not-an-address");
    await user.click(screen.getByRole("button", { name: "Sign in with Microsoft" }));

    const email = screen.getByLabelText("Email address (optional)");
    expect(email).toHaveAttribute("aria-invalid", "true");
    expect(email).toHaveAccessibleDescription(/Enter an email address like name@example.com/);
    expect(signIn).not.toHaveBeenCalled();
  });

  it("explains admin approval with a copyable link, an email to IT and a retry", async () => {
    const user = userEvent.setup();
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    const backend = new MockBackend();
    const openLink = vi.spyOn(backend, "openLink").mockResolvedValue(undefined);
    const signIn = vi.spyOn(backend, "addMicrosoftAccount");
    openFlow({ step: "microsoft" }, backend);

    // The permissions disclosure has one name, before sign-in and after.
    const disclosure = () =>
      screen.getByText("What Cumulas Mail asks for", { selector: "summary" });
    expect(disclosure()).toBeInTheDocument();

    await user.type(screen.getByLabelText("Email address (optional)"), "admin@contoso.example");
    await user.click(screen.getByRole("button", { name: "Sign in with Microsoft" }));

    const heading = await screen.findByRole("heading", {
      name: "Your organization needs to approve Cumulas Mail",
    });
    await waitFor(() => expect(heading).toHaveFocus());
    expect(disclosure()).toBeInTheDocument();
    expect(screen.getByText(/isn't something you did wrong, and nothing is broken/)).toBeVisible();
    expect(screen.getByText("contoso.example")).toBeInTheDocument();

    const link = screen.getByLabelText("Approval link for your IT administrator");
    expect(link).toHaveValue(ADMIN_URL);
    expect(link).toHaveAttribute("readonly");

    await user.click(screen.getByRole("button", { name: "Copy" }));
    expect(writeText).toHaveBeenCalledWith(ADMIN_URL);
    expect(await screen.findByRole("button", { name: "Copied" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Email IT" }));
    expect(openLink).toHaveBeenCalledTimes(1);
    const mailto = openLink.mock.calls[0]![0];
    expect(mailto).toMatch(/^mailto:\?subject=/);
    const decoded = decodeURIComponent(mailto);
    expect(decoded).toContain(ADMIN_URL);
    expect(decoded).toContain("Cumulas Mail");
    expect(decoded).toContain("admin@contoso.example");
    for (const scope of ["Mail.ReadWrite", "Mail.Send", "Calendars.ReadWrite", "offline_access"]) {
      expect(decoded).toContain(scope);
    }

    // After IT approves, "Try again" signs in again with the same address.
    signIn.mockClear();
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(signIn).toHaveBeenCalledWith("admin@contoso.example", { kind: "all" });
    expect(
      await screen.findByRole("heading", {
        name: "Your organization needs to approve Cumulas Mail",
      }),
    ).toBeInTheDocument();
  });

  it("walks through Microsoft's Request approval when the organization offers it", async () => {
    const user = userEvent.setup();
    const { backend } = openFlow({ step: "microsoft" });
    const signIn = vi.spyOn(backend, "addMicrosoftAccount");

    // "workflow@…": the admin consent workflow is on; the request then gets declined.
    await user.type(screen.getByLabelText("Email address (optional)"), "workflow@contoso.example");
    await user.click(screen.getByRole("button", { name: "Sign in with Microsoft" }));
    await screen.findByRole("heading", { name: "Your organization needs to approve Cumulas Mail" });

    const steps = within(screen.getByRole("list", { name: "" }))
      .getAllByRole("listitem")
      .map((li) => li.textContent);
    expect(steps).toEqual([
      "Select Request approval. Microsoft's sign-in page opens with a form to ask your IT team.",
      "Add a short reason, such as “I use Linux and need my work mail”, and send the request.",
      "Microsoft usually emails you when your IT team responds. Once they approve it, come back and select Try again.",
    ]);
    expect(screen.getByText("Or send the approval link to IT yourself:")).toBeInTheDocument();
    expect(screen.queryByText(/You can also select/)).not.toBeInTheDocument();
    // Request approval is the main action, so Email IT isn't primary next to it.
    const request = screen.getByRole("button", { name: "Request approval" });
    expect(request).toHaveClass("btn--primary");
    expect(screen.getByRole("button", { name: "Email IT" })).not.toHaveClass("btn--primary");

    signIn.mockClear();
    await user.click(request);
    expect(signIn).toHaveBeenCalledWith("workflow@contoso.example", { kind: "all" });
    expect(
      await screen.findByRole("heading", { name: "Cumulas Mail doesn't have permission yet" }),
    ).toBeInTheDocument();
    expect(paragraph("If you haven't sent a request yet:")).toHaveTextContent(
      "If you haven't sent a request yet: select Try again, then Request approval on Microsoft's page.",
    );
  });

  it("handles a canceled browser sign-in quietly", async () => {
    const user = userEvent.setup();
    const { backend } = openFlow({ step: "microsoft" });
    const signIn = vi.spyOn(backend, "addMicrosoftAccount");

    await user.type(screen.getByLabelText("Email address (optional)"), "cancel@contoso.example");
    await user.click(screen.getByRole("button", { name: "Sign in with Microsoft" }));

    expect(
      await screen.findByRole("heading", { name: "Sign-in was canceled" }),
    ).toBeInTheDocument();
    expect(screen.getByText(/Nothing was changed/)).toBeInTheDocument();

    signIn.mockResolvedValueOnce({ needsPermission: [] } as unknown as Account);
    signIn.mockClear();
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(signIn).toHaveBeenCalledWith("cancel@contoso.example", { kind: "all" });
  });

  it("explains why the permissions are needed when consent is declined", async () => {
    const user = userEvent.setup();
    const { backend } = openFlow({ step: "microsoft" });
    vi.spyOn(backend, "addMicrosoftAccount").mockRejectedValueOnce(
      rejection("consentDeclined", "The user declined consent."),
    );

    await user.click(screen.getByRole("button", { name: "Sign in with Microsoft" }));

    expect(
      await screen.findByRole("heading", { name: "Cumulas Mail doesn't have permission yet" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Send mail as you")).toBeInTheDocument();
    expect(screen.getByText("Read and write your calendars")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
  });

  it("points to the Microsoft app registration settings when not configured", async () => {
    const user = userEvent.setup();
    const { backend } = openFlow({ step: "microsoft" });
    vi.spyOn(backend, "addMicrosoftAccount").mockRejectedValueOnce(
      rejection("notConfigured", "No Microsoft application ID is configured."),
    );

    await user.click(screen.getByRole("button", { name: "Sign in with Microsoft" }));
    expect(
      await screen.findByRole("heading", { name: "Microsoft sign-in isn't set up yet" }),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Open Microsoft settings" }));
    expect(useAppStore.getState().addAccountOpen).toBe(false);
    expect(useAppStore.getState().settingsOpen).toBe("microsoft");
  });

  it("shows other errors with their details", async () => {
    const user = userEvent.setup();
    const { backend } = openFlow({ step: "microsoft" });
    vi.spyOn(backend, "addMicrosoftAccount")
      .mockRejectedValueOnce(rejection("provider", "Graph returned 503."))
      .mockRejectedValueOnce(rejection("network", "dns error"));

    await user.click(screen.getByRole("button", { name: "Sign in with Microsoft" }));
    expect(await screen.findByRole("heading", { name: "Couldn't sign in" })).toBeInTheDocument();
    expect(screen.getByText("Graph returned 503.")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(
      await screen.findByRole("heading", { name: "Couldn't reach Microsoft" }),
    ).toBeInTheDocument();
  });

  it("signs in again straight away for an existing account", async () => {
    const backend = new MockBackend();
    const signIn = vi.spyOn(backend, "addMicrosoftAccount");
    openFlow({ step: "microsoft", email: "alex.morgan@acme.example", autoStart: true }, backend);

    expect(screen.getByRole("dialog", { name: "Sign in again" })).toBeInTheDocument();
    await waitFor(() => expect(useAppStore.getState().addAccountOpen).toBe(false));
    expect(signIn).toHaveBeenCalledTimes(1);
    // What the account has now, not everything.
    expect(signIn).toHaveBeenCalledWith("alex.morgan@acme.example", { kind: "current" });
    expect(toastMessages()).toContain("You're signed in again as alex.morgan@acme.example.");
  });
});

describe("Microsoft sign-in with or without admin approval", () => {
  const emailField = () => screen.getByLabelText("Email address (optional)");
  const approvalLink = () => screen.getByLabelText("Approval link for your IT administrator");
  const steps = () =>
    Array.from(document.querySelectorAll(".consent-steps li"), (li) => li.textContent);

  async function signInAs(user: ReturnType<typeof userEvent.setup>, hint: string) {
    await user.type(emailField(), hint);
    await user.click(screen.getByRole("button", { name: "Sign in with Microsoft" }));
  }

  /** The last mailto: link opened, decoded. */
  function lastMailto(openLink: { mock: { lastCall?: unknown[] } }) {
    return decodeURIComponent(String(openLink.mock.lastCall?.[0]));
  }

  it("starts with mail only where the organization approved just that", async () => {
    const user = userEvent.setup();
    const { backend, client } = openFlow({ step: "microsoft" });
    const signIn = vi.spyOn(backend, "addMicrosoftAccount");

    await signInAs(user, "partial@contoso.example");
    await screen.findByRole("heading", { name: "Your organization needs to approve Cumulas Mail" });
    expect(paragraph("Your organization may let you use mail")).toHaveTextContent(
      "Your organization may let you use mail before an administrator approves the rest. Select Try with mail only to sign in without calendar, contacts and tasks; you can turn them on later.",
    );

    await user.click(screen.getByRole("button", { name: "Try with mail only" }));
    expect(signIn).toHaveBeenLastCalledWith("partial@contoso.example", { kind: "mailOnly" });
    await waitFor(() => expect(useAppStore.getState().addAccountOpen).toBe(false));
    expect(toastMessages()).toContain(
      "partial@contoso.example is ready. Your mail will appear in a moment. Cumulas Mail needs permission for your calendar, contacts, tasks and mailbox time zone (Settings → Accounts).",
    );
    expect(client.getQueryData<Account[]>(qk.accounts())).toContainEqual(
      expect.objectContaining({
        email: "partial@contoso.example",
        needsPermission: ["calendar", "contacts", "tasks", "mailboxSettings"],
      }),
    );
  });

  it("says when mail needs approval too, and asks for everything again", async () => {
    const user = userEvent.setup();
    const { backend } = openFlow({ step: "microsoft" });
    const signIn = vi.spyOn(backend, "addMicrosoftAccount");

    await signInAs(user, "admin@contoso.example");
    await user.click(await screen.findByRole("button", { name: "Try with mail only" }));
    expect(signIn).toHaveBeenLastCalledWith("admin@contoso.example", { kind: "mailOnly" });
    expect(
      await screen.findByText(
        "In your organization, mail needs an administrator's approval too, so mail only didn't help.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Try with mail only" })).not.toBeInTheDocument();

    signIn.mockClear();
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(signIn).toHaveBeenCalledWith("admin@contoso.example", { kind: "all" });
  });

  it("keeps a declined mail-only sign-in to mail", async () => {
    const user = userEvent.setup();
    const { backend } = openFlow({ step: "microsoft" });
    const signIn = vi
      .spyOn(backend, "addMicrosoftAccount")
      .mockRejectedValueOnce({
        code: "adminConsentRequired",
        message: "",
        details: { adminConsentUrl: ADMIN_URL, tenant: "contoso.example" },
      })
      .mockRejectedValueOnce(rejection("consentDeclined", "Declined."));

    await signInAs(user, "sam@contoso.example");
    await user.click(await screen.findByRole("button", { name: "Try with mail only" }));
    await screen.findByRole("heading", { name: "Cumulas Mail doesn't have permission yet" });
    // Only what mail needs is listed.
    expect(screen.getByText("Send mail as you")).toBeInTheDocument();
    expect(screen.queryByText("Read and write your calendars")).not.toBeInTheDocument();

    signIn.mockClear();
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(signIn).toHaveBeenCalledWith("sam@contoso.example", { kind: "mailOnly" });
  });

  it("explains a declined permission without blame", async () => {
    const user = userEvent.setup();
    openFlow({ step: "microsoft" });

    await signInAs(user, "declined@contoso.example");
    await screen.findByRole("heading", { name: "Cumulas Mail doesn't have permission yet" });
    expect(paragraph("Microsoft didn't give")).toHaveTextContent(
      "Microsoft didn't give Cumulas Mail permission to open your mailbox. It only ever acts as you, and your data stays on this computer.",
    );
    expect(paragraph("If you sent a request")).toHaveTextContent(
      "If you sent a request to your IT team: Microsoft usually emails you when they respond. Once they approve it, select Try again.",
    );
    expect(paragraph("If you selected Cancel")).toHaveTextContent(
      "If you selected Cancel or didn't accept: select Try again, then Accept on Microsoft's page.",
    );
    expect(screen.getByText("Read and write your calendars")).toBeInTheDocument();
  });

  it("leaves IT out of a declined personal account", async () => {
    const user = userEvent.setup();
    openFlow({ step: "microsoft" });
    await signInAs(user, "declined@outlook.com");
    await screen.findByRole("heading", { name: "Cumulas Mail doesn't have permission yet" });
    expect(screen.queryByText(/If you sent a request/)).not.toBeInTheDocument();
    expect(paragraph("If you selected Cancel")).toBeInTheDocument();
  });

  it("says when Microsoft's page didn't come back in time, and finds the approval link", async () => {
    const user = userEvent.setup();
    const { backend } = openFlow({ step: "microsoft" });
    const ask = vi.spyOn(backend, "microsoftAdminConsent");

    await signInAs(user, "timeout@fabrikam.example");
    await screen.findByRole("heading", { name: "Sign-in timed out" });
    expect(
      screen.getByText(
        "Microsoft's page didn't send you back to Cumulas Mail within 5 minutes. Nothing was changed.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("Stuck on Microsoft's page?")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Microsoft said I need admin approval" }));
    await screen.findByRole("heading", { name: "Your organization needs to approve Cumulas Mail" });
    expect(ask).toHaveBeenCalledWith(undefined, "timeout@fabrikam.example");
    expect(approvalLink()).toHaveValue(mockAdminConsentUrl("fabrikam.example"));
    // Whether Microsoft's page offered Request approval isn't known.
    expect(paragraph("If Microsoft's page offered")).toHaveTextContent(
      "If Microsoft's page offered Request approval, you can use it instead: select Try again and send the request from there.",
    );
  });

  it("stops waiting for the browser when Microsoft said admin approval is needed", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const signIn = vi.spyOn(backend, "addMicrosoftAccount");
    const cancel = vi.spyOn(backend, "cancelMicrosoftSignIn");
    openFlow({ step: "microsoft" }, backend);

    await signInAs(user, "slow@fabrikam.example");
    await screen.findByRole("heading", { name: "Continue in your browser" });
    await user.click(screen.getByRole("button", { name: "Microsoft said I need admin approval" }));

    expect(cancel).toHaveBeenCalledTimes(1);
    await screen.findByRole("heading", { name: "Your organization needs to approve Cumulas Mail" });
    expect(approvalLink()).toHaveValue(mockAdminConsentUrl("fabrikam.example"));
    await expect(signIn.mock.results[0]!.value).rejects.toMatchObject({
      code: "signInCancelled",
    });
    // The canceled sign-in doesn't replace the panel.
    expect(
      screen.getByRole("heading", { name: "Your organization needs to approve Cumulas Mail" }),
    ).toBeInTheDocument();
  });

  it("offers the approval link after a cancel, but not for a personal address", async () => {
    const user = userEvent.setup();
    openFlow({ step: "microsoft" });

    await signInAs(user, "cancel@contoso.example");
    await screen.findByRole("heading", { name: "Sign-in was canceled" });
    expect(
      screen.getByRole("button", { name: "Microsoft said I need admin approval" }),
    ).toBeInTheDocument();

    act(() => closeAddAccount());
    act(() => openAddAccount({ step: "microsoft" }));
    await signInAs(user, "cancel@outlook.com");
    await screen.findByRole("heading", { name: "Sign-in was canceled" });
    expect(
      screen.queryByRole("button", { name: "Microsoft said I need admin approval" }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Microsoft showed an error code" })).toBeVisible();
  });

  it("shows what to do for an error code Microsoft showed", async () => {
    const user = userEvent.setup();
    const { backend } = openFlow({ step: "microsoft" });
    const openLink = vi.spyOn(backend, "openLink").mockResolvedValue(undefined);
    const codeButton = () => screen.getByRole("button", { name: "Microsoft showed an error code" });
    const enterCode = async (code: string) => {
      await user.click(codeButton());
      if (code) await user.type(screen.getByLabelText("Error code"), code);
      await user.click(screen.getByRole("button", { name: "Show what to do" }));
    };

    await signInAs(user, "cancel@contoso.example");
    await screen.findByRole("heading", { name: "Sign-in was canceled" });
    expect(codeButton()).toHaveAttribute("aria-expanded", "false");
    await enterCode("");
    expect(codeButton()).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByLabelText("Error code")).toHaveAccessibleDescription(
      /Type the number after AADSTS\./,
    );

    await user.type(screen.getByLabelText("Error code"), "AADSTS50105");
    await user.click(screen.getByRole("button", { name: "Show what to do" }));
    await screen.findByRole("heading", { name: "You haven't been given access to Cumulas Mail" });
    expect(screen.getByText("Error code: AADSTS50105")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Try again" }));
    await screen.findByRole("heading", { name: "Sign-in was canceled" });
    await enterCode("90095");
    await screen.findByRole("heading", { name: "Your organization needs to approve Cumulas Mail" });
    expect(screen.getByRole("button", { name: "Request approval" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Try again" }));
    await screen.findByRole("heading", { name: "Sign-in was canceled" });
    await enterCode("12345");
    expect(await screen.findByText("Cumulas Mail doesn't know AADSTS12345.")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Look it up on Microsoft's site" }));
    expect(openLink).toHaveBeenCalledWith("https://login.microsoftonline.com/error?code=12345");
  });

  it.each([
    ["unassigned", "You haven't been given access to Cumulas Mail", 50105],
    ["disabled", "Your organization has blocked Cumulas Mail", 7000112],
    ["notsetup", "Your organization isn't set up for Cumulas Mail yet", 65005],
    ["noservice", "Your organization's Microsoft 365 is missing a service", 650052],
    ["mfa", "Set up your sign-in security info first", 53004],
    ["device", "Your organization's sign-in rules require a managed device", 53000],
    ["blocked", "Your organization's sign-in rules blocked this sign-in", 53003],
  ])("explains a sign-in %s@ is blocked from", async (prefix, title, code) => {
    const user = userEvent.setup();
    const { backend } = openFlow({ step: "microsoft" });
    const openLink = vi.spyOn(backend, "openLink").mockResolvedValue(undefined);

    await signInAs(user, `${prefix}@contoso.example`);
    await screen.findByRole("heading", { name: title });
    expect(screen.getByText(`Error code: AADSTS${code}`)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Email IT" }));
    const mailto = lastMailto(openLink);
    expect(mailto).toMatch(/^mailto:\?subject=Cumulas Mail sign-in is blocked \(AADSTS\d+\)&body=/);
    expect(mailto).toContain(`(AADSTS${code})`);
    expect(mailto).toContain(`${prefix}@contoso.example`);
    expect(mailto).toContain("More for administrators: https://flatpak.cumulas.co/admin-consent/");
  });

  it("offers approving an organization that isn't set up, and Cumulas's issues", async () => {
    const user = userEvent.setup();
    const { backend } = openFlow({ step: "microsoft" });
    const openLink = vi.spyOn(backend, "openLink").mockResolvedValue(undefined);

    await signInAs(user, "notsetup@contoso.example");
    await screen.findByRole("heading", {
      name: "Your organization isn't set up for Cumulas Mail yet",
    });
    expect(approvalLink()).toHaveValue(ADMIN_URL);
    await user.click(screen.getByRole("button", { name: "Email IT" }));
    expect(lastMailto(openLink)).toContain(
      `Approving it may fix this; an administrator can open this link and sign in: ${ADMIN_URL}`,
    );
    await user.click(screen.getByRole("button", { name: "Open approval page" }));
    expect(openLink).toHaveBeenLastCalledWith(ADMIN_URL);
    await user.click(screen.getByRole("button", { name: "Let Cumulas know" }));
    expect(openLink).toHaveBeenLastCalledWith("https://github.com/cumulas-inc/email-client/issues");
  });

  it("names the publisher and the page for administrators in the request to IT", async () => {
    const user = userEvent.setup();
    const { backend } = openFlow({ step: "microsoft" });
    const openLink = vi.spyOn(backend, "openLink").mockResolvedValue(undefined);

    await signInAs(user, "admin@contoso.example");
    await screen.findByRole("heading", { name: "Your organization needs to approve Cumulas Mail" });
    expect(screen.queryByText(/can't be reached/)).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Email IT" }));
    const mailto = lastMailto(openLink);
    expect(mailto).toContain("Cumulas Inc, a Microsoft-verified publisher");
    expect(mailto).toContain("https://flatpak.cumulas.co/admin-consent/");
  });

  it("leaves both out for an organization's own app registration", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ customClient: true });
    const openLink = vi.spyOn(backend, "openLink").mockResolvedValue(undefined);
    openFlow({ step: "microsoft" }, backend);

    await signInAs(user, "admin@contoso.example");
    await screen.findByRole("heading", { name: "Your organization needs to approve Cumulas Mail" });
    expect(approvalLink()).toHaveValue(mockAdminConsentUrl("contoso.example", true));
    expect(
      screen.getByText(/After approving, your browser shows a page that can't be reached/),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Email IT" }));
    const mailto = lastMailto(openLink);
    expect(mailto).not.toContain("Microsoft-verified publisher");
    expect(mailto).not.toContain("More for administrators");
    expect(mailto).not.toContain("Sync.Receive");
  });

  it("turns on a feature for an account whose mail works", async () => {
    const backend = new MockBackend({ missing: ["calendar"] });
    const signIn = vi.spyOn(backend, "addMicrosoftAccount");
    openFlow(
      {
        step: "microsoft",
        email: "alex.morgan@acme.example",
        autoStart: true,
        permissions: { kind: "feature", feature: "calendar" },
      },
      backend,
    );

    expect(screen.getByRole("dialog", { name: "Turn on calendar" })).toBeInTheDocument();
    await waitFor(() => expect(useAppStore.getState().addAccountOpen).toBe(false));
    expect(signIn).toHaveBeenCalledWith("alex.morgan@acme.example", {
      kind: "feature",
      feature: "calendar",
    });
    expect(toastMessages()).toEqual(["Turned on calendar for alex.morgan@acme.example."]);
    const [work] = await backend.listAccounts();
    expect(work!.needsPermission).toEqual([]);
    expect((await backend.listCalendars()).map((c) => c.accountId)).toContain("acct-work");
  });

  it("explains approving a feature, with Turn on to try again", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    await backend.addMicrosoftAccount("partial@contoso.example", { kind: "mailOnly" });
    const openLink = vi.spyOn(backend, "openLink").mockResolvedValue(undefined);
    const signIn = vi.spyOn(backend, "addMicrosoftAccount");
    const request = {
      step: "microsoft",
      email: "partial@contoso.example",
      autoStart: true,
      permissions: { kind: "feature", feature: "contacts" },
    } as const;
    openFlow(request, backend);

    await screen.findByRole("heading", {
      name: "Your organization needs to approve access to your contacts",
    });
    expect(paragraph("Your mail keeps working.")).toHaveTextContent(
      "Your mail keeps working. contoso.example only lets apps use your contacts after its IT team approves them.",
    );
    expect(steps()).toEqual([
      "Send the approval link below to your IT team.",
      "Once they've approved it, Cumulas Mail picks up the approval by itself within a couple of hours. To use it right away, select Turn on.",
    ]);
    expect(screen.queryByRole("button", { name: "Try with mail only" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Back" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Try again" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Email IT" }));
    expect(lastMailto(openLink)).toContain(
      "My mail works, but Cumulas Mail needs an administrator's approval to use my contacts.",
    );

    signIn.mockClear();
    signIn.mockRejectedValueOnce(rejection("consentDeclined", "Declined."));
    await user.click(screen.getByRole("button", { name: "Turn on" }));
    expect(signIn).toHaveBeenCalledWith("partial@contoso.example", request.permissions);
    await screen.findByRole("heading", { name: "Contacts weren't turned on" });
    expect(paragraph("Your mail keeps working.")).toHaveTextContent(
      "Your mail keeps working. Microsoft didn't give Cumulas Mail permission to use your contacts.",
    );
    expect(screen.queryByText("Send mail as you")).not.toBeInTheDocument();
  });
});

describe("IMAP account", () => {
  async function typeEmail(user: ReturnType<typeof userEvent.setup>, email: string) {
    const field = screen.getByLabelText("Email address");
    await user.clear(field);
    await user.type(field, email);
  }

  function servers() {
    const [imapPort, smtpPort] = screen.getAllByLabelText("Port");
    const [imapTls, smtpTls] = screen.getAllByLabelText("Security");
    return {
      imapHost: (screen.getByLabelText("IMAP server") as HTMLInputElement).value,
      imapPort: (imapPort as HTMLInputElement).value,
      imapTls: (imapTls as HTMLSelectElement).value,
      smtpHost: (screen.getByLabelText("SMTP server") as HTMLInputElement).value,
      smtpPort: (smtpPort as HTMLInputElement).value,
      smtpTls: (smtpTls as HTMLSelectElement).value,
      username: (screen.getByLabelText("User name") as HTMLInputElement).value,
    };
  }

  it("fills in server settings for well-known providers", async () => {
    const user = userEvent.setup();
    openFlow({ step: "imap" });

    await typeEmail(user, "sam@gmail.com");
    expect(screen.getByText("Gmail needs an app password")).toBeInTheDocument();
    expect(screen.getByLabelText("Password")).toHaveAccessibleDescription(
      "Use an app password, not your usual Gmail password.",
    );
    const toggle = screen.getByRole("button", { name: /Advanced settings/ });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    await user.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(servers()).toEqual({
      imapHost: "imap.gmail.com",
      imapPort: "993",
      imapTls: "tls",
      smtpHost: "smtp.gmail.com",
      smtpPort: "465",
      smtpTls: "tls",
      username: "sam@gmail.com",
    });

    await typeEmail(user, "sam@icloud.com");
    expect(screen.getByText("iCloud Mail needs an app-specific password")).toBeInTheDocument();
    expect(servers()).toMatchObject({
      imapHost: "imap.mail.me.com",
      smtpHost: "smtp.mail.me.com",
      smtpPort: "587",
      smtpTls: "startTls",
    });

    await typeEmail(user, "sam@proton.me");
    expect(screen.getByText("Proton Mail works through Proton Mail Bridge")).toBeInTheDocument();
    expect(servers()).toMatchObject({
      imapHost: "127.0.0.1",
      imapPort: "1143",
      imapTls: "startTls",
      smtpPort: "1025",
    });

    await typeEmail(user, "sam@example.org");
    expect(screen.queryByText(/needs an app/)).not.toBeInTheDocument();
    expect(servers()).toMatchObject({
      imapHost: "imap.example.org",
      imapPort: "993",
      smtpHost: "smtp.example.org",
      smtpPort: "587",
      smtpTls: "startTls",
      username: "sam@example.org",
    });
  });

  it("suggests Microsoft sign-in for Outlook.com addresses", async () => {
    const user = userEvent.setup();
    openFlow({ step: "imap" });

    await typeEmail(user, "sam@hotmail.com");
    expect(screen.getByText("Outlook.com works best with Microsoft sign-in")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Use Microsoft sign-in" }));

    expect(screen.getByRole("dialog", { name: "Add a Microsoft account" })).toBeInTheDocument();
    expect(screen.getByLabelText("Email address (optional)")).toHaveValue("sam@hotmail.com");
  });

  it("uses edited advanced settings", async () => {
    const user = userEvent.setup();
    const { backend } = openFlow({ step: "imap" });
    const add = vi.spyOn(backend, "addImapAccount");

    await typeEmail(user, "me@example.org");
    await user.type(screen.getByLabelText("Your name"), "Me Myself");
    await user.type(screen.getByLabelText("Password"), "hunter2");
    await user.click(screen.getByRole("button", { name: /Advanced settings/ }));

    const imapHost = screen.getByLabelText("IMAP server");
    await user.clear(imapHost);
    await user.type(imapHost, "mail.example.org");
    const [imapTls, smtpTls] = screen.getAllByLabelText("Security");
    // Changing security moves a standard port along with it.
    await user.selectOptions(imapTls!, "startTls");
    await user.selectOptions(smtpTls!, "tls");
    const username = screen.getByLabelText("User name");
    await user.clear(username);
    await user.type(username, "me");
    expect(screen.getByRole("button", { name: "Use suggested settings" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Add account" }));

    await waitFor(() => expect(add).toHaveBeenCalledTimes(1));
    expect(add).toHaveBeenCalledWith({
      displayName: "Me Myself",
      email: "me@example.org",
      password: "hunter2",
      settings: {
        imapHost: "mail.example.org",
        imapPort: 143,
        imapTls: "startTls",
        smtpHost: "smtp.example.org",
        smtpPort: 465,
        smtpTls: "tls",
        username: "me",
      },
    });
    await waitFor(() => expect(useAppStore.getState().addAccountOpen).toBe(false));
    expect(toastMessages()).toContainEqual(expect.stringMatching(/^me@example.org is ready/));
  });

  it("validates fields inline before checking with the server", async () => {
    const user = userEvent.setup();
    const { backend } = openFlow({ step: "imap" });
    const add = vi.spyOn(backend, "addImapAccount");

    await user.click(screen.getByRole("button", { name: "Add account" }));
    const email = screen.getByLabelText("Email address");
    expect(email).toHaveAttribute("aria-invalid", "true");
    expect(email).toHaveAccessibleDescription("Enter your email address.");
    expect(screen.getByLabelText("Password")).toHaveAccessibleDescription("Enter your password.");
    await waitFor(() => expect(email).toHaveFocus());

    await typeEmail(user, "me@example.org");
    await user.type(screen.getByLabelText("Password"), "pw");
    await user.click(screen.getByRole("button", { name: /Advanced settings/ }));
    const [imapPort] = screen.getAllByLabelText("Port");
    await user.clear(imapPort!);
    await user.type(imapPort!, "70000");
    const [, smtpTls] = screen.getAllByLabelText("Security");
    await user.selectOptions(smtpTls!, "none");
    expect(
      screen.getByText(/Without encryption your password is sent as plain text/),
    ).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Add account" }));
    expect(imapPort).toHaveAccessibleDescription("Enter a port between 1 and 65535.");
    expect(smtpTls).toHaveAccessibleDescription(/only allowed to this computer/);
    await waitFor(() => expect(imapPort).toHaveFocus());
    expect(add).not.toHaveBeenCalled();
  });

  it("shows a rejected password next to the password field", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const { release } = holdCalls(backend, "addImapAccount");
    openFlow({ step: "imap" }, backend);

    await typeEmail(user, "me@example.org");
    await user.type(screen.getByLabelText("Password"), "wrong");
    await user.click(screen.getByRole("button", { name: "Add account" }));

    expect(await screen.findByRole("button", { name: /Checking your settings…/ })).toBeDisabled();
    expect(screen.getByLabelText("Password")).toBeDisabled();
    release();
    const password = screen.getByLabelText("Password");
    await waitFor(() => expect(password).toHaveAttribute("aria-invalid", "true"));
    expect(password).toHaveAccessibleDescription(
      /The server didn't accept this email address and password/,
    );
    await waitFor(() => expect(password).toHaveFocus());
    expect(useAppStore.getState().addAccountOpen).toBe(true);
  });

  it("reminds Gmail users about app passwords when the password is rejected", async () => {
    const user = userEvent.setup();
    openFlow({ step: "imap" });

    await typeEmail(user, "me@gmail.com");
    await user.type(screen.getByLabelText("Password"), "wrong");
    await user.click(screen.getByRole("button", { name: "Add account" }));

    await waitFor(() =>
      expect(screen.getByLabelText("Password")).toHaveAccessibleDescription(
        /Gmail didn't accept this password. Make sure you're using an app password/,
      ),
    );
  });

  it("explains connection problems and opens the server settings", async () => {
    const user = userEvent.setup();
    const { backend } = openFlow({ step: "imap" });
    vi.spyOn(backend, "addImapAccount").mockRejectedValueOnce(
      rejection("network", "Connection refused (os error 111)"),
    );

    await typeEmail(user, "me@example.org");
    await user.type(screen.getByLabelText("Password"), "pw");
    await user.click(screen.getByRole("button", { name: "Add account" }));

    const alert = await screen.findByRole("alert");
    expect(within(alert).getByText("Couldn't connect to the mail server")).toBeInTheDocument();
    expect(within(alert).getByText(/Check the server names and ports/)).toBeInTheDocument();
    expect(within(alert).getByText("Connection refused (os error 111)")).toBeInTheDocument();
    expect(screen.getByLabelText("IMAP server")).toBeVisible();
  });

  it("shows server-side input errors inline", async () => {
    const user = userEvent.setup();
    openFlow({ step: "imap" });

    await typeEmail(user, "alex@morgan-family.example");
    await user.type(screen.getByLabelText("Password"), "pw");
    await user.click(screen.getByRole("button", { name: "Add account" }));

    const email = screen.getByLabelText("Email address");
    await waitFor(() =>
      expect(email).toHaveAccessibleDescription(
        "alex@morgan-family.example has already been added.",
      ),
    );
    expect(email).toHaveAttribute("aria-invalid", "true");
  });

  it("shows and hides the password", async () => {
    const user = userEvent.setup();
    openFlow({ step: "imap" });
    const password = screen.getByLabelText("Password");

    await user.click(screen.getByRole("button", { name: "Show password" }));
    expect(password).toHaveAttribute("type", "text");
    await user.click(screen.getByRole("button", { name: "Hide password" }));
    expect(password).toHaveAttribute("type", "password");
  });
});
