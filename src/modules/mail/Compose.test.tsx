import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { MockBackend } from "../../api/mock";
import { ACME_SIGNATURE } from "../../api/mock/data";
import type { ComposeSignature, ProtectionSetup } from "../../api/types";
import { useAppStore } from "../../app/store";
import { useMailStore } from "./mailStore";
import { checkDate } from "./protection";
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
    const letter = LETTER.exec(html)![1]!;
    // The body, then the signature after one empty line: in the letter's
    // font but not its leading, so its table keeps its layout, and marked
    // for the relay as signed.
    expect(html).toBe(
      `<div style="${letter}"><p style="margin:0"><br></p></div>` +
        `<div class="cumulas-signature" data-signature-stamp="1" style="${letter.replace(";line-height:1.5", "")}">` +
        // Sanitized, as the core keeps it.
        `<br>${new DOMParser().parseFromString(ACME_SIGNATURE, "text/html").body.innerHTML}</div>`,
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
    const signature = html.indexOf('<div class="cumulas-signature"');
    expect(signature).toBeGreaterThan(0);
    expect(html.indexOf('<div class="cumulas-quote"')).toBeGreaterThan(signature);
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
      `<div class="cumulas-signature" style="${letter}"><br><div>Alex Morgan</div><div>+1 (206) 555-0199</div></div>`,
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
    expect(html).toMatch(
      /<div class="cumulas-signature" data-signature-stamp="1" [^>]*><br><table/,
    );
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
    // It stays right under the text, where the signature was.
    expect(c.querySelector(".mail-compose__page")).toHaveClass("mail-compose__page--signed");
    await user.click(add);
    expect(await within(c).findByTitle("Signature")).toBeInTheDocument();
    await user.click(within(c).getByRole("button", { name: "Remove signature" }));

    await address(user, c);
    await user.click(within(c).getByRole("button", { name: "Send" }));
    expect(await sent(backend as MockBackend)).not.toContain("cumulas-signature");
  });

  it("brings an edited signature back as edited after removing it", async () => {
    const user = userEvent.setup();
    const { backend } = await renderMail();
    const c = await newMessage(user);
    let doc = await loadFrame(await signatureFrame(c));
    editFrame(doc, "<div>Alex, out of office until Monday</div>");
    await user.click(within(c).getByRole("button", { name: "Remove signature" }));
    await user.click(within(c).getByRole("button", { name: "Add signature" }));
    doc = await loadFrame(await signatureFrame(c));
    expect(doc.body.innerHTML).toBe("<div>Alex, out of office until Monday</div>");
    // Still an edit: the account's own comes back with Restore.
    expect(within(c).getByRole("button", { name: "Restore signature" })).toBeInTheDocument();

    await user.click(within(c).getByRole("button", { name: "Remove signature" }));
    // Removed, it goes without.
    await address(user, c);
    await user.click(within(c).getByRole("button", { name: "Send" }));
    expect(await sent(backend as MockBackend)).not.toContain("out of office");
  });

  it("drops a removed edit when From changes", async () => {
    const user = userEvent.setup();
    await renderMail();
    const c = await newMessage(user);
    const doc = await loadFrame(await signatureFrame(c));
    editFrame(doc, "<div>Edited</div>");
    await user.click(within(c).getByRole("button", { name: "Remove signature" }));
    await user.selectOptions(within(c).getByRole("combobox", { name: "From" }), "acct-home");
    expect(useMailStore.getState().compose?.signature).toBe("auto");
    const home = await loadFrame(await signatureFrame(c));
    await waitFor(() => expect(home.body.textContent).toBe("Alex Morgan+1 (206) 555-0199"));
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
    expect(await sent(backend)).toMatch(
      /<div class="cumulas-signature" [^>]*><br><p>Late, but here<\/p><\/div>/,
    );
  });

  it("sends the signature of the account it's sent from, even if From changes while it waits", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const slow = deferred<ComposeSignature>();
    const original = backend.getComposeSignature.bind(backend);
    backend.getComposeSignature = (accountId, replyKind) =>
      accountId === "acct-work" ? slow.promise : original(accountId, replyKind);
    await renderMail({ backend });
    const c = await newMessage(user);
    await address(user, c);
    await user.click(within(c).getByRole("button", { name: "Send" }));
    // From can't change while Send waits for the signature.
    expect(within(c).getByRole("combobox", { name: "From" })).toBeDisabled();

    // Should the draft's account change all the same, the work account's
    // signature doesn't go out from the personal one.
    act(() => useMailStore.getState().updateCompose({ accountId: "acct-home", signature: "auto" }));
    slow.resolve({ kind: "organization", html: "<p>Acme Corp</p>" });
    const html = await sent(backend);
    expect(backend.sent[0]!.accountId).toBe("acct-home");
    expect(html).not.toContain("Acme Corp");
    expect(html).toContain("<br><div>Alex Morgan</div><div>+1 (206) 555-0199</div>");
    await waitFor(() =>
      expect(screen.queryByRole("region", { name: "New message" })).not.toBeInTheDocument(),
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
    // The note sits right under the text, where a signature would be.
    expect(c.querySelector(".mail-compose__page")).toHaveClass("mail-compose__page--signed");
    await address(user, c);
    await user.click(within(c).getByRole("button", { name: "Send" }));
    expect(await sent(backend)).not.toContain("cumulas-signature");
  });

  it("says the organization signs a reply on send only when it does", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ signature: "addedOnSend" });
    const { client } = await renderMail({ backend });
    const note = "Your organization adds your signature when you send.";
    // The demo's relay signs replies and forwards too.
    await open(user, CHECKLIST);
    await user.click(respondButton("Reply"));
    let c = await screen.findByRole("region", { name: "Reply" });
    expect(await within(c).findByText(note)).toBeInTheDocument();
    await user.click(within(c).getByRole("button", { name: "Discard" }));

    // One that leaves them unsigned: no note, and no own signature either.
    const relay = { state: "addedOnSend", html: null, onReplies: false } as const;
    backend.setOrganizationSignature(
      "acct-work",
      { ...relay, checkedAt: new Date().toISOString() },
      relay,
    );
    await backend.updateOwnSignature("acct-work", "<div>Alex</div>", true);
    // As the app does when the backend says signatures changed.
    await act(() => client.invalidateQueries({ queryKey: ["signatures"] }));
    await user.click(respondButton("Reply"));
    c = await screen.findByRole("region", { name: "Reply" });
    await within(c).findByRole("textbox", { name: "Message body" });
    await waitFor(() => expect(within(c).queryByText(note)).not.toBeInTheDocument());
    expect(within(c).queryByTitle("Signature")).not.toBeInTheDocument();
    expect(within(c).queryByRole("button", { name: "Add signature" })).not.toBeInTheDocument();
  });
});

