import { act, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { useAppStore } from "../../../app/store";
import { renderWithBackend } from "../../../test/render";
import { MailSearchField } from "./MailSearchField";
import { RECENT_SEARCHES_KEY, addRecentSearch } from "./recentSearches";

beforeEach(() => {
  window.localStorage.clear();
  useAppStore.setState({ search: "" });
});

function setup() {
  const user = userEvent.setup();
  renderWithBackend(<MailSearchField />);
  const box = screen.getByRole("combobox", { name: "Search mail" });
  return { user, box };
}

const suggestions = () => screen.queryByRole("listbox", { name: "Search suggestions" });
const options = () => within(suggestions()!).getAllByRole("option");
const text = () => useAppStore.getState().search;

describe("MailSearchField", () => {
  it("is a combobox over its suggestions, with none chosen at first", async () => {
    const { user, box } = setup();
    expect(box).toHaveAttribute("id", "global-search");
    expect(box).toHaveAttribute("type", "search");
    expect(box).toHaveAttribute("aria-autocomplete", "list");
    expect(box).toHaveAttribute("aria-controls", "mail-search-suggestions");
    expect(box).toHaveAttribute("autocomplete", "off");
    expect(box).toHaveAttribute("spellcheck", "false");
    expect(box).toHaveAttribute("aria-expanded", "false");
    expect(screen.getByRole("search")).toContainElement(box);

    await user.click(box);
    expect(box).toHaveAttribute("aria-expanded", "true");
    const list = suggestions()!;
    expect(list).toHaveAttribute("id", "mail-search-suggestions");
    // The empty box offers the filters, in a group named by its heading.
    const group = within(list).getByRole("group", { name: "Filters" });
    expect(within(group).getAllByRole("option")).toHaveLength(6);
    expect(box).not.toHaveAttribute("aria-activedescendant");
    expect(options().filter((o) => o.getAttribute("aria-selected") === "true")).toEqual([]);
  });

  it("moves through the options with the arrows, around the ends", async () => {
    const { user, box } = setup();
    await user.click(box);
    const all = options();
    await user.keyboard("{ArrowDown}");
    expect(box).toHaveAttribute("aria-activedescendant", all[0]!.id);
    expect(all[0]).toHaveAttribute("aria-selected", "true");
    await user.keyboard("{ArrowUp}");
    expect(box).toHaveAttribute("aria-activedescendant", all.at(-1)!.id);
    await user.keyboard("{ArrowDown}");
    expect(box).toHaveAttribute("aria-activedescendant", all[0]!.id);
    // Closed, an arrow opens them again, with none chosen.
    await user.keyboard("{Escape}");
    expect(suggestions()).toBeNull();
    await user.keyboard("{ArrowDown}");
    expect(suggestions()).not.toBeNull();
    expect(box).not.toHaveAttribute("aria-activedescendant");
  });

  it("keeps the text on Enter and remembers it; Enter on an option picks it", async () => {
    const { user, box } = setup();
    await user.click(box);
    await user.keyboard("lunch{Enter}");
    expect(text()).toBe("lunch");
    expect(box).toHaveAttribute("aria-expanded", "false");
    expect(JSON.parse(window.localStorage.getItem(RECENT_SEARCHES_KEY)!)).toEqual(["lunch"]);

    await user.keyboard(" is:");
    expect(within(suggestions()!).getByRole("group", { name: "Filters" })).toBeInTheDocument();
    await user.keyboard("{ArrowDown}{Enter}");
    expect(text()).toBe("lunch is:unread ");
    expect(suggestions()).toBeNull();
    // The caret after it, ready for more.
    expect((box as HTMLInputElement).selectionStart).toBe(text().length);
  });

  it("keeps the suggestions open for an operator that waits for its value", async () => {
    const { user, box } = setup();
    await user.click(box);
    await user.click(within(suggestions()!).getByRole("option", { name: /^in:/ }));
    expect(text()).toBe("in:");
    expect(box).toHaveFocus();
    const folders = within(suggestions()!).getByRole("group", { name: "Folders" });
    expect(within(folders).getByRole("option", { name: "In Inbox" })).toBeInTheDocument();
    await user.click(within(folders).getByRole("option", { name: "In Archive" }));
    expect(text()).toBe("in:archive ");
    expect(suggestions()).toBeNull();
  });

  it("closes on Escape, then clears; Tab closes without a pick", async () => {
    const { user, box } = setup();
    await user.click(box);
    await user.keyboard("is");
    expect(suggestions()).not.toBeNull();
    await user.keyboard("{Escape}");
    expect(suggestions()).toBeNull();
    expect(text()).toBe("is");
    expect(box).toHaveFocus();
    await user.keyboard("{Escape}");
    expect(text()).toBe("");
    expect(box).not.toHaveFocus();

    await user.click(box);
    await user.keyboard("{ArrowDown}{Tab}");
    expect(text()).toBe("");
    expect(suggestions()).toBeNull();
  });

  it("forgets a recent search on Shift+Delete, and says so", async () => {
    act(() => {
      addRecentSearch("lunch");
      addRecentSearch("atlas");
    });
    const { user, box } = setup();
    await user.click(box);
    const recent = within(suggestions()!).getByRole("group", { name: "Recent searches" });
    expect(
      within(recent)
        .getAllByRole("option")
        .map((o) => o.textContent),
    ).toEqual(["atlas", "lunch", "Clear recent searches"]);
    await user.keyboard("{ArrowDown}{Shift>}{Delete}{/Shift}");
    expect(screen.getByRole("status")).toHaveTextContent("Removed “atlas” from recent searches.");
    expect(JSON.parse(window.localStorage.getItem(RECENT_SEARCHES_KEY)!)).toEqual(["lunch"]);
    expect(text()).toBe("");
    // Not on another option.
    await user.keyboard("{ArrowUp}{Shift>}{Delete}{/Shift}");
    expect(JSON.parse(window.localStorage.getItem(RECENT_SEARCHES_KEY)!)).toEqual(["lunch"]);

    // All of them.
    await user.click(within(suggestions()!).getByRole("option", { name: "Clear recent searches" }));
    expect(window.localStorage.getItem(RECENT_SEARCHES_KEY)).toBeNull();
    expect(box).toHaveFocus();
  });

  it("takes a pick by the pointer without taking the keyboard from the box", async () => {
    act(() => addRecentSearch("from:megan lunch"));
    const { user, box } = setup();
    await user.click(box);
    const option = within(suggestions()!).getByRole("option", { name: "from:megan lunch" });
    await user.hover(option);
    expect(box).toHaveAttribute("aria-activedescendant", option.id);
    await user.click(option);
    expect(text()).toBe("from:megan lunch");
    expect(box).toHaveFocus();
    expect(suggestions()).toBeNull();
  });

  it("offers people for from:, and inserts their address", async () => {
    const { user, box } = setup();
    await user.click(box);
    await user.keyboard("from:pri");
    const people = await screen.findByRole("group", { name: "People" });
    const priya = within(people).getByRole("option", {
      name: "From Priya Raman, priya.raman@acme.example",
    });
    await user.click(priya);
    expect(text()).toBe("from:priya.raman@acme.example ");
  });
});
