import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { MockBackend } from "../../api/mock";
import { qk } from "../../api/queries";
import type { Account, BackendError } from "../../api/types";
import { useAppStore } from "../../app/store";
import { useToasts } from "../../components/toast";
import { renderWithBackend } from "../../test/render";
import { AddAccountDialog } from "./AddAccountDialog";
import { type AddAccountRequest, openAddAccount, useAddAccountRequest } from "./addAccountStore";

const ADMIN_URL =
  "https://login.microsoftonline.com/contoso.example/v2.0/adminconsent?client_id=00000000-0000-0000-0000-000000000000";

function openFlow(request: AddAccountRequest, backend = new MockBackend()) {
  const utils = renderWithBackend(<AddAccountDialog />, { backend });
  act(() => openAddAccount(request));
  return { ...utils, backend };
}

function toastMessages() {
  return useToasts.getState().toasts.map((t) => t.message);
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
    expect(signIn).toHaveBeenCalledWith("sam@contoso.example");
    expect(toastMessages()).toContainEqual(expect.stringMatching(/^sam@contoso.example is ready/));
    expect(client.getQueryData<Account[]>(qk.accounts())).toContainEqual(
      expect.objectContaining({ email: "sam@contoso.example", kind: "microsoft" }),
    );
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
    expect(signIn).toHaveBeenCalledWith("admin@contoso.example");
    expect(
      await screen.findByRole("heading", {
        name: "Your organization needs to approve Cumulas Mail",
      }),
    ).toBeInTheDocument();
  });

  it("mentions Microsoft's Request approval button when the organization offers it", async () => {
    const user = userEvent.setup();
    const { backend } = openFlow({ step: "microsoft" });
    const details = { adminConsentUrl: ADMIN_URL, tenant: "contoso.example" };
    const needsApproval = (canRequestApproval: boolean): BackendError => ({
      code: "adminConsentRequired",
      message: "An administrator needs to approve Cumulas Mail.",
      details: { ...details, canRequestApproval },
    });
    vi.spyOn(backend, "addMicrosoftAccount")
      .mockRejectedValueOnce(needsApproval(true))
      .mockRejectedValueOnce(needsApproval(false));
    const heading = { name: "Your organization needs to approve Cumulas Mail" };

    await user.click(screen.getByRole("button", { name: "Sign in with Microsoft" }));
    await screen.findByRole("heading", heading);
    expect(screen.getByText(/You can also select/)).toHaveTextContent(
      "You can also select Request approval on Microsoft's sign-in page, then wait for IT to approve.",
    );

    await user.click(screen.getByRole("button", { name: "Try again" }));
    await screen.findByRole("heading", heading);
    expect(screen.queryByText(/Request approval/)).not.toBeInTheDocument();
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

    signIn.mockResolvedValueOnce({} as Account);
    signIn.mockClear();
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(signIn).toHaveBeenCalledWith("cancel@contoso.example");
  });

  it("explains why the permissions are needed when consent is declined", async () => {
    const user = userEvent.setup();
    const { backend } = openFlow({ step: "microsoft" });
    vi.spyOn(backend, "addMicrosoftAccount").mockRejectedValueOnce(
      rejection("consentDeclined", "The user declined consent."),
    );

    await user.click(screen.getByRole("button", { name: "Sign in with Microsoft" }));

    expect(
      await screen.findByRole("heading", { name: "Cumulas Mail needs your permission" }),
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
    expect(signIn).toHaveBeenCalledWith("alex.morgan@acme.example");
    expect(toastMessages()).toContain("You're signed in again as alex.morgan@acme.example.");
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
