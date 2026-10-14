import { describe, expect, it } from "vitest";

import type { Contact } from "../../api/types";
import {
  contactCount,
  contactName,
  contactSecondary,
  contactSubtitle,
  groupContacts,
  letterFor,
  nameStartsWith,
} from "./contactUtils";

let seq = 0;
function contact(displayName: string, extra: Partial<Contact> = {}): Contact {
  seq += 1;
  return {
    id: `c-${seq}`,
    accountId: "acct",
    remoteId: `r-${seq}`,
    displayName,
    givenName: null,
    surname: null,
    emails: [],
    phones: [],
    company: null,
    jobTitle: null,
    department: null,
    address: null,
    birthday: null,
    notes: null,
    ...extra,
  };
}

/** "A: Ann, Ærø · B: Bo" for compact assertions. */
function layout(names: string[], locale: string): string {
  return groupContacts(
    names.map((n) => contact(n)),
    locale,
  )
    .map((g) => `${g.letter}: ${g.contacts.map((c) => c.displayName).join(", ")}`)
    .join(" · ");
}

describe("letterFor", () => {
  it("folds letters English reads as plain A–Z", () => {
    const letters = ["Łukasz", "Øyvind", "Ærøskøbing", "Œuvre", "Đặng", "Émile", "Zoë", "ßtraße"];
    expect(letters.map((n) => letterFor(n, "en-US"))).toEqual([
      "L",
      "O",
      "A",
      "O",
      "D",
      "E",
      "Z",
      "S",
    ]);
  });

  it("keeps the letters a locale's alphabet adds", () => {
    expect(letterFor("Łukasz", "pl")).toBe("Ł");
    expect(letterFor("Ślązak", "pl")).toBe("Ś");
    expect(letterFor("Øyvind", "da")).toBe("Ø");
    expect(letterFor("Æsa", "da")).toBe("Æ");
    expect(letterFor("Åsa", "sv")).toBe("Å");
    expect(letterFor("Ñandú", "es")).toBe("Ñ");
    expect(letterFor("ilker", "tr")).toBe("İ");
    // German reads Ä as A, Swedish doesn't.
    expect(letterFor("Ärla", "de")).toBe("A");
    expect(letterFor("Ärla", "sv")).toBe("Ä");
  });

  it("uses each script's own letters", () => {
    const names: [string, string][] = [
      ["Александра", "А"],
      ["Йосиф", "Й"],
      ["Ёлка", "Е"],
      ["Γιώργος", "Γ"],
      ["Άννα", "Α"],
      // Georgian has a capital form, but names don't use it.
      ["ნინო", "ნ"],
      ["נועה", "נ"],
      ["ليلى", "ل"],
      // Hamza is not a letter of its own.
      ["أحمد", "ا"],
      ["प्रिया", "प"],
      ["สมชาย", "ส"],
      // Thai writes this vowel before the consonant it follows.
      ["เอก", "อ"],
    ];
    expect(names.map(([n]) => letterFor(n, "en-US"))).toEqual(names.map(([, l]) => l));
  });

  it("files Korean by first consonant, kana by row and Chinese characters by themselves", () => {
    expect(letterFor("김민준", "en-US")).toBe("ㄱ");
    expect(letterFor("깊은", "en-US")).toBe("ㄱ");
    expect(letterFor("박지성", "en-US")).toBe("ㅂ");
    expect(letterFor("やまだ", "en-US")).toBe("や");
    expect(letterFor("ガイ", "en-US")).toBe("か");
    // Half-width katakana from old systems.
    expect(letterFor("ｶﾌﾞｼｷｶﾞｲｼｬ", "en-US")).toBe("か");
    expect(letterFor("山田 花子", "en-US")).toBe("山");
  });

  it("looks past wrapping punctuation but not past symbols", () => {
    const names: [string, string][] = [
      ['"Bart" Fitzgerald', "B"],
      ["'Nana' Okafor", "N"],
      ["(unknown caller)", "U"],
      ["¡Hola! Café Barcelona", "H"],
      ["「山田」", "山"],
      ["@home Wi-Fi support", "#"],
      ["#ProcurementHelp", "#"],
      ["_old import", "#"],
      ["+48 601 234 567", "#"],
      ["24/7 Locksmith", "#"],
      ["🦄✨🌈", "#"],
      ["", "#"],
    ];
    expect(names.map(([n]) => letterFor(n, "en-US"))).toEqual(names.map(([, l]) => l));
  });
});

