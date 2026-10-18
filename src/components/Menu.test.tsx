import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { Menu, type MenuItem } from "./Menu";

function renderMenu(items: MenuItem[]) {
  render(
    <Menu
      label="Choices"
      items={items}
      trigger={(props) => (
        <button type="button" {...props}>
          Open
        </button>
      )}
    />,
  );
}

describe("Menu", () => {
  it("announces one choice of several as radio items, checked or not", async () => {
    const user = userEvent.setup();
    const pick = vi.fn();
    renderMenu([
      { label: "Any time", radio: true, checked: false, onSelect: pick },
      { label: "Today", radio: true, checked: true, onSelect: pick },
      { label: "Custom range…", radio: true, separator: true, onSelect: pick },
    ]);
    await user.click(screen.getByRole("button", { name: "Open" }));
    expect(screen.getByRole("menuitemradio", { name: "Any time" })).toHaveAttribute(
      "aria-checked",
      "false",
    );
    expect(screen.getByRole("menuitemradio", { name: "Today" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    expect(screen.getByRole("menuitemradio", { name: "Custom range…" })).toHaveAttribute(
      "aria-checked",
      "false",
    );
    await user.click(screen.getByRole("menuitemradio", { name: "Any time" }));
    expect(pick).toHaveBeenCalledTimes(1);
    // A pick closes the menu and gives the trigger the keyboard back.
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open" })).toHaveFocus();
  });

  it("keeps check items and plain items as they were", async () => {
    const user = userEvent.setup();
    renderMenu([
      { label: "Dark background", checked: true, onSelect: () => {} },
      { label: "Copy", onSelect: () => {} },
    ]);
    await user.click(screen.getByRole("button", { name: "Open" }));
    expect(screen.getByRole("menuitemcheckbox", { name: "Dark background" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    const copy = screen.getByRole("menuitem", { name: "Copy" });
    expect(copy).not.toHaveAttribute("aria-checked");
  });
});
