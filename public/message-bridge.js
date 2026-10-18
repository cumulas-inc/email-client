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
// - Everything it reads events and elements with is taken now. Named
//   elements can shadow properties of document, window and forms (an <img
//   name="getElementById">), and a script could redefine a getter or a
//   method (KeyboardEvent's key, Element's getAttribute, Function's call) or
//   give an element another prototype, so events and elements are read
//   through the getters and methods as they are now, called by
//   Reflect.apply; those go by what an object is, not by its prototype.
//   (An event's isTrusted is its own, and no script can redefine it.)
//   Measurements are the exception: the document's size and scroll, and
//   where a target within the message sits, are read as they are then. A
//   script changing those could only size the frame or scroll the reader
//   wrongly.
// - Its listeners are the window's, capturing, and added before any other:
//   they hear every event first, so nothing can keep one from them or act on
//   one before they have.
// - The status line shows where the link under the pointer leads, and
//   follows along when an address changes there.
// - A link clicked with the pointer goes where it led when the pointer went
//   down on it (what the status line then shows), and only if it still
//   leads there; changed in between, it goes nowhere. One activated without
//   the pointer (Enter, or a screen reader) goes where it leads then.
// Such a script could still rewrite the mail itself, links and all, as the
// mail could have been written that way.
//
// The app tells the bridge what the search found, to mark in the message
// (`highlight`) and which match to show (`reveal`). It accepts those only
// when trusted, from the app's window, carrying the second secret its
// script was given (`data-inbound`), which the frame never sends: reading
// the frame's own messages doesn't give it away. It stops them before any
// other listener in the frame hears them (its listener is the first), and
// marks text by splitting text nodes, never by parsing markup. A script
// that learned the secret could only mark text or scroll to a mark. Text
// is matched within one text node, so a phrase split by markup isn't
// marked; text hidden only by clipping (max-height: 0 and overflow: hidden)
// is still counted.
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
  const inbound = script ? script.getAttribute("data-inbound") : null;
  if (!root || !channel) return;
  script.removeAttribute("data-channel");
  script.removeAttribute("data-inbound");

  const parentWindow = window.parent;
  const send = parentWindow.postMessage.bind(parentWindow);
  const listen = method(EventTarget.prototype.addEventListener);
  const ElementProto = Element.prototype;
  const closest = method(ElementProto.closest);
  const attribute = method(ElementProto.getAttribute);
  const scrollIntoView = method(ElementProto.scrollIntoView);
  const rectOf = method(ElementProto.getBoundingClientRect);
  const contains = method(Node.prototype.contains);
  const byId = Document.prototype.getElementById.bind(doc);
  const byName = Document.prototype.getElementsByName.bind(doc);
  const style = window.getComputedStyle.bind(window);
  const Observer = window.ResizeObserver;
  const Mutations = window.MutationObserver;
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
  const sourceOf = getter(MessageEvent.prototype, "source");
  const messageOf = getter(MessageEvent.prototype, "data");
  const stopImmediatePropagation = method(Event.prototype.stopImmediatePropagation);
  /** Listens before anything else in the frame can. */
  const first = (type, fn) => listen(window, type, fn, true);

  // Measured with this in place, the frame is as tall as its content; if
  // this never ran, the app couldn't size the frame and it scrolls instead.
  root.setAttribute("data-bridge", "");

  /** The link at or around `node`, as the reader follows them. */
  const linkAt = (node) => {
    // closest checks that it's called on an element, and throws if not.
    try {
      return closest(node, "a[href], area[href]");
    } catch {
      return null;
    }
  };
  const linkOf = (e) => linkAt(targetOf(e));
  const hrefOf = (link) => attribute(link, "href");

  // Where the link under the pointer really goes, for the reader's status line.
  let hovered = null;
  // What the pointer is over, while it's in the frame.
  let under = null;
  const hover = (href) => {
    if (href === hovered) return;
    hovered = href;
    post({ type: "hover", href });
  };
  const pointAt = (node) => {
    under = node;
    const link = linkAt(node);
    hover(link ? hrefOf(link) : null);
  };
  first("mouseover", (e) => {
    if (e.isTrusted) pointAt(targetOf(e));
  });
  first("mouseout", (e) => {
    if (!e.isTrusted || relatedTargetOf(e)) return;
    under = null;
    hover(null);
  });
  // An address changed under the pointer (or one made there, or the page
  // rebuilt around it, its root included) shows at once, not only once the
  // pointer goes down.
  if (typeof Mutations === "function") {
    new Mutations(() => {
      if (under) pointAt(contains(doc, under) ? under : null);
    }).observe(doc, {
      subtree: true,
      childList: true,
      attributes: true,
      attributeFilter: ["href"],
    });
  }

  // The link the pointer last went down on, and where it led then. Pointer
  // events come before the mouse's (pointerdown before mousedown).
  let pressed = null;
  const PRESS = typeof window.PointerEvent === "function" ? "pointerdown" : "mousedown";
  first(PRESS, (e) => {
    if (!e.isTrusted) return;
    under = targetOf(e);
    const link = linkAt(under);
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

  // --- What the search found ----------------------------------------------------

  // What marking reads and changes the document with, as it is now.
  const NodeProto = Node.prototype;
  const createElement = method(Document.prototype.createElement);
  const createTextNode = method(Document.prototype.createTextNode);
  const createTreeWalker = method(Document.prototype.createTreeWalker);
  const nextNode = method(TreeWalker.prototype.nextNode);
  const splitText = method(Text.prototype.splitText);
  const insertBefore = method(NodeProto.insertBefore);
  const replaceChild = method(NodeProto.replaceChild);
  const normalizeNode = method(NodeProto.normalize);
  const parentOf = getter(NodeProto, "parentNode");
  const parentElementOf = getter(NodeProto, "parentElement");
  const nodeTypeOf = getter(NodeProto, "nodeType");
  const textContentOf = getter(NodeProto, "textContent");
  const dataOf = getter(CharacterData.prototype, "data");
  const localNameOf = getter(ElementProto, "localName");
  const bodyOf = getter(Document.prototype, "body");
  const setAttribute = method(ElementProto.setAttribute);
  const removeAttribute = method(ElementProto.removeAttribute);
  // Missing in some engines: then all text counts as shown.
  const checkVisibility =
    typeof ElementProto.checkVisibility === "function"
      ? method(ElementProto.checkVisibility)
      : null;
  const normalizeText = method(String.prototype.normalize);
  const charCodeAt = method(String.prototype.charCodeAt);
  const exec = method(RegExp.prototype.exec);
  const mapGet = method(Map.prototype.get);
  const mapSet = method(Map.prototype.set);
  const mapHas = method(Map.prototype.has);
  const sort = method(Array.prototype.sort);
  const isArray = Array.isArray;
  const isInteger = Number.isInteger;

  // MAX_WORDS and MAX_WORD_CHARS are highlight.ts's MAX_PATTERN_WORDS and
  // MAX_WORD_UNITS: the app sends no more.
  const MAX_PATTERNS = 32;
  const MAX_WORDS = 16;
  const MAX_WORD_CHARS = 128;
  const MAX_HITS = 1000;
  const MAX_SCAN_CHARS = 250000;
  const ELEMENT = 1;
  const SHOW_ELEMENT_AND_TEXT = 0x1 | 0x4;
  const ACCEPT = 1;
  const REJECT = 2;
  /** Elements whose text isn't the message's to read. */
  const SKIPPED = Object.create(null);
  for (const name of [
    "script",
    "style",
    "noscript",
    "template",
    "textarea",
    "select",
    "option",
    "title",
  ]) {
    SKIPPED[name] = true;
  }

  /** The search's patterns, if `value` is a list of them; else null. */
  const readPatterns = (value) => {
    if (!isArray(value) || value.length > MAX_PATTERNS) return null;
    const patterns = [];
    for (let i = 0; i < value.length; i++) {
      const p = value[i];
      if (typeof p !== "object" || p === null || typeof p.prefix !== "boolean") return null;
      const words = p.words;
      if (!isArray(words) || words.length < 1 || words.length > MAX_WORDS) return null;
      const copy = [];
      for (let k = 0; k < words.length; k++) {
        const w = words[k];
        if (typeof w !== "string" || w.length < 1 || w.length > MAX_WORD_CHARS) return null;
        copy[k] = w;
      }
      // Words only: what's inside words is for subjects and senders.
      patterns[i] = { words: copy, prefix: p.prefix };
    }
    return patterns;
  };

  // Each code point's fold (accents off, lowercased) doesn't depend on its
  // neighbors: worked out once each.
  const folds = new Map();
  const isMark = (c) => exec(/^\p{M}$/u, c) !== null;
  /** How many code units the code point at `i` takes: 2 for a surrogate pair. */
  const unitsAt = (text, i) => {
    const code = charCodeAt(text, i);
    if (code < 0xd800 || code >= 0xdc00 || i + 1 >= text.length) return 1;
    const next = charCodeAt(text, i + 1);
    return next >= 0xdc00 && next < 0xe000 ? 2 : 1;
  };
  const foldChar = (c, key) => {
    if (mapHas(folds, key)) return mapGet(folds, key);
    const decomposed = normalizeText(c, "NFD");
    let f = "";
    for (let i = 0; i < decomposed.length;) {
      const point = slice(decomposed, i, i + unitsAt(decomposed, i));
      if (!isMark(point)) f += point;
      i += point.length;
    }
    f = lower(f);
    mapSet(folds, key, f);
    return f;
  };

  /**
   * `text` folded as the app folds it (src/lib/search/highlight.ts), and
   * for each of its units where it came from; null starts for ASCII, whose
   * units are where they were.
   */
  const fold = (text) => {
    let ascii = true;
    for (let i = 0; i < text.length && ascii; i++) ascii = charCodeAt(text, i) < 0x80;
    if (ascii) return { folded: lower(text), starts: null, ends: null };
    let folded = "";
    const starts = [];
    const ends = [];
    for (let i = 0; i < text.length;) {
      const size = unitsAt(text, i);
      const end = i + size;
      const key = charCodeAt(text, i) * 0x10000 + (size === 2 ? charCodeAt(text, i + 1) : 0);
      const f = foldChar(slice(text, i, end), key);
      for (let k = 0; k < f.length; k++) {
        starts[starts.length] = i;
        ends[ends.length] = end;
      }
      // A mark stays with the letter before it.
      if (f.length === 0 && ends.length > 0) ends[ends.length - 1] = end;
      folded += f;
      i = end;
    }
    return { folded, starts, ends };
  };

  /** Where the patterns are in `text`: UTF-16 ranges, in order, merged. */
  const findHits = (text, patterns) => {
    const { folded, starts, ends } = fold(text);
    const startOf = (k) => (starts ? starts[k] : k);
    const endOf = (k) => (ends ? ends[k] : k + 1);
    const tokens = [];
    const pattern = /[\p{L}\p{N}]+/gu;
    for (let m = exec(pattern, folded); m !== null; m = exec(pattern, folded)) {
      tokens[tokens.length] = { text: m[0], start: m.index, end: m.index + m[0].length };
    }
    const ranges = [];
    for (let p = 0; p < patterns.length; p++) {
      const { words, prefix } = patterns[p];
      const last = words.length - 1;
      // A pattern's hits come in order: past its 1,000th, none is among the first 1,000.
      let found = 0;
      for (let t = 0; t + last < tokens.length && found < MAX_HITS; t++) {
        let all = true;
        for (let k = 0; k <= last && all; k++) {
          const token = tokens[t + k].text;
          const w = words[k];
          all = k === last && prefix ? slice(token, 0, w.length) === w : token === w;
        }
        if (!all) continue;
        const end = prefix ? tokens[t + last].start + words[last].length : tokens[t + last].end;
        ranges[ranges.length] = [startOf(tokens[t].start), endOf(end - 1)];
        found++;
      }
    }
    // In order, those that overlap or touch made one.
    sort(ranges, (a, b) => a[0] - b[0] || a[1] - b[1]);
    const merged = [];
    for (let i = 0; i < ranges.length; i++) {
      const top = merged[merged.length - 1];
      if (top && ranges[i][0] <= top[1]) {
        if (ranges[i][1] > top[1]) top[1] = ranges[i][1];
      } else merged[merged.length] = [ranges[i][0], ranges[i][1]];
    }
    return merged;
  };

  // The marks made, in the document's order: put back from here, never
  // found again in the document (the mail may have marks of its own).
  let marks = [];
  let shown = null;
  let applied = null;

  const unmark = () => {
    const parents = [];
    for (let i = 0; i < marks.length; i++) {
      const mark = marks[i];
      const parent = parentOf(mark);
      if (!parent) continue;
      replaceChild(parent, createTextNode(doc, textContentOf(mark)), mark);
      parents[parents.length] = parent;
    }
    for (let i = 0; i < parents.length; i++) normalizeNode(parents[i]);
    marks = [];
    shown = null;
  };

  /** Marks what `patterns` find in the message's text that shows. */
  const mark = (patterns) => {
    unmark();
    const body = bodyOf(doc);
    if (!body || patterns.length === 0) return;
    // First everything is found, reading layout before anything changes...
    const walker = createTreeWalker(doc, body, SHOW_ELEMENT_AND_TEXT, (node) =>
      nodeTypeOf(node) === ELEMENT && SKIPPED[localNameOf(node)] === true ? REJECT : ACCEPT,
    );
    const visible = new Map();
    const shows = (element) => {
      if (!checkVisibility || !element) return true;
      if (!mapHas(visible, element)) {
        mapSet(
          visible,
          element,
          checkVisibility(element, { visibilityProperty: true, opacityProperty: true }),
        );
      }
      return mapGet(visible, element);
    };
    const found = [];
    let hits = 0;
    let scanned = 0;
    for (let node = nextNode(walker); node !== null; node = nextNode(walker)) {
      if (nodeTypeOf(node) === ELEMENT || !shows(parentElementOf(node))) continue;
      let text = dataOf(node);
      if (scanned + text.length > MAX_SCAN_CHARS) text = slice(text, 0, MAX_SCAN_CHARS - scanned);
      scanned += text.length;
      const ranges = findHits(text, patterns);
      if (hits + ranges.length > MAX_HITS) ranges.length = MAX_HITS - hits;
      if (ranges.length > 0) found[found.length] = { node, ranges };
      hits += ranges.length;
      if (hits >= MAX_HITS || scanned >= MAX_SCAN_CHARS) break;
    }
    // ...then each text node is split around its hits, the last first, so
    // the offsets of those before stay as they were.
    for (let i = 0; i < found.length; i++) {
      const { node, ranges } = found[i];
      const made = [];
      for (let r = ranges.length - 1; r >= 0; r--) {
        const [start, end] = ranges[r];
        if (end < dataOf(node).length) splitText(node, end);
        const hit = start > 0 ? splitText(node, start) : node;
        const element = createElement(doc, "mark");
        setAttribute(element, "data-cumulas-hit", "");
        replaceChild(parentOf(hit), element, hit);
        insertBefore(element, hit, null);
        made[made.length] = element;
      }
      for (let k = made.length - 1; k >= 0; k--) marks[marks.length] = made[k];
    }
  };

  /** Makes match `index` the one shown, scrolls it into view, and says where it is. */
  const reveal = (index) => {
    if (marks.length === 0) return;
    const i = index < 0 ? 0 : index >= marks.length ? marks.length - 1 : index;
    if (shown) removeAttribute(shown, "data-cumulas-current");
    shown = marks[i];
    setAttribute(shown, "data-cumulas-current", "");
    // Sideways, mail wider than the pane scrolls in the frame itself.
    scrollIntoView(shown, { block: "nearest", inline: "nearest" });
    const at = rectOf(shown);
    const top = rectOf(root).top;
    post({ type: "reveal", top: at.top - top, bottom: at.bottom - top });
  };

  if (inbound) {
    first("message", (e) => {
      const data = messageOf(e);
      if (!e.isTrusted || sourceOf(e) !== parentWindow) return;
      if (typeof data !== "object" || data === null || data.inbound !== inbound) return;
      // No other listener in the frame hears the app's words, or its secret.
      stopImmediatePropagation(e);
      if (data.type === "highlight") {
        // What isn't a list of patterns marks nothing: the last marks and
        // count never outlive the app's words.
        const read = readPatterns(data.patterns);
        const patterns = read === null ? [] : read;
        const key = stringify(patterns);
        if (key !== applied) {
          applied = key;
          mark(patterns);
        }
        post({ type: "hits", count: marks.length });
      } else if (data.type === "reveal") {
        const index = data.index;
        if (isInteger(index)) reveal(index);
      }
    });
  }

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
    // The app may send what to mark from now on.
    post({ type: "ready" });
    measure();
  });
  listen(window, "load", measure);
  listen(doc, "scroll", measure, { passive: true });
  // Pictures and fonts change the height once they've loaded.
  listen(doc, "load", measure, true);
  listen(doc, "error", measure, true);
  if (typeof Observer === "function") new Observer(measure).observe(root);
})();
