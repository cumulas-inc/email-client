// An editable signature, kept exactly as it comes. An organization's
// signature is laid out with tables, inline styles, its own fonts and
// pictures, which the message editor's schema would strip, so it is edited
// where it's shown: in a frame of its own whose body is the editing host.
// This component reaches into the frame to size it, report edits, keep
// links from navigating it and pass the app's shortcuts on, since keys
// pressed inside never reach this document.
//
// The frame is sandboxed (no forms, popups, navigation of the app or
// downloads), but unlike a message's it has allow-scripts: WebKitGTK calls
// no event listener at all in a document that may not run scripts, even
// this component's, so without it edits, focus and keys would go unheard.
// No script can run in it all the same: its Content Security Policy allows
// none, from anywhere, on top of the app's own.
//
// Read-only, the same frame shows a signature exactly as the composer
// does (an organization's, in Settings), without editing.

import clsx from "clsx";
import {
  type Ref,
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from "react";

import { useBackend } from "../../api/context";
import { useBodyFont } from "./bodyFont";
import {
  anchorOf,
  buildSignatureSrcDoc,
  classifyLink,
  isBlankHtml,
  signatureAppearance,
} from "./bodyHtml";
import { useIsDark } from "./hooks";

export interface SignatureEditorHandle {
  focus: () => void;
}

interface SignatureEditorProps {
  /**
   * The signature's HTML. Edits come back through `onChange`; another value
   * (a different signature) replaces what's shown.
   */
  html: string;
  /** The body's HTML after each edit; "" once nothing is left. */
  onChange?: (html: string) => void;
  onBlur?: () => void;
  /** Set text the signature doesn't style like the letter (the person's own). */
  letter?: boolean;
  placeholder?: string;
  /** Lines it's tall at least. */
  minLines?: number;
  /** Shown, not edited; its links open in the browser. */
  readOnly?: boolean;
  /** The frame's name. */
  title?: string;
  className?: string;
  ref?: Ref<SignatureEditorHandle>;
}

const FORMATS: Record<string, string> = { b: "bold", i: "italic", u: "underline" };

/** The body's HTML, or "" when it shows nothing (an emptied editor leaves a <br>). */
function serialize(body: HTMLElement): string {
  const html = body.innerHTML;
  return isBlankHtml(html) ? "" : html;
}

/** What makes the document's body an editor (see buildSignatureSrcDoc). */
const EDITING = [
  "contenteditable",
  "role",
  "aria-multiline",
  "aria-label",
  "spellcheck",
  "data-placeholder",
  "data-empty",
];

/** Makes the loaded document show its signature without editing it. */
function showOnly(doc: Document): void {
  for (const name of EDITING) doc.body.removeAttribute(name);
  const style = doc.createElement("style");
  style.textContent = "body, a { cursor: auto; } a[href] { cursor: pointer; }";
  doc.head.append(style);
}

export function SignatureEditor({
  html,
  onChange,
  onBlur,
  letter = false,
  placeholder = "",
  minLines = 1,
  readOnly = false,
  title = "Signature",
  className,
  ref,
}: SignatureEditorProps) {
  const backend = useBackend();
  const frameRef = useRef<HTMLIFrameElement>(null);
  const [height, setHeight] = useState(Math.round(minLines * 25));
  const [focused, setFocused] = useState(false);
  const handlers = useRef({ onChange, onBlur, readOnly, backend });
  useEffect(() => {
    handlers.current = { onChange, onBlur, readOnly, backend };
  });

  // What the frame holds: the latest `html`, or what it last reported. It
  // goes in when the frame loads, so a new reading font (a new document)
  // keeps the edits, and typing never reloads the frame.
  const content = useRef(html);
  // What the frame shows, as its body serializes it, which may differ from
  // the HTML that went in (a table gains its <tbody>): an edit is a change
  // from this.
  const shown = useRef("");
  const teardown = useRef<(() => void) | null>(null);
  /** Shows `content` in the loaded frame. */
  const fill = useRef<(() => void) | null>(null);

  const isDark = useIsDark();
  const font = useBodyFont();
  // How it's painted follows the signature as it's edited, switched in the
  // loaded frame; the frame starts as it first was.
  const appearance = signatureAppearance(html, isDark);
  const [firstAppearance] = useState(appearance);
  const look = useRef(appearance);
  useEffect(() => {
    look.current = appearance;
    frameRef.current?.contentDocument?.documentElement?.setAttribute("data-appearance", appearance);
  }, [appearance]);
  const srcDoc = useMemo(
    () =>
      font
        ? buildSignatureSrcDoc({
            appearance: firstAppearance,
            font,
            letter,
            placeholder,
            label: title,
            minLines,
          })
        : null,
    [firstAppearance, font, letter, placeholder, title, minLines],
  );

  // Another signature: shown in place, unless it's the edit just reported.
  useEffect(() => {
    if (html === content.current) return;
    content.current = html;
    fill.current?.();
  }, [html]);

  useEffect(() => {
    const listening = teardown;
    return () => listening.current?.();
  }, []);

  useImperativeHandle(ref, () => ({
    focus: () => {
      const frame = frameRef.current;
      frame?.contentWindow?.focus();
      frame?.contentDocument?.body?.focus();
    },
  }));

  const onLoad = useCallback(() => {
    const frame = frameRef.current;
    const doc = frame?.contentDocument;
    const body = doc?.body;
    // The document from srcdoc, not the blank one a frame starts with.
    if (!frame || !doc || !body || !body.hasAttribute("contenteditable")) return;
    teardown.current?.();
    const readOnly = handlers.current.readOnly;
    if (readOnly) showOnly(doc);
    doc.documentElement.setAttribute("data-appearance", look.current);
    // A new document (the reading font changed) starts without the focus.
    setFocused(false);

    const measure = () => {
      const root = doc.documentElement;
      if (root) setHeight(Math.max(Math.ceil(root.getBoundingClientRect().height), 20));
    };
    const markEmpty = () => {
      if (!readOnly) body.toggleAttribute("data-empty", serialize(body) === "");
    };
    fill.current = () => {
      body.innerHTML = content.current;
      shown.current = serialize(body);
      markEmpty();
      measure();
    };
    fill.current();

    const report = () => {
      markEmpty();
      const next = serialize(body);
      // Ctrl+B with nothing selected, say, changes nothing yet.
      if (next === shown.current) return;
      shown.current = next;
      content.current = next;
      handlers.current.onChange?.(next);
    };

    // A click places the caret in a link's text; it never follows the link
    // in the frame. Read-only, the link opens in the browser instead.
    const stayPut = (e: MouseEvent) => {
      const anchor = anchorOf(e.target);
      if (!anchor) return;
      e.preventDefault();
      if (readOnly && e.type === "click") {
        const link = classifyLink(anchor.getAttribute?.("href"));
        if (link.kind === "open") void handlers.current.backend.openLink(link.url);
      }
    };
    // Dropped files would come in as links to this computer's files.
    const noFiles = (e: DragEvent) => {
      if (e.dataTransfer?.files.length) e.preventDefault();
    };

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.isComposing) return;
      const ctrl = (e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey;
      const key = e.key.toLowerCase();
      const format = ctrl && !readOnly ? FORMATS[key] : undefined;
      if (format) {
        e.preventDefault();
        doc.execCommand?.(format);
        report();
        return;
      }
      // Ctrl+K inserts a link in the message body; here it does nothing,
      // rather than leaving for the search box mid-edit.
      if (ctrl && key === "k" && !readOnly) {
        e.preventDefault();
        return;
      }
      // Replay the key on the frame so the app's shortcuts work here as in
      // the message body: Ctrl+Enter sends, Ctrl+1–4 switch modules...
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
      if (replay.defaultPrevented) {
        e.preventDefault();
        return;
      }
      // ...and Escape closes the dialog around it, as it would from a field
      // (a replayed key doesn't do what the key itself would).
      if (e.key === "Escape") {
        const dialog = frame.closest("dialog");
        if (dialog?.open) dialog.dispatchEvent(new Event("cancel", { cancelable: true }));
      }
    };

    const onFocus = () => setFocused(true);
    const onBlurred = () => {
      setFocused(false);
      handlers.current.onBlur?.();
    };

    body.addEventListener("input", report);
    body.addEventListener("focus", onFocus);
    body.addEventListener("blur", onBlurred);
    doc.addEventListener("keydown", onKeyDown);
    doc.addEventListener("click", stayPut);
    doc.addEventListener("auxclick", stayPut);
    doc.addEventListener("drop", noFiles);
    // Pictures and fonts change the height once they've loaded.
    doc.addEventListener("load", measure, true);
    doc.addEventListener("error", measure, true);
    let observer: ResizeObserver | null = null;
    if (typeof ResizeObserver === "function") {
      try {
        observer = new ResizeObserver(measure);
        observer.observe(doc.documentElement);
        observer.observe(frame);
      } catch {
        // An engine that can't observe another document still gets load events.
      }
    }

    teardown.current = () => {
      body.removeEventListener("input", report);
      body.removeEventListener("focus", onFocus);
      body.removeEventListener("blur", onBlurred);
      doc.removeEventListener("keydown", onKeyDown);
      doc.removeEventListener("click", stayPut);
      doc.removeEventListener("auxclick", stayPut);
      doc.removeEventListener("drop", noFiles);
      doc.removeEventListener("load", measure, true);
      doc.removeEventListener("error", measure, true);
      observer?.disconnect();
      fill.current = null;
    };
  }, []);

  return (
    <div
      className={clsx("signature-editor", `signature-editor--${appearance}`, className)}
      data-focused={focused || undefined}
    >
      {srcDoc === null ? (
        // The reading font's faces are still loading (only ever the first time).
        <div className="signature-editor__frame" style={{ height }} />
      ) : (
        <iframe
          ref={frameRef}
          className="signature-editor__frame"
          title={title}
          sandbox="allow-same-origin allow-scripts"
          referrerPolicy="no-referrer"
          srcDoc={srcDoc}
          onLoad={onLoad}
          style={{ height, colorScheme: appearance === "plain" ? "light" : "dark" }}
        />
      )}
    </div>
  );
}
