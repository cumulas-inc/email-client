import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { Dialog } from "./Dialog";

function dialogOf(props: { canClose?: boolean; closeOnBackdrop?: boolean } = {}) {
  const onClose = vi.fn();
  render(
    <Dialog open onClose={onClose} title="Setup" {...props}>
      <p>Body</p>
    </Dialog>,
  );
  return { onClose, dialog: screen.getByRole("dialog", { hidden: true }) };
}

describe("Dialog", () => {
  it("closes on a click on its backdrop, unless it says otherwise", () => {
    const plain = dialogOf();
    fireEvent.mouseDown(plain.dialog);
    expect(plain.onClose).toHaveBeenCalledTimes(1);
    // A click inside isn't on the backdrop.
    fireEvent.mouseDown(screen.getByText("Body"));
    expect(plain.onClose).toHaveBeenCalledTimes(1);
  });

  it("stays open on a backdrop click when closeOnBackdrop is false", () => {
    const { onClose, dialog } = dialogOf({ closeOnBackdrop: false });
    fireEvent.mouseDown(dialog);
    expect(onClose).not.toHaveBeenCalled();
    // Escape still closes it.
    fireEvent(dialog, new Event("cancel", { cancelable: true }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("opens again when the browser closes it while it can't close", () => {
    const { onClose, dialog } = dialogOf({ canClose: false });
    const showModal = vi.fn();
    Object.assign(dialog, { showModal });
    // A second Escape the browser doesn't let the page refuse.
    dialog.removeAttribute("open");
    fireEvent(dialog, new Event("close"));
    expect(onClose).not.toHaveBeenCalled();
    expect(showModal).toHaveBeenCalledTimes(1);
  });

  it("doesn't close a dialog it was opened from when the browser closes it", () => {
    const outer = vi.fn();
    const inner = vi.fn();
    render(
      <Dialog open onClose={outer} title="Settings">
        <Dialog open onClose={inner} title="Setup">
          <p>Body</p>
        </Dialog>
      </Dialog>,
    );
    const dialog = screen.getByRole("dialog", { name: "Setup", hidden: true });
    dialog.removeAttribute("open");
    fireEvent(dialog, new Event("close"));
    expect(inner).toHaveBeenCalledTimes(1);
    expect(outer).not.toHaveBeenCalled();
  });

  it("lets its owner know when the browser closes it otherwise", () => {
    const { onClose, dialog } = dialogOf();
    dialog.removeAttribute("open");
    fireEvent(dialog, new Event("close"));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
