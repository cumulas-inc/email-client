import { afterEach, describe, expect, it, onTestFinished, vi } from "vitest";

import type {
  Account,
  ActiveReminder,
  BackendError,
  DataChangedEvent,
  OutgoingMessage,
  PermissionRequest,
  RemindersChangedEvent,
  SearchOptions,
} from "../types";
import { DEMO_UPDATE_COMMIT, MockBackend, mockAdminConsentUrl } from ".";

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

describe("the demo's updates", () => {
  it("start with none, or one installed", async () => {
    expect(await new MockBackend().getUpdateState()).toEqual({ kind: "none" });
    for (const update of ["installed", "restartfails"] as const) {
      expect(await new MockBackend({ update }).getUpdateState()).toEqual({
        kind: "installed",
        commit: DEMO_UPDATE_COMMIT,
        restart: "idle",
      });
    }
    expect((await new MockBackend().getAppInfo()).restartedToUpdate).toBe(false);
    const updated = new MockBackend({ restartedToUpdate: true });
    expect((await updated.getAppInfo()).restartedToUpdate).toBe(true);
  });

  it("restart once at a time, into an installed update only", async () => {
    const onRestart = vi.fn();
    const backend = new MockBackend({ onRestart });
    const states: unknown[] = [];
    backend.onUpdateState((state) => states.push(state));
    await expect(backend.restartApp()).rejects.toEqual({
      code: "unsupported",
      message: "There's no new version to restart into.",
    });
    expect(backend.calls.restart).toBe(0);

    backend.setUpdateState({ kind: "installed", commit: "c", restart: "idle" });
    await backend.restartApp();
    expect(backend.calls.restart).toBe(1);
    expect(onRestart).toHaveBeenCalledTimes(1);
    expect(await backend.getUpdateState()).toEqual({
      kind: "installed",
      commit: "c",
      restart: "restarting",
    });
    // Asked again while it restarts: nothing more.
    await backend.restartApp();
    expect(backend.calls.restart).toBe(1);
    expect(onRestart).toHaveBeenCalledTimes(1);
    await Promise.resolve();
    expect(states).toEqual([
      { kind: "installed", commit: "c", restart: "idle" },
      { kind: "installed", commit: "c", restart: "restarting" },
    ]);
  });

  it("fail a restart the demo is asked to, which can be tried again", async () => {
    const onRestart = vi.fn();
    const backend = new MockBackend({ update: "restartfails", onRestart });
    const states: unknown[] = [];
    backend.onUpdateState((state) => states.push(state));
    const failed = {
      code: "unsupported",
      message: "Couldn't restart. Try again, or quit Cumulas Mail and open it again.",
    };
    await expect(backend.restartApp()).rejects.toEqual(failed);
    await expect(backend.restartApp()).rejects.toEqual(failed);
    expect(backend.calls.restart).toBe(2);
    expect(onRestart).not.toHaveBeenCalled();
    await Promise.resolve();
    expect(states.map((s) => (s as { restart: string }).restart)).toEqual([
      "restarting",
      "failed",
      "restarting",
      "failed",
    ]);
  });

  it("ask the page to restart, as the tray does", () => {
    const backend = new MockBackend({ update: "installed" });
    const asked = vi.fn();
    const off = backend.onRestartRequested(asked);
    backend.requestRestart();
    expect(asked).toHaveBeenCalledTimes(1);
    off();
    backend.requestRestart();
    expect(asked).toHaveBeenCalledTimes(1);
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

  it("ask an organization at most once a minute, not before its service said, and say when", async () => {
    const backend = new MockBackend();
    const start = Date.now();
    const now = vi.spyOn(Date, "now").mockReturnValue(start);
    onTestFinished(() => now.mockRestore());
    const at = (ms: number) => new Date(start + ms).toISOString();
    const ready = { state: "ready", html: "<div>Acme</div>", onReplies: true } as const;
    const check = async () => (await backend.checkOrganizationSignature("acct-work")).organization;
    expect((await backend.getAccountSignature("acct-work")).organization.checkableAt).toBeNull();

    // A failed check: not again for a minute.
    backend.setOrganizationSignature("acct-work", {}, "unreachable");
    expect(await check()).toMatchObject({ stale: true, checkableAt: at(60_000) });
    backend.setOrganizationSignature("acct-work", {}, ready);
    now.mockReturnValue(start + 59_000);
    expect(await check()).toMatchObject({ stale: true, checkableAt: at(60_000) });
    expect((await backend.getAccountSignature("acct-work")).organization.checkableAt).toBe(
      at(60_000),
    );
    now.mockReturnValue(start + 60_000);
    expect(await check()).toMatchObject({ state: "ready", stale: false, checkableAt: at(120_000) });

    // A busy service that says to wait five minutes: not before then.
    now.mockReturnValue(start + 10 * 60_000);
    backend.setOrganizationSignature("acct-work", {}, { retryAfter: 300 });
    expect(await check()).toMatchObject({ stale: true, checkableAt: at(15 * 60_000) });
    backend.setOrganizationSignature("acct-work", {}, ready);
    now.mockReturnValue(start + 14 * 60_000);
    expect(await check()).toMatchObject({ stale: true, checkableAt: at(15 * 60_000) });
    now.mockReturnValue(start + 15 * 60_000);
    expect(await check()).toMatchObject({ stale: false, checkableAt: at(16 * 60_000) });

    // Less than a minute is a minute; one said while the clock was ahead
    // isn't waited for once it's set right, only the minute.
    now.mockReturnValue(start + 20 * 60_000);
    backend.setOrganizationSignature("acct-work", {}, { retryAfter: 5 });
    expect(await check()).toMatchObject({ stale: true, checkableAt: at(21 * 60_000) });
    now.mockReturnValue(start + 30 * 60_000);
    backend.setOrganizationSignature("acct-work", {}, { retryAfter: 3600 });
    expect(await check()).toMatchObject({ checkableAt: at(90 * 60_000) });
    now.mockReturnValue(start + 25 * 60_000);
    expect((await backend.getAccountSignature("acct-work")).organization.checkableAt).toBe(
      at(26 * 60_000),
    );
  });
});

describe("the demo's search", () => {
  const subjects = (messages: { subject: string }[]) => messages.map((m) => m.subject);

  it("finds what the app's index finds: operators, prefixes and inside subjects", async () => {
    const backend = new MockBackend();
    const fromPriya = await backend.searchMessages("from:priya");
    expect(fromPriya.length).toBeGreaterThan(1);
    for (const m of fromPriya) expect(m.from?.name).toBe("Priya Raman");
    expect(subjects(await backend.searchMessages("lisb"))).toEqual([
      "Your trip to Lisbon: booking confirmed",
    ]);
    expect(subjects(await backend.searchMessages("isbo"))).toEqual([
      "Your trip to Lisbon: booking confirmed",
    ]);
    expect(subjects(await backend.searchMessages('subject:"launch checklist"'))).toEqual([
      "Atlas launch checklist — final review today",
    ]);
    // Newest first, and only the account asked about.
    const atlas = await backend.searchMessages("atlas", "acct-work");
    expect(atlas.map((m) => m.receivedAt)).toEqual(
      atlas.map((m) => m.receivedAt).sort((a, b) => b.localeCompare(a)),
    );
    expect(await backend.searchMessages("atlas", "acct-home")).toEqual([]);
    // Nothing to look for: nothing, not everything.
    expect(await backend.searchMessages("*** ??")).toEqual([]);
  });

  it("adds the messages server searches found, once each", async () => {
    const backend = new MockBackend();
    const [lisbon] = await backend.searchMessages("lisbon");
    const [sunday] = await backend.searchMessages("subject:sunday");
    const ids = [sunday!.id, lisbon!.id, "msg-gone"];
    const found = await backend.searchMessages("lisbon", undefined, ids, { sort: "newest" });
    expect(subjects(found)).toEqual(["Sunday lunch", "Your trip to Lisbon: booking confirmed"]);
    // Best first: the text's own match above a server hit it doesn't match.
    expect(subjects(await backend.searchMessages("lisbon", undefined, ids))).toEqual([
      "Your trip to Lisbon: booking confirmed",
      "Sunday lunch",
    ]);
    // Even when the text finds nothing, but only in the account asked about.
    expect(subjects(await backend.searchMessages("", undefined, [sunday!.id]))).toEqual([
      "Sunday lunch",
    ]);
    expect(await backend.searchMessages("", "acct-work", [sunday!.id])).toEqual([]);
  });

  it("searches a whole mailbox on its server and stores what it finds", async () => {
    const backend = new MockBackend();
    const changes: DataChangedEvent[] = [];
    backend.onDataChanged((e) => changes.push(e));
    expect(await backend.searchMessages("invoice")).toEqual([]);
    const archive = (await backend.listFolders()).find((f) => f.id === "w-archive")!;

    const result = await backend.searchServer("s-1", "invoice", "acct-work");
    expect(result).toMatchObject({
      searchId: "s-1",
      accountId: "acct-work",
      status: "done",
      added: 1,
      hasMore: false,
      error: null,
    });
    expect(result.messageIds).toHaveLength(1);
    await Promise.resolve();
    expect(changes).toEqual([{ kinds: ["mail"], accountId: "acct-work" }]);

    // Stored: found locally, openable and listed in its folder, which
    // counted it already.
    const [invoice] = await backend.searchMessages("invoice", undefined, result.messageIds);
    expect(invoice).toMatchObject({ subject: "Northwind invoice 2023-118", folderId: "w-archive" });
    expect((await backend.getMessage(invoice!.id)).bodyText).toMatch(/invoice 2023-118/);
    const listed = await backend.listMessages("w-archive", { offset: 0, limit: 50 });
    expect(subjects(listed.items)).toContain("Northwind invoice 2023-118");
    expect((await backend.listFolders()).find((f) => f.id === "w-archive")).toEqual(archive);

    // Found again: nothing new, and no change to tell.
    const again = await backend.searchServer("s-2", "invoice", "acct-work");
    expect(again).toMatchObject({ status: "done", messageIds: result.messageIds, added: 0 });
    await Promise.resolve();
    expect(changes).toHaveLength(1);

    // Like Graph: no matches inside words.
    expect((await backend.searchServer("s-3", "nvoice", "acct-work")).messageIds).toEqual([]);
    // Messages already on the device count too, but aren't added.
    expect(await backend.searchServer("s-4", "lisbon", "acct-work")).toMatchObject({
      messageIds: [expect.any(String)],
      added: 0,
    });
    expect(await backend.searchServer("s-5", "*** ??", "acct-work")).toMatchObject({
      status: "done",
      messageIds: [],
    });
  });

  it("pages through more than 50 matches", async () => {
    const backend = new MockBackend({ worst: true });
    const pages = [
      await backend.searchServer("s-1", "purchase order", "acct-long"),
      await backend.searchServer("s-1", "purchase order", "acct-long", true),
      await backend.searchServer("s-1", "purchase order", "acct-long", true),
      await backend.searchServer("s-1", "purchase order", "acct-long", true),
    ];
    expect(pages.map((p) => [p.messageIds.length, p.hasMore])).toEqual([
      [50, true],
      [50, true],
      [expect.any(Number), false],
      [0, false],
    ]);
    const all = pages.flatMap((p) => p.messageIds);
    expect(new Set(all).size).toBe(all.length);
    const found = await backend.searchMessages("", undefined, all);
    expect(found.filter((m) => /^Purchase order 4500/.test(m.subject))).toHaveLength(130);
    // Newest first across the pages.
    const dates = all.map((id) => found.find((m) => m.id === id)!.receivedAt);
    expect(dates).toEqual([...dates].sort((a, b) => b.localeCompare(a)));
    // The worst case's archive still counts what its server reports.
    const archive = (await backend.listFolders()).find((f) => f.id === "l-archive")!;
    expect(archive.totalCount).toBe(251_877);
  });

  it("cancels a search when a newer one starts, or when asked, and stores nothing", async () => {
    const backend = new MockBackend();
    const resume = backend.pauseServerSearch();
    const first = backend.searchServer("s-1", "offsite", "acct-work");
    const second = backend.searchServer("s-2", "offsite kickoff", "acct-work");
    const third = backend.searchServer("s-2", "offsite kickoff", "acct-home");
    resume();
    expect(await first).toMatchObject({ status: "cancelled", messageIds: [], added: 0 });
    expect(await second).toMatchObject({ status: "done", added: 1 });
    expect(await third).toMatchObject({ status: "done", messageIds: [] });
    // The first search's own find, "Offsite venue options", is still only on the server.
    expect(subjects(await backend.searchMessages("venue"))).toEqual([]);

    const paused = backend.pauseServerSearch();
    const fourth = backend.searchServer("s-3", "venue", "acct-work");
    await backend.cancelServerSearch("s-2"); // Ended already: nothing happens.
    await backend.cancelServerSearch("s-3");
    paused();
    expect(await fourth).toMatchObject({ status: "cancelled" });
    expect(subjects(await backend.searchMessages("venue"))).toEqual([]);
    // Its pages are gone too.
    expect(await backend.searchServer("s-4", "venue", "acct-work", true)).toMatchObject({
      status: "done",
      messageIds: [],
      hasMore: false,
    });
  });

  it("cancels a late call for an ended search without replacing the newer one", async () => {
    const backend = new MockBackend();
    await backend.searchServer("s-1", "northwind", "acct-work");
    const resume = backend.pauseServerSearch();
    const newer = backend.searchServer("s-2", "venue", "acct-work");
    // A call for the first search arriving after the second started (as the
    // app's commands can), and one canceled before it started.
    const late = backend.searchServer("s-1", "offsite", "acct-home");
    await backend.cancelServerSearch("s-0");
    const early = backend.searchServer("s-0", "offsite", "acct-home");
    resume();
    expect(await late).toMatchObject({ status: "cancelled", messageIds: [] });
    expect(await early).toMatchObject({ status: "cancelled", messageIds: [] });
    expect(await newer).toMatchObject({ status: "done", added: 1 });
    // An empty query is done at once and leaves the search alone.
    expect(await backend.searchServer("s-9", "  ", "acct-work")).toMatchObject({ status: "done" });
    expect(await backend.searchServer("s-2", "venue", "acct-work")).toMatchObject({
      status: "done",
    });
  });

  it("searches a message's text here once it was opened, and the server all of it", async () => {
    const backend = new MockBackend();
    const checklist = "Atlas launch checklist — final review today";
    // "retention" is only in its text, past its preview.
    expect(await backend.searchMessages("retention")).toEqual([]);
    const server = await backend.searchServer("s-1", "retention", "acct-work");
    expect(server).toMatchObject({ status: "done", added: 0 });
    const included = await backend.searchMessages("retention", undefined, server.messageIds);
    expect(subjects(included)).toEqual([checklist]);
    await backend.getMessage(server.messageIds[0]!);
    expect(subjects(await backend.searchMessages("retention"))).toEqual([checklist]);
  });

  it("matches inside words on an IMAP server, by the start of words on Graph", async () => {
    const backend = new MockBackend();
    const home = await backend.searchServer("s-1", "fari phot", "acct-home");
    const found = await backend.searchMessages("", undefined, home.messageIds);
    expect(subjects(found)).toEqual(["Safari photos from Etosha"]);
    expect((await backend.searchServer("s-2", "nvoice", "acct-work")).messageIds).toEqual([]);
  });

  it("fails server searches the demo is asked to, and those of an account whose sync fails", async () => {
    const failing = new MockBackend({ searchFail: true });
    expect(await failing.searchServer("s-1", "zebras", "acct-home")).toMatchObject({
      status: "failed",
      error: { code: "network", message: "Could not reach imap.morgan-family.example." },
    });
    expect(await failing.searchServer("s-1", "invoice", "acct-work")).toMatchObject({
      status: "done",
    });
    const worst = new MockBackend({ worst: true });
    const broken = (await worst.listAccounts()).filter((a) => a.syncError);
    expect(broken).not.toHaveLength(0);
    for (const account of broken) {
      expect(await worst.searchServer("s-2", "order", account.id)).toMatchObject({
        status: "failed",
        error: { code: "network" },
      });
    }
  });

  it("reports an account's failure in the result, and rejects only bad calls", async () => {
    const backend = new MockBackend();
    const offline: BackendError = {
      code: "network",
      message: "Could not reach imap.morgan-family.example.",
    };
    backend.failServerSearch("acct-home", offline);
    const [home, work] = await Promise.all([
      backend.searchServer("s-1", "zebras", "acct-home"),
      backend.searchServer("s-1", "invoice", "acct-work"),
    ]);
    expect(home).toEqual({
      searchId: "s-1",
      accountId: "acct-home",
      status: "failed",
      messageIds: [],
      added: 0,
      hasMore: false,
      error: offline,
    });
    expect(work).toMatchObject({ status: "done", added: 1 });

    backend.failServerSearch("acct-home", null);
    expect(await backend.searchServer("s-1", "zebras", "acct-home")).toMatchObject({
      status: "done",
      added: 1,
    });
    await expect(backend.searchServer("s-1", "zebras", "acct-gone")).rejects.toMatchObject({
      code: "notFound",
    });
    await expect(backend.searchServer("", "zebras", "acct-home")).rejects.toMatchObject({
      code: "invalidInput",
    });
    await expect(
      backend.searchServer("s".repeat(101), "zebras", "acct-home"),
    ).rejects.toMatchObject({ code: "invalidInput" });
  });
});

describe("the demo's Microsoft sign-in", () => {
  const PUBLISHED_LINK =
    "https://login.microsoftonline.com/organizations/v2.0/adminconsent?client_id=10abb2b9-1a7a-4dbc-ba0f-c9a0de9088ad&scope=openid+profile+email+offline_access+https%3A%2F%2Fgraph.microsoft.com%2FUser.Read+https%3A%2F%2Fgraph.microsoft.com%2FMail.ReadWrite+https%3A%2F%2Fgraph.microsoft.com%2FMail.Send+https%3A%2F%2Fgraph.microsoft.com%2FMailboxSettings.Read+https%3A%2F%2Fgraph.microsoft.com%2FCalendars.ReadWrite+https%3A%2F%2Fgraph.microsoft.com%2FContacts.ReadWrite+https%3A%2F%2Fgraph.microsoft.com%2FTasks.ReadWrite+https%3A%2F%2Fgraph.microsoft.com%2FInformationProtectionPolicy.Read+api%3A%2F%2F10abb2b9-1a7a-4dbc-ba0f-c9a0de9088ad%2FSync.Receive&redirect_uri=https%3A%2F%2Fflatpak.cumulas.co%2Fadmin-consent%2F";
  const ADMIN_PAGE = "https://flatpak.cumulas.co/admin-consent/";
  const ALL: PermissionRequest = { kind: "all" };
  const MAIL_ONLY: PermissionRequest = { kind: "mailOnly" };
  const CURRENT: PermissionRequest = { kind: "current" };
  const calendar: PermissionRequest = { kind: "feature", feature: "calendar" };
  const EVERY_FEATURE = ["calendar", "contacts", "tasks", "mailboxSettings", "sensitivityLabels"];

  /** Every data-changed event, once the microtasks that carry them ran. */
  function changes(backend: MockBackend) {
    const events: DataChangedEvent[] = [];
    backend.onDataChanged((e) => events.push(e));
    return async () => {
      await Promise.resolve();
      return events;
    };
  }

  const work = async (backend: MockBackend) =>
    (await backend.listAccounts()).find((a) => a.id === "acct-work")!;

  it("builds the approval link the landing page publishes", () => {
    expect(mockAdminConsentUrl("organizations")).toBe(PUBLISHED_LINK);
    const own = new URL(mockAdminConsentUrl("contoso.example", true));
    expect(own.pathname).toBe("/contoso.example/v2.0/adminconsent");
    expect(own.searchParams.get("client_id")).toBe("11111111-2222-3333-4444-555555555555");
    expect(own.searchParams.get("redirect_uri")).toBe("http://localhost");
    expect(own.searchParams.get("scope")).not.toContain("Sync.Receive");
  });

  it("starts the Microsoft account without the features ?missing and ?refused name", async () => {
    const backend = new MockBackend({
      missing: ["tasks", "calendar", "mailboxSettings"],
      refused: ["calendar", "contacts", "mailboxSettings"],
    });
    const account = await work(backend);
    // In display order; a missing feature isn't refused too, and the mailbox
    // time zone never is.
    expect(account.needsPermission).toEqual(["calendar", "tasks", "mailboxSettings"]);
    expect(account.refusedFeatures).toEqual(["contacts"]);
    const own = (items: { accountId: string }[]) =>
      items.filter((i) => i.accountId === "acct-work");
    expect(own(await backend.listCalendars())).toEqual([]);
    expect(own(await backend.listContacts())).toEqual([]);
    expect(own(await backend.listTaskLists())).toEqual([]);
    expect(own(await backend.listTasks(null, true))).toEqual([]);
    // The IMAP account keeps everything.
    expect((await backend.listCalendars()).length).toBeGreaterThan(0);
    const home = (await backend.listAccounts()).find((a) => a.id === "acct-home")!;
    expect([home.needsPermission, home.refusedFeatures]).toEqual([[], []]);
  });

  it("gives approval links for the published registration, or an organization's own", async () => {
    const published = new MockBackend({ push: "refused" });
    const [pushStatus] = await published.getPushStatuses();
    expect(pushStatus!.adminConsent).toEqual({
      url: mockAdminConsentUrl("acme.example"),
      tenant: "acme.example",
      adminPageUrl: ADMIN_PAGE,
    });
    await expect(published.addMicrosoftAccount("admin@contoso.example")).rejects.toEqual({
      code: "adminConsentRequired",
      message:
        "Your organization requires an administrator to approve Cumulas Mail before you can sign in. Send the approval link to your IT department.",
      details: {
        adminConsentUrl: mockAdminConsentUrl("contoso.example"),
        tenant: "contoso.example",
        adminPageUrl: ADMIN_PAGE,
        canRequestApproval: false,
      },
    });

    const own = new MockBackend({ push: "refused", customClient: true });
    const [ownPush] = await own.getPushStatuses();
    expect(ownPush!.adminConsent).toMatchObject({
      url: mockAdminConsentUrl("acme.example", true),
      adminPageUrl: null,
    });
    await expect(own.microsoftAdminConsent("acct-work")).resolves.toEqual({
      adminConsentUrl: mockAdminConsentUrl("demo-tenant", true),
      tenant: "demo-tenant",
      adminPageUrl: null,
    });
  });

  it("needs approval for everything at admin@, whatever is asked", async () => {
    const backend = new MockBackend();
    for (const request of [ALL, MAIL_ONLY, CURRENT, calendar]) {
      await expect(
        backend.addMicrosoftAccount("admin@contoso.example", request),
      ).rejects.toMatchObject({ code: "adminConsentRequired" });
    }
  });

  it("lets partial@ start with mail only, and needs approval for the rest", async () => {
    const backend = new MockBackend();
    await expect(backend.addMicrosoftAccount("partial@contoso.example")).rejects.toMatchObject({
      code: "adminConsentRequired",
    });
    const account = await backend.addMicrosoftAccount("partial@contoso.example", MAIL_ONLY);
    expect(account.needsPermission).toEqual(EVERY_FEATURE);
    await expect(
      backend.addMicrosoftAccount("partial@contoso.example", calendar),
    ).rejects.toMatchObject({ code: "adminConsentRequired" });
    // Mail only again updates the account in place.
    const again = await backend.addMicrosoftAccount("Partial@Contoso.example", MAIL_ONLY);
    expect(again.id).toBe(account.id);
    expect(
      (await backend.listAccounts()).filter((a) => a.email.startsWith("partial")),
    ).toHaveLength(1);
  });

  it("offers Request approval at workflow@ once, then reports the request declined", async () => {
    const backend = new MockBackend();
    await expect(backend.addMicrosoftAccount("workflow@contoso.example")).rejects.toMatchObject({
      code: "adminConsentRequired",
      details: { canRequestApproval: true },
    });
    await expect(backend.addMicrosoftAccount("workflow@contoso.example")).rejects.toMatchObject({
      code: "consentDeclined",
    });
    // Per backend: a new one asks again.
    await expect(
      new MockBackend().addMicrosoftAccount("workflow@contoso.example"),
    ).rejects.toMatchObject({ code: "adminConsentRequired" });
  });

  it("declines, cancels and times out", async () => {
    const backend = new MockBackend();
    await expect(backend.addMicrosoftAccount("declined@contoso.example")).rejects.toMatchObject({
      code: "consentDeclined",
    });
    await expect(backend.addMicrosoftAccount("cancel@contoso.example")).rejects.toEqual({
      code: "signInCancelled",
      message: "Sign-in was canceled.",
    });
    await expect(backend.addMicrosoftAccount("timeout@contoso.example")).rejects.toEqual({
      code: "signInCancelled",
      message: "Sign-in was not completed in time. Try again.",
      details: { reason: "timeout" },
    });
  });

  it.each([
    ["unassigned", "notAssigned", 50105],
    ["disabled", "appDisabled", 7000112],
    ["notsetup", "appNotSetUp", 65005],
    ["noservice", "serviceNotEnabled", 650052],
    ["mfa", "securityInfo", 53004],
    ["device", "conditionalAccess", 53000],
    ["blocked", "conditionalAccess", 53003],
  ])("blocks %s@ as %s", async (prefix, reason, aadsts) => {
    const error = await new MockBackend()
      .addMicrosoftAccount(`${prefix}@fabrikam.example`, MAIL_ONLY)
      .catch((e: unknown) => e as BackendError);
    expect(error).toMatchObject({
      code: "signInBlocked",
      details: { reason, aadsts, tenant: "fabrikam.example", adminPageUrl: ADMIN_PAGE },
    });
    expect((error as BackendError).message).toMatch(/Ask your IT team/);
    expect((error as BackendError).details?.adminConsentUrl).toBe(
      reason === "appNotSetUp" ? mockAdminConsentUrl("fabrikam.example") : undefined,
    );
  });

  it("adds a new account with what was asked for", async () => {
    const backend = new MockBackend();
    // Sensitivity labels are asked for only when turned on by themselves.
    expect((await backend.addMicrosoftAccount("a@contoso.example")).needsPermission).toEqual([
      "sensitivityLabels",
    ]);
    expect(
      (await backend.addMicrosoftAccount("b@contoso.example", MAIL_ONLY)).needsPermission,
    ).toEqual(EVERY_FEATURE);
    expect(
      (await backend.addMicrosoftAccount("c@contoso.example", calendar)).needsPermission,
    ).toEqual(["contacts", "tasks", "mailboxSettings", "sensitivityLabels"]);
    expect(
      (await backend.addMicrosoftAccount("d@contoso.example", CURRENT)).needsPermission,
    ).toEqual(["sensitivityLabels"]);
  });

  it("never lists sensitivity labels for personal accounts", async () => {
    const backend = new MockBackend();
    const personal = await backend.addMicrosoftAccount("sam@outlook.com");
    expect(personal.needsPermission).toEqual([]);
    expect(
      (await backend.addMicrosoftAccount("kim@hotmail.com", MAIL_ONLY)).needsPermission,
    ).toEqual(["calendar", "contacts", "tasks", "mailboxSettings"]);
    await expect(
      backend.addMicrosoftAccount("sam@outlook.com", {
        kind: "feature",
        feature: "sensitivityLabels",
      }),
    ).rejects.toEqual({
      code: "invalidInput",
      message: "Sensitivity labels are only for work and school accounts.",
    });
    expect(await backend.getProtectionSetup(personal.id)).toMatchObject({
      supported: false,
      labels: [],
    });
  });

  it("keeps sensitivity labels off when an account signs in again for everything", async () => {
    vi.useFakeTimers();
    onTestFinished(() => void vi.useRealTimers());
    const backend = new MockBackend({ missing: ["calendar", "sensitivityLabels"] });
    const again = await backend.addMicrosoftAccount("alex.morgan@acme.example", ALL);
    expect(again.needsPermission).toEqual(["sensitivityLabels"]);
    await vi.advanceTimersByTimeAsync(1000);
    expect((await backend.getProtectionSetup("acct-work")).labels).toEqual([]);

    // Turning them on lists them with the sync that follows.
    const seen = changes(backend);
    const on = await backend.addMicrosoftAccount("alex.morgan@acme.example", {
      kind: "feature",
      feature: "sensitivityLabels",
    });
    expect(on.needsPermission).toEqual([]);
    await vi.advanceTimersByTimeAsync(1000);
    const setup = await backend.getProtectionSetup("acct-work");
    expect(setup.labels.map((l) => l.name)).toEqual([
      "Public",
      "General",
      "Confidential",
      "All Employees",
      "Project Atlas",
    ]);
    expect(setup.labelsListedAt).not.toBeNull();
    expect(await seen()).toContainEqual({
      kinds: ["accounts", "protection"],
      accountId: "acct-work",
    });
  });

  it("updates an account signing in again in place, by its address", async () => {
    vi.useFakeTimers();
    onTestFinished(() => void vi.useRealTimers());
    const backend = new MockBackend({ missing: ["calendar", "tasks"], refused: ["contacts"] });
    const count = (await backend.listAccounts()).length;
    const seen = changes(backend);
    const statuses = vi.fn();
    backend.onSyncStatus(statuses);
    /** The sync after each sign-in, which brings in what came back. */
    const synced = () => vi.advanceTimersByTimeAsync(1000);

    // Mail only and current keep what's missing, but check refusals again.
    const current = await backend.addMicrosoftAccount("Alex.Morgan@acme.example", CURRENT);
    expect(current).toMatchObject({
      id: "acct-work",
      needsPermission: ["calendar", "tasks"],
      refusedFeatures: [],
    });
    expect(statuses).toHaveBeenLastCalledWith(
      expect.objectContaining({ accountId: "acct-work", state: "syncing" }),
    );
    expect(await backend.getSyncStatuses()).toContainEqual(
      expect.objectContaining({ accountId: "acct-work", state: "syncing" }),
    );
    expect(await seen()).toEqual([{ kinds: ["accounts"], accountId: "acct-work" }]);
    await synced();
    expect(await seen()).toContainEqual({
      kinds: ["accounts", "contacts"],
      accountId: "acct-work",
    });
    expect(await backend.getSyncStatuses()).toContainEqual(
      expect.objectContaining({ accountId: "acct-work", state: "idle" }),
    );
    expect(
      (await backend.addMicrosoftAccount("alex.morgan@acme.example", MAIL_ONLY)).needsPermission,
    ).toEqual(["calendar", "tasks"]);
    await synced();

    // A feature comes back with its default calendar, once synced.
    const turnedOn = await backend.addMicrosoftAccount("alex.morgan@acme.example", calendar);
    expect(turnedOn.needsPermission).toEqual(["tasks"]);
    const workCalendars = async () =>
      (await backend.listCalendars()).filter((c) => c.accountId === "acct-work");
    expect(await workCalendars()).toEqual([]);
    await synced();
    expect(await workCalendars()).toMatchObject([
      { name: "Calendar", isDefault: true, canEdit: true },
    ]);

    // Everything: the rest, with a task list.
    const all = await backend.addMicrosoftAccount("alex.morgan@acme.example", ALL);
    expect([all.needsPermission, all.refusedFeatures]).toEqual([[], []]);
    await synced();
    expect(
      (await backend.listTaskLists()).filter((l) => l.accountId === "acct-work"),
    ).toMatchObject([{ name: "Tasks", isDefault: true }]);
    expect(await seen()).toContainEqual({ kinds: ["accounts", "tasks"], accountId: "acct-work" });
    expect(await backend.listAccounts()).toHaveLength(count);
  });

  it("finds the approval link for an account, an address or anyone", async () => {
    const backend = new MockBackend();
    expect(await backend.microsoftAdminConsent("acct-work")).toEqual({
      adminConsentUrl: mockAdminConsentUrl("demo-tenant"),
      tenant: "demo-tenant",
      adminPageUrl: ADMIN_PAGE,
    });
    expect((await backend.microsoftAdminConsent(undefined, "sam@fabrikam.example")).tenant).toBe(
      "fabrikam.example",
    );
    expect((await backend.microsoftAdminConsent()).tenant).toBe("organizations");

    const invalid = (message: string) => ({ code: "invalidInput", message });
    await expect(backend.microsoftAdminConsent("acct-home")).rejects.toEqual(
      invalid("Only Microsoft accounts need this."),
    );
    await expect(backend.microsoftAdminConsent(undefined, "sam@Hotmail.co.uk")).rejects.toEqual(
      invalid("Personal Microsoft accounts don't need an administrator's approval."),
    );
    await expect(backend.microsoftAdminConsent("acct-gone")).rejects.toMatchObject({
      code: "notFound",
    });
    // A personal account: Microsoft's tenant for consumers.
    const personal = new MockBackend();
    const state = (personal as unknown as { state: { accounts: Account[] } }).state;
    state.accounts[0]!.settings = {
      kind: "microsoft",
      tenantId: "9188040d-6c67-4c5b-b112-36a304b66dad",
      userId: "u",
    };
    await expect(personal.microsoftAdminConsent("acct-work")).rejects.toEqual(
      invalid("Personal Microsoft accounts don't need an administrator's approval."),
    );
  });

  it("checks refused features again, which the demo still refuses", async () => {
    const backend = new MockBackend({ refused: ["tasks"] });
    const seen = changes(backend);
    await backend.retryMicrosoftFeatures("acct-work");
    expect(await seen()).toEqual([{ kinds: ["accounts"], accountId: "acct-work" }]);
    expect((await work(backend)).refusedFeatures).toEqual(["tasks"]);
    await expect(backend.retryMicrosoftFeatures("acct-home")).rejects.toEqual({
      code: "invalidInput",
      message: "Only Microsoft accounts need this.",
    });
  });

  it("grants and revokes features, as later syncs would find", async () => {
    const backend = new MockBackend();
    const seen = changes(backend);

    backend.revokeFeature("acct-work", "contacts");
    backend.revokeFeature("acct-work", "tasks", "refused");
    expect(await work(backend)).toMatchObject({
      needsPermission: ["contacts"],
      refusedFeatures: ["tasks"],
    });
    expect((await backend.listContacts("acct-work")).length).toBe(0);
    expect((await backend.listTaskLists()).some((l) => l.accountId === "acct-work")).toBe(false);
    expect(await seen()).toEqual([
      { kinds: ["accounts", "contacts"], accountId: "acct-work" },
      { kinds: ["accounts", "tasks"], accountId: "acct-work" },
    ]);

    // Refused now: in one list only.
    backend.revokeFeature("acct-work", "contacts", "refused");
    expect(await work(backend)).toMatchObject({
      needsPermission: [],
      refusedFeatures: ["contacts", "tasks"],
    });

    backend.grantFeature("acct-work", "tasks");
    backend.grantFeature("acct-work", "mailboxSettings");
    expect((await work(backend)).refusedFeatures).toEqual(["contacts"]);
    expect(
      (await backend.listTaskLists()).filter((l) => l.accountId === "acct-work"),
    ).toMatchObject([{ name: "Tasks" }]);
    expect((await seen()).slice(-2)).toEqual([
      { kinds: ["accounts", "tasks"], accountId: "acct-work" },
      { kinds: ["accounts"], accountId: "acct-work" },
    ]);
  });

  it("refuses to create what a feature without permission would hold", async () => {
    const backend = new MockBackend();
    const [calendarId] = (await backend.listCalendars())
      .filter((c) => c.accountId === "acct-work")
      .map((c) => c.id);
    const [listId] = (await backend.listTaskLists())
      .filter((l) => l.accountId === "acct-work")
      .map((l) => l.id);
    // Gated with its data still there, as just before a sync drops it.
    const state = (backend as unknown as { state: { accounts: Account[] } }).state;
    const account = state.accounts.find((a) => a.id === "acct-work")!;
    account.needsPermission = ["calendar", "contacts"];
    account.refusedFeatures = ["tasks"];

    await expect(
      backend.createContact({ accountId: "acct-work", displayName: "Kim", emails: [], phones: [] }),
    ).rejects.toEqual({
      code: "permissionRequired",
      message:
        "Cumulas Mail needs permission to use the contacts of alex.morgan@acme.example. Turn it on in Settings → Accounts.",
      details: { feature: "contacts", accountId: "acct-work", reason: "notGranted" },
    });
    await expect(
      backend.createEvent({
        calendarId: calendarId!,
        subject: "Lunch",
        start: "2026-10-20T12:00:00Z",
        end: "2026-10-20T13:00:00Z",
        isAllDay: false,
        attendees: [],
        showAs: "busy",
        isOnlineMeeting: false,
      }),
    ).rejects.toMatchObject({ code: "permissionRequired", details: { feature: "calendar" } });
    await expect(backend.createTask({ listId: listId!, title: "Call Kim" })).rejects.toEqual({
      code: "permissionRequired",
      message:
        "Microsoft 365 refused access to the tasks of alex.morgan@acme.example. See Settings → Accounts.",
      details: { feature: "tasks", accountId: "acct-work", reason: "refused" },
    });
    // The IMAP account's are fine.
    await expect(
      backend.createContact({ accountId: "acct-home", displayName: "Kim", emails: [], phones: [] }),
    ).resolves.toMatchObject({ accountId: "acct-home" });
  });
});

describe("the demo's protected mail", () => {
  const EMAIL = "alex.morgan@acme.example";
  const GENERAL = "2a7f3c10-8b4e-4f6a-a1d2-3b4c5d6e7f80";
  const PUBLIC = "0f6c58a2-4a6b-4d3e-9b1a-1c2d3e4f5a60";
  const ALL_EMPLOYEES = "7b2d4e61-9c3a-4e5f-8a7b-6c5d4e3f2a10";
  const ATLAS = "9e8d7c6b-5a4f-4e3d-8c2b-1a0f9e8d7c60";
  const encrypt = { kind: "action", action: "encrypt" } as const;
  const general = { kind: "label", labelId: GENERAL } as const;

  const message = (fields: Partial<OutgoingMessage> = {}): OutgoingMessage => ({
    accountId: "acct-work",
    to: [{ name: null, address: "priya.raman@acme.example" }],
    cc: [],
    bcc: [],
    subject: "Pricing",
    bodyHtml: "<p>Hi</p>",
    attachments: [],
    ...fields,
  });

  it("lists the work account's labels, with nothing checked", async () => {
    const backend = new MockBackend();
    const setup = await backend.getProtectionSetup("acct-work");
    expect(setup).toMatchObject({
      accountId: "acct-work",
      supported: true,
      encrypt: { state: "notChecked", at: null, note: null, testMessage: false },
      doNotForward: { state: "notChecked" },
      policy: {
        required: true,
        defaultLabelId: GENERAL,
        justifyDowngrade: true,
        moreInfoUrl: "https://acme.example/labels",
      },
      labelsError: null,
      checking: false,
    });
    expect(setup.labels.map((l) => [l.parentName, l.name, l.hasProtection])).toEqual([
      [null, "Public", false],
      [null, "General", false],
      [null, "Confidential", false],
      ["Confidential", "All Employees", true],
      ["Highly Confidential", "Project Atlas", true],
    ]);
    expect(setup.labels.every((l) => l.check.state === "notChecked")).toBe(true);
    // IMAP accounts can't send protected mail.
    expect(await backend.getProtectionSetup("acct-home")).toEqual({
      accountId: "acct-home",
      supported: false,
      encrypt: {
        state: "notChecked",
        at: null,
        note: null,
        testMessage: false,
        passedAt: null,
        otherLabelId: null,
        otherLabelOnControl: false,
        changedAt: null,
      },
      doNotForward: {
        state: "notChecked",
        at: null,
        note: null,
        testMessage: false,
        passedAt: null,
        otherLabelId: null,
        otherLabelOnControl: false,
        changedAt: null,
      },
      labels: [],
      policy: { required: false, defaultLabelId: null, justifyDowngrade: false, moreInfoUrl: null },
      labelsListedAt: null,
      labelsError: null,
      checking: false,
    });
    await expect(backend.checkProtection("acct-home", [encrypt])).rejects.toEqual({
      code: "unsupported",
      message: "Only Microsoft 365 work and school accounts can send protected mail.",
    });
  });

  it.each(["missing", "refused"] as const)(
    "has no labels when the permission is %s, and keeps the encryption checks",
    async (how) => {
      const backend = new MockBackend({ [how]: ["sensitivityLabels"] });
      const setup = await backend.getProtectionSetup("acct-work");
      expect(setup).toMatchObject({ supported: true, labels: [], labelsListedAt: null });
      expect(setup.encrypt.state).toBe("notChecked");
      await expect(backend.refreshSensitivityLabels("acct-work")).rejects.toMatchObject({
        code: "permissionRequired",
        details: { feature: "sensitivityLabels", accountId: "acct-work" },
      });
    },
  );

  it("starts in the states ?protection names", async () => {
    const setup = await new MockBackend({ protection: "setup" }).getProtectionSetup("acct-work");
    expect(setup.encrypt).toMatchObject({ state: "applied", testMessage: true });
    expect(setup.doNotForward.state).toBe("applied");
    expect(
      Object.fromEntries(setup.labels.map((l) => [l.name, [l.check.state, l.check.note]])),
    ).toEqual({
      Public: ["unknown", "otherRule"],
      General: ["applied", null],
      Confidential: ["applied", null],
      "All Employees": ["applied", null],
      "Project Atlas": ["notApplied", null],
    });

    const expired = await new MockBackend({ protection: "expired" }).getProtectionSetup(
      "acct-work",
    );
    expect(expired.encrypt.state).toBe("expired");
    expect(Date.now() - Date.parse(expired.encrypt.at!)).toBeGreaterThan(90 * 86_400_000);
    expect(expired.doNotForward.state).toBe("applied");

    const checking = await new MockBackend({ protection: "checking" }).getProtectionSetup(
      "acct-work",
    );
    expect(checking).toMatchObject({ checking: true, encrypt: { state: "waiting" } });
  });

  it("checks: waiting, then what arrived by the second look, moved to Deleted Items", async () => {
    const backend = new MockBackend();
    const deleted = async () =>
      (await backend.listMessages("w-trash", { offset: 0, limit: 50 })).items.map((m) => m.subject);
    expect(await deleted()).toEqual([]);

    const waiting = await backend.checkProtection("acct-work", [encrypt, general]);
    expect(waiting.checking).toBe(true);
    expect(waiting.encrypt.state).toBe("waiting");
    expect(waiting.labels.find((l) => l.id === GENERAL)!.check.state).toBe("waiting");
    await expect(backend.checkProtection("acct-work", [encrypt])).rejects.toEqual({
      code: "invalidInput",
      message: "Wait for the check that's running to finish.",
    });
    await expect(backend.openProtectionCheckMessage("acct-work", encrypt)).rejects.toEqual({
      code: "notFound",
      message: "There's no test message to open. Check again.",
    });

    expect((await backend.refreshProtectionCheck("acct-work")).checking).toBe(true);
    const done = await backend.refreshProtectionCheck("acct-work");
    expect(done.checking).toBe(false);
    expect(done.encrypt).toMatchObject({ state: "applied", testMessage: true });
    expect(done.labels.find((l) => l.id === GENERAL)!.check.state).toBe("applied");
    // The control and the two test messages, with the same neutral subject.
    const subjects = await deleted();
    expect(subjects).toHaveLength(3);
    expect(new Set(subjects).size).toBe(1);
    expect(subjects[0]).toMatch(/^Cumulas Mail setup check \[[a-z0-9]{8}\]$/);

    await backend.openProtectionCheckMessage("acct-work", encrypt);
    expect(backend.calls.openedInOutlook).toEqual(["check:acct-work:encrypt"]);
  });

  it("keeps a pass while it's checked again, as the core does", async () => {
    const backend = new MockBackend({ protection: "setup" });
    const before = await backend.getProtectionSetup("acct-work");
    const waiting = await backend.checkProtection("acct-work", [encrypt, general]);
    expect(waiting.encrypt).toMatchObject({
      state: "waiting",
      testMessage: false,
      passedAt: before.encrypt.at,
    });
    // Still offered, and still enforced: a label is required.
    await expect(backend.sendMessage(message({ protect: "encrypt" }))).rejects.toMatchObject({
      details: { reason: "labelRequired" },
    });
    await expect(
      backend.sendMessage(message({ protect: "encrypt", labelId: GENERAL })),
    ).resolves.toBeUndefined();
  });

  it("refuses checks it can't send", async () => {
    const backend = new MockBackend();
    const invalid = (message: string) => ({ code: "invalidInput", message });
    await expect(backend.checkProtection("acct-work", [])).rejects.toEqual(
      invalid("Choose what to check."),
    );
    const many = Array.from({ length: 20 }, () => general);
    await expect(backend.checkProtection("acct-work", many)).rejects.toEqual(
      invalid("Cumulas Mail checks at most 19 at a time."),
    );
    await expect(
      backend.checkProtection("acct-work", [
        { kind: "label", labelId: "00000000-0000-0000-0000-000000000000" },
      ]),
    ).rejects.toEqual(invalid(`That sensitivity label isn't available for ${EMAIL}.`));
    expect((await backend.getProtectionSetup("acct-work")).checking).toBe(false);
    // Only work and school accounts have checks, as in the core.
    const unsupported = {
      code: "unsupported",
      message: "Only Microsoft 365 work and school accounts can send protected mail.",
    };
    await expect(backend.checkProtection("acct-home", [encrypt])).rejects.toEqual(unsupported);
    await expect(backend.openProtectionCheckMessage("acct-home", encrypt)).rejects.toEqual(
      unsupported,
    );
  });

  it("refuses to send what isn't set up, as the core does", async () => {
    const backend = new MockBackend({ protectedMail: true });
    const invalid = (text: string, details?: object) => ({
      code: "invalidInput",
      message: text,
      ...(details ? { details } : {}),
    });
    // Nothing checked: the label isn't required yet, and nothing can be asked for.
    await expect(backend.sendMessage(message())).resolves.toBeUndefined();
    await expect(backend.sendMessage(message({ protect: "encrypt" }))).rejects.toEqual(
      invalid(`Encrypt isn't set up for ${EMAIL}. Check it in Settings → Accounts.`),
    );
    await expect(backend.sendMessage(message({ labelId: GENERAL }))).rejects.toEqual(
      invalid(`The label General isn't set up for ${EMAIL}. Check it in Settings → Accounts.`),
    );
    await expect(backend.sendMessage(message({ labelId: "not-a-guid" }))).rejects.toEqual(
      invalid(`That sensitivity label isn't available for ${EMAIL}.`),
    );
    await expect(
      backend.sendMessage(
        message({
          accountId: "acct-home",
          to: [{ name: null, address: "b@x.example" }],
          protect: "encrypt",
        }),
      ),
    ).rejects.toEqual({
      code: "unsupported",
      message: "Only Microsoft 365 work and school accounts can send protected mail.",
    });
  });

  it("sends what passed its check, and the Sent copy says what was asked for", async () => {
    const backend = new MockBackend({ protection: "expired", protectedMail: true });
    const invalid = (text: string, details?: object) => ({
      code: "invalidInput",
      message: text,
      ...(details ? { details } : {}),
    });
    await expect(backend.sendMessage(message())).rejects.toEqual(
      invalid("Choose a sensitivity label. Your organization requires one.", {
        reason: "labelRequired",
      }),
    );
    await expect(
      backend.sendMessage(message({ labelId: GENERAL, protect: "encrypt" })),
    ).rejects.toEqual(
      invalid(
        "Encrypt was last checked more than 90 days ago. Check it again in Settings → Accounts.",
      ),
    );
    await expect(
      backend.sendMessage(message({ labelId: ALL_EMPLOYEES, protect: "doNotForward" })),
    ).rejects.toEqual(
      invalid(
        "The label Confidential › All Employees already encrypts this message. Choose either the label or Do Not Forward.",
      ),
    );
    // In the core's order: the label is judged before Encrypt's own (too old) check.
    await expect(
      backend.sendMessage(message({ labelId: ALL_EMPLOYEES, protect: "encrypt" })),
    ).rejects.toEqual(
      invalid(
        "The label Confidential › All Employees already encrypts this message. Choose either the label or Encrypt.",
      ),
    );
    await expect(backend.sendMessage(message({ labelId: ATLAS }))).rejects.toEqual(
      invalid(
        `The label Highly Confidential › Project Atlas isn't set up for ${EMAIL}. Check it in Settings → Accounts.`,
      ),
    );
    await expect(backend.sendMessage(message({ labelId: PUBLIC }))).rejects.toMatchObject({
      code: "invalidInput",
    });

    await backend.sendMessage(
      message({
        subject: "Atlas numbers",
        labelId: GENERAL.toUpperCase(),
        protect: "doNotForward",
      }),
    );
    expect(backend.sent.at(-1)).toMatchObject({ protect: "doNotForward" });
    const sent = (await backend.listMessages("w-sent", { offset: 0, limit: 50 })).items.find(
      (m) => m.subject === "Atlas numbers",
    )!;
    expect((await backend.getMessage(sent.id)).requested).toEqual({
      action: "doNotForward",
      label: { id: GENERAL, name: "General", color: "#2b7aa8", fromCatalog: true },
    });
  });

  it("hands replies to encrypted mail to Outlook on the web", async () => {
    const backend = new MockBackend({ protectedMail: true });
    await expect(
      backend.sendMessage(
        message({ replyToMessageId: "msg-protected-1", replyKind: "reply", subject: "Re: Q4" }),
      ),
    ).rejects.toEqual({
      code: "unsupported",
      message: "This message is protected. Reply to it or forward it in Outlook on the web.",
      details: {
        reason: "protectedOriginal",
        messageId: "msg-protected-1",
        protection: "rightsManaged",
      },
    });
    await expect(
      backend.sendMessage(
        message({
          accountId: "acct-home",
          replyToMessageId: "msg-protected-5",
          replyKind: "forward",
        }),
      ),
    ).rejects.toEqual({
      code: "unsupported",
      message:
        "This message is encrypted. Cumulas Mail can't reply to it or forward it without removing the encryption.",
      details: {
        reason: "protectedOriginal",
        messageId: "msg-protected-5",
        protection: "smimeEncrypted",
      },
    });
    // Signed mail can be answered.
    await expect(
      backend.sendMessage(message({ replyToMessageId: "msg-protected-4", replyKind: "reply" })),
    ).resolves.toBeUndefined();
  });

  it("opens messages in Outlook on the web, never a protection's own files", async () => {
    const backend = new MockBackend({ protectedMail: true });
    const forecast = await backend.getMessage("msg-protected-1");
    expect(forecast.protection).toEqual({
      kind: "rightsManaged",
      containerIds: ["msg-protected-1-att-0"],
      portalUrl: null,
      contentHidden: false,
    });
    expect(forecast.label).toEqual({
      id: ALL_EMPLOYEES,
      name: "Confidential › All Employees",
      color: "#b35f26",
      fromCatalog: true,
    });
    await expect(
      backend.openAttachment("msg-protected-1", "msg-protected-1-att-0"),
    ).rejects.toEqual({
      code: "unsupported",
      message:
        "This file holds a protected message's content, which Cumulas Mail can't open. Open the message in Outlook on the web.",
    });
    // Other files open as before.
    await expect(backend.openAttachment("msg-1", "msg-1-att-0")).resolves.toBeUndefined();

    await backend.openInOutlook("msg-protected-1");
    expect(backend.calls.openedInOutlook).toEqual(["msg-protected-1"]);
    await expect(backend.openInOutlook("msg-protected-5")).rejects.toEqual({
      code: "unsupported",
      message: "Only Microsoft 365 messages open in Outlook on the web.",
    });
  });

  it("describes a message's protection without its content", async () => {
    const backend = new MockBackend({ protectedMail: true });
    const text = await backend.getMessageDiagnostics("msg-protected-1");
    expect(text.split("\n").slice(0, 6)).toEqual([
      "Cumulas Mail 0.1.0-demo: protection details of one message",
      "These details don't include the message's subject, addresses, text, links, or the names of its files other than the protection's own.",
      "",
      "Account: Microsoft 365 (work or school)",
      "Folder: inbox",
      "Detected: Microsoft Purview encryption",
    ]);
    expect(text).toContain(
      "  - message.rpmsg · application/x-microsoft-rpmsg-message · 47 KB · protection file",
    );
    expect(text).toContain(`  - ${ALL_EMPLOYEES} · this organization: yes`);
    expect(text).not.toMatch(/Q4 forecast|Dana|dana\.whitfield/);

    const portal = await backend.getMessageDiagnostics("msg-protected-3");
    expect(portal).toContain("Portal link: yes (outlook.office365.com)");
    expect(portal).not.toContain("recipientemailaddress");
    // Every message has them, protected or not.
    expect(await backend.getMessageDiagnostics("msg-1")).toContain("Detected: nothing");
  });
});

describe("the demo's organization setup", () => {
  const ALL_EMPLOYEES = "7b2d4e61-9c3a-4e5f-8a7b-6c5d4e3f2a10";
  const WORK_ONLY = {
    code: "unsupported",
    message: "Only Microsoft 365 work and school accounts have an organization to set up.",
  };
  const NOT_AVAILABLE = {
    code: "notConfigured",
    message:
      "Automatic setup isn't available in this version of Cumulas Mail. Use the PowerShell script instead.",
  };
  const choices = {
    actions: true,
    encryptTemplate: null,
    doNotForwardTemplate: null,
    labelIds: [],
    labelsWithoutLicense: false,
  };

  it("has no setup app, as a build without one: only the script", async () => {
    const backend = new MockBackend();
    expect(await backend.getOrgSetupInfo("acct-work")).toEqual({
      automatic: false,
      tenantId: "demo-tenant",
      signedIn: null,
      approvalUrl: null,
      rulesChangedAt: null,
      policiesChangedAt: null,
    });
    await expect(backend.orgSetupSignIn("acct-work")).rejects.toEqual(NOT_AVAILABLE);
    await expect(backend.readOrgSetup("acct-work")).rejects.toEqual(NOT_AVAILABLE);
    await expect(backend.planOrgSetup("acct-work", choices)).rejects.toEqual(NOT_AVAILABLE);
    await expect(backend.applyOrgSetup("acct-work", choices, [])).rejects.toEqual(NOT_AVAILABLE);
    expect(await backend.getOrgSetupDetails()).toBe("");
    expect(await backend.getProtectionCheckDetails("acct-work")).toBe("");
    await backend.cancelOrgSetupSignIn();
    await backend.endOrgSetup();
    expect(backend.calls.orgSetupEnded).toBe(1);

    // Only work and school accounts have an organization.
    await expect(backend.getOrgSetupInfo("acct-home")).rejects.toEqual(WORK_ONLY);
    await expect(backend.orgSetupSignIn("acct-home")).rejects.toEqual(WORK_ONLY);
    await expect(
      backend.getOrgSetupScript("acct-home", { choices: null, actions: true, labels: false }),
    ).rejects.toEqual(WORK_ONLY);
  });

  it("fills in the script and the portal steps for the labels it covers", async () => {
    const backend = new MockBackend();
    const actionsOnly = await backend.getOrgSetupScript("acct-work", {
      choices: null,
      actions: true,
      labels: false,
    });
    expect(actionsOnly.script).toMatch(
      /^# Cumulas Mail: encryption and sensitivity labels for acme\.example \(tenant demo-tenant\)\./,
    );
    expect(actionsOnly.script).toContain("-Name 'Cumulas Mail: Encrypt'");
    expect(actionsOnly.script).not.toContain("Connect-IPPSSession");
    expect(actionsOnly.portalSteps).toEqual([]);

    const allLabels = await backend.getOrgSetupScript("acct-work", {
      choices: null,
      actions: false,
      labels: true,
    });
    expect(allLabels.portalSteps).toHaveLength(5);
    expect(allLabels.script).not.toContain("'Cumulas Mail: Encrypt'");
    expect(allLabels.script).toContain("Connect-IPPSSession");

    const chosen = await backend.getOrgSetupScript("acct-work", {
      choices: { ...choices, labelIds: [ALL_EMPLOYEES] },
      actions: true,
      labels: true,
    });
    expect(chosen.portalSteps).toEqual([
      {
        labelTitle: "Confidential › All Employees",
        policyName: "Cumulas Mail: All Employees",
        ruleName: "Cumulas Mail: All Employees (header)",
        pattern: `(?i)^(${ALL_EMPLOYEES})$`,
      },
    ]);
  });
});

describe("the demo's ranking and filters", () => {
  const subjects = (messages: { subject: string }[]) => messages.map((m) => m.subject);
  const PORTO = ["Porto offsite: tickets and hotel", "Porto itinerary", "Friday plans"];

  it("lists best matches first, or newest first on request", async () => {
    const backend = new MockBackend();
    expect(subjects(await backend.searchMessages("porto"))).toEqual(PORTO);
    expect(subjects(await backend.searchMessages("porto", undefined, [], null))).toEqual(PORTO);
    expect(
      subjects(await backend.searchMessages("porto", undefined, [], { sort: "newest" })),
    ).toEqual(["Friday plans", "Porto offsite: tickets and hotel", "Porto itinerary"]);
    // An order it doesn't know, as the core's command, it refuses.
    const unknown = { sort: "oldest" } as unknown as SearchOptions;
    await expect(backend.searchMessages("porto", undefined, [], unknown)).rejects.toMatchObject({
      code: "invalidInput",
    });
    // Addresses alone, or filters alone, have no words to rank by: newest first.
    for (const text of ["from:priya", "is:flagged", "has:attachment"]) {
      const found = await backend.searchMessages(text);
      expect(found.length, text).toBeGreaterThan(1);
      const dates = found.map((m) => m.receivedAt);
      expect(dates, text).toEqual([...dates].sort((a, b) => b.localeCompare(a)));
    }
  });

  it("applies every filter, to the server's hits too, and where to look", async () => {
    const backend = new MockBackend();
    const flagged = await backend.searchMessages("porto is:flagged");
    expect(subjects(flagged)).toEqual(["Porto offsite: tickets and hotel"]);
    expect(subjects(await backend.searchMessages("porto has:attachment is:read"))).toEqual([
      "Porto offsite: tickets and hotel",
    ]);
    expect(subjects(await backend.searchMessages("porto received:today"))).toEqual([]);
    expect(subjects(await backend.searchMessages("porto received:yesterday"))).toEqual([
      "Friday plans",
    ]);
    expect(subjects(await backend.searchMessages("porto before:yesterday"))).toEqual([
      "Porto offsite: tickets and hotel",
      "Porto itinerary",
    ]);
    // A reversed range: nothing, not even the server's hits.
    const [lisbon] = await backend.searchMessages("lisbon");
    expect(
      await backend.searchMessages("received:2026-10-15..2026-10-01", undefined, [lisbon!.id]),
    ).toEqual([]);
    // Server hits pass the filters too.
    expect(
      subjects(await backend.searchMessages("is:unread", undefined, [lisbon!.id])),
    ).not.toContain("Your trip to Lisbon: booking confirmed");
    expect(
      subjects(await backend.searchMessages("has:attachment", undefined, [lisbon!.id])),
    ).toContain("Your trip to Lisbon: booking confirmed");
    // Folders by role or name, accounts by address or name.
    expect(subjects(await backend.searchMessages("is:unread in:sent"))).toEqual([
      "Notes for the export endpoint fix",
    ]);
    expect(subjects(await backend.searchMessages("is:unread in:projects"))).toEqual([
      "Projects board: what moves next sprint",
    ]);
    expect(subjects(await backend.searchMessages("porto in:inbox"))).toEqual([]);
    expect(await backend.searchMessages("porto account:alex@morgan-family.example")).toEqual([]);
    expect(subjects(await backend.searchMessages('porto account:"Alex Morgan"'))).toEqual(PORTO);
    expect(await backend.searchMessages("porto account:nobody")).toEqual([]);
    // Where to look alone is nothing to look for.
    expect(await backend.searchMessages("in:inbox")).toEqual([]);
  });

  it("shows the words around a match in an opened message's text", async () => {
    const backend = new MockBackend();
    const [checklist] = await backend.searchMessages("subject:checklist");
    // Only its preview is on the device until it's opened.
    expect(await backend.searchMessages("retention")).toEqual([]);
    expect((await backend.searchMessages("checklist"))[0]!.snippet).toBeNull();
    await backend.getMessage(checklist!.id);
    const [hit] = await backend.searchMessages("retention");
    expect(hit!.snippet!.filter((r) => r.hit).map((r) => r.text)).toEqual(["retention"]);
    expect(hit!.snippet!.map((r) => r.text).join("")).toMatch(/data retention change/);
    // Newest first too; but no words to rank by, no snippet.
    const [newest] = await backend.searchMessages("retention", undefined, [], { sort: "newest" });
    expect(newest!.snippet).not.toBeNull();
    expect((await backend.searchMessages("from:priya"))[0]!.snippet).toBeNull();
  });

  it("asks the servers only for what narrows a search, and only where it may be", async () => {
    const backend = new MockBackend();
    expect(await backend.searchServer("s-1", "is:read", "acct-work")).toMatchObject({
      status: "done",
      messageIds: [],
      hasMore: false,
    });
    const flagged = await backend.searchServer("s-2", "is:flagged", "acct-work");
    expect(flagged.added).toBe(1);
    // The flagged mail only the server had, among what it found.
    expect(
      subjects(await backend.searchMessages("is:flagged", undefined, flagged.messageIds)),
    ).toContain("Signed partnership agreement with Fabrikam");
    expect(subjects(await backend.searchMessages("fabrikam"))).toEqual([
      "Signed partnership agreement with Fabrikam",
    ]);
    // Another account's address, or a folder the account hasn't: nothing.
    expect(
      await backend.searchServer("s-3", "invoice account:alex@morgan-family.example", "acct-work"),
    ).toMatchObject({ status: "done", messageIds: [] });
    expect(await backend.searchServer("s-4", "invoice in:nowhere", "acct-work")).toMatchObject({
      status: "done",
      messageIds: [],
    });
    expect(await backend.searchMessages("invoice")).toEqual([]);
    // Last year's mail with attachments, only the server has some.
    const older = await backend.searchServer(
      "s-5",
      'has:attachment received:"last year"',
      "acct-work",
    );
    expect(older.added).toBeGreaterThan(0);
    expect(
      subjects(
        await backend.searchMessages(
          'has:attachment received:"last year"',
          undefined,
          older.messageIds,
        ),
      ),
    ).toContain("Brand guidelines, final edition");
  });

  it("reads a page and then filters it, as the core does, so a call may find nothing yet", async () => {
    const backend = new MockBackend({ serverSearchPage: 1, serverSearchMaxPages: 1 });
    // Graph's $search can't say flagged: its newest match isn't, the next is.
    const first = await backend.searchServer("s-1", "porto is:flagged", "acct-work");
    expect(first).toMatchObject({ status: "done", messageIds: [], hasMore: true });
    const second = await backend.searchServer("s-1", "porto is:flagged", "acct-work", true);
    expect(second.messageIds).toHaveLength(1);
    expect(second.hasMore).toBe(true);
  });

  it("searches the one folder in: names on Graph, and every folder for several", async () => {
    const backend = new MockBackend({ serverSearchPage: 1, serverSearchMaxPages: 1 });
    // The Inbox alone: Graph has no porto there, and says so.
    expect(await backend.searchServer("s-1", "porto in:inbox", "acct-work")).toMatchObject({
      status: "done",
      messageIds: [],
      hasMore: false,
    });
    // Two folders: Graph looks everywhere, and what it finds outside them
    // is left out here, with more to look through.
    expect(await backend.searchServer("s-2", "porto in:inbox in:sent", "acct-work")).toMatchObject({
      status: "done",
      messageIds: [],
      hasMore: true,
    });
    const archived = await backend.searchServer("s-3", "porto in:archive", "acct-work");
    expect(archived.messageIds).toHaveLength(1);
  });

  it("suggests the people who wrote most lately for an empty text", async () => {
    const backend = new MockBackend();
    const people = await backend.suggestAddresses("");
    expect(people.length).toBeGreaterThan(0);
    expect(people.length).toBeLessThanOrEqual(8);
    const addresses = people.map((p) => p.address);
    expect(addresses).not.toContain("alex.morgan@acme.example");
    expect(addresses).not.toContain("alex@morgan-family.example");
    expect(new Set(addresses).size).toBe(addresses.length);
    expect(await backend.suggestAddresses("   ")).toEqual(people);
  });
});
