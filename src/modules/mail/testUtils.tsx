// Test helpers for the Mail module.

import { act, screen, waitFor, within } from "@testing-library/react";

import type { Backend } from "../../api/backend";
import { MockBackend } from "../../api/mock";
import type { Settings } from "../../api/types";
import { useAppStore } from "../../app/store";
import { Toaster } from "../../components/Toaster";
import { useToasts } from "../../components/toast";
import { renderWithBackend } from "../../test/render";
import { MailModule } from "./MailModule";
import { resetMailStore } from "./mailStore";

/** jsdom lacks the layout APIs ProseMirror touches when focusing. */
function polyfillLayout() {
  const rect = () => ({
    x: 0,
    y: 0,
    top: 0,
    left: 0,
    right: 0,
    bottom: 0,
    width: 0,
    height: 0,
    toJSON: () => ({}),
  });
  const list = () => Object.assign([], { item: () => null }) as unknown as DOMRectList;
  if (!Range.prototype.getClientRects) Range.prototype.getClientRects = list;
  if (!Range.prototype.getBoundingClientRect) {
    Range.prototype.getBoundingClientRect = rect as unknown as () => DOMRect;
  }
  if (!document.elementFromPoint) document.elementFromPoint = () => null;
}

export function resetStores(): void {
  resetMailStore();
  useAppStore.setState({ search: "", composeRequest: null, module: "mail" });
  useToasts.setState({ toasts: [] });
  try {
    window.localStorage.clear();
  } catch {
    // ignore
  }
}

export async function renderMail(
  options: { backend?: Backend; settings?: Partial<Settings> } = {},
) {
  polyfillLayout();
  resetStores();
  const backend = options.backend ?? new MockBackend();
  if (options.settings) {
    const current = await backend.getSettings();
    await backend.updateSettings({ ...current, ...options.settings });
  }
  const result = renderWithBackend(
    <>
      <MailModule />
      <Toaster />
    </>,
    { backend },
  );
  // Wait for the first inbox to be listed.
  await screen.findByRole("listbox", { name: /^Messages in/ });
  return result;
}

/** The message row whose accessible name matches. */
export function row(name: RegExp | string) {
  return screen.getByRole("option", { name: typeof name === "string" ? new RegExp(name) : name });
}

export function folderTree() {
  return screen.getByRole("tree", { name: "Folders" });
}

export function messageToolbar() {
  return screen.getByRole("toolbar", { name: "Message actions" });
}

export function toolbarButton(name: string | RegExp) {
  return within(messageToolbar()).getByRole("button", { name });
}

/** Reply, Reply all or Forward: the group beside the sender. */
export function respondButton(name: string | RegExp) {
  return within(screen.getByRole("group", { name: "Respond" })).getByRole("button", { name });
}

export async function setSearch(text: string) {
  await act(async () => {
    useAppStore.getState().setSearch(text);
  });
}

/** A promise that tests resolve or reject by hand. */
export function deferred<T = void>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/**
 * Loads a frame's srcdoc, which jsdom leaves blank: writes it into the
 * frame, which then fires load (a signature frame fills on it), and waits
 * for its body.
 */
export async function loadFrame(frame: HTMLIFrameElement): Promise<Document> {
  const doc = frame.contentDocument!;
  doc.open();
  doc.write(frame.getAttribute("srcdoc") ?? "");
  doc.close();
  await waitFor(() => {
    if (!doc.body) throw new Error("not loaded");
  });
  return doc;
}

/** Types into an editable signature's frame: sets its body and reports the edit. */
export function editFrame(doc: Document, html: string): void {
  act(() => {
    doc.body.innerHTML = html;
    doc.body.dispatchEvent(new doc.defaultView!.Event("input", { bubbles: true }));
  });
}
