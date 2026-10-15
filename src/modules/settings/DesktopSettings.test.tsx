// Settings for notifications, running in the background and sync, and the
// per-account push status (the desktop integration).

import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { MockBackend, type MockOptions } from "../../api/mock";
import { qk } from "../../api/queries";
import type { PushStatus, Settings } from "../../api/types";
import { useAppStore, type SettingsSection } from "../../app/store";
import { useToasts } from "../../components/toast";
import { renderWithBackend } from "../../test/render";
import { useAddAccountRequest } from "./addAccountStore";
import { PRIVACY_URL } from "./constants";
import { describePush, everyLabel } from "./pushStatus";
import { resetSaveStatus } from "./saveStatus";
import { SettingsDialog } from "./SettingsDialog";

function openAt(section: SettingsSection, backend = new MockBackend()) {
  const updateSettings = vi.spyOn(backend, "updateSettings");
  const utils = renderWithBackend(<SettingsDialog />, { backend });
  act(() => useAppStore.getState().openSettings(section));
  const lastSaved = () => updateSettings.mock.lastCall?.[0];
  return { ...utils, backend, updateSettings, lastSaved };
}

function group(name: string) {
  return screen.getByRole("region", { name });
}

beforeEach(() => {
  useAppStore.setState({ addAccountOpen: false, settingsOpen: null });
  useToasts.setState({ toasts: [] });
  resetSaveStatus();
});

