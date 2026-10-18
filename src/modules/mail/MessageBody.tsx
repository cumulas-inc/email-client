// Renders untrusted email HTML. The HTML was sanitized by the backend; as
// defense in depth it is shown in an <iframe sandbox="allow-scripts">
// without allow-same-origin: an opaque origin, which can't reach the app or
// its IPC (no forms, popups or top navigation either), with a Content
// Security Policy that allows only inline styles, data:/cid: images (and
// remote ones once the user allows them), data: fonts (the reading face
// travels inside the document) and one script, the bridge.
//
// WebKitGTK calls no event listener in a frame that may not run scripts,
// this component's included, and an opaque frame can't be reached into, so
// the bridge (public/message-bridge.js) runs inside and reports by
// postMessage: links clicked and pointed at, keys pressed, the document's
// size. Messages count only from this frame, on the channel its document
// was given. The app tells the bridge what a search found, to mark, on a
// second channel the frame never sends on.

import { Fragment, type ReactNode, useEffect, useMemo, useRef, useState } from "react";

import { type HitPattern, MAX_SCAN_CHARS, findHits } from "../../lib/search/highlight";

import { useBodyFont } from "./bodyFont";
import {
  type BodyAppearance,
  type BodyTone,
  type BridgeCommand,
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
  /** What a search found, to mark in the message; none clears the marks. */
  highlight?: readonly HitPattern[];
  /** The match to show, of those marked. */
  current?: number | null;
  /** Changes with each step to a match, even to the one shown: it is shown again. */
  revealStep?: number;
  /** How many matches were marked. */
  onHits?: (count: number) => void;
}

const NO_PATTERNS: readonly HitPattern[] = [];

export function MessageBody({
  html,
  allowRemote,
  appearance,
  tone = "normal",
  title,
  onLink,
  onHoverLink,
  highlight = NO_PATTERNS,
  current = null,
  revealStep = 0,
  onHits,
}: MessageBodyProps) {
  const frameRef = useRef<HTMLIFrameElement>(null);
  // Until the bridge reports. The next message keeps the last one's height
  // until it reports in turn, so the pane doesn't jump in between.
  const [height, setHeight] = useState(80);
  // Mail wider than the pane scrolls sideways; faded edges say there's more.
  const [hidden, setHidden] = useState({ left: false, right: false });
  const handlers = useRef({ onLink, onHoverLink, onHits });
  useEffect(() => {
    handlers.current = { onLink, onHoverLink, onHits };
  });

  // The reading font is only the default: mail that sets its own fonts keeps them.
  const font = useBodyFont();
  // One document and channel until what they're built from changes (the
  // font stays the same object until then): a new one reloads the frame.
  const frameDoc = useMemo(() => {
    if (!font) return null;
    const bridge = { url: messageBridgeUrl(), channel: newChannel(), inbound: newChannel() };
    const srcDoc = buildSrcDoc(html, { allowRemote, appearance, tone, font, bridge });
    return { srcDoc, channel: bridge.channel, inbound: bridge.inbound };
  }, [html, allowRemote, appearance, tone, font]);
  const channel = frameDoc?.channel;
  const inbound = frameDoc?.inbound;
  // The document whose bridge said it listens (by its inbound secret).
  const [readyFor, setReadyFor] = useState<string | null>(null);
  const ready = !!inbound && readyFor === inbound;
  // The same words typed again are the same list: nothing is sent again.
  const patterns = JSON.stringify(highlight);

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
        case "ready":
          setReadyFor(inbound ?? null);
          break;
        case "hits":
          handlers.current.onHits?.(message.count);
          break;
        case "reveal":
          revealFrameRange(frame, message.top, message.bottom, findBarHeight(frame));
          break;
      }
    };
    window.addEventListener("message", onMessage);
    return () => {
      window.removeEventListener("message", onMessage);
      // The link under the pointer was in the document that's going.
      handlers.current.onHoverLink?.(null);
    };
  }, [channel, inbound]);

  // What to mark goes to the bridge once it listens, and again when it
  // changes (an empty list clears); then which match to show.
  useEffect(() => {
    if (!ready) return;
    const command: BridgeCommand = {
      inbound: inbound!,
      type: "highlight",
      patterns: JSON.parse(patterns) as HitPattern[],
    };
    // The frame's origin is opaque, so no target origin names it.
    frameRef.current?.contentWindow?.postMessage(command, "*");
  }, [ready, inbound, patterns]);
  useEffect(() => {
    if (!ready || current === null) return;
    const command: BridgeCommand = { inbound: inbound!, type: "reveal", index: current };
    frameRef.current?.contentWindow?.postMessage(command, "*");
  }, [ready, inbound, current, revealStep]);

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

