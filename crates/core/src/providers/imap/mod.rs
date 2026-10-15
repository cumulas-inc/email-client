//! IMAP (mail access) + SMTP (sending) provider for standards-based accounts.
//!
//! One authenticated IMAP session is kept per account and shared by all
//! operations; it is replaced when it breaks or has been idle for a while,
//! and an operation that fails because the connection dropped is retried once
//! on a fresh one.
//!
//! Message remote ids encode mailbox, UIDVALIDITY and UID (see [`remote_id`]),
//! so they are unique per account and stale ids are detected when a server
//! renumbers a mailbox.

mod connection;
mod folders;
mod mime;
mod smtp;
mod sync;
mod utf7;

use std::collections::BTreeMap;

use async_imap::imap_proto::rfc4315::UidSetMember;
use async_imap::imap_proto::{Response, ResponseCode};
use async_trait::async_trait;
use tokio::sync::{Mutex, MutexGuard};

use self::connection::{ImapSession, MailboxInfo, quote};
use super::{FolderCounts, MailProvider, MessageBatch, MessageContent, MessageLocator, SyncWindow};
use crate::error::{Error, ErrorCode, Result};
use crate::model::{Folder, ImapSettings, Message, OutgoingMessage};

/// UIDs per `UID STORE`/`MOVE`/`COPY`/`EXPUNGE` command, keeping command
/// lines well below server limits.
const UIDS_PER_COMMAND: usize = 500;

/// Separates the parts of a message remote id; cannot occur in mailbox names
/// sent by servers (control characters are not allowed there).
const ID_SEPARATOR: char = '\u{1f}';

/// Credentials and server settings for one IMAP account.
#[derive(Clone)]
pub struct ImapCredentials {
    pub settings: ImapSettings,
    pub password: String,
    /// Used as the `From` display name when sending.
    pub display_name: String,
    pub email: String,
}

/// [`MailProvider`] for IMAP accounts, sending through SMTP.
pub struct ImapProvider {
    account_id: String,
    credentials: ImapCredentials,
    session: Mutex<Option<ImapSession>>,
}

/// Runs `$operation` (an async expression using `$session`) on the account's
/// session with the reconnect-and-retry policy of [`SessionSlot`].
macro_rules! with_session {
    ($provider:expr, |$session:ident| $operation:expr) => {{
        let mut slot = $provider.slot().await;
        loop {
            let $session = slot.session().await?;
            let result = $operation.await;
            if !slot.retry(&result) {
                break result;
            }
        }
    }};
}

impl ImapProvider {
    pub fn new(account_id: impl Into<String>, credentials: ImapCredentials) -> Self {
        Self {
            account_id: account_id.into(),
            credentials,
            session: Mutex::new(None),
        }
    }

    async fn slot(&self) -> SessionSlot<'_> {
        SessionSlot {
            guard: self.session.lock().await,
            credentials: &self.credentials,
            retried: false,
        }
    }

    async fn save_sent_copy(&self, raw: &[u8]) -> Result<()> {
        with_session!(self, |session| save_to_sent(session, raw))
    }
}

/// Saves a sent message to the Sent folder, creating it if needed. Gmail
/// files messages sent through its SMTP server by itself.
async fn save_to_sent(session: &mut ImapSession, raw: &[u8]) -> Result<()> {
    if session.has_capability("X-GM-EXT-1") {
        return Ok(());
    }
    let mailbox = match folders::sent_mailbox(session).await? {
        Some(mailbox) => mailbox,
        None => {
            let name = folders::new_sent_mailbox_name(session).await?;
            session
                .execute(&format!("CREATE {}", quote(&name)?), |_| {})
                .await?;
            // Unsubscribed folders are hidden by some other clients.
            let _ = session
                .execute(&format!("SUBSCRIBE {}", quote(&name)?), |_| {})
                .await;
            name
        }
    };
    session.append(&mailbox, "(\\Seen)", raw).await
}

/// Exclusive use of the account's session for one operation.
struct SessionSlot<'a> {
    guard: MutexGuard<'a, Option<ImapSession>>,
    credentials: &'a ImapCredentials,
    retried: bool,
}

