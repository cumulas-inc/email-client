import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { MockBackend } from "../../api/mock";
import { qk } from "../../api/queries";
import type { ImapAccountUpdate, Settings } from "../../api/types";
import { type SettingsSection, useAppStore } from "../../app/store";
import { useToasts } from "../../components/toast";
import { renderWithBackend } from "../../test/render";
import { AddAccountDialog } from "./AddAccountDialog";
import { useAddAccountRequest } from "./addAccountStore";
import { resetSaveStatus } from "./saveStatus";
import { SettingsDialog } from "./SettingsDialog";

const GUID = "1b2c3d4e-0000-4a5b-8c9d-0123456789ab";

function openAt(section: SettingsSection, backend = new MockBackend()) {
  const updateSettings = vi.spyOn(backend, "updateSettings");
  const utils = renderWithBackend(
    <>
      <SettingsDialog />
      <AddAccountDialog />
    </>,
    { backend },
  );
  act(() => useAppStore.getState().openSettings(section));
  const lastSaved = () => updateSettings.mock.lastCall?.[0];
  return { ...utils, backend, updateSettings, lastSaved };
}

/** The IMAP demo account starts with a sync error (until the next status refresh). */
function withHomeSyncError(backend: MockBackend) {
  vi.spyOn(backend, "getSyncStatuses").mockResolvedValueOnce([
    {
      accountId: "acct-home",
      state: "error",
      message: "Connection timed out.",
      lastSyncedAt: null,
    },
  ]);
  return backend;
}

/** Holds updateImapAccount calls until `release()`, to see the checking state. */
function holdUpdates(backend: MockBackend) {
  const original = backend.updateImapAccount.bind(backend);
  let release!: () => void;
  const gate = new Promise<void>((resolve) => (release = resolve));
  const spy = vi
    .spyOn(backend, "updateImapAccount")
    .mockImplementation(async (accountId: string, update: ImapAccountUpdate) => {
      await gate;
      return original(accountId, update);
    });
  return { spy, release };
}

function settingsDialog() {
  return screen.getByRole("dialog", { name: "Settings" });
}

beforeEach(() => {
  useAppStore.setState({ addAccountOpen: false, settingsOpen: null });
  useAddAccountRequest.setState({ request: { step: "choose" } });
  useToasts.setState({ toasts: [] });
  resetSaveStatus();
});

describe("Settings dialog", () => {
  it("opens on the requested section and navigates with the keyboard", async () => {
    const user = userEvent.setup();
    openAt("general");

    const general = screen.getByRole("tab", { name: "General" });
    expect(general).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tabpanel", { name: "General" })).toBeInTheDocument();
    await waitFor(() => expect(general).toHaveFocus());

    await user.keyboard("{ArrowDown}");
    expect(useAppStore.getState().settingsOpen).toBe("mail");
    await waitFor(() => expect(screen.getByRole("tab", { name: "Mail" })).toHaveFocus());
    expect(await screen.findByRole("heading", { name: "Mail" })).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "About" }));
    expect(screen.getByRole("tabpanel", { name: "About" })).toBeInTheDocument();
  });

  it("closes with the close button", async () => {
    const user = userEvent.setup();
    openAt("accounts");
    await user.click(within(settingsDialog()).getByRole("button", { name: "Close" }));
    expect(useAppStore.getState().settingsOpen).toBeNull();
  });
});