describe("Encryption and sensitivity labels", () => {
  const GENERAL = "2a7f3c10-8b4e-4f6a-a1d2-3b4c5d6e7f80";
  const CONFIDENTIAL = "5c1e9d72-3f4a-4b8c-9d0e-1f2a3b4c5d60";
  const ALL_EMPLOYEES = "7b2d4e61-9c3a-4e5f-8a7b-6c5d4e3f2a10";
  const ATLAS = "9e8d7c6b-5a4f-4e3d-8c2b-1a0f9e8d7c60";

  const sensitivity = (name: RegExp | string = /^Sensitivity label/) =>
    within(composer()).getByRole("button", { name });
  const encryption = (name: RegExp | string = /^Encryption/) =>
    within(composer()).getByRole("button", { name });

  async function sendNew(user: User, subject = "Numbers") {
    const c = composer();
    await user.type(within(c).getByRole("combobox", { name: "To" }), "ann@example.org{Enter}");
    await user.type(within(c).getByRole("textbox", { name: "Subject" }), subject);
    await user.click(within(c).getByRole("button", { name: "Send" }));
  }

  /** The demo with its checks passed (`?protection=setup`), its setup changed as a test needs. */
  async function withSetup(
    options: ConstructorParameters<typeof MockBackend>[0],
    change?: (setup: ProtectionSetup) => Partial<ProtectionSetup>,
  ) {
    const backend = new MockBackend({ protectedMail: true, ...options });
    if (change) {
      const get = backend.getProtectionSetup.bind(backend);
      vi.spyOn(backend, "getProtectionSetup").mockImplementation(async (id) => {
        const setup = await get(id);
        return { ...setup, ...change(setup) };
      });
    }
    await renderMail({ backend });
    return backend;
  }

  it("offers nothing until a check passed, and says when a label is required", async () => {
    const user = userEvent.setup();
    await renderMail({ backend: new MockBackend({ protectedMail: true }) });
    const c = await newMessage(user);
    // Nothing checked yet: the person's own step, not IT's.
    expect(
      await within(c).findByText(
        "Your organization requires a sensitivity label on email. Cumulas Mail can apply labels for alex.morgan@acme.example after a check in Settings → Accounts shows a test message arrives labeled.",
      ),
    ).toBeInTheDocument();
    expect(within(c).queryByText(/your IT team/)).toBeNull();
    expect(within(c).queryByRole("button", { name: /^Encryption/ })).toBeNull();
    expect(within(c).queryByRole("button", { name: /^Sensitivity label/ })).toBeNull();
  });

  it.each([
    [
      "waiting",
      "Your organization requires a sensitivity label on email. Cumulas Mail is checking its labels for alex.morgan@acme.example; they appear here once a test message arrives labeled.",
    ],
    [
      "expired",
      "Your organization requires a sensitivity label on email. The labels for alex.morgan@acme.example were last checked more than 90 days ago. Check them again in Settings → Accounts to apply them here.",
    ],
    [
      "notApplied",
      "Your organization requires a sensitivity label on email. Cumulas Mail can't apply labels for alex.morgan@acme.example until your IT team sets them up.",
    ],
  ] as const)(
    "says why a required label can't be applied when the checks are %s",
    async (state, hint) => {
      const user = userEvent.setup();
      await withSetup({}, (setup) => ({
        labels: setup.labels.map((l) => ({
          ...l,
          check: {
            state,
            at: "2026-06-01T09:00:00Z",
            note: null,
            testMessage: false,
            passedAt: null,
            otherLabelId: null,
            otherLabelOnControl: false,
            changedAt: null,
          },
        })),
      }));
      const c = await newMessage(user);
      expect(await within(c).findByText(hint)).toBeInTheDocument();
    },
  );

  it("says when the labels were checked before the mail went through the relay", async () => {
    const user = userEvent.setup();
    await withSetup({}, (setup) => ({
      labels: setup.labels.map((l) => ({
        ...l,
        check: {
          state: "expired",
          at: "2026-10-01T09:00:00Z",
          note: "relayChanged",
          testMessage: false,
          passedAt: null,
          otherLabelId: null,
          otherLabelOnControl: false,
          changedAt: null,
        },
      })),
    }));
    const c = await newMessage(user);
    expect(
      await within(c).findByText(
        "Your organization requires a sensitivity label on email. The labels for alex.morgan@acme.example were checked before your organization's mail went through the Cumulas Signatures relay. Check them again in Settings → Accounts to apply them here.",
      ),
    ).toBeInTheDocument();
    expect(within(c).queryByText(/90 days/)).toBeNull();
  });

  it("replies to labeled mail directly when no label is in use from Cumulas Mail", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ protectedMail: true });
    await renderMail({ backend });
    await open(user, /Updated org chart/);
    await user.click(respondButton("Reply"));
    const c = await screen.findByRole("region", { name: "Reply" });
    await user.click(within(c).getByRole("button", { name: "Send" }));
    await waitFor(() => expect(backend.sent).toHaveLength(1));
    expect(within(document.body).queryByText(/The original is labeled/)).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(backend.sent[0]).toMatchObject({ protect: null, labelId: null });
  });

  it("starts with the default label, says what it asks for, and sends it", async () => {
    const user = userEvent.setup();
    const backend = await withSetup({ protection: "setup" });
    const at = (await backend.getProtectionSetup("acct-work")).encrypt.at!;
    await newMessage(user);

    await waitFor(() => expect(sensitivity("Sensitivity label: General")).toBeInTheDocument());
    expect(within(composer()).getByText("General", { selector: ".mail-tag__name" })).toBeVisible();
    expect(
      within(composer()).getByText(
        `Cumulas Mail asks your organization's mail server to apply the label General. A test message on ${checkDate(at)} arrived with it.`,
      ),
    ).toBeInTheDocument();
    // Filling in the default isn't an edit: closing asks nothing.
    expect(useMailStore.getState().compose?.dirty).toBe(false);

    await user.click(encryption("Encryption"));
    await user.click(screen.getByRole("menuitemcheckbox", { name: "Do Not Forward" }));
    expect(encryption("Encryption: Do Not Forward")).toHaveTextContent("Do Not Forward");
    expect(
      within(composer()).getByText(
        `Cumulas Mail asks your organization's mail server to apply Do Not Forward to this message. A test message on ${checkDate(at)} arrived encrypted. Your copy in Sent Items won't show it.`,
      ),
    ).toBeInTheDocument();
    await user.click(sensitivity());
    await user.click(screen.getByRole("menuitemcheckbox", { name: "Confidential" }));

    await sendNew(user);
    await waitFor(() => expect(backend.sent).toHaveLength(1));
    expect(backend.sent[0]).toMatchObject({ protect: "doNotForward", labelId: CONFIDENTIAL });
  });

  it("lets a label that encrypts bring its own encryption", async () => {
    const user = userEvent.setup();
    const backend = await withSetup({ protection: "setup" });
    await newMessage(user);
    await waitFor(() => expect(sensitivity("Sensitivity label: General")).toBeInTheDocument());

    await user.click(encryption());
    await user.click(screen.getByRole("menuitemcheckbox", { name: "Encrypt" }));
    await user.click(sensitivity());
    await user.click(
      screen.getByRole("menuitemcheckbox", { name: "Confidential › All Employees" }),
    );
    const button = encryption("Encryption");
    expect(button).toBeDisabled();
    expect(button).toHaveAttribute(
      "title",
      "The label Confidential › All Employees encrypts this message.",
    );
    expect(within(composer()).getByText(/The label encrypts the message\.$/)).toBeInTheDocument();

    await sendNew(user);
    await waitFor(() => expect(backend.sent).toHaveLength(1));
    expect(backend.sent[0]).toMatchObject({ protect: null, labelId: ALL_EMPLOYEES });
  });

  it("needs a label when the organization requires one", async () => {
    const user = userEvent.setup();
    // Its default can't be applied, so nothing is filled in.
    const backend = new MockBackend({ protectedMail: true, protection: "setup" });
    backend.setLabelPolicy("acct-work", { defaultLabelId: ATLAS });
    await renderMail({ backend });
    await newMessage(user);
    await waitFor(() => expect(sensitivity("Sensitivity label")).toBeInTheDocument());
    await user.click(sensitivity());
    // No "No label" when one is required.
    expect(screen.queryByRole("menuitemcheckbox", { name: "No label" })).toBeNull();
    await user.keyboard("{Escape}");

    await sendNew(user);
    expect(within(composer()).getByRole("alert")).toHaveTextContent(
      "Choose a sensitivity label. Your organization requires one.",
    );
    expect(sensitivity()).toHaveFocus();
    expect(backend.sent).toHaveLength(0);

    await user.click(sensitivity());
    await user.click(screen.getByRole("menuitemcheckbox", { name: "General" }));
    expect(within(composer()).queryByRole("alert")).toBeNull();
    await user.click(within(composer()).getByRole("button", { name: "Send" }));
    await waitFor(() => expect(backend.sent).toHaveLength(1));
    expect(backend.sent[0]!.labelId).toBe(GENERAL);
  });

  it("keeps a reply's label, and won't lower it", async () => {
    const user = userEvent.setup();
    // Public passed its check too, here.
    const backend = await withSetup({ protection: "setup" }, (setup) => ({
      labels: setup.labels.map((l) =>
        l.name === "Public"
          ? { ...l, check: { ...l.check, state: "applied", note: null, passedAt: l.check.at } }
          : l,
      ),
    }));
    await open(user, /Updated org chart/);
    await user.click(respondButton("Reply"));
    await screen.findByRole("region", { name: "Reply" });
    await waitFor(() => expect(sensitivity("Sensitivity label: General")).toBeInTheDocument());
    expect(
      within(composer()).getByText(
        "Your organization asks for a reason to lower a label, which Cumulas Mail can't give. To lower it, use Outlook on the web.",
      ),
    ).toBeInTheDocument();
    await user.click(sensitivity());
    expect(screen.getByRole("menuitemcheckbox", { name: "Public" })).toBeDisabled();
    expect(screen.getByRole("menuitemcheckbox", { name: "Confidential" })).toBeEnabled();
    await user.keyboard("{Escape}");

    await user.click(within(composer()).getByRole("button", { name: "Send" }));
    await waitFor(() => expect(backend.sent).toHaveLength(1));
    expect(backend.sent[0]).toMatchObject({ labelId: GENERAL, replyKind: "reply" });
  });

  it("makes a label be chosen when the original's can't be kept and one is required", async () => {
    const user = userEvent.setup();
    const backend = await withSetup({ protection: "setup" });
    await open(user, /Atlas public release notes/);
    await user.click(respondButton("Reply"));
    const c = await screen.findByRole("region", { name: "Reply" });
    // Labeled Public, which Cumulas Mail can't apply here: nothing filled in.
    await waitFor(() =>
      expect(
        within(c).getByText(
          "The original is labeled Public. Cumulas Mail can't apply that label, so this reply might not keep it.",
        ),
      ).toBeInTheDocument(),
    );
    expect(sensitivity("Sensitivity label")).toBeInTheDocument();

    await user.click(within(c).getByRole("button", { name: "Send" }));
    expect(within(c).getByRole("alert")).toHaveTextContent(
      "The original is labeled Public. Cumulas Mail can't apply that label, so choose one for this reply. Your organization requires one.",
    );
    // The choice opens; there's no sending without one.
    const menu = await screen.findByRole("menu", { name: "Sensitivity label" });
    expect(screen.queryByRole("dialog", { name: "Send without the label?" })).toBeNull();
    expect(backend.sent).toHaveLength(0);
    await user.click(within(menu).getByRole("menuitemcheckbox", { name: "General" }));

    await user.click(within(c).getByRole("button", { name: "Send" }));
    await waitFor(() => expect(backend.sent).toHaveLength(1));
    expect(backend.sent[0]!.labelId).toBe(GENERAL);
  });

  it("sends a reply Outlook on the web only can when every label here is lower", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ protectedMail: true, protection: "setup" });
    // The release notes, labeled Project Atlas: above every label offered.
    const notes = (
      backend as unknown as { state: { messages: import("../../api/types").Message[] } }
    ).state.messages.find((m) => m.id === "msg-protected-8")!;
    notes.label = {
      id: ATLAS,
      name: "Highly Confidential › Project Atlas",
      color: "#a4262c",
      fromCatalog: true,
    };
    await renderMail({ backend });
    const openInOutlook = vi.spyOn(backend, "openInOutlook");
    await open(user, /Atlas public release notes/);
    await user.click(respondButton("Reply"));
    const c = await screen.findByRole("region", { name: "Reply" });
    await waitFor(() => expect(sensitivity("Sensitivity label")).toBeInTheDocument());

    await user.click(within(c).getByRole("button", { name: "Send" }));
    expect(within(c).getByRole("alert")).toHaveTextContent(
      "The original is labeled Highly Confidential › Project Atlas. Cumulas Mail can't apply that label or one at least as sensitive, so send this reply from Outlook on the web.",
    );
    // No menu of labels that can't be chosen: the way out instead.
    expect(screen.queryByRole("menu", { name: "Sensitivity label" })).toBeNull();
    await user.click(
      within(c).getByRole("button", { name: "Open original in Outlook on the web" }),
    );
    expect(openInOutlook).toHaveBeenCalledWith("msg-protected-8");
    expect(backend.sent).toHaveLength(0);
  });

  it("replies to a Sent copy with the label it asked for, and won't lower it", async () => {
    const user = userEvent.setup();
    const backend = await withSetup({ protection: "setup" });
    const account = screen.getByRole("treeitem", { name: /^Alex Morgan \(/ });
    await user.click(within(account).getByText("Sent Items"));
    await open(user, /Re: Atlas pricing/);
    await user.click(respondButton("Reply all"));
    await screen.findByRole("region", { name: "Reply" });
    // Not the default (General): the Confidential the message asked for.
    await waitFor(() => expect(sensitivity("Sensitivity label: Confidential")).toBeInTheDocument());
    await user.click(sensitivity());
    expect(screen.getByRole("menuitemcheckbox", { name: "General" })).toBeDisabled();
    await user.keyboard("{Escape}");
    await user.click(within(composer()).getByRole("button", { name: "Send" }));
    await waitFor(() => expect(backend.sent).toHaveLength(1));
    expect(backend.sent[0]).toMatchObject({ labelId: CONFIDENTIAL, replyKind: "replyAll" });
  });

  /** The demo's saved draft, labeled as Outlook saved it. */
  async function editLabeledDraft(
    user: User,
    backend: MockBackend,
    label: { id: string; name: string },
  ) {
    const get = backend.getMessage.bind(backend);
    vi.spyOn(backend, "getMessage").mockImplementation(async (id, allow) => {
      const m = await get(id, allow);
      return m.isDraft ? { ...m, label: { ...label, color: null, fromCatalog: true } } : m;
    });
    const account = screen.getByRole("treeitem", { name: /^Alex Morgan \(/ });
    await user.click(within(account).getByText("Drafts"));
    await user.click(await screen.findByRole("option", { name: /Thoughts on the rollback plan/ }));
    await user.click(await screen.findByRole("button", { name: "Continue editing" }));
    return screen.findByRole("region", { name: "New message" });
  }

  it("keeps an edited draft's own label rather than the default, and won't lower it", async () => {
    const user = userEvent.setup();
    const backend = await withSetup({ protection: "setup" });
    await editLabeledDraft(user, backend, { id: CONFIDENTIAL, name: "Confidential" });
    await waitFor(() => expect(sensitivity("Sensitivity label: Confidential")).toBeInTheDocument());
    await user.click(sensitivity());
    expect(screen.getByRole("menuitemcheckbox", { name: "General" })).toBeDisabled();
    await user.keyboard("{Escape}");
    await user.click(within(composer()).getByRole("button", { name: "Send" }));
    await waitFor(() => expect(backend.sent).toHaveLength(1));
    expect(backend.sent[0]).toMatchObject({ labelId: CONFIDENTIAL, replyKind: null });
  });

  it("says when an edited draft's label can't be applied, rather than lowering it", async () => {
    const user = userEvent.setup();
    const backend = await withSetup({ protection: "setup" });
    const c = await editLabeledDraft(user, backend, {
      id: ATLAS,
      name: "Highly Confidential › Project Atlas",
    });
    await waitFor(() => expect(sensitivity("Sensitivity label")).toBeInTheDocument());
    // Nothing filled in: not the default.
    expect(useMailStore.getState().compose?.labelId).toBeNull();
    await user.click(within(c).getByRole("button", { name: "Send" }));
    expect(within(c).getByRole("alert")).toHaveTextContent(
      "This draft is labeled Highly Confidential › Project Atlas. Cumulas Mail can't apply that label or one at least as sensitive, so send this message from Outlook on the web.",
    );
    expect(backend.sent).toHaveLength(0);
  });

  it("keeps a choice that stopped being offered in view, and won't send it", async () => {
    const user = userEvent.setup();
    let failed = false;
    const backend = new MockBackend({ protectedMail: true, protection: "setup" });
    const get = backend.getProtectionSetup.bind(backend);
    vi.spyOn(backend, "getProtectionSetup").mockImplementation(async (id) => {
      const setup = await get(id);
      // Checked again meanwhile: it didn't pass.
      return failed
        ? {
            ...setup,
            encrypt: { ...setup.encrypt, state: "notApplied", passedAt: null, testMessage: true },
          }
        : setup;
    });
    const { client } = await renderMail({ backend });
    const c = await newMessage(user);
    await waitFor(() => expect(sensitivity("Sensitivity label: General")).toBeInTheDocument());
    await user.click(encryption());
    await user.click(screen.getByRole("menuitemcheckbox", { name: "Encrypt" }));
    failed = true;
    await act(() => client.invalidateQueries());
    await waitFor(() =>
      expect(encryption("Encryption: Encrypt (not available now)")).toBeInTheDocument(),
    );

    await user.type(within(c).getByRole("combobox", { name: "To" }), "ann@example.org{Enter}");
    await user.type(within(c).getByRole("textbox", { name: "Subject" }), "Numbers");
    await user.click(within(c).getByRole("button", { name: "Send" }));
    expect(within(c).getByRole("alert")).toHaveTextContent(
      "Encrypt can't be applied: its latest check in Settings → Accounts didn't pass. Choose Don't encrypt.",
    );
    expect(encryption()).toHaveFocus();
    expect(backend.sent).toHaveLength(0);

    await user.click(encryption());
    expect(
      screen.getByRole("menuitemcheckbox", { name: "Encrypt (not available now)" }),
    ).toBeDisabled();
    await user.click(screen.getByRole("menuitemcheckbox", { name: "Don't encrypt" }));
    // Cleared: only what's offered is left to choose.
    await user.click(encryption("Encryption"));
    expect(screen.queryByRole("menuitemcheckbox", { name: /not available/ })).toBeNull();
    expect(screen.queryByRole("menuitemcheckbox", { name: "Encrypt" })).toBeNull();
    await user.keyboard("{Escape}");
    await user.click(within(c).getByRole("button", { name: "Send" }));
    await waitFor(() => expect(backend.sent).toHaveLength(1));
    expect(backend.sent[0]).toMatchObject({ protect: null, labelId: GENERAL });
  });

  it("won't send a choice checked before the mail went through the relay", async () => {
    const user = userEvent.setup();
    let relay = false;
    const backend = new MockBackend({ protectedMail: true, protection: "setup" });
    const get = backend.getProtectionSetup.bind(backend);
    vi.spyOn(backend, "getProtectionSetup").mockImplementation(async (id) => {
      const setup = await get(id);
      // The organization's mail goes through the relay now: the pass doesn't count.
      return relay
        ? {
            ...setup,
            encrypt: { ...setup.encrypt, state: "expired", note: "relayChanged", passedAt: null },
          }
        : setup;
    });
    const { client } = await renderMail({ backend });
    const c = await newMessage(user);
    await waitFor(() => expect(sensitivity("Sensitivity label: General")).toBeInTheDocument());
    await user.click(encryption());
    await user.click(screen.getByRole("menuitemcheckbox", { name: "Encrypt" }));
    relay = true;
    await act(() => client.invalidateQueries());
    await waitFor(() =>
      expect(encryption("Encryption: Encrypt (not available now)")).toBeInTheDocument(),
    );

    await sendNew(user);
    expect(within(c).getByRole("alert")).toHaveTextContent(
      "Encrypt was checked before your organization's mail went through the Cumulas Signatures relay, so it can't be applied. Check it again in Settings → Accounts, or choose Don't encrypt.",
    );
    expect(backend.sent).toHaveLength(0);
  });

  it("asks before dropping the original's label when none is required", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ protectedMail: true, protection: "setup" });
    backend.setLabelPolicy("acct-work", { required: false });
    await renderMail({ backend });
    const openInOutlook = vi.spyOn(backend, "openInOutlook");
    await open(user, /Atlas public release notes/);
    await user.click(respondButton("Reply"));
    const c = await screen.findByRole("region", { name: "Reply" });
    await within(c).findByText(/The original is labeled Public/);

    await user.click(within(c).getByRole("button", { name: "Send" }));
    let dialog = await screen.findByRole("dialog", { name: "Send without the label?" });
    expect(dialog).toHaveTextContent(
      "The original is labeled Public. Cumulas Mail can't apply that label, so this reply might not keep it.",
    );
    await user.click(
      within(dialog).getByRole("button", { name: "Open original in Outlook on the web" }),
    );
    expect(openInOutlook).toHaveBeenCalledWith("msg-protected-8");
    expect(backend.sent).toHaveLength(0);

    await user.click(within(c).getByRole("button", { name: "Send" }));
    dialog = await screen.findByRole("dialog", { name: "Send without the label?" });
    await user.click(within(dialog).getByRole("button", { name: "Send anyway" }));
    await waitFor(() => expect(backend.sent).toHaveLength(1));
    expect(backend.sent[0]).toMatchObject({ labelId: null, replyToMessageId: "msg-protected-8" });
  });

  it("starts again from the new account's options when From changes", async () => {
    const user = userEvent.setup();
    await withSetup({ protection: "setup" });
    const c = await newMessage(user);
    await waitFor(() => expect(sensitivity("Sensitivity label: General")).toBeInTheDocument());
    await user.click(sensitivity());
    await user.click(screen.getByRole("menuitemcheckbox", { name: "Confidential" }));

    const from = within(c).getByRole("combobox", { name: "From" });
    await user.selectOptions(from, "acct-home");
    expect(within(c).queryByRole("button", { name: /^Sensitivity label/ })).toBeNull();
    expect(useMailStore.getState().compose).toMatchObject({
      protect: null,
      labelId: null,
      labelChosen: false,
    });
    await user.selectOptions(from, "acct-work");
    await waitFor(() => expect(sensitivity("Sensitivity label: General")).toBeInTheDocument());
  });

  it("doesn't offer an option whose check is too old", async () => {
    const user = userEvent.setup();
    await withSetup({ protection: "expired" });
    await newMessage(user);
    await waitFor(() => expect(encryption("Encryption")).toBeInTheDocument());
    await user.click(encryption());
    expect(screen.queryByRole("menuitemcheckbox", { name: "Encrypt" })).toBeNull();
    expect(screen.getByRole("menuitemcheckbox", { name: "Do Not Forward" })).toBeInTheDocument();
  });

  it.each([
    [
      "labelCopied",
      "This reply already carries the label General from the original, and Cumulas Mail can't make sure your organization's mail server applies Confidential instead. Choose General, or send it from Outlook on the web.",
    ],
    [
      "headersNotKept",
      "Cumulas Mail couldn't add the label or encryption to this reply, so it wasn't sent. Send it from Outlook on the web instead.",
    ],
  ])("says why a %s send stopped, and keeps the draft", async (reason, message) => {
    const user = userEvent.setup();
    const backend = await withSetup({ protection: "setup" });
    vi.spyOn(backend, "sendMessage").mockRejectedValue({
      code: reason === "labelCopied" ? "invalidInput" : "provider",
      message,
      details: { reason, labelId: GENERAL },
    });
    await open(user, /Updated org chart/);
    await user.click(respondButton("Reply"));
    const c = await screen.findByRole("region", { name: "Reply" });
    await waitFor(() => expect(sensitivity("Sensitivity label: General")).toBeInTheDocument());
    await user.click(within(c).getByRole("button", { name: "Send" }));
    expect(await within(c).findByRole("alert")).toHaveTextContent(message);
    expect(composer("Reply")).toBeInTheDocument();
    expect(screen.queryByText("Message sent")).toBeNull();
  });

  it("hands the reply to Outlook on the web when the backend finds the original encrypted", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    vi.spyOn(backend, "sendMessage").mockRejectedValue({
      code: "unsupported",
      message: "This message is protected. Reply to it or forward it in Outlook on the web.",
      details: { reason: "protectedOriginal", messageId: "msg-1" },
    });
    await renderMail({ backend });
    await open(user, CHECKLIST);
    await user.click(respondButton("Forward"));
    const c = await screen.findByRole("region", { name: "Forward" });
    await user.type(within(c).getByRole("combobox", { name: "To" }), "ann@example.org{Enter}");
    await user.click(within(c).getByRole("button", { name: "Send" }));
    const dialog = await screen.findByRole("dialog", { name: "Forward in Outlook on the web" });
    // How it's protected wasn't said: no promise about what Outlook on the web keeps.
    expect(dialog).toHaveTextContent(
      "This message is encrypted. Cumulas Mail can't forward it without removing the encryption, so open it in Outlook on the web.",
    );
  });
});