describe("groupContacts", () => {
  it("puts letters in the locale's order, other scripts after Latin, and # last", () => {
    expect(
      layout(
        ["24/7 Locksmith", "Дмитрий", "Zoë", "Łukasz", "ليلى", "Aisha", "山田", "Γιώργος", "Lucía"],
        "en-US",
      ),
    ).toBe(
      "A: Aisha · L: Lucía, Łukasz · Z: Zoë · Γ: Γιώργος · Д: Дмитрий · ل: ليلى · 山: 山田 · #: 24/7 Locksmith",
    );
  });

  it("follows the locale's alphabet", () => {
    expect(layout(["Lucía", "Łukasz", "Marek"], "pl")).toBe("L: Lucía · Ł: Łukasz · M: Marek");
    expect(layout(["Åsa", "Øyvind", "Æsa", "Anders", "Zeki"], "da")).toBe(
      "A: Anders · Z: Zeki · Æ: Æsa · Ø: Øyvind · Å: Åsa",
    );
    // Russian lists its own alphabet first.
    expect(layout(["Anna", "Дмитрий"], "ru")).toBe("Д: Дмитрий · A: Anna");
  });

  it("merges forms the locale counts as one letter, named by the plain one", () => {
    // Danish treats Ä as a form of Æ, and Ö as a form of Ø.
    expect(layout(["Ärla", "Ærø", "Ödön", "Øyvind"], "da")).toBe("Æ: Ärla, Ærø · Ø: Ödön, Øyvind");
  });

  it("sorts names by their letters, numbers by value", () => {
    expect(layout(['"Bart" Fitzgerald', "Bartholomew", "Barbara"], "en-US")).toBe(
      'B: Barbara, "Bart" Fitzgerald, Bartholomew',
    );
    expect(layout(["Apartment 10", "Apartment 2"], "en-US")).toBe("A: Apartment 2, Apartment 10");
    expect(layout(["24/7 Locksmith", "3rd floor", "007 Courier"], "en-US")).toBe(
      "#: 3rd floor, 007 Courier, 24/7 Locksmith",
    );
  });

  it("files a contact without a name by what it goes by", () => {
    const groups = groupContacts([contact("", { company: "Contoso Reception" })], "en-US");
    expect(groups.map((g) => g.letter)).toEqual(["C"]);
  });
});

describe("nameStartsWith", () => {
  it("matches typed text the way the list files names", () => {
    expect(nameStartsWith("Łukasz Brzęczyszczykiewicz", "l", "en-US")).toBe(true);
    expect(nameStartsWith("Łukasz Brzęczyszczykiewicz", "luk", "en-US")).toBe(true);
    expect(nameStartsWith("Łukasz Brzęczyszczykiewicz", "l", "pl")).toBe(false);
    expect(nameStartsWith('"Bart" Fitzgerald', "bart", "en-US")).toBe(true);
    // One letter finds the names filed under it.
    expect(nameStartsWith("Æsa Al-Rashid", "a", "en-US")).toBe(true);
    expect(nameStartsWith("Zoë Adeyemi", "zoe", "en-US")).toBe(true);
    expect(nameStartsWith("Zoë Adeyemi", "zz", "en-US")).toBe(false);
  });
});

describe("contact display", () => {
  it("shows a second line that adds something to the name", () => {
    const address = "bartholomew.fitzgerald@northwind-industries-holdings.example";
    // A contact named after its own address shows the name alone.
    expect(contactSecondary(contact(address, { emails: [{ name: null, address }] }))).toBe("");
    // A company that repeats the name gives way to the address.
    expect(
      contactSecondary(
        contact("24/7 Emergency Plumbing", {
          company: "24/7 Emergency Plumbing",
          emails: [{ name: null, address: "help@247plumbing.example" }],
        }),
      ),
    ).toBe("help@247plumbing.example");
    // With no company or address, the phone number.
    expect(
      contactSecondary(
        contact("Mom ❤️", { phones: [{ kind: "home", number: "+48 12 345 67 89" }] }),
      ),
    ).toBe("+48 12 345 67 89");
    // A contact named after its number shows nothing more.
    expect(
      contactSecondary(
        contact("+48 601 234 567", { phones: [{ kind: "mobile", number: "+48 601 234 567" }] }),
      ),
    ).toBe("");
  });

  it("names contacts that arrive without a name", () => {
    expect(contactName(contact("  Priya Raman "))).toBe("Priya Raman");
    expect(contactName(contact("", { company: "Contoso Reception" }))).toBe("Contoso Reception");
    expect(contactName(contact("", { phones: [{ kind: "mobile", number: "+1 555 0100" }] }))).toBe(
      "+1 555 0100",
    );
    expect(contactName(contact(""))).toBe("Unnamed contact");
    // The subtitle doesn't repeat the company it is named after.
    expect(
      contactSubtitle(contact("", { company: "Contoso Reception", jobTitle: "Front desk" })),
    ).toBe("Front desk");
  });

  it("counts with the locale's digit grouping", () => {
    expect(contactCount(1, "en-US")).toBe("1 contact");
    expect(contactCount(1204, "en-US")).toBe("1,204 contacts");
    expect(contactCount(0, "en-US")).toBe("0 contacts");
  });
});