describe("General", () => {
  it("applies the theme immediately and saves it", async () => {
    const user = userEvent.setup();
    const { client, lastSaved } = openAt("general");

    const dark = await screen.findByRole("radio", { name: "Dark" });
    expect(screen.getByRole("radio", { name: "System" })).toBeChecked();
    await user.click(dark);

    expect(dark).toBeChecked();
    expect(client.getQueryData<Settings>(qk.settings())?.theme).toBe("dark");
    await waitFor(() => expect(lastSaved()).toMatchObject({ theme: "dark" }));
    expect(await screen.findByText("Saved")).toBeInTheDocument();
  });

  it("saves notifications and the sync interval", async () => {
    const user = userEvent.setup();
    const { lastSaved } = openAt("general");

    const toggle = await screen.findByRole("switch", { name: "New mail notifications" });
    expect(toggle).toBeChecked();
    await user.click(toggle);
    await waitFor(() => expect(lastSaved()).toMatchObject({ notificationsEnabled: false }));

    await user.selectOptions(screen.getByLabelText("Check for new mail"), "15");
    await waitFor(() =>
      expect(lastSaved()).toMatchObject({ syncIntervalMinutes: 15, notificationsEnabled: false }),
    );
  });

  it("reports a failed save", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    vi.spyOn(backend, "updateSettings").mockRejectedValueOnce({
      code: "storage",
      message: "Disk is full.",
    });
    openAt("general", backend);

    await user.click(await screen.findByRole("radio", { name: "Light" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't save: Disk is full.");
  });
});

describe("Mail", () => {
  it("saves the reading pane, remote content and mark-as-read choices", async () => {
    const user = userEvent.setup();
    const { lastSaved } = openAt("mail");

    await user.click(await screen.findByRole("radio", { name: "Bottom" }));
    await waitFor(() => expect(lastSaved()).toMatchObject({ readingPane: "bottom" }));

    expect(screen.getByText(/tracking pixels/)).toBeInTheDocument();
    const block = screen.getByRole("radio", { name: "Block" });
    expect(block).toBeChecked();
    expect(block).toHaveAccessibleDescription(/Recommended/);
    await user.click(screen.getByRole("radio", { name: "Allow from my contacts" }));
    await waitFor(() => expect(lastSaved()).toMatchObject({ remoteContent: "allowContacts" }));

    const markRead = screen.getByLabelText("Mark messages as read");
    expect(markRead).toHaveValue("0");
    await user.selectOptions(markRead, "Never");
    await waitFor(() => expect(lastSaved()).toMatchObject({ markReadAfterSeconds: null }));
    await user.selectOptions(markRead, "After 3 seconds");
    await waitFor(() => expect(lastSaved()).toMatchObject({ markReadAfterSeconds: 3 }));
  });
});

describe("Microsoft app registration", () => {
  it("validates the application ID before saving it", async () => {
    const user = userEvent.setup();
    const { updateSettings, lastSaved } = openAt("microsoft");

    const clientId = await screen.findByLabelText("Application (client) ID");
    expect(clientId).toHaveAccessibleDescription(
      /Leave empty to use the ID built into Cumulas Mail/,
    );
    await user.type(clientId, "not-a-guid");
    await user.tab();

    expect(clientId).toHaveAttribute("aria-invalid", "true");
    expect(clientId).toHaveAccessibleDescription(/doesn't look like an application ID/);
    expect(updateSettings).not.toHaveBeenCalled();

    await user.clear(clientId);
    await user.type(clientId, GUID.toUpperCase());
    expect(clientId).not.toHaveAttribute("aria-invalid");
    await waitFor(() => expect(lastSaved()).toMatchObject({ microsoftClientId: GUID }));

    await user.clear(clientId);
    await user.tab();
    await waitFor(() => expect(lastSaved()).toMatchObject({ microsoftClientId: null }));
  });

  it("saves the tenant, checking custom values", async () => {
    const user = userEvent.setup();
    const { lastSaved } = openAt("microsoft");

    const who = await screen.findByLabelText("Who can sign in");
    await user.selectOptions(who, "organizations");
    await waitFor(() => expect(lastSaved()).toMatchObject({ microsoftTenant: "organizations" }));

    await user.selectOptions(who, "custom");
    const tenant = screen.getByLabelText("Tenant ID or domain");
    await waitFor(() => expect(tenant).toHaveFocus());
    await user.type(tenant, "contoso");
    await user.tab();
    expect(tenant).toHaveAccessibleDescription(/Enter a tenant ID/);

    await user.type(tenant, ".com{Enter}");
    await waitFor(() => expect(lastSaved()).toMatchObject({ microsoftTenant: "contoso.com" }));
    expect(tenant).not.toHaveAttribute("aria-invalid");
  });

  it("asks for an ID when the build has none", async () => {
    const backend = new MockBackend();
    vi.spyOn(backend, "getAppInfo").mockResolvedValue({
      version: "1.0.0",
      persistentSecrets: true,
      microsoftClientIdBuiltIn: false,
    });
    openAt("microsoft", backend);
    expect(
      await screen.findByText("Microsoft sign-in needs an application ID"),
    ).toBeInTheDocument();
  });
});

describe("Accounts", () => {
  it("lists accounts with their kind and sync state", async () => {
    openAt("accounts");
    const list = await screen.findByRole("list");
    const items = within(list).getAllByRole("listitem");
    expect(items).toHaveLength(2);
    expect(within(items[0]!).getByText("Alex Morgan")).toBeInTheDocument();
    expect(within(items[0]!).getByText("alex.morgan@acme.example")).toBeInTheDocument();
    expect(within(items[0]!).getByText("Microsoft 365")).toBeInTheDocument();
    expect(within(items[0]!).getByText("Synced just now")).toBeInTheDocument();
    expect(within(items[1]!).getByText("IMAP")).toBeInTheDocument();
  });

  it("renames an account inline", async () => {
    const user = userEvent.setup();
    const { backend } = openAt("accounts");
    const update = vi.spyOn(backend, "updateAccount");

    await user.click(await screen.findByRole("button", { name: "Manage Alex (Personal)" }));
    const name = screen.getByLabelText("Account name");
    await user.clear(name);
    await user.type(name, "Home{Enter}");

    expect(update).toHaveBeenCalledWith("acct-home", "Home", undefined);
    expect(await screen.findByRole("button", { name: "Manage Home" })).toBeInTheDocument();
    expect(await screen.findByText("Saved")).toBeInTheDocument();

    // An empty name isn't saved.
    await user.clear(name);
    await user.tab();
    expect(name).toHaveAccessibleDescription(/Enter a name for this account/);
    expect(update).toHaveBeenCalledTimes(1);
  });

  it("undoes an unsaved rename with Escape", async () => {
    const user = userEvent.setup();
    openAt("accounts");
    await user.click(await screen.findByRole("button", { name: "Manage Alex Morgan" }));
    const name = screen.getByLabelText("Account name");
    await user.type(name, " (work)");
    await user.keyboard("{Escape}");
    expect(name).toHaveValue("Alex Morgan");
    expect(useAppStore.getState().settingsOpen).toBe("accounts");
  });

  it("changes an account's colour", async () => {
    const user = userEvent.setup();
    const { backend } = openAt("accounts");
    const update = vi.spyOn(backend, "updateAccount");

    await user.click(await screen.findByRole("button", { name: "Manage Alex (Personal)" }));
    const colours = screen.getByRole("radiogroup", { name: "Colour" });
    expect(within(colours).getAllByRole("radio")).toHaveLength(8);
    expect(within(colours).getByRole("radio", { name: "Berry" })).toBeChecked();

    await user.click(within(colours).getByRole("radio", { name: "Green" }));
    expect(update).toHaveBeenCalledWith("acct-home", undefined, "#107c10");
    expect(within(colours).getByRole("radio", { name: "Green" })).toBeChecked();
  });

  it("removes an account after confirmation", async () => {
    const user = userEvent.setup();
    const { backend } = openAt("accounts");
    const remove = vi.spyOn(backend, "removeAccount");

    await user.click(await screen.findByRole("button", { name: "Manage Alex (Personal)" }));
    await user.click(screen.getByRole("button", { name: "Remove…" }));

    const confirm = screen.getByRole("dialog", { name: "Remove this account?" });
    expect(within(confirm).getByText(/Nothing changes on the server/)).toBeInTheDocument();
    expect(within(confirm).getByText(/saved password is removed/)).toBeInTheDocument();
    expect(within(confirm).getByText(/kept only on this computer/)).toBeInTheDocument();
    await waitFor(() =>
      expect(within(confirm).getByRole("button", { name: "Cancel" })).toHaveFocus(),
    );

    await user.click(within(confirm).getByRole("button", { name: "Cancel" }));
    expect(remove).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog", { name: "Remove this account?" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Remove…" })).toHaveFocus();

    await user.click(screen.getByRole("button", { name: "Remove…" }));
    await user.click(screen.getByRole("button", { name: "Remove account" }));

    expect(remove).toHaveBeenCalledWith("acct-home");
    await waitFor(() =>
      expect(screen.queryByText("alex@morgan-family.example")).not.toBeInTheDocument(),
    );
    expect(useToasts.getState().toasts.map((t) => t.message)).toContain(
      "alex@morgan-family.example was removed from this computer.",
    );
    await waitFor(() => expect(screen.getByRole("button", { name: "Add account" })).toHaveFocus());
  });

  it("offers to sign in again when a Microsoft account can't sync", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    vi.spyOn(backend, "getSyncStatuses").mockResolvedValue([
      {
        accountId: "acct-work",
        state: "error",
        message: "Your sign-in has expired.",
        lastSyncedAt: null,
      },
    ]);
    const signIn = vi.spyOn(backend, "addMicrosoftAccount");
    openAt("accounts", backend);

    expect(await screen.findByText("Your sign-in has expired.")).toBeInTheDocument();
    expect(screen.getByText("Couldn't sync")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Sign in again" }));

    expect(signIn).toHaveBeenCalledWith("alex.morgan@acme.example");
    await waitFor(() =>
      expect(useToasts.getState().toasts.map((t) => t.message)).toContain(
        "You're signed in again as alex.morgan@acme.example.",
      ),
    );
  });

  it("opens the password and server form to fix an IMAP account that can't sync", async () => {
    const user = userEvent.setup();
    const backend = withHomeSyncError(new MockBackend());
    const sync = vi.spyOn(backend, "syncAccount");
    openAt("accounts", backend);

    await user.click(await screen.findByRole("button", { name: "Fix Alex (Personal)" }));
    expect(screen.getByText("This account can't sync right now")).toBeInTheDocument();
    const password = screen.getByLabelText("New password");
    await waitFor(() => expect(password).toHaveFocus());
    expect(password).toHaveValue("");
    expect(password).toHaveAccessibleDescription("Leave blank to keep the current password.");
    expect(screen.getByLabelText("IMAP server")).toHaveValue("imap.morgan-family.example");
    expect(screen.getByLabelText("SMTP server")).toHaveValue("smtp.morgan-family.example");

    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(sync).toHaveBeenCalledWith("acct-home");
  });

  it("shows a rejected password in the Fix form", async () => {
    const user = userEvent.setup();
    const backend = withHomeSyncError(new MockBackend());
    const { spy: updateImap, release } = holdUpdates(backend);
    openAt("accounts", backend);

    await user.click(await screen.findByRole("button", { name: "Fix Alex (Personal)" }));
    const password = screen.getByLabelText("New password");
    await user.type(password, "wrong");
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(screen.getByRole("button", { name: /Checking your settings…/ })).toBeDisabled();
    expect(password).toBeDisabled();
    release();

    await waitFor(() => expect(password).toHaveAttribute("aria-invalid", "true"));
    expect(password).toHaveAccessibleDescription(
      /The server didn't accept this email address and password/,
    );
    await waitFor(() => expect(password).toHaveFocus());
    expect(updateImap).toHaveBeenCalledWith("acct-home", { password: "wrong" });
    expect(screen.getByText("Couldn't sync")).toBeInTheDocument();
  });

  it("saves new server settings from the Fix form and clears the sync error", async () => {
    const user = userEvent.setup();
    const backend = withHomeSyncError(new MockBackend());
    const updateImap = vi.spyOn(backend, "updateImapAccount");
    openAt("accounts", backend);

    expect(await screen.findByText("Connection timed out.")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Fix Alex (Personal)" }));
    const imapHost = screen.getByLabelText("IMAP server");
    await user.clear(imapHost);
    await user.type(imapHost, "mail.morgan-family.example");
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(updateImap).toHaveBeenCalledTimes(1));
    expect(updateImap).toHaveBeenCalledWith("acct-home", {
      settings: {
        imapHost: "mail.morgan-family.example",
        imapPort: 993,
        imapTls: "tls",
        smtpHost: "smtp.morgan-family.example",
        smtpPort: 465,
        smtpTls: "tls",
        username: "alex@morgan-family.example",
      },
    });

    await waitFor(() => expect(screen.queryByText("Couldn't sync")).not.toBeInTheDocument());
    expect(screen.queryByText("Connection timed out.")).not.toBeInTheDocument();
    expect(screen.queryByText("This account can't sync right now")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("New password")).not.toBeInTheDocument();
    expect(screen.getByText(/mail\.morgan-family\.example:993/)).toBeInTheDocument();
    expect(useToasts.getState().toasts.map((t) => t.message)).toContain(
      "Saved. alex@morgan-family.example is connected.",
    );
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Change password or servers" })).toHaveFocus(),
    );
  });

  it("keeps the current password when the field is left blank", async () => {
    const user = userEvent.setup();
    const { backend } = openAt("accounts");
    const updateImap = vi.spyOn(backend, "updateImapAccount");

    await user.click(await screen.findByRole("button", { name: "Manage Alex (Personal)" }));
    await user.click(screen.getByRole("button", { name: "Change password or servers" }));
    const [, smtpTls] = screen.getAllByLabelText("Security");
    await user.selectOptions(smtpTls!, "startTls");
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(updateImap).toHaveBeenCalledTimes(1));
    const [accountId, update] = updateImap.mock.calls[0]!;
    expect(accountId).toBe("acct-home");
    expect(update).not.toHaveProperty("password");
    expect(update.settings).toMatchObject({ smtpTls: "startTls", smtpPort: 587 });
  });

  it("changes only the password when the servers are untouched", async () => {
    const user = userEvent.setup();
    const { backend } = openAt("accounts");
    const updateImap = vi.spyOn(backend, "updateImapAccount");

    await user.click(await screen.findByRole("button", { name: "Manage Alex (Personal)" }));
    await user.click(screen.getByRole("button", { name: "Change password or servers" }));
    await user.type(screen.getByLabelText("New password"), "new-secret{Enter}");

    await waitFor(() =>
      expect(updateImap).toHaveBeenCalledWith("acct-home", { password: "new-secret" }),
    );
    await waitFor(() => expect(screen.queryByLabelText("New password")).not.toBeInTheDocument());
  });

  it("explains connection problems in the Fix form, and Escape leaves it", async () => {
    const user = userEvent.setup();
    const { backend } = openAt("accounts");
    vi.spyOn(backend, "updateImapAccount").mockRejectedValueOnce({
      code: "network",
      message: "Connection refused (os error 111)",
    });

    await user.click(await screen.findByRole("button", { name: "Manage Alex (Personal)" }));
    await user.click(screen.getByRole("button", { name: "Change password or servers" }));
    await user.type(screen.getByLabelText("New password"), "pw");
    await user.click(screen.getByRole("button", { name: "Save" }));

    const alert = await screen.findByRole("alert");
    expect(within(alert).getByText("Couldn't connect to the mail server")).toBeInTheDocument();
    expect(within(alert).getByText(/Check the server names and ports below/)).toBeInTheDocument();

    await user.click(screen.getByLabelText("IMAP server"));
    await user.keyboard("{Escape}");
    expect(screen.queryByLabelText("New password")).not.toBeInTheDocument();
    expect(useAppStore.getState().settingsOpen).toBe("accounts");
  });

  it("syncs an account on request and opens add account", async () => {
    const user = userEvent.setup();
    const { backend } = openAt("accounts");
    const sync = vi.spyOn(backend, "syncAccount");

    await user.click(await screen.findByRole("button", { name: "Manage Alex Morgan" }));
    await user.click(screen.getByRole("button", { name: "Sync now" }));
    expect(sync).toHaveBeenCalledWith("acct-work");

    await user.click(screen.getByRole("button", { name: "Add account" }));
    expect(useAppStore.getState().addAccountOpen).toBe(true);
    expect(screen.getByRole("dialog", { name: "Add an account" })).toBeInTheDocument();
  });
});

describe("About", () => {
  it("shows the version and the privacy statement", async () => {
    openAt("about");
    expect(await screen.findByText("Version 0.1.0-demo")).toBeInTheDocument();
    expect(screen.getByText(/no telemetry/)).toBeInTheDocument();
    expect(screen.queryByText(/No keyring service was found/)).not.toBeInTheDocument();
  });

  it("warns when sign-ins can't be stored", async () => {
    const backend = new MockBackend();
    vi.spyOn(backend, "getAppInfo").mockResolvedValue({
      version: "1.0.0",
      persistentSecrets: false,
      microsoftClientIdBuiltIn: true,
    });
    openAt("about", backend);
    expect(await screen.findByText(/No keyring service was found/)).toHaveTextContent(
      "Install GNOME Keyring or KWallet.",
    );
  });
});
