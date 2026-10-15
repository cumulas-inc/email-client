import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { type FontOption, FontPicker } from "./FontPicker";

type Font = { kind: "default" } | { kind: "family"; family: string };

const PRESETS: FontOption<Font>[] = [
  {
    key: "default",
    label: "Inter (default)",
    value: { kind: "default" },
    fontFamily: "sans-serif",
  },
];
const FAMILIES = Array.from({ length: 3000 }, (_, i) => `Family ${String(i).padStart(4, "0")}`);
const familyValue = (family: string): Font => ({ kind: "family", family });

// jsdom lays nothing out, so these stand in for the geometry: the panel the
// input is in, the input, and the list with its rows scrolled by `scrolled`.
const PANEL = { top: 100, bottom: 740 };
const LIST = { top: 100, height: 320 };
const HEADING = 30;
/** The first installed font's top in the list: padding, the preset, the heading. */
const ROWS_TOP = 4 + 32 + HEADING;
/** The list's opening scale, which shrinks its boxes on screen but not its layout. */
const SCALE = 0.97;
let inputTop = 200;
let scrolled = 0;

function rect(top: number, height: number): DOMRect {
  const box = {
    top,
    bottom: top + height,
    height,
    left: 300,
    right: 540,
    width: 240,
    x: 300,
    y: top,
  };
  return { ...box, toJSON: () => box };
}

const isList = (el: Element) => el.classList.contains("font-picker__popup");
const isHeading = (el: Element) => el.classList.contains("font-picker__group");

beforeEach(() => {
  inputTop = 200;
  scrolled = 0;
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement,
  ) {
    if (this.dataset.panel) return rect(PANEL.top, PANEL.bottom - PANEL.top);
    if (this instanceof HTMLInputElement) return rect(inputTop, 34);
    // On screen, the list is still scaling open.
    if (isList(this)) return rect(LIST.top, LIST.height * SCALE);
    if (isHeading(this)) {
      return rect(LIST.top + (ROWS_TOP - HEADING - scrolled) * SCALE, HEADING * SCALE);
    }
    return rect(0, 0);
  });
  // Its layout, which the rows rendered follow.
  const scrollTop = Object.getOwnPropertyDescriptor(Element.prototype, "scrollTop")!.get!;
  vi.spyOn(Element.prototype, "scrollTop", "get").mockImplementation(function (this: Element) {
    return isList(this) ? scrolled : scrollTop.call(this);
  });
  vi.spyOn(Element.prototype, "clientHeight", "get").mockImplementation(function (this: Element) {
    return isList(this) ? LIST.height : 0;
  });
  vi.spyOn(HTMLElement.prototype, "offsetTop", "get").mockImplementation(function (
    this: HTMLElement,
  ) {
    return isHeading(this) ? ROWS_TOP - HEADING : 0;
  });
  vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockImplementation(function (
    this: HTMLElement,
  ) {
    return isHeading(this) ? HEADING : 0;
  });
  // Scrolls an installed font into view the way the browser would.
  Object.defineProperty(Element.prototype, "scrollIntoView", {
    configurable: true,
    value(this: Element) {
      if (!this.hasAttribute("aria-posinset")) return;
      const top = ROWS_TOP + (Number(this.getAttribute("aria-posinset")) - 1) * 32;
      if (top < scrolled) scrolled = top;
      else if (top + 32 > scrolled + LIST.height) scrolled = top + 32 - LIST.height;
    },
  });
});

afterEach(() => {
  vi.restoreAllMocks();
  delete (Element.prototype as Partial<Element>).scrollIntoView;
});

function renderPicker(selectedKey = "default") {
  const onChange = vi.fn();
  render(
    <div data-panel="true">
      <span id="label">Font</span>
      <FontPicker
        id="font"
        labelledBy="label"
        presets={PRESETS}
        families={FAMILIES}
        familyValue={familyValue}
        selectedKey={selectedKey}
        selectedLabel={selectedKey}
        onChange={onChange}
      />
    </div>,
  );
  return { input: screen.getByRole("combobox"), onChange };
}

/** Makes the panel scroll, as the settings panel does, moving the input. */
function scrollingPanel(panel: HTMLElement) {
  let top = 0;
  panel.style.overflowY = "auto";
  panel.style.paddingTop = "12px";
  Object.defineProperties(panel, {
    clientHeight: { configurable: true, get: () => PANEL.bottom - PANEL.top },
    scrollHeight: { configurable: true, get: () => 2000 },
    scrollTop: {
      configurable: true,
      get: () => top,
      set(value: number) {
        inputTop -= value - top;
        top = value;
      },
    },
  });
  return {
    scrollBy(by: number) {
      panel.scrollTop += by;
      fireEvent.scroll(panel);
    },
  };
}

/** The installed fonts rendered. */
function rendered() {
  return screen
    .getAllByRole("option")
    .slice(PRESETS.length)
    .map((o) => o.textContent);
}

function highlighted(input: HTMLElement) {
  const id = input.getAttribute("aria-activedescendant");
  return id ? document.getElementById(id) : null;
}

