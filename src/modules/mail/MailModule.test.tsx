import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { MockBackend } from "../../api/mock";
import type { Settings } from "../../api/types";
import { useAppStore } from "../../app/store";
import { useAddAccountRequest } from "../settings/addAccountStore";
import { messageBridgeUrl } from "./bodyHtml";
import { useMailStore } from "./mailStore";
import {
  deferred,
  folderTree,
  renderMail,
  respondButton,
  row,
  setSearch,
  toolbarButton,
} from "./testUtils";

// Full-module tests in jsdom; allow for slow, busy CI machines.
vi.setConfig({ testTimeout: 20_000 });

const CHECKLIST = /Atlas launch checklist — final review today/;

function treeItem(name: RegExp) {
  return within(folderTree()).getByRole("treeitem", { name });
}

async function openMessage(user: ReturnType<typeof userEvent.setup>, name: RegExp) {
  await user.click(row(name));
  return screen.findByRole("heading", { level: 1, name });
}

/** The open message's frame, and the channel its bridge reports on. */
async function messageFrame(title: RegExp) {
  const frame = (await screen.findByTitle(title)) as HTMLIFrameElement;
  const channel = /data-channel="([0-9a-f]+)"/.exec(frame.getAttribute("srcdoc") ?? "")?.[1];
  expect(channel).toMatch(/^[0-9a-f]{32}$/);
  return { frame, channel: channel! };
}

/** A message from the bridge, as if posted by `source` (the frame's own window). */
function post(data: unknown, source: MessageEventSource | null) {
  act(() => {
    window.dispatchEvent(new MessageEvent("message", { data, source }));
  });
}

/** How many new documents `frame` has been given since this was called. */
function documentsGiven(frame: HTMLIFrameElement): () => number {
  let given = 0;
  const observer = new MutationObserver((records) => (given += records.length));
  observer.observe(frame, { attributeFilter: ["srcdoc"] });
  return () => (given += observer.takeRecords().length);
}

describe("Mail folders", () => {
  it("shows favorites and account trees with unread badges, and switches folders", async () => {
    const user = userEvent.setup();
    await renderMail();

    // Favorites: unified inbox plus each account's inbox.
    expect(treeItem(/^All inboxes, 5 unread$/)).toBeInTheDocument();
    expect(treeItem(/^Inbox \(Alex Morgan\), 3 unread$/)).toHaveAttribute("aria-selected", "true");
    expect(treeItem(/^Inbox \(Alex \(Personal\)\), 2 unread$/)).toBeInTheDocument();
    // Account tree with nested folders and counts (drafts count items).
    const account = treeItem(/^Alex Morgan \(alex\.morgan@acme\.example\)$/);
    expect(within(account).getByRole("treeitem", { name: "Junk Email, 1 unread" })).toBeVisible();
    expect(within(account).getByRole("treeitem", { name: "Drafts, 1 item" })).toBeVisible();
    const projects = within(account).getByRole("treeitem", { name: "Projects" });
    expect(projects).toHaveAttribute("aria-expanded", "true");
    expect(within(projects).getByRole("treeitem", { name: "Atlas" })).toBeInTheDocument();

    await user.click(within(projects).getByText("Atlas"));
    expect(await screen.findByRole("listbox", { name: "Messages in Atlas" })).toBeInTheDocument();
    expect(row(/Atlas naming research summary/)).toBeInTheDocument();
    expect(screen.queryByRole("option", { name: CHECKLIST })).not.toBeInTheDocument();

    // Collapse a subtree.
    await user.click(
      within(projects)
        .getAllByText("Projects")[0]!
        .parentElement!.querySelector(".mail-tree__chevron")!,
    );
    expect(projects).toHaveAttribute("aria-expanded", "false");
  });

  it("supports keyboard navigation in the folder tree", async () => {
    const user = userEvent.setup();
    await renderMail();
    const selected = treeItem(/^Inbox \(Alex Morgan\)/);
    act(() => selected.focus());
    await user.keyboard("{ArrowDown}{Enter}");
    expect(await screen.findByRole("listbox", { name: "Messages in Inbox" })).toBeInTheDocument();
    expect(useMailStore.getState().folderId).toBe("h-inbox");
  });

  it("shows the unified inbox across accounts", async () => {
    const user = userEvent.setup();
    await renderMail();
    await user.click(within(treeItem(/^All inboxes/)).getByText("All inboxes"));
    const list = await screen.findByRole("listbox", { name: "Messages in All inboxes" });
    expect(within(list).getByRole("option", { name: /Sunday lunch/ })).toBeInTheDocument();
    expect(within(list).getByRole("option", { name: CHECKLIST })).toBeInTheDocument();
  });
});

