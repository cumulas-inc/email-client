import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { MockBackend } from "../../api/mock";
import { ACME_SIGNATURE } from "../../api/mock/data";
import type { ComposeSignature } from "../../api/types";
import { useAppStore } from "../../app/store";
import { useMailStore } from "./mailStore";
import { deferred, editFrame, loadFrame, renderMail, respondButton, row } from "./testUtils";

// Full-module tests in jsdom; allow for slow, busy CI machines.
vi.setConfig({ testTimeout: 20_000 });

const CHECKLIST = /Atlas launch checklist — final review today/;

type User = ReturnType<typeof userEvent.setup>;

async function open(user: User, name: RegExp) {
  await user.click(row(name));
  await screen.findByRole("heading", { level: 1, name });
}

function composer(name: string | RegExp = /New message|Reply|Forward/) {
  return screen.getByRole("region", { name });
}

/** Chip texts in a recipient field. */
function chips(label: "To" | "Cc" | "Bcc") {
  const input = within(composer()).getByRole("combobox", { name: label });
  const box = input.closest(".mail-recipients__box")!;
  return Array.from(box.querySelectorAll(".mail-chip__text")).map((c) => c.textContent);
}

async function newMessage(user: User) {
  await user.click(screen.getByRole("button", { name: "New message" }));
  return screen.findByRole("region", { name: "New message" });
}

