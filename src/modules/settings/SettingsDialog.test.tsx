import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { MockBackend } from "../../api/mock";
import { qk } from "../../api/queries";
import { SANS_STACK, SERIF_STACK } from "../../lib/fonts";
import type { ImapAccountUpdate, Settings } from "../../api/types";
import { type SettingsSection, useAppStore } from "../../app/store";
import { useToasts } from "../../components/toast";
import { renderWithBackend } from "../../test/render";
import { editFrame, loadFrame } from "../mail/testUtils";
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

    const toggle = await screen.findByRole("switch", { name: "New message notifications" });
    expect(toggle).toBeChecked();
    await user.click(toggle);
    await waitFor(() => expect(lastSaved()).toMatchObject({ notificationsEnabled: false }));

    await user.selectOptions(screen.getByLabelText("Also check everything else"), "15");
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

describe("Appearance", () => {
  const preview = () => screen.getByRole("figure", { name: "Preview" });
  /** A font picker's status, next to its input. */
  const statusOf = (input: HTMLElement) => within(input.parentElement!).getByRole("status");

  it("chooses an interface font by typing to filter the list", async () => {
    const user = userEvent.setup();
    const { lastSaved } = openAt("general");

    const input = await screen.findByRole("combobox", { name: "Interface font" });
    expect(input).toHaveValue("Inter (default)");
    expect(input).toHaveAttribute("aria-expanded", "false");
    await user.click(input);

    const list = screen.getByRole("listbox", { name: "Interface font" });
    expect(input).toHaveAttribute("aria-expanded", "true");
    expect(within(list).getByRole("option", { name: "Inter (default)" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(within(list).getByRole("option", { name: "System font" })).toBeInTheDocument();
    expect(await within(list).findByRole("option", { name: "Fira Sans" })).toBeInTheDocument();
    expect(within(list).getByRole("group", { name: "Installed fonts" })).toBeInTheDocument();
    const inter = input.getAttribute("aria-activedescendant");

    // Typing replaces the current name and filters, accents and case aside.
    await user.keyboard("FIRA");
    expect(input).toHaveValue("FIRA");
    expect(
      within(list)
        .getAllByRole("option")
        .map((o) => o.textContent),
    ).toEqual(["Fira Code", "Fira Sans"]);
    // The first match is highlighted, and screen readers are told it is.
    const code = within(list).getByRole("option", { name: "Fira Code" });
    expect(code).toHaveAttribute("aria-selected", "true");
    expect(input).toHaveAttribute("aria-activedescendant", code.id);
    expect(code.id).not.toBe(inter);
    expect(statusOf(input)).toHaveTextContent("2 fonts match.");
    await user.keyboard("{ArrowDown}");
    const fira = within(list).getByRole("option", { name: "Fira Sans" });
    expect(fira).toHaveAttribute("aria-selected", "true");
    expect(input).toHaveAttribute("aria-activedescendant", fira.id);
    // Each family is shown in its own face.
    expect(fira.style.fontFamily).toContain("Fira Sans");

    await user.keyboard("{Enter}");
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    expect(input).toHaveValue("Fira Sans");
    await waitFor(() =>
      expect(lastSaved()).toMatchObject({ interfaceFont: { kind: "family", family: "Fira Sans" } }),
    );
  });

  it("matches every word of the search, and says when nothing does", async () => {
    const user = userEvent.setup();
    openAt("general");
    const input = await screen.findByRole("combobox", { name: "Reading font" });
    // There before it has anything to say, so what it says is announced.
    const status = statusOf(input);
    expect(status).toBeEmptyDOMElement();
    await user.click(input);
    const list = screen.getByRole("listbox", { name: "Reading font" });
    await within(list).findByRole("option", { name: "Noto Serif" });

    await user.keyboard("noto ara");
    expect(
      within(list)
        .getAllByRole("option")
        .map((o) => o.textContent),
    ).toEqual(["Noto Naskh Arabic", "Noto Sans Arabic"]);
    expect(status).toHaveTextContent("2 fonts match.");
    await user.keyboard("bic xyz");
    expect(within(list).queryAllByRole("option")).toHaveLength(0);
    expect(input).not.toHaveAttribute("aria-activedescendant");
    expect(status).toHaveTextContent("No font matches “noto arabic xyz”.");
    expect(statusOf(input)).toBe(status);
  });

  it("picks with the pointer; Escape closes the list but not the dialog", async () => {
    const user = userEvent.setup();
    const { lastSaved, updateSettings } = openAt("general");

    const input = await screen.findByRole("combobox", { name: "Reading font" });
    expect(input).toHaveValue("Charis SIL (default)");
    await user.click(input);
    await user.click(screen.getByRole("option", { name: "Same as interface font" }));
    expect(input).toHaveValue("Same as interface font");
    await waitFor(() => expect(lastSaved()).toMatchObject({ readingFont: { kind: "interface" } }));

    const saves = updateSettings.mock.calls.length;
    await user.click(input);
    expect(screen.getByRole("listbox", { name: "Reading font" })).toBeInTheDocument();
    await user.keyboard("lit{Escape}");
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    expect(input).toHaveValue("Same as interface font");
    expect(settingsDialog()).toBeInTheDocument();
    expect(updateSettings).toHaveBeenCalledTimes(saves);
  });

  it("previews the highlighted font, and forgets it when the list closes", async () => {
    const user = userEvent.setup();
    openAt("general");
    expect(
      (await screen.findByRole("figure", { name: "Preview" })).style.getPropertyValue(
        "--preview-serif",
      ),
    ).toBe(SERIF_STACK);

    await user.click(screen.getByRole("combobox", { name: "Reading font" }));
    const literata = await screen.findByRole("option", { name: "Literata" });
    // Moving onto an option highlights it; the list scrolling under a still
    // pointer (a mousemove without movement, in WebKit) doesn't.
    fireEvent.mouseMove(literata, { movementX: 0, movementY: 0 });
    expect(literata).toHaveAttribute("aria-selected", "false");
    fireEvent.mouseMove(literata, { movementX: 2, movementY: 1 });
    expect(literata).toHaveAttribute("aria-selected", "true");
    expect(preview().style.getPropertyValue("--preview-serif")).toBe(`"Literata", ${SERIF_STACK}`);
    expect(preview()).toHaveTextContent("Atlas launch checklist");

    await user.keyboard("{Escape}");
    expect(preview().style.getPropertyValue("--preview-serif")).toBe(SERIF_STACK);
  });

  it("saves the text size, and resets the fonts and size to the defaults", async () => {
    const user = userEvent.setup();
    const { lastSaved } = openAt("general");

    const reset = await screen.findByRole("button", { name: "Reset to defaults" });
    expect(reset).toBeDisabled();
    expect(screen.getByRole("radiogroup", { name: "Text size" })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Default" })).toBeChecked();
    await user.click(screen.getByRole("radio", { name: "Largest" }));
    await waitFor(() => expect(lastSaved()).toMatchObject({ textSize: "largest" }));
    expect(screen.getByRole("radio", { name: "Largest" })).toBeChecked();

    await user.click(screen.getByRole("radio", { name: "Dark" }));
    expect(reset).toBeEnabled();
    reset.focus();
    await user.keyboard("{Enter}");
    await waitFor(() =>
      expect(lastSaved()).toMatchObject({
        interfaceFont: { kind: "default" },
        readingFont: { kind: "default" },
        textSize: "default",
        // The theme isn't part of it.
        theme: "dark",
      }),
    );
    expect(screen.getByRole("radio", { name: "Default" })).toBeChecked();
    expect(reset).toBeDisabled();
    // Focus moves on to the next control rather than dropping to the page.
    expect(screen.getByRole("combobox", { name: "Interface font" })).toHaveFocus();
  });

  it("says when a saved font isn't available, and still shows it as chosen", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    await backend.updateSettings({
      ...(await backend.getSettings()),
      interfaceFont: { kind: "family", family: "Gone Sans" },
    });
    openAt("general", backend);

    const input = await screen.findByRole("combobox", { name: "Interface font" });
    expect(input).toHaveValue("Gone Sans");
    expect(input).toHaveAccessibleDescription(
      "Menus, lists, buttons and labels. Gone Sans isn't available on this computer, so Inter is used.",
    );
    await user.click(input);
    expect(await screen.findByRole("option", { name: "Gone Sans" })).toHaveAttribute(
      "data-current",
    );
    // It isn't listed as installed.
    const installed = screen.getByRole("group", { name: "Installed fonts" });
    expect(within(installed).queryByRole("option", { name: "Gone Sans" })).not.toBeInTheDocument();
    expect(within(installed).getAllByRole("option")[0]).toHaveTextContent("Adwaita Mono");
  });

  it("keeps the saved font when its list opens before the installed fonts arrive", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    await backend.updateSettings({
      ...(await backend.getSettings()),
      interfaceFont: { kind: "family", family: "Fira Sans" },
    });
    const listFonts = backend.listFonts.bind(backend);
    let release!: () => void;
    const listed = new Promise<void>((resolve) => (release = resolve));
    vi.spyOn(backend, "listFonts").mockImplementation(async () => {
      await listed;
      return listFonts();
    });
    const { updateSettings } = openAt("general", backend);
    const saved = `"Fira Sans", ${SANS_STACK}`;

    const input = await screen.findByRole("combobox", { name: "Interface font" });
    expect(input).toHaveValue("Fira Sans");
    input.focus();
    await user.keyboard("{ArrowDown}");
    expect(statusOf(input)).toHaveTextContent("Looking for installed fonts…");
    // Nothing else is highlighted or previewed in its place, and Enter
    // doesn't choose anything.
    expect(screen.getByRole("option", { name: "Inter (default)" })).toHaveAttribute(
      "aria-selected",
      "false",
    );
    expect(input).not.toHaveAttribute("aria-activedescendant");
    expect(preview().style.getPropertyValue("--preview-sans")).toBe(saved);
    await user.keyboard("{Enter}");

    // Once they arrive, the saved font is highlighted.
    act(() => release());
    const fira = await screen.findByRole("option", { name: "Fira Sans" });
    expect(fira).toHaveAttribute("aria-selected", "true");
    expect(input).toHaveAttribute("aria-activedescendant", fira.id);
    expect(preview().style.getPropertyValue("--preview-sans")).toBe(saved);
    await user.keyboard("{Enter}");
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    expect(input).toHaveValue("Fira Sans");
    expect(updateSettings).not.toHaveBeenCalled();
  });

  it("still offers the built-in fonts when the installed ones can't be listed", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    vi.spyOn(backend, "listFonts").mockRejectedValue({ code: "internal", message: "No fonts." });
    const { lastSaved } = openAt("general", backend);

    const input = await screen.findByRole("combobox", { name: "Interface font" });
    await user.click(input);
    await waitFor(() =>
      expect(statusOf(input)).toHaveTextContent("Couldn't list the installed fonts."),
    );
    await user.click(screen.getByRole("option", { name: "System font" }));
    await waitFor(() => expect(lastSaved()).toMatchObject({ interfaceFont: { kind: "system" } }));
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

  it("names an unlisted mark-as-read delay with the right plural", async () => {
    const backend = new MockBackend();
    await backend.updateSettings({ ...(await backend.getSettings()), markReadAfterSeconds: 1 });
    openAt("mail", backend);

    const markRead = await screen.findByLabelText("Mark messages as read");
    expect(markRead).toHaveValue("1");
    expect(within(markRead).getByRole("option", { name: "After 1 second" })).toBeInTheDocument();
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

  it("states the license and links to it from About", async () => {
    const backend = new MockBackend();
    const openLink = vi.spyOn(backend, "openLink").mockResolvedValue();
    openAt("about", backend);
    expect(await screen.findByText("Free software")).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: "Read the license" }));
    expect(openLink).toHaveBeenCalledWith("https://www.gnu.org/licenses/gpl-3.0.html");
  });

  it("asks for an ID when the build has none", async () => {
    const backend = new MockBackend();
    vi.spyOn(backend, "getAppInfo").mockResolvedValue({
      version: "1.0.0",
      persistentSecrets: true,
      microsoftClientIdBuiltIn: false,
      syncServer: "sync.cumulas.co",
      signatureServer: "signature.cumulas.co",
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

  it("changes an account's color", async () => {
    const user = userEvent.setup();
    const { backend } = openAt("accounts");
    const update = vi.spyOn(backend, "updateAccount");

    await user.click(await screen.findByRole("button", { name: "Manage Alex (Personal)" }));
    const colors = screen.getByRole("radiogroup", { name: "Color" });
    expect(within(colors).getAllByRole("radio")).toHaveLength(8);
    expect(within(colors).getByRole("radio", { name: "Apricot" })).toBeChecked();

    await user.click(within(colors).getByRole("radio", { name: "Moss" }));
    expect(update).toHaveBeenCalledWith("acct-home", undefined, "#4d7a2e");
    expect(within(colors).getByRole("radio", { name: "Moss" })).toBeChecked();
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
    // Cancel comes first and the confirming button last, as in every confirmation.
    const cancelButton = within(confirm).getByRole("button", { name: "Cancel" });
    const removeButton = within(confirm).getByRole("button", { name: "Remove account" });
    expect(cancelButton.compareDocumentPosition(removeButton)).toBe(
      Node.DOCUMENT_POSITION_FOLLOWING,
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
    // Its editor is open, so the header's Fix button would do nothing.
    expect(screen.queryByRole("button", { name: "Fix Alex (Personal)" })).not.toBeInTheDocument();
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
    await user.click(screen.getByRole("button", { name: "Save changes" }));

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
    await user.click(screen.getByRole("button", { name: "Save changes" }));

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
    await user.click(screen.getByRole("button", { name: "Save changes" }));

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
    await user.click(screen.getByRole("button", { name: "Save changes" }));

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
      syncServer: "sync.cumulas.co",
      signatureServer: null,
    });
    openAt("about", backend);
    expect(await screen.findByText(/No keyring service was found/)).toHaveTextContent(
      "Install GNOME Keyring or KWallet.",
    );
  });
});

describe("Signatures", () => {
  function group(name: string) {
    return screen.getByRole("region", { name: new RegExp(`^${name}`) });
  }

  it("sits between Mail and Microsoft app registration", async () => {
    const user = userEvent.setup();
    openAt("mail");
    await waitFor(() => expect(screen.getByRole("tab", { name: "Mail" })).toHaveFocus());
    await user.keyboard("{ArrowDown}");
    expect(useAppStore.getState().settingsOpen).toBe("signatures");
    await user.keyboard("{ArrowDown}");
    expect(useAppStore.getState().settingsOpen).toBe("microsoft");
  });

  it("shows the organization's signature, and the person's own for the others", async () => {
    openAt("signatures");
    const work = await screen.findByRole("region", {
      name: "Alex Morgan alex.morgan@acme.example",
    });
    expect(group("Alex Morgan")).toBe(work);
    const preview = await within(work).findByTitle("Your organization's signature");
    expect(preview.getAttribute("srcdoc")).toContain("Engineering Manager, Atlas");
    expect(work).toHaveTextContent(
      "Set by your organization with Cumulas Signatures. Added to new messages and replies. " +
        "You can change it in any message before you send.",
    );
    // The organization's replaces the own one: nothing to edit here.
    expect(within(work).queryByTitle("Signature")).not.toBeInTheDocument();
    expect(within(work).queryByText(/Couldn't check for updates/)).not.toBeInTheDocument();

    const home = group("Alex \\(Personal\\)");
    expect(await within(home).findByTitle("Signature")).toBeInTheDocument();
    expect(
      within(home).getByRole("checkbox", { name: "Add it to replies and forwards" }),
    ).toBeChecked();
  });

  it("saves the own signature as you type, and the replies choice at once", async () => {
    const user = userEvent.setup();
    const { backend } = openAt("signatures");
    const update = vi.spyOn(backend, "updateOwnSignature");
    const home = await screen.findByRole("region", { name: /^Alex \(Personal\)/ });
    const doc = await loadFrame((await within(home).findByTitle("Signature")) as HTMLIFrameElement);
    expect(doc.body.innerHTML).toBe("<div>Alex Morgan</div><div>+1 (206) 555-0199</div>");
    expect(doc.defaultView!.document.body.getAttribute("data-placeholder")).toBe("Add a signature");

    editFrame(doc, "<div>Alex</div>");
    editFrame(doc, "<div>Alex M.</div>");
    // After a pause, once.
    await waitFor(() => expect(update).toHaveBeenCalledTimes(1));
    expect(update).toHaveBeenLastCalledWith("acct-home", "<div>Alex M.</div>", true);
    expect(await screen.findByText("Saved")).toBeInTheDocument();

    await user.click(
      within(home).getByRole("checkbox", { name: "Add it to replies and forwards" }),
    );
    expect(update).toHaveBeenLastCalledWith("acct-home", "<div>Alex M.</div>", false);
    expect((await backend.getAccountSignature("acct-home")).ownOnReplies).toBe(false);

    // Leaving the editor saves without waiting.
    editFrame(doc, "<div>Alex Morgan</div>");
    act(() => void doc.body.dispatchEvent(new doc.defaultView!.FocusEvent("blur")));
    await waitFor(() =>
      expect(update).toHaveBeenLastCalledWith("acct-home", "<div>Alex Morgan</div>", false),
    );
    expect(update).toHaveBeenCalledTimes(3);
  });

  it("saves an edit still waiting when Settings closes", async () => {
    const user = userEvent.setup();
    const { backend } = openAt("signatures");
    const update = vi.spyOn(backend, "updateOwnSignature");
    const home = await screen.findByRole("region", { name: /^Alex \(Personal\)/ });
    const doc = await loadFrame((await within(home).findByTitle("Signature")) as HTMLIFrameElement);
    editFrame(doc, "<div>Bye</div>");
    await user.click(within(settingsDialog()).getByRole("button", { name: "Close" }));
    expect(update).toHaveBeenCalledWith("acct-home", "<div>Bye</div>", true);
  });

  it("explains an organization that hasn't allowed Cumulas Mail, and offers the own signature", async () => {
    openAt("signatures", new MockBackend({ signature: "notAllowed" }));
    const work = await screen.findByRole("region", { name: /^Alex Morgan/ });
    expect(
      await within(work).findByText(
        "Your organization uses Cumulas Signatures but hasn't allowed Cumulas Mail yet. Ask your IT admin to allow it, and your signature will appear here.",
      ),
    ).toBeInTheDocument();
    expect(within(work).getByTitle("Signature")).toBeInTheDocument();
  });

  it("says when the organization signs on send", async () => {
    openAt("signatures", new MockBackend({ signature: "addedOnSend" }));
    const work = await screen.findByRole("region", { name: /^Alex Morgan/ });
    expect(
      await within(work).findByText(
        "Your organization adds your signature when you send, so Cumulas Mail doesn't add one.",
      ),
    ).toBeInTheDocument();
    expect(within(work).queryByTitle("Signature")).not.toBeInTheDocument();
  });

  it("shows the last version when the organization couldn't be asked", async () => {
    openAt("signatures", new MockBackend({ signature: "stale" }));
    const work = await screen.findByRole("region", { name: /^Alex Morgan/ });
    expect(
      await within(work).findByText("Couldn't check for updates; showing the last version."),
    ).toBeInTheDocument();
    expect(within(work).getByTitle("Your organization's signature")).toBeInTheDocument();
  });

  it("asks organizations again when their answer is old, quietly", async () => {
    const backend = new MockBackend();
    const sam = await backend.addMicrosoftAccount("sam@contoso.example");
    const original = backend.checkOrganizationSignature.bind(backend);
    let answer!: () => void;
    const asked = new Promise<void>((resolve) => (answer = resolve));
    const check = vi
      .spyOn(backend, "checkOrganizationSignature")
      .mockImplementation(async (accountId) => {
        await asked;
        return original(accountId);
      });
    openAt("signatures", backend);
    const contoso = await screen.findByRole("region", { name: /^sam/ });
    // Not asked yet: it says so while it asks.
    expect(
      await within(contoso).findByText("Checking for your organization's signature…"),
    ).toBeInTheDocument();
    await waitFor(() => expect(check).toHaveBeenCalledWith(sam.id));
    answer();
    await waitFor(() =>
      expect(
        within(contoso).queryByText("Checking for your organization's signature…"),
      ).not.toBeInTheDocument(),
    );
    expect(check).toHaveBeenCalledWith(sam.id);
    // Contoso doesn't use Cumulas Signatures: the own signature applies.
    expect(within(contoso).getByTitle("Signature")).toBeInTheDocument();
    // Acme answered five minutes ago; IMAP accounts are never asked.
    expect(check).not.toHaveBeenCalledWith("acct-work");
    expect(check).not.toHaveBeenCalledWith("acct-home");
  });
});
