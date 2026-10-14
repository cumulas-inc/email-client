import { isValid, parseISO } from "date-fns";

import type { Contact, ContactDraft, EmailAddress, Phone } from "../../api/types";
import { formatDate } from "../../lib/datetime";

// --- List grouping -----------------------------------------------------------

/** Letter header for a name: A–Z (accents folded), "#" for anything else. */
export function letterFor(name: string): string {
  const ch = name.trim().normalize("NFD").replace(/[̀-ͯ]/g, "").charAt(0).toUpperCase();
  return /^[A-Z]$/.test(ch) ? ch : "#";
}

export interface ContactGroup {
  letter: string;
  contacts: Contact[];
}

const byName = (a: Contact, b: Contact) =>
  a.displayName.localeCompare(b.displayName, undefined, { sensitivity: "base" });

/** Contacts grouped under letter headers, A–Z then "#", each sorted by name. */
export function groupContacts(contacts: Contact[]): ContactGroup[] {
  const groups = new Map<string, Contact[]>();
  for (const c of contacts) {
    const letter = letterFor(c.displayName);
    const list = groups.get(letter);
    if (list) list.push(c);
    else groups.set(letter, [c]);
  }
  return [...groups.entries()]
    .sort(([a], [b]) => (a === "#" ? 1 : b === "#" ? -1 : a.localeCompare(b)))
    .map(([letter, list]) => ({ letter, contacts: list.sort(byName) }));
}

// --- Display -------------------------------------------------------------------

/** "Job title · Department · Company", skipping empty parts. */
export function contactSubtitle(c: Contact): string {
  return [c.jobTitle, c.department, c.company]
    .map((p) => p?.trim())
    .filter(Boolean)
    .join(" · ");
}

/**
 * Second line of a list row: the company, else the first email address.
 * A company that only repeats the name ("24/7 Emergency Plumbing") gives way
 * to the address too.
 */
export function contactSecondary(c: Contact): string {
  const company = c.company?.trim() ?? "";
  const repeatsName =
    company.localeCompare(c.displayName.trim(), undefined, { sensitivity: "base" }) === 0;
  return (repeatsName ? "" : company) || c.emails[0]?.address || company;
}

/** How many contacts each account holds. */
export function countByAccount(contacts: Contact[]): Map<string, number> {
  const counts = new Map<string, number>();
  for (const c of contacts) counts.set(c.accountId, (counts.get(c.accountId) ?? 0) + 1);
  return counts;
}

/** "1 contact", "16 contacts". */
export function contactCount(n: number): string {
  return n === 1 ? "1 contact" : `${n} contacts`;
}

/** The address the Email button writes to, with the contact's name. */
export function primaryEmail(c: Contact): EmailAddress | null {
  const first = c.emails[0];
  if (!first) return null;
  return { name: first.name?.trim() || c.displayName, address: first.address };
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
