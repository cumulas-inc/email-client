// Renders untrusted email HTML. The HTML was sanitized by the backend; as
// defense in depth it is shown in an <iframe sandbox="allow-scripts">
// without allow-same-origin: an opaque origin, which can't reach the app or
// its IPC (no forms, popups or top navigation either), with a Content
// Security Policy that only allows inline styles, data:/cid: images (unless
// the user allowed remote content) and one script, the bridge.
//
// WebKitGTK calls no event listener in a frame that may not run scripts,
// this component's included, and an opaque frame can't be reached into, so
// the bridge (public/message-bridge.js) runs inside and reports by
// postMessage: links clicked and pointed at, keys pressed, the document's
// size. Messages count only from this frame, on the channel its document
// was given.

import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";

import { useBodyFont } from "./bodyFont";
import {
  type BodyAppearance,
  type BodyTone,
  buildSrcDoc,
  linkify,
  messageBridgeUrl,
  newChannel,
  readBridgeMessage,
} from "./bodyHtml";
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
  // Until the bridge reports. The next message keeps the last one's height
  // until it reports in turn, so the pane doesn't jump in between.
  const [height, setHeight] = useState(80);
  // Mail wider than the pane scrolls sideways; faded edges say there's more.
  const [hidden, setHidden] = useState({ left: false, right: false });
  const handlers = useRef({ onLink, onHoverLink });
  useEffect(() => {
    handlers.current = { onLink, onHoverLink };
  });

  // The reading font is only the default: mail that sets its own fonts keeps them.
  const font = useBodyFont();
  const frameDoc = useMemo(() => {
    if (!font) return null;
    const bridge = { url: messageBridgeUrl(), channel: newChannel() };
    const srcDoc = buildSrcDoc(html, { allowRemote, appearance, tone, font, bridge });
    return { srcDoc, channel: bridge.channel };
  }, [html, allowRemote, appearance, tone, font]);
  const channel = frameDoc?.channel;

  useEffect(() => {
    if (!channel) return;
    const onMessage = (e: MessageEvent) => {
      const frame = frameRef.current;
      if (!frame || e.source === null || e.source !== frame.contentWindow) return;
      const message = readBridgeMessage(e.data, channel);
      if (!message) return;
      switch (message.type) {
        case "link":
          handlers.current.onLink(message.href);
          break;
        case "hover":
          handlers.current.onHoverLink?.(message.href);
          break;
        case "key": {
          // The app's shortcuts (Ctrl+R, Delete...) hear the key on the frame.
          const { key, code, ctrlKey, shiftKey, altKey, metaKey } = message;
          const replay = new KeyboardEvent("keydown", {
            key,
            code,
            ctrlKey,
            shiftKey,
            altKey,
            metaKey,
            bubbles: true,
            cancelable: true,
          });
          frame.dispatchEvent(replay);
          break;
        }
        case "size": {
          // Room for a horizontal scrollbar on very wide newsletters.
          const next = Math.ceil(message.height) + (message.wide ? 16 : 0);
          setHeight(Math.max(next, 24));
          const { left, right } = message;
          setHidden((h) => (h.left === left && h.right === right ? h : { left, right }));
          break;
        }
        case "scroll":
          scrollToFrameOffset(frame, message.top);
          break;
      }
    };
    window.addEventListener("message", onMessage);
    return () => {
      window.removeEventListener("message", onMessage);
      // The link under the pointer was in the document that's going.
      handlers.current.onHoverLink?.(null);
    };
  }, [channel]);

  return (
    <div
      className="mail-body-wide"
      data-more-left={hidden.left || undefined}
      data-more-right={hidden.right || undefined}
    >
      {frameDoc === null ? (
        // The reading font's faces are still loading (only ever the first time).
        <div className="mail-body-frame" style={{ height }} />
      ) : (
        <iframe
          ref={frameRef}
          className={`mail-body-frame mail-body-frame--${appearance}`}
          title={title}
          sandbox="allow-scripts"
          referrerPolicy="no-referrer"
          srcDoc={frameDoc.srcDoc}
          style={{ height }}
        />
      )}
    </div>
  );
}

/** Scrolls what holds the frame so the frame's `top` is at the top of it. */
function scrollToFrameOffset(frame: HTMLElement, top: number): void {
  let scroller = frame.parentElement;
  while (scroller) {
    const { overflowY } = getComputedStyle(scroller);
    if (/auto|scroll/.test(overflowY) && scroller.scrollHeight > scroller.clientHeight) break;
    scroller = scroller.parentElement;
  }
  const offset = frame.getBoundingClientRect().top + top;
  if (scroller) scroller.scrollBy({ top: offset - scroller.getBoundingClientRect().top });
  else window.scrollBy({ top: offset });
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
