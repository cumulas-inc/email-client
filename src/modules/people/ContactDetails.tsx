import { Cake, Copy, Mail, MapPin, Pencil, Phone, StickyNote, Trash2 } from "lucide-react";
import { useId, type ReactNode, type Ref } from "react";

import type { Account, Contact, EmailAddress } from "../../api/types";
import { useAppStore } from "../../app/store";
import { Avatar } from "../../components/Avatar";
import { toast } from "../../components/toast";
import { contactSubtitle, formatBirthday, phoneKindLabel, primaryEmail } from "./contactUtils";

interface ContactDetailsProps {
  contact: Contact;
  account: Account | undefined;
  onEdit: () => void;
  onDelete: () => void;
  editRef?: Ref<HTMLButtonElement>;
}

function copyText(text: string, what: string) {
  const clipboard = typeof navigator !== "undefined" ? navigator.clipboard : undefined;
  if (!clipboard) {
    toast.error("Copying isn't available here. Select the text instead.");
    return;
  }
  clipboard.writeText(text).then(
    () => toast.success(`${what} copied`),
    (e: unknown) => toast.error(e),
  );
}

/** One label/value row. The icon names the kind of value at a glance. */
function Field({
  icon,
  label,
  children,
  copy,
}: {
  icon: ReactNode;
  label: string;
  children: ReactNode;
  /** A copy button at the end of the row. */
  copy?: ReactNode;
}) {
  return (
    <div className="people-field">
      <dt className="people-field__label">
        <span className="people-field__icon" aria-hidden="true">
          {icon}
        </span>
        {label}
      </dt>
      <dd className="people-field__value">
        {children}
        {copy}
      </dd>
    </div>
  );
}

function CopyButton({ label, onCopy }: { label: string; onCopy: () => void }) {
  return (
    <button
      type="button"
      className="icon-btn icon-btn--sm people-field__copy"
      aria-label={label}
      title={label}
      onClick={onCopy}
    >
      <Copy size={14} strokeWidth={2} />
    </button>
  );
}

const ICON = { size: 16, strokeWidth: 1.75 } as const;

export function ContactDetails({
  contact,
  account,
  onEdit,
  onDelete,
  editRef,
}: ContactDetailsProps) {
  const requestCompose = useAppStore((s) => s.requestCompose);
  const headingId = useId();
  const subtitle = contactSubtitle(contact);
  const primary = primaryEmail(contact);

  const compose = (to: EmailAddress) => requestCompose({ to: [to], accountId: contact.accountId });

  const hasInfo =
    contact.emails.length > 0 ||
    contact.phones.length > 0 ||
    Boolean(contact.address) ||
    Boolean(contact.birthday) ||
    Boolean(contact.notes);

  return (
    <article className="people-details" aria-labelledby={headingId}>
      <header className="people-details__header">
        <Avatar
          name={contact.displayName}
          colorKey={contact.emails[0]?.address ?? contact.displayName}
          size={72}
        />
        <div className="people-details__identity">
          <div className="selectable">
            <h2 id={headingId} className="people-details__name" title={contact.displayName}>
              {contact.displayName}
            </h2>
            {subtitle && (
              <p className="people-details__subtitle" title={subtitle}>
                {subtitle}
              </p>
            )}
          </div>
          {account && (
            <p className="people-details__account" title={account.displayName}>
              <span
                className="people-details__account-dot"
                style={{ background: account.color }}
                aria-hidden="true"
              />
              <span className="visually-hidden">Saved in </span>
              <span className="people-details__account-email">{account.email}</span>
            </p>
          )}
        </div>
      </header>

      <div className="people-actions" role="group" aria-label="Contact actions">
        <button
          type="button"
          className="people-actions__btn people-actions__btn--primary"
          disabled={!primary}
          title={primary ? `Write to ${primary.address}` : "This contact has no email address"}
          onClick={() => primary && compose(primary)}
        >
          <Mail {...ICON} aria-hidden="true" />
          Email
        </button>
        <button type="button" className="people-actions__btn" ref={editRef} onClick={onEdit}>
          <Pencil {...ICON} aria-hidden="true" />
          Edit
        </button>
        <button
          type="button"
          className="people-actions__btn people-actions__btn--danger"
          onClick={onDelete}
        >
          <Trash2 {...ICON} aria-hidden="true" />
          Delete
        </button>
      </div>

      <section className="people-info" aria-label="Contact information">
        {hasInfo ? (
          <dl className="people-fields">
            {contact.emails.map((email, i) => (
              <Field
                key={`e-${i}`}
                icon={<Mail {...ICON} />}
                label={contact.emails.length > 1 ? `Email ${i + 1}` : "Email"}
                copy={
                  <CopyButton
                    label={
                      contact.emails.length > 1
                        ? `Copy email address ${i + 1}`
                        : "Copy email address"
                    }
                    onCopy={() => copyText(email.address, "Email address")}
                  />
                }
              >
                <button
                  type="button"
                  className="people-link"
                  title={`Write to ${email.address}`}
                  onClick={() =>
                    compose({
                      name: email.name?.trim() || contact.displayName,
                      address: email.address,
                    })
                  }
                >
                  {email.address}
                </button>
              </Field>
            ))}
            {contact.phones.map((phone, i) => (
              <Field
                key={`p-${i}`}
                icon={<Phone {...ICON} />}
                label={phoneKindLabel(phone.kind)}
                copy={
                  <CopyButton
                    label={`Copy ${phoneKindLabel(phone.kind).toLowerCase()} number`}
                    onCopy={() => copyText(phone.number, "Phone number")}
                  />
                }
              >
                <span className="selectable people-field__text">{phone.number}</span>
              </Field>
            ))}
            {contact.address && (
              <Field icon={<MapPin {...ICON} />} label="Address">
                <span className="selectable people-field__text people-field__multiline">
                  {contact.address}
                </span>
              </Field>
            )}
            {contact.birthday && (
              <Field icon={<Cake {...ICON} />} label="Birthday">
                <span className="selectable people-field__text">
                  {formatBirthday(contact.birthday)}
                </span>
              </Field>
            )}
            {contact.notes && (
              <Field icon={<StickyNote {...ICON} />} label="Notes">
                <span className="selectable people-field__text people-field__multiline">
                  {contact.notes}
                </span>
              </Field>
            )}
          </dl>
        ) : (
          <div className="people-info__empty">
            <p>No email address, phone number or address yet.</p>
            <button type="button" className="btn btn--sm" onClick={onEdit}>
              <Pencil size={14} strokeWidth={2} aria-hidden="true" />
              Add details
            </button>
          </div>
        )}
      </section>
    </article>
  );
}
