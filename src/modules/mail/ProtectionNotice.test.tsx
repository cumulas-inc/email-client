import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { MockBackend } from "../../api/mock";
import { DEMO_PORTAL_URL } from "../../api/mock/data";
import type { Message } from "../../api/types";
import { folderTree, renderMail, respondButton, row, toolbarButton } from "./testUtils";

// Full-module tests in jsdom; allow for slow, busy CI machines.
vi.setConfig({ testTimeout: 20_000 });

type User = ReturnType<typeof userEvent.setup>;

function protectedBackend() {
  return new MockBackend({ protectedMail: true });
}

async function open(user: User, name: RegExp) {
  await user.click(row(name));
  await screen.findByRole("heading", { level: 1, name });
}

async function openHomeInbox(user: User) {
  const inbox = within(folderTree()).getByRole("treeitem", {
    name: /^Inbox \(Alex \(Personal\)\)/,
  });
  await user.click(within(inbox).getByText("Inbox"));
  await screen.findByRole("listbox", { name: /^Messages in Inbox/ });
}

function notice(name: string) {
  return screen.getByRole("note", { name });
}

/** The demo's messages, to change one as a test needs (before rendering). */
function messages(backend: MockBackend): Message[] {
  return (backend as unknown as { state: { messages: Message[] } }).state.messages;
}

