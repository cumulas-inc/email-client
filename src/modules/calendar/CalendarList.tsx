import { Check, ChevronDown } from "lucide-react";
import { useId, useState } from "react";

import type { Account, Calendar } from "../../api/types";
import { AccountBadge } from "../../components/AccountBadge";
import { tintStyle } from "./colors";
import { type CalendarGroup, groupCalendars } from "./model";

interface CalendarListProps {
  accounts: Account[];
  calendars: Calendar[];
  hidden: ReadonlySet<string>;
  onToggle: (calendarId: string, visible: boolean) => void;
}

function AccountGroup({
  group,
  hidden,
  onToggle,
}: {
  group: CalendarGroup;
  hidden: ReadonlySet<string>;
  onToggle: CalendarListProps["onToggle"];
}) {
  const [open, setOpen] = useState(true);
  const listId = useId();
  const name = group.account?.displayName ?? "Other calendars";
  return (
    <section className="cal-list__group">
      <button
        type="button"
        className="cal-list__account"
        aria-expanded={open}
        aria-controls={listId}
        title={open ? "Hide this account's calendars" : "Show this account's calendars"}
        onClick={() => setOpen((o) => !o)}
      >
        <AccountBadge
          name={name}
          color={group.account?.color ?? group.calendars[0]?.color ?? "#0b7a75"}
        />
        <span className="cal-list__account-text">
          <span className="cal-list__account-name">{name}</span>
          {group.account && <span className="cal-list__account-email">{group.account.email}</span>}
        </span>
        <ChevronDown size={14} strokeWidth={2} className="cal-list__chevron" aria-hidden="true" />
      </button>
      {open && (
        <ul id={listId} className="cal-list__items" aria-label={`${name} calendars`}>
          {group.calendars.map((calendar) => {
            const visible = !hidden.has(calendar.id);
            return (
              <li key={calendar.id}>
                <label className="cal-list__item cal-tint" style={tintStyle(calendar.color)}>
                  <input
                    type="checkbox"
                    className="cal-list__checkbox"
                    checked={visible}
                    aria-label={
                      group.account
                        ? `${calendar.name} (${group.account.displayName})`
                        : calendar.name
                    }
                    onChange={(e) => onToggle(calendar.id, e.target.checked)}
                  />
                  <span className="cal-list__swatch" aria-hidden="true">
                    <Check size={12} strokeWidth={3} />
                  </span>
                  <span className="cal-list__name">{calendar.name}</span>
                </label>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

export function CalendarList({ accounts, calendars, hidden, onToggle }: CalendarListProps) {
  const groups = groupCalendars(accounts, calendars);
  const headingId = useId();
  return (
    <nav className="cal-list" aria-labelledby={headingId}>
      <h2 id={headingId} className="cal-section-label">
        Calendars
      </h2>
      {groups.map((group) => (
        <AccountGroup key={group.accountId} group={group} hidden={hidden} onToggle={onToggle} />
      ))}
    </nav>
  );
}