describe("FontPicker with thousands of fonts", () => {
  it("renders only the installed fonts in view, and the highlighted one", async () => {
    const user = userEvent.setup();
    const { input } = renderPicker();
    await user.click(input);
    // Eight show under the preset and the heading, and 24 more are rendered.
    expect(rendered()).toHaveLength(32);
    expect(screen.getByRole("option", { name: "Family 0000" })).toHaveAttribute(
      "aria-posinset",
      "1",
    );
    expect(screen.getByRole("option", { name: "Family 0000" })).toHaveAttribute(
      "aria-setsize",
      "3000",
    );

    // The highlight moves past the rendered rows, and they follow.
    for (let i = 0; i < 40; i++) {
      await user.keyboard("{ArrowDown}");
      expect(highlighted(input)).toHaveAttribute("aria-selected", "true");
    }
    expect(highlighted(input)).toHaveTextContent("Family 0039");
    expect(rendered()).toContain("Family 0030");
    expect(rendered()).not.toContain("Family 0000");
    expect(rendered().length).toBeLessThan(60);

    // Far down, the highlighted font is rendered first, then those around it.
    for (let i = 0; i < 100; i++) await user.keyboard("{PageDown}");
    expect(highlighted(input)).toHaveTextContent("Family 1039");
    expect(rendered()).toContain("Family 1035");
    expect(rendered().length).toBeLessThan(60);
  });

  it("renders the fonts scrolled to, keeping the highlighted one", async () => {
    const user = userEvent.setup();
    const { input, onChange } = renderPicker(`family:Family 2500`);
    await user.click(input);
    // The current choice is highlighted and in view.
    expect(highlighted(input)).toHaveTextContent("Family 2500");
    expect(rendered()).toContain("Family 2495");

    scrolled = ROWS_TOP + 100 * 32;
    fireEvent.scroll(screen.getByRole("listbox").parentElement!);
    expect(rendered()).toContain("Family 0100");
    expect(rendered()).toContain("Family 0109");
    expect(rendered()).toContain("Family 2500");
    expect(rendered()).not.toContain("Family 2499");

    await user.click(screen.getByRole("option", { name: "Family 0105" }));
    expect(onChange).toHaveBeenCalledWith({ kind: "family", family: "Family 0105" });
  });

  it("opens below when four fonts fit there, and stays there as the panel scrolls", async () => {
    const user = userEvent.setup();
    const { input } = renderPicker();
    // 150px below the input, far more above it.
    inputTop = window.innerHeight - 8 - 4 - 150 - 34;
    await user.click(input);
    const popup = screen.getByRole("listbox").parentElement!;
    expect(popup).toHaveAttribute("data-side", "bottom");
    expect(popup.style.maxHeight).toBe("150px");

    // The panel scrolls the input down; the list follows it on the same side.
    inputTop = PANEL.bottom - 34;
    fireEvent.scroll(input.closest("[data-panel]")!);
    expect(popup).toHaveAttribute("data-side", "bottom");
    expect(popup.style.top).toBe(`${PANEL.bottom + 4}px`);

    // And closes once the input is scrolled out of view.
    inputTop = PANEL.bottom - 10;
    fireEvent.scroll(input.closest("[data-panel]")!);
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    expect(input).toHaveAttribute("aria-expanded", "false");
    expect(input).toHaveFocus();
  });

  it("keeps Page Up and Page Down in the list when nothing matches", async () => {
    const user = userEvent.setup();
    const { input } = renderPicker();
    await user.click(input);
    await user.keyboard("qqq");
    // Not passed on, which would scroll the panel and close the list.
    expect(fireEvent.keyDown(input, { key: "PageDown" })).toBe(false);
    expect(fireEvent.keyDown(input, { key: "PageUp" })).toBe(false);
    expect(input).toHaveValue("qqq");
    expect(input).toHaveAttribute("aria-expanded", "true");
  });

  it("renders the fonts scrolled to before the frame is painted", async () => {
    const user = userEvent.setup();
    const { input } = renderPicker();
    await user.click(input);
    const popup = screen.getByRole("listbox").parentElement!;
    // Outside act(), as the browser sends it: the rows are there as soon
    // as the scroll event is handled, not some time after.
    const env = globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean };
    const actEnvironment = env.IS_REACT_ACT_ENVIRONMENT;
    env.IS_REACT_ACT_ENVIRONMENT = false;
    try {
      scrolled = ROWS_TOP + 1000 * 32;
      popup.dispatchEvent(new Event("scroll"));
      expect(rendered()).toContain("Family 1000");
      expect(rendered()).not.toContain("Family 0020");
    } finally {
      env.IS_REACT_ACT_ENVIRONMENT = actEnvironment;
    }
  });

  it("brings the input back into view when it opens again after the panel scrolled it out", async () => {
    const user = userEvent.setup();
    const { input } = renderPicker();
    const panel = scrollingPanel(input.closest<HTMLElement>("[data-panel]")!);
    await user.click(input);
    expect(screen.getByRole("listbox")).toBeInTheDocument();

    // The panel scrolls the input out above its top, which closes the list.
    panel.scrollBy(150);
    expect(inputTop).toBe(50);
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    expect(input).toHaveFocus();

    // A key opens it again, with the input back in view (clear of the
    // panel's padding) and the list under it.
    await user.keyboard("{ArrowDown}");
    expect(inputTop).toBe(PANEL.top + 12);
    const popup = screen.getByRole("listbox").parentElement!;
    expect(popup).toHaveAttribute("data-side", "bottom");
    expect(popup.style.top).toBe(`${PANEL.top + 12 + 34 + 4}px`);

    // Typing does too.
    await user.keyboard("{Escape}");
    panel.scrollBy(150);
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    await user.keyboard("Family 1");
    expect(screen.getByRole("listbox")).toBeInTheDocument();
    expect(inputTop).toBe(PANEL.top + 12);
  });
});
