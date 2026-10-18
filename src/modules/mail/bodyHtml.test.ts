import { describe, expect, it } from "vitest";

import {
  bodyAppearance,
  buildSrcDoc,
  classifyLink,
  contentSecurityPolicy,
  linkify,
  messageBridgeUrl,
  newChannel,
  paintsOwnBackground,
  readBridgeMessage,
} from "./bodyHtml";

const BRIDGE = {
  url: "tauri://localhost/message-bridge.js",
  channel: "c0ffee".padEnd(32, "0"),
  inbound: "1b0d".padEnd(32, "0"),
};

describe("message frame document", () => {
  it("blocks everything but inline styles and embedded images by default", () => {
    const csp = contentSecurityPolicy(false);
    expect(csp).toContain("default-src 'none'");
    expect(csp).toContain("script-src 'none'");
    expect(csp).toContain("style-src 'unsafe-inline'");
    expect(csp).toContain("img-src data: cid:;");
    expect(csp).toContain("font-src data:");
    expect(csp).not.toContain("https:");
    expect(contentSecurityPolicy(true)).toContain("img-src data: cid: https: http:");
  });

  it("runs the bridge, by its exact URL, and no other script", () => {
    const csp = contentSecurityPolicy(false, BRIDGE.url);
    const directives = csp.split("; ");
    expect(directives).toContain("default-src 'none'");
    // Exactly one script source: no 'self', 'unsafe-inline', 'unsafe-eval',
    // hashes, nonces or schemes.
    expect(directives.filter((d) => d.startsWith("script-src"))).toEqual([
      "script-src tauri://localhost/message-bridge.js",
    ]);
    expect(csp).not.toContain("unsafe-eval");
    expect(csp).not.toMatch(/script-src[^;]*'unsafe-inline'/);
    expect(csp).toContain("form-action 'none'");
    expect(csp).toContain("base-uri 'none'");

    const doc = buildSrcDoc("<p>Hello</p>", {
      allowRemote: false,
      appearance: "plain",
      bridge: BRIDGE,
    });
    expect(doc).toContain(`content="${csp}"`);
    // The one script element, after the policy and before any of the mail.
    const script = `<script src="${BRIDGE.url}" data-channel="${BRIDGE.channel}" data-inbound="${BRIDGE.inbound}"></script>`;
    expect(doc.match(/<script/g)).toHaveLength(1);
    expect(doc.indexOf(script)).toBeGreaterThan(doc.indexOf("Content-Security-Policy"));
    expect(doc.indexOf(script)).toBeLessThan(doc.indexOf("<style>"));
    expect(doc.indexOf(script)).toBeLessThan(doc.indexOf("<p>Hello</p>"));
    // Measured by the bridge, the frame doesn't scroll; without it, it does.
    expect(doc).toContain("html[data-bridge] { overflow-y: hidden; }");
  });

  it("serves the bridge from the app's own origin", () => {
    expect(messageBridgeUrl("tauri://localhost/")).toBe("tauri://localhost/message-bridge.js");
    expect(messageBridgeUrl("http://localhost:1420/index.html?x=1#mail")).toBe(
      "http://localhost:1420/message-bridge.js",
    );
    expect(messageBridgeUrl()).toBe(new URL("/message-bridge.js", window.location.href).href);
  });

  it("gives each frame document a channel of its own", () => {
    const a = newChannel();
    expect(a).toMatch(/^[0-9a-f]{32}$/);
    expect(newChannel()).not.toBe(a);
  });

  it("wraps the body with the CSP meta tag and readable base styles", () => {
    const doc = buildSrcDoc("<p>Hello</p>", { allowRemote: false, appearance: "paper" });
    expect(doc).toMatch(/^<!doctype html>/);
    expect(doc).toContain(
      `<meta http-equiv="Content-Security-Policy" content="${contentSecurityPolicy(false)}">`,
    );
    expect(doc).toContain("img { max-width: 100%; height: auto; }");
    expect(doc).toContain("overflow-wrap: break-word");
    expect(doc).toContain(`<body dir="auto"><p>Hello</p></body>`);
    // The quiet tone (the original under a reply) sets the text in secondary.
    expect(buildSrcDoc("<p>x</p>", { allowRemote: false, appearance: "plain" })).toMatch(
      /body \{[^}]*color: #1d1c1a;/,
    );
    expect(
      buildSrcDoc("<p>x</p>", { allowRemote: false, appearance: "plain", tone: "quiet" }),
    ).toMatch(/body \{[^}]*color: #57534c;/);
    // Dark mode inverts the page and turns images back.
    expect(buildSrcDoc("", { allowRemote: false, appearance: "dark" })).toContain(
      "filter: invert(1) hue-rotate(180deg)",
    );
  });
});

describe("bridge messages", () => {
  const channel = BRIDGE.channel;
  const read = (data: object) => readBridgeMessage({ channel, ...data }, channel);

  it("reads the messages the bridge sends", () => {
    expect(read({ type: "link", href: "https://example.org" })).toEqual({
      type: "link",
      href: "https://example.org",
    });
    expect(read({ type: "hover", href: "mailto:a@example.org" })).toEqual({
      type: "hover",
      href: "mailto:a@example.org",
    });
    expect(read({ type: "hover", href: null })).toEqual({ type: "hover", href: null });
    const key = { key: "r", code: "KeyR", ctrlKey: true, shiftKey: false, altKey: false };
    expect(read({ type: "key", ...key, metaKey: false })).toEqual({
      type: "key",
      ...key,
      metaKey: false,
    });
    const size = { height: 640.5, wide: false, left: false, right: true };
    expect(read({ type: "size", ...size })).toEqual({ type: "size", ...size });
    expect(read({ type: "scroll", top: 120 })).toEqual({ type: "scroll", top: 120 });
    expect(read({ type: "ready" })).toEqual({ type: "ready" });
    expect(read({ type: "hits", count: 0 })).toEqual({ type: "hits", count: 0 });
    expect(read({ type: "hits", count: 12 })).toEqual({ type: "hits", count: 12 });
    expect(read({ type: "reveal", top: 40, bottom: 58.5 })).toEqual({
      type: "reveal",
      top: 40,
      bottom: 58.5,
    });
    expect(read({ type: "reveal", top: 7, bottom: 7 })).toEqual({
      type: "reveal",
      top: 7,
      bottom: 7,
    });
    // Nothing else comes along.
    expect(read({ type: "link", href: "https://example.org", extra: 1 })).toEqual({
      type: "link",
      href: "https://example.org",
    });
  });

  it("ignores anything else", () => {
    const link = { type: "link", href: "https://example.org" };
    expect(readBridgeMessage(link, channel)).toBeNull();
    expect(readBridgeMessage({ ...link, channel: "other" }, channel)).toBeNull();
    expect(readBridgeMessage({ ...link, channel }, "")).toBeNull();
    for (const bad of [null, undefined, "link", 1, [link]]) {
      expect(readBridgeMessage(bad, channel)).toBeNull();
    }
    expect(read({ type: "link" })).toBeNull();
    expect(read({ type: "link", href: 1 })).toBeNull();
    expect(read({ type: "hover" })).toBeNull();
    expect(read({ type: "key", key: "r", code: "KeyR", ctrlKey: "yes" })).toBeNull();
    expect(read({ type: "size", height: -1, wide: false, left: false, right: false })).toBeNull();
    expect(
      read({ type: "size", height: Infinity, wide: false, left: false, right: false }),
    ).toBeNull();
    expect(read({ type: "size", height: "10", wide: false, left: false, right: false })).toBeNull();
    expect(read({ type: "scroll", top: Number.NaN })).toBeNull();
    expect(read({ type: "hits" })).toBeNull();
    expect(read({ type: "hits", count: -1 })).toBeNull();
    expect(read({ type: "hits", count: 1.5 })).toBeNull();
    expect(read({ type: "hits", count: "3" })).toBeNull();
    expect(read({ type: "reveal", top: 10, bottom: 5 })).toBeNull();
    expect(read({ type: "reveal", top: 0, bottom: Infinity })).toBeNull();
    expect(read({ type: "reveal", top: "0", bottom: 5 })).toBeNull();
    // The app's own commands come back as nothing the app reads.
    expect(read({ type: "highlight", patterns: [] })).toBeNull();
    expect(readBridgeMessage({ type: "ready", inbound: channel }, channel)).toBeNull();
    expect(read({ type: "eval", code: "1" })).toBeNull();
  });
});

describe("dark theme", () => {
  it("tells designed mail (own backgrounds) from mail that only sets text colors", () => {
    expect(paintsOwnBackground('<p style="color:#605e5c">— Priya</p>')).toBe(false);
    expect(
      paintsOwnBackground('<table style="background:#f7f4ee"><tr><td>x</td></tr></table>'),
    ).toBe(true);
    expect(paintsOwnBackground('<td bgcolor="#fff">x</td>')).toBe(true);
    expect(paintsOwnBackground("<style>p { margin: 0 }</style><p>x</p>")).toBe(true);
  });

  it("inverts plain mail and lets the reader choose a sheet for designed mail", () => {
    const plain = "<p>Hi Alex,</p>";
    const designed = '<div style="background-color:#003">News</div>';
    expect(bodyAppearance(plain, false)).toBe("plain");
    expect(bodyAppearance(designed, false)).toBe("plain");
    expect(bodyAppearance(plain, true)).toBe("dark");
    expect(bodyAppearance(designed, true)).toBe("paper");
    expect(bodyAppearance(designed, true, "dark")).toBe("dark");
  });

  it("paints the inverted frame so it comes out in the dark theme's colors", () => {
    const doc = buildSrcDoc("<p>x</p>", { allowRemote: false, appearance: "dark" });
    // invert(1) hue-rotate(180deg) turns these into the card #22211e and ink #edebe6.
    expect(doc).toContain("background: #dfdedb");
    expect(doc).toContain("color: #16140f");
    // And search hits into the dark theme's #5a4512 and #8a6a1a.
    expect(doc).toContain("mark[data-cumulas-hit] { background: #cdb885 !important;");
    expect(doc).toContain(
      "mark[data-cumulas-hit][data-cumulas-current] { background: #b39343 !important;",
    );
  });

  it("marks what the search found in the light theme's hit colors", () => {
    const doc = buildSrcDoc("<p>x</p>", { allowRemote: false, appearance: "plain" });
    expect(doc).toContain(
      "mark[data-cumulas-hit] { background: #fde8a8 !important; color: inherit !important;",
    );
    expect(doc).toContain(
      "mark[data-cumulas-hit][data-cumulas-current] { background: #f6c94c !important; outline: 2px solid #f6c94c; }",
    );
  });
});

describe("links", () => {
  it("only follows web and mail links", () => {
    expect(classifyLink("https://example.org/a?b=1")).toEqual({
      kind: "open",
      url: "https://example.org/a?b=1",
    });
    expect(classifyLink("http://example.org")).toEqual({
      kind: "open",
      url: "http://example.org/",
    });
    expect(classifyLink("mailto:ann@example.org?subject=Hi")).toEqual({
      kind: "mailto",
      url: "mailto:ann@example.org?subject=Hi",
    });
    expect(classifyLink("javascript:alert(1)")).toEqual({ kind: "ignore" });
    expect(classifyLink("file:///etc/passwd")).toEqual({ kind: "ignore" });
    expect(classifyLink("data:text/html,hi")).toEqual({ kind: "ignore" });
    expect(classifyLink("not a url")).toEqual({ kind: "ignore" });
    expect(classifyLink(null)).toEqual({ kind: "ignore" });
  });

  it("linkifies plain text without trailing punctuation", () => {
    expect(
      linkify("See https://example.org/x. Or mail ann@example.org, or www.example.org!"),
    ).toEqual([
      { text: "See " },
      { text: "https://example.org/x", href: "https://example.org/x" },
      { text: ". Or mail " },
      { text: "ann@example.org", href: "mailto:ann@example.org" },
      { text: ", or " },
      { text: "www.example.org", href: "https://www.example.org" },
      { text: "!" },
    ]);
    expect(linkify("<b>not html</b>")).toEqual([{ text: "<b>not html</b>" }]);
  });
});
