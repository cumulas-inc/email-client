import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { MockBackend } from "../../api/mock";
import { useAppStore } from "../../app/store";
import { useMailStore } from "./mailStore";
import { deferred, folderTree, renderMail, row, setSearch, toolbarButton } from "./testUtils";

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
    expect(frame.getAttribute("sandbox")).toBe("allow-same-origin");
    const srcdoc = frame.getAttribute("srcdoc") ?? "";
    expect(srcdoc).toContain('<meta http-equiv="Content-Security-Policy"');
    expect(srcdoc).toContain("default-src 'none'");
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

    // Insert flags, Ctrl+U marks unread.
    await user.keyboard("{Insert}");
    await waitFor(async () => expect((await backend.getMessage("msg-2")).isFlagged).toBe(true));
    await user.keyboard("{Control>}u{/Control}");
    await waitFor(async () => expect((await backend.getMessage("msg-2")).isRead).toBe(false));
    await user.keyboard("{Control>}q{/Control}");
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

describe("Search", () => {
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
