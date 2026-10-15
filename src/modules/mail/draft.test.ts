import { describe, expect, it } from "vitest";

import type { Account, Message } from "../../api/types";
import {
  attributionLine,
  composeBodyHtml,
  isValidAddress,
  newDraft,
  parseAddress,
  parseAddressList,
  parseMailto,
  prefixSubject,
  quoteOriginal,
  replyDraft,
  replyRecipients,
  splitAddressList,
  stripSignatureStamps,
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

  it("quotes a reply under one attribution line, in a cited blockquote", () => {
    const html = quoteOriginal(message(), "reply");
    expect(html).toMatch(
      /^<div class="cumulas-quote"><p style="margin:0 0 8px">On Wed, Oct 14, 2026, at \d{1,2}:\d\d [AP]M, Priya Raman wrote:<\/p><blockquote type="cite" style="[^"]*border-left:2px solid #d8d4cb[^"]*"><p>Original<\/p><\/blockquote><\/div>$/,
    );
    // None of Outlook's bold From/Sent/To/Subject block, or its rule.
    expect(html).not.toMatch(/<b>|<hr|Sent:|Subject:/);
    // Reply all quotes the same way; reply is the default.
    expect(quoteOriginal(message(), "replyAll")).toBe(html);
    expect(quoteOriginal(message())).toBe(html);
    // Plain-text originals are escaped.
    expect(quoteOriginal(message({ bodyHtml: null, bodyText: "a < b\nc" }))).toContain(
      "<blockquote",
    );
    expect(quoteOriginal(message({ bodyHtml: null, bodyText: "a < b\nc" }))).toContain(
      "a &lt; b<br>c</blockquote>",
    );
  });

  it("names the sender as well as it can in the attribution", () => {
    expect(attributionLine(message())).toMatch(
      /^On Wed, Oct 14, 2026, at \d{1,2}:\d\d [AP]M, Priya Raman wrote:$/,
    );
    // No name: the address. No sender at all: "the sender".
    expect(attributionLine(message({ from: { name: null, address: "ops@acme.example" } }))).toMatch(
      /, ops@acme\.example wrote:$/,
    );
    expect(attributionLine(message({ from: null }))).toMatch(/^On .*, the sender wrote:$/);
    // A message without a date (1 Jan 1970) gets no "On".
    expect(attributionLine(message({ receivedAt: "1970-01-01T00:00:00.000Z" }))).toBe(
      "Priya Raman wrote:",
    );
    expect(attributionLine(message({ from: null, receivedAt: "1970-01-01T00:00:00.000Z" }))).toBe(
      "The sender wrote:",
    );
    // Names are escaped in the quote.
    expect(
      quoteOriginal(message({ from: { name: "Ann <Ops> & Co", address: "ann@acme.example" } })),
    ).toContain("Ann &lt;Ops&gt; &amp; Co wrote:");
  });

  it("forwards under a compact header, without quoting the body", () => {
    const html = quoteOriginal(message(), "forward");
    expect(html).toMatch(/^<div class="cumulas-quote cumulas-forward">/);
    expect(html).toContain(">Forwarded message</div>");
    const rows = Array.from(
      html.matchAll(/<span style="[^"]*">(\w+)<\/span>(.*?)<\/div>/g),
      (m) => [m[1], m[2]],
    );
    expect(rows).toEqual([
      ["From", "Priya Raman &lt;priya.raman@acme.example&gt;"],
      ["Date", expect.stringMatching(/^Wed, Oct 14, 2026, at \d{1,2}:\d\d [AP]M$/)],
      ["Subject", "Plans &lt;for&gt; Q3 &amp; beyond"],
      [
        "To",
        "Alex Morgan &lt;alex.morgan@acme.example&gt;, Mei Tanaka &lt;mei.tanaka@acme.example&gt;",
      ],
      ["Cc", "Jonas Becker &lt;jonas.becker@acme.example&gt;"],
    ]);
    // Quiet labels, not Outlook's bold ones; the original follows unindented.
    expect(html).not.toMatch(/<b>|<strong>|<hr|<blockquote/);
    expect(html).toMatch(/<div><p>Original<\/p><\/div><\/div>$/);
    // Missing parts are left out rather than shown empty.
    const bare = quoteOriginal(
      message({ from: null, subject: " ", cc: [], receivedAt: "1970-01-01T00:00:00.000Z" }),
      "forward",
    );
    expect(bare).not.toMatch(/>(From|Date|Subject|Cc)<\/span>/);
    expect(bare).toMatch(/>To<\/span>/);
  });

  it("strips signature stamps from the quoted original", () => {
    const stamped =
      '<div data-signature-stamp="v2:abc"><p>Original</p></div>' +
      "<p DATA-SIGNATURE-STAMP>Second</p>" +
      '<span data-signature-stamp=x title="keep">Third</span>';
    const clean = stripSignatureStamps(stamped);
    expect(clean).not.toMatch(/data-signature-stamp/i);
    expect(clean).toBe('<div><p>Original</p></div><p>Second</p><span title="keep">Third</span>');
    // Text that mentions the marker is the sender's words: left alone.
    expect(stripSignatureStamps("<p>Set data-signature-stamp on the div</p>")).toBe(
      "<p>Set data-signature-stamp on the div</p>",
    );
    // Untouched (not reserialized) when there's nothing to strip.
    expect(stripSignatureStamps("<P>Hi<br/>there")).toBe("<P>Hi<br/>there");
    // Inside a <template>, too.
    expect(stripSignatureStamps('<template><b data-signature-stamp="1">x</b></template>')).toBe(
      "<template><b>x</b></template>",
    );

    // Replies and forwards both quote the stripped original.
    for (const kind of ["reply", "replyAll", "forward"] as const) {
      const html = quoteOriginal(message({ bodyHtml: stamped }), kind);
      expect(html).not.toMatch(/data-signature-stamp/i);
      expect(html).toContain('<span title="keep">Third</span>');
    }
  });

  it("starts a reply with the quote and the cursor in the body", () => {
    const accounts = [{ displayName: me.name, email: me.address }] as Account[];
    const reply = replyDraft(message(), "replyAll", accounts);
    expect(reply).toMatchObject({
      to: [priya, mei],
      cc: [jonas],
      showCc: true,
      subject: "Re: Plans <for> Q3 & beyond",
      importance: "normal",
      replyKind: "replyAll",
      focus: "body",
    });
    expect(reply.quoteHtml).toContain('<blockquote type="cite"');
    const forward = replyDraft(message(), "forward", accounts);
    expect(forward).toMatchObject({ to: [], subject: "Fwd: Plans <for> Q3 & beyond", focus: "to" });
    expect(forward.quoteHtml).toContain("Forwarded message");
  });

  it("builds the sent body from the editor and the quote", () => {
    const body = composeBodyHtml("<p>Hi</p><p></p><p>Bye</p>", null, "<div>quote</div>");
    expect(body).toContain('<p style="margin:0">Hi</p><p style="margin:0"><br></p>');
    expect(body).toContain("<br><div>quote</div>");
    expect(composeBodyHtml("<p></p>", null, null)).toContain('<p style="margin:0"><br></p>');
  });

  it("puts the signature after one empty line, above the quote", () => {
    const style = /^<div style="(font-family:'Charis SIL'[^"]*)">/.exec(
      composeBodyHtml("<p>Hi</p>", null, null),
    )![1];
    const quote = '<div class="cumulas-quote"><p>Original</p></div>';
    const organization = '<table style="font-family:Arial"><tr><td>Ada</td></tr></table>';

    // An organization's keeps its own fonts: not in the letter's style.
    expect(composeBodyHtml("<p>Hi</p>", { html: organization, own: false }, quote)).toBe(
      `<div style="${style}"><p style="margin:0">Hi</p><p style="margin:0"><br></p></div>` +
        `<div class="cumulas-signature">${organization}</div>` +
        `<div style="${style}"><br>${quote}</div>`,
    );
    // The person's own is set like the letter, so plain text matches it.
    expect(composeBodyHtml("<p>Hi</p>", { html: "<div>Ada</div>", own: true }, null)).toBe(
      `<div style="${style}"><p style="margin:0">Hi</p><p style="margin:0"><br></p></div>` +
        `<div class="cumulas-signature" style="${style}"><div>Ada</div></div>`,
    );
    // A signature emptied in the editor is none.
    expect(composeBodyHtml("<p>Hi</p>", { html: "<div><br></div>", own: true }, quote)).toBe(
      composeBodyHtml("<p>Hi</p>", null, quote),
    );
    // A signature that's only a picture is one.
    expect(
      composeBodyHtml(
        "<p>Hi</p>",
        { html: '<img src="data:image/png;base64,AA">', own: false },
        null,
      ),
    ).toContain('<div class="cumulas-signature"><img');
  });

  it("starts new messages and replies following the account's signature", () => {
    const accounts = [{ displayName: me.name, email: me.address }] as Account[];
    expect(replyDraft(message(), "reply", accounts).signature).toBe("auto");
    expect(newDraft("a1").signature).toBe("auto");
    expect(newDraft("a1", { signature: "off" }).signature).toBe("off");
  });
});
