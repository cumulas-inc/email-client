import { describe, expect, it } from "vitest";

import type { Account, EmailAddress, Folder } from "../../../api/types";
import { folder } from "../../../api/mock/data";
import {
  type Applied,
  MAX_SUGGESTIONS,
  type SuggestionGroup,
  type SuggestionInput,
  buildSuggestions,
  peopleQuery,
} from "./suggestions";

const FOLDERS: Folder[] = [
  folder("w", "w-inbox", "Inbox", "inbox"),
  folder("w", "w-sent", "Sent Items", "sent"),
  folder("w", "w-archive", "Archive", "archive"),
  folder("w", "w-projects", "Project  X", "other"),
  folder("h", "h-projects", "project x", "other"),
  folder("h", "h-q", "Q1\\Q2 plans", "other"),
];

const ACCOUNTS = [
  { id: "w", displayName: "Alex Morgan", email: "alex@acme.example" },
  { id: "h", displayName: "Alex (Personal)", email: "alex@home.example" },
] as Account[];

const PEOPLE: EmailAddress[] = [
  { name: "Megan Bowen", address: "megan@contoso.example" },
  { name: null, address: "megan.b@fabrikam.example" },
];

function build(text: string, input: Partial<SuggestionInput> = {}): SuggestionGroup[] {
  return buildSuggestions({
    text,
    caret: input.caret ?? text.length,
    recent: [],
    people: [],
    folders: FOLDERS,
    accounts: ACCOUNTS,
    ...input,
  });
}

/** Headings, each with its options' labels. */
const shown = (groups: SuggestionGroup[]) =>
  groups.map((g) => [g.heading, g.options.map((o) => o.label)]);

/** Picks the option labeled `label`. */
function pick(groups: SuggestionGroup[], label: string, text: string, caret = text.length) {
  const option = groups.flatMap((g) => g.options).find((o) => o.label === label)!;
  return option.apply(text, caret) as Applied;
}

describe("buildSuggestions", () => {
  it("offers recent searches and the filters in an empty box", () => {
    expect(shown(build(""))).toEqual([
      ["Filters", ["from:", "is:unread", "is:flagged", "has:attachment", "received:", "in:"]],
    ]);
    const recent = Array.from({ length: 10 }, (_, i) => `search ${i}`);
    const groups = build("  ", { recent });
    expect(shown(groups)[0]).toEqual([
      "Recent searches",
      [...recent.slice(0, 8), "Clear recent searches"],
    ]);
    expect(groups.flatMap((g) => g.options)).toHaveLength(MAX_SUGGESTIONS);
    expect(groups[0]!.options.at(-1)!.apply("", 0)).toBe("clearRecent");
    // A recent search replaces the text.
    expect(pick(groups, "search 3", "  ")).toEqual({
      text: "search 3",
      caret: 8,
      awaitsValue: false,
    });
    // A filter that completes a clause starts the search; one that awaits a value doesn't.
    expect(pick(groups, "is:unread", "")).toEqual({
      text: "is:unread ",
      caret: 10,
      awaitsValue: false,
    });
    expect(pick(groups, "from:", "")).toEqual({ text: "from:", caret: 5, awaitsValue: true });
  });

  it("offers recent searches, people and filters for a word", () => {
    const groups = build("lunch me", {
      recent: ["lunch meeting", "LUNCH ME", "Q3 memo", "atlas", "me", "meet", "memo"],
      people: PEOPLE,
    });
    expect(shown(groups)).toEqual([
      ["Recent searches", ["lunch meeting", "Q3 memo", "me"]],
      ["People", ["Megan Bowen", "megan.b@fabrikam.example"]],
    ]);
    const megan = groups[1]!.options[0]!;
    expect(megan.detail).toBe("megan@contoso.example");
    expect(megan.ariaLabel).toBe("From Megan Bowen, megan@contoso.example");
    expect(pick(groups, "Megan Bowen", "lunch me")).toEqual({
      text: "lunch from:megan@contoso.example ",
      caret: 33,
      awaitsValue: false,
    });
    // Operators whose name the word starts.
    expect(shown(build("i"))).toEqual([["Filters", ["is:unread", "is:read", "is:flagged", "in:"]]]);
    expect(pick(build("lunch i"), "in:", "lunch i")).toEqual({
      text: "lunch in:",
      caret: 9,
      awaitsValue: true,
    });
  });

  it("offers people for from:, to: and cc:", () => {
    const groups = build("x to:me", { people: PEOPLE });
    expect(shown(groups)).toEqual([["People", ["Megan Bowen", "megan.b@fabrikam.example"]]]);
    expect(groups[0]!.options[0]!.ariaLabel).toBe("To Megan Bowen, megan@contoso.example");
    expect(pick(groups, "megan.b@fabrikam.example", "x to:me")).toEqual({
      text: "x to:megan.b@fabrikam.example ",
      caret: 30,
      awaitsValue: false,
    });
    expect(peopleQuery("x from:", 7)).toBe("");
    expect(peopleQuery("x cc:meg", 8)).toBe("meg");
    expect(peopleQuery("lunch", 5)).toBe("lunch");
    expect(peopleQuery("lunch ", 6)).toBeNull();
    expect(peopleQuery("is:un", 5)).toBeNull();
    expect(peopleQuery('"lun', 4)).toBeNull();
  });

  it("offers is:, has: and the dates", () => {
    expect(shown(build("is:"))).toEqual([["Filters", ["is:unread", "is:read", "is:flagged"]]]);
    expect(shown(build("is:Un"))).toEqual([["Filters", ["is:unread"]]]);
    expect(shown(build("has:a"))).toEqual([["Filters", ["has:attachment"]]]);
    expect(shown(build("has:x"))).toEqual([]);
    expect(shown(build("received:this"))).toEqual([
      [
        "Dates",
        [
          'received:"this week"',
          'received:"this month"',
          'received:"this year"',
          "Or a date: YYYY-MM-DD",
        ],
      ],
    ]);
    const before = build("before:");
    expect(before[0]!.options).toHaveLength(8);
    expect(before[0]!.options.map((o) => o.detail)).toContain("Last month");
    const hint = build("received:")[0]!.options.at(-1)!;
    expect(hint).toMatchObject({ kind: "hint", disabled: true });
    expect(pick(build('x received:"last w'), 'received:"last week"', 'x received:"last w')).toEqual(
      { text: 'x received:"last week" ', caret: 23, awaitsValue: false },
    );
  });

  it("offers folders by role, then by name, written so they name the folder", () => {
    expect(shown(build("in:"))).toEqual([
      ["Folders", ["Inbox", "Sent", "Archive", "Project  X", "Q1\\Q2 plans"]],
    ]);
    expect(shown(build("in:pro"))).toEqual([["Folders", ["Project  X"]]]);
    expect(pick(build("in:pro"), "Project  X", "in:pro")).toEqual({
      text: 'in:"Project  X" ',
      caret: 16,
      awaitsValue: false,
    });
    expect(pick(build("in:q1"), "Q1\\Q2 plans", "in:q1").text).toBe('in:"Q1Q2 plans" ');
    expect(pick(build("in:sen"), "Sent", "in:sen").text).toBe("in:sent ");
  });

  it("offers the accounts", () => {
    expect(shown(build("account:home"))).toEqual([["Accounts", ["Alex (Personal)"]]]);
    expect(pick(build("account:"), "Alex Morgan", "account:").text).toBe(
      "account:alex@acme.example ",
    );
  });

  it("offers nothing for what has nothing to offer", () => {
    expect(build("subject:q3")).toEqual([]);
    expect(build('"quarterly re')).toEqual([]);
  });
});
