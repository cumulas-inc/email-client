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

function Field({ icon, label, children }: { icon: ReactNode; label: string; children: ReactNode }) {
  return (
    <div className="people-field">
      <dt className="people-field__label">
        <span className="people-field__icon" aria-hidden="true">
          {icon}
        </span>
        {label}
      </dt>
      <dd className="people-field__value">{children}</dd>
    </div>
  );
}

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

  const hasContactInfo =
    contact.emails.length > 0 || contact.phones.length > 0 || Boolean(contact.address);

  return (
    <article className="people-details" aria-labelledby={headingId}>
      <header className="people-card people-details__header">
        <Avatar
          name={contact.displayName}
          colorKey={contact.emails[0]?.address ?? contact.displayName}
          size={80}
        />
        <div className="people-details__identity">
          <div className="selectable">
            <h2 id={headingId} className="people-details__name">
              {contact.displayName}
            </h2>
            {subtitle && <p className="people-details__subtitle">{subtitle}</p>}
            {account && (
              <p className="people-details__account">
                <span
                  className="people-details__account-dot"
                  style={{ background: account.color }}
                  aria-hidden="true"
                />
                <span className="visually-hidden">Saved in </span>
                {account.email}
              </p>
            )}
          </div>
          <div className="people-details__actions">
            <button
              type="button"
              className="btn btn--primary"
              disabled={!primary}
              title={primary ? `Send email to ${primary.address}` : "This contact has no email"}
              onClick={() => primary && compose(primary)}
            >
              <Mail size={16} aria-hidden="true" />
              Send email
            </button>
            <button type="button" className="btn" ref={editRef} onClick={onEdit}>
              <Pencil size={16} aria-hidden="true" />
              Edit
            </button>
            <button type="button" className="btn" onClick={onDelete}>
              <Trash2 size={16} aria-hidden="true" />
              Delete
            </button>
          </div>
        </div>
      </header>

      <section className="people-card" aria-label="Contact information">
        <h3 className="people-card__title">Contact information</h3>
        {hasContactInfo ? (
          <dl className="people-fields">
            {contact.emails.map((email, i) => (
              <Field
                key={`e-${i}`}
                icon={<Mail size={16} />}
                label={contact.emails.length > 1 ? `Email ${i + 1}` : "Email"}
              >
                <button
                  type="button"
                  className="people-link"
                  title={`Send email to ${email.address}`}
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
              <Field key={`p-${i}`} icon={<Phone size={16} />} label={phoneKindLabel(phone.kind)}>
                <span className="selectable people-field__text">{phone.number}</span>
                <button
                  type="button"
                  className="icon-btn icon-btn--sm people-field__copy"
                  aria-label={`Copy ${phoneKindLabel(phone.kind).toLowerCase()} number`}
                  title="Copy number"
                  onClick={() => copyText(phone.number, "Phone number")}
                >
                  <Copy size={14} />
                </button>
              </Field>
            ))}
            {contact.address && (
              <Field icon={<MapPin size={16} />} label="Address">
                <span className="selectable people-field__text people-field__multiline">
                  {contact.address}
                </span>
              </Field>
            )}
          </dl>
        ) : (
          <p className="people-card__empty">No email address, phone number or address yet.</p>
        )}
      </section>

      {contact.birthday && (
        <section className="people-card" aria-label="Personal">
          <h3 className="people-card__title">Personal</h3>
          <dl className="people-fields">
            <Field icon={<Cake size={16} />} label="Birthday">
              <span className="selectable people-field__text">
                {formatBirthday(contact.birthday)}
              </span>
            </Field>
          </dl>
        </section>
      )}

      {contact.notes && (
        <section className="people-card" aria-label="Notes">
          <h3 className="people-card__title">
            <StickyNote size={16} aria-hidden="true" />
            Notes
          </h3>
          <p className="selectable people-details__notes">{contact.notes}</p>
        </section>
      )}
    </article>
  );
}