describe("Protected messages", () => {
  it("explains mail protected with Microsoft 365, and opens it in Outlook on the web", async () => {
    const user = userEvent.setup();
    const backend = protectedBackend();
    const openInOutlook = vi.spyOn(backend, "openInOutlook");
    await renderMail({ backend });
    await open(user, /Q4 forecast/);

    const note = notice("Protected message");
    expect(within(note).getByText(/is protected with Microsoft 365/)).toHaveTextContent(
      "This message from Dana Whitfield (dana.whitfield@acme.example) is protected with Microsoft 365, so Cumulas Mail can't open it. Open it in Outlook on the web to read it, reply to it or forward it.",
    );
    expect(
      within(note)
        .getByText(/holds the protected content/)
        .closest("li"),
    ).toHaveTextContent(
      "message.rpmsg holds the protected content. It isn't a file you can open on this computer, so it isn't listed with the attachments.",
    );
    // The file itself isn't offered.
    expect(screen.queryByRole("button", { name: "Open message.rpmsg" })).toBeNull();
    expect(screen.queryByRole("region", { name: "Attachments" })).toBeNull();
    // Nor a portal button: this message has no link to it.
    expect(
      within(note).queryByRole("button", { name: "Open Microsoft's encrypted message portal" }),
    ).toBeNull();

    const button = within(note).getByRole("button", { name: "Open in Outlook on the web" });
    expect(button).toHaveClass("btn--primary");
    await user.click(button);
    expect(openInOutlook).toHaveBeenCalledWith("msg-protected-1");
    expect(backend.calls.openedInOutlook).toEqual(["msg-protected-1"]);

    // Its label, from the account's labels.
    const label = screen.getByTitle("Sensitivity label: Confidential › All Employees");
    expect(label).toHaveTextContent("Sensitivity label: Confidential › All Employees");
  });

  it("opens only the verified portal link, never the look-alike button", async () => {
    const user = userEvent.setup();
    const backend = protectedBackend();
    const openLink = vi.spyOn(backend, "openLink").mockResolvedValue(undefined);
    await renderMail({ backend });
    await open(user, /Contract draft/);

    const note = notice("Link to an encrypted message");
    expect(
      within(note).getByText(/links to Microsoft's encrypted message portal/),
    ).toHaveTextContent(
      "This message from Lucía Fernández (lucia@northwind-design.example) links to Microsoft's encrypted message portal, where you sign in to read it. Only open it if you expected a protected message from this sender, and use the button below, not links in the message: it goes only to Microsoft's portal.",
    );
    expect(within(note).queryByRole("button", { name: "Open in Outlook on the web" })).toBeNull();
    const portal = within(note).getByRole("button", {
      name: "Open Microsoft's encrypted message portal",
    });
    expect(portal).toHaveClass("btn--primary");
    await user.click(portal);
    expect(openLink).toHaveBeenCalledTimes(1);
    expect(openLink).toHaveBeenCalledWith(DEMO_PORTAL_URL);
    expect(String(openLink.mock.lastCall?.[0])).not.toContain("evil.example");
  });

  it("gives an IMAP account the portal, not Outlook advice, and only the sender's address", async () => {
    const user = userEvent.setup();
    const backend = protectedBackend();
    const openLink = vi.spyOn(backend, "openLink").mockResolvedValue(undefined);
    await renderMail({ backend });
    await openHomeInbox(user);
    await open(user, /Your protected message/);

    const note = notice("Protected message");
    expect(within(note).getByText(/is protected with Microsoft 365/)).toHaveTextContent(
      "This message from alerts@notacme.example is protected with Microsoft 365, so Cumulas Mail can't open it. Open it in Microsoft's encrypted message portal with the button below, not with links in the message.",
    );
    // The display name holds an address (and a right-to-left override): left out.
    expect(note).not.toHaveTextContent("IT Service Desk");
    expect(note.textContent).not.toContain("\u202e");
    expect(within(note).queryByRole("button", { name: "Open in Outlook on the web" })).toBeNull();
    const portal = within(note).getByRole("button", {
      name: "Open Microsoft's encrypted message portal",
    });
    expect(portal).toHaveClass("btn--primary");
    await user.click(portal);
    expect(String(openLink.mock.lastCall?.[0])).toMatch(
      /^https:\/\/outlook\.office365\.com\/Encryption\/retrieve\.ashx\?/,
    );
  });

  it("says S/MIME-encrypted mail can't be opened yet, without Outlook advice on IMAP", async () => {
    const user = userEvent.setup();
    await renderMail({ backend: protectedBackend() });
    await openHomeInbox(user);
    await open(user, /Tax documents/);

    const note = notice("Encrypted message (S/MIME)");
    expect(within(note).getByText(/is encrypted with S\/MIME/)).toHaveTextContent(
      "This message from Ben Morgan (ben.morgan@mailbox.example) is encrypted with S/MIME. Cumulas Mail can't open S/MIME-encrypted messages yet.",
    );
    expect(
      within(note)
        .getByText(/holds the encrypted content/)
        .closest("li"),
    ).toHaveTextContent(
      "smime.p7m holds the encrypted content, so it isn't listed with the attachments.",
    );
    expect(within(note).queryByRole("button")).toBeNull();
  });

  it("says a signed message's attachments are inside its signature file", async () => {
    const user = userEvent.setup();
    await renderMail({ backend: protectedBackend() });
    await open(user, /VPN certificate renewal/);

    const note = notice("Signed message (S/MIME)");
    expect(within(note).getByText(/has an S\/MIME signature/)).toHaveTextContent(
      "This message from Acme IT Service Desk (servicedesk@acme.example) has an S/MIME signature, which Cumulas Mail doesn't check. Any attachments it has are inside the signature file, which Cumulas Mail can't open yet. Open it in Outlook on the web to see them.",
    );
    expect(
      within(note)
        .getByText(/holds the signed content/)
        .closest("li"),
    ).toHaveTextContent(
      "smime.p7m holds the signed content and its attachments, so it isn't listed with the attachments.",
    );
    expect(within(note).getByRole("button", { name: "Open in Outlook on the web" })).toBeEnabled();
    // The body is still there, under the notice.
    expect(screen.getByTitle(/^Message: VPN certificate renewal/)).toBeInTheDocument();
  });

  it("tags signed mail whose content is all there, and says the signature isn't checked", async () => {
    const user = userEvent.setup();
    const backend = protectedBackend();
    const vpn = messages(backend).find((m) => m.id === "msg-protected-4")!;
    vpn.protection = { ...vpn.protection!, contentHidden: false, containerIds: [] };
    vpn.attachments = [];
    await renderMail({ backend });
    await open(user, /VPN certificate renewal/);

    expect(screen.queryByRole("note", { name: "Signed message (S/MIME)" })).toBeNull();
    expect(screen.getByText("S/MIME signature (not checked)")).toBeInTheDocument();
  });

  it("names labels from the account's list, or as the message names them", async () => {
    const user = userEvent.setup();
    const backend = protectedBackend();
    const [first, second] = messages(backend).filter((m) => m.folderId === "w-inbox" && !m.label);
    first!.label = { id: "1", name: "Partner Only", color: null, fromCatalog: false };
    second!.label = { id: "2", name: null, color: null, fromCatalog: false };
    await renderMail({ backend });

    await open(user, /Updated org chart/);
    expect(screen.getByTitle("Sensitivity label: General")).toHaveTextContent("General");
    // No notice for a label alone.
    expect(screen.queryByRole("note")).toBeNull();

    await open(user, new RegExp(first!.subject.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
    expect(screen.getByTitle("Label named in the message: Partner Only")).toHaveTextContent(
      "Label named in the message: Partner Only",
    );
    await open(user, new RegExp(second!.subject.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
    expect(
      screen.getByText("Sensitivity label", { selector: ".mail-tag__name bdi" }),
    ).toBeInTheDocument();
  });

  it("shows what a sent message asked for, and that the copy doesn't show the outcome", async () => {
    const user = userEvent.setup();
    await renderMail({ backend: protectedBackend() });
    const account = within(folderTree()).getByRole("treeitem", {
      name: /^Alex Morgan \(alex\.morgan@acme\.example\)$/,
    });
    await user.click(within(account).getByText("Sent Items"));
    await open(user, /Re: Atlas pricing/);

    expect(screen.getByTitle("Sensitivity label: Confidential")).toHaveTextContent(
      "Confidential (requested)",
    );
    expect(screen.getByText("Encryption requested")).toBeInTheDocument();
    expect(
      screen.getByText(
        "Cumulas Mail asked your organization's mail server for this when the message was sent. This copy doesn't show whether it did.",
      ),
    ).toBeVisible();
  });

  it("copies diagnostic details for every message, and opens Microsoft mail in Outlook on the web", async () => {
    const user = userEvent.setup();
    const backend = protectedBackend();
    await renderMail({ backend });
    await open(user, /Atlas launch checklist/);

    await user.click(toolbarButton("More actions"));
    const menu = screen.getByRole("menu", { name: "More actions" });
    expect(
      within(menu).getByRole("menuitem", { name: "Open in Outlook on the web" }),
    ).toBeEnabled();
    await user.click(within(menu).getByRole("menuitem", { name: "Copy diagnostic details" }));
    await waitFor(async () =>
      expect(await navigator.clipboard.readText()).toMatch(
        /^Cumulas Mail 0\.1\.0-demo: protection details of one message\n/,
      ),
    );
    expect(await navigator.clipboard.readText()).toContain("Detected: nothing");
    expect(
      await screen.findByText(
        "Copied diagnostic details. They describe the message's protection, not its content.",
      ),
    ).toBeInTheDocument();

    await user.click(toolbarButton("More actions"));
    await user.click(screen.getByRole("menuitem", { name: "Open in Outlook on the web" }));
    expect(backend.calls.openedInOutlook).toEqual(["msg-1"]);

    // IMAP mail has no Outlook on the web, but has the details.
    await openHomeInbox(user);
    await open(user, /Sunday lunch/);
    await user.click(toolbarButton("More actions"));
    expect(screen.queryByRole("menuitem", { name: "Open in Outlook on the web" })).toBeNull();
    expect(screen.getByRole("menuitem", { name: "Copy diagnostic details" })).toBeInTheDocument();
  });
});

describe("Replying to protected mail", () => {
  it("hands Reply, Reply all and Forward to Outlook on the web", async () => {
    const user = userEvent.setup();
    const backend = protectedBackend();
    const openInOutlook = vi.spyOn(backend, "openInOutlook");
    await renderMail({ backend });
    await open(user, /Q4 forecast/);

    await user.click(respondButton("Reply"));
    let dialog = await screen.findByRole("dialog", { name: "Reply in Outlook on the web" });
    expect(dialog).toHaveTextContent(
      "This message is protected. Cumulas Mail can't reply to it without removing the protection, so open it in Outlook on the web, which keeps it.",
    );
    expect(screen.queryByRole("region", { name: "Reply" })).toBeNull();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();

    // Ctrl+R too, and Forward, which goes to Outlook on the web.
    await user.click(row(/Q4 forecast/));
    await user.keyboard("{Control>}r{/Control}");
    dialog = await screen.findByRole("dialog", { name: "Reply in Outlook on the web" });
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    await user.click(respondButton("Forward"));
    dialog = await screen.findByRole("dialog", { name: "Forward in Outlook on the web" });
    expect(dialog).toHaveTextContent(
      "Cumulas Mail can't forward it without removing the protection",
    );
    await user.click(within(dialog).getByRole("button", { name: "Open in Outlook on the web" }));
    expect(openInOutlook).toHaveBeenCalledWith("msg-protected-1");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.queryByRole("region", { name: "Forward" })).toBeNull();
  });

  it("tells IMAP accounts to use their organization's app", async () => {
    const user = userEvent.setup();
    await renderMail({ backend: protectedBackend() });
    await openHomeInbox(user);
    await open(user, /Tax documents/);

    await user.click(respondButton("Reply all"));
    const dialog = await screen.findByRole("dialog", {
      name: "Can't reply to a protected message",
    });
    expect(dialog).toHaveTextContent(
      "Cumulas Mail can't reply to this message without removing its protection. Use Outlook, or the app your organization provides for protected mail.",
    );
    expect(within(dialog).queryByRole("button", { name: "Open in Outlook on the web" })).toBeNull();
    // Its own Close, beside the dialog's.
    await user.click(within(dialog).getAllByRole("button", { name: "Close" }).at(-1)!);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("lets signed mail be answered as usual", async () => {
    const user = userEvent.setup();
    await renderMail({ backend: protectedBackend() });
    await open(user, /VPN certificate renewal/);
    await user.click(respondButton("Reply"));
    expect(await screen.findByRole("region", { name: "Reply" })).toBeInTheDocument();
  });
});
