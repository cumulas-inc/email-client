// The bridge (public/message-bridge.js) in a frame of jsdom's: what it reports
// to the window around it, and what it doesn't. Input from a real person
// (trusted events) can't be made here; the desktop end-to-end test clicks,
// points and types in the real webview.

import { afterEach, describe, expect, it } from "vitest";

import bridge from "../../../public/message-bridge.js?raw";

const CHANNEL = "5ec2e7".padEnd(32, "0");

interface Loaded {
  frame: HTMLIFrameElement;
  doc: Document;
  messages: unknown[];
}

let cleanup: (() => void) | null = null;
afterEach(() => {
  cleanup?.();
  cleanup = null;
});

/** A frame running the bridge before `body`, as a message's frame does. */
async function load(body: string): Promise<Loaded> {
  const frame = document.createElement("iframe");
  document.body.append(frame);
  // Everything posted to this window: only the frame posts here. (jsdom's
  // events don't carry the frame's window as their source, as a browser's do.)
  const messages: unknown[] = [];
  const onMessage = (e: MessageEvent) => messages.push(e.data);
  window.addEventListener("message", onMessage);
  cleanup = () => {
    window.removeEventListener("message", onMessage);
    frame.remove();
  };
  const doc = frame.contentDocument!;
  doc.open();
  doc.write(
    `<!doctype html><html><head><script data-channel="${CHANNEL}">${bridge}</script></head>` +
      `<body>${body}</body></html>`,
  );
  doc.close();
  await settle();
  return { frame, doc, messages };
}

/** Lets posted messages arrive. */
const settle = () => new Promise((r) => setTimeout(r, 20));

describe("message bridge", () => {
  it("reports the document's size on its channel, and keeps the channel to itself", async () => {
    const { doc, messages } = await load("<p>Hello</p>");
    expect(messages).toEqual([
      { type: "size", height: 0, wide: false, left: false, right: false, channel: CHANNEL },
    ]);
    // Nothing in the document can read the channel afterwards.
    expect(doc.querySelector("script")!.hasAttribute("data-channel")).toBe(false);
    expect(doc.documentElement.hasAttribute("data-bridge")).toBe(true);
  });

  it("keeps links from navigating the frame, and ignores input no one gave", async () => {
    const { frame, doc, messages } = await load(
      '<p><a id="out" href="https://example.org/x">Out</a> <a href="#below">Below</a></p>' +
        '<p id="below">Below</p>',
    );
    messages.length = 0;
    const view = frame.contentWindow! as Window & typeof globalThis;
    const out = doc.getElementById("out")!;
    const click = new view.MouseEvent("click", { bubbles: true, cancelable: true });
    out.dispatchEvent(click);
    expect(click.defaultPrevented).toBe(true);
    const middle = new view.MouseEvent("auxclick", { bubbles: true, cancelable: true, button: 1 });
    out.dispatchEvent(middle);
    expect(middle.defaultPrevented).toBe(true);
    out.dispatchEvent(new view.MouseEvent("mouseover", { bubbles: true }));
    const key = new view.KeyboardEvent("keydown", {
      key: "Delete",
      bubbles: true,
      cancelable: true,
    });
    doc.body.dispatchEvent(key);
    doc.querySelector<HTMLElement>('a[href="#below"]')!.click();
    await settle();
    // A script that got into the frame could make these, never a person: none
    // is reported.
    expect(messages).toEqual([]);
  });

  it("works whatever names the mail gives its elements", async () => {
    const { frame, doc, messages } = await load(
      '<img name="getElementById"><img name="addEventListener"><img name="documentElement">' +
        '<form name="currentScript"><input name="parentNode"><input name="getAttribute">' +
        '<a href="https://example.org/f">In a form</a></form>' +
        '<a id="parent" href="https://example.org/p">Parent</a><a id="JSON">JSON</a>',
    );
    expect(messages).toEqual([
      { type: "size", height: 0, wide: false, left: false, right: false, channel: CHANNEL },
    ]);
    const view = frame.contentWindow! as Window & typeof globalThis;
    const link = doc.querySelector('form a[href^="https"]')!;
    const click = new view.MouseEvent("click", { bubbles: true, cancelable: true });
    link.dispatchEvent(click);
    expect(click.defaultPrevented).toBe(true);
  });
});