describe("Message list", () => {
  it("filters by unread and flagged", async () => {
    const user = userEvent.setup();
    await renderMail();
    expect(row(/Lunch on Thursday/)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Unread" }));
    await waitFor(() =>
      expect(screen.queryByRole("option", { name: /Lunch on Thursday/ })).not.toBeInTheDocument(),
    );
    const unread = screen.getAllByRole("option");
    expect(unread).toHaveLength(3);
    unread.forEach((o) => expect(o.getAttribute("aria-label")).toMatch(/^Unread/));

    await user.click(screen.getByRole("button", { name: "Flagged" }));
    await waitFor(() => expect(row(/Quarterly planning/)).toBeInTheDocument());
    screen
      .getAllByRole("option")
      .forEach((o) => expect(o.getAttribute("aria-label")).toMatch(/Flagged/));
  });

  it("pages through long folders", async () => {
    const user = userEvent.setup();
    await renderMail();
    expect(screen.getAllByRole("option")).toHaveLength(50);
    await user.click(screen.getByRole("button", { name: "Load more messages" }));
    await waitFor(() => expect(screen.getAllByRole("option")).toHaveLength(76));
    expect(screen.queryByRole("button", { name: "Load more messages" })).not.toBeInTheDocument();
  });

  it("groups messages by date", async () => {
    await renderMail();
    expect(screen.getByRole("group", { name: "Today" })).toBeInTheDocument();
    expect(screen.getByRole("group", { name: "Yesterday" })).toBeInTheDocument();
  });

  it("selects several messages with Ctrl and Shift and acts on the selection", async () => {
    const user = userEvent.setup();
    const { backend } = await renderMail();
    await user.click(row(CHECKLIST));
    fireEvent.click(row(/Customer announcement draft/), { shiftKey: true });
    const bar = await screen.findByRole("toolbar", { name: "Selected messages" });
    expect(within(bar).getByText("3 selected")).toBeInTheDocument();
    fireEvent.click(row(/Lunch on Thursday/), { ctrlKey: true });
    expect(within(bar).getByText("4 selected")).toBeInTheDocument();
    expect(screen.getByText("4 items selected")).toBeInTheDocument();

    await user.click(within(bar).getByRole("button", { name: "Mark as unread" }));
    await waitFor(async () => {
      const page = await backend.listMessages("w-inbox", { offset: 0, limit: 5, unreadOnly: true });
      expect(page.total).toBe(4);
    });
  });
});

describe("Reading pane", () => {
  it("marks the message read and renders the body in a sandboxed frame", async () => {
    const user = userEvent.setup();
    const { backend } = await renderMail();
    expect(row(CHECKLIST).getAttribute("aria-label")).toMatch(/^Unread/);

    await openMessage(user, CHECKLIST);
    await waitFor(() => expect(row(CHECKLIST).getAttribute("aria-label")).not.toMatch(/^Unread/));
    expect(treeItem(/^Inbox \(Alex Morgan\), 2 unread$/)).toBeInTheDocument();
    expect((await backend.getMessage("msg-1")).isRead).toBe(true);

    const frame = await screen.findByTitle(/^Message: Atlas launch checklist/);
    expect(frame.tagName).toBe("IFRAME");
    // An opaque origin: the bridge runs, nothing in the frame reaches the app.
    expect(frame.getAttribute("sandbox")).toBe("allow-scripts");
    const srcdoc = frame.getAttribute("srcdoc") ?? "";
    expect(srcdoc).toContain('<meta http-equiv="Content-Security-Policy"');
    expect(srcdoc).toContain("default-src 'none'");
    expect(srcdoc).toContain(`script-src ${messageBridgeUrl()};`);
    expect(srcdoc).toMatch(
      new RegExp(`<script src="${messageBridgeUrl()}" data-channel="[0-9a-f]{32}"></script>`),
    );
    expect(srcdoc).toContain("img-src data: cid:;");
    expect(srcdoc).not.toMatch(/img-src[^;]*https:/);
    expect(srcdoc).toContain("rollback plan");

    // Header: sender, recipients, attachment card.
    expect(screen.getByText("<priya.raman@acme.example>")).toBeInTheDocument();
    expect(within(screen.getByRole("article")).getByText("Jonas Becker")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Open Atlas-launch-checklist-v7.docx" }),
    ).toBeVisible();

    // High importance reads "Important", in the row and on the letter.
    expect(row(CHECKLIST).getAttribute("aria-label")).toContain(", Important");
    expect(within(screen.getByRole("article")).getByText("Important")).toBeInTheDocument();

    // Quick reply: the user's avatar in the account's color, as in the header.
    const quick = screen.getByRole("button", { name: /Reply to Priya Raman/ });
    const avatar = quick.querySelector<HTMLElement>(".avatar")!;
    expect(avatar).toHaveClass("avatar--account");
    expect(avatar.style.getPropertyValue("--account-color")).toBe("#0b7a75");
    expect(avatar).toHaveTextContent("AM");
    // A sender who's someone else keeps the hue of their address.
    expect(
      within(screen.getByRole("article")).getByText("PR", { selector: ".avatar" }),
    ).not.toHaveClass("avatar--account");
  });

  it("shows the user's own sent mail with their account-colored avatar", async () => {
    const user = userEvent.setup();
    await renderMail();
    const account = treeItem(/^Alex Morgan \(alex\.morgan@acme\.example\)$/);
    await user.click(within(account).getByText("Sent Items"));
    await openMessage(user, /Load test results/);
    // The sender and the quick reply: both the user, both in the account's color.
    const avatars = within(screen.getByRole("article")).getAllByText("AM", { selector: ".avatar" });
    expect(avatars).toHaveLength(2);
    for (const avatar of avatars) {
      expect(avatar).toHaveClass("avatar--account");
      expect(avatar.style.getPropertyValue("--account-color")).toBe("#0b7a75");
    }
  });

  it("doesn't mark as read when the setting is 'never'", async () => {
    const user = userEvent.setup();
    const { backend } = await renderMail({ settings: { markReadAfterSeconds: null } });
    await openMessage(user, CHECKLIST);
    await new Promise((r) => setTimeout(r, 50));
    expect((await backend.getMessage("msg-1")).isRead).toBe(false);
  });

  it("marks as read after the configured delay", async () => {
    const user = userEvent.setup();
    const { backend } = await renderMail({ settings: { markReadAfterSeconds: 1 } });
    await openMessage(user, CHECKLIST);
    expect((await backend.getMessage("msg-1")).isRead).toBe(false);
    await waitFor(async () => expect((await backend.getMessage("msg-1")).isRead).toBe(true), {
      timeout: 2500,
    });
  });

  it("shows blocked content only when asked", async () => {
    const user = userEvent.setup();
    await renderMail();
    await openMessage(user, /GNOME 51 lands/);
    const frame = await screen.findByTitle(/^Message: This week: GNOME 51/);
    expect(frame.getAttribute("srcdoc")).not.toContain("images.linuxweekly.example");
    expect(screen.getByText(/images and content from the internet were blocked/)).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Show blocked content" }));
    await waitFor(() =>
      expect(screen.getByTitle(/^Message: This week: GNOME 51/).getAttribute("srcdoc")).toContain(
        "https://images.linuxweekly.example/header.png",
      ),
    );
    const srcdoc = screen.getByTitle(/^Message: This week/).getAttribute("srcdoc") ?? "";
    expect(srcdoc).toContain("img-src data: cid: https: http:");
    expect(screen.queryByRole("button", { name: "Show blocked content" })).not.toBeInTheDocument();
  });

  it("renders plain-text bodies escaped and linkified", async () => {
    const user = userEvent.setup();
    const { backend } = await renderMail();
    const opened: string[] = [];
    backend.openLink = async (url: string) => {
      opened.push(url);
    };
    await user.click(within(treeItem(/^Inbox \(Alex \(Personal\)\)/)).getByText("Inbox"));
    await screen.findByRole("option", { name: /Climbing on Thursday/ });
    await openMessage(user, /Climbing on Thursday/);
    const link = screen.getByRole("link", { name: "https://riversideclimbing.example/routes" });
    await user.click(link);
    expect(opened).toEqual(["https://riversideclimbing.example/routes"]);
    // An address opens the composer instead.
    await user.click(screen.getByRole("link", { name: "sam.okafor@acme.example" }));
    const composer = await screen.findByRole("region", { name: "New message" });
    expect(within(composer).getByText("sam.okafor@acme.example")).toBeInTheDocument();
  });

  it("follows links, shows where they go, sizes the frame and hears keys through its bridge", async () => {
    const user = userEvent.setup();
    const { backend } = await renderMail();
    const opened: string[] = [];
    backend.openLink = async (url: string) => {
      opened.push(url);
    };
    await openMessage(user, CHECKLIST);
    const { frame, channel } = await messageFrame(/^Message: Atlas launch checklist/);
    const fromFrame = (data: object) => post({ channel, ...data }, frame.contentWindow);

    // A web link opens in the browser; other schemes go nowhere.
    fromFrame({ type: "link", href: "https://example.org/plan" });
    fromFrame({ type: "link", href: "javascript:alert(1)" });
    fromFrame({ type: "link", href: "file:///etc/passwd" });
    await waitFor(() => expect(opened).toEqual(["https://example.org/plan"]));

    // Only this frame's messages count, on this document's channel, in shape.
    post({ channel, type: "link", href: "https://example.org/a" }, window);
    post({ channel, type: "link", href: "https://example.org/b" }, null);
    post(
      { channel: "0".repeat(32), type: "link", href: "https://example.org/c" },
      frame.contentWindow,
    );
    post({ type: "link", href: "https://example.org/d" }, frame.contentWindow);
    fromFrame({ type: "link", href: ["https://example.org/e"] });
    fromFrame({ type: "open", href: "https://example.org/f" });
    post("https://example.org/g", frame.contentWindow);
    fromFrame({ type: "size", height: Number.NaN, wide: false, left: false, right: false });
    fromFrame({ type: "size", height: 300 });
    await new Promise((r) => setTimeout(r, 20));
    expect(opened).toEqual(["https://example.org/plan"]);
    expect(frame.style.height).toBe("80px");

    // Hovering a link shows where it really goes.
    fromFrame({ type: "hover", href: "https://evil.example/login" });
    expect(screen.getByText("https://evil.example/login")).toHaveClass("mail-reader__status");
    fromFrame({ type: "hover", href: null });
    expect(screen.queryByText("https://evil.example/login")).not.toBeInTheDocument();

    // The frame is as tall as the mail, with room for a scrollbar when it's
    // wider than the pane, and a faded edge where there's more.
    fromFrame({ type: "size", height: 412.3, wide: true, left: false, right: true });
    expect(frame.style.height).toBe("429px");
    const wide = frame.closest(".mail-body-wide")!;
    expect(wide).toHaveAttribute("data-more-right");
    expect(wide).not.toHaveAttribute("data-more-left");
    fromFrame({ type: "size", height: 3, wide: false, left: false, right: false });
    expect(frame.style.height).toBe("24px");
    expect(wide).not.toHaveAttribute("data-more-right");

    // A link within the message scrolls the reader to its target.
    const scroller = frame.closest<HTMLElement>(".mail-reader__scroll")!;
    scroller.style.overflowY = "auto";
    Object.defineProperty(scroller, "scrollHeight", { configurable: true, value: 2000 });
    Object.defineProperty(scroller, "clientHeight", { configurable: true, value: 500 });
    scroller.scrollBy = vi.fn() as typeof scroller.scrollBy;
    fromFrame({ type: "scroll", top: 640 });
    expect(scroller.scrollBy).toHaveBeenCalledWith({ top: 640 });

    // A mail address starts a message to it.
    fromFrame({ type: "link", href: "mailto:sam.okafor@acme.example?subject=Plan" });
    const composer = await screen.findByRole("region", { name: "New message" });
    expect(within(composer).getByText("sam.okafor@acme.example")).toBeInTheDocument();
    expect(opened).toEqual(["https://example.org/plan"]);
  });

  it("acts on shortcuts pressed in the message's frame", async () => {
    const user = userEvent.setup();
    await renderMail();
    await openMessage(user, CHECKLIST);
    const first = await messageFrame(/^Message: Atlas launch checklist/);
    const key = (frame: HTMLIFrameElement, channel: string, init: Partial<KeyboardEventInit>) =>
      post(
        {
          channel,
          type: "key",
          code: "",
          ctrlKey: false,
          shiftKey: false,
          altKey: false,
          metaKey: false,
          ...init,
        },
        frame.contentWindow,
      );

    // J: the next message, in the same frame with a new document.
    key(first.frame, first.channel, { key: "j", code: "KeyJ" });
    await screen.findByRole("heading", { level: 1, name: /Scheduled maintenance/ });
    const next = await messageFrame(/^Message: Scheduled maintenance/);
    expect(next.channel).not.toBe(first.channel);

    // The last document's messages no longer count.
    key(next.frame, first.channel, { key: "r", code: "KeyR", ctrlKey: true });
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByRole("region", { name: "Reply" })).not.toBeInTheDocument();

    key(next.frame, next.channel, { key: "r", code: "KeyR", ctrlKey: true });
    const composer = await screen.findByRole("region", { name: "Reply" });
    expect(within(composer).getByDisplayValue(/^Re: Scheduled maintenance/)).toBeInTheDocument();
  });

  // A reading font other than the default: the frame's font is built from
  // the settings, and the frame keeps its document while the reader changes
  // around it (a new one would reload it, and change its channel).
  describe.each<[string, Settings["readingFont"], string]>([
    ["a chosen family", { kind: "family", family: "Literata" }, 'font-family: "Literata"'],
    ["the interface font", { kind: "interface" }, '@font-face { font-family: "Inter Variable"'],
  ])("in %s", (_, readingFont, inFrame) => {
    it("keeps the message's document while the reader changes around it", async () => {
      const user = userEvent.setup();
      await renderMail({ settings: { readingFont } });
      await openMessage(user, CHECKLIST);
      const title = /^Message: Atlas launch checklist/;
      await waitFor(() =>
        expect(screen.getByTitle(title).getAttribute("srcdoc")).toContain(inFrame),
      );
      const { frame, channel } = await messageFrame(title);
      const srcdoc = frame.getAttribute("srcdoc");
      const given = documentsGiven(frame);
      const fromFrame = (data: object) => post({ channel, ...data }, frame.contentWindow);

      // Pointing at a link shows where it goes, and keeps showing it.
      fromFrame({ type: "hover", href: "https://evil.example/login" });
      await new Promise((r) => setTimeout(r, 20));
      expect(screen.getByText("https://evil.example/login")).toHaveClass("mail-reader__status");

      // The reader renders again and again: the frame's size, another link.
      fromFrame({ type: "size", height: 300, wide: false, left: false, right: false });
      fromFrame({ type: "hover", href: "https://example.org/plan" });
      fromFrame({ type: "size", height: 412, wide: true, left: false, right: true });
      await new Promise((r) => setTimeout(r, 20));
      expect(screen.getByText("https://example.org/plan")).toHaveClass("mail-reader__status");
      expect(frame.style.height).toBe("428px");
      expect(screen.getByTitle(title)).toBe(frame);
      expect(frame.getAttribute("srcdoc")).toBe(srcdoc);
      expect(given()).toBe(0);
    });

    it("keeps the original's document under a reply as it's written", async () => {
      const user = userEvent.setup();
      await renderMail({ settings: { readingFont } });
      await openMessage(user, CHECKLIST);
      await user.click(respondButton("Reply"));
      const composer = await screen.findByRole("region", { name: "Reply" });
      // Focused once it's there: typing goes to the subject after that.
      await within(composer).findByRole("textbox", { name: "Message body" });
      await waitFor(() =>
        expect(within(composer).getByTitle("Original message").getAttribute("srcdoc")).toContain(
          inFrame,
        ),
      );
      const quote = within(composer).getByTitle("Original message") as HTMLIFrameElement;
      const srcdoc = quote.getAttribute("srcdoc");
      const given = documentsGiven(quote);

      await user.type(within(composer).getByRole("textbox", { name: "Subject" }), " (agreed)");
      expect(within(composer).getByRole("textbox", { name: "Subject" })).toHaveValue(
        "Re: Atlas launch checklist — final review today (agreed)",
      );
      expect(within(composer).getByTitle("Original message")).toBe(quote);
      expect(quote.getAttribute("srcdoc")).toBe(srcdoc);
      expect(given()).toBe(0);
    });
  });

  it("opens and saves attachments", async () => {
    const user = userEvent.setup();
    await renderMail();
    await openMessage(user, CHECKLIST);
    await user.click(screen.getByRole("button", { name: "Save Atlas-launch-checklist-v7.docx" }));
    expect(
      await screen.findByText("Saved to /home/alex/Downloads/Atlas-launch-checklist-v7.docx"),
    ).toBeInTheDocument();
  });

  it("expands the recipient list", async () => {
    const user = userEvent.setup();
    await renderMail();
    await openMessage(user, /Quarterly planning/);
    await user.click(screen.getByRole("button", { name: "Show all recipients" }));
    expect(screen.getByText("Engineering Leads <eng-leads@acme.example>")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Hide recipient details" })).toHaveAttribute(
      "aria-expanded",
      "true",
    );
  });

  it("gives the list 45% of the height with the reading pane at the bottom", async () => {
    // jsdom has no layout: the area below the header is 840px tall.
    const height = vi.spyOn(Element.prototype, "clientHeight", "get").mockImplementation(function (
      this: Element,
    ) {
      return this.classList.contains("mail__main") ? 840 : 0;
    });
    try {
      await renderMail({ settings: { readingPane: "bottom" } });
      const root = document.querySelector<HTMLElement>(".mail--bottom")!;
      expect(root.style.getPropertyValue("--mail-list-h")).toBe("378px");
      const splitter = screen.getByRole("separator", { name: "Resize message list height" });
      expect(splitter).toHaveAttribute("aria-valuenow", "378");
      // The default isn't stored, so it can follow the window (and change later).
      expect(window.localStorage.getItem("mail.bottomListHeight")).toBeNull();

      // Once resized, the list keeps the user's height.
      fireEvent.keyDown(splitter, { key: "ArrowDown" });
      expect(root.style.getPropertyValue("--mail-list-h")).toBe("394px");
      expect(window.localStorage.getItem("mail.bottomListHeight")).toBe("394");
    } finally {
      height.mockRestore();
    }
  });

  it("shows the message full width with a back button when the reading pane is off", async () => {
    const user = userEvent.setup();
    await renderMail({ settings: { readingPane: "off" } });
    expect(screen.queryByText("No message open")).not.toBeInTheDocument();
    await openMessage(user, CHECKLIST);
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Back to message list" }));
    expect(await screen.findByRole("listbox", { name: "Messages in Inbox" })).toBeInTheDocument();
  });
});

