import { isValid, parseISO } from "date-fns";

import type { Contact, ContactDraft, EmailAddress, Phone } from "../../api/types";
import { appLocale, formatDate } from "../../lib/datetime";

// --- Collation -------------------------------------------------------------------

interface Collation {
  /** Full ordering, with "Apartment 2" before "Apartment 10". */
  sort: Intl.Collator;
  /** Letters only: ignores case and the accents the locale ignores. */
  base: Intl.Collator;
  /** Base letters, also ignoring spaces and punctuation, for "is this the same text?". */
  loose: Intl.Collator;
  /** Index letter per first character, worked out once. */
  letters: Map<string, string>;
}

const collations = new Map<string, Collation>();

function collation(locale: string): Collation {
  let c = collations.get(locale);
  if (!c) {
    c = {
      sort: new Intl.Collator(locale, { numeric: true }),
      base: new Intl.Collator(locale, { sensitivity: "base" }),
      loose: new Intl.Collator(locale, { sensitivity: "base", ignorePunctuation: true }),
      letters: new Map(),
    };
    collations.set(locale, c);
  }
  return c;
}

/** Whether two strings say the same thing, ignoring case, accents and punctuation. */
function sameText(a: string, b: string, locale: string): boolean {
  return collation(locale).loose.compare(a, b) === 0;
}

// --- List grouping -----------------------------------------------------------

/**
 * Quotes, brackets and inverted marks around a name aren't part of it, so
 * '"Bart" Fitzgerald' files under B and "¡Hola! Café" under H. Symbols that
 * start a name (@, #, _, +) are kept: those names go under "#".
 */
