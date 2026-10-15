import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ACME_SIGNATURE } from "../../api/mock/data";
import { renderWithBackend } from "../../test/render";
import { buildSignatureSrcDoc, DEFAULT_BODY_FONT, isBlankHtml } from "./bodyHtml";
import { SignatureEditor } from "./SignatureEditor";
import { editFrame, loadFrame } from "./testUtils";

function frame() {
  return screen.getByTitle("Signature") as HTMLIFrameElement;
}

function key(doc: Document, init: KeyboardEventInit) {
  const event = new doc.defaultView!.KeyboardEvent("keydown", {
    bubbles: true,
    cancelable: true,
    ...init,
  });
  act(() => void doc.body.dispatchEvent(event));
  return event;
}

afterEach(() => {
  delete document.documentElement.dataset.theme;
});

describe("SignatureEditor", () => {
  it("shows the signature exactly as it comes, in a sandboxed editable frame", async () => {
    renderWithBackend(<SignatureEditor html={ACME_SIGNATURE} onChange={() => undefined} />);
    const el = frame();
    // No scripts, forms or navigation; the app reaches in, nothing else.
    expect(el.getAttribute("sandbox")).toBe("allow-same-origin");
    const srcdoc = el.getAttribute("srcdoc") ?? "";
    expect(srcdoc).toContain(
      "default-src 'none'; img-src https: data:; style-src 'unsafe-inline'; font-src https: data:",
    );
    expect(srcdoc).toMatch(
      /<body contenteditable="true" role="textbox" aria-multiline="true"[^>]* aria-label="Signature"/,
    );
    // No reader styles: the signature looks as it will be sent.
    expect(srcdoc).not.toContain("max-width: 72ch");
    expect(srcdoc).not.toMatch(/\bp \{ margin/);

    const doc = await loadFrame(el);
    // Tables, inline styles and the inline logo, untouched.
    expect(doc.body.innerHTML).toBe(
      new DOMParser().parseFromString(ACME_SIGNATURE, "text/html").body.innerHTML,
    );
    expect(doc.body.hasAttribute("data-empty")).toBe(false);
  });

  it("reports edits as the body's HTML, and an emptied editor as none", async () => {
    const onChange = vi.fn();
    renderWithBackend(
      <SignatureEditor html="<div>Ada</div>" placeholder="Add a signature" onChange={onChange} />,
    );
    expect(frame().getAttribute("srcdoc")).toContain('data-placeholder="Add a signature"');
    const doc = await loadFrame(frame());

    editFrame(doc, "<div>Ada <b>Lovelace</b></div>");
    expect(onChange).toHaveBeenLastCalledWith("<div>Ada <b>Lovelace</b></div>");
    expect(doc.body.hasAttribute("data-empty")).toBe(false);

    // What an editor leaves when everything is deleted.
    editFrame(doc, "<div><br></div>");
    expect(onChange).toHaveBeenLastCalledWith("");
    // The placeholder shows.
    expect(doc.body.hasAttribute("data-empty")).toBe(true);
    // A picture alone is a signature.
    editFrame(doc, '<img src="data:image/png;base64,AA">');
    expect(onChange).toHaveBeenLastCalledWith('<img src="data:image/png;base64,AA">');
  });

  it("keeps edits in place, and shows another signature when one is given", async () => {
    function Harness() {
      const [html, setHtml] = useState("<div>Ada</div>");
      return (
        <>
          <SignatureEditor html={html} onChange={setHtml} />
          <button type="button" onClick={() => setHtml("<div>Grace</div>")}>
            Other
          </button>
        </>
      );
    }
    renderWithBackend(<Harness />);
    const srcdoc = frame().getAttribute("srcdoc");
    const doc = await loadFrame(frame());
    editFrame(doc, "<div>Ada L.</div>");
    // Typing doesn't reload the frame (which would lose the caret).
    expect(frame().getAttribute("srcdoc")).toBe(srcdoc);
    expect(doc.body.innerHTML).toBe("<div>Ada L.</div>");

    fireEvent.click(screen.getByRole("button", { name: "Other" }));
    await waitFor(() => expect(doc.body.innerHTML).toBe("<div>Grace</div>"));
  });

  it("never follows a link, so its text can be edited", async () => {
    renderWithBackend(
      <SignatureEditor
        html='<a href="https://acme.example">acme.example</a>'
        onChange={() => undefined}
      />,
    );
    const doc = await loadFrame(frame());
    const click = new doc.defaultView!.MouseEvent("click", { bubbles: true, cancelable: true });
    doc.querySelector("a")!.dispatchEvent(click);
    expect(click.defaultPrevented).toBe(true);
    // Elsewhere, clicks are the editor's.
    const plain = new doc.defaultView!.MouseEvent("click", { bubbles: true, cancelable: true });
    doc.body.dispatchEvent(plain);
    expect(plain.defaultPrevented).toBe(false);
  });

  it("passes keys on to the app, so Ctrl+Enter sends", async () => {
    const onKeyDown = vi.fn((e: React.KeyboardEvent) => {
      if (e.ctrlKey && e.key === "Enter") e.preventDefault();
    });
    renderWithBackend(
      <section onKeyDown={onKeyDown}>
        <SignatureEditor html="<div>Ada</div>" onChange={() => undefined} />
      </section>,
    );
    const doc = await loadFrame(frame());
    const send = key(doc, { key: "Enter", ctrlKey: true });
    expect(onKeyDown).toHaveBeenCalledTimes(1);
    expect(onKeyDown.mock.calls[0]![0]).toMatchObject({ key: "Enter", ctrlKey: true });
    // Handled there, so the editor doesn't also start a new line.
    expect(send.defaultPrevented).toBe(true);

    // Typing goes on as usual.
    const typed = key(doc, { key: "a" });
    expect(onKeyDown).toHaveBeenCalledTimes(2);
    expect(typed.defaultPrevented).toBe(false);
  });

  it("formats with Ctrl+B, I and U, and keeps Ctrl+K from leaving", async () => {
    const onKeyDown = vi.fn();
    const onChange = vi.fn();
    renderWithBackend(
      <div onKeyDown={onKeyDown}>
        <SignatureEditor html="<div>Ada</div>" onChange={onChange} />
      </div>,
    );
    const doc = await loadFrame(frame());
    const execCommand = vi.fn((command: string) => {
      doc.body.innerHTML = `<div><${command[0]}>Ada</${command[0]}></div>`;
      return true;
    });
    doc.execCommand = execCommand;

    for (const [letter, command] of [
      ["b", "bold"],
      ["i", "italic"],
      ["u", "underline"],
    ]) {
      const event = key(doc, { key: letter!, ctrlKey: true });
      expect(event.defaultPrevented).toBe(true);
      expect(execCommand).toHaveBeenLastCalledWith(command);
    }
    expect(onChange).toHaveBeenLastCalledWith("<div><u>Ada</u></div>");
    expect(key(doc, { key: "k", ctrlKey: true }).defaultPrevented).toBe(true);
    // None of these reach the app (Ctrl+U marks mail unread there).
    expect(onKeyDown).not.toHaveBeenCalled();
  });

  it("closes the dialog around it on Escape, as a field would", async () => {
    const onCancel = vi.fn((e: React.SyntheticEvent) => e.preventDefault());
    renderWithBackend(
      <dialog open onCancel={onCancel}>
        <SignatureEditor html="<div>Ada</div>" onChange={() => undefined} />
      </dialog>,
    );
    const doc = await loadFrame(frame());
    key(doc, { key: "Escape" });
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("keeps a signature's own colors on a light sheet in the dark theme", async () => {
    document.documentElement.dataset.theme = "dark";
    const { unmount } = renderWithBackend(
      <SignatureEditor html={ACME_SIGNATURE} onChange={() => undefined} />,
    );
    expect(frame().closest(".signature-editor")).toHaveClass("signature-editor--paper");
    expect(frame().getAttribute("srcdoc")).toContain("background: #ffffff");
    unmount();

    // Plain text takes the dark theme's colors instead.
    renderWithBackend(<SignatureEditor html="<div>Ada</div>" onChange={() => undefined} />);
    expect(frame().closest(".signature-editor")).toHaveClass("signature-editor--dark");
    expect(frame().style.colorScheme).toBe("dark");
    expect(frame().getAttribute("srcdoc")).toContain("color-scheme: dark");
  });
});

describe("buildSignatureSrcDoc", () => {
  it("sets the own signature like the letter, and leaves an organization's neutral", () => {
    const options = {
      appearance: "plain" as const,
      font: DEFAULT_BODY_FONT,
      placeholder: 'Say "hi" <here>',
      label: "Signature",
      minLines: 3,
    };
    const own = buildSignatureSrcDoc({ ...options, letter: true });
    expect(own).toContain("line-height: 1.6;");
    expect(own).toContain("min-height: calc(3 * 1.6em);");
    expect(own).toContain('data-placeholder="Say &quot;hi&quot; &lt;here>"');
    expect(buildSignatureSrcDoc({ ...options, letter: false })).toContain("line-height: normal;");
  });

  it("knows an empty editor from a signature", () => {
    expect(isBlankHtml("")).toBe(true);
    expect(isBlankHtml("<div><br></div>")).toBe(true);
    expect(isBlankHtml("<p>&nbsp;</p>")).toBe(true);
    expect(isBlankHtml("<style>p{}</style>")).toBe(true);
    expect(isBlankHtml("<div>Ada</div>")).toBe(false);
    expect(isBlankHtml('<img src="data:image/png;base64,AA">')).toBe(false);
  });
});
