// An update of the Flatpak, installed by a software center: the card in the
// corner, the main menu, Settings → About, restarting into it (from the page
// and from the tray) and saying it worked.

import { QueryClient } from "@tanstack/react-query";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, beforeEach, describe, expect, it } from "vitest";

import { DEMO_UPDATE_COMMIT, MockBackend, type MockOptions } from "../api/mock";
import { useModals } from "../components/modals";
import { useToasts } from "../components/toast";
import { resetMailStore, useMailStore } from "../modules/mail/mailStore";
import { loadLazyModules } from "../test/lazyModules";
import { App } from "./App";
import { DISMISSED_UPDATE_KEY, useAppStore } from "./store";

function renderApp(options: MockOptions = { update: "installed" }) {
  const backend = new MockBackend(options);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const view = render(<App backend={backend} queryClient={client} />);
  return { backend, client, ...view };
}

const card = () => screen.findByRole("region", { name: "Update installed" });
const noCard = () => screen.queryByRole("region", { name: "Update installed" });

async function openMenu(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: /^Main menu/ }));
  return screen.getByRole("menu", { name: "Main menu" });
}

function startUnsentMessage() {
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
      signature: "auto",
      attachments: [],
      importance: "normal",
      replyToMessageId: null,
      replyKind: null,
      focus: "body",
    });
    useMailStore.getState().updateCompose({ subject: "Half written, edited" });
  });
}

beforeAll(loadLazyModules, 60_000);

beforeEach(() => {
  resetMailStore();
  useAppStore.setState({
    module: "mail",
    search: "",
    settingsOpen: null,
    addAccountOpen: false,
    composeRequest: null,
    leaveConfirm: null,
    startAtLoginOffer: false,
    remindersOpen: false,
    calendarRequest: null,
    dismissedUpdate: null,
  });
  useModals.setState({ open: 0 });
  useToasts.setState({ toasts: [] });
  window.localStorage.clear();
});

describe("without an update", () => {
  it("shows nothing about updates", async () => {
    const user = userEvent.setup();
    renderApp({});
    await screen.findByRole("navigation", { name: "Apps" });
    expect(noCard()).not.toBeInTheDocument();
    // No dot: the button is just the main menu.
    expect(screen.getByRole("button", { name: "Main menu" })).toBeInTheDocument();
    const menu = await openMenu(user);
    expect(
      within(menu)
        .getAllByRole("menuitem")
        .map((i) => i.textContent),
    ).toEqual(["SettingsCtrl+,", "About Cumulas Mail", "Quit"]);
    await user.click(within(menu).getByRole("menuitem", { name: "About Cumulas Mail" }));
    expect(await screen.findByText("Free software")).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Updates" })).not.toBeInTheDocument();
    expect(screen.queryByText("Update installed")).not.toBeInTheDocument();
  });
});