describe("Optimistic actions", () => {
  it("flags immediately and keeps the flag when the server agrees", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const gate = deferred();
    const original = backend.setFlagged.bind(backend);
    backend.setFlagged = async (ids, flagged) => {
      await gate.promise;
      return original(ids, flagged);
    };
    await renderMail({ backend });
    const target = row(/Lunch on Thursday/);
    expect(target.getAttribute("aria-label")).not.toMatch(/Flagged/);
    await user.click(within(target).getByRole("button", { name: "Flag" }));
    // Updated before the backend answered.
    expect(row(/Lunch on Thursday/).getAttribute("aria-label")).toMatch(/Flagged/);
    gate.resolve();
    await waitFor(async () => expect((await backend.getMessage("msg-8")).isFlagged).toBe(true));
    expect(row(/Lunch on Thursday/).getAttribute("aria-label")).toMatch(/Flagged/);
  });

  it("rolls back a failed flag and reports the error", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    backend.setFlagged = async () => {
      throw { code: "network", message: "The server could not be reached." };
    };
    await renderMail({ backend });
    await user.click(within(row(/Lunch on Thursday/)).getByRole("button", { name: "Flag" }));
    expect(await screen.findByText("The server could not be reached.")).toBeInTheDocument();
    expect(row(/Lunch on Thursday/).getAttribute("aria-label")).not.toMatch(/Flagged/);
  });

  it("deletes optimistically, selects the next message and offers undo", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const gate = deferred();
    const original = backend.deleteMessages.bind(backend);
    backend.deleteMessages = async (ids) => {
      await gate.promise;
      return original(ids);
    };
    await renderMail({ backend });
    await openMessage(user, /Scheduled maintenance/);
    await user.click(toolbarButton("Delete"));

    expect(screen.queryByRole("option", { name: /Scheduled maintenance/ })).not.toBeInTheDocument();
    // The next message opens.
    expect(row(/Customer announcement draft/)).toHaveAttribute("aria-selected", "true");
    expect(
      await screen.findByRole("heading", { level: 1, name: /Customer announcement draft/ }),
    ).toBeInTheDocument();
    expect(treeItem(/^Deleted Items/)).toBeInTheDocument();

    gate.resolve();
    const undo = await screen.findByRole("button", { name: "Undo" });
    expect(screen.getByText("Deleted 1 item")).toBeInTheDocument();
    expect((await backend.getMessage("msg-2")).folderId).toBe("w-trash");
    await user.click(undo);
    await waitFor(async () => expect((await backend.getMessage("msg-2")).folderId).toBe("w-inbox"));
    expect(
      await screen.findByRole("option", { name: /Scheduled maintenance/ }),
    ).toBeInTheDocument();
  });

  it("restores a message when deleting fails", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    backend.deleteMessages = async () => {
      throw { code: "provider", message: "Deleting failed on the server." };
    };
    await renderMail({ backend });
    await user.click(within(row(/Lunch on Thursday/)).getByRole("button", { name: "Delete" }));
    expect(await screen.findByText("Deleting failed on the server.")).toBeInTheDocument();
    expect(row(/Lunch on Thursday/)).toBeInTheDocument();
  });

  it("moves a message with the Move to menu", async () => {
    const user = userEvent.setup();
    const { backend } = await renderMail();
    await openMessage(user, /Lunch on Thursday/);
    await user.click(toolbarButton("Move to"));
    await user.click(screen.getByRole("menuitem", { name: "Projects / Atlas" }));
    expect(screen.queryByRole("option", { name: /Lunch on Thursday/ })).not.toBeInTheDocument();
    await waitFor(async () => expect((await backend.getMessage("msg-8")).folderId).toBe("w-atlas"));
    expect(await screen.findByText("Moved 1 item to Atlas")).toBeInTheDocument();
  });

  it("archives with undo", async () => {
    const user = userEvent.setup();
    const { backend } = await renderMail();
    await openMessage(user, /Lunch on Thursday/);
    await user.click(toolbarButton("Archive"));
    await waitFor(async () =>
      expect((await backend.getMessage("msg-8")).folderId).toBe("w-archive"),
    );
    await user.click(await screen.findByRole("button", { name: "Undo" }));
    await waitFor(async () => expect((await backend.getMessage("msg-8")).folderId).toBe("w-inbox"));
  });

  it("moves messages dragged onto a folder", async () => {
    const { backend } = await renderMail();
    const source = row(/Lunch on Thursday/);
    const account = treeItem(/^Alex Morgan \(/);
    const archive = within(account).getByRole("treeitem", { name: "Archive" });
    const target = archive.querySelector<HTMLElement>(".mail-tree__row")!;
    const dataTransfer = {
      setData: () => undefined,
      setDragImage: () => undefined,
      dropEffect: "",
    };
    fireEvent.dragStart(source, { dataTransfer });
    fireEvent.dragOver(target, { dataTransfer });
    fireEvent.drop(target, { dataTransfer });
    fireEvent.dragEnd(source, { dataTransfer });
    await waitFor(async () =>
      expect((await backend.getMessage("msg-8")).folderId).toBe("w-archive"),
    );
  });
});

