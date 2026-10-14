// Renders untrusted email HTML. The HTML was sanitized by the backend; as
// defense in depth it is shown in an <iframe sandbox="allow-same-origin">
// (no scripts, forms, popups or top navigation) with a Content Security
// Policy that only allows inline styles and data:/cid: images, unless the
// user allowed remote content. allow-same-origin lets this component reach
// into the frame to size it and to route link clicks through the app.

import { type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from "react";

import { anchorOf, type BodyAppearance, buildSrcDoc, DEFAULT_FONT, linkify } from "./bodyHtml";

interface MessageBodyProps {
  html: string;
  allowRemote: boolean;
  appearance: BodyAppearance;
  title: string;
  onLink: (href: string) => void;
  /** The URL under the pointer, to show where a link really goes. */
  onHoverLink?: (href: string | null) => void;
}

export function MessageBody({
  html,
  allowRemote,
  appearance,
  title,
  onLink,
  onHoverLink,
}: MessageBodyProps) {
  const frameRef = useRef<HTMLIFrameElement>(null);
  const [height, setHeight] = useState(80);
  const handlers = useRef({ onLink, onHoverLink });
  useEffect(() => {
    handlers.current = { onLink, onHoverLink };
  });

  const fontFamily = useMemo(() => {
    const family = typeof window !== "undefined" ? getComputedStyle(document.body).fontFamily : "";
    return family || DEFAULT_FONT;
  }, []);

  const srcDoc = useMemo(
    () => buildSrcDoc(html, { allowRemote, appearance, fontFamily }),
    [html, allowRemote, appearance, fontFamily],
  );

  const observers = useRef<ResizeObserver[]>([]);
  useEffect(() => {
    const active = observers;
    return () => active.current.forEach((o) => o.disconnect());
  }, []);

  const onLoad = useCallback(() => {
    const frame = frameRef.current;
    const doc = frame?.contentDocument;
    if (!frame || !doc?.documentElement) return;
    observers.current.forEach((o) => o.disconnect());
    observers.current = [];

    const measure = () => {
      const root = doc.documentElement;
      if (!root) return;
      let next = Math.ceil(root.getBoundingClientRect().height);
      // Room for a horizontal scrollbar on very wide newsletters.
      if (root.scrollWidth > root.clientWidth + 1) next += 16;
      setHeight(Math.max(next, 24));
    };
    measure();

    // Images and fonts change the height after load.
    doc.addEventListener("load", measure, true);
    doc.addEventListener("error", measure, true);
    if (typeof ResizeObserver === "function") {
      try {
        const contentObserver = new ResizeObserver(measure);
        contentObserver.observe(doc.documentElement);
        const frameObserver = new ResizeObserver(measure);
        frameObserver.observe(frame);
        observers.current = [contentObserver, frameObserver];
      } catch {
        // Older engines can't observe another document; load events still apply.
      }
    }

    const follow = (e: MouseEvent) => {
      const anchor = anchorOf(e.target);
      if (!anchor) return;
      e.preventDefault();
      if (e.type === "auxclick" && e.button !== 1) return;
      const href = anchor.getAttribute?.("href") ?? "";
      if (href.startsWith("#")) {
        doc.getElementById(decodeURIComponent(href.slice(1)))?.scrollIntoView();
        return;
      }
      handlers.current.onLink(href);
    };
    doc.addEventListener("click", follow);
    doc.addEventListener("auxclick", follow);
    doc.addEventListener("submit", (e) => e.preventDefault(), true);
    doc.addEventListener("mouseover", (e) => {
      const anchor = anchorOf(e.target);
      handlers.current.onHoverLink?.(anchor?.getAttribute?.("href") ?? null);
    });
    doc.addEventListener("mouseleave", () => handlers.current.onHoverLink?.(null));

    // Keyboard events don't leave the frame: replay them on the frame element
    // so the mail shortcuts (Ctrl+R, Delete...) still work after clicking text.
    doc.addEventListener("keydown", (e) => {
      const replay = new KeyboardEvent("keydown", {
        key: e.key,
        code: e.code,
        ctrlKey: e.ctrlKey,
        shiftKey: e.shiftKey,
        altKey: e.altKey,
        metaKey: e.metaKey,
        bubbles: true,
        cancelable: true,
      });
      frame.dispatchEvent(replay);
      if (replay.defaultPrevented) e.preventDefault();
    });
  }, []);

  return (
    <iframe
      ref={frameRef}
      className={`mail-body-frame mail-body-frame--${appearance}`}
      title={title}
      sandbox="allow-same-origin"
      referrerPolicy="no-referrer"
      srcDoc={srcDoc}
      onLoad={onLoad}
      style={{ height }}
    />
  );
}

export function PlainTextBody({
  text,
  onLink,
}: {
  text: string;
  onLink: (href: string) => void;
}): ReactNode {
  const parts = useMemo(() => linkify(text), [text]);
  return (
    <div className="mail-plaintext selectable">
      {parts.map((part, i) =>
        "href" in part ? (
          <a
            key={i}
            href={part.href}
            title={part.href}
            onClick={(e) => {
              e.preventDefault();
              onLink(part.href);
            }}
            onAuxClick={(e) => e.preventDefault()}
          >
            {part.text}
          </a>
        ) : (
          <span key={i}>{part.text}</span>
        ),
      )}
    </div>
  );
}