const WRAPPING = /^[\s\p{Ps}\p{Pe}\p{Pi}\p{Pf}"'¡¿]+/u;

/** A name as it sorts: without the punctuation wrapped around its start. */
export function sortName(name: string): string {
  const trimmed = name.trim();
  return trimmed.replace(WRAPPING, "") || trimmed;
}

const A_TO_Z = [..."ABCDEFGHIJKLMNOPQRSTUVWXYZ"];
const MARKS = /\p{M}/gu;
/** Scripts with upper and lower case. Georgian has a capital form that isn't used for names. */
const CASED = /[\p{Script=Latin}\p{Script=Greek}\p{Script=Cyrillic}\p{Script=Armenian}]/u;
const LATIN = /\p{Script=Latin}/u;
/** Hangul initial consonants, with the doubled ones (ㄲ) under their plain one (ㄱ). */
const CHOSEONG = [..."ㄱㄱㄴㄷㄷㄹㅁㅂㅂㅅㅅㅇㅈㅈㅊㅋㅌㅍㅎ"];
/** The kana rows (gojūon) Japanese address books are indexed by. */
const KANA_ROWS = [
  "あいうえおぁぃぅぇぉゔ",
  "かきくけこゕゖ",
  "さしすせそ",
  "たちつてとっ",
  "なにぬねの",
  "はひふへほ",
  "まみむめも",
  "やゆよゃゅょ",
  "らりるれろ",
  "わゐゑをんゎ",
];

/** The kana row a kana character belongs to: カ, ガ and か are all "か". */
function kanaRow(code: number): string | null {
  const hiragana = code >= 0x30a1 && code <= 0x30f6 ? code - 0x60 : code;
  if (hiragana < 0x3041 || hiragana > 0x3096) return null;
  const plain = String.fromCodePoint(hiragana).normalize("NFD").charAt(0);
  return KANA_ROWS.find((row) => row.includes(plain))?.charAt(0) ?? null;
}

/** The index letter for a name's first character, in the locale's terms. */
function indexLetter(ch: string, c: Collation, locale: string): string {
  const code = ch.codePointAt(0)!;
  // Korean address books file a name under its first consonant.
  if (code >= 0xac00 && code <= 0xd7a3) return CHOSEONG[Math.floor((code - 0xac00) / 588)]!;
  const kana = kanaRow(code);
  if (kana) return kana;
  // Hamza and vowel points aren't separate letters: أحمد files under ا.
  if (/[\p{Script=Arabic}\p{Script=Hebrew}]/u.test(ch))
    return ch.normalize("NFD").replace(MARKS, "") || ch;

  const upper = CASED.test(ch) ? [...ch.toLocaleUpperCase(locale)][0]! : ch;
  // Accents the locale ignores fold away: É under E, Ё under Е. Polish keeps Ś.
  const bare = upper.normalize("NFD").replace(MARKS, "");
  if (bare && bare !== upper && c.base.compare(bare, upper) === 0) return bare;
  if (LATIN.test(upper)) {
    // Letters the locale reads as a plain one: Ł as L and Ø as O in English,
    // but not in Polish or Danish, which give them their own place.
    const same = A_TO_Z.find((az) => c.base.compare(az, upper) === 0);
    if (same) return same;
    // Ligatures the locale reads as two letters: Æ as AE, Œ as OE.
    const before = A_TO_Z.findLast((az) => c.base.compare(az, upper) < 0);
    if (before && A_TO_Z.some((az) => c.base.compare(before + az, upper) === 0)) return before;
  }
  // Anything else heads its own group: Ñ in Spanish, Й, Γ, ნ, 山.
  return upper;
}

/** The first character a name is filed by, or "" when there is none. */
function firstCharacter(name: string): string {
  const chars = [...sortName(name.normalize("NFKC"))];
  // Thai and Lao write some vowels before the consonant they follow.
  if (/^[\u0e40-\u0e44\u0ec0-\u0ec4]$/u.test(chars[0] ?? "")) return chars[1] ?? "";
  return chars[0] ?? "";
}

/**
 * The letter header a name goes under, by the locale's alphabet: Łukasz
 * under L in English and under Ł in Polish, Æsa under A in English and
 * under Æ in Danish. Other alphabets use their own letters (Д, Γ, ع), Korean
 * its first consonant (ㄱ), kana its row (か) and Chinese characters
 * themselves. Names that start with a digit, symbol or emoji go under "#".
 */
export function letterFor(name: string, locale = appLocale()): string {
  const ch = firstCharacter(name);
  if (!/^\p{L}$/u.test(ch)) return "#";
  const c = collation(locale);
  let letter = c.letters.get(ch);
  if (letter === undefined) {
    letter = indexLetter(ch, c, locale);
    c.letters.set(ch, letter);
  }
  return letter;
}

/**
 * Whether a name starts with typed text, the way the list files it: "l"
 * finds Łukasz in English, and "b" finds '"Bart" Fitzgerald'. One letter also
 * finds the names under that letter's header, such as Æsa for "a".
 */
export function nameStartsWith(name: string, text: string, locale = appLocale()): boolean {
  const { base } = collation(locale);
  const typed = [...text];
  const start = [...sortName(name)].slice(0, typed.length).join("");
  if (base.compare(start, text) === 0) return true;
  return typed.length === 1 && base.compare(letterFor(name, locale), text) === 0;
}

export interface ContactGroup {
  letter: string;
  contacts: Contact[];
}

/** Contacts under letter headers in the locale's order, "#" last, each sorted by name. */
export function groupContacts(contacts: Contact[], locale = appLocale()): ContactGroup[] {
  const { sort, base } = collation(locale);
  const sorted = contacts
    .map((contact) => ({ contact, name: contactName(contact) }))
    .map((e) => ({ ...e, key: sortName(e.name), letter: letterFor(e.name, locale) }))
    .sort((a, b) => sort.compare(a.key, b.key) || sort.compare(a.name, b.name));

  const groups: ContactGroup[] = [];
  const byLetter = new Map<string, ContactGroup>();
  for (const { contact, letter } of sorted) {
    let group = byLetter.get(letter);
    if (!group) {
      // Two forms the locale counts as one letter share a header, named by
      // the plain one: Danish Æ rather than Ä.
      group = groups.find(
        (g) => letter !== "#" && g.letter !== "#" && base.compare(g.letter, letter) === 0,
      );
      if (!group) {
        group = { letter, contacts: [] };
        groups.push(group);
      } else if (letter.normalize("NFD").length < group.letter.normalize("NFD").length) {
        group.letter = letter;
      }
      byLetter.set(letter, group);
    }
    group.contacts.push(contact);
  }
  return groups.sort((a, b) =>
    a.letter === "#" ? 1 : b.letter === "#" ? -1 : sort.compare(a.letter, b.letter),
  );
}

// --- Display -------------------------------------------------------------------

/**
 * The name to show. Synced contacts can arrive without one (a company and a
 * phone number only); they go by the company, then the number.
 */
export function contactName(c: Contact): string {
  return (
    c.displayName.trim() ||
    c.company?.trim() ||
    c.emails[0]?.address ||
    c.phones[0]?.number.trim() ||
    "Unnamed contact"
  );
}

/** "Job title · Department · Company", skipping empty parts and the name itself. */
export function contactSubtitle(c: Contact, locale = appLocale()): string {
  const name = contactName(c);
  return [c.jobTitle, c.department, c.company]
    .map((p) => p?.trim() ?? "")
    .filter((p) => p && !sameText(p, name, locale))
    .join(" · ");
}

/**
 * Second line of a list row: the company, else the first email address,
 * else the first phone number. Anything that only repeats the name ("24/7
 * Emergency Plumbing", a contact named after its address) is skipped, and a
 * row with nothing left shows the name alone.
 */
export function contactSecondary(c: Contact, locale = appLocale()): string {
  const name = contactName(c);
  return (
    [c.company, c.emails[0]?.address, c.phones[0]?.number]
      .map((value) => value?.trim() ?? "")
      .find((value) => value && !sameText(value, name, locale)) ?? ""
  );
}

/** How many contacts each account holds. */
export function countByAccount(contacts: Contact[]): Map<string, number> {
  const counts = new Map<string, number>();
  for (const c of contacts) counts.set(c.accountId, (counts.get(c.accountId) ?? 0) + 1);
  return counts;
}

/** "1 contact", "1,204 contacts": the exact number, for labels. */
export function contactCount(n: number, locale = appLocale()): string {
  const count = new Intl.NumberFormat(locale).format(n);
  return n === 1 ? `${count} contact` : `${count} contacts`;
}

/** The address the Email button writes to, with the contact's name. */
export function primaryEmail(c: Contact): EmailAddress | null {
  const first = c.emails[0];
  if (!first) return null;
  return { name: first.name?.trim() || contactName(c), address: first.address };
}

export const PHONE_KINDS: { value: Phone["kind"]; label: string }[] = [
  { value: "mobile", label: "Mobile" },
  { value: "business", label: "Work" },
  { value: "home", label: "Home" },
  { value: "other", label: "Other" },
];

export function phoneKindLabel(kind: string): string {
  return PHONE_KINDS.find((k) => k.value === kind)?.label ?? "Phone";
}

/** "November 6, 1988" from "1988-11-06"; the raw text if it is not a date. */
export function formatBirthday(birthday: string): string {
  const date = parseISO(birthday);
  return isValid(date) ? formatDate(date, "longYear") : birthday;
}

// --- Editor form ------------------------------------------------------------------

export interface PhoneRow {
  kind: Phone["kind"];
  number: string;
}

export interface EmailRow {
  /** Display name stored with the address by the server, kept on edit. */
  name: string | null;
  address: string;
}

export interface ContactForm {
  accountId: string;
  givenName: string;
  surname: string;
  displayName: string;
  /** False while the display name simply follows first + last name. */
  displayNameEdited: boolean;
  emails: EmailRow[];
  phones: PhoneRow[];
  company: string;
  jobTitle: string;
  department: string;
  address: string;
  birthday: string;
  notes: string;
}

export function composeName(givenName: string, surname: string): string {
  return [givenName.trim(), surname.trim()].filter(Boolean).join(" ");
}

export function emptyForm(accountId: string): ContactForm {
  return {
    accountId,
    givenName: "",
    surname: "",
    displayName: "",
    displayNameEdited: false,
    emails: [{ name: null, address: "" }],
    phones: [],
    company: "",
    jobTitle: "",
    department: "",
    address: "",
    birthday: "",
    notes: "",
  };
}

export function formFromContact(c: Contact): ContactForm {
  const givenName = c.givenName ?? "";
  const surname = c.surname ?? "";
  return {
    accountId: c.accountId,
    givenName,
    surname,
    displayName: c.displayName,
    displayNameEdited: c.displayName !== composeName(givenName, surname),
    emails: c.emails.length > 0 ? c.emails.map((e) => ({ ...e })) : [{ name: null, address: "" }],
    phones: c.phones.map((p) => ({
      kind: PHONE_KINDS.some((k) => k.value === p.kind) ? p.kind : "other",
      number: p.number,
    })),
    company: c.company ?? "",
    jobTitle: c.jobTitle ?? "",
    department: c.department ?? "",
    address: c.address ?? "",
    birthday: c.birthday ?? "",
    notes: c.notes ?? "",
  };
}

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

export function isValidEmail(address: string): boolean {
  return EMAIL_RE.test(address.trim());
}

export interface FormErrors {
  displayName?: string;
  /** Error per email row index. */
  emails: Record<number, string>;
}

export function validateForm(form: ContactForm): FormErrors {
  const errors: FormErrors = { emails: {} };
  const name = form.displayName.trim() || composeName(form.givenName, form.surname);
  if (!name) errors.displayName = "Enter a name for the contact.";
  form.emails.forEach((e, i) => {
    if (e.address.trim() && !isValidEmail(e.address)) {
      errors.emails[i] = "Enter a valid email address, like name@example.com.";
    }
  });
  return errors;
}

export function hasErrors(errors: FormErrors): boolean {
  return Boolean(errors.displayName) || Object.keys(errors.emails).length > 0;
}

const orNull = (s: string) => s.trim() || null;

export function draftFromForm(form: ContactForm): ContactDraft {
  return {
    accountId: form.accountId,
    displayName: form.displayName.trim() || composeName(form.givenName, form.surname),
    givenName: orNull(form.givenName),
    surname: orNull(form.surname),
    emails: form.emails
      .filter((e) => e.address.trim())
      .map((e) => ({ name: e.name, address: e.address.trim() })),
    phones: form.phones
      .filter((p) => p.number.trim())
      .map((p) => ({ kind: p.kind, number: p.number.trim() })),
    company: orNull(form.company),
    jobTitle: orNull(form.jobTitle),
    department: orNull(form.department),
    address: orNull(form.address),
    birthday: form.birthday || null,
    notes: orNull(form.notes),
  };
}
