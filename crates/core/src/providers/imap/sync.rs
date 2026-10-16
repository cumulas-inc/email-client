//! Folder synchronization.
//!
//! The cursor stored between syncs is `v1:{uidvalidity}:{uidnext}:{modseq}:
//! {low}:{exists}`. `low` is the lowest UID the local cache may hold for the
//! folder (the oldest message of the initial sync window); everything the
//! incremental sync asks the server about is bounded by it, so huge folders
//! cost no more than the window.

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Utc};

use super::connection::{FetchedMessage, ImapSession, MailboxInfo};
use super::mime::{self, PreviewPart};
use super::{remote_id, uid_set};
use crate::error::{Error, ErrorCode, Result};
use crate::model::{EntityKind, Folder, MessageSummary, local_id};
use crate::providers::{FlagUpdate, MessageBatch, SyncWindow};

/// Messages per summary `UID FETCH`, bounding response sizes.
const FETCH_CHUNK: usize = 100;

/// Header fields needed for message summaries.
const SUMMARY_HEADERS: &str = "BODY.PEEK[HEADER.FIELDS (DATE FROM TO CC SUBJECT MESSAGE-ID \
     IN-REPLY-TO REFERENCES IMPORTANCE PRIORITY X-PRIORITY X-MSMAIL-PRIORITY CONTENT-TYPE)]";

/// Sync position of a folder, stored by the sync engine between syncs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Cursor {
    pub uid_validity: u32,
    pub uid_next: u32,
    /// 0 when the server does not support CONDSTORE.
    pub highest_modseq: u64,
    pub low_uid: u32,
    pub exists: u32,
}

impl Cursor {
    pub(super) fn parse(state: &str) -> Option<Self> {
        let mut parts = state.split(':');
        if parts.next()? != "v1" {
            return None;
        }
        let mut number = || parts.next()?.parse::<u64>().ok();
        let cursor = Self {
            uid_validity: u32::try_from(number()?).ok()?,
            uid_next: u32::try_from(number()?).ok()?,
            highest_modseq: number()?,
            low_uid: u32::try_from(number()?).ok()?,
            exists: u32::try_from(number()?).ok()?,
        };
        parts.next().is_none().then_some(cursor)
    }

    pub(super) fn encode(&self) -> String {
        format!(
            "v1:{}:{}:{}:{}:{}",
            self.uid_validity, self.uid_next, self.highest_modseq, self.low_uid, self.exists
        )
    }
}

/// The folder being synced.
struct Target<'a> {
    folder: &'a Folder,
    uid_validity: u32,
}

impl Target<'_> {
    fn remote_id(&self, uid: u32) -> String {
        remote_id(&self.folder.remote_id, self.uid_validity, uid)
    }
}

/// Syncs one folder; see [`crate::providers::MailProvider::sync_messages`].
pub(super) async fn sync_folder(
    session: &mut ImapSession,
    folder: &Folder,
    state: Option<&str>,
    window: SyncWindow,
) -> Result<MessageBatch> {
    let info = session.select(&folder.remote_id, false).await?;
    let target = Target {
        folder,
        uid_validity: info.uid_validity,
    };
    match state
        .and_then(Cursor::parse)
        .filter(|c| c.uid_validity == info.uid_validity)
    {
        Some(previous) => incremental_sync(session, &target, &info, previous, window).await,
        None => full_sync(session, &target, &info, window).await,
    }
}

/// First sync, or the server renumbered the folder (UIDVALIDITY changed):
/// the newest messages of the window replace whatever is cached.
async fn full_sync(
    session: &mut ImapSession,
    target: &Target<'_>,
    info: &MailboxInfo,
    window: SyncWindow,
) -> Result<MessageBatch> {
    let mut uids = if info.exists == 0 {
        Vec::new()
    } else {
        session
            .uid_search(&format!("SINCE {} UNDELETED", imap_date(window.since)))
            .await?
    };
    newest_first(&mut uids, window.max_messages);
    let upserts = fetch_summaries(session, target, &uids).await?;
    let cursor = Cursor {
        uid_validity: info.uid_validity,
        uid_next: info.uid_next,
        highest_modseq: info.highest_modseq.unwrap_or(0),
        low_uid: uids.last().copied().unwrap_or(info.uid_next).max(1),
        exists: info.exists,
    };
    Ok(MessageBatch {
        upserts,
        full_resync: true,
        sync_state: Some(cursor.encode()),
        ..Default::default()
    })
}