describe("Keyboard", () => {
  it("navigates with arrows and j/k, and acts with shortcuts", async () => {
    const user = userEvent.setup();
    const { backend } = await renderMail();
    await openMessage(user, CHECKLIST);
    expect(row(CHECKLIST)).toHaveFocus();

    await user.keyboard("{ArrowDown}");
    expect(row(/Scheduled maintenance/)).toHaveAttribute("aria-selected", "true");
    expect(row(/Scheduled maintenance/)).toHaveFocus();
    expect(
      await screen.findByRole("heading", { level: 1, name: /Scheduled maintenance/ }),
    ).toBeInTheDocument();

    await user.keyboard("j");
    expect(row(/Customer announcement draft/)).toHaveAttribute("aria-selected", "true");
    await user.keyboard("k");
    expect(row(/Scheduled maintenance/)).toHaveAttribute("aria-selected", "true");

    // Insert flags, Ctrl+U and U mark unread, Ctrl+Q (as in Outlook) and Q
    // mark read.
    await user.keyboard("{Insert}");
    await waitFor(async () => expect((await backend.getMessage("msg-2")).isFlagged).toBe(true));
    await user.keyboard("{Control>}u{/Control}");
    await waitFor(async () => expect((await backend.getMessage("msg-2")).isRead).toBe(false));
    await user.keyboard("{Control>}q{/Control}");
    await waitFor(async () => expect((await backend.getMessage("msg-2")).isRead).toBe(true));
    await user.keyboard("u");
    await waitFor(async () => expect((await backend.getMessage("msg-2")).isRead).toBe(false));
    await user.keyboard("q");
    await waitFor(async () => expect((await backend.getMessage("msg-2")).isRead).toBe(true));
    await user.keyboard("u");
    await waitFor(async () => expect((await backend.getMessage("msg-2")).isRead).toBe(false));
    await user.keyboard("q");
    await waitFor(async () => expect((await backend.getMessage("msg-2")).isRead).toBe(true));

    // Delete removes it and focus moves to the next message.
    await user.keyboard("{Delete}");
    expect(screen.queryByRole("option", { name: /Scheduled maintenance/ })).not.toBeInTheDocument();
    await waitFor(() => expect(row(/Customer announcement draft/)).toHaveFocus());

    // Shift+Down extends the selection.
    await user.keyboard("{Shift>}{ArrowDown}{/Shift}");
    expect(screen.getByText("2 selected")).toBeInTheDocument();
  });

  it("opens reply, reply all and forward with Ctrl+R, Ctrl+Shift+R and Ctrl+F", async () => {
    const user = userEvent.setup();
    await renderMail();
    await openMessage(user, CHECKLIST);
    await user.keyboard("{Control>}{Shift>}r{/Shift}{/Control}");
    const composer = await screen.findByRole("region", { name: "Reply" });
    expect(within(composer).getByDisplayValue(/^Re: Atlas launch checklist/)).toBeInTheDocument();
    expect(within(composer).getByText("Jonas Becker")).toBeInTheDocument();
  });

  it("starts a new message with Ctrl+N", async () => {
    const user = userEvent.setup();
    await renderMail();
    act(() => row(CHECKLIST).focus());
    await user.keyboard("{Control>}n{/Control}");
    expect(await screen.findByRole("region", { name: "New message" })).toBeInTheDocument();
  });
});

