import { describe, expect, it } from "vitest";

import type { Message } from "../../api/types";
import {
  composeBodyHtml,
  isValidAddress,
  parseAddress,
  parseAddressList,
  parseMailto,
  prefixSubject,
  quoteOriginal,
  replyRecipients,
  splitAddressList,
} from "./draft";

const me = { name: "Alex Morgan", address: "alex.morgan@acme.example" };
const priya = { name: "Priya Raman", address: "priya.raman@acme.example" };
const jonas = { name: "Jonas Becker", address: "jonas.becker@acme.example" };
const mei = { name: "Mei Tanaka", address: "mei.tanaka@acme.example" };

function message(patch: Partial<Message> = {}): Message {
  return {
    id: "m1",
    accountId: "a1",
    folderId: "f1",
    remoteId: "r1",
    conversationId: null,
    subject: "Plans <for> Q3 & beyond",
    from: priya,
    to: [me, mei],
    cc: [jonas],
    preview: "",
    receivedAt: "2026-10-14T09:42:00.000Z",
    isRead: true,
    isFlagged: false,
    isDraft: false,
    hasAttachments: false,
    importance: "normal",
    bcc: [],
    replyTo: [],
    bodyHtml: "<p>Original</p>",
    bodyText: "Original",
    hasBlockedContent: false,
    attachments: [],
    internetMessageId: null,
    inReplyTo: null,
    references: [],
    ...patch,
  };
}

describe("address parsing", () => {
  it("validates addresses", () => {
    expect(isValidAddress("a@b.example")).toBe(true);
    expect(isValidAddress(" a.b+c@sub.b.example ")).toBe(true);
    expect(isValidAddress("a@b")).toBe(false);
    expect(isValidAddress("a b@c.example")).toBe(false);
    expect(isValidAddress("@c.example")).toBe(false);
  });

  it("parses names and angle brackets", () => {
    expect(parseAddress("a@b.example")).toEqual({ name: null, address: "a@b.example" });
    expect(parseAddress("Ann Lee <ann@b.example>")).toEqual({
      name: "Ann Lee",
      address: "ann@b.example",
    });
    expect(parseAddress('"Lee, Ann" <ann@b.example>')).toEqual({
      name: "Lee, Ann",
      address: "ann@b.example",
    });
    expect(parseAddress("mailto:ann@b.example")).toEqual({ name: null, address: "ann@b.example" });
    expect(parseAddress("Ann <nope>")).toBeNull();
  });

  it("splits lists on commas, semicolons and lines, but not inside quotes", () => {
    expect(splitAddressList('"Lee, Ann" <ann@b.example>; bob@b.example,\ncarol@b.example')).toEqual(
      ['"Lee, Ann" <ann@b.example>', "bob@b.example", "carol@b.example"],
    );
    expect(parseAddressList("a@b.example; oops, c@d.example")).toEqual({
      valid: [
        { name: null, address: "a@b.example" },
        { name: null, address: "c@d.example" },
      ],
      invalid: ["oops"],
    });
  });

  it("parses mailto links", () => {
    expect(
      parseMailto(
        "mailto:ann@b.example,bob@b.example?subject=Hi%20there&cc=c@d.example&body=Line+1",
      ),
    ).toEqual({
      to: [
        { name: null, address: "ann@b.example" },
        { name: null, address: "bob@b.example" },
      ],
      subject: "Hi there",
      cc: [{ name: null, address: "c@d.example" }],
      bodyHtml: "<p>Line 1</p>",
    });
    expect(parseMailto("https://example.org")).toBeNull();
  });
});

describe("reply helpers", () => {
  it("prefixes subjects once", () => {
    expect(prefixSubject("Hello", "reply")).toBe("Re: Hello");
    expect(prefixSubject("RE: Hello", "replyAll")).toBe("RE: Hello");
    expect(prefixSubject("AW: Hallo", "reply")).toBe("AW: Hallo");
    expect(prefixSubject("Reminder: today", "reply")).toBe("Re: Reminder: today");
    expect(prefixSubject("Hello", "forward")).toBe("Fwd: Hello");
    expect(prefixSubject("FW: Hello", "forward")).toBe("FW: Hello");
    expect(prefixSubject("Re: Hello", "forward")).toBe("Fwd: Re: Hello");
  });

  it("works out reply recipients without the user's own address", () => {
    const own = [me];
    expect(replyRecipients(message(), "reply", own)).toEqual({ to: [priya], cc: [] });
    expect(replyRecipients(message(), "replyAll", own)).toEqual({ to: [priya, mei], cc: [jonas] });
    expect(replyRecipients(message(), "forward", own)).toEqual({ to: [], cc: [] });
    // Reply-To wins over From.
    const list = { name: "List", address: "list@acme.example" };
    expect(replyRecipients(message({ replyTo: [list] }), "reply", own).to).toEqual([list]);
    // Replying to a message we sent goes to its recipients.
    expect(replyRecipients(message({ from: me, to: [jonas] }), "reply", own).to).toEqual([jonas]);
    // Own addresses match case-insensitively.
    expect(
      replyRecipients(
        message({ cc: [{ name: null, address: "ALEX.MORGAN@acme.example" }] }),
        "replyAll",
        own,
      ).cc,
    ).toEqual([]);
  });

  it("quotes the original with an Outlook-style header", () => {
    const html = quoteOriginal(message());
    expect(html).toContain("<b>From:</b> Priya Raman &lt;priya.raman@acme.example&gt;");
    expect(html).toContain("<b>To:</b> Alex Morgan &lt;alex.morgan@acme.example&gt;; Mei Tanaka");
    expect(html).toContain("<b>Cc:</b> Jonas Becker");
    expect(html).toContain("<b>Subject:</b> Plans &lt;for&gt; Q3 &amp; beyond");
    expect(html).toMatch(/<b>Sent:<\/b> Wednesday, October 14, 2026 \d{1,2}:\d\d [AP]M/);
    expect(html).toContain("<p>Original</p>");
    // Plain-text originals are escaped.
    expect(quoteOriginal(message({ bodyHtml: null, bodyText: "a < b\nc" }))).toContain(
      "a &lt; b<br>c",
    );
  });

  it("builds the sent body from the editor and the quote", () => {
    const body = composeBodyHtml("<p>Hi</p><p></p><p>Bye</p>", "<div>quote</div>");
    expect(body).toContain('<p style="margin:0">Hi</p><p style="margin:0"><br></p>');
    expect(body).toContain("<br><div>quote</div>");
    expect(composeBodyHtml("<p></p>", null)).toContain('<p style="margin:0"><br></p>');
  });
});
