import { CircleAlert, Plus, X } from "lucide-react";
import { useId, useState, type FormEvent } from "react";

import { errorMessage } from "../../api/errors";
import type { Account, Contact } from "../../api/types";
import { Dialog } from "../../components/Dialog";
import { toast } from "../../components/toast";
import {
  type ContactForm,
  PHONE_KINDS,
  type PhoneRow,
  composeName,
  draftFromForm,
  emptyForm,
  formFromContact,
  hasErrors,
  validateForm,
} from "./contactUtils";
import { useCreateContact, useUpdateContact } from "./queries";

interface ContactEditorProps {
  /** The contact to edit, or null to create one. */
  contact: Contact | null;
  accounts: Account[];
  defaultAccountId: string;
  onClose: () => void;
  onSaved: (contact: Contact) => void;
}

/** Marks an element as the one <dialog>.showModal() focuses first. */
function autofocus(el: HTMLElement | null) {
  el?.setAttribute("autofocus", "");
}

/** Focuses an element after React has rendered it. */
function focusSoon(id: string) {
  setTimeout(() => document.getElementById(id)?.focus(), 0);
}

/** Create/edit dialog. Mount it only while open so it starts from fresh state. */
export function ContactEditor({
  contact,
  accounts,
  defaultAccountId,
  onClose,
  onSaved,
}: ContactEditorProps) {
  const id = useId();
  const fieldId = (name: string) => `${id}-${name}`;
  const [form, setForm] = useState<ContactForm>(() =>
    contact ? formFromContact(contact) : emptyForm(defaultAccountId),
  );
  const [submitted, setSubmitted] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const create = useCreateContact();
  const update = useUpdateContact();
  const saving = create.isPending || update.isPending;
  const errors = submitted ? validateForm(form) : { emails: {} as Record<number, string> };

  const set = <K extends keyof ContactForm>(key: K, value: ContactForm[K]) =>
    setForm((f) => ({ ...f, [key]: value }));

  const setName = (key: "givenName" | "surname", value: string) =>
    setForm((f) => {
      const next = { ...f, [key]: value };
      if (!f.displayNameEdited) next.displayName = composeName(next.givenName, next.surname);
      return next;
    });

  const setEmail = (index: number, address: string) =>
    set(
      "emails",
      form.emails.map((e, i) => (i === index ? { ...e, address } : e)),
    );

  const setPhone = (index: number, patch: Partial<PhoneRow>) =>
    set(
      "phones",
      form.phones.map((p, i) => (i === index ? { ...p, ...patch } : p)),
    );

  const onSubmit = async (e: FormEvent) => {
    e.preventDefault();
    if (saving) return;
    setSubmitted(true);
    setSubmitError(null);
    const found = validateForm(form);
    if (hasErrors(found)) {
      const firstEmail = Object.keys(found.emails).map(Number).sort()[0];
      focusSoon(found.displayName ? fieldId("display") : fieldId(`email-${firstEmail}`));
      return;
    }
    const draft = draftFromForm(form);
    try {
      const saved = contact
        ? await update.mutateAsync({ id: contact.id, draft })
        : await create.mutateAsync(draft);
      toast.success(contact ? "Contact saved" : `${saved.displayName} was added to your contacts`);
      onSaved(saved);
    } catch (err) {
      setSubmitError(errorMessage(err));
    }
  };

  const formId = fieldId("form");
  const account = accounts.find((a) => a.id === form.accountId);

  return (
    <Dialog
      open
      onClose={onClose}
      title={contact ? "Edit contact" : "New contact"}
      width={600}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" form={formId} className="btn btn--primary" disabled={saving}>
            {saving ? "Saving…" : "Save"}
          </button>
        </>
      }
    >
      <form id={formId} className="people-form" noValidate onSubmit={onSubmit}>
        {submitError && (
          <div className="people-form__alert" role="alert">
            <CircleAlert size={16} aria-hidden="true" />
            {submitError}
          </div>
        )}

        <div className="people-form__grid">
          {contact ? (
            account && (
              <p className="people-form__account people-form__full">
                Saved in <strong>{account.email}</strong>
              </p>
            )
          ) : (
            <div className="field people-form__full">
              <label className="field__label" htmlFor={fieldId("account")}>
                Save to account
              </label>
              <select
                id={fieldId("account")}
                className="select"
                value={form.accountId}
                onChange={(e) => set("accountId", e.target.value)}
              >
                {accounts.map((a) => (
                  <option key={a.id} value={a.id}>
                    {a.displayName} ({a.email})
                  </option>
                ))}
              </select>
            </div>
          )}

          <div className="field">
            <label className="field__label" htmlFor={fieldId("given")}>
              First name
            </label>
            <input
              id={fieldId("given")}
              ref={autofocus}
              className="input"
              autoComplete="off"
              value={form.givenName}
              onChange={(e) => setName("givenName", e.target.value)}
            />
          </div>
          <div className="field">
            <label className="field__label" htmlFor={fieldId("surname")}>
              Last name
            </label>
            <input
              id={fieldId("surname")}
              className="input"
              autoComplete="off"
              value={form.surname}
              onChange={(e) => setName("surname", e.target.value)}
            />
          </div>

          <div className="field people-form__full">
            <label className="field__label" htmlFor={fieldId("display")}>
              Display name
            </label>
            <input
              id={fieldId("display")}
              className="input"
              autoComplete="off"
              value={form.displayName}
              aria-invalid={errors.displayName ? true : undefined}
              aria-describedby={
                errors.displayName ? fieldId("display-error") : fieldId("display-hint")
              }
              onChange={(e) =>
                setForm((f) => ({
                  ...f,
                  displayName: e.target.value,
                  displayNameEdited: e.target.value.trim() !== "",
                }))
              }
            />
            {errors.displayName ? (
              <span id={fieldId("display-error")} className="field__error">
                {errors.displayName}
              </span>
            ) : (
              <span id={fieldId("display-hint")} className="field__hint">
                Filled in from the first and last name unless you change it.
              </span>
            )}
          </div>

          <fieldset className="people-form__group people-form__full">
            <legend className="field__label">Email</legend>
            {form.emails.map((email, i) => {
              const inputId = fieldId(`email-${i}`);
              const error = errors.emails[i];
              return (
                <div key={i} className="people-form__row-wrap">
                  <div className="people-form__row">
                    <input
                      id={inputId}
                      type="email"
                      className="input"
                      autoComplete="off"
                      placeholder="name@example.com"
                      aria-label={`Email address ${i + 1}`}
                      aria-invalid={error ? true : undefined}
                      aria-describedby={error ? `${inputId}-error` : undefined}
                      value={email.address}
                      onChange={(e) => setEmail(i, e.target.value)}
                    />
                    <button
                      type="button"
                      className="icon-btn"
                      aria-label={`Remove email address ${i + 1}`}
                      onClick={() =>
                        set(
                          "emails",
                          form.emails.filter((_, j) => j !== i),
                        )
                      }
                    >
                      <X size={16} />
                    </button>
                  </div>
                  {error && (
                    <span id={`${inputId}-error`} className="field__error">
                      {error}
                    </span>
                  )}
                </div>
              );
            })}
            <button
              type="button"
              className="btn btn--subtle btn--sm people-form__add"
              onClick={() => {
                set("emails", [...form.emails, { name: null, address: "" }]);
                focusSoon(fieldId(`email-${form.emails.length}`));
              }}
            >
              <Plus size={16} aria-hidden="true" />
              Add email address
            </button>
          </fieldset>

          <fieldset className="people-form__group people-form__full">
            <legend className="field__label">Phone</legend>
            {form.phones.map((phone, i) => (
              <div key={i} className="people-form__row">
                <select
                  className="select people-form__kind"
                  aria-label={`Phone ${i + 1} type`}
                  value={phone.kind}
                  onChange={(e) => setPhone(i, { kind: e.target.value as PhoneRow["kind"] })}
                >
                  {PHONE_KINDS.map((k) => (
                    <option key={k.value} value={k.value}>
                      {k.label}
                    </option>
                  ))}
                </select>
                <input
                  id={fieldId(`phone-${i}`)}
                  type="tel"
                  className="input"
                  autoComplete="off"
                  aria-label={`Phone number ${i + 1}`}
                  value={phone.number}
                  onChange={(e) => setPhone(i, { number: e.target.value })}
                />
                <button
                  type="button"
                  className="icon-btn"
                  aria-label={`Remove phone number ${i + 1}`}
                  onClick={() =>
                    set(
                      "phones",
                      form.phones.filter((_, j) => j !== i),
                    )
                  }
                >
                  <X size={16} />
                </button>
              </div>
            ))}
            <button
              type="button"
              className="btn btn--subtle btn--sm people-form__add"
              onClick={() => {
                set("phones", [
                  ...form.phones,
                  { kind: form.phones.length === 0 ? "mobile" : "business", number: "" },
                ]);
                focusSoon(fieldId(`phone-${form.phones.length}`));
              }}
            >
              <Plus size={16} aria-hidden="true" />
              Add phone number
            </button>
          </fieldset>

          <div className="field">
            <label className="field__label" htmlFor={fieldId("company")}>
              Company
            </label>
            <input
              id={fieldId("company")}
              className="input"
              autoComplete="off"
              value={form.company}
              onChange={(e) => set("company", e.target.value)}
            />
          </div>
          <div className="field">
            <label className="field__label" htmlFor={fieldId("job")}>
              Job title
            </label>
            <input
              id={fieldId("job")}
              className="input"
              autoComplete="off"
              value={form.jobTitle}
              onChange={(e) => set("jobTitle", e.target.value)}
            />
          </div>
          <div className="field">
            <label className="field__label" htmlFor={fieldId("department")}>
              Department
            </label>
            <input
              id={fieldId("department")}
              className="input"
              autoComplete="off"
              value={form.department}
              onChange={(e) => set("department", e.target.value)}
            />
          </div>
          <div className="field">
            <label className="field__label" htmlFor={fieldId("birthday")}>
              Birthday
            </label>
            <input
              id={fieldId("birthday")}
              type="date"
              className="input"
              value={form.birthday}
              onChange={(e) => set("birthday", e.target.value)}
            />
          </div>

          <div className="field people-form__full">
            <label className="field__label" htmlFor={fieldId("address")}>
              Address
            </label>
            <textarea
              id={fieldId("address")}
              className="textarea people-form__address"
              rows={3}
              value={form.address}
              onChange={(e) => set("address", e.target.value)}
            />
          </div>
          <div className="field people-form__full">
            <label className="field__label" htmlFor={fieldId("notes")}>
              Notes
            </label>
            <textarea
              id={fieldId("notes")}
              className="textarea"
              rows={3}
              value={form.notes}
              onChange={(e) => set("notes", e.target.value)}
            />
          </div>
        </div>
      </form>
    </Dialog>
  );
}
