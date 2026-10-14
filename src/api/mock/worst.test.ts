import { MockBackend } from ".";
import { createWorstState } from "./worst";

const length = (s: string) => [...s].length;

describe("worst-case demo data", () => {
  it("is deterministic", () => {
    const names = () => {
      const s = createWorstState();
      return [
        ...s.messages.map((m) => m.subject),
        ...s.contacts.map((c) => c.displayName),
        ...s.tasks.map((t) => `${t.title} ${t.dueDate}`),
      ];
    };
    expect(names()).toEqual(names());
  });

  it("has three extreme accounts, one with a long sync error", () => {
    const { accounts } = createWorstState();
    expect(accounts).toHaveLength(3);
    const [long, single, japanese] = accounts;
    expect(long!.email.length).toBeGreaterThanOrEqual(60);
    expect(long!.email).not.toMatch(/[\s/-]/);
    expect(single!.displayName).toHaveLength(1);
    expect(japanese!.displayName).toMatch(/\p{Script=Han}/u);
    expect(accounts.some((a) => (a.syncError?.length ?? 0) > 200)).toBe(true);
  });

  it("covers the mail extremes", async () => {
    const backend = new MockBackend({ worst: true });
    const folders = await backend.listFolders();
    const counts = folders.map((f) => f.unreadCount);
    expect(counts).toEqual(expect.arrayContaining([12_345, 1_000, 999]));
    expect(folders.some((f) => length(f.name) === 60)).toBe(true);
    expect(folders.some((f) => f.role === "other" && f.totalCount === 0)).toBe(true);
    let depth = 0;
    for (const f of folders) {
      let d = 0;
      for (let p = f.parentId; p; p = folders.find((x) => x.id === p)?.parentId ?? null) d++;
      depth = Math.max(depth, d);
    }
    expect(depth).toBeGreaterThanOrEqual(7);

    const { messages } = createWorstState();
    expect(messages.some((m) => m.to.length + m.cc.length === 40)).toBe(true);
    const many = messages.find((m) => m.attachments.length === 12)!;
    expect(many.attachments.some((a) => a.size === 0)).toBe(true);
    expect(messages.some((m) => m.subject === "")).toBe(true);
    expect(messages.some((m) => length(m.from?.name ?? "") === 120)).toBe(true);
    expect(messages.some((m) => m.from && m.from.name === null)).toBe(true);
    expect(messages.some((m) => /https:\/\/\S{296,}/.test(m.bodyText ?? ""))).toBe(true);
    expect(messages.some((m) => m.bodyHtml?.includes('width="900"'))).toBe(true);
    expect(messages.some((m) => m.receivedAt.startsWith("1999"))).toBe(true);
    expect(messages.some((m) => new Date(m.receivedAt) > new Date(Date.now() + 86_400_000))).toBe(
      true,
    );
  });

  it("covers the calendar, contacts and tasks extremes", () => {
    const s = createWorstState();
    expect(s.events.some((e) => e.attendees.length >= 60)).toBe(true);
    expect(s.events.some((e) => e.subject.split(/\s+/).length === 40)).toBe(true);
    const sameStart = new Map<string, number>();
    for (const e of s.events) sameStart.set(e.start, (sameStart.get(e.start) ?? 0) + 1);
    expect(Math.max(...sameStart.values())).toBeGreaterThanOrEqual(7);
    expect(s.contacts).toHaveLength(300);
    expect(s.contacts.some((c) => c.emails.length === 10 && c.phones.length === 10)).toBe(true);
    expect(s.contacts.some((c) => length(c.displayName) === 100)).toBe(true);
    expect(s.contacts.some((c) => (c.notes?.length ?? 0) === 2_000)).toBe(true);
    const perList = (id: string) => s.tasks.filter((t) => t.listId === id);
    expect(perList("wtl-tasks")).toHaveLength(200);
    expect(perList("wtl-empty")).toHaveLength(0);
    expect(perList("wtl-done").every((t) => t.status === "completed")).toBe(true);
    expect(s.tasks.some((t) => length(t.title) === 200)).toBe(true);
  });

  it("leaves the default demo alone", () => {
    const backend = new MockBackend();
    return backend.listAccounts().then((accounts) => {
      expect(accounts.map((a) => a.displayName)).toEqual(["Alex Morgan", "Alex (Personal)"]);
    });
  });
});
