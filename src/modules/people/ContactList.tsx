import { useEffect, useId, useRef, type KeyboardEvent, type Ref } from "react";

import type { Contact } from "../../api/types";
import { Avatar } from "../../components/Avatar";
import { listboxTarget, scrollIdIntoView } from "../../components/listboxKeys";
import { type ContactGroup, contactSecondary } from "./contactUtils";

interface ContactListProps {
  groups: ContactGroup[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  /** Delete key on the selected contact. */
  onDelete: (contact: Contact) => void;
  listRef?: Ref<HTMLDivElement>;
}

/**
 * Contacts under letter headers. One tab stop; ↑/↓, Home/End, PageUp/Down
 * and typing a name move the selection, which the details pane follows.
 */
export function ContactList({ groups, selectedId, onSelect, onDelete, listRef }: ContactListProps) {
  const baseId = useId();
  const ordered = groups.flatMap((g) => g.contacts);
  const index = ordered.findIndex((c) => c.id === selectedId);
  const optionId = (id: string) => `${baseId}-option-${id}`;
  const activeId = index >= 0 && selectedId ? optionId(selectedId) : undefined;
  const typeahead = useRef({ text: "", at: 0 });

  useEffect(() => {
    scrollIdIntoView(activeId);
  }, [activeId]);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const target = listboxTarget(e.key, index, ordered.length);
    if (target !== null) {
      e.preventDefault();
      const contact = ordered[target];
      if (contact) onSelect(contact.id);
      return;
    }
    if (e.key === "Delete" && index >= 0) {
      e.preventDefault();
      onDelete(ordered[index]!);
      return;
    }
    if (e.key.length === 1 && e.key !== " " && !e.ctrlKey && !e.metaKey && !e.altKey) {
      // Type-ahead: jump to the next contact whose name starts with the typed text.
      const now = Date.now();
      const state = typeahead.current;
      state.text = now - state.at < 700 ? state.text + e.key.toLowerCase() : e.key.toLowerCase();
      state.at = now;
      const start = state.text.length === 1 ? index + 1 : Math.max(index, 0);
      for (let i = 0; i < ordered.length; i++) {
        const contact = ordered[(start + i) % ordered.length]!;
        if (contact.displayName.toLowerCase().startsWith(state.text)) {
          e.preventDefault();
          onSelect(contact.id);
          break;
        }
      }
    }
  };

  return (
    <div
      ref={listRef}
      role="listbox"
      aria-label="Contacts"
      aria-activedescendant={activeId}
      tabIndex={0}
      className="people-list"
      onKeyDown={onKeyDown}
    >
      {groups.map((group) => {
        const headerId = `${baseId}-letter-${group.letter}`;
        return (
          <div
            key={group.letter}
            role="group"
            aria-labelledby={headerId}
            className="people-list__group"
          >
            <div id={headerId} className="people-list__letter">
              {group.letter}
            </div>
            {group.contacts.map((contact) => {
              const secondary = contactSecondary(contact);
              return (
                <div
                  key={contact.id}
                  id={optionId(contact.id)}
                  role="option"
                  aria-selected={contact.id === selectedId}
                  aria-label={
                    secondary ? `${contact.displayName}, ${secondary}` : contact.displayName
                  }
                  className="people-row"
                  onClick={() => onSelect(contact.id)}
                >
                  <Avatar
                    name={contact.displayName}
                    colorKey={contact.emails[0]?.address ?? contact.displayName}
                    size={34}
                  />
                  <span className="people-row__text">
                    <span className="people-row__name">{contact.displayName}</span>
                    {secondary && <span className="people-row__secondary">{secondary}</span>}
                  </span>
                </div>
              );
            })}
          </div>
        );
      })}
    </div>
  );
}
