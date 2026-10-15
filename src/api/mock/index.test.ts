import { afterEach, describe, expect, it, vi } from "vitest";

import type { ActiveReminder, DataChangedEvent, RemindersChangedEvent } from "../types";
import { MockBackend } from ".";

function addStyle(css: string): CSSRule[] {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  return Array.from(style.sheet!.cssRules);
}

afterEach(async () => {
  await new MockBackend().setZoom(1);
  document.head.querySelectorAll("style").forEach((s) => s.remove());
  document.documentElement.removeAttribute("style");
});

describe("the demo's text size", () => {
  it("scales media queries and viewport units to the zoomed window", async () => {
    const [media, dialog] = addStyle(
      "@media (max-width: 900px) { a { color: red } }" +
        ".dialog { width: 400px; max-height: calc(100vh - 48px) }",
    ) as [CSSMediaRule, CSSStyleRule];
    const backend = new MockBackend();

    await backend.setZoom(1.2);
    expect(media.media.mediaText).toBe("(max-width: 1080px)");
    expect(dialog.style.maxHeight).toBe("calc(83.333vh - 48px)");
    expect(dialog.style.width).toBe("400px");

    await backend.setZoom(1);
    expect(media.media.mediaText).toBe("(max-width: 900px)");
    expect(dialog.style.maxHeight).toBe("calc(100vh - 48px)");
  });

  it("scales style sheets that load later", async () => {
    await new MockBackend().setZoom(1.1);
    const [media] = addStyle("@media (max-width: 900px) { a { color: red } }") as [CSSMediaRule];
    await new Promise((resolve) => setTimeout(resolve));
    expect(media.media.mediaText).toBe("(max-width: 990px)");
  });
});

describe("push and background settings", () => {
  it("default like the desktop app", async () => {
    const settings = await new MockBackend().getSettings();
    expect(settings).toMatchObject({
      notifyScope: "inbox",
      notifyShowPreview: true,
      mutedAccountIds: [],
      notificationsPausedUntil: null,
      defaultSnoozeMinutes: 5,
      instantPush: true,
      usePushRelay: true,
      runInBackground: true,
      startAtLogin: false,
      startHidden: true,
      showTrayIcon: true,
    });
  });

  it("report push per account and follow the instant updates switch", async () => {
    const backend = new MockBackend();
    const accounts = await backend.listAccounts();
    const statuses = await backend.getPushStatuses();
    expect(statuses.map((s) => s.accountId)).toEqual(accounts.map((a) => a.id));
    for (const status of statuses) {
      const account = accounts.find((a) => a.id === status.accountId)!;
      expect(status.mode).toEqual(
        account.kind === "imap" ? { kind: "imapIdle" } : { kind: "graphRelay" },
      );
      expect(status.health).toEqual({ kind: "live" });
    }

    // Without the push service, Microsoft accounts check every 30 seconds.
    const settings = await backend.getSettings();
    await backend.updateSettings({ ...settings, usePushRelay: false });
    for (const status of await backend.getPushStatuses()) {
      const account = accounts.find((a) => a.id === status.accountId)!;
      if (account.kind === "microsoft") {
        expect(status.mode).toEqual({ kind: "polling", everySecs: 30 });
      }
    }

    const seen: string[] = [];
    backend.onPushStatus((s) => seen.push(s.mode.kind));
    await backend.updateSettings({ ...settings, instantPush: false });
    await new Promise((resolve) => setTimeout(resolve));
    expect(seen).toEqual(accounts.map(() => "off"));
    expect((await backend.getPushStatuses()).every((s) => s.mode.kind === "off")).toBe(true);
  });

  it("reject a snooze shorter than a minute", async () => {
    const backend = new MockBackend();
    const settings = await backend.getSettings();
    await expect(
      backend.updateSettings({ ...settings, defaultSnoozeMinutes: 0 }),
    ).rejects.toMatchObject({
      code: "invalidInput",
    });
  });
});