describe("Reply and forward", () => {
  it("prefills a reply to the sender with the quoted original", async () => {
    const user = userEvent.setup();
    await renderMail();
    await open(user, CHECKLIST);
    await user.click(respondButton("Reply"));
    const c = await screen.findByRole("region", { name: "Reply" });
    expect(chips("To")).toEqual(["Priya Raman"]);
    expect(within(c).queryByRole("combobox", { name: "Cc" })).not.toBeInTheDocument();
    expect(within(c).getByRole("textbox", { name: "Subject" })).toHaveValue(
      "Re: Atlas launch checklist — final review today",
    );
    const quote = within(c).getByTitle("Original message").getAttribute("srcdoc") ?? "";
    // The demo message arrived this morning.
    const today = new Date().toLocaleDateString("en-US", {
      weekday: "short",
      month: "short",
      day: "numeric",
      year: "numeric",
    });
    expect(quote).toContain(`On ${today}, at `);
    expect(quote).toMatch(/, at \d{1,2}:\d\d [AP]M, Priya Raman wrote:<\/p>/);
    expect(quote).toContain('<blockquote type="cite"');
    expect(quote).toContain("rollback plan");
    expect(quote).not.toContain("<b>From:</b>");
    // Shown quieter than what's being written: secondary text.
    expect(quote).toMatch(/body \{[^}]*color: #57534c;/);
    // The original can be tucked away, and the toggle sits right under the editor.
    const toggle = within(c).getByRole("button", { name: "Hide original message" });
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    await user.click(toggle);
    expect(within(c).queryByTitle("Original message")).not.toBeInTheDocument();
    expect(within(c).getByRole("button", { name: "Show original message" })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
  });

  it("replies to all except the user's own address", async () => {
    const user = userEvent.setup();
    await renderMail();
    await open(user, /Quarterly planning/);
    await user.click(respondButton("Reply all"));
    await screen.findByRole("region", { name: "Reply" });
    expect(chips("To")).toEqual([
      "Dana Whitfield",
      "Priya Raman",
      "Jonas Becker",
      "Mei Tanaka",
      "Sam Okafor",
    ]);
    expect(chips("Cc")).toEqual(["Engineering Leads"]);
  });

  it("doesn't repeat Re: and forwards with Fwd: and no recipients (Ctrl+F)", async () => {
    const user = userEvent.setup();
    await renderMail();
    await open(user, /Re: Load test results/);
    await user.click(respondButton("Reply"));
    expect(
      within(await screen.findByRole("region", { name: "Reply" })).getByRole("textbox", {
        name: "Subject",
      }),
    ).toHaveValue("Re: Load test results");

    // Back to the message, then forward with the keyboard.
    await user.click(screen.getByRole("button", { name: "Minimize draft" }));
    await user.click(row(/Re: Load test results/));
    await user.keyboard("{Control>}f{/Control}");
    // The unsent reply has no changes, so it is replaced without asking.
    const fwd = await screen.findByRole("region", { name: "Forward" });
    expect(within(fwd).getByRole("textbox", { name: "Subject" })).toHaveValue(
      "Fwd: Re: Load test results",
    );
    expect(chips("To")).toEqual([]);
    // A forward passes the original on under a compact header, not a quote.
    const quote = within(fwd).getByTitle("Original message").getAttribute("srcdoc") ?? "";
    expect(quote).toContain(">Forwarded message</div>");
    expect(quote).toMatch(/>From<\/span>Jonas Becker &lt;jonas\.becker@acme\.example&gt;/);
    expect(quote).toMatch(/>Subject<\/span>Re: Load test results/);
    expect(quote).not.toContain('<blockquote type="cite"');
  });

  it("sends a reply linked to the original", async () => {
    const user = userEvent.setup();
    const { backend } = await renderMail();
    await open(user, CHECKLIST);
    await user.keyboard("{Control>}r{/Control}");
    const c = await screen.findByRole("region", { name: "Reply" });
    await user.click(within(c).getByRole("button", { name: "Send" }));
    await waitFor(() => expect((backend as MockBackend).sent).toHaveLength(1));
    const sent = (backend as MockBackend).sent[0]!;
    expect(sent.replyToMessageId).toBe("msg-1");
    expect(sent.replyKind).toBe("reply");
    expect(sent.to).toEqual([{ name: "Priya Raman", address: "priya.raman@acme.example" }]);
    expect(sent.subject).toBe("Re: Atlas launch checklist — final review today");
    expect(sent.bodyHtml).toContain('class="cumulas-quote"');
    expect(await screen.findByText("Message sent")).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Reply" })).not.toBeInTheDocument();
  });
});

describe("Recipients", () => {
  it("autocompletes from contacts with the keyboard", async () => {
    const user = userEvent.setup();
    await renderMail();
    const c = await newMessage(user);
    const to = within(c).getByRole("combobox", { name: "To" });
    expect(to).toHaveFocus();
    await user.type(to, "jon");
    const list = await screen.findByRole("listbox", { name: "Suggested contacts" });
    expect(within(list).getByRole("option", { name: /Jonas Becker/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(to).toHaveAttribute("aria-expanded", "true");
    await user.keyboard("{Enter}");
    expect(chips("To")).toEqual(["Jonas Becker"]);
    expect(to).toHaveValue("");

    // Arrow keys move through suggestions.
    await user.type(to, "a");
    await screen.findByRole("listbox", { name: "Suggested contacts" });
    await user.keyboard("{ArrowDown}");
    const options = within(
      screen.getByRole("listbox", { name: "Suggested contacts" }),
    ).getAllByRole("option");
    expect(options[1]).toHaveAttribute("aria-selected", "true");
    await user.click(options[0]!);
    expect(chips("To")).toHaveLength(2);
  });

  it("turns typed and pasted addresses into chips and validates them", async () => {
    const user = userEvent.setup();
    await renderMail();
    const c = await newMessage(user);
    const to = within(c).getByRole("combobox", { name: "To" });

    await user.type(to, "ann@example.org;");
    expect(chips("To")).toEqual(["ann@example.org"]);

    await user.paste('"Bob Stone" <bob@example.org>, carol@example.org; not-an-address');
    expect(chips("To")).toEqual(["ann@example.org", "Bob Stone", "carol@example.org"]);
    expect(to).toHaveValue("not-an-address");
    expect(screen.getByRole("alert")).toHaveTextContent(
      "“not-an-address” isn't a valid email address.",
    );
    expect(to).toHaveAttribute("aria-invalid", "true");

    // Backspace in an empty field removes the last chip.
    await user.clear(to);
    await user.keyboard("{Backspace}");
    expect(chips("To")).toEqual(["ann@example.org", "Bob Stone"]);

    await user.click(within(c).getByRole("button", { name: "Remove Bob Stone" }));
    expect(chips("To")).toEqual(["ann@example.org"]);

    // Cc and Bcc fields appear on demand.
    await user.click(within(c).getByRole("button", { name: "Cc" }));
    expect(within(c).getByRole("combobox", { name: "Cc" })).toBeInTheDocument();
  });
});

describe("Sending", () => {
  it("validates recipients, confirms a missing subject and sends", async () => {
    const user = userEvent.setup();
    const { backend } = await renderMail();
    const c = await newMessage(user);
    await user.click(within(c).getByRole("button", { name: "Send" }));
    expect(screen.getByRole("alert")).toHaveTextContent("Add at least one recipient.");
    expect((backend as MockBackend).sent).toHaveLength(0);

    // Half-typed invalid address blocks sending.
    const to = within(c).getByRole("combobox", { name: "To" });
    await user.type(to, "nobody@");
    await user.click(within(c).getByRole("button", { name: "Send" }));
    expect(screen.getByRole("alert")).toHaveTextContent("“nobody@” isn't a valid email address.");

    await user.clear(to);
    await user.type(to, "sam.okafor@acme.example");
    // A valid typed address is accepted on send without pressing Enter.
    await user.click(within(c).getByRole("button", { name: "Send" }));
    const dialog = await screen.findByRole("dialog", { name: "Send without a subject?" });
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));

    await user.type(within(c).getByRole("textbox", { name: "Subject" }), "Lunch?");
    const important = within(c).getByRole("button", { name: "Important" });
    expect(important).toHaveAttribute("aria-pressed", "false");
    await user.click(important);
    expect(important).toHaveAttribute("aria-pressed", "true");
    await user.click(within(c).getByRole("button", { name: "Send" }));
    await waitFor(() => expect((backend as MockBackend).sent).toHaveLength(1));
    const sent = (backend as MockBackend).sent[0]!;
    expect(sent).toMatchObject({
      accountId: "acct-work",
      to: [{ name: null, address: "sam.okafor@acme.example" }],
      cc: [],
      bcc: [],
      subject: "Lunch?",
      importance: "high",
      replyToMessageId: null,
      replyKind: null,
    });
    expect(await screen.findByText("Message sent")).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "New message" })).not.toBeInTheDocument();
  });

  it("sends with Ctrl+Enter and includes the editor content and attachments", async () => {
    const user = userEvent.setup();
    const { backend } = await renderMail();
    act(() =>
      useAppStore.getState().requestCompose({
        to: [{ name: "Mei Tanaka", address: "mei.tanaka@acme.example" }],
        subject: "Notes",
        bodyHtml: "<p>Hello <b>there</b></p>",
      }),
    );
    const c = await screen.findByRole("region", { name: "New message" });
    expect(await within(c).findByRole("textbox", { name: "Message body" })).toHaveTextContent(
      "Hello there",
    );

    const file = new File(["hello world"], "notes.txt", { type: "text/plain" });
    await user.upload(within(c).getByTestId("compose-file-input"), file);
    expect(await within(c).findByText("notes.txt")).toBeInTheDocument();

    const big = new File(["x"], "video.mp4", { type: "video/mp4" });
    Object.defineProperty(big, "size", { value: 21 * 1024 * 1024 });
    await user.upload(within(c).getByTestId("compose-file-input"), big);
    expect(
      await within(c).findByText(/Many mail servers reject messages larger than 20 MB/),
    ).toBeVisible();
    await user.click(within(c).getByRole("button", { name: "Remove video.mp4" }));
    expect(within(c).queryByText(/larger than 20 MB/)).not.toBeInTheDocument();

    await user.click(within(c).getByRole("textbox", { name: "Subject" }));
    await user.keyboard("{Control>}{Enter}{/Control}");
    await waitFor(() => expect((backend as MockBackend).sent).toHaveLength(1));
    const sent = (backend as MockBackend).sent[0]!;
    expect(sent.bodyHtml).toMatch(/<p style="margin:0">Hello <(b|strong)>there<\/(b|strong)><\/p>/);
    expect(sent.attachments).toEqual([
      { name: "notes.txt", contentType: "text/plain", dataBase64: btoa("hello world") },
    ]);
  });

  it("sends low importance from the overflow menu", async () => {
    const user = userEvent.setup();
    const { backend } = await renderMail();
    const c = await newMessage(user);
    await user.type(within(c).getByRole("combobox", { name: "To" }), "ann@example.org{Enter}");
    await user.type(within(c).getByRole("textbox", { name: "Subject" }), "FYI");
    // The only importance control on the toolbar is "Important".
    expect(within(c).queryByRole("button", { name: /importance/i })).not.toBeInTheDocument();

    const important = within(c).getByRole("button", { name: "Important" });
    await user.click(important);
    await user.click(within(c).getByRole("button", { name: "More options" }));
    const menu = screen.getByRole("menu", { name: "More options" });
    const low = within(menu).getByRole("menuitemcheckbox", { name: "Low importance" });
    expect(low).toHaveAttribute("aria-checked", "false");
    await user.click(low);
    // One or the other: low turns "Important" off.
    expect(important).toHaveAttribute("aria-pressed", "false");
    expect(within(c).getByText("Low importance")).toBeInTheDocument();
    await user.click(within(c).getByRole("button", { name: "More options" }));
    expect(
      within(screen.getByRole("menu")).getByRole("menuitemcheckbox", { name: "Low importance" }),
    ).toHaveAttribute("aria-checked", "true");
    await user.keyboard("{Escape}");

    await user.click(within(c).getByRole("button", { name: "Send" }));
    await waitFor(() => expect((backend as MockBackend).sent).toHaveLength(1));
    expect((backend as MockBackend).sent[0]!.importance).toBe("low");
  });

  it("reports send failures and keeps the draft", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    backend.sendMessage = async () => {
      throw { code: "network", message: "You're offline." };
    };
    await renderMail({ backend });
    const c = await newMessage(user);
    await user.type(within(c).getByRole("combobox", { name: "To" }), "ann@example.org{Enter}");
    await user.type(within(c).getByRole("textbox", { name: "Subject" }), "Hi");
    await user.click(within(c).getByRole("button", { name: "Send" }));
    expect(await screen.findByText("You're offline.")).toBeInTheDocument();
    expect(composer("New message")).toBeInTheDocument();
  });
});