impl SessionSlot<'_> {
    /// The current session, connecting first if there is none or it is no
    /// longer usable.
    async fn session(&mut self) -> Result<&mut ImapSession> {
        let session = match self.guard.take() {
            Some(session) if session.is_reusable() => session,
            _ => ImapSession::connect(self.credentials).await?,
        };
        Ok(self.guard.insert(session))
    }

    /// Drops a broken session and decides whether to run the operation again:
    /// once, and only when the connection failed.
    fn retry<T>(&mut self, result: &Result<T>) -> bool {
        if self.guard.as_ref().is_some_and(|s| !s.is_healthy()) {
            *self.guard = None;
        }
        let retry = !self.retried && matches!(result, Err(e) if e.code == ErrorCode::Network);
        if retry {
            tracing::debug!("IMAP connection failed, retrying on a new one");
            self.retried = true;
            *self.guard = None;
        }
        retry
    }
}

/// Logs in to the IMAP server and authenticates against the SMTP server
/// without changing anything; used to validate settings when adding an account.
pub async fn test_connection(credentials: &ImapCredentials) -> Result<()> {
    ImapSession::connect(credentials).await?.logout().await;
    smtp::verify(credentials).await
}

/// Builds the remote id of a message.
fn remote_id(mailbox: &str, uid_validity: u32, uid: u32) -> String {
    format!("{mailbox}{ID_SEPARATOR}{uid_validity}{ID_SEPARATOR}{uid}")
}

/// A message's address on the server, parsed from its remote id.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MessageUid {
    mailbox: String,
    uid_validity: u32,
    uid: u32,
}

impl MessageUid {
    fn parse(remote_id: &str) -> Result<Self> {
        let invalid = || Error::invalid_input("Not an IMAP message id.");
        // Parsed from the right: the mailbox name may contain anything else.
        let mut parts = remote_id.rsplitn(3, ID_SEPARATOR);
        let uid = parts
            .next()
            .and_then(|p| p.parse().ok())
            .ok_or_else(invalid)?;
        let uid_validity = parts
            .next()
            .and_then(|p| p.parse().ok())
            .ok_or_else(invalid)?;
        let mailbox = parts.next().filter(|m| !m.is_empty()).ok_or_else(invalid)?;
        Ok(Self {
            mailbox: mailbox.to_owned(),
            uid_validity,
            uid,
        })
    }
}

/// Formats UIDs as a compact sequence set (`1:3,7,9:10`).
fn uid_set(uids: &[u32]) -> String {
    let mut sorted = uids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut ranges: Vec<String> = Vec::new();
    let mut iter = sorted.into_iter().peekable();
    while let Some(start) = iter.next() {
        let mut end = start;
        while iter.peek() == Some(&end.wrapping_add(1)) {
            end += 1;
            iter.next();
        }
        ranges.push(if start == end {
            start.to_string()
        } else {
            format!("{start}:{end}")
        });
    }
    ranges.join(",")
}

/// Expands a `COPYUID` UID set in server order (ranges are ascending).
fn expand_uid_set(members: &[UidSetMember]) -> Vec<u32> {
    let mut uids = Vec::new();
    for member in members {
        match member {
            UidSetMember::Uid(uid) => uids.push(*uid),
            UidSetMember::UidRange(range) => uids.extend(range.clone()),
        }
    }
    uids
}

/// Locators grouped by mailbox, keeping each message's position in the input.
fn group_by_mailbox(
    messages: &[MessageLocator],
) -> Result<BTreeMap<String, Vec<(usize, MessageUid)>>> {
    let mut groups: BTreeMap<String, Vec<(usize, MessageUid)>> = BTreeMap::new();
    for (i, locator) in messages.iter().enumerate() {
        let id = MessageUid::parse(&locator.remote_id)?;
        groups.entry(id.mailbox.clone()).or_default().push((i, id));
    }
    Ok(groups)
}

