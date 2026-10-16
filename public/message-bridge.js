// The message reader's bridge, the one script a message's frame runs.
//
// A message is shown in a frame sandboxed with allow-scripts and without
// allow-same-origin: its origin is opaque, so nothing in it can reach the
// app (or the app's IPC), and the app can't reach into it either. WebKitGTK
// calls no event listener in a frame that may not run scripts, so the frame
// runs this, the only script its policy allows, and it tells the app what
// happens there by postMessage: a link clicked or pointed at, a key pressed,
// the document's size. The app checks that each message comes from its
// frame and carries the channel the frame was given.
//
// The frame's own markup is untrusted mail, parsed after this runs. Its
// scripts don't run, but if one somehow did, it still couldn't reach the
// app, read the channel or forge input: the bridge reports only trusted
// events, which only a person's input makes. It could change what the
// bridge reads, though, so the bridge limits what such a script can change:
// - Everything used later is taken now. Named elements can shadow
//   properties of document, window and forms (an <img name="getElementById">),
//   and a script could redefine a getter or a method (KeyboardEvent's key,
//   Element's getAttribute, Function's call), so events and elements are read
//   through the getters and methods as they are now, called by Reflect.apply.
//   (An event's isTrusted is its own, and no script can redefine it.)
// - Its listeners are the window's, capturing, and added before any other:
//   they hear every event first, so nothing can keep one from them or act on
//   one before they have.
// - A link clicked with the pointer goes where it led when the pointer went
//   down on it (what the status line then shows), and only if it still
//   leads there; changed in between, it goes nowhere. One activated without
//   the pointer (Enter, or a screen reader) goes where it leads then.
// Such a script could still rewrite the mail itself, links and all, as the
// mail could have been written that way.
(() => {
  "use strict";
  const apply = Reflect.apply;
  const descriptor = Object.getOwnPropertyDescriptor;
  /** `proto`'s getter for `name`, as it is now. */
  const getter = (proto, name) => {
    const get = descriptor(proto, name).get;
    return (target) => apply(get, target, []);
  };
  /** `fn`, called on a target as it is now. */
  const method =
    (fn) =>
    (target, ...args) =>
      apply(fn, target, args);

  const doc = document;
  const root = doc.documentElement;
  const script = doc.currentScript;
  const channel = script ? script.getAttribute("data-channel") : null;
  if (!root || !channel) return;
  script.removeAttribute("data-channel");

  const send = window.parent.postMessage.bind(window.parent);
  const listen = method(EventTarget.prototype.addEventListener);
  const ElementProto = Element.prototype;
  const isPrototypeOf = method(Object.prototype.isPrototypeOf);
  const closest = method(ElementProto.closest);
  const attribute = method(ElementProto.getAttribute);
  const scrollIntoView = method(ElementProto.scrollIntoView);
  const rectOf = method(ElementProto.getBoundingClientRect);
  const byId = Document.prototype.getElementById.bind(doc);
  const byName = Document.prototype.getElementsByName.bind(doc);
  const style = window.getComputedStyle.bind(window);
  const Observer = window.ResizeObserver;
  const decode = decodeURIComponent;
  const stringify = JSON.stringify;
  const slice = method(String.prototype.slice);
  const lower = method(String.prototype.toLowerCase);
  const post = (message) => send({ ...message, channel }, "*");

  // What the bridge reads from events.
  const preventDefault = method(Event.prototype.preventDefault);
  const targetOf = getter(Event.prototype, "target");
  const detailOf = getter(UIEvent.prototype, "detail");
  const buttonOf = getter(MouseEvent.prototype, "button");
  const relatedTargetOf = getter(MouseEvent.prototype, "relatedTarget");
  const Key = KeyboardEvent.prototype;
  const keyOf = getter(Key, "key");
  const codeOf = getter(Key, "code");
  const ctrlOf = getter(Key, "ctrlKey");
  const shiftOf = getter(Key, "shiftKey");
  const altOf = getter(Key, "altKey");
  const metaOf = getter(Key, "metaKey");
  const composingOf = getter(Key, "isComposing");
  /** Listens before anything else in the frame can. */
  const first = (type, fn) => listen(window, type, fn, true);

  // Measured with this in place, the frame is as tall as its content; if
  // this never ran, the app couldn't size the frame and it scrolls instead.
  root.setAttribute("data-bridge", "");

  /** The link at or around an event's target, as the reader follows them. */
  const linkOf = (e) => {
    const target = targetOf(e);
    return isPrototypeOf(ElementProto, target) ? closest(target, "a[href], area[href]") : null;
  };
  const hrefOf = (link) => attribute(link, "href");

  // Where the link under the pointer really goes, for the reader's status line.
  let hovered = null;
  const hover = (href) => {
    if (href === hovered) return;
    hovered = href;
    post({ type: "hover", href });
  };
  first("mouseover", (e) => {
    if (!e.isTrusted) return;
    const link = linkOf(e);
    hover(link ? hrefOf(link) : null);
  });
  first("mouseout", (e) => {
    if (e.isTrusted && !relatedTargetOf(e)) hover(null);
  });

  // The link the pointer last went down on, and where it led then. Pointer
  // events come before the mouse's (pointerdown before mousedown).
  let pressed = null;
  const PRESS = typeof window.PointerEvent === "function" ? "pointerdown" : "mousedown";
  first(PRESS, (e) => {
    if (!e.isTrusted) return;
    const link = linkOf(e);
    const href = link ? hrefOf(link) : null;
    pressed = { link, href };
    // The status line shows where a click now goes.
    hover(href);
  });

  // Links open through the app (in the browser, or a new message for
  // mailto:), never in the frame. A link within the message shows its
  // target: the frame is as tall as the message, so the app scrolls the
  // reader (WebKit doesn't let a cross-origin frame scroll its parent).
  const follow = (e, aux) => {
    const link = linkOf(e);
    if (!link) return;
    preventDefault(e);
    if (!e.isTrusted || (aux && buttonOf(e) !== 1)) return;
    let href = hrefOf(link) || "";
    // A click by the pointer counts its clicks; Enter or a screen reader's
    // counts none.
    if (detailOf(e) > 0) {
      if (!pressed || pressed.link !== link || pressed.href !== hrefOf(link)) return;
      href = pressed.href || "";
    }
    if (href[0] === "#") {
      let name = slice(href, 1);
      try {
        name = decode(name);
      } catch {
        // Not percent-encoded after all: the name as written.
      }
      const target = byId(name) || byName(name)[0];
      if (!target) return;
      // Sideways, mail wider than the pane scrolls in the frame itself.
      scrollIntoView(target, { block: "nearest", inline: "nearest" });
      post({ type: "scroll", top: rectOf(target).top - rectOf(root).top });
      return;
    }
    post({ type: "link", href });
  };
  first("click", (e) => follow(e, false));
  first("auxclick", (e) => follow(e, true));
  // The sandbox already refuses forms; this keeps one from trying.
  first("submit", (e) => preventDefault(e));

  // Keys pressed in the message go to the app, whose shortcuts (Ctrl+R,
  // Delete, J and K...) work here as anywhere in the reader. Ctrl with a
  // character or Enter is the app's, so the frame doesn't act on it too;
  // copy and select all stay the frame's own.
  first("keydown", (e) => {
    if (!e.isTrusted || composingOf(e)) return;
    const key = keyOf(e);
    const ctrlKey = ctrlOf(e);
    const altKey = altOf(e);
    const metaKey = metaOf(e);
    if ((ctrlKey || metaKey) && !altKey && (key.length === 1 || key === "Enter")) {
      const name = lower(key);
      if (name !== "a" && name !== "c") preventDefault(e);
    }
    post({ type: "key", key, code: codeOf(e), ctrlKey, shiftKey: shiftOf(e), altKey, metaKey });
  });

  // The app sizes the frame to the document. Mail wider than the pane
  // scrolls sideways; the app fades an edge with more beyond it.
  let parsed = false;
  let reported = "";
  const measure = () => {
    // Not before the mail is all there: an empty page is no height to show.
    if (!parsed) return;
    const width = root.scrollWidth;
    const visible = root.clientWidth;
    const max = width - visible;
    // Right-to-left pages scroll from 0 down to -max.
    const min = style(root).direction === "rtl" ? -max : 0;
    const size = {
      type: "size",
      height: rectOf(root).height,
      wide: width > visible + 1,
      left: max > 1 && root.scrollLeft - min > 1,
      right: max > 1 && min + max - root.scrollLeft > 1,
    };
    const key = stringify(size);
    if (key === reported) return;
    reported = key;
    post(size);
  };
  listen(doc, "DOMContentLoaded", () => {
    parsed = true;
    measure();
  });
  listen(window, "load", measure);
  listen(doc, "scroll", measure, { passive: true });
  // Pictures and fonts change the height once they've loaded.
  listen(doc, "load", measure, true);
  listen(doc, "error", measure, true);
  if (typeof Observer === "function") new Observer(measure).observe(root);
})();