describe("Drafts", () => {
  it("asks before discarding a changed draft", async () => {
    const user = userEvent.setup();
    await renderMail();
    let c = await newMessage(user);
    // Untouched: closes right away.
    await user.click(within(c).getByRole("button", { name: "Discard" }));
    expect(screen.queryByRole("region", { name: "New message" })).not.toBeInTheDocument();

    c = await newMessage(user);
    await user.type(within(c).getByRole("textbox", { name: "Subject" }), "Plans");
    await user.click(within(c).getByRole("button", { name: "Discard" }));
    const dialog = await screen.findByRole("dialog", { name: "Discard this draft?" });
    await user.click(within(dialog).getByRole("button", { name: "Discard" }));
    expect(screen.queryByRole("region", { name: "New message" })).not.toBeInTheDocument();
  });

  it("keeps a draft while reading other mail and resumes it", async () => {
    const user = userEvent.setup();
    await renderMail();
    const c = await newMessage(user);
    await user.type(within(c).getByRole("textbox", { name: "Subject" }), "Offsite ideas");
    await user.click(row(/Lunch on Thursday/));
    expect(
      await screen.findByRole("heading", { level: 1, name: /Lunch on Thursday/ }),
    ).toBeVisible();
    const tab = screen.getByRole("region", { name: "Unsent draft" });
    expect(within(tab).getByText("Offsite ideas")).toBeInTheDocument();

    // Starting another message asks first.
    await user.click(screen.getByRole("button", { name: "New message" }));
    const dialog = await screen.findByRole("dialog", { name: "Discard this draft?" });
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));

    await user.click(within(tab).getByRole("button", { name: /Offsite ideas/ }));
    expect(
      within(await screen.findByRole("region", { name: "New message" })).getByRole("textbox", {
        name: "Subject",
      }),
    ).toHaveValue("Offsite ideas");
  });

  it("continues editing a saved draft", async () => {
    const user = userEvent.setup();
    await renderMail();
    const account = screen.getByRole("treeitem", { name: /^Alex Morgan \(/ });
    await user.click(within(account).getByText("Drafts"));
    await user.click(await screen.findByRole("option", { name: /Thoughts on the rollback plan/ }));
    expect(await screen.findByText("This message hasn't been sent.")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Continue editing" }));
    const c = await screen.findByRole("region", { name: "New message" });
    expect(within(c).getByRole("textbox", { name: "Subject" })).toHaveValue(
      "Thoughts on the rollback plan",
    );
    expect(chips("To")).toEqual(["Priya Raman"]);
    expect(await within(c).findByRole("textbox", { name: "Message body" })).toHaveTextContent(
      "a few thoughts on the rollback plan",
    );
  });

  it("chooses the sending account when there are several", async () => {
    const user = userEvent.setup();
    const { backend } = await renderMail();
    const c = await newMessage(user);
    await user.selectOptions(within(c).getByRole("combobox", { name: "From" }), "acct-home");
    await user.type(
      within(c).getByRole("combobox", { name: "To" }),
      "ben.morgan@mailbox.example{Enter}",
    );
    await user.type(within(c).getByRole("textbox", { name: "Subject" }), "Sunday");
    await user.click(within(c).getByRole("button", { name: "Send" }));
    await waitFor(() => expect((backend as MockBackend).sent).toHaveLength(1));
    expect((backend as MockBackend).sent[0]!.accountId).toBe("acct-home");
  });
});