/** What scrolls the frame up and down: the closest ancestor that scrolls. */
function scrollerOf(frame: HTMLElement): HTMLElement | null {
  let scroller = frame.parentElement;
  while (scroller) {
    const { overflowY } = getComputedStyle(scroller);
    if (/auto|scroll/.test(overflowY) && scroller.scrollHeight > scroller.clientHeight) break;
    scroller = scroller.parentElement;
  }
  return scroller;
}

/** Scrolls what holds the frame so the frame's `top` is at the top of it. */
function scrollToFrameOffset(frame: HTMLElement, top: number): void {
  const scroller = scrollerOf(frame);
  const offset = frame.getBoundingClientRect().top + top;
  if (scroller) scroller.scrollBy({ top: offset - scroller.getBoundingClientRect().top });
  else window.scrollBy({ top: offset });
}

/** Room kept around a match shown, so its lines around it show too. */
const REVEAL_MARGIN = 48;

/** The reader's find bar's height: it covers the top of what scrolls under it. */
function findBarHeight(frame: HTMLElement): number {
  const bar = frame.closest(".mail-reader__scroll")?.querySelector(".mail-findbar");
  return bar?.getBoundingClientRect().height ?? 0;
}

/**
 * Scrolls what holds the frame as little as shows the frame's `top` to
 * `bottom`, with a margin, below the top `inset` (the find bar).
 */
function revealFrameRange(frame: HTMLElement, top: number, bottom: number, inset: number): void {
  const scroller = scrollerOf(frame);
  const frameTop = frame.getBoundingClientRect().top;
  const view = scroller ? scroller.getBoundingClientRect() : { top: 0, bottom: window.innerHeight };
  const start = frameTop + top - REVEAL_MARGIN - inset;
  const end = frameTop + bottom + REVEAL_MARGIN;
  let by = 0;
  if (start < view.top) by = start - view.top;
  else if (end > view.bottom) by = Math.min(end - view.bottom, start - view.top);
  if (by === 0) return;
  if (scroller) scroller.scrollBy({ top: by });
  else window.scrollBy({ top: by });
}

/** Part of a line of plain text: a link or not, its runs, and the match each is, if one. */
interface LinkedPart {
  href: string | null;
  runs: { text: string; hit: number | null }[];
}

/**
 * Each text of a plain-text body, in the order shown, as links and runs,
 * with what `patterns` find numbered in that order (1,000 at most). The
 * scan stops after MAX_SCAN_CHARS of the body in all.
 */
function markTexts(texts: string[], patterns: readonly HitPattern[]): LinkedPart[][] {
  let budget = MAX_SCAN_CHARS;
  let numbered = 0;
  return texts.map((text) => {
    const hits =
      patterns.length > 0 && budget > 0 && numbered < MAX_HITS
        ? findHits(text, patterns, budget)
            .slice(0, MAX_HITS - numbered)
            .map(([start, end]) => ({ start, end, k: numbered++ }))
        : [];
    budget -= text.length;
    let offset = 0;
    return linkify(text).map((part) => {
      const start = offset;
      const end = (offset += part.text.length);
      const runs: LinkedPart["runs"] = [];
      let at = start;
      for (const hit of hits) {
        if (hit.end <= start || hit.start >= end) continue;
        const from = Math.max(hit.start, start);
        const to = Math.min(hit.end, end);
        if (from > at) runs.push({ text: text.slice(at, from), hit: null });
        runs.push({ text: text.slice(from, to), hit: hit.k });
        at = to;
      }
      if (at < end) runs.push({ text: text.slice(at, end), hit: null });
      return { href: "href" in part ? part.href : null, runs };
    });
  });
}

