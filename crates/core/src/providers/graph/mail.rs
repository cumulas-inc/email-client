//! Mail on Microsoft Graph: folders, delta sync, message bodies, flags,
//! moves and sending.

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chrono::{DateTime, SecondsFormat, Utc};
use futures::stream::{self, StreamExt, TryStreamExt};
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};

use super::GraphProvider;
use super::client::{
    BatchRequest, BatchResponse, Failure, GraphResult, IdOnly, MAX_PAGES, PREFER_HTML_BODY,
    PREFER_IMMUTABLE_ID, PREFER_PAGE_SIZE, Page, segment, take_response,
};
use super::types::{ItemBody, Recipient, addresses, null_default, recipients_json};
use crate::error::{Error, Result};
use crate::html::make_preview;
use crate::model::{
    AttachmentMeta, EntityKind, Folder, FolderRole, Importance, Message, MessageSummary,
    OutgoingMessage, ReplyKind, local_id,
};
use crate::providers::{
    FolderCounts, MailProvider, MessageBatch, MessageContent, MessageLocator, SearchPage,
    SyncWindow,
};
use crate::search::{SearchQuery, kql};

const FOLDER_FIELDS: &str =
    "id,displayName,parentFolderId,childFolderCount,unreadItemCount,totalItemCount";
const FOLDER_COUNT_FIELDS: &str = "unreadItemCount,totalItemCount";
const SUMMARY_FIELDS: &str = "id,conversationId,subject,from,sender,toRecipients,ccRecipients,\
     bodyPreview,receivedDateTime,isRead,flag,isDraft,hasAttachments,importance";
const DETAIL_FIELDS: &str =
    "body,bccRecipients,replyTo,internetMessageId,internetMessageHeaders,hasAttachments";
const ATTACHMENT_FIELDS: &str = "id,name,contentType,size,isInline";

/// Well-known folder names (Graph `mailFolder` docs) and the role they give.
const WELL_KNOWN_FOLDERS: &[(&str, FolderRole)] = &[
    ("inbox", FolderRole::Inbox),
    ("drafts", FolderRole::Drafts),
    ("sentitems", FolderRole::Sent),
    ("archive", FolderRole::Archive),
    ("junkemail", FolderRole::Junk),
    ("deleteditems", FolderRole::Trash),
    ("outbox", FolderRole::Outbox),
];

/// Outlook allows four concurrent requests per mailbox.
const CONCURRENCY: usize = 4;
/// Removals a delta sync looks up to tell moves from deletions (three JSON
/// batches); a bulk delete doesn't fan out into hundreds of lookups.
const MAX_MOVE_LOOKUPS: usize = 60;
const PREVIEW_CHARS: usize = 200;

/// Inline images up to this size are embedded as `data:` URIs.
const MAX_INLINE_IMAGE_BYTES: u64 = 2 * 1024 * 1024;
/// Total size of embedded images per message.
const MAX_INLINE_TOTAL_BYTES: u64 = 10 * 1024 * 1024;
/// Images fetched per message (one batch).
const MAX_INLINE_IMAGES: usize = 20;

/// `sendMail` and attachment POSTs are limited to 4 MB per request, and
/// base64 adds a third; bigger messages go through a draft.
const SEND_MAIL_MAX_BYTES: usize = 3 * 1024 * 1024;
/// Attachments of this size and more need an upload session.
const UPLOAD_SESSION_THRESHOLD: usize = 3 * 1024 * 1024;
/// Upload-session chunks must stay under 4 MB.
const UPLOAD_CHUNK_BYTES: usize = 3 * 1024 * 1024;
/// Largest attachment Outlook accepts.
const MAX_ATTACHMENT_BYTES: usize = 150 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphFolder {
    id: String,
    display_name: Option<String>,
    parent_folder_id: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    child_folder_count: u32,
    #[serde(default, deserialize_with = "null_default")]
    unread_item_count: u32,
    #[serde(default, deserialize_with = "null_default")]
    total_item_count: u32,
}

/// A message as returned by list and delta queries (summary fields only).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphMessage {
    id: String,
    /// Present on delta items that left the folder (deleted or moved).
    #[serde(rename = "@removed")]
    removed: Option<Value>,
    conversation_id: Option<String>,
    subject: Option<String>,
    from: Option<Recipient>,
    sender: Option<Recipient>,
    #[serde(default, deserialize_with = "null_default")]
    to_recipients: Vec<Recipient>,
    #[serde(default, deserialize_with = "null_default")]
    cc_recipients: Vec<Recipient>,
    body_preview: Option<String>,
    received_date_time: Option<DateTime<Utc>>,
    is_read: Option<bool>,
    flag: Option<GraphFlag>,
    is_draft: Option<bool>,
    has_attachments: Option<bool>,
    importance: Option<String>,
    /// Only where asked for.
    parent_folder_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphFlag {
    flag_status: Option<String>,
}

