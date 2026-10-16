// The bridge (public/message-bridge.js) in a frame of jsdom's: what it reports
// to the window around it, and what it doesn't. jsdom trusts only the events
// it fires itself, so input from a person is made here by marking an event
// trusted in jsdom's own state; the desktop end-to-end test clicks, points
// and types in the real webview.

import { createRequire } from "node:module";

import { afterEach, describe, expect, it } from "vitest";

import bridge from "../../../public/message-bridge.js?raw";

const CHANNEL = "5ec2e7".padEnd(32, "0");

interface Impl {
  isTrusted: boolean;
  _dispatch(event: Impl): boolean;
}
const { implForWrapper } = createRequire(import.meta.url)("jsdom/lib/generated/idl/utils.js") as {
  implForWrapper(wrapper: object): Impl;
};

/**
 * Dispatches `event` at `target` as if a person gave it, as jsdom fires its
 * own events (dispatchEvent marks every event it dispatches untrusted).
 */
function give<E extends Event>(target: EventTarget, event: E): E {
  const impl = implForWrapper(event);
  impl.isTrusted = true;
  implForWrapper(target)._dispatch(impl);
  return event;
}

/** What a script that got into the frame could do: make `name` read `value`. */
function forge(proto: object, name: string, value: unknown) {
  Object.defineProperty(proto, name, { configurable: true, get: () => value });
}

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

  /** The frame's window, and events made in it as a person would. */
  function person(frame: HTMLIFrameElement) {
    const view = frame.contentWindow! as Window & typeof globalThis;
    const mouse = { bubbles: true, cancelable: true, view };
    return {
      view,
      point: (el: Element) => give(el, new view.MouseEvent("mouseover", mouse)),
      /** The pointer leaves `el` for `to`, or for outside the frame. */
      leave: (el: Element, to: Element | null) =>
        give(el, new view.MouseEvent("mouseout", { ...mouse, relatedTarget: to })),
      press: (el: Element, button = 0) =>
        give(el, new view.PointerEvent("pointerdown", { ...mouse, button })),
      /** A click: by the pointer (`detail` 1), or by Enter or a screen reader (0). */
      click: (el: Element, init: MouseEventInit = {}) =>
        give(
          el,
          new view.MouseEvent(init.button ? "auxclick" : "click", { ...mouse, detail: 1, ...init }),
        ),
      key: (el: Element, init: KeyboardEventInit) =>
        give(el, new view.KeyboardEvent("keydown", { ...mouse, ...init })),
    };
  }

  const link = (href: string) => ({ type: "link", href, channel: CHANNEL });
  const hover = (href: string | null) => ({ type: "hover", href, channel: CHANNEL });

  it("reports what a person does: links followed and pointed at, keys pressed", async () => {
    const { frame, doc, messages } = await load(
      '<p><a id="out" href="https://example.org/x"><b>Out</b></a> <span id="text">Text</span></p>',
    );
    messages.length = 0;
    const { point, leave, press, click, key } = person(frame);
    const out = doc.querySelector("#out b")!;
    const text = doc.getElementById("text")!;

    point(out);
    // Out of the frame and back: the status line clears and comes back.
    leave(out, null);
    point(out);
    press(out);
    expect(click(out).defaultPrevented).toBe(true);
    // A middle click opens it too; a right click doesn't.
    press(out, 1);
    click(out, { button: 1 });
    press(out, 2);
    click(out, { button: 2 });
    point(text);
    // Ctrl+R is the app's (a reply), so the frame doesn't act on it as well;
    // Ctrl+C copies in the frame.
    const reply = key(text, { key: "r", code: "KeyR", ctrlKey: true });
    const copy = key(text, { key: "c", code: "KeyC", ctrlKey: true });
    await settle();
    expect(reply.defaultPrevented).toBe(true);
    expect(copy.defaultPrevented).toBe(false);
    const keys = { shiftKey: false, altKey: false, metaKey: false, channel: CHANNEL };
    expect(messages).toEqual([
      hover("https://example.org/x"),
      hover(null),
      hover("https://example.org/x"),
      link("https://example.org/x"),
      link("https://example.org/x"),
      hover(null),
      { type: "key", key: "r", code: "KeyR", ctrlKey: true, ...keys },
      { type: "key", key: "c", code: "KeyC", ctrlKey: true, ...keys },
    ]);
  });

  it("follows a link clicked with the pointer only where it led when the pointer went down", async () => {
    const { frame, doc, messages } = await load(
      '<p><a id="swap" href="https://example.org/shown">Swap</a> ' +
        '<a id="other" href="https://example.org/other">Other</a></p>',
    );
    messages.length = 0;
    const { point, press, click } = person(frame);
    const swap = doc.getElementById("swap")!;
    const other = doc.getElementById("other")!;
    // A script in the frame, had one run, changing the address as the
    // pointer goes down: the bridge has read it already, and listens first.
    swap.addEventListener("pointerdown", () => swap.setAttribute("href", "https://evil.example/"));

    point(swap);
    press(swap);
    expect(click(swap).defaultPrevented).toBe(true);
    await settle();
    // The status line showed one address and then the link's new one, but
    // the press was on the first: nothing opens.
    expect(messages).toEqual([hover("https://example.org/shown"), hover("https://evil.example/")]);

    // Pressed again, it opens where it now leads.
    press(swap);
    click(swap);
    // Pressed on one link and let go on another, nothing opens either.
    press(other);
    click(swap);
    await settle();
    expect(messages.slice(2)).toEqual([
      link("https://evil.example/"),
      hover("https://example.org/other"),
    ]);

    // Enter on a focused link, or a screen reader, goes where it leads then.
    messages.length = 0;
    click(other, { detail: 0 });
    await settle();
    expect(messages).toEqual([link("https://example.org/other")]);
  });

  it("keeps the status line on where the link under the pointer leads as that changes", async () => {
    const { frame, doc, messages } = await load(
      '<p><a id="swap" href="https://example.org/shown"><b>Swap</b></a> ' +
        '<a id="other" href="https://example.org/other">Other</a> <a id="bare"><i>Bare</i></a></p>',
    );
    messages.length = 0;
    const { point, leave } = person(frame);
    const swap = doc.getElementById("swap")!;
    const other = doc.getElementById("other")!;
    const bare = doc.getElementById("bare")!;

    // A script in the frame, had one run, changing addresses after the
    // status line showed them.
    point(swap.firstElementChild!);
    await settle();
    swap.setAttribute("href", "https://evil.example/");
    await settle();
    // Another link changing changes nothing on it.
    other.setAttribute("href", "https://evil.example/other");
    await settle();
    // No longer a link, nothing to show; a link again, where it leads.
    swap.removeAttribute("href");
    await settle();
    swap.setAttribute("href", "https://evil.example/again");
    await settle();
    // A link made under the pointer of what wasn't one.
    point(bare.firstElementChild!);
    await settle();
    bare.setAttribute("href", "https://evil.example/bare");
    await settle();
    // Out of the frame, the status line stays clear.
    leave(bare.firstElementChild!, null);
    await settle();
    bare.setAttribute("href", "https://evil.example/later");
    await settle();
    expect(messages).toEqual([
      hover("https://example.org/shown"),
      hover("https://evil.example/"),
      hover(null),
      hover("https://evil.example/again"),
      hover(null),
      hover("https://evil.example/bare"),
      hover(null),
    ]);
  });

  it("keeps the status line right as the page around the pointer is rebuilt", async () => {
    const { frame, doc, messages } = await load(
      '<p id="p"><span id="text">Plain</span> <a id="link" href="https://example.org/shown">Shown</a></p>',
    );
    messages.length = 0;
    const { point } = person(frame);
    const text = doc.getElementById("text")!;

    // A script in the frame, had one run, wrapping a link around what the
    // pointer is on: the status line shows where it now leads.
    point(text);
    await settle();
    const wrap = doc.createElement("a");
    wrap.setAttribute("href", "https://evil.example/wrapped");
    text.replaceWith(wrap);
    wrap.append(text);
    await settle();
    // The link under the pointer replaced wholesale: what the pointer was
    // on is gone, so the status line clears rather than keep its address.
    point(doc.getElementById("link")!);
    await settle();
    doc.getElementById("link")!.outerHTML = '<a href="https://evil.example/new">Shown</a>';
    await settle();
    // A new document element: changes in it still reach the status line.
    const html = doc.createElement("html");
    const body = doc.createElement("body");
    const fresh = doc.createElement("a");
    fresh.setAttribute("href", "https://example.org/fresh");
    body.append(fresh);
    html.append(body);
    doc.replaceChild(html, doc.documentElement);
    point(fresh);
    await settle();
    fresh.setAttribute("href", "https://evil.example/fresh");
    await settle();
    // (Plain text under a clear status line sends nothing.)
    expect(messages).toEqual([
      hover("https://evil.example/wrapped"),
      hover("https://example.org/shown"),
      hover(null),
      hover("https://example.org/fresh"),
      hover("https://evil.example/fresh"),
    ]);
  });

  it("knows a link for what it is, whichever window made it", async () => {
    const { frame, doc, messages } = await load('<p id="here"></p>');
    messages.length = 0;
    const { view, point, press, click } = person(frame);
    const errors: unknown[] = [];
    view.addEventListener("error", (e) => errors.push(e.error));
    // A script in the frame, had one run, putting in a link made in a frame
    // of its own: an element whose prototypes are that frame's, not this one's.
    const own = doc.createElement("iframe");
    doc.body.append(own);
    const made = own.contentDocument!;
    const odd = made.createElement("a");
    odd.setAttribute("href", "https://example.org/odd");
    const inside = made.createElement("b");
    odd.append(inside);
    doc.getElementById("here")!.append(odd);
    expect(inside instanceof view.Element).toBe(false);

    point(inside);
    press(inside);
    const followed = click(inside);
    // What isn't an element is no link (nor anything to stumble on).
    give(doc, new view.MouseEvent("click", { bubbles: true, cancelable: true, detail: 1 }));
    await settle();
    expect(followed.defaultPrevented).toBe(true);
    expect(messages).toEqual([hover("https://example.org/odd"), link("https://example.org/odd")]);
    expect(errors).toEqual([]);
  });

  it("reads events and links as they are, whatever a script in the frame redefines", async () => {
    const { frame, doc, messages } = await load(
      '<p><a id="real" href="https://example.org/real">Real</a> ' +
        '<a id="evil" href="https://evil.example/">Evil</a></p>',
    );
    messages.length = 0;
    const { view, point, leave, press, click, key } = person(frame);
    const real = doc.getElementById("real")!;
    const evil = doc.getElementById("evil")!;

    // What a script that got into the frame could change, to make the app
    // act on input no one gave: the keys pressed, what was clicked and
    // where a link leads.
    const Key = view.KeyboardEvent.prototype;
    forge(Key, "key", "Delete");
    forge(Key, "code", "Delete");
    forge(Key, "ctrlKey", false);
    forge(Key, "shiftKey", true);
    forge(Key, "altKey", true);
    forge(Key, "metaKey", true);
    forge(Key, "isComposing", true);
    forge(view.Event.prototype, "target", evil);
    forge(view.UIEvent.prototype, "detail", 0);
    forge(view.MouseEvent.prototype, "button", 1);
    forge(view.MouseEvent.prototype, "relatedTarget", null);
    Object.defineProperty(view.Event.prototype, "preventDefault", { value: () => {} });
    Object.defineProperty(view.Element.prototype, "getAttribute", {
      value: () => "https://evil.example/",
    });
    Object.defineProperty(view.Element.prototype, "closest", { value: () => evil });
    // And keep events from reaching the bridge, or act on them first.
    view.addEventListener("click", (e) => e.stopImmediatePropagation(), true);
    doc.addEventListener("keydown", (e) => e.stopPropagation(), true);

    point(real);
    // Moving on within the frame: only leaving it clears the status line.
    leave(real, doc.body);
    point(real);
    press(real);
    const followed = click(real);
    // A right click, which the forged button would make a middle one.
    press(real, 2);
    click(real, { button: 2 });
    const reply = key(doc.body, { key: "r", code: "KeyR", ctrlKey: true });
    await settle();
    expect(followed.defaultPrevented).toBe(true);
    expect(reply.defaultPrevented).toBe(true);
    expect(messages).toEqual([
      hover("https://example.org/real"),
      link("https://example.org/real"),
      {
        type: "key",
        key: "r",
        code: "KeyR",
        ctrlKey: true,
        shiftKey: false,
        altKey: false,
        metaKey: false,
        channel: CHANNEL,
      },
    ]);
  });
});