/// Selects `mailbox` and checks that the UIDs of `ids` still refer to the
/// same messages (the server did not renumber the mailbox).
async fn open_checked(
    session: &mut ImapSession,
    mailbox: &str,
    ids: &[(usize, MessageUid)],
    writable: bool,
) -> Result<MailboxInfo> {
    let info = session.open(mailbox, writable).await?;
    if ids
        .iter()
        .any(|(_, id)| id.uid_validity != info.uid_validity)
    {
        return Err(Error::not_found(
            "message (the folder changed on the server; it is being refreshed)",
        ));
    }
    Ok(info)
}

/// Adds or removes a flag on messages, folder by folder.
async fn store_flag(
    session: &mut ImapSession,
    messages: &[MessageLocator],
    flag: &str,
    add: bool,
) -> Result<()> {
    for (mailbox, ids) in group_by_mailbox(messages)? {
        open_checked(session, &mailbox, &ids, true).await?;
        let uids: Vec<u32> = ids.iter().map(|(_, id)| id.uid).collect();
        for chunk in uids.chunks(UIDS_PER_COMMAND) {
            let command = format!(
                "UID STORE {} {}FLAGS.SILENT ({flag})",
                uid_set(chunk),
                if add { '+' } else { '-' }
            );
            session.execute(&command, |_| {}).await?;
        }
    }
    Ok(())
}

/// Removes messages flagged `\Deleted`: only `uids` when the server supports
/// UIDPLUS, otherwise every such message of the selected mailbox.
async fn expunge(session: &mut ImapSession, uids: &[u32]) -> Result<()> {
    if session.has_capability("UIDPLUS") {
        session
            .execute(&format!("UID EXPUNGE {}", uid_set(uids)), |_| {})
            .await
    } else {
        session.execute("EXPUNGE", |_| {}).await
    }
}

async fn fetch_raw(
    session: &mut ImapSession,
    id: &MessageUid,
    items: &str,
) -> Result<connection::FetchedMessage> {
    session
        .uid_fetch(&id.uid.to_string(), items)
        .await?
        .into_iter()
        .find(|m| m.uid == id.uid && m.body.is_some())
        .ok_or_else(|| Error::not_found("message"))
}

async fn fetch_message(
    session: &mut ImapSession,
    locator: &MessageLocator,
) -> Result<MessageContent> {
    let id = MessageUid::parse(&locator.remote_id)?;
    open_checked(session, &id.mailbox, &[(0, id.clone())], false).await?;
    let fetched = fetch_raw(session, &id, "(UID BODY.PEEK[])").await?;
    Ok(mime::parse_message(
        fetched.body.as_deref().unwrap_or_default(),
    ))
}

async fn fetch_attachment(
    session: &mut ImapSession,
    locator: &MessageLocator,
    attachment_id: &str,
) -> Result<Vec<u8>> {
    let id = MessageUid::parse(&locator.remote_id)?;
    // Attachment ids are section paths; anything else is not ours.
    if attachment_id.is_empty()
        || !attachment_id
            .split('.')
            .all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(Error::not_found("attachment"));
    }
    let checked = [(0, id.clone())];
    open_checked(session, &id.mailbox, &checked, false).await?;
    // Fetch just the part when the server can give us its MIME header;
    // otherwise fall back to the whole message.
    let items = format!("(UID BODY.PEEK[{attachment_id}.MIME] BODY.PEEK[{attachment_id}])");
    match fetch_raw(session, &id, &items).await {
        Ok(part) => {
            if let (Some(header), Some(body)) = (&part.mime, &part.body)
                && !header.trim_ascii().is_empty()
            {
                // Some servers (GreenMail) omit the blank line ending the header.
                let mut entity = header.trim_ascii_end().to_vec();
                entity.extend_from_slice(b"\r\n\r\n");
                entity.extend_from_slice(body);
                if let Some(content) = mime::entity_content(&entity) {
                    return Ok(content);
                }
            }
        }
        // The caller retries on a new connection.
        Err(e) if e.code == ErrorCode::Network => return Err(e),
        Err(e) => {
            tracing::debug!("fetching attachment part {attachment_id} failed: {e}");
            if !session.is_healthy() {
                session.reconnect().await?;
                open_checked(session, &id.mailbox, &checked, false).await?;
            }
        }
    }
    let whole = fetch_raw(session, &id, "(UID BODY.PEEK[])").await?;
    mime::attachment_content(whole.body.as_deref().unwrap_or_default(), attachment_id)
        .ok_or_else(|| Error::not_found("attachment"))
}

