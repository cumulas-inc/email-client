use std::collections::{HashMap, HashSet};

use rusqlite::{OptionalExtension, Row, Transaction, params, params_from_iter};
use serde::{Deserialize, Serialize};

use super::{Store, from_json, from_ts, like_pattern, to_json, ts};
use crate::error::{Error, Result};
use crate::html::html_to_text;
use crate::model::{
    AttachmentMeta, EmailAddress, EntityKind, Folder, FolderRole, Importance, MessagePage,
    MessageQuery, MessageSummary, local_id,
};
use crate::providers::{MessageBatch, MessageContent};

const FOLDER_COLUMNS: &str =
    "id, account_id, remote_id, parent_id, name, role, unread_count, total_count";

const SUMMARY_COLUMNS: &str = "id, account_id, folder_id, remote_id, conversation_id, subject, \
     from_name, from_address, to_json, cc_json, preview, received_at, is_read, is_flagged, \
     is_draft, has_attachments, importance";

/// Change to a folder's unread count caused by a local operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderUnreadDelta {
    pub folder_id: String,
    pub delta: i64,
}

/// What applying a folder sync changed in the store.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BatchOutcome {
    /// Messages that were not stored before (candidates for new-mail
    /// notifications).
    pub inserted: Vec<MessageSummary>,
    /// Local ids of stored messages that were unread and are now read.
    pub became_read: Vec<String>,
    /// Local ids of stored messages that were read and are now unread.
    pub became_unread: Vec<String>,
    /// Local ids of stored messages that were removed from the folder.
    pub removed: Vec<String>,
    /// Ids of the folders that stored messages moved in from. Graph keeps a
    /// message's id when it moves, so the move shows as an update.
    pub moved_from: Vec<String>,
}

impl BatchOutcome {
    /// Whether the folder's unread or total count may have changed (and
    /// those of `moved_from`).
    pub fn changes_counts(&self) -> bool {
        !(self.inserted.is_empty()
            && self.became_read.is_empty()
            && self.became_unread.is_empty()
            && self.removed.is_empty()
            && self.moved_from.is_empty())
    }
}

/// Header fields that are only known once the full message has been fetched.
#[derive(Debug, Default, Serialize, Deserialize)]
struct StoredContent {
    bcc: Vec<EmailAddress>,
    reply_to: Vec<EmailAddress>,
    attachments: Vec<AttachmentMeta>,
    internet_message_id: Option<String>,
    in_reply_to: Option<String>,
    references: Vec<String>,
}

fn row_to_folder(row: &Row<'_>) -> rusqlite::Result<Folder> {
    Ok(Folder {
        id: row.get(0)?,
        account_id: row.get(1)?,
        remote_id: row.get(2)?,
        parent_id: row.get(3)?,
        name: row.get(4)?,
        role: FolderRole::parse(&row.get::<_, String>(5)?),
        unread_count: row.get(6)?,
        total_count: row.get(7)?,
    })
}

fn row_to_summary(row: &Row<'_>) -> rusqlite::Result<MessageSummary> {
    let from_name: Option<String> = row.get(6)?;
    let from_address: Option<String> = row.get(7)?;
    Ok(MessageSummary {
        id: row.get(0)?,
        account_id: row.get(1)?,
        folder_id: row.get(2)?,
        remote_id: row.get(3)?,
        conversation_id: row.get(4)?,
        subject: row.get(5)?,
        from: from_address.map(|address| EmailAddress {
            name: from_name,
            address,
        }),
        to: from_json(&row.get::<_, String>(8)?),
        cc: from_json(&row.get::<_, String>(9)?),
        preview: row.get(10)?,
        received_at: from_ts(row.get(11)?),
        is_read: row.get(12)?,
        is_flagged: row.get(13)?,
        is_draft: row.get(14)?,
        has_attachments: row.get(15)?,
        importance: Importance::parse(&row.get::<_, String>(16)?),
    })
}

