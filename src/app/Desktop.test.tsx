// The app's side of the desktop integration: following notification clicks
// (`navigate`), quitting, the main menu and the start-at-login offer.

import { QueryClient } from "@tanstack/react-query";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { MockBackend } from "../api/mock";
import { qk } from "../api/queries";
import type { MessageSummary, Settings } from "../api/types";
import { useToasts } from "../components/toast";
import { ALL_INBOXES, resetMailStore, useMailStore } from "../modules/mail/mailStore";
import { useAddAccountRequest } from "../modules/settings/addAccountStore";
import { App } from "./App";
import { OFFERED_KEY } from "./offers";
import { useAppStore } from "./store";

function renderApp(backend = new MockBackend()) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(<App backend={backend} queryClient={client} />);
  return { backend, client };
}

async function firstInboxMessage(backend: MockBackend, folderId = "w-inbox") {
  const page = await backend.listMessages(folderId, { offset: 0, limit: 5 });
  return page.items[0] as MessageSummary;
}

beforeEach(() => {
  resetMailStore();
  useAppStore.setState({
    module: "mail",
    search: "",
    settingsOpen: null,
    addAccountOpen: false,
    composeRequest: null,
    quitConfirmOpen: false,
    startAtLoginOffer: false,
  });
  useAddAccountRequest.setState({ request: { step: "choose" } });
  useToasts.setState({ toasts: [] });
  window.localStorage.clear();
});

describe("following the desktop", () => {
  it("opens the message a notification was clicked for", async () => {
    const backend = new MockBackend();
    renderApp(backend);
    await screen.findByRole("navigation", { name: "Apps" });
    act(() => {
      useAppStore.setState({ module: "calendar", search: "budget" });
      useAppStore.getState().openSettings("general");
    });
    const message = await firstInboxMessage(backend, "h-inbox");

    act(() =>
      backend.navigate({
        kind: "message",
        messageId: message.id,
        folderId: message.folderId,
        accountId: message.accountId,
      }),
    );

    await waitFor(() => expect(useMailStore.getState().activeId).toBe(message.id));
    const app = useAppStore.getState();
    expect(app).toMatchObject({ module: "mail", search: "", settingsOpen: null });
    expect(useMailStore.getState()).toMatchObject({ folderId: "h-inbox", readerOpen: true });
    expect(
      await screen.findByRole("heading", { level: 1, name: message.subject }),
    ).toBeInTheDocument();
  });

  it("stays in All inboxes for an Inbox message, and shows it past a Flagged filter", async () => {
    const backend = new MockBackend();
    renderApp(backend);
    await screen.findByRole("navigation", { name: "Apps" });
    act(() => {
      useMailStore.getState().selectFolder(ALL_INBOXES);
      useMailStore.getState().setFilter("flagged");
    });
    const message = await firstInboxMessage(backend);
    act(() =>
      backend.navigate({
        kind: "message",
        messageId: message.id,
        folderId: message.folderId,
        accountId: message.accountId,
      }),
    );
    await waitFor(() => expect(useMailStore.getState().activeId).toBe(message.id));
    expect(useMailStore.getState()).toMatchObject({ folderId: ALL_INBOXES, filter: "all" });
  });

  it("follows a click that came before the window, once the page starts", async () => {
    const backend = new MockBackend();
    const message = await firstInboxMessage(backend);
    backend.navigate({
      kind: "message",
      messageId: message.id,
      folderId: message.folderId,
      accountId: message.accountId,
    });
    renderApp(backend);
    await waitFor(() => expect(useMailStore.getState().activeId).toBe(message.id));
    expect(useMailStore.getState().folderId).toBe("w-inbox");
  });

  it("opens an account's Inbox, saying why when the message is gone", async () => {
    const backend = new MockBackend();
    renderApp(backend);
    await screen.findByRole("navigation", { name: "Apps" });
    act(() => backend.navigate({ kind: "inbox", accountId: "acct-home", notice: null }));
    await waitFor(() => expect(useMailStore.getState().folderId).toBe("h-inbox"));

    act(() =>
      backend.navigate({
        kind: "inbox",
        accountId: null,
        notice: "That message was moved or deleted.",
      }),
    );
    await waitFor(() => expect(useMailStore.getState().folderId).toBe(ALL_INBOXES));
    expect(useToasts.getState().toasts.map((t) => t.message)).toContain(
      "That message was moved or deleted.",
    );
  });

  it("starts a new message from the tray", async () => {
    const backend = new MockBackend();
    renderApp(backend);
    await screen.findByRole("navigation", { name: "Apps" });
    act(() => useAppStore.getState().setModule("tasks"));
    act(() => backend.navigate({ kind: "compose" }));
    await waitFor(() => expect(useMailStore.getState().compose).not.toBeNull());
    expect(useAppStore.getState().module).toBe("mail");
  });

  it("keeps the settings in step with the background state", async () => {
    const backend = new MockBackend();
    const { client } = renderApp(backend);
    await screen.findByRole("navigation", { name: "Apps" });
    await waitFor(() => expect(client.getQueryData(qk.settings())).toBeDefined());
    await act(async () => {
      await backend.setRunInBackground(false);
    });
    await waitFor(() =>
      expect(client.getQueryData<Settings>(qk.settings())?.runInBackground).toBe(false),
    );
  });
});