/** The search results' rows. */
function results() {
  return within(screen.getByRole("listbox", { name: "Search results" })).getAllByRole("option");
}

/** Waits until the search results are rows matching these names, in order. */
async function expectResults(...names: RegExp[]) {
  await waitFor(() => {
    const options = results();
    expect(options.map((o) => o.getAttribute("aria-label"))).toEqual(
      names.map(() => expect.any(String)),
    );
    names.forEach((name, i) => expect(options[i]!.getAttribute("aria-label")).toMatch(name));
  });
}

/** What the message list's live region last said. */
function announced() {
  const list = screen.getByRole("region", { name: "Message list" });
  return list.querySelector(":scope > p[role=status]")!.textContent;
}

/** Whether `text` was ever on the page, from now on. */
function watchFor(text: string): () => boolean {
  let seen = false;
  const look = () => {
    seen ||= document.body.textContent?.includes(text) ?? false;
  };
  const observer = new MutationObserver(look);
  observer.observe(document.body, { childList: true, subtree: true, characterData: true });
  return () => {
    look();
    observer.disconnect();
    return seen;
  };
}

/** A search error row's text. */
function failureRow() {
  return screen.queryByText(/Couldn't search/, { selector: ".mail-search-error p" });
}

describe("Search", () => {
  it("finds by operator, by the start of words and inside subjects", async () => {
    const backend = new MockBackend();
    const fromPriya = await backend.searchMessages("from:priya");
    await renderMail({ backend });

    await setSearch("from:priya");
    await expectResults(
      ...fromPriya.map((m) => new RegExp(m.subject.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"))),
    );
    results().forEach((o) => expect(o.getAttribute("aria-label")).toMatch(/Priya Raman/));

    await setSearch('subject:"launch checklist"');
    await expectResults(CHECKLIST);
    await setSearch("lisb");
    await expectResults(/Your trip to Lisbon/);
    await setSearch("isbo");
    await expectResults(/Your trip to Lisbon/);
  });

  it("says when Unread or Flagged hides results, and shows them all in one click", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const fromPriya = await backend.searchMessages("from:priya");
    const unread = fromPriya.filter((m) => !m.isRead);
    const fromJonas = await backend.searchMessages("from:jonas");
    expect(unread.length).toBeGreaterThan(0);
    expect(fromJonas.every((m) => m.isRead)).toBe(true);
    await renderMail({ backend });
    await user.click(screen.getByRole("button", { name: "Unread" }));
    const list = screen.getByRole("region", { name: "Message list" });

    // Some hidden: the title says how many are listed, of how many.
    await setSearch("from:priya");
    await expectResults(...unread.map(() => /^Unread/));
    expect(
      within(list).getByText(new RegExp(`^${unread.length} unread of ${fromPriya.length} results`)),
    ).toHaveTextContent(`${unread.length} unread of ${fromPriya.length} results for “from:priya”`);

    // All hidden: not "No results", but what the filter did.
    await setSearch("from:jonas");
    expect(await screen.findByText("No unread results")).toBeInTheDocument();
    expect(
      screen.getByText(`None of the ${fromJonas.length} results for “from:jonas” are unread.`),
    ).toBeInTheDocument();
    expect(screen.queryByText("No results")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Show all results" }));
    await expectResults(...fromJonas.map(() => /Jonas Becker/));
    expect(useMailStore.getState().filter).toBe("all");
    expect(screen.getByRole("button", { name: "All" })).toHaveAttribute("aria-pressed", "true");

    await user.click(screen.getByRole("button", { name: "Flagged" }));
    await setSearch("lisbon");
    expect(await screen.findByText("No flagged results")).toBeInTheDocument();
    expect(screen.getByText("The only result for “lisbon” isn't flagged.")).toBeInTheDocument();
  });

  it("says what the filter hides only about the results of the text they are for", async () => {
    const user = userEvent.setup();
    // Slow enough that the next text's results take a moment.
    const backend = new MockBackend({ latency: 60 });
    await renderMail({ backend });
    await user.click(screen.getByRole("button", { name: "Unread" }));
    await setSearch("from:jonas");
    expect(await screen.findByText("No unread results")).toBeInTheDocument();
    const claimed = watchFor("for “from:pri” are unread");

    await setSearch("from:pri");
    await waitFor(() => expect(announced()).toMatch(/^\d+ unread of \d+ results\.$/));
    // None of Jonas's results being unread was never said of Priya's.
    expect(claimed()).toBe(false);
    expect(document.querySelector(".mail-list__count")).toHaveTextContent(/for “from:pri”$/);
  });

  it("searches the whole mailbox on the servers, and what it finds acts like any message", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const resume = backend.pauseServerSearch();
    await renderMail({ backend });
    const sawNoResults = watchFor("No results");

    // Nothing on the device: the servers are being asked, and it says so.
    await setSearch("invoice");
    expect(
      await screen.findByText("Searching your whole mailbox…", { selector: ".empty-state__title" }),
    ).toBeInTheDocument();
    expect(
      await screen.findByText("Nothing on this device matches “invoice”."),
    ).toBeInTheDocument();
    expect(announced()).toBe("Searching your whole mailbox…");

    resume();
    const results = await screen.findByRole("listbox", { name: "Search results" });
    const hit = await within(results).findByRole("option", { name: /Northwind invoice 2023-118/ });
    expect(hit.getAttribute("aria-label")).toMatch(/Archive · Alex Morgan$/);
    await waitFor(() => expect(announced()).toBe("1 result."));
    expect(screen.queryByText("Searching your whole mailbox…")).not.toBeInTheDocument();
    expect(sawNoResults()).toBe(false);

    // Opened, flagged and moved like any other message.
    await openMessage(user, /Northwind invoice 2023-118/);
    const frame = await screen.findByTitle(/^Message: Northwind invoice 2023-118/);
    expect(frame.getAttribute("srcdoc")).toContain("Payment terms are 30 days");
    const id = (await backend.searchMessages("invoice"))[0]!.id;
    await user.click(within(row(/Northwind invoice/)).getByRole("button", { name: "Flag" }));
    expect(row(/Northwind invoice/).getAttribute("aria-label")).toMatch(/Flagged/);
    await waitFor(async () => expect((await backend.getMessage(id)).isFlagged).toBe(true));
    await user.click(toolbarButton("Move to"));
    await user.click(screen.getByRole("menuitem", { name: "Inbox" }));
    await waitFor(() =>
      expect(row(/Northwind invoice/).getAttribute("aria-label")).toMatch(/Inbox · Alex Morgan$/),
    );
    await waitFor(async () => expect((await backend.getMessage(id)).folderId).toBe("w-inbox"));
  });

  it("keeps what this device found when an account's server can't be searched", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    backend.failServerSearch("acct-home", {
      code: "network",
      message: "Could not reach imap.morgan-family.example.",
    });
    await renderMail({ backend });

    await setSearch("lisbon");
    expect(
      await screen.findByText(/Couldn't search/, { selector: ".mail-search-error p" }),
    ).toHaveTextContent(
      "Couldn't search Alex (Personal) on the server. Could not reach imap.morgan-family.example.",
    );
    await expectResults(/Your trip to Lisbon/);
    await waitFor(() =>
      expect(announced()).toBe("1 result. Couldn't search Alex (Personal) on the server."),
    );
    expect(document.querySelector(".mail-list__count")).toHaveAttribute("title", "1 result");

    // With nothing found, it says nothing was found on this device.
    await setSearch("zebras");
    expect(await screen.findByText("No results")).toBeInTheDocument();
    expect(
      screen.getByText("We didn't find anything for “zebras” on this device."),
    ).toBeInTheDocument();
    expect(failureRow()).toBeInTheDocument();

    // Back online: trying again finds the photos only its server has.
    backend.failServerSearch("acct-home", null);
    await user.click(screen.getByRole("button", { name: "Try again" }));
    await expectResults(/Safari photos from Etosha.*Inbox · Alex \(Personal\)$/);
    expect(failureRow()).not.toBeInTheDocument();
  });

  it("shows the servers' progress in the title, and an account not searched above the results", async () => {
    const backend = new MockBackend();
    backend.failServerSearch("acct-home", {
      code: "network",
      message: "Could not reach imap.morgan-family.example.",
    });
    const many = await backend.searchMessages("alex");
    expect(many.length).toBeGreaterThan(20);
    const resume = backend.pauseServerSearch();
    await renderMail({ backend });
    await setSearch("alex");
    const listbox = await screen.findByRole("listbox", { name: "Search results" });
    await waitFor(() => expect(results()).toHaveLength(many.length));
    // Beside the count, so the rows don't move when it goes.
    const subtitle = document.querySelector(".mail-list__subtitle")!;
    await waitFor(() => expect(subtitle).toHaveTextContent(/Searching your whole mailbox…$/));
    expect(listbox.previousElementSibling).toBeNull();

    resume();
    await waitFor(() => expect(subtitle).not.toHaveTextContent("Searching"));
    // However long the list, the account that wasn't searched is above it.
    const note = await waitFor(() => failureRow()!);
    expect(listbox.previousElementSibling).toContainElement(note);
    expect(note).toHaveTextContent("Couldn't search Alex (Personal) on the server.");
  });

  it("never shows the servers as busy for single letters typed over a longer search", async () => {
    const backend = new MockBackend();
    await renderMail({ backend });
    await setSearch("lisbon");
    await expectResults(/Your trip to Lisbon/);
    await waitFor(() => expect(announced()).toBe("1 result."));
    const searchServer = vi.spyOn(backend, "searchServer");
    const sawBusy = watchFor("Searching your whole mailbox…");

    await setSearch("l");
    await waitFor(() => expect(announced()).toMatch(/^\d+ results\.$/));
    await setSearch("from:l");
    await waitFor(() => expect(announced()).toMatch(/^\d+ results?\.$/));
    expect(sawBusy()).toBe(false);
    expect(searchServer).not.toHaveBeenCalled();
  });

  it("counts only the latest search when the text changes while the servers search", async () => {
    const backend = new MockBackend();
    const resume = backend.pauseServerSearch();
    const searches: [id: string, text: string][] = [];
    const canceled: string[] = [];
    const searchServer = backend.searchServer.bind(backend);
    backend.searchServer = (id, text, ...rest) => {
      searches.push([id, text]);
      return searchServer(id, text, ...rest);
    };
    const cancelServerSearch = backend.cancelServerSearch.bind(backend);
    backend.cancelServerSearch = (id) => {
      canceled.push(id);
      return cancelServerSearch(id);
    };
    await renderMail({ backend });

    await setSearch("offsite");
    await waitFor(() => expect(searches.map(([, text]) => text)).toContain("offsite"));
    const first = searches[0]![0];
    await setSearch("offsite kickoff");
    await waitFor(() => expect(searches.map(([, text]) => text)).toContain("offsite kickoff"));
    expect(canceled).toContain(first);
    resume();

    await expectResults(/Kickoff notes from the 2022 offsite/);
    // What only the first search would have found was neither shown nor stored.
    expect(screen.queryByRole("option", { name: /Offsite venue options/ })).not.toBeInTheDocument();
    expect(await backend.searchMessages("venue")).toEqual([]);
  });

  it("shows more results from the servers on request", async () => {
    const user = userEvent.setup();
    await renderMail({ backend: new MockBackend({ worst: true }) });
    await setSearch('subject:"purchase order"');
    await expectResults(...Array.from({ length: 50 }, () => /Purchase order 4500/));
    await waitFor(() => expect(announced()).toBe("50 results."));
    // A button after the list, not an option in it.
    const listbox = screen.getByRole("listbox", { name: "Search results" });
    expect(within(listbox).queryByRole("button", { name: "Show more results" })).toBeNull();

    await user.click(screen.getByRole("button", { name: "Show more results" }));
    await expectResults(...Array.from({ length: 100 }, () => /Purchase order 4500/));
    await user.click(screen.getByRole("button", { name: "Show more results" }));
    await expectResults(...Array.from({ length: 130 }, () => /Purchase order 4500/));
    expect(screen.queryByRole("button", { name: "Show more results" })).not.toBeInTheDocument();
    await waitFor(() => expect(announced()).toBe("130 results."));
  });

  it("offers to sign in again, not to try again, when an account's sign-in failed", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const work = (await backend.listAccounts()).find((a) => a.kind === "microsoft")!;
    const home = (await backend.listAccounts()).find((a) => a.kind === "imap")!;
    backend.failServerSearch(work.id, {
      code: "authRequired",
      message: "Microsoft did not accept the sign-in. Sign in again.",
    });
    backend.failServerSearch(home.id, {
      code: "authRequired",
      message: "The IMAP server rejected the password.",
    });
    await renderMail({ backend });
    await setSearch("lisbon");
    await waitFor(() =>
      expect(
        screen.getAllByText(/Couldn't search/, { selector: ".mail-search-error p" }),
      ).toHaveLength(2),
    );
    expect(screen.queryByRole("button", { name: "Try again" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Sign in again" }));
    expect(useAppStore.getState().addAccountOpen).toBe(true);
    expect(useAddAccountRequest.getState().request).toEqual({
      step: "microsoft",
      email: work.email,
      autoStart: true,
    });
    await user.click(screen.getByRole("button", { name: "Account settings" }));
    expect(useAppStore.getState().settingsOpen).toBe("accounts");
  });

  it("keeps the keyboard on Show more results, and at the end once all are shown", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ worst: true });
    await renderMail({ backend });
    await setSearch('subject:"purchase order"');
    const pages = (n: number) => Array.from({ length: n }, () => /Purchase order 4500/);
    await expectResults(...pages(50));
    const more = screen.getByRole("button", { name: "Show more results" });
    more.focus();

    const resume = backend.pauseServerSearch();
    await user.keyboard("{Enter}");
    // Busy where it was, and still where the keyboard is.
    expect(more).toHaveAccessibleName("Loading more results…");
    expect(more).toHaveAttribute("aria-busy", "true");
    expect(more).toHaveFocus();
    resume();
    await expectResults(...pages(100));
    expect(more).toHaveAccessibleName("Show more results");
    expect(more).toHaveFocus();

    await user.keyboard("{Enter}");
    await expectResults(...pages(130));
    const end = await screen.findByText("No more results.");
    await waitFor(() => expect(end).toHaveFocus());
  });

  it("keeps the keyboard on Try again while it tries, and in the search box once it worked", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    backend.failServerSearch("acct-home", {
      code: "network",
      message: "Could not reach imap.morgan-family.example.",
    });
    await renderMail({ backend });
    await setSearch("zebras");
    expect(await screen.findByText("No results")).toBeInTheDocument();
    const retry = screen.getByRole("button", { name: "Try again" });
    retry.focus();

    backend.failServerSearch("acct-home", null);
    const resume = backend.pauseServerSearch();
    await user.keyboard("{Enter}");
    expect(retry).toHaveAccessibleName("Trying again…");
    expect(retry).toHaveFocus();
    resume();
    await expectResults(/Safari photos from Etosha/);
    expect(failureRow()).not.toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByRole("searchbox", { name: "Search mail" })).toHaveFocus(),
    );
  });

  it("shows results with their folder and account", async () => {
    const user = userEvent.setup();
    await renderMail();
    await setSearch("lisbon");
    const results = await screen.findByRole("listbox", { name: "Search results" });
    const hit = within(results).getByRole("option", { name: /Your trip to Lisbon/ });
    expect(hit.getAttribute("aria-label")).toMatch(/Inbox · Alex Morgan$/);
    expect(within(results).getAllByRole("option")).toHaveLength(1);
    const list = screen.getByRole("region", { name: "Message list" });
    expect(within(list).getByText(/^1 result for/)).toHaveTextContent("1 result for “lisbon”");

    await setSearch("zzzz-nothing");
    expect(await screen.findByText("No results")).toBeInTheDocument();

    // The search field's own Clear button leaves search.
    await user.click(screen.getByRole("button", { name: "Clear" }));
    expect(useAppStore.getState().search).toBe("");
    expect(await screen.findByRole("listbox", { name: "Messages in Inbox" })).toBeInTheDocument();
  });
});

describe("Compose requests", () => {
  it("opens the composer from another module's request and clears it", async () => {
    await renderMail();
    act(() =>
      useAppStore.getState().requestCompose({
        to: [{ name: "Sam Okafor", address: "sam.okafor@acme.example" }],
        subject: "Lunch",
        accountId: "acct-home",
      }),
    );
    const composer = await screen.findByRole("region", { name: "New message" });
    expect(within(composer).getByText("Sam Okafor")).toBeInTheDocument();
    expect(within(composer).getByRole("textbox", { name: "Subject" })).toHaveValue("Lunch");
    expect(within(composer).getByRole("combobox", { name: "From" })).toHaveValue("acct-home");
    expect(useAppStore.getState().composeRequest).toBeNull();
  });
});