async fn move_messages(
    session: &mut ImapSession,
    messages: &[MessageLocator],
    destination: &str,
) -> Result<Vec<Option<String>>> {
    let mut new_ids = vec![None; messages.len()];
    let target = quote(destination)?;
    for (mailbox, ids) in group_by_mailbox(messages)? {
        if mailbox == destination {
            for (index, _) in &ids {
                new_ids[*index] = Some(messages[*index].remote_id.clone());
            }
            continue;
        }
        open_checked(session, &mailbox, &ids, true).await?;
        for chunk in ids.chunks(UIDS_PER_COMMAND) {
            let uids: Vec<u32> = chunk.iter().map(|(_, id)| id.uid).collect();
            let set = uid_set(&uids);
            let mut copies: Vec<(u32, u32, u32)> = Vec::new();
            // COPYUID arrives in an untagged OK (MOVE) or the tagged OK (COPY).
            let mut collect = |response: &Response<'_>| {
                let outcome = match response {
                    Response::Data { outcome, .. } | Response::Done { outcome, .. } => outcome,
                    _ => return,
                };
                if let Some(ResponseCode::CopyUid(validity, source, destination)) = &outcome.code {
                    let pairs = expand_uid_set(source)
                        .into_iter()
                        .zip(expand_uid_set(destination));
                    copies.extend(pairs.map(|(from, to)| (from, *validity, to)));
                }
            };
            if session.has_capability("MOVE") {
                session
                    .execute(&format!("UID MOVE {set} {target}"), &mut collect)
                    .await?;
            } else {
                session
                    .execute(&format!("UID COPY {set} {target}"), &mut collect)
                    .await?;
                session
                    .execute(
                        &format!("UID STORE {set} +FLAGS.SILENT (\\Deleted)"),
                        |_| {},
                    )
                    .await?;
                expunge(session, &uids).await?;
            }
            for (index, id) in chunk {
                new_ids[*index] = copies
                    .iter()
                    .find(|(from, _, _)| *from == id.uid)
                    .map(|(_, validity, to)| remote_id(destination, *validity, *to));
            }
        }
    }
    Ok(new_ids)
}

async fn delete_messages(session: &mut ImapSession, messages: &[MessageLocator]) -> Result<()> {
    for (mailbox, ids) in group_by_mailbox(messages)? {
        open_checked(session, &mailbox, &ids, true).await?;
        let uids: Vec<u32> = ids.iter().map(|(_, id)| id.uid).collect();
        for chunk in uids.chunks(UIDS_PER_COMMAND) {
            session
                .execute(
                    &format!("UID STORE {} +FLAGS.SILENT (\\Deleted)", uid_set(chunk)),
                    |_| {},
                )
                .await?;
            expunge(session, chunk).await?;
        }
    }
    Ok(())
}

#[async_trait]
impl MailProvider for ImapProvider {
    async fn list_folders(&self) -> Result<Vec<Folder>> {
        with_session!(self, |session| folders::list(session, &self.account_id))
    }

    async fn sync_messages(
        &self,
        folder: &Folder,
        sync_state: Option<&str>,
        window: SyncWindow,
    ) -> Result<MessageBatch> {
        with_session!(self, |session| sync::sync_folder(
            session, folder, sync_state, window
        ))
    }

    async fn folder_counts(&self, folder: &Folder) -> Result<Option<FolderCounts>> {
        let counts = with_session!(self, |session| folders::counts(session, &folder.remote_id))?;
        Ok(counts.map(|(total, unread)| FolderCounts { unread, total }))
    }