describe("an installed update", () => {
  it("says so in the corner, and Restart restarts into it", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp();
    const notice = await card();
    // In the live region, there before the card came.
    expect(notice.parentElement).toHaveAttribute("aria-live", "polite");
    expect(notice).toHaveAccessibleDescription("Restart Cumulas Mail to use the new version.");
    // It takes no focus: it mustn't interrupt typing.
    expect(notice).not.toContainElement(document.activeElement as HTMLElement);

    const restart = within(notice).getByRole("button", { name: "Restart" });
    await user.click(restart);
    await waitFor(() => expect(backend.calls.restart).toBe(1));
    // The same button, still focused, while the new version takes over.
    await waitFor(() => expect(restart).toHaveTextContent("Restarting…"));
    expect(restart).toHaveAttribute("aria-disabled", "true");
    expect(restart).toHaveFocus();
    expect(within(notice).queryByRole("button", { name: "Later" })).not.toBeInTheDocument();
    // Nothing typed now would be kept.
    expect(screen.getByRole("main")).toHaveAttribute("inert");
    expect(backend.calls.quit).toBe(0);

    // Clicked again, or asked from the menu or the tray: one restart.
    await user.click(restart);
    const menu = await openMenu(user);
    expect(within(menu).getByRole("menuitem", { name: "Restarting…" })).toBeDisabled();
    act(() => backend.requestRestart());
    await act(() => new Promise((r) => setTimeout(r, 10)));
    expect(backend.calls.restart).toBe(1);
  });

  it("shows a dot on the main menu, whose item above Quit restarts", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp();
    await card();
    const button = screen.getByRole("button", { name: "Main menu, update installed" });
    expect(button.querySelector(".app-header__menu-dot")).not.toBeNull();
    const menu = await openMenu(user);
    const items = within(menu).getAllByRole("menuitem");
    expect(items.map((i) => i.textContent)).toEqual([
      "SettingsCtrl+,",
      "About Cumulas Mail",
      "Restart to update",
      "Quit",
    ]);
    // Settings stays first, where the menu puts the focus.
    expect(items[0]).toHaveFocus();
    await user.click(within(menu).getByRole("menuitem", { name: "Restart to update" }));
    await waitFor(() => expect(backend.calls.restart).toBe(1));
  });

  it("asks before an unsent message is lost, as Quit does", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp();
    await screen.findByRole("navigation", { name: "Apps" });
    startUnsentMessage();
    const notice = await card();

    await user.click(within(notice).getByRole("button", { name: "Restart" }));
    let dialog = await screen.findByRole("dialog", { name: "Restart Cumulas Mail?" });
    expect(dialog).toHaveTextContent("Your unsent message will be discarded.");
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(backend.calls.quitCanceled).toBe(1));
    expect(backend.calls.restart).toBe(0);
    expect(useMailStore.getState().compose?.subject).toBe("Half written, edited");

    await user.click(within(await card()).getByRole("button", { name: "Restart" }));
    dialog = await screen.findByRole("dialog", { name: "Restart Cumulas Mail?" });
    await user.click(within(dialog).getByRole("button", { name: "Discard and restart" }));
    await waitFor(() => expect(backend.calls.restart).toBe(1));
    expect(backend.calls.quit).toBe(0);

    // Quit still asks about quitting.
    act(() => useAppStore.getState().setLeaveConfirm("quit"));
    expect(await screen.findByRole("dialog", { name: "Quit Cumulas Mail?" })).toHaveTextContent(
      "Discard and quit",
    );
  });

  it("doesn't ask again while the restart is under way", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp();
    await screen.findByRole("navigation", { name: "Apps" });
    startUnsentMessage();
    await user.click(within(await card()).getByRole("button", { name: "Restart" }));
    const dialog = await screen.findByRole("dialog", { name: "Restart Cumulas Mail?" });
    await user.click(within(dialog).getByRole("button", { name: "Discard and restart" }));
    await waitFor(() => expect(backend.calls.restart).toBe(1));
    await screen.findByRole("button", { name: "Restarting…" });

    // The tray or a notification asks again: the message was given up, and
    // keeping it now couldn't stop the restart.
    act(() => backend.requestRestart());
    await act(() => new Promise((r) => setTimeout(r, 10)));
    expect(screen.queryByRole("dialog", { name: "Restart Cumulas Mail?" })).not.toBeInTheDocument();
    expect(backend.calls.restart).toBe(1);
    // Nor does Quit from the tray: it quits.
    act(() => backend.requestQuit());
    await waitFor(() => expect(backend.calls.quit).toBe(1));
    expect(screen.queryByRole("dialog", { name: "Quit Cumulas Mail?" })).not.toBeInTheDocument();
  });

  it("restarts from the tray the same way", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp();
    await card();
    startUnsentMessage();

    act(() => backend.requestRestart());
    const dialog = await screen.findByRole("dialog", { name: "Restart Cumulas Mail?" });
    expect(backend.calls.restart).toBe(0);
    await user.click(within(dialog).getByRole("button", { name: "Discard and restart" }));
    await waitFor(() => expect(backend.calls.restart).toBe(1));
    expect(backend.calls.quitCanceled).toBe(0);
  });

  it("asks about restarting when the tray's Restart comes while it asks about quitting", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp();
    await card();
    startUnsentMessage();

    act(() => backend.requestQuit());
    await screen.findByRole("dialog", { name: "Quit Cumulas Mail?" });
    act(() => backend.requestRestart());
    const dialog = await screen.findByRole("dialog", { name: "Restart Cumulas Mail?" });
    expect(backend.calls.quit).toBe(0);
    expect(backend.calls.restart).toBe(0);
    await user.click(within(dialog).getByRole("button", { name: "Discard and restart" }));
    await waitFor(() => expect(backend.calls.restart).toBe(1));
    expect(backend.calls.quit).toBe(0);
  });

  it("goes away with Later until another version is installed", async () => {
    const user = userEvent.setup();
    const { unmount } = renderApp();
    await user.click(within(await card()).getByRole("button", { name: "Later" }));
    expect(noCard()).not.toBeInTheDocument();
    expect(window.localStorage.getItem(DISMISSED_UPDATE_KEY)).toBe(DEMO_UPDATE_COMMIT);
    // The way back stays: the dot and the menu item.
    expect(screen.getByRole("button", { name: "Main menu, update installed" })).toBeInTheDocument();
    const menu = await openMenu(user);
    expect(within(menu).getByRole("menuitem", { name: "Restart to update" })).toBeEnabled();
    await user.keyboard("{Escape}");

    // Not again for this version, as long as the device remembers.
    unmount();
    useAppStore.setState({ dismissedUpdate: window.localStorage.getItem(DISMISSED_UPDATE_KEY) });
    const { backend } = renderApp();
    await screen.findByRole("button", { name: "Main menu, update installed" });
    await act(() => new Promise((r) => setTimeout(r, 10)));
    expect(noCard()).not.toBeInTheDocument();

    // A newer one comes back, and Escape puts it off too.
    act(() =>
      backend.setUpdateState({ kind: "installed", commit: "demo-commit-3", restart: "idle" }),
    );
    expect(await card()).toBeInTheDocument();
  });

  it("puts the card off with Escape", async () => {
    const user = userEvent.setup();
    renderApp();
    const notice = await card();
    within(notice).getByRole("button", { name: "Later" }).focus();
    await user.keyboard("{Escape}");
    expect(noCard()).not.toBeInTheDocument();
    expect(useAppStore.getState().dismissedUpdate).toBe(DEMO_UPDATE_COMMIT);
  });

  it("comes as the portal reports it, and waits for dialogs and toasts", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({});
    await screen.findByRole("navigation", { name: "Apps" });
    act(() => useAppStore.getState().openSettings("general"));
    await screen.findByRole("dialog", { name: "Settings" });
    act(() =>
      backend.setUpdateState({ kind: "installed", commit: DEMO_UPDATE_COMMIT, restart: "idle" }),
    );
    await screen.findByRole("button", { name: "Main menu, update installed" });
    // Settings makes the rest of the page inert, the corner included.
    expect(noCard()).not.toBeInTheDocument();
    await user.click(
      within(screen.getByRole("dialog", { name: "Settings" })).getByRole("button", {
        name: "Close",
      }),
    );
    expect(await card()).toBeInTheDocument();

    // Any other dialog too, such as the one for discarding a message.
    act(() => useAppStore.getState().setLeaveConfirm("quit"));
    await screen.findByRole("dialog", { name: "Quit Cumulas Mail?" });
    expect(noCard()).not.toBeInTheDocument();
    act(() => useToasts.getState().push({ kind: "info", message: "Archived" }));
    act(() => useAppStore.getState().setLeaveConfirm(null));
    // It comes back after the toasts, which would cover its buttons.
    await act(() => new Promise((r) => setTimeout(r, 10)));
    expect(noCard()).not.toBeInTheDocument();
    act(() => useToasts.setState({ toasts: [] }));
    expect(await card()).toBeInTheDocument();
  });

  it("is in Settings → About, which counts as seeing the card", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp();
    await card();
    act(() => useAppStore.getState().openSettings("about"));
    const updates = await screen.findByRole("region", { name: "Updates" });
    expect(updates).toHaveTextContent("Update installed");
    expect(updates).toHaveTextContent("Restart Cumulas Mail to use the new version.");
    act(() => useAppStore.getState().closeSettings());
    await act(() => new Promise((r) => setTimeout(r, 10)));
    expect(noCard()).not.toBeInTheDocument();

    act(() => useAppStore.getState().openSettings("about"));
    const restart = within(await screen.findByRole("region", { name: "Updates" })).getByRole(
      "button",
      { name: "Restart" },
    );
    await user.click(restart);
    await waitFor(() => expect(backend.calls.restart).toBe(1));
    await waitFor(() => expect(restart).toHaveTextContent("Restarting…"));
    expect(restart).toHaveAttribute("aria-disabled", "true");
  });

  it("says when the restart failed, and tries again", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ update: "restartfails" });
    const notice = await card();
    const restart = within(notice).getByRole("button", { name: "Restart" });
    await user.click(restart);
    const failed = "Couldn't restart. Try again, or quit Cumulas Mail and open it again.";
    await waitFor(() =>
      expect(useToasts.getState().toasts.map((t) => t.message)).toContain(failed),
    );
    // The same button, focused, offers Restart again; the card says what happened.
    await waitFor(() => expect(restart).toHaveTextContent(/^Restart$/));
    expect(restart).not.toHaveAttribute("aria-disabled");
    expect(restart).toHaveFocus();
    expect(notice).toHaveAccessibleDescription(failed);
    expect(screen.getByRole("main")).not.toHaveAttribute("inert");
    const menu = await openMenu(user);
    expect(within(menu).getByRole("menuitem", { name: "Restart to update" })).toBeEnabled();
    await user.keyboard("{Escape}");

    await user.click(restart);
    await waitFor(() => expect(backend.calls.restart).toBe(2));
    act(() => useAppStore.getState().openSettings("about"));
    expect(await screen.findByRole("region", { name: "Updates" })).toHaveTextContent(failed);
  });

  it("shares the corner with the start at login offer", async () => {
    renderApp();
    await card();
    act(() => useAppStore.getState().setStartAtLoginOffer(true));
    expect(
      await screen.findByRole("region", { name: "Start Cumulas Mail when you log in?" }),
    ).toBeInTheDocument();
    expect(await card()).toBeInTheDocument();
  });
});

describe("after a restart into an update", () => {
  it("says the app was updated, once", async () => {
    const { unmount } = renderApp({ restartedToUpdate: true });
    await waitFor(() =>
      expect(useToasts.getState().toasts.map((t) => t.message)).toEqual([
        "Updated to version 0.1.0-demo.",
      ]),
    );
    expect(noCard()).not.toBeInTheDocument();
    unmount();
    renderApp({});
    await screen.findByRole("navigation", { name: "Apps" });
    await act(() => new Promise((r) => setTimeout(r, 10)));
    expect(useToasts.getState().toasts).toHaveLength(1);
  });
});