/// Later syncs: messages above the previous UIDNEXT are new, deletions are
/// reported through `retain_only`, and flag changes come from CONDSTORE
/// (`CHANGEDSINCE`) or, without it, a flag re-fetch of the known range.
async fn incremental_sync(
    session: &mut ImapSession,
    target: &Target<'_>,
    info: &MailboxInfo,
    previous: Cursor,
    window: SyncWindow,
) -> Result<MessageBatch> {
    let low = previous.low_uid;
    let mut batch = MessageBatch::default();

    // Without new UIDs the message count can only drop through expunges, so
    // an unchanged count and UIDNEXT mean nothing was added or removed.
    if info.uid_next != previous.uid_next || info.exists != previous.exists {
        // `low:*` also matches the highest UID when that is below `low`.
        let present: Vec<u32> = session
            .uid_search(&format!("UID {low}:* UNDELETED"))
            .await?
            .into_iter()
            .filter(|&uid| uid >= low)
            .collect();
        let mut new: Vec<u32> = present
            .iter()
            .copied()
            .filter(|&uid| uid >= previous.uid_next)
            .collect();
        newest_first(&mut new, window.max_messages);
        batch.upserts = fetch_summaries(session, target, &new).await?;
        batch.retain_only = Some(present.iter().map(|&uid| target.remote_id(uid)).collect());
    }

    // Flag changes of the messages known before this sync.
    if previous.uid_next > low {
        let changed_since = match (info.highest_modseq, previous.highest_modseq) {
            (Some(now), before) if before > 0 && now == before => None,
            (Some(_), before) if before > 0 => Some(format!(" (CHANGEDSINCE {before})")),
            _ => Some(String::new()),
        };
        if let Some(modifier) = changed_since {
            let range = format!("{low}:{}", previous.uid_next - 1);
            for message in session
                .uid_fetch(&range, &format!("(UID FLAGS){modifier}"))
                .await?
            {
                if message.uid < low || message.uid >= previous.uid_next {
                    continue;
                }
                let remote_id = target.remote_id(message.uid);
                if message.flags.deleted {
                    batch.removed_remote_ids.push(remote_id);
                } else {
                    batch.flag_updates.push(FlagUpdate {
                        remote_id,
                        is_read: message.flags.seen,
                        is_flagged: message.flags.flagged,
                    });
                }
            }
        }
    }

    batch.sync_state = Some(
        Cursor {
            uid_validity: info.uid_validity,
            uid_next: info.uid_next.max(previous.uid_next),
            highest_modseq: info.highest_modseq.unwrap_or(0),
            low_uid: low,
            exists: info.exists,
        }
        .encode(),
    );
    Ok(batch)
}

/// Sorts UIDs descending (newest first) and keeps at most `max`.
fn newest_first(uids: &mut Vec<u32>, max: u32) {
    uids.sort_unstable_by(|a, b| b.cmp(a));
    uids.dedup();
    uids.truncate(max as usize);
}

/// `SEARCH` date (`1-Jan-2024`); IMAP dates have no time of day.
fn imap_date(instant: DateTime<Utc>) -> String {
    instant.format("%-d-%b-%Y").to_string()
}

/// Fetches summaries (headers, flags, structure and preview) for `uids`.
async fn fetch_summaries(
    session: &mut ImapSession,
    target: &Target<'_>,
    uids: &[u32],
) -> Result<Vec<MessageSummary>> {
    let mut summaries = Vec::with_capacity(uids.len());
    for chunk in uids.chunks(FETCH_CHUNK) {
        let set = uid_set(chunk);
        let query = format!("(UID FLAGS INTERNALDATE BODYSTRUCTURE {SUMMARY_HEADERS})");
        let messages = match session.uid_fetch(&set, &query).await {
            Ok(messages) => messages,
            // Rather than failing the whole folder over a BODYSTRUCTURE the
            // parser rejects, fetch the chunk again without it.
            Err(e) if is_unparsable(&e, session) => {
                tracing::warn!(
                    "fetching message structure in {} failed ({e}); retrying without it",
                    target.folder.remote_id
                );
                recover(session, target).await?;
                let query = format!("(UID FLAGS INTERNALDATE {SUMMARY_HEADERS})");
                session.uid_fetch(&set, &query).await?
            }
            Err(e) => return Err(e),
        };
        let wanted: HashSet<u32> = chunk.iter().copied().collect();
        let messages: Vec<FetchedMessage> = messages
            .into_iter()
            .filter(|m| wanted.contains(&m.uid) && m.header.is_some() && !m.flags.deleted)
            .collect();
        let mut previews = fetch_previews(session, target, &messages).await?;
        for message in messages {
            let preview = previews.remove(&message.uid).unwrap_or_default();
            summaries.push(summary(target, message, preview));
        }
    }
    Ok(summaries)
}