describe("Notifications", () => {
  it("saves the preview and sound choices, which need notifications on", async () => {
    const user = userEvent.setup();
    const { lastSaved } = openAt("general");
    const preview = await screen.findByRole("switch", { name: "Show sender and subject" });
    const sound = screen.getByRole("switch", { name: "Play a sound" });
    expect(preview).toBeChecked();
    expect(sound).toBeChecked();

    await user.click(preview);
    await waitFor(() => expect(lastSaved()).toMatchObject({ notifyShowPreview: false }));
    await user.click(sound);
    await waitFor(() => expect(lastSaved()).toMatchObject({ notifySound: false }));

    await user.click(screen.getByRole("switch", { name: "New message notifications" }));
    expect(preview).toBeDisabled();
    expect(sound).toBeDisabled();
  });

  it("sends a test notification, and mentions Do Not Disturb", async () => {
    const user = userEvent.setup();
    const { backend } = openAt("general");
    const send = await screen.findByRole("button", { name: "Send a test notification" });
    expect(send).toHaveAccessibleDescription(/Do Not Disturb/);
    await user.click(send);
    await waitFor(() => expect(backend.calls.testNotifications).toBe(1));
  });

  it("says when no notification service is running", async () => {
    openAt("general", new MockBackend({ background: { notifications: null } }));
    expect(await screen.findByText(/No notification service is running/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Send a test notification" })).toBeDisabled();
  });

  it("shows a pause from the tray and resumes it", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const settings = await backend.getSettings();
    const inAnHour = new Date(Date.now() + 60 * 60 * 1000).toISOString();
    await backend.updateSettings({ ...settings, notificationsPausedUntil: inAnHour });
    const { lastSaved } = openAt("general", backend);

    expect(await screen.findByText("Notifications are paused")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Resume" }));
    await waitFor(() => expect(lastSaved()).toMatchObject({ notificationsPausedUntil: null }));
    expect(screen.queryByText("Notifications are paused")).not.toBeInTheDocument();
  });
});

describe("Background", () => {
  it("keeps running when closed through its own call, not a settings save", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const setRun = vi.spyOn(backend, "setRunInBackground");
    const { updateSettings, client } = openAt("general", backend);

    const keep = await screen.findByRole("switch", { name: "Keep running when closed" });
    expect(keep).toBeChecked();
    expect(keep).toHaveAccessibleDescription(/Quit with Ctrl\+Q/);
    await user.click(keep);
    await waitFor(() => expect(setRun).toHaveBeenCalledWith(false));
    await waitFor(() => expect(keep).not.toBeChecked());
    expect(client.getQueryData<Settings>(qk.settings())?.runInBackground).toBe(false);
    expect(updateSettings).not.toHaveBeenCalled();
  });

  it("starts at login, with or without a window", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const setLogin = vi.spyOn(backend, "setStartAtLogin");
    openAt("general", backend);

    const login = await screen.findByRole("switch", { name: "Start at login" });
    expect(login).not.toBeChecked();
    expect(screen.queryByRole("switch", { name: "Without a window" })).not.toBeInTheDocument();
    await user.click(login);
    await waitFor(() => expect(setLogin).toHaveBeenLastCalledWith(true, true));

    const hidden = await screen.findByRole("switch", { name: "Without a window" });
    expect(hidden).toBeChecked();
    await user.click(hidden);
    await waitFor(() => expect(setLogin).toHaveBeenLastCalledWith(true, false));
    await waitFor(() => expect(hidden).not.toBeChecked());
  });

  it("explains a refused background permission and a desktop without tray icons", async () => {
    openAt(
      "general",
      new MockBackend({
        background: { autostart: "portal", backgroundAllowed: false, trayAvailable: false },
      }),
    );
    const keep = await screen.findByRole("switch", { name: "Keep running when closed" });
    expect(keep).toHaveAccessibleDescription(/doesn't allow Cumulas Mail to run in the background/);
    const tray = screen.getByRole("switch", { name: "Show a tray icon" });
    expect(tray).toBeDisabled();
    expect(tray).not.toBeChecked();
    expect(tray).toHaveAccessibleDescription(/AppIndicator extension/);
  });

  it("gives the Background portal's answer", async () => {
    openAt("general", new MockBackend({ background: { autostart: "portal" } }));
    expect(
      await screen.findByRole("switch", { name: "Keep running when closed" }),
    ).toHaveAccessibleDescription(/Your system allows it\./);
  });

  it("reports a start at login the system refused", async () => {
    const user = userEvent.setup();
    openAt(
      "general",
      new MockBackend({ background: { autostart: "portal", backgroundAllowed: false } }),
    );
    await user.click(await screen.findByRole("switch", { name: "Start at login" }));
    await waitFor(() =>
      expect(
        useToasts
          .getState()
          .toasts.map((t) => t.message)
          .join(),
      ).toMatch(/didn't allow Cumulas Mail to start at login/),
    );
    expect(screen.getByRole("switch", { name: "Start at login" })).not.toBeChecked();
  });

  it("hides and shows the tray icon through the settings", async () => {
    const user = userEvent.setup();
    const { lastSaved } = openAt("general");
    const tray = await screen.findByRole("switch", { name: "Show a tray icon" });
    expect(tray).toBeChecked();
    await user.click(tray);
    await waitFor(() => expect(lastSaved()).toMatchObject({ showTrayIcon: false }));
  });
});

describe("Sync", () => {
  it("explains the push service and links to the privacy policy", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const openLink = vi.spyOn(backend, "openLink").mockResolvedValue();
    const { lastSaved } = openAt("general", backend);

    const relay = await screen.findByRole("switch", { name: "Microsoft push service" });
    expect(relay).toHaveAccessibleDescription(/sync\.cumulas\.co.*never what's in it/);
    await user.click(within(group("Sync")).getByRole("button", { name: "Privacy policy" }));
    expect(openLink).toHaveBeenCalledWith(PRIVACY_URL);

    await user.click(relay);
    await waitFor(() => expect(lastSaved()).toMatchObject({ usePushRelay: false }));
  });

  it("relabels the interval once instant updates are off", async () => {
    const user = userEvent.setup();
    const { lastSaved } = openAt("general");
    expect(await screen.findByLabelText("Also check everything else")).toHaveValue("5");
    await user.click(screen.getByRole("switch", { name: "Instant updates" }));
    await waitFor(() => expect(lastSaved()).toMatchObject({ instantPush: false }));
    expect(screen.getByLabelText("Check for new mail")).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Microsoft push service" })).toBeDisabled();
  });
});

describe("Push status per account", () => {
  function item(name: string) {
    return screen.getAllByRole("listitem").find((li) => within(li).queryByText(name))!;
  }

  async function manage(name: string) {
    await userEvent.setup().click(await screen.findByRole("button", { name: `Manage ${name}` }));
  }

  it("says each account gets mail instantly", async () => {
    const user = userEvent.setup();
    openAt("accounts");
    await manage("Alex Morgan");
    expect(await within(item("Alex Morgan")).findByText("Instant (Microsoft push)")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Manage Alex (Personal)" }));
    expect(await within(item("Alex (Personal)")).findByText("Instant (IMAP IDLE)")).toBeVisible();
  });

  it("offers to sign in again when consent is missing", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ push: "consent" });
    openAt("accounts", backend);
    await manage("Alex Morgan");
    const work = await screen.findByText(/instant updates need permission/);
    expect(work).toHaveTextContent("Every 30 seconds — instant updates need permission");
    expect(screen.getByText("Sign in again to turn on instant updates.")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Sign in again" }));
    expect(useAppStore.getState().addAccountOpen).toBe(true);
    expect(useAddAccountRequest.getState().request).toEqual({
      step: "microsoft",
      email: "alex.morgan@acme.example",
      autoStart: true,
    });
  });

  it("gives the approval link when the organization refused", async () => {
    openAt("accounts", new MockBackend({ push: "refused" }));
    await manage("Alex Morgan");
    expect(
      await screen.findByText(/requires an administrator to approve instant updates/),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Sign in again" })).not.toBeInTheDocument();
    const link = screen.getByLabelText("Approval link for your IT administrator");
    expect((link as HTMLInputElement).value).toContain("/acme.example/v2.0/adminconsent");
    expect(screen.getByRole("button", { name: "Email IT" })).toBeInTheDocument();
  });

  it("follows status changes as they arrive", async () => {
    const backend = new MockBackend();
    const { client } = openAt("accounts", backend);
    await manage("Alex Morgan");
    await screen.findByText("Instant (Microsoft push)");
    act(() => {
      // As useBackendEvents would apply a push-status event.
      client.setQueryData<PushStatus[]>(qk.pushStatuses(), (old) =>
        old?.map((s) =>
          s.accountId === "acct-work"
            ? {
                ...s,
                mode: { kind: "polling", everySecs: 30 },
                health: { kind: "degraded", reason: "Could not reach the push service." },
              }
            : s,
        ),
      );
    });
    expect(
      await screen.findByText("Every 30 seconds — push service unreachable"),
    ).toBeInTheDocument();
    expect(screen.getByText("Could not reach the push service.")).toBeInTheDocument();
  });
});

describe("describePush", () => {
  const base: PushStatus = {
    accountId: "a",
    mode: { kind: "imapIdle" },
    health: { kind: "live" },
    detail: null,
    consentRefused: false,
    adminConsent: null,
    since: "2026-10-15T09:00:00Z",
  };
  const label = (patch: Partial<PushStatus>, kind: "imap" | "microsoft" = "imap") =>
    describePush({ ...base, ...patch }, kind).label;

  it("names how mail arrives", () => {
    expect(label({})).toBe("Instant (IMAP IDLE)");
    expect(label({ mode: { kind: "imapNotify" } })).toBe("Instant (IMAP NOTIFY)");
    expect(label({ mode: { kind: "graphRelay" } }, "microsoft")).toBe("Instant (Microsoft push)");
    expect(label({ mode: { kind: "polling", everySecs: 30 } }, "microsoft")).toBe(
      "Every 30 seconds",
    );
    expect(label({ mode: { kind: "off" } })).toBe("Instant updates are off");
    expect(
      label({ mode: { kind: "polling", everySecs: 120 }, health: { kind: "unsupported" } }),
    ).toBe("Every 2 minutes — this server can't push new mail");
    expect(
      label(
        { mode: { kind: "polling", everySecs: 30 }, health: { kind: "degraded", reason: "x" } },
        "microsoft",
      ),
    ).toBe("Every 30 seconds — push service unreachable");
    expect(
      label({
        mode: { kind: "polling", everySecs: 60 },
        health: { kind: "degraded", reason: "x" },
      }),
    ).toBe("Every minute — reconnecting");
    expect(label({ health: { kind: "connecting" } })).toBe("Connecting…");
  });

  it("knows what the user can do", () => {
    const consent = { health: { kind: "consentMissing" } as const };
    expect(describePush({ ...base, ...consent }, "microsoft").action).toBe("signIn");
    expect(
      describePush({ ...base, ...consent, consentRefused: true }, "microsoft").action,
    ).toBeNull();
    expect(
      describePush(
        { ...base, ...consent, consentRefused: true, adminConsent: { url: "u", tenant: "t" } },
        "microsoft",
      ).action,
    ).toBe("approve");
  });

  it("counts seconds and minutes", () => {
    expect(everyLabel(30)).toBe("Every 30 seconds");
    expect(everyLabel(60)).toBe("Every minute");
    expect(everyLabel(300)).toBe("Every 5 minutes");
  });
});

describe("About", () => {
  it("discloses the push service and links to the privacy policy", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const openLink = vi.spyOn(backend, "openLink").mockResolvedValue();
    openAt("about", backend);
    expect(await screen.findByText("Instant updates for Microsoft accounts")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Privacy policy" }));
    expect(openLink).toHaveBeenCalledWith(PRIVACY_URL);
  });
});

describe("the demo backend", () => {
  it("keeps the background settings out of settings saves", async () => {
    const backend = new MockBackend();
    const settings = await backend.getSettings();
    const saved = await backend.updateSettings({
      ...settings,
      runInBackground: false,
      startAtLogin: true,
      theme: "dark",
    });
    expect(saved).toMatchObject({ runInBackground: true, startAtLogin: false, theme: "dark" });
    const state = await backend.setStartAtLogin(true, false);
    expect(state).toMatchObject({ startAtLogin: true, startHidden: false });
    expect(await backend.getSettings()).toMatchObject({ startAtLogin: true, startHidden: false });
  });

  it("keeps navigation for the page until it asks", async () => {
    const backend = new MockBackend();
    backend.navigate({ kind: "compose" });
    expect(await backend.takePendingNavigation()).toEqual({ kind: "compose" });
    expect(await backend.takePendingNavigation()).toBeNull();
    const seen: string[] = [];
    backend.onNavigate((to) => seen.push(to.kind));
    backend.navigate({ kind: "inbox", accountId: null, notice: null });
    expect(seen).toEqual(["inbox"]);
    expect(await backend.takePendingNavigation()).toBeNull();
  });

  it.each<[MockOptions["push"], string]>([
    ["consent", "consentMissing"],
    ["refused", "consentMissing"],
    ["unreachable", "degraded"],
  ])("shows the Microsoft account's push as %s", async (push, health) => {
    const statuses = await new MockBackend({ push }).getPushStatuses();
    expect(statuses.find((s) => s.accountId === "acct-work")?.health.kind).toBe(health);
    expect(statuses.find((s) => s.accountId === "acct-home")?.health.kind).toBe("live");
  });
});
