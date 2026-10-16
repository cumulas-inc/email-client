import { afterEach, describe, expect, it, onTestFinished, vi } from "vitest";

import type {
  ActiveReminder,
  BackendError,
  DataChangedEvent,
  RemindersChangedEvent,
} from "../types";
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
    const found = await backend.searchMessages("lisbon", undefined, [
      sunday!.id,
      lisbon!.id,
      "msg-gone",
    ]);
    expect(subjects(found)).toEqual(["Sunday lunch", "Your trip to Lisbon: booking confirmed"]);
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
    await backend.cancelServerSearch("s-2"); // Not the current one: nothing happens.
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
