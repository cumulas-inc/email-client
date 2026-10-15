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
// The frame's own markup is untrusted mail, parsed after this runs. Named
// elements can shadow properties of document, window and forms (an
// <img name="getElementById">), so everything used later is taken now, and
// only real (trusted) input is reported, so markup that somehow ran a script
// couldn't make the app act.
(() => {
  "use strict";
  const doc = document;
  const root = doc.documentElement;
  const script = doc.currentScript;
  const channel = script ? script.getAttribute("data-channel") : null;
  if (!root || !channel) return;
  script.removeAttribute("data-channel");

  const send = window.parent.postMessage.bind(window.parent);
  const listen = (target, type, fn, options) =>
    EventTarget.prototype.addEventListener.call(target, type, fn, options);
  const ElementType = Element;
  const closest = Element.prototype.closest;
  const attribute = Element.prototype.getAttribute;
  const scrollIntoView = Element.prototype.scrollIntoView;
  const rectOf = Element.prototype.getBoundingClientRect;
  const byId = Document.prototype.getElementById.bind(doc);
  const byName = Document.prototype.getElementsByName.bind(doc);
  const style = window.getComputedStyle.bind(window);
  const Observer = window.ResizeObserver;
  const decode = decodeURIComponent;
  const stringify = JSON.stringify;
  const post = (message) => send({ ...message, channel }, "*");

  // Measured with this in place, the frame is as tall as its content; if
  // this never ran, the app couldn't size the frame and it scrolls instead.
  root.setAttribute("data-bridge", "");

  /** The link at or around an event's target, as the reader follows them. */
  const linkOf = (target) =>
    target instanceof ElementType ? closest.call(target, "a[href], area[href]") : null;

  // Links open through the app (in the browser, or a new message for
  // mailto:), never in the frame. A link within the message shows its
  // target: the frame is as tall as the message, so the app scrolls the
  // reader (WebKit doesn't let a cross-origin frame scroll its parent).
  const follow = (e) => {
    const link = linkOf(e.target);
    if (!link) return;
    e.preventDefault();
    if (!e.isTrusted || (e.type === "auxclick" && e.button !== 1)) return;
    const href = attribute.call(link, "href") || "";
    if (href.startsWith("#")) {
      let name = href.slice(1);
      try {
        name = decode(name);
      } catch {
        // Not percent-encoded after all: the name as written.
      }
      const target = byId(name) || byName(name)[0];
      if (!target) return;
      // Sideways, mail wider than the pane scrolls in the frame itself.
      scrollIntoView.call(target, { block: "nearest", inline: "nearest" });
      post({ type: "scroll", top: rectOf.call(target).top - rectOf.call(root).top });
      return;
    }
    post({ type: "link", href });
  };
  listen(doc, "click", follow);
  listen(doc, "auxclick", follow);
  // The sandbox already refuses forms; this keeps one from trying.
  listen(doc, "submit", (e) => e.preventDefault(), true);

  // Where the link under the pointer really goes, for the reader's status line.
  let hovered = null;
  const hover = (href) => {
    if (href === hovered) return;
    hovered = href;
    post({ type: "hover", href });
  };
  listen(doc, "mouseover", (e) => {
    if (!e.isTrusted) return;
    const link = linkOf(e.target);
    hover(link ? attribute.call(link, "href") : null);
  });
  listen(doc, "mouseout", (e) => {
    if (!e.relatedTarget) hover(null);
  });

  // Keys pressed in the message go to the app, whose shortcuts (Ctrl+R,
  // Delete, J and K...) work here as anywhere in the reader. Ctrl with a
  // character or Enter is the app's, so the frame doesn't act on it too;
  // copy and select all stay the frame's own.
  const NATIVE = new Set(["a", "c", "insert"]);
  listen(doc, "keydown", (e) => {
    if (!e.isTrusted || e.isComposing) return;
    const key = e.key;
    const command = (e.ctrlKey || e.metaKey) && !e.altKey;
    if (command && (key.length === 1 || key === "Enter") && !NATIVE.has(key.toLowerCase())) {
      e.preventDefault();
    }
    post({
      type: "key",
      key,
      code: e.code,
      ctrlKey: e.ctrlKey,
      shiftKey: e.shiftKey,
      altKey: e.altKey,
      metaKey: e.metaKey,
    });
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
      height: rectOf.call(root).height,
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