/** Most matches marked in a message. */
const MAX_HITS = 1000;

function Runs({ runs, current }: { runs: LinkedPart["runs"]; current: number | null }) {
  return runs.map((run, i) =>
    run.hit === null ? (
      <Fragment key={i}>{run.text}</Fragment>
    ) : (
      <mark
        key={i}
        className={run.hit === current ? "search-hit is-current" : "search-hit"}
        data-hit={run.hit}
      >
        {run.text}
      </mark>
    ),
  );
}

function Linked({
  parts,
  current,
  onLink,
}: {
  parts: LinkedPart[];
  current: number | null;
  onLink: (href: string) => void;
}) {
  return parts.map((part, i) =>
    part.href !== null ? (
      <a
        key={i}
        href={part.href}
        title={part.href}
        onClick={(e) => {
          e.preventDefault();
          onLink(part.href!);
        }}
        onAuxClick={(e) => e.preventDefault()}
      >
        <Runs runs={part.runs} current={current} />
      </a>
    ) : (
      <span key={i}>
        <Runs runs={part.runs} current={current} />
      </span>
    ),
  );
}

const NO_HITS: readonly HitPattern[] = [];

/**
 * Plain text, set like the letter: paragraphs in the serif, each in its own
 * direction; quoted lines quieter; tables and aligned columns in monospace,
 * unwrapped, scrolling sideways when they're wider than the pane. What a
 * search found is marked, and the match shown scrolled to.
 */
export function PlainTextBody({
  text,
  onLink,
  highlight = NO_HITS,
  current = null,
  revealStep = 0,
  onHits,
}: {
  text: string;
  onLink: (href: string) => void;
  highlight?: readonly HitPattern[];
  current?: number | null;
  /** As MessageBody's. */
  revealStep?: number;
  onHits?: (count: number) => void;
}): ReactNode {
  const blocks = useMemo(() => plainTextBlocks(text), [text]);
  const marked = useMemo(
    () =>
      markTexts(
        blocks.flatMap((b) => (b.kind === "text" ? b.lines.map((l) => l.text) : [b.text])),
        highlight,
      ),
    [blocks, highlight],
  );
  const count = useMemo(() => {
    const hits = new Set<number>();
    for (const parts of marked) {
      for (const part of parts)
        for (const run of part.runs) if (run.hit !== null) hits.add(run.hit);
    }
    return hits.size;
  }, [marked]);
  const ref = useRef<HTMLDivElement>(null);
  const onHitsRef = useRef(onHits);
  useEffect(() => {
    onHitsRef.current = onHits;
  });
  // Said again for each new marking, even of as many matches.
  useEffect(() => {
    onHitsRef.current?.(count);
  }, [marked, count]);
  useEffect(() => {
    if (current === null) return;
    ref.current?.querySelector(`[data-hit="${current}"]`)?.scrollIntoView?.({ block: "center" });
  }, [current, revealStep]);

  let next = 0;
  const linked = () => <Linked parts={marked[next++] ?? []} current={current} onLink={onLink} />;
  return (
    <div ref={ref} className="mail-plaintext selectable">
      {blocks.map((block, i) => {
        if (block.kind === "pre") {
          return (
            <pre key={i} className="mail-plaintext__pre" tabIndex={0}>
              {linked()}
            </pre>
          );
        }
        if (block.kind === "signature") {
          return (
            <p key={i} className="mail-plaintext__p mail-plaintext__signature">
              {linked()}
            </p>
          );
        }
        return (
          <p key={i} className="mail-plaintext__p">
            {block.lines.map((line, j) => (
              <span key={j} className={line.quoted ? "mail-plaintext__quote" : undefined}>
                {linked()}
                {j < block.lines.length - 1 ? "\n" : ""}
              </span>
            ))}
          </p>
        );
      })}
    </div>
  );
}