describe("quitting", () => {
  it("quits with Ctrl+Q and from the main menu", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    renderApp(backend);
    await screen.findByRole("navigation", { name: "Apps" });

    await user.keyboard("{Control>}q{/Control}");
    await waitFor(() => expect(backend.calls.quit).toBe(1));

    await user.click(screen.getByRole("button", { name: "Main menu" }));
    const menu = screen.getByRole("menu", { name: "Main menu" });
    expect(within(menu).getByRole("menuitem", { name: /Quit/ })).toHaveTextContent("Ctrl+Q");
    await user.click(within(menu).getByRole("menuitem", { name: /Quit/ }));
    await waitFor(() => expect(backend.calls.quit).toBe(2));
  });

  it("asks before an unsent message is lost", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    renderApp(backend);
    await screen.findByRole("navigation", { name: "Apps" });
    act(() => {
      useMailStore.getState().startCompose({
        accountId: "acct-work",
        to: [],
        cc: [],
        bcc: [],
        showCc: false,
        showBcc: false,
        subject: "Half written",
        bodyHtml: "",
        quoteHtml: null,
        attachments: [],
        importance: "normal",
        replyToMessageId: null,
        replyKind: null,
        focus: "body",
      });
      useMailStore.getState().updateCompose({ subject: "Half written, edited" });
    });

    await user.keyboard("{Control>}q{/Control}");
    const dialog = await screen.findByRole("dialog", { name: "Quit Cumulas Mail?" });
    expect(backend.calls.quit).toBe(0);
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(backend.calls.quit).toBe(0);

    await user.keyboard("{Control>}q{/Control}");
    await user.click(
      within(await screen.findByRole("dialog", { name: "Quit Cumulas Mail?" })).getByRole(
        "button",
        { name: "Discard and quit" },
      ),
    );
    await waitFor(() => expect(backend.calls.quit).toBe(1));
  });

  it("opens Settings from the main menu and with Ctrl+,", async () => {
    const user = userEvent.setup();
    renderApp();
    await screen.findByRole("navigation", { name: "Apps" });
    await user.click(screen.getByRole("button", { name: "Main menu" }));
    await user.click(screen.getByRole("menuitem", { name: /Settings/ }));
    expect(useAppStore.getState().settingsOpen).toBe("accounts");
    act(() => useAppStore.getState().closeSettings());
    await user.keyboard("{Control>},{/Control}");
    expect(useAppStore.getState().settingsOpen).toBe("accounts");
  });
});

describe("the start at login offer", () => {
  async function addFirstAccount(user: ReturnType<typeof userEvent.setup>) {
    await user.click(await screen.findByRole("button", { name: "Microsoft 365 or Outlook.com" }));
    await user.click(await screen.findByRole("button", { name: "Sign in with Microsoft" }));
    await waitFor(() => expect(useAppStore.getState().addAccountOpen).toBe(false));
  }

  it("comes once, after the first account, and turns it on", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ empty: true });
    const setLogin = vi.spyOn(backend, "setStartAtLogin");
    renderApp(backend);
    await addFirstAccount(user);

    const offer = await screen.findByRole("region", {
      name: "Start Cumulas Mail when you log in?",
    });
    expect(offer).toHaveAccessibleDescription(/without a window/);
    expect(window.localStorage.getItem(OFFERED_KEY)).toBe("offered");
    await user.click(within(offer).getByRole("button", { name: "Start at login" }));
    await waitFor(() => expect(setLogin).toHaveBeenCalledWith(true, true));
    await waitFor(() => expect(offer).not.toBeInTheDocument());
    expect(useToasts.getState().toasts.map((t) => t.message)).toContain(
      "Cumulas Mail will start when you log in.",
    );
  });

  it("goes away when declined, and isn't offered again", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ empty: true });
    const setLogin = vi.spyOn(backend, "setStartAtLogin");
    renderApp(backend);
    await addFirstAccount(user);
    const offer = await screen.findByRole("region", { name: /log in/ });
    await user.click(within(offer).getByRole("button", { name: "Not now" }));
    expect(offer).not.toBeInTheDocument();
    expect(setLogin).not.toHaveBeenCalled();

    // A second first account (after removing the first) isn't asked about again.
    act(() => useAppStore.getState().setStartAtLoginOffer(false));
    window.localStorage.setItem(OFFERED_KEY, "offered");
    const { offerStartAtLoginOnce } = await import("./offers");
    offerStartAtLoginOnce();
    expect(useAppStore.getState().startAtLoginOffer).toBe(false);
  });

  it("isn't offered when starting at login is already on or can't be", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ empty: true, background: { autostart: "unsupported" } });
    renderApp(backend);
    await addFirstAccount(user);
    expect(useAppStore.getState().startAtLoginOffer).toBe(true);
    expect(screen.queryByRole("region", { name: /log in/ })).not.toBeInTheDocument();
  });
});
