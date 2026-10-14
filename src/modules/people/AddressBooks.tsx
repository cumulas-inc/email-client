import { Users } from "lucide-react";
import { useId } from "react";

import type { Account } from "../../api/types";
import { AccountBadge } from "../../components/AccountBadge";
import { contactCount } from "./contactUtils";

interface AddressBooksProps {
  accounts: Account[];
  /** Contacts per account, or null while they load. */
  counts: Map<string, number> | null;
  /** The account being shown, or null for every account. */
  selectedId: string | null;
  onSelect: (accountId: string | null) => void;
}

/**
 * The sidebar's address books: every contact, then one per account. The one
 * on show is marked with aria-current, like the current page of a site.
 */
export function AddressBooks({ accounts, counts, selectedId, onSelect }: AddressBooksProps) {
  const labelId = useId();
  const total = counts ? [...counts.values()].reduce((sum, n) => sum + n, 0) : null;
  const countOf = (accountId: string) => (counts ? (counts.get(accountId) ?? 0) : null);
  const named = (name: string, n: number | null) =>
    n === null ? name : `${name}, ${contactCount(n)}`;

  return (
    <div className="people-books">
      <h2 id={labelId} className="people-books__label">
        Address books
      </h2>
      <ul className="people-books__list" aria-labelledby={labelId}>
        <li>
          <button
            type="button"
            className="people-book people-book--all"
            aria-current={selectedId === null ? "true" : undefined}
            aria-label={named("All contacts", total)}
            onClick={() => onSelect(null)}
          >
            <span className="people-book__mark people-book__mark--all" aria-hidden="true">
              <Users size={14} strokeWidth={2} />
            </span>
            <span className="people-book__name">All contacts</span>
            {total !== null && <span className="people-book__count">{total}</span>}
          </button>
        </li>
        {accounts.map((account) => {
          const n = countOf(account.id);
          return (
            <li key={account.id}>
              <button
                type="button"
                className="people-book"
                aria-current={selectedId === account.id ? "true" : undefined}
                aria-label={named(`${account.displayName} (${account.email})`, n)}
                title={`${account.displayName} (${account.email})`}
                onClick={() => onSelect(account.id)}
              >
                <AccountBadge
                  className="people-book__mark"
                  name={account.displayName}
                  color={account.color}
                />
                <span className="people-book__name">{account.displayName}</span>
                <span className="people-book__email">{account.email}</span>
                {n !== null && <span className="people-book__count">{n}</span>}
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