describe("the demo's reminders", () => {
  it("come due, snooze, dismiss and clear like the desktop app's", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    try {
      // Wednesday, October 14, 2026, 10:24 AM.
      vi.setSystemTime(new Date(2026, 9, 14, 10, 24));
      const backend = new MockBackend({ overdueReminder: true });
      const events: RemindersChangedEvent[] = [];
      backend.onRemindersChanged((e) => events.push(e));
      const active = await backend.listActiveReminders();
      expect(active.map((r) => [r.title, r.kind, r.overdue])).toEqual([
        ["Send Q3 metrics to Dana", "task", true],
        ["Focus time", "event", false],
        ["Security review", "event", false],
      ]);
      const [task, focus] = active as [ActiveReminder, ActiveReminder];

      await expect(backend.snoozeReminder(focus.key, { minutes: 0 })).rejects.toMatchObject({
        code: "invalidInput",
      });
      await expect(backend.snoozeReminder("event:gone:1", { minutes: 5 })).rejects.toMatchObject({
        code: "notFound",
      });
      const snoozed = await backend.snoozeReminder(focus.key, { minutes: 5 });
      expect(snoozed.at(-1)).toMatchObject({
        key: focus.key,
        snoozedUntil: new Date(2026, 9, 14, 10, 29).toISOString(),
      });
      await Promise.resolve();
      expect(events.at(-1)?.cleared).toEqual([focus.key]);

      // Completing the task clears its reminder.
      await backend.updateTask(task.itemId, { status: "completed" });
      await Promise.resolve();
      expect(events.at(-1)?.cleared).toEqual([task.key]);

      await backend.dismissReminders([focus.key]);
      expect((await backend.listActiveReminders()).map((r) => r.title)).toEqual([
        "Security review",
      ]);

      // The snooze is over: due again, and announced.
      vi.setSystemTime(new Date(2026, 9, 14, 10, 30));
      const fresh = new MockBackend();
      const freshKey = (await fresh.listActiveReminders())[0]!.key;
      await fresh.snoozeReminder(freshKey, { minutes: 5 });
      const seen: RemindersChangedEvent[] = [];
      fresh.onRemindersChanged((e) => seen.push(e));
      vi.setSystemTime(new Date(2026, 9, 14, 10, 36));
      fresh.checkReminders();
      await Promise.resolve();
      expect(seen.at(-1)?.newlyDue.map((r) => r.key)).toContain(freshKey);

      // Off: nothing is active.
      const settings = await fresh.getSettings();
      await fresh.updateSettings({ ...settings, remindersEnabled: false });
      expect(await fresh.listActiveReminders()).toEqual([]);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("the demo's signatures", () => {
  it("start with Acme's on the work account and Alex's own at home", async () => {
    const backend = new MockBackend();
    expect((await backend.getAppInfo()).signatureServer).toBe("signature.cumulas.co");

    const work = await backend.getAccountSignature("acct-work");
    expect(work.organization).toMatchObject({ state: "ready", onReplies: true, stale: false });
    // A Cumulas Signatures table: the logo travels inside it, so the demo needs no network.
    expect(work.organization.html).toMatch(/^<table[^>]*style="[^"]*font-family:Arial/);
    expect(work.organization.html).toContain('src="data:image/svg+xml;base64,');
    expect(work.organization.html).toContain(">Alex Morgan</div>");
    expect(work.organization.html).toContain('href="https://acme.example"');
    // Sanitized as the core keeps it, which a frame shows as it is.
    expect(work.organization.html).toContain("<tbody>");
    expect(await backend.getComposeSignature("acct-work", null)).toEqual({
      kind: "organization",
      html: work.organization.html,
    });
    expect((await backend.getComposeSignature("acct-work", "replyAll")).kind).toBe("organization");

    const home = await backend.getAccountSignature("acct-home");
    expect(home.organization.state).toBe("unsupported");
    expect(home.ownHtml).toContain("Alex Morgan");
    expect(await backend.getComposeSignature("acct-home", "forward")).toEqual({
      kind: "own",
      html: home.ownHtml,
    });
    // An IMAP account's organization is never asked.
    expect(
      (await backend.checkOrganizationSignature("acct-home")).organization.checkedAt,
    ).toBeNull();
  });

  it("save the own signature, cleaned, and say so", async () => {
    const backend = new MockBackend();
    const events: DataChangedEvent[] = [];
    backend.onDataChanged((e) => events.push(e));
    const saved = await backend.updateOwnSignature(
      "acct-home",
      '<p onclick="steal()">Alex</p><script>steal()</script>',
      false,
    );
    expect(saved).toMatchObject({ ownHtml: "<p>Alex</p>", ownOnReplies: false });
    await Promise.resolve();
    expect(events).toEqual([{ kinds: ["signatures"], accountId: "acct-home" }]);
    expect((await backend.getComposeSignature("acct-home", "reply")).kind).toBe("none");
    expect((await backend.getComposeSignature("acct-home", null)).kind).toBe("own");

    // What an emptied editor leaves is no signature.
    expect((await backend.updateOwnSignature("acct-home", "<div><br></div>", true)).ownHtml).toBe(
      "",
    );
    await expect(
      backend.updateOwnSignature("acct-home", "x".repeat(512 * 1024 + 1), true),
    ).rejects.toMatchObject({ code: "invalidInput" });
  });

  it("check a new work account's organization before its first message", async () => {
    const backend = new MockBackend();
    const account = await backend.addMicrosoftAccount("sam@contoso.example");
    const before = await backend.getAccountSignature(account.id);
    expect(before.organization).toMatchObject({ state: "unchecked", checkedAt: null });

    // Contoso doesn't use Cumulas Signatures, so the own signature applies.
    await backend.updateOwnSignature(account.id, "<div>Sam</div>", true);
    expect(await backend.getComposeSignature(account.id, null)).toEqual({
      kind: "own",
      html: "<div>Sam</div>",
    });
    const after = await backend.getAccountSignature(account.id);
    expect(after.organization.state).toBe("notUsed");
    expect(after.organization.checkedAt).not.toBeNull();
  });

  it("show the states the demo can be started in", async () => {
    const relay = new MockBackend({ signature: "addedOnSend" });
    expect(await relay.getComposeSignature("acct-work", null)).toEqual({
      kind: "addedOnSend",
      html: null,
    });
    expect((await relay.getComposeSignature("acct-work", "forward")).kind).toBe("addedOnSend");
    const blocked = new MockBackend({ signature: "notAllowed" });
    expect((await blocked.getAccountSignature("acct-work")).organization.state).toBe("notAllowed");
    const stale = new MockBackend({ signature: "stale" });
    const checked = await stale.checkOrganizationSignature("acct-work");
    expect(checked.organization).toMatchObject({ state: "ready", stale: true });
    // Its first check failed: nothing known, and asking again fails too.
    const unreachable = new MockBackend({ signature: "unreachable" });
    expect((await unreachable.checkOrganizationSignature("acct-work")).organization).toMatchObject({
      state: "unchecked",
      checkedAt: null,
      stale: true,
    });
  });

  it("follow the core's rules for replies and for asking again", async () => {
    const backend = new MockBackend();
    const check = vi.spyOn(backend, "checkOrganizationSignature");
    const relay = { state: "addedOnSend", html: null, onReplies: false } as const;
    const now = new Date().toISOString();
    // A relay that leaves replies and forwards unsigned: nothing is said
    // for them, and the own signature isn't used either.
    backend.setOrganizationSignature("acct-work", { ...relay, checkedAt: now }, relay);
    await backend.updateOwnSignature("acct-work", "<div>Alex</div>", true);
    expect((await backend.getComposeSignature("acct-work", null)).kind).toBe("addedOnSend");
    expect((await backend.getComposeSignature("acct-work", "reply")).kind).toBe("none");
    expect(check).not.toHaveBeenCalled();

    // One that signs is asked again when its answer is over an hour old...
    const old = new Date(Date.now() - 2 * 60 * 60_000).toISOString();
    backend.setOrganizationSignature("acct-work", { checkedAt: old }, relay);
    await backend.getComposeSignature("acct-work", null);
    expect(check).toHaveBeenCalledTimes(1);
    // ...but one that doesn't sign the account's mail isn't (once a day).
    backend.setOrganizationSignature(
      "acct-work",
      { state: "notUsed", html: null, onReplies: false, checkedAt: old },
      { state: "notUsed", html: null, onReplies: false },
    );
    expect((await backend.getComposeSignature("acct-work", null)).kind).toBe("own");
    expect(check).toHaveBeenCalledTimes(1);
  });
});
