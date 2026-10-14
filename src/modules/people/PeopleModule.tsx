import { ContactRound, SearchX, UserPlus, Users } from "lucide-react";
import { useMemo, useRef, useState } from "react";

import { errorMessage } from "../../api/errors";
import { useAccounts } from "../../api/queries";
import type { Contact } from "../../api/types";
import { ModuleSidebar } from "../../app/ModuleSidebar";
import { useAppStore } from "../../app/store";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { EmptyState } from "../../components/EmptyState";
import { SearchField } from "../../components/SearchField";
import { PaneSpinner } from "../../components/Spinner";
import { toast } from "../../components/toast";
import { ContactDetails } from "./ContactDetails";
import { ContactEditor } from "./ContactEditor";
import { ContactList } from "./ContactList";
import { groupContacts } from "./contactUtils";
import { useContacts, useDeleteContact } from "./queries";
import { usePeopleStore } from "./store";
import { useDebouncedValue } from "./useDebouncedValue";
import "./people.css";

type EditorState = { mode: "create" } | { mode: "edit"; contact: Contact };

/** Puts focus back on an element once a dialog has gone. */
function refocus(el: HTMLElement | null | undefined) {
  if (el) setTimeout(() => el.isConnected && el.focus(), 0);
}

export function PeopleModule() {
  const { data: accounts = [] } = useAccounts();
  const { selectedId, accountId, select, setAccountId } = usePeopleStore();
  const search = useAppStore((s) => (s.module === "people" ? s.search : ""));
  const setSearch = useAppStore((s) => s.setSearch);
  const debouncedSearch = useDebouncedValue(search.trim(), 200);

  // Ignore a remembered filter for an account that has since been removed.
  const filterAccountId = accountId && accounts.some((a) => a.id === accountId) ? accountId : null;
  const query = useContacts(filterAccountId, debouncedSearch);
  const groups = useMemo(() => groupContacts(query.data ?? []), [query.data]);
  const ordered = useMemo(() => groups.flatMap((g) => g.contacts), [groups]);
  const selected = ordered.find((c) => c.id === selectedId) ?? ordered[0] ?? null;

  const [editor, setEditor] = useState<EditorState | null>(null);
  const [toDelete, setToDelete] = useState<Contact | null>(null);
  const remove = useDeleteContact();
  const listRef = useRef<HTMLDivElement>(null);
  const editRef = useRef<HTMLButtonElement>(null);
  const returnFocus = useRef<HTMLElement | null>(null);

  const rememberFocus = () => {
    returnFocus.current = document.activeElement as HTMLElement | null;
  };

  const openEditor = (state: EditorState) => {
    rememberFocus();
    setEditor(state);
  };

  const askDelete = (contact: Contact) => {
    rememberFocus();
    select(contact.id);
    setToDelete(contact);
  };

  const onSaved = (contact: Contact) => {
    const created = editor?.mode === "create";
    setEditor(null);
    // Make sure the saved contact is visible, then select it.
    if (filterAccountId && filterAccountId !== contact.accountId) setAccountId(null);
    if (created && search) setSearch("");
    select(contact.id);
    refocus(created ? listRef.current : (returnFocus.current ?? editRef.current));
  };

  const confirmDelete = async () => {
    const contact = toDelete;
    if (!contact) return;
    const index = ordered.findIndex((c) => c.id === contact.id);
    const next = ordered[index + 1] ?? ordered[index - 1] ?? null;
    try {
      await remove.mutateAsync(contact.id);
      select(next?.id ?? null);
      toast.success(`${contact.displayName} was deleted`);
      refocus(listRef.current);
    } catch (e) {
      toast.error(e);
      refocus(returnFocus.current);
    } finally {
      setToDelete(null);
    }
  };

  const filterLabel = filterAccountId
    ? (accounts.find((a) => a.id === filterAccountId)?.email ?? "this account")
    : null;

  let list;
  if (query.isLoading) {
    list = <PaneSpinner />;
  } else if (query.isError) {
    list = (
      <EmptyState
        title="Couldn't load contacts"
        description={errorMessage(query.error)}
        action={
          <button type="button" className="btn" onClick={() => void query.refetch()}>
            Try again
          </button>
        }
      />
    );
  } else if (ordered.length === 0) {
    list = debouncedSearch ? (
      <EmptyState
        icon={<SearchX size={40} strokeWidth={1.5} />}
        title="No matching contacts"
        description={`Nobody matches “${debouncedSearch}”. Try a name, company or email address.`}
        action={
          <button type="button" className="btn" onClick={() => setSearch("")}>
            Clear search
          </button>
        }
      />
    ) : (
      <EmptyState
        icon={<Users size={40} strokeWidth={1.5} />}
        title={filterLabel ? "No contacts in this account" : "No contacts yet"}
        description={
          filterLabel
            ? `${filterLabel} has no contacts. Add one, or show all accounts.`
            : "People you add, or sync from your accounts, show up here."
        }
        action={
          <button
            type="button"
            className="btn btn--primary"
            onClick={() => openEditor({ mode: "create" })}
          >
            <UserPlus size={16} aria-hidden="true" />
            New contact
          </button>
        }
      />
    );
  } else {
    list = (
      <ContactList
        groups={groups}
        selectedId={selected?.id ?? null}
        onSelect={select}
        onDelete={askDelete}
        listRef={listRef}
      />
    );
  }

  return (
    <div className="people module-layout">
      <ModuleSidebar
        label="Address books"
        className="people__sidebar"
        action={
          <button
            type="button"
            className="btn btn--primary"
            onClick={() => openEditor({ mode: "create" })}
            disabled={accounts.length === 0}
          >
            <UserPlus size={16} aria-hidden="true" />
            New contact
          </button>
        }
      >
        {null}
      </ModuleSidebar>
      <section className="people__list-pane card" aria-label="Contact list">
        <div className="people__search">
          <SearchField placeholder="Search contacts" />
        </div>
        <div className="people__list-header">
          <h1 className="people__title">
            Contacts
            {!query.isLoading && <span className="people__count">{ordered.length}</span>}
          </h1>
          <select
            className="select people__account-filter"
            aria-label="Show contacts from"
            value={filterAccountId ?? ""}
            onChange={(e) => setAccountId(e.target.value || null)}
          >
            <option value="">All accounts</option>
            {accounts.map((a) => (
              <option key={a.id} value={a.id}>
                {a.email}
              </option>
            ))}
          </select>
        </div>
        {debouncedSearch && ordered.length > 0 && (
          <p className="people__search-note" role="status">
            Results for “{debouncedSearch}”
          </p>
        )}
        <div className="people__list-body">{list}</div>
      </section>

      <section className="people__details-pane card" aria-label="Contact details">
        {selected ? (
          <ContactDetails
            key={selected.id}
            contact={selected}
            account={accounts.find((a) => a.id === selected.accountId)}
            onEdit={() => openEditor({ mode: "edit", contact: selected })}
            onDelete={() => askDelete(selected)}
            editRef={editRef}
          />
        ) : (
          !query.isLoading && (
            <EmptyState
              icon={<ContactRound size={48} strokeWidth={1.25} />}
              title="No contact selected"
              description="Pick someone from the list to see their details."
            />
          )
        )}
      </section>

      {editor && (
        <ContactEditor
          contact={editor.mode === "edit" ? editor.contact : null}
          accounts={accounts}
          defaultAccountId={filterAccountId ?? accounts[0]?.id ?? ""}
          onClose={() => {
            setEditor(null);
            refocus(returnFocus.current);
          }}
          onSaved={onSaved}
        />
      )}

      <ConfirmDialog
        open={toDelete !== null}
        title="Delete contact?"
        message={
          toDelete ? (
            <>
              <strong>{toDelete.displayName}</strong> will be permanently deleted from{" "}
              {accounts.find((a) => a.id === toDelete.accountId)?.email ?? "this account"}.
            </>
          ) : null
        }
        busy={remove.isPending}
        onConfirm={() => void confirmDelete()}
        onCancel={() => {
          setToDelete(null);
          refocus(returnFocus.current);
        }}
      />
    </div>
  );
}
