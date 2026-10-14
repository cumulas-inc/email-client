import { describe, expect, it } from "vitest";

import {
  bodyAppearance,
  buildSrcDoc,
  classifyLink,
  contentSecurityPolicy,
  linkify,
  paintsOwnBackground,
} from "./bodyHtml";

describe("message frame document", () => {
  it("blocks everything but inline styles and embedded images by default", () => {
    const csp = contentSecurityPolicy(false);
    expect(csp).toContain("default-src 'none'");
    expect(csp).toContain("style-src 'unsafe-inline'");
    expect(csp).toContain("img-src data: cid:;");
    expect(csp).toContain("font-src data:");
    expect(csp).not.toContain("https:");
    expect(contentSecurityPolicy(true)).toContain("img-src data: cid: https: http:");
  });

  it("wraps the body with the CSP meta tag and readable base styles", () => {
    const doc = buildSrcDoc("<p>Hello</p>", { allowRemote: false, appearance: "paper" });
    expect(doc).toMatch(/^<!doctype html>/);
    expect(doc).toContain(
      `<meta http-equiv="Content-Security-Policy" content="${contentSecurityPolicy(false)}">`,
    );
    expect(doc).toContain("img { max-width: 100%; height: auto; }");
    expect(doc).toContain("overflow-wrap: break-word");
    expect(doc).toContain("<body><p>Hello</p></body>");
    // Dark mode inverts the page and turns images back.
    expect(buildSrcDoc("", { allowRemote: false, appearance: "dark" })).toContain(
      "filter: invert(1) hue-rotate(180deg)",
    );
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
