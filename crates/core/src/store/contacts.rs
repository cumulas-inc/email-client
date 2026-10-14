use std::collections::HashSet;

use rusqlite::{OptionalExtension, Row, params};

use super::{Store, like_pattern, to_json};
use crate::error::{Error, Result};
use crate::model::Contact;

fn row_to_contact(row: &Row<'_>) -> rusqlite::Result<Contact> {
    let json: String = row.get(0)?;
    serde_json::from_str(&json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn search_text(c: &Contact) -> String {
    let mut parts = vec![c.display_name.clone()];
    parts.extend(c.given_name.clone());
    parts.extend(c.surname.clone());
    parts.extend(c.company.clone());
    for e in &c.emails {
        parts.push(e.address.clone());
    }
    parts.join(" ").to_lowercase()
}

fn upsert_contact_in(conn: &rusqlite::Connection, c: &Contact) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO contacts (id, account_id, remote_id, display_name, search_text, data_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET remote_id = excluded.remote_id,
           display_name = excluded.display_name, search_text = excluded.search_text,
           data_json = excluded.data_json",
        params![
            c.id,
            c.account_id,
            c.remote_id,
            c.display_name,
            search_text(c),
            to_json(c)
        ],
    )?;
    Ok(())
}

impl Store {
    pub fn replace_contacts(&self, account_id: &str, contacts: &[Contact]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let keep: HashSet<&str> = contacts.iter().map(|c| c.id.as_str()).collect();
        let existing: Vec<String> = {
            let mut stmt = tx.prepare("SELECT id FROM contacts WHERE account_id = ?1")?;
            stmt.query_map([account_id], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        for id in existing.iter().filter(|id| !keep.contains(id.as_str())) {
            tx.execute("DELETE FROM contacts WHERE id = ?1", [id])?;
        }
        for c in contacts {
            upsert_contact_in(&tx, c)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Contacts sorted by name, optionally filtered by account and/or a
    /// case-insensitive substring of name, company or email.
    pub fn list_contacts(
        &self,
        account_id: Option<&str>,
        text: Option<&str>,
    ) -> Result<Vec<Contact>> {
        let pattern = text
            .filter(|t| !t.trim().is_empty())
            .map(|t| like_pattern(&t.trim().to_lowercase()));
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT data_json FROM contacts
             WHERE (?1 IS NULL OR account_id = ?1)
               AND (?2 IS NULL OR search_text LIKE ?2 ESCAPE '\\')
             ORDER BY display_name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map(params![account_id, pattern], row_to_contact)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Like [`Store::list_contacts`] but with a ready-made LIKE pattern; used
    /// by address autocomplete.
    pub(super) fn search_contacts_raw(&self, pattern: &str, limit: u32) -> Result<Vec<Contact>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT data_json FROM contacts WHERE search_text LIKE ?1 ESCAPE '\\'
             ORDER BY display_name COLLATE NOCASE LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pattern, limit], row_to_contact)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get_contact(&self, id: &str) -> Result<Contact> {
        self.conn()
            .query_row(
                "SELECT data_json FROM contacts WHERE id = ?1",
                [id],
                row_to_contact,
            )
            .optional()?
            .ok_or_else(|| Error::not_found("contact"))
    }

    pub fn upsert_contact(&self, contact: &Contact) -> Result<()> {
        upsert_contact_in(&self.conn(), contact)?;
        Ok(())
    }

    pub fn delete_contact(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM contacts WHERE id = ?1", [id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EmailAddress, EntityKind, local_id};
    use crate::store::test_support::imap_account;

    fn contact(name: &str, email: &str) -> Contact {
        Contact {
            id: local_id("a", EntityKind::Contact, email),
            account_id: "a".into(),
            remote_id: email.into(),
            display_name: name.into(),
            given_name: None,
            surname: None,
            emails: vec![EmailAddress::new(None, email)],
            phones: vec![],
            company: Some("Analytical Engines Ltd".into()),
            job_title: None,
            department: None,
            address: None,
            birthday: None,
            notes: None,
        }
    }

    #[test]
    fn contacts_search_and_autocomplete() {
        let store = Store::open_in_memory().unwrap();
        store.insert_account(&imap_account("a")).unwrap();
        store
            .replace_contacts(
                "a",
                &[
                    contact("Ada Lovelace", "ada@example.com"),
                    contact("Charles Babbage", "charles@example.com"),
                ],
            )
            .unwrap();
        assert_eq!(store.list_contacts(None, None).unwrap().len(), 2);
        assert_eq!(
            store.list_contacts(Some("a"), Some("BABB")).unwrap().len(),
            1
        );
        assert_eq!(store.list_contacts(None, Some("engines")).unwrap().len(), 2);

        let suggestions = store.suggest_addresses("ada", 5).unwrap();
        assert_eq!(suggestions[0].name.as_deref(), Some("Ada Lovelace"));

        store
            .replace_contacts("a", &[contact("Ada Lovelace", "ada@example.com")])
            .unwrap();
        assert_eq!(store.list_contacts(None, None).unwrap().len(), 1);
    }
}