describe("Signature", () => {
  /** The letter's style, as the sent body's first block has it. */
  const LETTER = /^<div style="(font-family:'Charis SIL'[^"]*)">/;

  async function signatureFrame(c: HTMLElement) {
    return (await within(c).findByTitle("Signature")) as HTMLIFrameElement;
  }

  async function address(user: User, c: HTMLElement, subject = "Plans") {
    await user.type(within(c).getByRole("combobox", { name: "To" }), "ann@example.org{Enter}");
    await user.type(within(c).getByRole("textbox", { name: "Subject" }), subject);
  }

  async function sent(backend: MockBackend) {
    await waitFor(() => expect(backend.sent).toHaveLength(1));
    return backend.sent[0]!.bodyHtml;
  }

  it("adds the organization's signature, in its own style, without asking to discard", async () => {
    const user = userEvent.setup();
    const { backend } = await renderMail();
    let c = await newMessage(user);
    const doc = await loadFrame(await signatureFrame(c));
    expect(doc.body.innerHTML).toContain("Engineering Manager, Atlas");
    expect(within(c).getByRole("group", { name: "Signature" })).toBeInTheDocument();
    // Following the account's signature isn't a change: closing doesn't ask.
    await user.click(within(c).getByRole("button", { name: "Discard" }));
    expect(screen.queryByRole("dialog", { name: "Discard this draft?" })).not.toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "New message" })).not.toBeInTheDocument();

    c = await newMessage(user);
    await address(user, c);
    await user.click(within(c).getByRole("button", { name: "Send" }));
    const html = await sent(backend as MockBackend);
    const letter = LETTER.exec(html)![1];
    // The body, one empty line, then the signature: not in the letter's
    // style, so it keeps its own fonts.
    expect(html).toBe(
      `<div style="${letter}"><p style="margin:0"><br></p><p style="margin:0"><br></p></div>` +
        `<div class="cumulas-signature">${ACME_SIGNATURE}</div>`,
    );
  });

  it("puts the signature above the quoted original in a reply", async () => {
    const user = userEvent.setup();
    const { backend } = await renderMail();
    await open(user, CHECKLIST);
    await user.click(respondButton("Reply"));
    const c = await screen.findByRole("region", { name: "Reply" });
    await signatureFrame(c);
    await user.click(within(c).getByRole("button", { name: "Send" }));
    const html = await sent(backend as MockBackend);
    const signature = html.indexOf('<div class="cumulas-signature">');
    expect(signature).toBeGreaterThan(0);
    expect(html.indexOf('<div class="cumulas-quote">')).toBeGreaterThan(signature);
  });

  it("sets the person's own signature like the letter", async () => {
    const user = userEvent.setup();
    const { backend } = await renderMail();
    const c = await newMessage(user);
    await user.selectOptions(within(c).getByRole("combobox", { name: "From" }), "acct-home");
    const doc = await loadFrame(await signatureFrame(c));
    expect(doc.body.innerHTML).toBe("<div>Alex Morgan</div><div>+1 (206) 555-0199</div>");
    await address(user, c);
    await user.click(within(c).getByRole("button", { name: "Send" }));
    const html = await sent(backend as MockBackend);
    const letter = LETTER.exec(html)![1];
    expect(html).toContain(
      `<div class="cumulas-signature" style="${letter}"><div>Alex Morgan</div><div>+1 (206) 555-0199</div></div>`,
    );
  });

  it("sends the signature as edited; changing it asks before discarding", async () => {
    const user = userEvent.setup();
    const { backend } = await renderMail();
    const c = await newMessage(user);
    const doc = await loadFrame(await signatureFrame(c));
    editFrame(doc, doc.body.innerHTML.replace("Engineering Manager", "Engineering Director"));
    expect(useMailStore.getState().compose?.dirty).toBe(true);

    await user.click(within(c).getByRole("button", { name: "Discard" }));
    const dialog = await screen.findByRole("dialog", { name: "Discard this draft?" });
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));

    await address(user, c);
    await user.click(within(c).getByRole("button", { name: "Send" }));
    const html = await sent(backend as MockBackend);
    expect(html).toContain('<div class="cumulas-signature"><table');
    expect(html).toContain("Engineering Director, Atlas");
    expect(html).not.toContain("Engineering Manager");
  });

  it("removes, adds back and restores the signature", async () => {
    const user = userEvent.setup();
    const { backend } = await renderMail();
    const c = await newMessage(user);
    let doc = await loadFrame(await signatureFrame(c));
    editFrame(doc, "<div>Alex</div>");
    const restore = within(c).getByRole("button", { name: "Restore signature" });
    await user.click(restore);
    doc = await loadFrame(await signatureFrame(c));
    expect(doc.body.innerHTML).toContain("Engineering Manager, Atlas");
    expect(within(c).queryByRole("button", { name: "Restore signature" })).not.toBeInTheDocument();
    // Focus moves to what took the pressed button's place.
    expect(within(c).getByRole("button", { name: "Remove signature" })).toHaveFocus();

    await user.click(within(c).getByRole("button", { name: "Remove signature" }));
    expect(within(c).queryByTitle("Signature")).not.toBeInTheDocument();
    const add = within(c).getByRole("button", { name: "Add signature" });
    expect(add).toHaveFocus();
    await user.click(add);
    expect(await within(c).findByTitle("Signature")).toBeInTheDocument();
    await user.click(within(c).getByRole("button", { name: "Remove signature" }));

    await address(user, c);
    await user.click(within(c).getByRole("button", { name: "Send" }));
    expect(await sent(backend as MockBackend)).not.toContain("cumulas-signature");
  });

  it("follows the From account, replacing an edited signature", async () => {
    const user = userEvent.setup();
    await renderMail();
    const c = await newMessage(user);
    let doc = await loadFrame(await signatureFrame(c));
    editFrame(doc, "<div>Edited</div>");
    const from = within(c).getByRole("combobox", { name: "From" });
    await user.selectOptions(from, "acct-home");
    doc = await loadFrame(await signatureFrame(c));
    await waitFor(() => expect(doc.body.textContent).toBe("Alex Morgan+1 (206) 555-0199"));
    expect(useMailStore.getState().compose?.signature).toBe("auto");

    // Removed, too.
    await user.click(within(c).getByRole("button", { name: "Remove signature" }));
    await user.selectOptions(from, "acct-work");
    doc = await loadFrame(await signatureFrame(c));
    await waitFor(() => expect(doc.body.textContent).toContain("Engineering Manager, Atlas"));
  });

  it("doesn't add one to a saved draft", async () => {
    const user = userEvent.setup();
    const { backend } = await renderMail();
    const account = screen.getByRole("treeitem", { name: /^Alex Morgan \(/ });
    await user.click(within(account).getByText("Drafts"));
    await user.click(await screen.findByRole("option", { name: /Thoughts on the rollback plan/ }));
    await user.click(await screen.findByRole("button", { name: "Continue editing" }));
    const c = await screen.findByRole("region", { name: "New message" });
    await within(c).findByRole("textbox", { name: "Message body" });
    expect(within(c).queryByTitle("Signature")).not.toBeInTheDocument();
    expect(within(c).queryByRole("button", { name: "Add signature" })).not.toBeInTheDocument();
    await user.click(within(c).getByRole("button", { name: "Send" }));
    expect(await sent(backend as MockBackend)).not.toContain("cumulas-signature");
  });

  it("waits for a signature that's still on its way before sending", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const slow = deferred<ComposeSignature>();
    backend.getComposeSignature = () => slow.promise;
    await renderMail({ backend });
    const c = await newMessage(user);
    await address(user, c);
    expect(within(c).queryByTitle("Signature")).not.toBeInTheDocument();
    await user.click(within(c).getByRole("button", { name: "Send" }));
    expect(within(c).getByRole("button", { name: "Sending…" })).toBeDisabled();
    expect(backend.sent).toHaveLength(0);

    slow.resolve({ kind: "organization", html: "<p>Late, but here</p>" });
    expect(await sent(backend)).toContain(
      '<div class="cumulas-signature"><p>Late, but here</p></div>',
    );
  });

  it("says when the organization signs on send, and adds none", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ signature: "addedOnSend" });
    await renderMail({ backend });
    const c = await newMessage(user);
    expect(
      await within(c).findByText("Your organization adds your signature when you send."),
    ).toBeInTheDocument();
    expect(within(c).queryByTitle("Signature")).not.toBeInTheDocument();
    await address(user, c);
    await user.click(within(c).getByRole("button", { name: "Send" }));
    expect(await sent(backend)).not.toContain("cumulas-signature");
  });
});
