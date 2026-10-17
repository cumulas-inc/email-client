import { ContactRound, SearchX, UserPlus, Users } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

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
import { formatCount } from "../../lib/format";
import { FeaturePermissionNotice } from "../settings/FeaturePermission";
import { accountsWithout, noticeHeadingIds } from "../settings/microsoft";
import { AddressText, ContactDetails } from "./ContactDetails";
import { ContactEditor } from "./ContactEditor";
import { ContactList } from "./ContactList";
import { contactCount, contactName, countByAccount, groupContacts } from "./contactUtils";
import { useContacts, useDeleteContact } from "./queries";
import { usePeopleStore } from "./store";
import { useDebouncedValue } from "./useDebouncedValue";
import { AddressBooks } from "./AddressBooks";
import "./people.css";

type EditorState = { mode: "create" } | { mode: "edit"; contact: Contact };

/** The New contact button, which gets the keyboard after contacts are turned on. */
const NEW_CONTACT_BUTTON_ID = "people-new-contact";
/** The contacts permission notice, which explains a disabled New contact. */
const NOTICE_ID = "people-contacts-permission";

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
  // Every contact, for the address book counts. Unfiltered and unsearched,
  // this is the same query as the list's, so it costs nothing extra.
  const everyone = useContacts(null, "");
  const counts = useMemo(
    () => (everyone.data ? countByAccount(everyone.data) : null),
    [everyone.data],
  );
  const groups = useMemo(() => groupContacts(query.data ?? []), [query.data]);
  const ordered = useMemo(() => groups.flatMap((g) => g.contacts), [groups]);
  const selected = ordered.find((c) => c.id === selectedId) ?? ordered[0] ?? null;

  const [editor, setEditor] = useState<EditorState | null>(null);
  const [toDelete, setToDelete] = useState<Contact | null>(null);
  const remove = useDeleteContact();
  const listRef = useRef<HTMLDivElement>(null);
  const detailsRef = useRef<HTMLElement>(null);
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
      toast.success(`${contactName(contact)} was deleted`);
      refocus(listRef.current);
    } catch (e) {
      toast.error(e);
      refocus(returnFocus.current);
    } finally {
      setToDelete(null);
    }
  };

  // Each contact opens at the top of the details card, not wherever the
  // previous one was scrolled to.
  const selectedKey = selected?.id ?? null;
  useLayoutEffect(() => {
    if (detailsRef.current) detailsRef.current.scrollTop = 0;
  }, [selectedKey]);

  // Microsoft accounts whose contacts can't be used get none: not even local ones.
  const contactAccounts = useMemo(
    () =>
      accounts.filter(
        (a) =>
          a.kind === "imap" ||
          (!a.needsPermission.includes("contacts") && !a.refusedFeatures.includes("contacts")),
      ),
    [accounts],
  );
  const withoutContacts = accountsWithout(accounts, "contacts");
  const noticeHeading =
    withoutContacts.needing.length > 0
      ? noticeHeadingIds(NOTICE_ID).needing
      : withoutContacts.refused.length > 0
        ? noticeHeadingIds(NOTICE_ID).refused
        : undefined;

  // Ctrl+N starts a new contact, as it starts a message in Mail.
  const canCreate = contactAccounts.length > 0;
  useEffect(() => {
    if (!canCreate) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || document.querySelector("dialog[open]")) return;
      if (e.ctrlKey && !e.altKey && !e.metaKey && !e.shiftKey && e.key.toLowerCase() === "n") {
        e.preventDefault();
        returnFocus.current = document.activeElement as HTMLElement | null;
        setEditor({ mode: "create" });
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [canCreate]);

  const filterAccount = filterAccountId
    ? accounts.find((a) => a.id === filterAccountId)
    : undefined;

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
        title={filterAccount ? "No contacts in this account" : "No contacts yet"}
        description={
          filterAccount
            ? `${filterAccount.email} has no contacts yet.`
            : "Contacts you add, or sync from your accounts, show up here."
        }
        action={
          <span className="people__empty-actions">
            {canCreate && (
              <button
                type="button"
                className="btn btn--primary"
                onClick={() => openEditor({ mode: "create" })}
              >
                <UserPlus size={16} aria-hidden="true" />
                New contact
              </button>
            )}
            {filterAccount && (
              <button type="button" className="btn" onClick={() => setAccountId(null)}>
                Show all contacts
              </button>
            )}
          </span>
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
            id={NEW_CONTACT_BUTTON_ID}
            type="button"
            className="btn btn--primary"
            onClick={() => openEditor({ mode: "create" })}
            disabled={!canCreate}
            aria-describedby={canCreate ? undefined : noticeHeading}
            aria-keyshortcuts="Control+N"
          >
            <UserPlus size={18} strokeWidth={1.75} aria-hidden="true" />
            New contact
            <span className="kbd" aria-hidden="true">
              Ctrl N
            </span>
          </button>
        }
      >
        <AddressBooks
          accounts={accounts}
          counts={counts}
          selectedId={filterAccountId}
          onSelect={setAccountId}
        />
        <FeaturePermissionNotice
          feature="contacts"
          accounts={accounts}
          returnFocusId={NEW_CONTACT_BUTTON_ID}
          id={NOTICE_ID}
        />
      </ModuleSidebar>
      <section className="people__list-pane card" aria-label="Contact list">
        <div className="people__list-head">
          <SearchField placeholder="Search contacts" />
          <div className="people__title-row">
            <h1 className="pane-title people__title">
              Contacts
              {!query.isLoading && (
                <span
                  className="people__count"
                  title={ordered.length >= 10_000 ? contactCount(ordered.length) : undefined}
                >
                  {formatCount(ordered.length)}
                </span>
              )}
            </h1>
            {filterAccount && (
              <span
                className="people__chip"
                title={`${filterAccount.displayName} (${filterAccount.email})`}
              >
                <span
                  className="people__chip-dot"
                  style={{ background: filterAccount.color }}
                  aria-hidden="true"
                />
                <span className="people__chip-name">{filterAccount.displayName}</span>
              </span>
            )}
          </div>
          {debouncedSearch && ordered.length > 0 && (
            <p className="people__search-note" role="status">
              Results for “{debouncedSearch}”
            </p>
          )}
        </div>
        <div className="people__list-body">{list}</div>
      </section>

      <section ref={detailsRef} className="people__details-pane card" aria-label="Contact details">
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
            // Only shown while the list is empty, so it doesn't send anyone
            // to pick from it: the list's own empty state says what to do.
            <EmptyState
              icon={<ContactRound size={48} strokeWidth={1.25} />}
              title="No contact selected"
            />
          )
        )}
      </section>

      {editor && (
        <ContactEditor
          contact={editor.mode === "edit" ? editor.contact : null}
          // A contact being edited stays in its account; new ones go where contacts can.
          accounts={editor.mode === "edit" ? accounts : contactAccounts}
          defaultAccountId={
            filterAccountId && contactAccounts.some((a) => a.id === filterAccountId)
              ? filterAccountId
              : (contactAccounts[0]?.id ?? "")
          }
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
        confirmLabel="Delete contact"
        message={
          toDelete ? (
            <>
              <strong dir="auto">{contactName(toDelete)}</strong> will be permanently deleted from{" "}
              <span className="people-confirm__account">
                <AddressText
                  address={
                    accounts.find((a) => a.id === toDelete.accountId)?.email ?? "this account"
                  }
                />
                .
              </span>
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