    async fn fetch_message(&self, message: &MessageLocator) -> Result<MessageContent> {
        with_session!(self, |session| fetch_message(session, message))
    }

    async fn fetch_attachment(
        &self,
        message: &MessageLocator,
        attachment_id: &str,
    ) -> Result<Vec<u8>> {
        with_session!(self, |session| fetch_attachment(
            session,
            message,
            attachment_id
        ))
    }

    async fn set_read(&self, messages: &[MessageLocator], read: bool) -> Result<()> {
        with_session!(self, |session| store_flag(
            session, messages, "\\Seen", read
        ))
    }

    async fn set_flagged(&self, messages: &[MessageLocator], flagged: bool) -> Result<()> {
        with_session!(self, |session| store_flag(
            session,
            messages,
            "\\Flagged",
            flagged
        ))
    }

    async fn move_messages(
        &self,
        messages: &[MessageLocator],
        destination_folder_remote_id: &str,
    ) -> Result<Vec<Option<String>>> {
        with_session!(self, |session| move_messages(
            session,
            messages,
            destination_folder_remote_id
        ))
    }

    async fn delete_messages(&self, messages: &[MessageLocator]) -> Result<()> {
        with_session!(self, |session| delete_messages(session, messages))
    }

    async fn send(&self, message: &OutgoingMessage, original: Option<&Message>) -> Result<()> {
        let built = smtp::build_message(&self.credentials, message, original)?;
        let raw = built.formatted();
        smtp::send(&self.credentials, &built, &raw).await?;
        // The message is out; failing to file a copy must not make the user
        // send it again.
        if let Err(e) = self.save_sent_copy(&raw).await {
            tracing::warn!("saving the sent message to the Sent folder failed: {e}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_ids_round_trip() {
        for mailbox in ["INBOX", "Archive/2024", "Entw&APw-rfe", "weird\u{1e}name 7"] {
            let id = remote_id(mailbox, 1_234_567, 42);
            assert_eq!(
                MessageUid::parse(&id).unwrap(),
                MessageUid {
                    mailbox: mailbox.to_owned(),
                    uid_validity: 1_234_567,
                    uid: 42
                }
            );
        }
    }

    #[test]
    fn rejects_foreign_remote_ids() {
        for id in [
            "AAMkAGI2",
            "INBOX\u{1f}x\u{1f}1",
            "\u{1f}1\u{1f}2",
            "INBOX\u{1f}1",
        ] {
            assert_eq!(
                MessageUid::parse(id).unwrap_err().code,
                ErrorCode::InvalidInput,
                "{id:?}"
            );
        }
    }

    #[test]
    fn formats_uid_sets() {
        assert_eq!(uid_set(&[7, 1, 2, 3, 9, 10, 3]), "1:3,7,9:10");
        assert_eq!(uid_set(&[5]), "5");
        assert_eq!(uid_set(&[]), "");
        assert_eq!(
            uid_set(&[u32::MAX - 1, u32::MAX]),
            format!("{}:{}", u32::MAX - 1, u32::MAX)
        );
    }

    #[test]
    fn expands_copyuid_sets() {
        let members = [
            UidSetMember::Uid(4),
            UidSetMember::UidRange(10..=12),
            UidSetMember::Uid(2),
        ];
        assert_eq!(expand_uid_set(&members), [4, 10, 11, 12, 2]);
    }

    #[test]
    fn groups_locators_by_mailbox_in_input_order() {
        let locator = |mailbox: &str, uid| MessageLocator {
            folder_remote_id: mailbox.to_owned(),
            remote_id: remote_id(mailbox, 1, uid),
        };
        let groups = group_by_mailbox(&[
            locator("INBOX", 5),
            locator("Archive", 2),
            locator("INBOX", 3),
        ])
        .unwrap();
        let inbox: Vec<_> = groups["INBOX"].iter().map(|(i, id)| (*i, id.uid)).collect();
        assert_eq!(inbox, [(0, 5), (2, 3)]);
        assert_eq!(groups["Archive"][0].0, 1);
    }
}
