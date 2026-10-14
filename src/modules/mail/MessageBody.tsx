// Renders untrusted email HTML. The HTML was sanitized by the backend; as
// defense in depth it is shown in an <iframe sandbox="allow-same-origin">
// (no scripts, forms, popups or top navigation) with a Content Security
// Policy that only allows inline styles and data:/cid: images, unless the
// user allowed remote content. allow-same-origin lets this component reach
// into the frame to size it and to route link clicks through the app.

import { type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useBodyFont } from "./bodyFont";
import { anchorOf, type BodyAppearance, type BodyTone, buildSrcDoc, linkify } from "./bodyHtml";
import { plainTextBlocks } from "./text";

interface MessageBodyProps {
  html: string;
  allowRemote: boolean;
  appearance: BodyAppearance;
  /** "quiet": secondary text, for the original under a reply or forward. */
  tone?: BodyTone;
  title: string;
  onLink: (href: string) => void;
  /** The URL under the pointer, to show where a link really goes. */
  onHoverLink?: (href: string | null) => void;
}

export function MessageBody({
  html,
  allowRemote,
  appearance,
  tone = "normal",
  title,
  onLink,
  onHoverLink,
}: MessageBodyProps) {
  const frameRef = useRef<HTMLIFrameElement>(null);
  const [height, setHeight] = useState(80);
  // Mail wider than the pane scrolls sideways; faded edges say there's more.
  const [hidden, setHidden] = useState({ left: false, right: false });
  const handlers = useRef({ onLink, onHoverLink });
  useEffect(() => {
    handlers.current = { onLink, onHoverLink };
  });

  // The reading font is only the default: mail that sets its own fonts keeps them.
  const font = useBodyFont();
  const srcDoc = useMemo(
    () => (font ? buildSrcDoc(html, { allowRemote, appearance, tone, font }) : null),
    [html, allowRemote, appearance, tone, font],
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

    const edges = () => {
      const scroller = doc.scrollingElement ?? doc.documentElement;
      if (!scroller) return;
      const max = scroller.scrollWidth - scroller.clientWidth;
      // Right-to-left pages scroll from 0 down to -max.
      const rtl = doc.defaultView?.getComputedStyle(scroller).direction === "rtl";
      const min = rtl ? -max : 0;
      const left = max > 1 && scroller.scrollLeft - min > 1;
      const right = max > 1 && min + max - scroller.scrollLeft > 1;
      setHidden((h) => (h.left === left && h.right === right ? h : { left, right }));
    };
    const measure = () => {
      const root = doc.documentElement;
      if (!root) return;
      let next = Math.ceil(root.getBoundingClientRect().height);
      // Room for a horizontal scrollbar on very wide newsletters.
      if (root.scrollWidth > root.clientWidth + 1) next += 16;
      setHeight(Math.max(next, 24));
      edges();
    };
    measure();
    doc.addEventListener("scroll", edges, { passive: true });

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
    <div
      className="mail-body-wide"
      data-more-left={hidden.left || undefined}
      data-more-right={hidden.right || undefined}
    >
      {srcDoc === null ? (
        // The reading font's faces are still loading (only ever the first time).
        <div className="mail-body-frame" style={{ height }} />
      ) : (
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
      )}
    </div>
  );
}

function Linked({ text, onLink }: { text: string; onLink: (href: string) => void }) {
  return linkify(text).map((part, i) =>
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
  );
}

/**
 * Plain text, set like the letter: paragraphs in the serif, each in its own
 * direction; quoted lines quieter; tables and aligned columns in monospace,
 * unwrapped, scrolling sideways when they're wider than the pane.
 */
export function PlainTextBody({
  text,
  onLink,
}: {
  text: string;
  onLink: (href: string) => void;
}): ReactNode {
  const blocks = useMemo(() => plainTextBlocks(text), [text]);
  return (
    <div className="mail-plaintext selectable">
      {blocks.map((block, i) => {
        if (block.kind === "pre") {
          return (
            <pre key={i} className="mail-plaintext__pre" tabIndex={0}>
              <Linked text={block.text} onLink={onLink} />
            </pre>
          );
        }
        if (block.kind === "signature") {
          return (
            <p key={i} className="mail-plaintext__p mail-plaintext__signature">
              <Linked text={block.text} onLink={onLink} />
            </p>
          );
        }
        return (
          <p key={i} className="mail-plaintext__p">
            {block.lines.map((line, j) => (
              <span key={j} className={line.quoted ? "mail-plaintext__quote" : undefined}>
                <Linked text={line.text} onLink={onLink} />
                {j < block.lines.length - 1 ? "\n" : ""}
              </span>
            ))}
          </p>
        );
      })}
    </div>
  );
}
