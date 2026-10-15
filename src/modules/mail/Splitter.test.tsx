import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { Splitter } from "./Splitter";

/** A pane the user set to `initial` px (260 unless given), and its splitter. */
function Panes({ initial = 260, minWidth }: { initial?: number; minWidth?: number }) {
  const [width, setWidth] = useState(initial);
  return (
    <div>
      <div className="pane" style={minWidth ? { minWidth } : undefined} />
      <Splitter
        orientation="vertical"
        value={width}
        min={180}
        max={420}
        onChange={setWidth}
        label="Resize folder pane"
      />
    </div>
  );
}

/** jsdom has no layout: a narrow window shows the pane `width` px wide. */
function layOut(width: number) {
  vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockImplementation(function (
    this: HTMLElement,
  ) {
    return this.classList.contains("pane") ? width : 0;
  });
}

const splitter = () => screen.getByRole("separator", { name: "Resize folder pane" });

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("Splitter", () => {
  it("drags from the pane's size on screen when the window squeezed it", () => {
    layOut(208);
    render(<Panes />);
    fireEvent.pointerDown(splitter(), { button: 0, clientX: 300 });
    fireEvent.pointerMove(splitter(), { clientX: 290 });
    // Not 250, which the layout would squeeze back to 208.
    expect(splitter()).toHaveAttribute("aria-valuenow", "198");
    fireEvent.pointerUp(splitter());
  });

  it("drags wider from the size the user chose when the window squeezed the pane", () => {
    layOut(208);
    render(<Panes />);
    fireEvent.pointerDown(splitter(), { button: 0, clientX: 300 });
    fireEvent.pointerMove(splitter(), { clientX: 310 });
    // Not 218: the squeezed pane cannot grow, and a wider window shows 270.
    expect(splitter()).toHaveAttribute("aria-valuenow", "270");
    fireEvent.pointerMove(splitter(), { clientX: 300 });
    expect(splitter()).toHaveAttribute("aria-valuenow", "260");
    fireEvent.pointerUp(splitter());
  });

  it("steps from the pane's size on screen", () => {
    layOut(208);
    render(<Panes />);
    fireEvent.keyDown(splitter(), { key: "ArrowLeft" });
    expect(splitter()).toHaveAttribute("aria-valuenow", "192");
  });

  it("steps wider from the size the user chose when the window squeezed the pane", () => {
    layOut(208);
    render(<Panes />);
    fireEvent.keyDown(splitter(), { key: "ArrowRight" });
    // Not 224, which would forget the 260 the user chose.
    expect(splitter()).toHaveAttribute("aria-valuenow", "276");
  });

  it("steps narrower from the size the user chose when a min-width holds the pane wider", () => {
    layOut(200);
    render(<Panes initial={190} />);
    fireEvent.keyDown(splitter(), { key: "ArrowLeft" });
    // Not 184, which would widen what the user chose.
    expect(splitter()).toHaveAttribute("aria-valuenow", "174");
    fireEvent.keyDown(splitter(), { key: "ArrowRight" });
    expect(splitter()).toHaveAttribute("aria-valuenow", "216");
  });

  it("reports the pane's size on screen", () => {
    layOut(208);
    vi.stubGlobal(
      "ResizeObserver",
      class {
        constructor(private readonly callback: () => void) {}
        observe() {
          this.callback();
        }
        disconnect() {}
      },
    );
    render(<Panes />);
    expect(splitter()).toHaveAttribute("aria-valuenow", "208");
  });

  it("keeps the size the user chose after a press that doesn't move sideways", () => {
    // A min-width holds the pane at 200, wider than the 180 chosen for smaller windows.
    layOut(200);
    render(<Panes initial={180} />);
    fireEvent.pointerDown(splitter(), { button: 0, clientX: 300, clientY: 100 });
    fireEvent.pointerMove(splitter(), { clientX: 300, clientY: 140 });
    expect(splitter()).toHaveAttribute("aria-valuenow", "180");
    fireEvent.pointerMove(splitter(), { clientX: 315, clientY: 140 });
    expect(splitter()).toHaveAttribute("aria-valuenow", "215");
    fireEvent.pointerMove(splitter(), { clientX: 300, clientY: 140 });
    expect(splitter()).toHaveAttribute("aria-valuenow", "180");
    fireEvent.pointerUp(splitter());
  });

  it("reports the least the pane can be at this window size", () => {
    layOut(208);
    vi.stubGlobal(
      "ResizeObserver",
      class {
        constructor(private readonly callback: () => void) {}
        observe() {
          this.callback();
        }
        disconnect() {}
      },
    );
    render(<Panes minWidth={200} />);
    // Not 180: at this size a min-width holds the pane at 200.
    expect(splitter()).toHaveAttribute("aria-valuemin", "200");
  });

  it("works from its value when nothing is laid out", () => {
    render(<Panes />);
    expect(splitter()).toHaveAttribute("aria-valuenow", "260");
    fireEvent.keyDown(splitter(), { key: "ArrowLeft" });
    expect(splitter()).toHaveAttribute("aria-valuenow", "244");
  });
});