fn format_addresses(addresses: &[EmailAddress]) -> String {
    addresses
        .iter()
        .map(|a| match &a.name {
            Some(n) => format!("{n} {}", a.address),
            None => a.address.clone(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn index_message(tx: &Transaction<'_>, m: &MessageSummary, body: &str) -> rusqlite::Result<()> {
    tx.execute("DELETE FROM messages_fts WHERE message_id = ?1", [&m.id])?;
    let sender = m
        .from
        .as_ref()
        .map(|f| format_addresses(std::slice::from_ref(f)))
        .unwrap_or_default();
    let recipients = format!("{} {}", format_addresses(&m.to), format_addresses(&m.cc));
    tx.execute(
        "INSERT INTO messages_fts (message_id, subject, sender, recipients, body)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![m.id, m.subject, sender, recipients, body],
    )?;
    Ok(())
}

/// Inserts or updates the summary columns of a message, leaving any fetched
/// body untouched. Returns the read state and folder stored before, `None`
/// for a new message.
fn upsert_summary(
    tx: &Transaction<'_>,
    m: &MessageSummary,
) -> rusqlite::Result<Option<(bool, String)>> {
    let previous = tx
        .query_row(
            "SELECT is_read, folder_id FROM messages WHERE id = ?1",
            [&m.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    tx.execute(
        &format!(
            "INSERT INTO messages ({SUMMARY_COLUMNS})
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
             ON CONFLICT(id) DO UPDATE SET
               folder_id = excluded.folder_id, remote_id = excluded.remote_id,
               conversation_id = excluded.conversation_id, subject = excluded.subject,
               from_name = excluded.from_name, from_address = excluded.from_address,
               to_json = excluded.to_json, cc_json = excluded.cc_json,
               preview = COALESCE(NULLIF(excluded.preview, ''), messages.preview),
               received_at = excluded.received_at, is_read = excluded.is_read,
               is_flagged = excluded.is_flagged, is_draft = excluded.is_draft,
               has_attachments = excluded.has_attachments, importance = excluded.importance"
        ),
        params![
            m.id,
            m.account_id,
            m.folder_id,
            m.remote_id,
            m.conversation_id,
            m.subject,
            m.from.as_ref().and_then(|f| f.name.clone()),
            m.from.as_ref().map(|f| f.address.clone()),
            to_json(&m.to),
            to_json(&m.cc),
            m.preview,
            ts(&m.received_at),
            m.is_read,
            m.is_flagged,
            m.is_draft,
            m.has_attachments,
            m.importance.as_str(),
        ],
    )?;
    if previous.is_none() {
        index_message(tx, m, &m.preview)?;
    }
    Ok(previous)
}

fn stored_read_state(tx: &Transaction<'_>, id: &str) -> rusqlite::Result<Option<bool>> {
    tx.query_row("SELECT is_read FROM messages WHERE id = ?1", [id], |r| {
        r.get(0)
    })
    .optional()
}

/// Deletes messages; returns the ids that were stored.
fn delete_message_rows(tx: &Transaction<'_>, ids: &[String]) -> rusqlite::Result<Vec<String>> {
    let mut deleted = Vec::new();
    for id in ids {
        tx.execute("DELETE FROM messages_fts WHERE message_id = ?1", [id])?;
        if tx.execute("DELETE FROM messages WHERE id = ?1", [id])? > 0 {
            deleted.push(id.clone());
        }
    }
    Ok(deleted)
}

/// Turns free text into an FTS5 query: every word becomes a quoted prefix
/// term, so user input can never be interpreted as FTS syntax.
fn fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split_whitespace()
        .map(|w| w.replace('"', ""))
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\"*"))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

impl Store {
    // --- folders ----------------------------------------------------------

    /// Replaces the folder list of an account. Folders that disappeared are
    /// deleted along with their messages; sync cursors of the others are kept.
    pub fn replace_folders(&self, account_id: &str, folders: &[Folder]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let keep: HashSet<&str> = folders.iter().map(|f| f.id.as_str()).collect();
        let existing: Vec<String> = {
            let mut stmt = tx.prepare("SELECT id FROM folders WHERE account_id = ?1")?;
            stmt.query_map([account_id], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        for id in existing.iter().filter(|id| !keep.contains(id.as_str())) {
            tx.execute(
                "DELETE FROM messages_fts WHERE message_id IN
                   (SELECT id FROM messages WHERE folder_id = ?1)",
                [id],
            )?;
            tx.execute("DELETE FROM folders WHERE id = ?1", [id])?;
        }
        for f in folders {
            tx.execute(
                &format!(
                    "INSERT INTO folders ({FOLDER_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT(id) DO UPDATE SET
                       remote_id = excluded.remote_id, parent_id = excluded.parent_id,
                       name = excluded.name, role = excluded.role,
                       unread_count = excluded.unread_count, total_count = excluded.total_count"
                ),
                params![
                    f.id,
                    account_id,
                    f.remote_id,
                    f.parent_id,
                    f.name,
                    f.role.as_str(),
                    f.unread_count,
                    f.total_count,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Folders of one account, or of all accounts, well-known folders first.
    pub fn list_folders(&self, account_id: Option<&str>) -> Result<Vec<Folder>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {FOLDER_COLUMNS} FROM folders WHERE ?1 IS NULL OR account_id = ?1"
        ))?;
        let mut folders: Vec<Folder> = stmt
            .query_map([account_id], row_to_folder)?
            .collect::<rusqlite::Result<_>>()?;
        folders.sort_by(|a, b| {
            (a.role.sort_rank(), a.name.to_lowercase())
                .cmp(&(b.role.sort_rank(), b.name.to_lowercase()))
        });
        Ok(folders)
    }

    pub fn get_folder(&self, id: &str) -> Result<Folder> {
        self.conn()
            .query_row(
                &format!("SELECT {FOLDER_COLUMNS} FROM folders WHERE id = ?1"),
                [id],
                row_to_folder,
            )
            .optional()?
            .ok_or_else(|| Error::not_found("folder"))
    }

    pub fn find_folder_by_role(
        &self,
        account_id: &str,
        role: FolderRole,
    ) -> Result<Option<Folder>> {
        Ok(self
            .conn()
            .query_row(
                &format!(
                    "SELECT {FOLDER_COLUMNS} FROM folders WHERE account_id = ?1 AND role = ?2
                     LIMIT 1"
                ),
                params![account_id, role.as_str()],
                row_to_folder,
            )
            .optional()?)
    }

    pub fn find_folder_by_remote_id(
        &self,
        account_id: &str,
        remote_id: &str,
    ) -> Result<Option<Folder>> {
        Ok(self
            .conn()
            .query_row(
                &format!(
                    "SELECT {FOLDER_COLUMNS} FROM folders WHERE account_id = ?1 AND remote_id = ?2
                     LIMIT 1"
                ),
                params![account_id, remote_id],
                row_to_folder,
            )
            .optional()?)
    }

    /// Stores a folder's counts as the server reported them.
    pub fn set_folder_counts(&self, folder_id: &str, unread: u32, total: u32) -> Result<()> {
        self.conn().execute(
            "UPDATE folders SET unread_count = ?2, total_count = ?3 WHERE id = ?1",
            params![folder_id, unread, total],
        )?;
        Ok(())
    }

    pub fn folder_sync_state(&self, folder_id: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT sync_state FROM folders WHERE id = ?1",
                [folder_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }

    pub fn set_folder_sync_state(&self, folder_id: &str, state: Option<&str>) -> Result<()> {
        self.conn().execute(
            "UPDATE folders SET sync_state = ?2 WHERE id = ?1",
            params![folder_id, state],
        )?;
        Ok(())
    }

    // --- messages ---------------------------------------------------------

    /// Applies the result of a folder sync and reports what changed.
    pub fn apply_message_batch(
        &self,
        folder: &Folder,
        batch: &MessageBatch,
    ) -> Result<BatchOutcome> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut outcome = BatchOutcome::default();
        if batch.full_resync {
            let keep: HashSet<&str> = batch.upserts.iter().map(|m| m.id.as_str()).collect();
            let existing: Vec<String> = {
                let mut stmt = tx.prepare("SELECT id FROM messages WHERE folder_id = ?1")?;
                stmt.query_map([&folder.id], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?
            };
            let stale: Vec<String> = existing
                .into_iter()
                .filter(|id| !keep.contains(id.as_str()))
                .collect();
            outcome.removed = delete_message_rows(&tx, &stale)?;
        }
        let mut removed: Vec<String> = batch
            .removed_remote_ids
            .iter()
            .map(|r| local_id(&folder.account_id, EntityKind::Message, r))
            .collect();
        if let Some(retain) = &batch.retain_only {
            let keep: HashSet<&str> = retain.iter().map(String::as_str).collect();
            let mut stmt = tx.prepare("SELECT id, remote_id FROM messages WHERE folder_id = ?1")?;
            let rows = stmt.query_map([&folder.id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (id, remote_id) = row?;
                if !keep.contains(remote_id.as_str()) {
                    removed.push(id);
                }
            }
        }
        removed.sort_unstable();
        removed.dedup();
        outcome.removed.extend(delete_message_rows(&tx, &removed)?);
        for m in &batch.upserts {
            let Some((was_read, was_in)) = upsert_summary(&tx, m)? else {
                outcome.inserted.push(m.clone());
                continue;
            };
            if was_in != m.folder_id && !outcome.moved_from.contains(&was_in) {
                outcome.moved_from.push(was_in);
            }
            match (was_read, m.is_read) {
                (false, true) => outcome.became_read.push(m.id.clone()),
                (true, false) => outcome.became_unread.push(m.id.clone()),
                _ => {}
            }
        }
        for update in &batch.flag_updates {
            let id = local_id(&folder.account_id, EntityKind::Message, &update.remote_id);
            let Some(was_read) = stored_read_state(&tx, &id)? else {
                continue;
            };
            tx.execute(
                "UPDATE messages SET is_read = ?2, is_flagged = ?3 WHERE id = ?1",
                params![id, update.is_read, update.is_flagged],
            )?;
            match (was_read, update.is_read) {
                (false, true) => outcome.became_read.push(id),
                (true, false) => outcome.became_unread.push(id),
                _ => {}
            }
        }
        tx.execute(
            "UPDATE folders SET sync_state = ?2 WHERE id = ?1",
            params![folder.id, batch.sync_state],
        )?;
        tx.commit()?;
        Ok(outcome)
    }

    pub fn list_messages(&self, folder_id: &str, query: &MessageQuery) -> Result<MessagePage> {
        let conn = self.conn();
        let filter = "folder_id = ?1 AND (?2 = 0 OR is_read = 0) AND (?3 = 0 OR is_flagged = 1)";
        let total: u32 = conn.query_row(
            &format!("SELECT COUNT(*) FROM messages WHERE {filter}"),
            params![folder_id, query.unread_only, query.flagged_only],
            |r| r.get(0),
        )?;
        let limit = if query.limit == 0 { 100 } else { query.limit };
        let mut stmt = conn.prepare(&format!(
            "SELECT {SUMMARY_COLUMNS} FROM messages WHERE {filter}
             ORDER BY received_at DESC, id LIMIT ?4 OFFSET ?5"
        ))?;
        let items = stmt
            .query_map(
                params![
                    folder_id,
                    query.unread_only,
                    query.flagged_only,
                    limit,
                    query.offset
                ],
                row_to_summary,
            )?
            .collect::<rusqlite::Result<_>>()?;
        Ok(MessagePage { items, total })
    }

    /// Full-text search over subject, sender, recipients and body.
    pub fn search_messages(
        &self,
        text: &str,
        account_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<MessageSummary>> {
        let Some(fts) = fts_query(text) else {
            return Ok(Vec::new());
        };
        let conn = self.conn();
        let columns = SUMMARY_COLUMNS
            .split(", ")
            .map(|c| format!("m.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut stmt = conn.prepare(&format!(
            "SELECT {columns} FROM messages_fts f JOIN messages m ON m.id = f.message_id
             WHERE messages_fts MATCH ?1 AND (?2 IS NULL OR m.account_id = ?2)
             ORDER BY m.received_at DESC LIMIT ?3"
        ))?;
        let rows = stmt.query_map(params![fts, account_id, limit], row_to_summary)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get_message_summary(&self, id: &str) -> Result<MessageSummary> {
        self.conn()
            .query_row(
                &format!("SELECT {SUMMARY_COLUMNS} FROM messages WHERE id = ?1"),
                [id],
                row_to_summary,
            )
            .optional()?
            .ok_or_else(|| Error::not_found("message"))
    }

    pub fn get_message_summaries(&self, ids: &[String]) -> Result<Vec<MessageSummary>> {
        ids.iter().map(|id| self.get_message_summary(id)).collect()
    }

    /// Returns the cached content of a message, or `None` if the body has not
    /// been fetched yet.
    pub fn get_message_content(&self, id: &str) -> Result<Option<MessageContent>> {
        let row: Option<(Option<String>, Option<String>, Option<String>)> = self
            .conn()
            .query_row(
                "SELECT content_json, body_html, body_text FROM messages WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((content_json, body_html, body_text)) = row else {
            return Err(Error::not_found("message"));
        };
        let Some(content_json) = content_json else {
            return Ok(None);
        };
        let stored: StoredContent = from_json(&content_json);
        Ok(Some(MessageContent {
            body_html,
            body_text,
            bcc: stored.bcc,
            reply_to: stored.reply_to,
            attachments: stored.attachments,
            internet_message_id: stored.internet_message_id,
            in_reply_to: stored.in_reply_to,
            references: stored.references,
        }))
    }

    pub fn save_message_content(&self, id: &str, content: &MessageContent) -> Result<()> {
        let stored = StoredContent {
            bcc: content.bcc.clone(),
            reply_to: content.reply_to.clone(),
            attachments: content.attachments.clone(),
            internet_message_id: content.internet_message_id.clone(),
            in_reply_to: content.in_reply_to.clone(),
            references: content.references.clone(),
        };
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE messages SET content_json = ?2, body_html = ?3, body_text = ?4 WHERE id = ?1",
            params![id, to_json(&stored), content.body_html, content.body_text],
        )?;
        if changed == 0 {
            return Err(Error::not_found("message"));
        }
        let summary = tx.query_row(
            &format!("SELECT {SUMMARY_COLUMNS} FROM messages WHERE id = ?1"),
            [id],
            row_to_summary,
        )?;
        let body = match (&content.body_text, &content.body_html) {
            (Some(text), _) => text.clone(),
            (None, Some(html)) => html_to_text(html),
            (None, None) => summary.preview.clone(),
        };
        index_message(&tx, &summary, &body)?;
        tx.commit()?;
        Ok(())
    }

    /// Marks messages read/unread and keeps folder unread counts in step.
    pub fn set_messages_read(&self, ids: &[String], read: bool) -> Result<Vec<FolderUnreadDelta>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut deltas: HashMap<String, i64> = HashMap::new();
        for id in ids {
            let row: Option<(String, bool)> = tx
                .query_row(
                    "SELECT folder_id, is_read FROM messages WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let Some((folder_id, was_read)) = row else {
                continue;
            };
            if was_read == read {
                continue;
            }
            tx.execute(
                "UPDATE messages SET is_read = ?2 WHERE id = ?1",
                params![id, read],
            )?;
            *deltas.entry(folder_id).or_default() += if read { -1 } else { 1 };
        }
        for (folder_id, delta) in &deltas {
            tx.execute(
                "UPDATE folders SET unread_count = MAX(0, unread_count + ?2) WHERE id = ?1",
                params![folder_id, delta],
            )?;
        }
        tx.commit()?;
        Ok(deltas
            .into_iter()
            .map(|(folder_id, delta)| FolderUnreadDelta { folder_id, delta })
            .collect())
    }

    pub fn set_messages_flagged(&self, ids: &[String], flagged: bool) -> Result<()> {
        let conn = self.conn();
        let placeholders = vec!["?"; ids.len()].join(", ");
        conn.execute(
            &format!("UPDATE messages SET is_flagged = ? WHERE id IN ({placeholders})"),
            params_from_iter(
                std::iter::once(rusqlite::types::Value::from(flagged)).chain(
                    ids.iter()
                        .map(|id| rusqlite::types::Value::from(id.clone())),
                ),
            ),
        )?;
        Ok(())
    }

    /// Reflects a server-side move locally. When the new remote id is unknown
    /// the message is dropped and will reappear with the next sync of the
    /// destination folder.
    pub fn relocate_message(
        &self,
        id: &str,
        destination: &Folder,
        new_remote_id: Option<&str>,
    ) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let row: Option<(String, bool)> = tx
            .query_row(
                "SELECT folder_id, is_read FROM messages WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((source_folder, is_read)) = row else {
            return Err(Error::not_found("message"));
        };
        if !is_read {
            tx.execute(
                "UPDATE folders SET unread_count = MAX(0, unread_count - 1) WHERE id = ?1",
                [&source_folder],
            )?;
            tx.execute(
                "UPDATE folders SET unread_count = unread_count + 1 WHERE id = ?1",
                [&destination.id],
            )?;
        }
        match new_remote_id {
            Some(remote_id) => {
                let new_id = local_id(&destination.account_id, EntityKind::Message, remote_id);
                tx.execute(
                    "UPDATE messages SET id = ?2, remote_id = ?3, folder_id = ?4 WHERE id = ?1",
                    params![id, new_id, remote_id, destination.id],
                )?;
                tx.execute(
                    "UPDATE messages_fts SET message_id = ?2 WHERE message_id = ?1",
                    params![id, new_id],
                )?;
            }
            None => {
                delete_message_rows(&tx, &[id.to_owned()])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_messages(&self, ids: &[String]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for id in ids {
            let row: Option<(String, bool)> = tx
                .query_row(
                    "SELECT folder_id, is_read FROM messages WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((folder_id, false)) = row {
                tx.execute(
                    "UPDATE folders SET unread_count = MAX(0, unread_count - 1),
                       total_count = MAX(0, total_count - 1) WHERE id = ?1",
                    [&folder_id],
                )?;
            } else if let Some((folder_id, true)) = row {
                tx.execute(
                    "UPDATE folders SET total_count = MAX(0, total_count - 1) WHERE id = ?1",
                    [&folder_id],
                )?;
            }
        }
        delete_message_rows(&tx, ids)?;
        tx.commit()?;
        Ok(())
    }

    /// Recipient autocomplete: contacts first, then people the user has
    /// corresponded with, most frequent first.
    pub fn suggest_addresses(&self, text: &str, limit: u32) -> Result<Vec<EmailAddress>> {
        let pattern = like_pattern(&text.to_lowercase());
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for contact in self.search_contacts_raw(&pattern, limit)? {
            for email in contact.emails {
                if seen.insert(email.address.to_lowercase()) {
                    out.push(EmailAddress {
                        name: email.name.or_else(|| Some(contact.display_name.clone())),
                        address: email.address,
                    });
                }
            }
        }
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT from_name, from_address, COUNT(*) AS n FROM messages
             WHERE from_address IS NOT NULL
               AND (LOWER(from_address) LIKE ?1 ESCAPE '\\' OR LOWER(from_name) LIKE ?1 ESCAPE '\\')
             GROUP BY LOWER(from_address) ORDER BY n DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pattern, limit], |r| {
            Ok(EmailAddress {
                name: r.get(0)?,
                address: r.get(1)?,
            })
        })?;
        for addr in rows {
            let addr = addr?;
            if seen.insert(addr.address.to_lowercase()) {
                out.push(addr);
            }
        }
        out.truncate(limit as usize);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};

    use super::*;
    use crate::store::test_support::imap_account;

    fn folder(account: &str, remote: &str, role: FolderRole) -> Folder {
        Folder {
            id: local_id(account, EntityKind::Folder, remote),
            account_id: account.into(),
            remote_id: remote.into(),
            parent_id: None,
            name: remote.into(),
            role,
            unread_count: 1,
            total_count: 2,
        }
    }

    fn message(folder: &Folder, remote: &str, subject: &str, read: bool) -> MessageSummary {
        MessageSummary {
            id: local_id(&folder.account_id, EntityKind::Message, remote),
            account_id: folder.account_id.clone(),
            folder_id: folder.id.clone(),
            remote_id: remote.into(),
            conversation_id: None,
            subject: subject.into(),
            from: Some(EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")),
            to: vec![EmailAddress::new(None, "me@example.com")],
            cc: vec![],
            preview: format!("preview of {subject}"),
            received_at: Utc::now() - Duration::minutes(remote.len() as i64),
            is_read: read,
            is_flagged: false,
            is_draft: false,
            has_attachments: false,
            importance: Importance::Normal,
        }
    }

    fn setup() -> (Store, Folder, Folder) {
        let store = Store::open_in_memory().unwrap();
        store.insert_account(&imap_account("a")).unwrap();
        let inbox = folder("a", "INBOX", FolderRole::Inbox);
        let archive = folder("a", "Archive", FolderRole::Archive);
        store
            .replace_folders("a", &[inbox.clone(), archive.clone()])
            .unwrap();
        (store, inbox, archive)
    }

    #[test]
    fn batch_upsert_list_and_full_resync() {
        let (store, inbox, _) = setup();
        let batch = MessageBatch {
            upserts: vec![
                message(&inbox, "1", "Quarterly report", false),
                message(&inbox, "22", "Lunch?", true),
            ],
            sync_state: Some("cursor-1".into()),
            ..Default::default()
        };
        let outcome = store.apply_message_batch(&inbox, &batch).unwrap();
        assert_eq!(outcome.inserted.len(), 2);
        assert_eq!(
            store.folder_sync_state(&inbox.id).unwrap().as_deref(),
            Some("cursor-1")
        );

        let page = store
            .list_messages(
                &inbox.id,
                &MessageQuery {
                    limit: 10,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(page.total, 2);
        assert_eq!(page.items[0].subject, "Quarterly report");

        let unread = store
            .list_messages(
                &inbox.id,
                &MessageQuery {
                    limit: 10,
                    unread_only: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(unread.total, 1);

        // Re-applying is not reported as new, nor as any other change.
        let again = store.apply_message_batch(&inbox, &batch).unwrap();
        assert_eq!(again, BatchOutcome::default());
        assert!(!again.changes_counts());

        let resync = MessageBatch {
            upserts: vec![message(&inbox, "1", "Quarterly report", false)],
            full_resync: true,
            ..Default::default()
        };
        store.apply_message_batch(&inbox, &resync).unwrap();
        assert_eq!(
            store
                .list_messages(&inbox.id, &MessageQuery::default())
                .unwrap()
                .total,
            1
        );
    }

    #[test]
    fn flag_updates_retain_only_and_preview_preservation() {
        let (store, inbox, _) = setup();
        let batch = MessageBatch {
            upserts: vec![
                message(&inbox, "1", "One", false),
                message(&inbox, "2", "Two", false),
                message(&inbox, "3", "Three", false),
            ],
            ..Default::default()
        };
        store.apply_message_batch(&inbox, &batch).unwrap();

        let mut changed = message(&inbox, "1", "One (edited)", false);
        changed.preview = String::new();
        let update = MessageBatch {
            upserts: vec![changed],
            flag_updates: vec![crate::providers::FlagUpdate {
                remote_id: "2".into(),
                is_read: true,
                is_flagged: true,
            }],
            retain_only: Some(vec!["1".into(), "2".into()]),
            ..Default::default()
        };
        store.apply_message_batch(&inbox, &update).unwrap();

        let page = store
            .list_messages(&inbox.id, &MessageQuery::default())
            .unwrap();
        assert_eq!(page.total, 2);
        let one = page.items.iter().find(|m| m.remote_id == "1").unwrap();
        assert_eq!(one.subject, "One (edited)");
        assert_eq!(one.preview, "preview of One");
        let two = page.items.iter().find(|m| m.remote_id == "2").unwrap();
        assert!(two.is_read && two.is_flagged);
        assert!(store.search_messages("three", None, 10).unwrap().is_empty());
    }

    #[test]
    fn batch_outcome_reports_read_changes_and_removals() {
        let (store, inbox, _) = setup();
        store
            .apply_message_batch(
                &inbox,
                &MessageBatch {
                    upserts: vec![
                        message(&inbox, "1", "One", false),
                        message(&inbox, "2", "Two", false),
                        message(&inbox, "3", "Three", true),
                        message(&inbox, "4", "Four", false),
                    ],
                    ..Default::default()
                },
            )
            .unwrap();
        let id = |remote: &str| local_id("a", EntityKind::Message, remote);
        let flag = |remote: &str, is_read: bool| crate::providers::FlagUpdate {
            remote_id: remote.into(),
            is_read,
            is_flagged: false,
        };

        // Read elsewhere (flag update and upsert), unread elsewhere, deleted
        // elsewhere, and an update for a message that isn't stored.
        let outcome = store
            .apply_message_batch(
                &inbox,
                &MessageBatch {
                    upserts: vec![message(&inbox, "2", "Two", true)],
                    flag_updates: vec![flag("1", true), flag("3", false), flag("99", true)],
                    removed_remote_ids: vec!["4".into(), "missing".into()],
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(outcome.inserted.is_empty());
        let mut became_read = outcome.became_read.clone();
        became_read.sort();
        let mut expected = vec![id("1"), id("2")];
        expected.sort();
        assert_eq!(became_read, expected);
        assert_eq!(outcome.became_unread, [id("3")]);
        assert_eq!(outcome.removed, [id("4")]);
        assert!(outcome.changes_counts());

        // Messages missing from `retain_only` are reported as removed once.
        let outcome = store
            .apply_message_batch(
                &inbox,
                &MessageBatch {
                    removed_remote_ids: vec!["3".into()],
                    retain_only: Some(vec!["1".into()]),
                    ..Default::default()
                },
            )
            .unwrap();
        let mut removed = outcome.removed.clone();
        removed.sort();
        let mut expected = vec![id("2"), id("3")];
        expected.sort();
        assert_eq!(removed, expected);

        // A full resync reports the stale messages it drops.
        let outcome = store
            .apply_message_batch(
                &inbox,
                &MessageBatch {
                    upserts: vec![message(&inbox, "5", "Five", false)],
                    full_resync: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(outcome.removed, [id("1")]);
        assert_eq!(outcome.inserted.len(), 1);
    }

    #[test]
    fn messages_moved_in_report_the_folder_they_left() {
        let (store, inbox, archive) = setup();
        let archived = message(&archive, "1", "One", false);
        let batch = MessageBatch {
            upserts: vec![archived.clone()],
            ..Default::default()
        };
        store.apply_message_batch(&archive, &batch).unwrap();

        // Graph keeps the id when a message moves to the Inbox.
        let moved = MessageSummary {
            folder_id: inbox.id.clone(),
            ..archived
        };
        let batch = MessageBatch {
            upserts: vec![moved],
            ..Default::default()
        };
        let outcome = store.apply_message_batch(&inbox, &batch).unwrap();
        assert!(outcome.inserted.is_empty(), "not new");
        assert_eq!(outcome.moved_from, std::slice::from_ref(&archive.id));
        assert!(outcome.changes_counts());
        // Applied again, it stays put.
        let again = store.apply_message_batch(&inbox, &batch).unwrap();
        assert_eq!(again, BatchOutcome::default());
    }

    #[test]
    fn folder_counts_and_lookup_by_remote_id() {
        let (store, inbox, archive) = setup();
        store.set_folder_counts(&inbox.id, 7, 42).unwrap();
        let stored = store.get_folder(&inbox.id).unwrap();
        assert_eq!((stored.unread_count, stored.total_count), (7, 42));
        assert_eq!(
            store
                .find_folder_by_remote_id("a", "Archive")
                .unwrap()
                .map(|f| f.id),
            Some(archive.id)
        );
        assert_eq!(store.find_folder_by_remote_id("a", "Nope").unwrap(), None);
        assert_eq!(store.find_folder_by_remote_id("b", "INBOX").unwrap(), None);
    }

    #[test]
    fn search_finds_subject_sender_and_fetched_body() {
        let (store, inbox, _) = setup();
        let m = message(&inbox, "1", "Quarterly report", false);
        store
            .apply_message_batch(
                &inbox,
                &MessageBatch {
                    upserts: vec![m.clone()],
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(store.search_messages("quarter", None, 10).unwrap().len(), 1);
        assert_eq!(
            store.search_messages("lovelace", None, 10).unwrap().len(),
            1
        );
        assert!(
            store
                .search_messages("pineapple", None, 10)
                .unwrap()
                .is_empty()
        );
        // Hostile FTS syntax is treated as plain words.
        assert!(store.search_messages("\"a\" OR NEAR(", None, 10).is_ok());

        store
            .save_message_content(
                &m.id,
                &MessageContent {
                    body_html: Some("<p>Pineapple <b>pizza</b></p>".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(
            store.search_messages("pineapple", None, 10).unwrap().len(),
            1
        );
        let content = store.get_message_content(&m.id).unwrap().unwrap();
        assert!(content.body_html.unwrap().contains("Pineapple"));
    }

    #[test]
    fn read_state_updates_unread_counts() {
        let (store, inbox, _) = setup();
        let m = message(&inbox, "1", "Hi", false);
        store
            .apply_message_batch(
                &inbox,
                &MessageBatch {
                    upserts: vec![m.clone()],
                    ..Default::default()
                },
            )
            .unwrap();
        let deltas = store
            .set_messages_read(std::slice::from_ref(&m.id), true)
            .unwrap();
        assert_eq!(
            deltas,
            vec![FolderUnreadDelta {
                folder_id: inbox.id.clone(),
                delta: -1
            }]
        );
        assert_eq!(store.get_folder(&inbox.id).unwrap().unread_count, 0);
        // Idempotent.
        assert!(
            store
                .set_messages_read(std::slice::from_ref(&m.id), true)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn relocate_with_and_without_new_remote_id() {
        let (store, inbox, archive) = setup();
        let m1 = message(&inbox, "1", "One", true);
        let m2 = message(&inbox, "2", "Two", true);
        store
            .apply_message_batch(
                &inbox,
                &MessageBatch {
                    upserts: vec![m1.clone(), m2.clone()],
                    ..Default::default()
                },
            )
            .unwrap();
        store
            .relocate_message(&m1.id, &archive, Some("Archive:9"))
            .unwrap();
        store.relocate_message(&m2.id, &archive, None).unwrap();
        let moved = store
            .list_messages(&archive.id, &MessageQuery::default())
            .unwrap();
        assert_eq!(moved.items.len(), 1);
        assert_eq!(moved.items[0].remote_id, "Archive:9");
        assert_eq!(
            store.search_messages("one", None, 10).unwrap()[0].id,
            moved.items[0].id
        );
        assert_eq!(
            store
                .list_messages(&inbox.id, &MessageQuery::default())
                .unwrap()
                .total,
            0
        );
    }

    #[test]
    fn suggests_previous_correspondents() {
        let (store, inbox, _) = setup();
        store
            .apply_message_batch(
                &inbox,
                &MessageBatch {
                    upserts: vec![message(&inbox, "1", "Hi", true)],
                    ..Default::default()
                },
            )
            .unwrap();
        let s = store.suggest_addresses("ada", 5).unwrap();
        assert_eq!(s[0].address, "ada@example.com");
        assert!(store.suggest_addresses("100%_", 5).unwrap().is_empty());
    }
}