impl GraphMessage {
    /// Delta items may carry only the properties that changed; such items
    /// lack `receivedDateTime` and are fetched again in full.
    fn is_complete(&self) -> bool {
        self.received_date_time.is_some()
    }

    fn into_summary(self, account_id: &str, folder_id: &str) -> Option<MessageSummary> {
        let received_at = self.received_date_time?;
        let from = self
            .from
            .as_ref()
            .and_then(Recipient::to_model)
            .or_else(|| self.sender.as_ref().and_then(Recipient::to_model));
        Some(MessageSummary {
            id: local_id(account_id, EntityKind::Message, &self.id),
            account_id: account_id.to_owned(),
            folder_id: folder_id.to_owned(),
            remote_id: self.id,
            conversation_id: self.conversation_id,
            subject: self.subject.unwrap_or_default(),
            from,
            to: addresses(&self.to_recipients),
            cc: addresses(&self.cc_recipients),
            preview: self
                .body_preview
                .map(|p| make_preview(&p, PREVIEW_CHARS))
                .unwrap_or_default(),
            received_at,
            is_read: self.is_read.unwrap_or(false),
            is_flagged: self
                .flag
                .and_then(|f| f.flag_status)
                .is_some_and(|s| s == "flagged"),
            is_draft: self.is_draft.unwrap_or(false),
            has_attachments: self.has_attachments.unwrap_or(false),
            importance: Importance::parse(self.importance.as_deref().unwrap_or("normal")),
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphMessageDetail {
    body: Option<ItemBody>,
    #[serde(default, deserialize_with = "null_default")]
    bcc_recipients: Vec<Recipient>,
    #[serde(default, deserialize_with = "null_default")]
    reply_to: Vec<Recipient>,
    internet_message_id: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    internet_message_headers: Vec<MessageHeader>,
    has_attachments: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct MessageHeader {
    name: String,
    value: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphAttachment {
    #[serde(rename = "@odata.type")]
    odata_type: Option<String>,
    id: String,
    name: Option<String>,
    content_type: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    size: u64,
    #[serde(default, deserialize_with = "null_default")]
    is_inline: bool,
    content_id: Option<String>,
    content_bytes: Option<String>,
}

impl GraphAttachment {
    fn to_meta(&self) -> AttachmentMeta {
        let is_item = self
            .odata_type
            .as_deref()
            .is_some_and(|t| t.ends_with("itemAttachment"));
        let content_type = match self.content_type.as_deref().map(str::trim) {
            Some(t) if !t.is_empty() => t.to_owned(),
            // Attached Outlook items download as MIME.
            _ if is_item => "message/rfc822".into(),
            _ => "application/octet-stream".into(),
        };
        AttachmentMeta {
            id: self.id.clone(),
            name: self
                .name
                .clone()
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| "attachment".into()),
            content_type,
            size: self.size,
            is_inline: self.is_inline,
            content_id: self.content_id.as_deref().map(normalize_cid),
        }
    }
}

/// Content-ID without the angle brackets some senders keep.
fn normalize_cid(cid: &str) -> String {
    cid.trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .to_owned()
}

/// MIME type for embedding an attachment as an image, if it is one. SVG is
/// left out: it can carry scripts.
fn image_mime(meta: &AttachmentMeta) -> Option<&'static str> {
    let by_type = match meta.content_type.to_ascii_lowercase().as_str() {
        "image/png" => Some("image/png"),
        "image/jpeg" | "image/jpg" | "image/pjpeg" => Some("image/jpeg"),
        "image/gif" => Some("image/gif"),
        "image/webp" => Some("image/webp"),
        "image/bmp" => Some("image/bmp"),
        _ => None,
    };
    by_type.or_else(|| {
        let ext = meta.name.rsplit_once('.')?.1.to_ascii_lowercase();
        match ext.as_str() {
            "png" => Some("image/png"),
            "jpg" | "jpeg" => Some("image/jpeg"),
            "gif" => Some("image/gif"),
            "webp" => Some("image/webp"),
            "bmp" => Some("image/bmp"),
            _ => None,
        }
    })
}

/// Result of a delta query, all pages read.
struct Delta {
    items: Vec<GraphMessage>,
    delta_link: String,
}

#[derive(Default)]
struct Changes {
    upserts: Vec<MessageSummary>,
    removed: Vec<String>,
    moved_out: Vec<MessageSummary>,
}

/// Delta tokens expire (410 Gone, or a `syncStateNotFound`-style code); the
/// folder must then be synced from scratch.
fn is_expired_sync_state(status: StatusCode, code: &str) -> bool {
    status == StatusCode::GONE
        || matches!(
            code.to_ascii_lowercase().as_str(),
            "syncstatenotfound"
                | "syncstateinvalid"
                | "resyncrequired"
                | "errorinvalidsyncstatedata"
        )
}

/// Keeps the last occurrence of each id (an item can show up on several
/// pages when it changes while the delta is being read).
fn collapse(items: Vec<GraphMessage>) -> Vec<GraphMessage> {
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut out: Vec<GraphMessage> = Vec::with_capacity(items.len());
    for item in items {
        match index.get(&item.id) {
            Some(&at) => out[at] = item,
            None => {
                index.insert(item.id.clone(), out.len());
                out.push(item);
            }
        }
    }
    out
}

/// An outgoing attachment, decoded.
struct Upload {
    name: String,
    content_type: String,
    base64: String,
    data: Vec<u8>,
}

fn decode_attachments(message: &OutgoingMessage) -> Result<Vec<Upload>> {
    message
        .attachments
        .iter()
        .map(|a| {
            let base64: String = a
                .data_base64
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            let data = STANDARD.decode(&base64).map_err(|_| {
                Error::invalid_input(format!("The attachment \"{}\" could not be read.", a.name))
            })?;
            if data.len() > MAX_ATTACHMENT_BYTES {
                return Err(Error::invalid_input(format!(
                    "\"{}\" is larger than 150 MB, the most Microsoft 365 accepts.",
                    a.name
                )));
            }
            Ok(Upload {
                name: a.name.clone(),
                content_type: match a.content_type.trim() {
                    "" => "application/octet-stream".into(),
                    t => t.to_owned(),
                },
                base64,
                data,
            })
        })
        .collect()
}

fn message_json(message: &OutgoingMessage) -> Value {
    let mut json = json!({
        "subject": message.subject,
        "body": { "contentType": "HTML", "content": message.body_html },
        "toRecipients": recipients_json(&message.to),
        "ccRecipients": recipients_json(&message.cc),
        "bccRecipients": recipients_json(&message.bcc),
    });
    if let Some(importance) = message.importance {
        json["importance"] = importance.as_str().into();
    }
    json
}

fn file_attachment_json(upload: &Upload) -> Value {
    json!({
        "@odata.type": "#microsoft.graph.fileAttachment",
        "name": upload.name,
        "contentType": upload.content_type,
        "contentBytes": upload.base64,
    })
}

impl GraphProvider {
    fn message_url(&self, remote_id: &str, suffix: &str) -> String {
        self.client
            .url(&format!("/me/messages/{}{suffix}", segment(remote_id)))
    }

    /// Resolves the well-known folders to their ids in one batch.
    async fn well_known_folders(&self) -> Result<HashMap<String, FolderRole>> {
        let requests: Vec<_> = WELL_KNOWN_FOLDERS
            .iter()
            .map(|(name, _)| BatchRequest::get(name, format!("/me/mailFolders/{name}?$select=id")))
            .collect();
        let mut responses = self.client.batch(&requests, &[PREFER_IMMUTABLE_ID]).await?;
        let mut roles = HashMap::new();
        for (name, role) in WELL_KNOWN_FOLDERS {
            let response = take_response(&mut responses, name)?;
            // Mailboxes do not always have every well-known folder (Archive).
            if response.status() == StatusCode::NOT_FOUND {
                continue;
            }
            if !response.is_success() {
                return Err(response.error());
            }
            let folder: IdOnly = response.json()?;
            roles.entry(folder.id).or_insert(*role);
        }
        Ok(roles)
    }

    /// Reads every page of a delta query up to its delta link.
    async fn read_delta(&self, url: &str) -> GraphResult<Delta> {
        let prefer = [PREFER_IMMUTABLE_ID, PREFER_PAGE_SIZE];
        let mut items = Vec::new();
        let mut next = url.to_owned();
        for _ in 0..MAX_PAGES {
            let page: Page<GraphMessage> = self.client.get_page(&next, &prefer).await?;
            items.extend(page.value);
            match (page.next_link, page.delta_link) {
                (Some(link), _) => next = link,
                (None, Some(delta_link)) => {
                    return Ok(Delta {
                        items: collapse(items),
                        delta_link,
                    });
                }
                (None, None) => {
                    return Err(Failure::Other(Error::provider(
                        "Microsoft Graph ended a delta query without a delta link.",
                    )));
                }
            }
        }
        Err(Failure::Other(Error::provider(
            "Microsoft Graph returned too many pages.",
        )))
    }

    /// Splits delta items into summaries and removals, fetching items that
    /// came with only their changed properties.
    async fn resolve(&self, folder: &Folder, items: Vec<GraphMessage>) -> Result<Changes> {
        let mut changes = Changes::default();
        let mut partial = Vec::new();
        for item in items {
            if item.removed.is_some() {
                changes.removed.push(item.id);
            } else if !item.is_complete() {
                partial.push(item.id);
            } else if let Some(summary) = item.into_summary(&self.account_id, &folder.id) {
                changes.upserts.push(summary);
            }
        }
        if partial.is_empty() {
            return Ok(changes);
        }
        let requests: Vec<_> = partial
            .iter()
            .enumerate()
            .map(|(i, id)| {
                BatchRequest::get(
                    i,
                    format!("/me/messages/{}?$select={SUMMARY_FIELDS}", segment(id)),
                )
            })
            .collect();
        let mut responses = self.client.batch(&requests, &[PREFER_IMMUTABLE_ID]).await?;
        for (i, id) in partial.into_iter().enumerate() {
            let response = take_response(&mut responses, &i.to_string())?;
            if response.status() == StatusCode::NOT_FOUND {
                changes.removed.push(id);
                continue;
            }
            if !response.is_success() {
                return Err(response.error());
            }
            let message: GraphMessage = response.json()?;
            if let Some(summary) = message.into_summary(&self.account_id, &folder.id) {
                changes.upserts.push(summary);
            }
        }
        Ok(changes)
    }

    /// Looks up the messages a delta reported removed from `folder`: delta
    /// says "deleted" for a move out too. One found in another folder moved
    /// there, one found in this folder is back, one not found is gone.
    /// Removals beyond [`MAX_MOVE_LOOKUPS`], or that can't be looked up,
    /// stay removals; the store only removes those still in this folder.
    async fn find_moves(&self, folder: &Folder, changes: &mut Changes) {
        let lookups: Vec<String> = changes
            .removed
            .iter()
            .take(MAX_MOVE_LOOKUPS)
            .cloned()
            .collect();
        if lookups.is_empty() {
            return;
        }
        let requests: Vec<_> = lookups
            .iter()
            .enumerate()
            .map(|(i, id)| {
                BatchRequest::get(
                    i,
                    format!(
                        "/me/messages/{}?$select={SUMMARY_FIELDS},parentFolderId",
                        segment(id)
                    ),
                )
            })
            .collect();
        let mut responses = match self.client.batch(&requests, &[PREFER_IMMUTABLE_ID]).await {
            Ok(responses) => responses,
            Err(e) => {
                tracing::debug!("looking up removed messages failed: {}", Error::from(e));
                return;
            }
        };
        let mut removed = Vec::new();
        for (i, id) in lookups.into_iter().enumerate() {
            let found = responses
                .remove(&i.to_string())
                .filter(|r| {
                    if !r.is_success() && r.status() != StatusCode::NOT_FOUND {
                        tracing::debug!("looking up a removed message returned {}", r.status());
                    }
                    r.is_success()
                })
                .and_then(|r| r.json::<GraphMessage>().ok());
            let Some(message) = found else {
                removed.push(id);
                continue;
            };
            let in_folder = match message.parent_folder_id.as_deref() {
                Some(parent) if parent == folder.remote_id => folder.id.clone(),
                Some(parent) => local_id(&self.account_id, EntityKind::Folder, parent),
                None => {
                    removed.push(id);
                    continue;
                }
            };
            match message.into_summary(&self.account_id, &in_folder) {
                Some(summary) if in_folder == folder.id => changes.upserts.push(summary),
                Some(summary) => changes.moved_out.push(summary),
                None => removed.push(id),
            }
        }
        removed.extend(changes.removed.drain(..).skip(MAX_MOVE_LOOKUPS));
        changes.removed = removed;
    }

    /// One page of a search of the whole mailbox; see
    /// [`MailProvider::search`]. `cursor` is Graph's `@odata.nextLink`.
    async fn search_page(
        &self,
        query: &SearchQuery,
        folders: &[Folder],
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<SearchPage> {
        let url = match cursor {
            Some(link) => link.to_owned(),
            None => {
                let Some(kql) = kql::kql(query) else {
                    return Ok(SearchPage::default());
                };
                // Graph refuses `$orderby` and `$filter` with `$search`; it
                // sorts by date sent itself.
                self.client.url_with_query(
                    "/me/messages",
                    &[
                        ("$search", &kql::search_param(&kql)),
                        ("$select", &format!("{SUMMARY_FIELDS},parentFolderId")),
                        ("$top", &limit.to_string()),
                    ],
                )
            }
        };
        // Refuses a cursor that isn't a link to Graph, so the token stays.
        let page: Page<GraphMessage> = self.client.get_page(&url, &[PREFER_IMMUTABLE_ID]).await?;
        let stored: HashSet<&str> = folders.iter().map(|f| f.remote_id.as_str()).collect();
        let mut unknown: Vec<&str> = page
            .value
            .iter()
            .filter_map(|m| m.parent_folder_id.as_deref())
            .filter(|id| !stored.contains(id))
            .collect();
        unknown.sort_unstable();
        unknown.dedup();
        let aliases = self.stored_folder_aliases(&unknown, &stored).await;
        let mut messages: Vec<MessageSummary> = Vec::with_capacity(page.value.len());
        for message in page.value {
            let Some(parent) = message.parent_folder_id.clone() else {
                continue;
            };
            let parent = aliases.get(&parent).cloned().unwrap_or(parent);
            let folder_id = local_id(&self.account_id, EntityKind::Folder, &parent);
            messages.extend(message.into_summary(&self.account_id, &folder_id));
        }
        messages.sort_by_key(|m| std::cmp::Reverse(m.received_at));
        Ok(SearchPage {
            messages,
            next: page.next_link,
        })
    }

    /// Of folder ids that aren't the stored folders' own, those that name a
    /// stored folder another way, mapped to its id: asked of Graph, which
    /// answers with the canonical id. Lookups that fail map nothing.
    async fn stored_folder_aliases(
        &self,
        ids: &[&str],
        stored: &HashSet<&str>,
    ) -> HashMap<String, String> {
        let mut aliases = HashMap::new();
        if ids.is_empty() {
            return aliases;
        }
        let requests: Vec<_> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| {
                BatchRequest::get(i, format!("/me/mailFolders/{}?$select=id", segment(id)))
            })
            .collect();
        let mut responses = match self.client.batch(&requests, &[PREFER_IMMUTABLE_ID]).await {
            Ok(responses) => responses,
            Err(e) => {
                tracing::debug!(
                    "looking up folders of search results failed: {}",
                    Error::from(e)
                );
                return aliases;
            }
        };
        for (i, id) in ids.iter().enumerate() {
            if let Some(folder) = responses
                .remove(&i.to_string())
                .filter(BatchResponse::is_success)
                .and_then(|r| r.json::<IdOnly>().ok())
                && folder.id != *id
                && stored.contains(folder.id.as_str())
            {
                aliases.insert((*id).to_owned(), folder.id);
            }
        }
        aliases
    }

    async fn full_sync(&self, folder: &Folder, window: SyncWindow) -> Result<MessageBatch> {
        let since = window.since.to_rfc3339_opts(SecondsFormat::Secs, true);
        let url = self.client.url_with_query(
            &format!(
                "/me/mailFolders/{}/messages/delta",
                segment(&folder.remote_id)
            ),
            &[
                ("$select", SUMMARY_FIELDS),
                ("$filter", &format!("receivedDateTime ge {since}")),
            ],
        );
        let delta = self.read_delta(&url).await?;
        let mut upserts = self.resolve(folder, delta.items).await?.upserts;
        // The whole window has to be paged through to reach the delta link
        // anyway; the cap keeps the newest messages.
        upserts.sort_by_key(|m| std::cmp::Reverse(m.received_at));
        upserts.truncate(window.max_messages as usize);
        Ok(MessageBatch {
            upserts,
            full_resync: true,
            sync_state: Some(delta.delta_link),
            ..Default::default()
        })
    }

    async fn list_attachments(&self, message_id: &str) -> Result<Vec<AttachmentMeta>> {
        let url = self.client.url_with_query(
            &format!("/me/messages/{}/attachments", segment(message_id)),
            &[("$select", ATTACHMENT_FIELDS)],
        );
        let attachments: Vec<GraphAttachment> =
            self.client.get_all(&url, &[PREFER_IMMUTABLE_ID]).await?;
        Ok(attachments.iter().map(GraphAttachment::to_meta).collect())
    }

    /// Replaces `cid:` references with `data:` URIs of the matching image
    /// attachments. The attachment listing does not include Content-IDs, so
    /// candidate images (inline ones first) are fetched in one batch. Images
    /// that cannot be fetched simply stay unresolved.
    async fn embed_inline_images(
        &self,
        message_id: &str,
        html: &mut String,
        attachments: &mut [AttachmentMeta],
    ) {
        let mut candidates: Vec<usize> = (0..attachments.len())
            .filter(|&i| {
                attachments[i].size <= MAX_INLINE_IMAGE_BYTES
                    && image_mime(&attachments[i]).is_some()
            })
            .collect();
        candidates.sort_by_key(|&i| !attachments[i].is_inline);
        let mut budget = MAX_INLINE_TOTAL_BYTES;
        candidates.retain(|&i| {
            let fits = attachments[i].size <= budget;
            if fits {
                budget -= attachments[i].size;
            }
            fits
        });
        candidates.truncate(MAX_INLINE_IMAGES);
        if candidates.is_empty() {
            return;
        }

        let requests: Vec<_> = candidates
            .iter()
            .map(|&i| {
                BatchRequest::get(
                    i,
                    format!(
                        "/me/messages/{}/attachments/{}",
                        segment(message_id),
                        segment(&attachments[i].id)
                    ),
                )
            })
            .collect();
        let mut responses = match self.client.batch(&requests, &[PREFER_IMMUTABLE_ID]).await {
            Ok(responses) => responses,
            Err(e) => {
                tracing::warn!("could not fetch inline images: {}", Error::from(e));
                return;
            }
        };
        for i in candidates {
            let Some(part) = responses
                .remove(&i.to_string())
                .filter(BatchResponse::is_success)
                .and_then(|r| r.json::<GraphAttachment>().ok())
            else {
                continue;
            };
            let (Some(cid), Some(bytes)) = (
                part.content_id.as_deref().map(normalize_cid),
                part.content_bytes,
            ) else {
                continue;
            };
            let meta = &mut attachments[i];
            let reference = format!("cid:{cid}");
            if !cid.is_empty()
                && html.contains(&reference)
                && let Some(mime) = image_mime(meta)
            {
                *html = html.replace(&reference, &format!("data:{mime};base64,{bytes}"));
                meta.is_inline = true;
            }
            meta.content_id = Some(cid);
        }
    }

    /// Runs one request per message (`/me/messages/{id}{suffix}`) in batches
    /// and returns the responses in input order; `None` for messages that no
    /// longer exist when `missing_ok`.
    async fn per_message(
        &self,
        messages: &[MessageLocator],
        method: &'static str,
        suffix: &str,
        body: Option<&Value>,
        missing_ok: bool,
    ) -> Result<Vec<Option<BatchResponse>>> {
        let requests: Vec<_> = messages
            .iter()
            .enumerate()
            .map(|(i, m)| {
                BatchRequest::new(
                    i,
                    method,
                    format!("/me/messages/{}{suffix}", segment(&m.remote_id)),
                    body.cloned(),
                )
            })
            .collect();
        let mut responses = self.client.batch(&requests, &[PREFER_IMMUTABLE_ID]).await?;
        (0..messages.len())
            .map(|i| {
                let response = take_response(&mut responses, &i.to_string())?;
                if response.is_success() {
                    Ok(Some(response))
                } else if missing_ok && response.status() == StatusCode::NOT_FOUND {
                    Ok(None)
                } else {
                    Err(response.error())
                }
            })
            .collect()
    }

    /// Creates the draft to send: a reply/forward of `reply` (which carries
    /// the threading headers and, for forwards, the original attachments)
    /// updated with our subject, recipients and body, or a new message.
    async fn create_draft(
        &self,
        message: &OutgoingMessage,
        reply: Option<(ReplyKind, &Message)>,
    ) -> Result<String> {
        let prefer = [PREFER_IMMUTABLE_ID];
        let payload = message_json(message);
        let Some((kind, original)) = reply else {
            let draft: IdOnly = self
                .client
                .send_json(
                    Method::POST,
                    &self.client.url("/me/messages"),
                    &payload,
                    &prefer,
                )
                .await?;
            return Ok(draft.id);
        };
        let action = match kind {
            ReplyKind::Reply => "/createReply",
            ReplyKind::ReplyAll => "/createReplyAll",
            ReplyKind::Forward => "/createForward",
        };
        let draft: IdOnly = self
            .client
            .send_json(
                Method::POST,
                &self.message_url(&original.summary.remote_id, action),
                &json!({}),
                &prefer,
            )
            .await?;
        // The UI's body already quotes the original, so it replaces the
        // generated one.
        if let Err(e) = self
            .client
            .send_no_content(
                Method::PATCH,
                &self.message_url(&draft.id, ""),
                Some(&payload),
                &prefer,
            )
            .await
        {
            self.discard_draft(&draft.id).await;
            return Err(e.into());
        }
        Ok(draft.id)
    }

    async fn add_attachments(&self, draft_id: &str, uploads: &[Upload]) -> Result<()> {
        for upload in uploads {
            if upload.data.len() < UPLOAD_SESSION_THRESHOLD {
                self.client
                    .send_no_content(
                        Method::POST,
                        &self.message_url(draft_id, "/attachments"),
                        Some(&file_attachment_json(upload)),
                        &[PREFER_IMMUTABLE_ID],
                    )
                    .await?;
            } else {
                self.upload_attachment(draft_id, upload).await?;
            }
        }
        Ok(())
    }

    /// Uploads a large attachment in chunks through an upload session.
    async fn upload_attachment(&self, draft_id: &str, upload: &Upload) -> Result<()> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct UploadSession {
            upload_url: String,
        }
        let session: UploadSession = self
            .client
            .send_json(
                Method::POST,
                &self.message_url(draft_id, "/attachments/createUploadSession"),
                &json!({
                    "AttachmentItem": {
                        "attachmentType": "file",
                        "name": upload.name,
                        "size": upload.data.len(),
                        "contentType": upload.content_type,
                    }
                }),
                &[PREFER_IMMUTABLE_ID],
            )
            .await?;
        let total = upload.data.len();
        for (index, chunk) in upload.data.chunks(UPLOAD_CHUNK_BYTES).enumerate() {
            let start = index * UPLOAD_CHUNK_BYTES;
            let range = format!("bytes {start}-{}/{total}", start + chunk.len() - 1);
            self.client
                .put_chunk(&session.upload_url, chunk, &range)
                .await?;
        }
        Ok(())
    }

    /// Best-effort removal of a draft that could not be completed.
    async fn discard_draft(&self, draft_id: &str) {
        if let Err(e) = self
            .client
            .send_no_content(
                Method::DELETE,
                &self.message_url(draft_id, ""),
                None,
                &[PREFER_IMMUTABLE_ID],
            )
            .await
        {
            tracing::warn!("could not delete an unsent draft: {}", Error::from(e));
        }
    }
}

#[async_trait]
impl MailProvider for GraphProvider {
    async fn list_folders(&self) -> Result<Vec<Folder>> {
        let prefer: &[&str] = &[PREFER_IMMUTABLE_ID];
        let query = [("$select", FOLDER_FIELDS), ("$top", "100")];
        let mut folders: Vec<GraphFolder> = self
            .client
            .get_all(
                &self.client.url_with_query("/me/mailFolders", &query),
                prefer,
            )
            .await?;
        let mut seen: HashSet<String> = folders.iter().map(|f| f.id.clone()).collect();
        let mut parents: Vec<String> = folders
            .iter()
            .filter(|f| f.child_folder_count > 0)
            .map(|f| f.id.clone())
            .collect();
        while !parents.is_empty() {
            let urls: Vec<String> = parents
                .drain(..)
                .map(|id| {
                    self.client.url_with_query(
                        &format!("/me/mailFolders/{}/childFolders", segment(&id)),
                        &query,
                    )
                })
                .collect();
            let children: Vec<Vec<GraphFolder>> = stream::iter(urls)
                .map(|url| async move { self.client.get_all(&url, prefer).await })
                .buffered(CONCURRENCY)
                .try_collect()
                .await?;
            for child in children.into_iter().flatten() {
                if seen.insert(child.id.clone()) {
                    if child.child_folder_count > 0 {
                        parents.push(child.id.clone());
                    }
                    folders.push(child);
                }
            }
        }

        let roles = self.well_known_folders().await?;
        Ok(folders
            .into_iter()
            .map(|f| Folder {
                id: local_id(&self.account_id, EntityKind::Folder, &f.id),
                account_id: self.account_id.clone(),
                // Top-level folders hang off the hidden root, which is not
                // listed; they have no parent here.
                parent_id: f
                    .parent_folder_id
                    .filter(|p| seen.contains(p))
                    .map(|p| local_id(&self.account_id, EntityKind::Folder, &p)),
                name: f.display_name.unwrap_or_default(),
                role: roles.get(&f.id).copied().unwrap_or(FolderRole::Other),
                unread_count: f.unread_item_count,
                total_count: f.total_item_count,
                remote_id: f.id,
            })
            .collect())
    }

    async fn sync_messages(
        &self,
        folder: &Folder,
        sync_state: Option<&str>,
        window: SyncWindow,
    ) -> Result<MessageBatch> {
        if let Some(link) = sync_state.filter(|l| self.client.is_own_link(l)) {
            match self.read_delta(link).await {
                Ok(delta) => {
                    let mut changes = self.resolve(folder, delta.items).await?;
                    self.find_moves(folder, &mut changes).await;
                    return Ok(MessageBatch {
                        upserts: changes.upserts,
                        removed_remote_ids: changes.removed,
                        moved_out: changes.moved_out,
                        sync_state: Some(delta.delta_link),
                        ..Default::default()
                    });
                }
                Err(Failure::Graph { status, code, .. })
                    if is_expired_sync_state(status, &code) =>
                {
                    tracing::info!(
                        "delta token of folder {} expired ({status} {code}); resyncing",
                        folder.name
                    );
                }
                Err(e) => return Err(e.into()),
            }
        }
        self.full_sync(folder, window).await
    }

    async fn folder_counts(&self, folder: &Folder) -> Result<Option<FolderCounts>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Counts {
            #[serde(default, deserialize_with = "null_default")]
            unread_item_count: u32,
            #[serde(default, deserialize_with = "null_default")]
            total_item_count: u32,
        }
        let url = self.client.url_with_query(
            &format!("/me/mailFolders/{}", segment(&folder.remote_id)),
            &[("$select", FOLDER_COUNT_FIELDS)],
        );
        let counts: Counts = self.client.get_json(&url, &[PREFER_IMMUTABLE_ID]).await?;
        Ok(Some(FolderCounts {
            unread: counts.unread_item_count,
            total: counts.total_item_count,
        }))
    }

    async fn fetch_message(&self, message: &MessageLocator) -> Result<MessageContent> {
        let url = self.client.url_with_query(
            &format!("/me/messages/{}", segment(&message.remote_id)),
            &[("$select", DETAIL_FIELDS)],
        );
        let detail: GraphMessageDetail = self
            .client
            .get_json(&url, &[PREFER_IMMUTABLE_ID, PREFER_HTML_BODY])
            .await?;
        let (mut body_html, body_text) = match detail.body {
            Some(body) if body.is_html() => (body.content, None),
            Some(body) => (None, body.content),
            None => (None, None),
        };
        let has_cid = body_html.as_deref().is_some_and(|h| h.contains("cid:"));
        // `hasAttachments` ignores inline attachments, hence the `cid:` check.
        let mut attachments = if detail.has_attachments.unwrap_or(false) || has_cid {
            self.list_attachments(&message.remote_id).await?
        } else {
            Vec::new()
        };
        if has_cid && let Some(html) = body_html.as_mut() {
            self.embed_inline_images(&message.remote_id, html, &mut attachments)
                .await;
        }
        let header = |name: &str| {
            detail
                .internet_message_headers
                .iter()
                .find(|h| h.name.eq_ignore_ascii_case(name))
                .map(|h| h.value.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        Ok(MessageContent {
            in_reply_to: header("In-Reply-To"),
            references: header("References")
                .map(|r| r.split_whitespace().map(str::to_owned).collect())
                .unwrap_or_default(),
            body_html,
            body_text,
            bcc: addresses(&detail.bcc_recipients),
            reply_to: addresses(&detail.reply_to),
            attachments,
            internet_message_id: detail.internet_message_id,
        })
    }

    async fn fetch_attachment(
        &self,
        message: &MessageLocator,
        attachment_id: &str,
    ) -> Result<Vec<u8>> {
        let url = self.message_url(
            &message.remote_id,
            &format!("/attachments/{}/$value", segment(attachment_id)),
        );
        Ok(self.client.get_bytes(&url, &[PREFER_IMMUTABLE_ID]).await?)
    }

    async fn set_read(&self, messages: &[MessageLocator], read: bool) -> Result<()> {
        // A message deleted on the server meanwhile needs no update.
        self.per_message(
            messages,
            "PATCH",
            "",
            Some(&json!({ "isRead": read })),
            true,
        )
        .await?;
        Ok(())
    }

    async fn set_flagged(&self, messages: &[MessageLocator], flagged: bool) -> Result<()> {
        let status = if flagged { "flagged" } else { "notFlagged" };
        self.per_message(
            messages,
            "PATCH",
            "",
            Some(&json!({ "flag": { "flagStatus": status } })),
            true,
        )
        .await?;
        Ok(())
    }

    async fn move_messages(
        &self,
        messages: &[MessageLocator],
        destination_folder_remote_id: &str,
    ) -> Result<Vec<Option<String>>> {
        let body = json!({ "destinationId": destination_folder_remote_id });
        let responses = self
            .per_message(messages, "POST", "/move", Some(&body), false)
            .await?;
        Ok(responses
            .into_iter()
            .map(|r| r.and_then(|r| r.json::<IdOnly>().ok()).map(|m| m.id))
            .collect())
    }

    async fn delete_messages(&self, messages: &[MessageLocator]) -> Result<()> {
        // Already gone counts as deleted.
        self.per_message(messages, "POST", "/permanentDelete", None, true)
            .await?;
        Ok(())
    }

    async fn search(
        &self,
        query: &SearchQuery,
        folders: &[Folder],
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<SearchPage> {
        self.search_page(query, folders, cursor, limit).await
    }

    async fn send(&self, message: &OutgoingMessage, original: Option<&Message>) -> Result<()> {
        let uploads = decode_attachments(message)?;
        let reply = message.reply_kind.zip(original);
        let inline_bytes =
            message.body_html.len() + uploads.iter().map(|u| u.base64.len()).sum::<usize>();
        if reply.is_none() && inline_bytes <= SEND_MAIL_MAX_BYTES {
            let mut payload = message_json(message);
            if !uploads.is_empty() {
                payload["attachments"] = uploads.iter().map(file_attachment_json).collect();
            }
            self.client
                .send_no_content(
                    Method::POST,
                    &self.client.url("/me/sendMail"),
                    Some(&json!({ "message": payload, "saveToSentItems": true })),
                    &[],
                )
                .await?;
            return Ok(());
        }

        let draft_id = self.create_draft(message, reply).await?;
        if let Err(e) = self.add_attachments(&draft_id, &uploads).await {
            self.discard_draft(&draft_id).await;
            return Err(e);
        }
        // Sending moves the draft to Sent Items; it must not be discarded
        // after this point even if the response is lost.
        self.client
            .send_no_content(
                Method::POST,
                &self.message_url(&draft_id, "/send"),
                None,
                &[PREFER_IMMUTABLE_ID],
            )
            .await?;
        Ok(())
    }
}
