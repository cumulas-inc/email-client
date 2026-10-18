import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { MockBackend } from "../../api/mock";
import type { Folder, Settings } from "../../api/types";
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
    const projects = within(account).getByRole("treeitem", { name: "Projects, 1 unread" });
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
      new RegExp(
        `<script src="${messageBridgeUrl()}" data-channel="[0-9a-f]{32}" data-inbound="[0-9a-f]{32}"></script>`,
      ),
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

/** The filters above the search results. */
function searchFilters() {
  return screen.getByRole("group", { name: "Search filters" });
}

/** One of the filters' buttons. */
function chip(name: string) {
  return within(searchFilters()).getByRole("button", { name });
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

  it("writes what the filters say into the search, and shows what the search asks for", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const fromPriya = await backend.searchMessages("from:priya");
    const unread = fromPriya.filter((m) => !m.isRead);
    expect(unread.length).toBeGreaterThan(0);
    expect(unread.length).toBeLessThan(fromPriya.length);
    await renderMail({ backend });

    await setSearch("from:priya");
    await expectResults(...fromPriya.map(() => /Priya Raman/));
    const filters = searchFilters();
    expect(within(filters).getByRole("button", { name: "From: priya" })).toBeInTheDocument();
    // One Unread: the folder's own filter gives way while searching.
    expect(screen.getAllByRole("button", { name: "Unread" })).toHaveLength(1);
    expect(screen.queryByRole("group", { name: "Filter messages" })).not.toBeInTheDocument();

    await user.click(chip("Unread"));
    expect(useAppStore.getState().search).toBe("from:priya is:unread");
    expect(chip("Unread")).toHaveAttribute("aria-pressed", "true");
    await expectResults(...unread.map(() => /^Unread/));
    expect(
      within(screen.getByRole("region", { name: "Message list" })).getByText(/^\d+ results? for/),
    ).toHaveTextContent(
      `${unread.length} ${unread.length === 1 ? "result" : "results"} for “from:priya is:unread”`,
    );

    // Typed, a filter shows as set; Unread on and off leaves Flagged be.
    await setSearch("from:priya is:flagged");
    expect(chip("Flagged")).toHaveAttribute("aria-pressed", "true");
    expect(chip("Unread")).toHaveAttribute("aria-pressed", "false");
    await user.click(chip("Unread"));
    expect(useAppStore.getState().search).toBe("from:priya is:flagged is:unread");
    await user.click(chip("Unread"));
    expect(useAppStore.getState().search).toBe("from:priya is:flagged");
    await user.click(chip("Flagged"));
    expect(useAppStore.getState().search).toBe("from:priya");

    // Has attachment, and the From filter's own remove button.
    await user.click(chip("Has attachment"));
    expect(useAppStore.getState().search).toBe("from:priya has:attachment");
    await user.click(within(searchFilters()).getByRole("button", { name: "Remove From filter" }));
    expect(useAppStore.getState().search).toBe("has:attachment");
    expect(within(searchFilters()).getByRole("button", { name: "From" })).toBeInTheDocument();

    // Clear filters keeps the words.
    await setSearch("lunch is:unread has:attachment received:today");
    await user.click(within(searchFilters()).getByRole("button", { name: "Clear filters" }));
    expect(useAppStore.getState().search).toBe("lunch");
    expect(
      within(searchFilters()).queryByRole("button", { name: "Clear filters" }),
    ).not.toBeInTheDocument();
  });

  it("shows no filters, and one Unread, while the box is empty, even with the keyboard in it", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const unread = await backend.searchMessages("is:unread");
    await renderMail({ backend });
    const box = screen.getByRole("combobox", { name: "Search mail" });
    await user.click(box);
    expect(box).toHaveFocus();
    expect(screen.queryByRole("group", { name: "Search filters" })).not.toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "Unread" })).toHaveLength(1);

    // The suggestions offer the filters instead; one starts the search.
    const suggestions = screen.getByRole("listbox", { name: "Search suggestions" });
    await user.click(within(suggestions).getByRole("option", { name: /^is:unread/ }));
    expect(useAppStore.getState().search).toBe("is:unread ");
    expect(box).toHaveFocus();
    expect(screen.queryByRole("listbox", { name: "Search suggestions" })).not.toBeInTheDocument();
    expect(chip("Unread")).toHaveAttribute("aria-pressed", "true");
    await expectResults(...unread.map(() => /^Unread/));
  });

  it("picks dates from the Date filter, or a range of days, and gives the keyboard back", async () => {
    const user = userEvent.setup();
    await renderMail();
    await setSearch("lunch");
    const date = () => within(searchFilters()).getByRole("button", { name: "Date" });
    await user.click(date());
    const menu = screen.getByRole("menu", { name: "Date" });
    // Over the window, so the row's scrolling can't cut it off.
    expect(menu).toHaveClass("menu--fixed");
    expect(within(menu).getByRole("menuitemradio", { name: "Any time" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    await user.click(within(menu).getByRole("menuitemradio", { name: "Last month" }));
    expect(useAppStore.getState().search).toBe('lunch received:"last month"');
    const dateChip = document.getElementById("mail-search-date")!;
    expect(dateChip).toHaveTextContent("Last month");
    expect(dateChip).toHaveFocus();
    await user.click(dateChip);
    expect(screen.getByRole("menuitemradio", { name: "Last month" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    await user.click(screen.getByRole("menuitemradio", { name: "Today" }));
    expect(useAppStore.getState().search).toBe("lunch received:today");

    // A range: both days, one of them, never none or backwards.
    await user.click(dateChip);
    await user.click(screen.getByRole("menuitemradio", { name: "Custom range…" }));
    const dialog = screen.getByRole("dialog", { name: "Custom date range" });
    const from = within(dialog).getByLabelText("From");
    const to = within(dialog).getByLabelText("To");
    // From what the search says now: today.
    expect((from as HTMLInputElement).value).toMatch(/^\d{4}-\d{2}-\d{2}$/);
    expect((to as HTMLInputElement).value).toBe((from as HTMLInputElement).value);
    fireEvent.change(from, { target: { value: "" } });
    fireEvent.change(to, { target: { value: "" } });
    await user.click(within(dialog).getByRole("button", { name: "Apply" }));
    expect(within(dialog).getByRole("alert")).toHaveTextContent(
      "Choose a start date, an end date or both.",
    );
    fireEvent.change(from, { target: { value: "2026-10-15" } });
    fireEvent.change(to, { target: { value: "2026-10-01" } });
    await user.click(within(dialog).getByRole("button", { name: "Apply" }));
    expect(within(dialog).getByRole("alert")).toHaveTextContent(
      "The end date is before the start date.",
    );
    fireEvent.change(to, { target: { value: "2026-10-20" } });
    await user.click(within(dialog).getByRole("button", { name: "Apply" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(useAppStore.getState().search).toBe("lunch received:2026-10-15..2026-10-20");
    await waitFor(() => expect(dateChip).toHaveFocus());
    expect(dateChip).not.toHaveTextContent(/^Date$/);

    // Cancel changes nothing, and gives the keyboard back too.
    await user.click(dateChip);
    expect(screen.getByRole("menuitemradio", { name: "Custom range…" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    await user.click(screen.getByRole("menuitemradio", { name: "Custom range…" }));
    const again = screen.getByRole("dialog", { name: "Custom date range" });
    expect((within(again).getByLabelText("From") as HTMLInputElement).value).toBe("2026-10-15");
    expect((within(again).getByLabelText("To") as HTMLInputElement).value).toBe("2026-10-20");
    await user.click(within(again).getByRole("button", { name: "Cancel" }));
    expect(useAppStore.getState().search).toBe("lunch received:2026-10-15..2026-10-20");
    await waitFor(() => expect(dateChip).toHaveFocus());
    await user.click(dateChip);
    await user.click(screen.getByRole("menuitemradio", { name: "Any time" }));
    expect(useAppStore.getState().search).toBe("lunch");
  });

  it.each(["Spam", "★"])(
    "offers no Current folder that its filter can't name alone (%s)",
    async (name) => {
      const user = userEvent.setup();
      const backend = new MockBackend();
      // A folder of the user's named as Junk is known ("Spam"), or with no
      // letters at all.
      const state = (backend as unknown as { state: { folders: Folder[] } }).state;
      state.folders.find((f) => f.name === "Atlas")!.name = name;
      await renderMail({ backend });
      await user.click(within(treeItem(/^Projects/)).getAllByText(name)[0]!);
      await setSearch("research");
      await expectResults(/Atlas naming research summary/);
      await user.click(within(searchFilters()).getByRole("button", { name: "Folder" }));
      const menu = screen.getByRole("menu", { name: "Folder" });
      expect(
        within(menu)
          .getAllByRole("menuitemradio")
          .map((i) => i.textContent),
      ).toEqual(["All folders", "Inbox", "Drafts", "Sent", "Archive", "Junk", "Trash"]);
    },
  );

  it("narrows to a folder or an account from their filters, as one choice of several", async () => {
    const user = userEvent.setup();
    await renderMail();
    await user.click(within(treeItem(/^Projects/)).getAllByText("Atlas")[0]!);
    await setSearch("research");
    await expectResults(/Atlas naming research summary/);
    const folder = within(searchFilters()).getByRole("button", { name: "Folder" });
    await user.click(folder);
    const menu = screen.getByRole("menu", { name: "Folder" });
    expect(menu).toHaveClass("menu--fixed");
    expect(
      within(menu)
        .getAllByRole("menuitemradio")
        .map((i) => i.textContent),
    ).toEqual([
      "All folders",
      "Current folder (Atlas)",
      "Inbox",
      "Drafts",
      "Sent",
      "Archive",
      "Junk",
      "Trash",
    ]);
    await user.click(within(menu).getByRole("menuitemradio", { name: "Current folder (Atlas)" }));
    expect(useAppStore.getState().search).toBe("research in:Atlas");
    expect(folder).toHaveTextContent("Current folder (Atlas)");
    await expectResults(/Atlas naming research summary/);
    await user.click(folder);
    await user.click(screen.getByRole("menuitemradio", { name: "Inbox" }));
    expect(useAppStore.getState().search).toBe("research in:inbox");
    expect(folder).toHaveTextContent("Inbox");

    const account = within(searchFilters()).getByRole("button", { name: "Account" });
    await user.click(account);
    const accounts = screen.getByRole("menu", { name: "Account" });
    expect(accounts).toHaveClass("menu--fixed");
    expect(within(accounts).getByRole("menuitemradio", { name: "All accounts" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    await user.click(within(accounts).getByRole("menuitemradio", { name: "Alex (Personal)" }));
    expect(useAppStore.getState().search).toBe(
      "research in:inbox account:alex@morgan-family.example",
    );
    expect(account).toHaveTextContent("Alex (Personal)");
    // Folder and account alone are nothing to look for.
    await setSearch("in:inbox account:alex@morgan-family.example");
    expect(await screen.findByText("Add words or a filter")).toBeInTheDocument();
    expect(
      screen.getByText(
        "Folder and account only narrow a search. Type what you're looking for, or choose a sender, a date, Unread or Flagged above.",
      ),
    ).toBeInTheDocument();
    // Said as it's shown, never as "No results".
    await waitFor(() => expect(announced()).toBe("Add words or a filter."));
    await setSearch("is:starred");
    expect(
      await screen.findByText("Type what you're looking for, or choose a filter above."),
    ).toBeInTheDocument();
    await waitFor(() => expect(announced()).toBe("Add words or a filter."));
  });

  it("orders by best match or newest, as chosen, and only words have a best match", async () => {
    const user = userEvent.setup();
    await renderMail();
    await setSearch("porto");
    await expectResults(/Porto offsite: tickets and hotel/, /Porto itinerary/, /Friday plans/);
    // Best matches aren't in date order: no dates over them.
    expect(document.querySelector(".mail-group__header")).toBeNull();
    const sort = screen.getByRole("radiogroup", { name: "Sort results" });
    expect(within(sort).getByRole("radio", { name: "Best match" })).toBeChecked();
    await user.click(within(sort).getByRole("radio", { name: "Newest" }));
    await expectResults(/Friday plans/, /Porto offsite: tickets and hotel/, /Porto itinerary/);
    expect(document.querySelector(".mail-group__header")).not.toBeNull();
    expect(window.localStorage.getItem("cumulas.mail.searchSort")).toBe("newest");

    // Addresses and filters alone have no words to rank by.
    const count = () => document.querySelector(".mail-list__count");
    await setSearch("from:priya");
    await waitFor(() => expect(count()).toHaveTextContent(/for “from:priya”$/));
    expect(screen.queryByRole("radiogroup", { name: "Sort results" })).not.toBeInTheDocument();
    await setSearch("is:flagged");
    await waitFor(() => expect(count()).toHaveTextContent(/for “is:flagged”$/));
    expect(screen.queryByRole("radiogroup", { name: "Sort results" })).not.toBeInTheDocument();
    await setSearch("porto");
    await expectResults(/Friday plans/, /Porto offsite: tickets and hotel/, /Porto itinerary/);
    await user.click(screen.getByRole("radio", { name: "Best match" }));
    await expectResults(/Porto offsite: tickets and hotel/, /Porto itinerary/, /Friday plans/);
  });

  it("reads an order it doesn't know (another version's) as Best match", async () => {
    const getItem = Storage.prototype.getItem;
    const stored = vi.spyOn(Storage.prototype, "getItem");
    stored.mockImplementation(function (this: Storage, key) {
      return key === "cumulas.mail.searchSort" ? "oldest" : getItem.call(this, key);
    });
    try {
      await renderMail();
      await setSearch("porto");
      await expectResults(/Porto offsite: tickets and hotel/, /Porto itinerary/, /Friday plans/);
      expect(screen.getByRole("radio", { name: "Best match" })).toBeChecked();
    } finally {
      stored.mockRestore();
    }
  });

  it("keeps the best matches in place while the servers' hits join below them", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const resume = backend.pauseServerSearch();
    await renderMail({ backend });
    await setSearch("final");
    await expectResults(CHECKLIST, /Photos from the hike/);

    resume();
    // Ranked afresh, last year's guidelines would come second: they come last.
    await expectResults(CHECKLIST, /Photos from the hike/, /Brand guidelines, final edition/);
    await waitFor(() => expect(announced()).toBe("3 results."));
    // Another order ranks them afresh, and so does coming back.
    await user.click(screen.getByRole("radio", { name: "Newest" }));
    await expectResults(CHECKLIST, /Photos from the hike/, /Brand guidelines, final edition/);
    await user.click(screen.getByRole("radio", { name: "Best match" }));
    await expectResults(CHECKLIST, /Brand guidelines, final edition/, /Photos from the hike/);
  });

  it("ranks a new search afresh, even for the same text", async () => {
    const backend = new MockBackend();
    const { client } = await renderMail({ backend });
    await setSearch("final");
    await expectResults(CHECKLIST, /Photos from the hike/, /Brand guidelines, final edition/);
    await waitFor(() => expect(announced()).toBe("3 results."));
    await setSearch("");
    // Meanwhile a better match arrives (as the app hears of new mail).
    const [account] = await backend.listAccounts();
    await backend.sendMessage({
      accountId: account!.id,
      to: [{ name: "Sam", address: "sam@example.com" }],
      cc: [],
      bcc: [],
      subject: "Final invoice",
      bodyHtml: "<p>Attached.</p>",
      attachments: [],
    });
    await act(async () => {
      await client.invalidateQueries({ queryKey: ["mail"] });
    });
    const fresh = await backend.searchMessages("final", undefined, [], { sort: "relevance" });
    expect(fresh.map((m) => m.subject)).toEqual([
      "Final invoice",
      "Atlas launch checklist — final review today",
      "Brand guidelines, final edition",
      "Photos from the hike",
    ]);
    // The same words again: the order of now, not of the search before.
    await setSearch("final");
    await expectResults(
      /Final invoice/,
      CHECKLIST,
      /Brand guidelines, final edition/,
      /Photos from the hike/,
    );
    await waitFor(() => expect(announced()).toBe("4 results."));
    await expectResults(
      /Final invoice/,
      CHECKLIST,
      /Brand guidelines, final edition/,
      /Photos from the hike/,
    );
  });

  it("says when the server has more to look through, and finds it there", async () => {
    const user = userEvent.setup();
    // A page of one, and two folders, which Graph can't search by: the
    // server's newest match for "data" is in the Inbox, and only the next
    // one is in Sent.
    await renderMail({
      backend: new MockBackend({ serverSearchPage: 1, serverSearchMaxPages: 1 }),
    });
    await setSearch("data in:sent in:drafts");
    expect(await screen.findByText("No results so far")).toBeInTheDocument();
    expect(
      screen.getByText(
        "Nothing matches “data in:sent in:drafts” yet. Your mailbox on the server has more to look through.",
      ),
    ).toBeInTheDocument();
    await waitFor(() => expect(announced()).toBe("No results so far."));
    const empty = document.querySelector<HTMLElement>(".mail-search-empty")!;
    expect(within(empty).getByRole("button", { name: "Clear filters" })).toBeInTheDocument();
    await user.click(within(empty).getByRole("button", { name: "Show more results" }));
    await expectResults(/Re: Vendor shortlist for the data warehouse/);
  });

  it("marks the words found in the results' subjects, senders, previews and snippets", async () => {
    const user = userEvent.setup();
    await renderMail();
    const marks = (row: HTMLElement, selector: string) =>
      Array.from(row.querySelectorAll(`${selector} mark.search-hit`), (m) => m.textContent);

    await setSearch("porto");
    await expectResults(/Porto offsite/, /Porto itinerary/, /Friday plans/);
    expect(marks(row(/Porto offsite/), ".mail-row__subject")).toEqual(["Porto"]);
    expect(marks(row(/Friday plans/), ".mail-row__preview-text")).toEqual(["porto"]);
    expect(marks(row(/Friday plans/), ".mail-row__subject")).toEqual([]);
    // The name stays plain to screen readers.
    expect(row(/Porto offsite/).getAttribute("aria-label")).toMatch(
      /Porto offsite: tickets and hotel/,
    );

    await setSearch("from:priya checklist");
    await expectResults(CHECKLIST);
    expect(marks(row(CHECKLIST), ".mail-row__sender")).toEqual(["Priya"]);
    expect(marks(row(CHECKLIST), ".mail-row__subject")).toEqual(["checklist"]);

    // Once opened, a message's text is searched: the row shows the words around the match.
    await openMessage(user, CHECKLIST);
    await setSearch("retention");
    await waitFor(() =>
      expect(row(CHECKLIST).querySelector(".mail-row__preview-text")).toHaveTextContent(
        /data retention change/,
      ),
    );
    expect(marks(row(CHECKLIST), ".mail-row__preview-text")).toEqual(["retention"]);
  });

  it("steps through the matches in a plain-text message with the find bar, F3 and Shift+F3", async () => {
    const user = userEvent.setup();
    await renderMail();
    await setSearch("climbing");
    await waitFor(() => expect(row(/Climbing on Thursday/)).toBeInTheDocument());
    await openMessage(user, /Climbing on Thursday/);
    const bar = await screen.findByRole("group", { name: "Matches in this message" });
    expect(bar).toHaveTextContent("1 match");
    await setSearch("sam");
    await waitFor(() => expect(bar).toHaveTextContent("2 matches"));
    const body = document.querySelector(".mail-plaintext")!;
    expect(Array.from(body.querySelectorAll("mark.search-hit"), (m) => m.textContent)).toEqual([
      "Sam",
      "sam",
    ]);
    const current = () => body.querySelector("mark.is-current")?.getAttribute("data-hit");
    const next = within(bar).getByRole("button", { name: "Next match" });
    await user.click(next);
    expect(within(bar).getByRole("status")).toHaveTextContent("Match 1 of 2");
    expect(current()).toBe("0");
    await user.click(next);
    expect(within(bar).getByRole("status")).toHaveTextContent("Match 2 of 2");
    await user.click(next);
    expect(current()).toBe("0");
    await user.click(within(bar).getByRole("button", { name: "Previous match" }));
    expect(current()).toBe("1");
    await user.keyboard("{F3}");
    expect(current()).toBe("0");
    await user.keyboard("{Shift>}{F3}{/Shift}");
    expect(current()).toBe("1");
    // Other words start over.
    await setSearch("thursday");
    await waitFor(() => expect(bar).toHaveTextContent("1 match"));
    expect(current()).toBeUndefined();
  });

  it("shows the only match again on each step, wherever the reader scrolled", async () => {
    const user = userEvent.setup();
    // jsdom has no layout: what is scrolled into view is recorded instead.
    const shown: Element[] = [];
    Element.prototype.scrollIntoView = function (this: Element) {
      shown.push(this);
    };
    try {
      await renderMail();
      await setSearch("climbing");
      await waitFor(() => expect(row(/Climbing on Thursday/)).toBeInTheDocument());
      await openMessage(user, /Climbing on Thursday/);
      const bar = await screen.findByRole("group", { name: "Matches in this message" });
      expect(bar).toHaveTextContent("1 match");
      const hit = () => document.querySelector(".mail-plaintext mark.search-hit");
      shown.length = 0;
      await user.keyboard("{F3}");
      expect(shown).toEqual([hit()]);
      // Scrolled away to read on, then back to it.
      await user.keyboard("{F3}");
      await user.click(within(bar).getByRole("button", { name: "Next match" }));
      expect(shown).toEqual([hit(), hit(), hit()]);
    } finally {
      delete (Element.prototype as Partial<Element>).scrollIntoView;
    }
  });

  it("tells the message's frame what to mark, on its own channel, and shows the count", async () => {
    const user = userEvent.setup();
    await renderMail();
    await setSearch("rollback");
    await waitFor(() => expect(row(CHECKLIST)).toBeInTheDocument());
    await openMessage(user, CHECKLIST);
    const { frame, channel } = await messageFrame(/^Message: Atlas launch checklist/);
    const inbound = /data-inbound="([0-9a-f]{32})"/.exec(frame.getAttribute("srcdoc") ?? "")![1]!;
    expect(inbound).not.toBe(channel);
    const sent = vi.spyOn(frame.contentWindow!, "postMessage");
    post({ type: "ready", channel }, frame.contentWindow);
    await waitFor(() =>
      expect(sent).toHaveBeenCalledWith(
        {
          inbound,
          type: "highlight",
          patterns: [{ words: ["rollback"], prefix: true, inside: null }],
        },
        "*",
      ),
    );
    // The frame's channel never goes back to it.
    expect(JSON.stringify(sent.mock.calls)).not.toContain(channel);
    expect(screen.queryByRole("group", { name: "Matches in this message" })).toBeNull();
    post({ type: "hits", count: 2, channel }, frame.contentWindow);
    const bar = await screen.findByRole("group", { name: "Matches in this message" });
    expect(bar).toHaveTextContent("2 matches");
    await user.click(within(bar).getByRole("button", { name: "Next match" }));
    expect(sent).toHaveBeenLastCalledWith({ inbound, type: "reveal", index: 0 }, "*");
    // A count from anywhere but the frame's channel counts for nothing.
    post({ type: "hits", count: 9, channel: inbound }, frame.contentWindow);
    expect(bar).toHaveTextContent("Match 1 of 2");
    // One match: each step shows it again.
    post({ type: "hits", count: 1, channel }, frame.contentWindow);
    expect(bar).toHaveTextContent("1 match");
    sent.mockClear();
    await user.click(within(bar).getByRole("button", { name: "Next match" }));
    await user.click(within(bar).getByRole("button", { name: "Next match" }));
    const reveal = { inbound, type: "reveal", index: 0 };
    expect(sent.mock.calls).toEqual([
      [reveal, "*"],
      [reveal, "*"],
    ]);
  });

  it("keeps an opened best match where it was after it's read", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const [itinerary] = await backend.searchMessages("subject:itinerary");
    const [friday] = await backend.searchMessages('subject:"friday plans"');
    await backend.setRead([itinerary!.id, friday!.id], false);
    await renderMail({ backend });

    // The older subject match first; once read, the other stays second
    // (by date it would come first).
    await setSearch("porto is:unread");
    await expectResults(/Porto itinerary/, /Friday plans/);
    await openMessage(user, /Friday plans/);
    await waitFor(async () => expect((await backend.getMessage(friday!.id)).isRead).toBe(true));
    await waitFor(async () =>
      expect(await backend.searchMessages("porto is:unread")).toHaveLength(1),
    );
    await expectResults(/Porto itinerary/, /Friday plans/);
  });

  it("keeps an opened result where it was after it's read, newest first", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const [itinerary] = await backend.searchMessages("subject:itinerary");
    const [friday] = await backend.searchMessages('subject:"friday plans"');
    await backend.setRead([itinerary!.id, friday!.id], false);
    await renderMail({ backend });

    await setSearch("porto is:unread");
    await expectResults(/Porto itinerary/, /Friday plans/);
    await user.click(screen.getByRole("radio", { name: "Newest" }));
    await expectResults(/Friday plans/, /Porto itinerary/);
    await openMessage(user, /Porto itinerary/);
    await waitFor(async () => expect((await backend.getMessage(itinerary!.id)).isRead).toBe(true));
    await waitFor(async () =>
      expect(await backend.searchMessages("porto is:unread")).toHaveLength(1),
    );
    await expectResults(/Friday plans/, /Porto itinerary/);
  });

  it("remembers a search when a result is opened, and offers it again", async () => {
    const user = userEvent.setup();
    await renderMail();
    await setSearch("lisbon");
    await expectResults(/Your trip to Lisbon/);
    await user.click(row(/Your trip to Lisbon/));
    expect(JSON.parse(window.localStorage.getItem("cumulas.mail.recentSearches")!)).toEqual([
      "lisbon",
    ]);
    await setSearch("");
    await user.click(screen.getByRole("combobox", { name: "Search mail" }));
    const suggestions = screen.getByRole("listbox", { name: "Search suggestions" });
    await user.click(within(suggestions).getByRole("option", { name: "lisbon" }));
    expect(useAppStore.getState().search).toBe("lisbon");
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
      await screen.findByText("Nothing on this computer matches “invoice”."),
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
      screen.getByText("We didn't find anything for “zebras” on this computer."),
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

  it("says a message found on the server is no longer there when it went, and drops it", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const { client } = await renderMail({ backend });
    // As the app does when the backend says mail changed.
    backend.onDataChanged(() => void client.invalidateQueries({ queryKey: ["mail"] }));
    await setSearch("invoice");
    await expectResults(/Northwind invoice 2023-118/);
    const [found] = await backend.searchMessages("invoice");
    backend.removeFromServer(found!.id);

    await user.click(row(/Northwind invoice 2023-118/));
    expect(await screen.findByText("Couldn't open this message")).toBeInTheDocument();
    expect(screen.getByText("This message is no longer on the server.")).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.queryByRole("option", { name: /Northwind invoice/ })).not.toBeInTheDocument(),
    );
    expect(await backend.searchMessages("invoice")).toEqual([]);
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
    // The servers have more: the count says it is so far. (The worst-case
    // data's account whose sync fails can't be searched either.)
    const unsearched = "Couldn't search J on the server.";
    await waitFor(() => expect(announced()).toBe(`50 results so far. ${unsearched}`));
    expect(document.querySelector(".mail-list__count")).toHaveTextContent(
      '50 results so far for “subject:"purchase order"”',
    );
    // A button after the list, not an option in it.
    const listbox = screen.getByRole("listbox", { name: "Search results" });
    expect(within(listbox).queryByRole("button", { name: "Show more results" })).toBeNull();

    await user.click(screen.getByRole("button", { name: "Show more results" }));
    await expectResults(...Array.from({ length: 100 }, () => /Purchase order 4500/));
    await user.click(screen.getByRole("button", { name: "Show more results" }));
    await expectResults(...Array.from({ length: 130 }, () => /Purchase order 4500/));
    expect(screen.queryByRole("button", { name: "Show more results" })).not.toBeInTheDocument();
    await waitFor(() => expect(announced()).toBe(`130 results. ${unsearched}`));
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

  it("stops offering more once as many results have come as the list shows", async () => {
    const backend = new MockBackend();
    // A server with ever more hits: a thousand ids at once, as if from many pages.
    const hits = Array.from({ length: 1000 }, (_, i) => `hit-${i}`);
    backend.searchServer = async (searchId, _text, accountId) => ({
      searchId,
      accountId,
      status: "done",
      messageIds: accountId === "acct-work" ? hits : [],
      added: 0,
      hasMore: true,
      error: null,
    });
    await renderMail({ backend });
    await setSearch("lisbon");
    expect(
      await screen.findByText(
        "The servers found more than can be shown. Add words to narrow the search.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Show more results" })).not.toBeInTheDocument();
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
      expect(screen.getByRole("combobox", { name: "Search mail" })).toHaveFocus(),
    );
  });

  it("hands the keyboard on quietly when Try again worked", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    backend.failServerSearch("acct-home", {
      code: "network",
      message: "Could not reach imap.morgan-family.example.",
    });
    await renderMail({ backend });
    // A word with people to suggest.
    await setSearch("priya");
    await waitFor(() => expect(failureRow()).toBeInTheDocument());
    act(() => screen.getByRole("button", { name: "Try again" }).focus());
    backend.failServerSearch("acct-home", null);
    await user.keyboard("{Enter}");
    await waitFor(() => expect(failureRow()).not.toBeInTheDocument());
    const box = screen.getByRole("combobox", { name: "Search mail" });
    await waitFor(() => expect(box).toHaveFocus());
    // The suggestions don't cover the results it was waiting for (people
    // to suggest would have come by now).
    const suggestions = () => screen.queryByRole("listbox", { name: "Search suggestions" });
    await expect(
      waitFor(() => expect(suggestions()).not.toBeNull(), { timeout: 1000 }),
    ).rejects.toThrow();
    expect(box).toHaveAttribute("aria-expanded", "false");
    // A key opens them as ever.
    await user.keyboard("{ArrowDown}");
    expect(await screen.findByRole("listbox", { name: "Search suggestions" })).toBeInTheDocument();
  });

  it("keeps the keyboard near when the filter control it was on goes away", async () => {
    const user = userEvent.setup();
    await renderMail();
    const text = () => useAppStore.getState().search;
    const box = screen.getByRole("combobox", { name: "Search mail" });
    const suggestions = () => screen.queryByRole("listbox", { name: "Search suggestions" });

    // Clear filters goes with the filters: to the first one.
    await setSearch("lunch is:unread");
    act(() => chip("Clear filters").focus());
    await user.keyboard("{Enter}");
    expect(text()).toBe("lunch");
    expect(chip("From")).toHaveFocus();

    // A From filter removed: to the From filter in its place.
    await setSearch("from:priya lunch");
    act(() => chip("Remove From filter").focus());
    await user.keyboard("{Enter}");
    expect(text()).toBe("lunch");
    expect(chip("From")).toHaveFocus();

    // The search over, and the filters with it: to the box, its
    // suggestions closed.
    await setSearch("is:unread");
    act(() => chip("Unread").focus());
    await user.keyboard(" ");
    expect(text()).toBe("");
    expect(box).toHaveFocus();
    expect(suggestions()).toBeNull();

    // So too from a menu.
    await setSearch("received:today");
    act(() => chip("Today").focus());
    await user.keyboard("{Enter}");
    await user.click(screen.getByRole("menuitemradio", { name: "Any time" }));
    expect(text()).toBe("");
    expect(box).toHaveFocus();
    expect(suggestions()).toBeNull();

    // And from Clear filters where nothing was found.
    await setSearch("zzzz-nothing is:flagged");
    expect(await screen.findByText("No results")).toBeInTheDocument();
    const empty = screen.getByText("No results").closest(".mail-search-empty") as HTMLElement;
    act(() => within(empty).getByRole("button", { name: "Clear filters" }).focus());
    await user.keyboard("{Enter}");
    expect(text()).toBe("zzzz-nothing");
    expect(box).toHaveFocus();
    expect(suggestions()).toBeNull();
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