/// Fetches each message's preview part, one command per distinct section,
/// and decodes it.
async fn fetch_previews(
    session: &mut ImapSession,
    target: &Target<'_>,
    messages: &[FetchedMessage],
) -> Result<HashMap<u32, String>> {
    let mut groups: BTreeMap<String, Vec<(u32, &PreviewPart)>> = BTreeMap::new();
    for message in messages {
        if let Some(part) = message.structure.as_ref().and_then(|s| s.preview.as_ref()) {
            groups
                .entry(part.fetch_item())
                .or_default()
                .push((message.uid, part));
        }
    }
    let mut previews = HashMap::new();
    for (item, members) in groups {
        let uids: Vec<u32> = members.iter().map(|(uid, _)| *uid).collect();
        let parts: HashMap<u32, &PreviewPart> = members.into_iter().collect();
        let fetched = match session
            .uid_fetch(&uid_set(&uids), &format!("(UID {item})"))
            .await
        {
            Ok(fetched) => fetched,
            // Previews are optional; the summaries are still worth having.
            Err(e) if is_unparsable(&e, session) => {
                tracing::warn!("fetching previews with {item} failed ({e}); skipping them");
                recover(session, target).await?;
                continue;
            }
            Err(e) => return Err(e),
        };
        for message in fetched {
            if let (Some(part), Some(data)) = (parts.get(&message.uid), &message.body) {
                previews.insert(message.uid, mime::decode_preview(part, data));
            }
        }
    }
    Ok(previews)
}

/// A response the parser rejected: the server is reachable but the
/// connection is unusable from here on.
fn is_unparsable(error: &Error, session: &ImapSession) -> bool {
    error.code == ErrorCode::Provider && !session.is_healthy()
}

/// Replaces a connection broken by an unparsable response and selects the
/// folder again so the sync can continue.
async fn recover(session: &mut ImapSession, target: &Target<'_>) -> Result<()> {
    session.reconnect().await?;
    let info = session.select(&target.folder.remote_id, false).await?;
    if info.uid_validity != target.uid_validity {
        return Err(Error::provider(
            "The folder changed on the server during sync; it will be synced again.",
        ));
    }
    Ok(())
}

fn summary(target: &Target<'_>, message: FetchedMessage, preview: String) -> MessageSummary {
    let headers = mime::parse_headers(message.header.as_deref().unwrap_or_default());
    let remote_id = target.remote_id(message.uid);
    let folder = target.folder;
    MessageSummary {
        id: local_id(&folder.account_id, EntityKind::Message, &remote_id),
        account_id: folder.account_id.clone(),
        folder_id: folder.id.clone(),
        remote_id,
        conversation_id: headers.conversation_id(),
        subject: headers.subject,
        from: headers.from,
        to: headers.to,
        cc: headers.cc,
        preview,
        received_at: message
            .internal_date
            .or(headers.date)
            .unwrap_or_else(Utc::now),
        is_read: message.flags.seen,
        is_flagged: message.flags.flagged,
        is_draft: message.flags.draft,
        has_attachments: message
            .structure
            .map_or(headers.is_mixed, |s| s.has_attachments),
        importance: headers.importance,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_round_trips() {
        let cursor = Cursor {
            uid_validity: 1_700_000_000,
            uid_next: 4242,
            highest_modseq: 98_765_432_101,
            low_uid: 3900,
            exists: 340,
        };
        let encoded = cursor.encode();
        assert_eq!(encoded, "v1:1700000000:4242:98765432101:3900:340");
        assert_eq!(Cursor::parse(&encoded), Some(cursor));
    }

    #[test]
    fn cursor_rejects_foreign_or_damaged_state() {
        assert_eq!(Cursor::parse("v1:1:2:3:4"), None);
        assert_eq!(Cursor::parse("v1:1:2:3:4:5:6"), None);
        assert_eq!(Cursor::parse("v2:1:2:3:4:5"), None);
        assert_eq!(Cursor::parse("v1:x:2:3:4:5"), None);
        assert_eq!(Cursor::parse("v1:99999999999:2:3:4:5"), None);
        // A Graph delta link must not be mistaken for an IMAP cursor.
        assert_eq!(
            Cursor::parse("https://graph.microsoft.com/v1.0/delta"),
            None
        );
    }

    #[test]
    fn keeps_newest_uids() {
        let mut uids = vec![5, 1, 9, 3, 9, 7];
        newest_first(&mut uids, 3);
        assert_eq!(uids, [9, 7, 5]);
    }

    #[test]
    fn formats_search_dates() {
        let date = DateTime::parse_from_rfc3339("2024-03-05T23:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(imap_date(date), "5-Mar-2024");
    }
}
