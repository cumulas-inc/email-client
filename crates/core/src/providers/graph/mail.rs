//! Mail on Microsoft Graph: folders, delta sync, message bodies, flags,
//! moves and sending, and what protected mail needs: the signals detection
//! reads, S/MIME content, sending through a draft that carries the
//! x-cumulas-* properties, and the setup check's lookups.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::time::Duration;

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
use crate::error::{Error, ErrorCode, Result};
use crate::html::make_preview;
use crate::model::{
    AttachmentMeta, EmailAddress, EntityKind, Folder, FolderRole, Importance, Message,
    MessageSummary, OutgoingMessage, ReplyKind, local_id,
};
use crate::protection::{
    AUTH_AS_HEADER, CONTENT_CLASS_PROPERTY, DIRECTIONALITY_HEADER, LABEL_HEADER, LABEL_PROPERTY,
    LABEL_STAMPED_HEADER, MAX_TRANSPORT_HEADERS_BYTES, MESSAGE_CLASS_PROPERTY,
    MIME_ATTACHMENT_PREFIX, MSIP_LABELS_PROPERTY, PROTECT_HEADER, PROTECT_PROPERTY, STAMP_HEADER,
    TRANSPORT_HEADERS_PROPERTY, headers_not_kept, is_blank, label_copied, own_label,
    smime_content_of, wants_attachments,
};
use crate::providers::imap::mime;
use crate::providers::{
    FolderCounts, HeaderRequest, MailProvider, MessageBatch, MessageContent, MessageLocator,
    MessageSignals, SearchPage, ServerQuery, SyncWindow,
};
use crate::search::dates::Received;
use crate::search::kql;

const FOLDER_FIELDS: &str =
    "id,displayName,parentFolderId,childFolderCount,unreadItemCount,totalItemCount";
const FOLDER_COUNT_FIELDS: &str = "unreadItemCount,totalItemCount";
const SUMMARY_FIELDS: &str = "id,conversationId,subject,from,sender,toRecipients,ccRecipients,\
     bodyPreview,receivedDateTime,isRead,flag,isDraft,hasAttachments,importance";
const DETAIL_FIELDS: &str =
    "body,bccRecipients,replyTo,internetMessageId,internetMessageHeaders,hasAttachments";
const ATTACHMENT_FIELDS: &str = "id,name,contentType,size,isInline";
const INBOX_LOOKUP_FIELDS: &str = "id,subject,receivedDateTime";

/// The named properties read next to a message's detail GET, by batch
/// request id: one at a time, as an `or` in an extended-property filter
/// isn't documented.
const NAMED_PROPERTIES: [(&str, &str); 4] = [
    ("contentClass", CONTENT_CLASS_PROPERTY),
    ("msipLabels", MSIP_LABELS_PROPERTY),
    ("requestedProtect", PROTECT_PROPERTY),
    ("requestedLabel", LABEL_PROPERTY),
];
/// How long reading a draft's properties back may take before nothing is
/// sent.
const READ_BACK_BUDGET: Duration = Duration::from_secs(30);
/// Bytes of a `.p7m` read to tell signed from encrypted.
const SMIME_PREFIX_BYTES: usize = 512;
/// Largest message whose MIME is read to show a clear-signed message's
/// content.
const MAX_MIME_BYTES: usize = 25 * 1024 * 1024;
/// Pages of Inbox messages a check's lookup reads.
const MAX_INBOX_LOOKUP_PAGES: usize = 3;

/// Whether Graph's answer to setting or reading back a draft's property
/// refuses it, rather than failing to answer: the sign-in, a timeout,
/// throttling, a request that failed only because the one before it did
/// (424), or a server error say nothing of whether the draft keeps it.
fn is_refusal(status: StatusCode) -> bool {
    status.is_client_error()
        && !matches!(
            status,
            StatusCode::UNAUTHORIZED
                | StatusCode::REQUEST_TIMEOUT
                | StatusCode::FAILED_DEPENDENCY
                | StatusCode::TOO_MANY_REQUESTS
        )
}

/// Microsoft 365 didn't answer while a draft's request was read back:
/// nothing was sent, and nothing was learned of what it keeps.
fn unanswered() -> Error {
    Error::provider(
        "Microsoft 365 didn't answer while Cumulas Mail checked the message before sending it, \
         so it wasn't sent. Try again.",
    )
}

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
    /// The one expanded: the message class.
    #[serde(default, deserialize_with = "null_default")]
    single_value_extended_properties: Vec<ExtendedProperty>,
}

/// A single-value extended property. Its `id` isn't matched on: Graph may
/// spell it otherwise than it was asked for.
#[derive(Debug, Deserialize)]
struct ExtendedProperty {
    value: Option<String>,
}

/// A message with only the extended properties asked for.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WithProperties {
    #[serde(default, deserialize_with = "null_default")]
    single_value_extended_properties: Vec<ExtendedProperty>,
}

impl WithProperties {
    /// The first property's value, trimmed; `None` when the message has
    /// none (Graph then leaves the array out).
    fn value(self) -> Option<String> {
        self.single_value_extended_properties
            .into_iter()
            .next()
            .and_then(|p| p.value)
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    }
}

/// What the named-property batch read: each property's value, and whether
/// every one could be read.
#[derive(Debug, Default)]
struct NamedValues {
    values: HashMap<&'static str, String>,
    complete: bool,
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

/// `$expand` of one single-value extended property.
fn expand_property(id: &str) -> String {
    format!("singleValueExtendedProperties($filter=id eq '{id}')")
}

/// A message's named property `id`, by itself, for a batch (whose URLs
/// Graph documents unencoded).
fn property_url(message_id: &str, id: &str) -> String {
    format!(
        "/me/messages/{}?$select=id&$expand={}",
        segment(message_id),
        expand_property(id)
    )
}

/// Requests reading `properties` of a message one after the other.
fn sequential_property_reads(message_id: &str, properties: &[(&str, &str)]) -> Vec<BatchRequest> {
    let mut requests: Vec<BatchRequest> = Vec::with_capacity(properties.len());
    for (request_id, property) in properties {
        let request = BatchRequest::get(request_id, property_url(message_id, property));
        let request = match requests.last() {
            Some(previous) => request.after(&previous.id),
            None => request,
        };
        requests.push(request);
    }
    requests
}

/// A value read two ways, and which gave it: `first`'s, else `then`'s.
fn sourced(
    first: (Option<String>, &str),
    then: (Option<String>, &str),
) -> (Option<String>, Option<String>) {
    match (first, then) {
        ((Some(value), source), _) | ((None, _), (Some(value), source)) => {
            (Some(value), Some(source.to_owned()))
        }
        _ => (None, None),
    }
}

/// A link that's https, else nothing.
fn https_only(link: Option<String>) -> Option<String> {
    link.filter(|l| url::Url::parse(l).is_ok_and(|u| u.scheme() == "https"))
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
        let mut found: Vec<(String, GraphMessage)> = Vec::new();
        for (i, id) in lookups.into_iter().enumerate() {
            let message = responses
                .remove(&i.to_string())
                .filter(|r| {
                    if !r.is_success() && r.status() != StatusCode::NOT_FOUND {
                        tracing::debug!("looking up a removed message returned {}", r.status());
                    }
                    r.is_success()
                })
                .and_then(|r| r.json::<GraphMessage>().ok());
            match message {
                Some(message) => found.push((id, message)),
                None => removed.push(id),
            }
        }
        // As a search does (see `search_page`): folders the store has, named
        // another way, are mapped to the ids it knows them by. Before the
        // folders were first listed, there is nothing to map to.
        let mut known = self.folder_ids.lock().unwrap().clone();
        let listed = !known.is_empty();
        known.insert(folder.remote_id.clone());
        let mut unknown: Vec<&str> = found
            .iter()
            .filter_map(|(_, m)| m.parent_folder_id.as_deref())
            .filter(|id| listed && !known.contains(*id))
            .collect();
        unknown.sort_unstable();
        unknown.dedup();
        let known_ids: HashSet<&str> = known.iter().map(String::as_str).collect();
        let aliases = self.stored_folder_aliases(&unknown, &known_ids).await;
        for (id, message) in found {
            let parent = message
                .parent_folder_id
                .as_ref()
                .map(|p| aliases.get(p).unwrap_or(p));
            let in_folder = match parent {
                Some(parent) if *parent == folder.remote_id => folder.id.clone(),
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

    /// One page of a search of the mailbox, or of the one folder it is
    /// scoped to; see [`MailProvider::search`]. `cursor` is Graph's
    /// `@odata.nextLink`.
    ///
    /// Words go to `$search` (KQL), which takes no `$filter` or `$orderby`
    /// (Graph sorts its matches by date sent itself), with the dates when
    /// the mailbox takes them. Filters alone go to `$filter`, newest first:
    /// exact, and so it finds old unread or flagged mail `$search` can't
    /// say. Graph can't say several folders: those, read and flagged with
    /// words, and the exact dates are the caller's to check.
    async fn search_page(
        &self,
        search: &ServerQuery<'_>,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<SearchPage> {
        let page: Page<GraphMessage> = match cursor {
            // Refuses a cursor that isn't a link to Graph, so the token stays.
            Some(link) => self.client.get_page(link, &[PREFER_IMMUTABLE_ID]).await?,
            None if search.query.is_empty() => return Ok(SearchPage::default()),
            None => self.first_search_page(search, limit).await?,
        };
        let folders = search.folders;
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

    /// The first page of a search; see [`Self::search_page`]. A mailbox that
    /// answers a search with dates with 400 is asked again without them,
    /// and, if that works, never sent them from then on.
    async fn first_search_page(
        &self,
        search: &ServerQuery<'_>,
        limit: u32,
    ) -> Result<Page<GraphMessage>> {
        let base = match search.scope {
            Some([folder]) => format!("/me/mailFolders/{}/messages", segment(&folder.remote_id)),
            _ => "/me/messages".to_owned(),
        };
        let select = format!("{SUMMARY_FIELDS},parentFolderId");
        let top = limit.to_string();
        let search_url = |kql: &str| {
            self.client.url_with_query(
                &base,
                &[
                    ("$search", &kql::search_param(kql)),
                    ("$select", &select),
                    ("$top", &top),
                ],
            )
        };
        let dates = if self.kql_dates_refused.load(Ordering::Relaxed) {
            Received::default()
        } else {
            search.received
        };
        let Some(kql) = kql::kql(search.query, dates) else {
            let url = self.client.url_with_query(
                &base,
                &[
                    ("$filter", &kql::graph_filter(search.query, search.received)),
                    ("$orderby", "receivedDateTime desc"),
                    ("$select", &select),
                    ("$top", &top),
                ],
            );
            return Ok(self.client.get_page(&url, &[PREFER_IMMUTABLE_ID]).await?);
        };
        let error = match self
            .client
            .get_page(&search_url(&kql), &[PREFER_IMMUTABLE_ID])
            .await
        {
            Ok(page) => return Ok(page),
            Err(failure) => Error::from(failure),
        };
        let undated = kql::kql(search.query, Received::default()).unwrap_or_default();
        if error.code != ErrorCode::Provider || error.http_status() != Some(400) || undated == kql {
            return Err(error);
        }
        // Refused again: the dates weren't why, and later searches keep them.
        let page = self
            .client
            .get_page(&search_url(&undated), &[PREFER_IMMUTABLE_ID])
            .await?;
        self.kql_dates_refused.store(true, Ordering::Relaxed);
        // Never the query: it is the user's words.
        tracing::debug!("the mailbox refused dates in a search; leaving them to the device");
        Ok(page)
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

    /// The message's named properties ([`NAMED_PROPERTIES`]) in one
    /// sequential batch, so that reading a message takes two of the
    /// mailbox's four concurrent requests at most. A sub-request that fails
    /// leaves its value out, and the read incomplete.
    async fn named_properties(&self, message_id: &str) -> Result<NamedValues> {
        let requests = sequential_property_reads(message_id, &NAMED_PROPERTIES);
        let mut responses = self.client.batch(&requests, &[PREFER_IMMUTABLE_ID]).await?;
        let mut read = NamedValues {
            complete: true,
            ..NamedValues::default()
        };
        for (request_id, _) in NAMED_PROPERTIES {
            match responses.remove(request_id) {
                Some(response) if response.is_success() => {
                    if let Some(value) = response
                        .json::<WithProperties>()
                        .ok()
                        .and_then(WithProperties::value)
                    {
                        read.values.insert(request_id, value);
                    }
                }
                other => {
                    tracing::debug!(
                        "reading a message property ({request_id}) returned {:?}",
                        other.map(|r| r.status())
                    );
                    read.complete = false;
                }
            }
        }
        Ok(read)
    }

    /// What the `.p7m` attachment holds, from its first bytes.
    async fn smime_prefix(
        &self,
        message_id: &str,
        attachment_id: &str,
    ) -> Option<crate::providers::SmimeContent> {
        let url = self.message_url(
            message_id,
            &format!("/attachments/{}/$value", segment(attachment_id)),
        );
        match self
            .client
            .get_bytes_prefix(&url, &[PREFER_IMMUTABLE_ID], SMIME_PREFIX_BYTES)
            .await
        {
            Ok(prefix) => smime_content_of(&prefix),
            Err(e) => {
                tracing::debug!("reading an S/MIME file's start failed: {}", Error::from(e));
                None
            }
        }
    }

    /// The message's MIME, up to [`MAX_MIME_BYTES`].
    async fn mime(&self, message_id: &str) -> Result<Vec<u8>> {
        Ok(self
            .client
            .get_bytes_limited(
                &self.message_url(message_id, "/$value"),
                &[PREFER_IMMUTABLE_ID],
                MAX_MIME_BYTES,
            )
            .await?)
    }

    /// Reads back what a draft must carry before it's sent: each requested
    /// property exactly, and, for a reply or forward asking for a label, no
    /// other label of the organization copied from the original (nor any,
    /// when the requested one encrypts). Errors: `headersNotKept` when
    /// Microsoft 365 answered without the value or refused to read it,
    /// `labelCopied`, and [`unanswered`] (or the network's error) when it
    /// didn't answer: nothing was learned of the draft then.
    async fn read_back(&self, draft_id: &str, reply: bool, request: &HeaderRequest) -> Result<()> {
        let not_kept = || headers_not_kept(None);
        let mut properties: Vec<(&str, &str)> = Vec::new();
        if request.protect.is_some() {
            properties.push(("protect", PROTECT_PROPERTY));
        }
        if request.label_id.is_some() {
            properties.push(("label", LABEL_PROPERTY));
            if reply {
                properties.push(("msipLabels", MSIP_LABELS_PROPERTY));
            }
        }
        let requests = sequential_property_reads(draft_id, &properties);
        let mut responses = tokio::time::timeout(
            READ_BACK_BUDGET,
            self.client.batch(&requests, &[PREFER_IMMUTABLE_ID]),
        )
        .await
        .map_err(|_| unanswered())??;
        let mut value = |id: &str| -> Result<Option<String>> {
            let response = responses.remove(id).ok_or_else(unanswered)?;
            let status = response.status();
            if !response.is_success() {
                tracing::debug!("reading back a draft's {id} returned {status}");
                return Err(if is_refusal(status) {
                    not_kept()
                } else {
                    unanswered()
                });
            }
            // An answer that doesn't read is no answer about the value.
            Ok(response
                .json::<WithProperties>()
                .map_err(|_| unanswered())?
                .value())
        };
        if let Some(action) = request.protect
            && value("protect")?.as_deref() != Some(action.header_value())
        {
            tracing::debug!("the draft didn't keep {PROTECT_HEADER}");
            return Err(not_kept());
        }
        if let Some(label) = &request.label_id {
            if value("label")?.as_deref() != Some(label.as_str()) {
                tracing::debug!("the draft didn't keep {LABEL_HEADER}");
                return Err(not_kept());
            }
            if reply
                && let Some(copied) =
                    value("msipLabels")?.and_then(|raw| own_label(&raw, &request.tenant_id))
                && (request.label_encrypts || copied.id != *label)
            {
                return Err(label_copied(&copied.id));
            }
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
        // The store takes this list as it is.
        *self.folder_ids.lock().unwrap() = folders.iter().map(|f| f.id.clone()).collect();
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
        let message_id = message.remote_id.as_str();
        let url = self.client.url_with_query(
            &format!("/me/messages/{}", segment(message_id)),
            &[
                ("$select", DETAIL_FIELDS),
                ("$expand", &expand_property(MESSAGE_CLASS_PROPERTY)),
            ],
        );
        let prefer = [PREFER_IMMUTABLE_ID, PREFER_HTML_BODY];
        // The properties are optional: a slow or failed batch never holds
        // up reading, and the message is read again later (see
        // `MessageSignals::properties_unavailable`).
        let (detail, properties) = tokio::join!(
            self.client.get_json::<GraphMessageDetail>(&url, &prefer),
            tokio::time::timeout(self.property_budget, self.named_properties(message_id)),
        );
        let detail = detail?;
        let properties = match properties {
            Ok(Ok(properties)) => properties,
            Ok(Err(e)) => {
                tracing::debug!("reading a message's properties failed: {e}");
                NamedValues::default()
            }
            Err(_) => {
                tracing::debug!("reading a message's properties took too long");
                NamedValues::default()
            }
        };
        let (mut body_html, mut body_text) = match detail.body {
            Some(body) if body.is_html() => (body.content, None),
            Some(body) => (None, body.content),
            None => (None, None),
        };
        let header = |name: &str| {
            detail
                .internet_message_headers
                .iter()
                .find(|h| h.name.eq_ignore_ascii_case(name))
                .map(|h| h.value.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        let property = |id: &str| properties.values.get(id).cloned();
        let (msip_labels, msip_labels_source) = sourced(
            (header("msip_labels"), "header"),
            (property("msipLabels"), "property"),
        );
        let (requested_action, requested_action_source) = sourced(
            (property("requestedProtect"), "property"),
            (header(PROTECT_HEADER), "header"),
        );
        let (requested_label, requested_label_source) = sourced(
            (property("requestedLabel"), "property"),
            (header(LABEL_HEADER), "header"),
        );
        let mut signals = MessageSignals {
            content_class: property("contentClass").or_else(|| header("Content-Class")),
            message_class: detail
                .single_value_extended_properties
                .into_iter()
                .find_map(|p| p.value)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty()),
            msip_labels,
            msip_labels_source,
            has_attachments_flag: detail.has_attachments,
            requested_action,
            requested_label,
            requested_action_source,
            requested_label_source,
            relay_stamped: header(STAMP_HEADER).is_some(),
            label_stamped: header(LABEL_STAMPED_HEADER),
            auth_as: header(AUTH_AS_HEADER),
            directionality: header(DIRECTIONALITY_HEADER),
            properties_unavailable: !properties.complete,
            ..MessageSignals::default()
        };

        let has_cid = body_html.as_deref().is_some_and(|h| h.contains("cid:"));
        // `hasAttachments` ignores inline attachments, hence the `cid:` check;
        // protected mail and blank bodies are listed whatever it says.
        let blank = is_blank(body_html.as_deref(), body_text.as_deref());
        let mut attachments = if detail.has_attachments.unwrap_or(false)
            || has_cid
            || wants_attachments(&signals)
            || blank
        {
            self.list_attachments(message_id).await?
        } else {
            Vec::new()
        };

        let class = signals
            .message_class
            .as_deref()
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        let named = |a: &&AttachmentMeta, name: &str| a.name.trim().eq_ignore_ascii_case(name);
        // Opaque-signed and encrypted S/MIME share this class: the .p7m's
        // first bytes tell them apart.
        if class == "ipm.note.smime" {
            let p7m = attachments
                .iter()
                .find(|a| named(a, "smime.p7m"))
                .or_else(|| {
                    attachments.iter().find(|a| {
                        let t = a.content_type.to_ascii_lowercase();
                        t.contains("pkcs7-mime") || t.contains("pkcs7-signature")
                    })
                })
                .map(|a| a.id.clone());
            if let Some(id) = p7m {
                signals.smime_content = self.smime_prefix(message_id, &id).await;
            }
        }
        // Exchange keeps a clear-signed message's attachments inside the
        // smime.p7m holding the whole multipart/signed entity: read them
        // from the message's MIME.
        let clear_signed = class.starts_with("ipm.note.smime.multipartsigned")
            || attachments.iter().any(|a| {
                named(&a, "smime.p7m")
                    && a.content_type
                        .to_ascii_lowercase()
                        .starts_with("multipart/signed")
            });
        let mut from_mime = false;
        if clear_signed {
            match self.mime(message_id).await {
                Ok(raw) => {
                    let parsed = mime::parse_message(&raw);
                    attachments = parsed
                        .attachments
                        .into_iter()
                        .map(|a| AttachmentMeta {
                            id: format!("{MIME_ATTACHMENT_PREFIX}{}", a.id),
                            ..a
                        })
                        .collect();
                    signals.mime_type = parsed.signals.mime_type;
                    if !is_blank(parsed.body_html.as_deref(), parsed.body_text.as_deref()) {
                        body_html = parsed.body_html;
                        body_text = parsed.body_text;
                    }
                    signals.body_from_mime = true;
                    from_mime = true;
                }
                Err(e) => tracing::debug!("reading a signed message's MIME failed: {e}"),
            }
        }
        // The MIME's images are inlined already.
        if has_cid
            && !from_mime
            && let Some(html) = body_html.as_mut()
        {
            self.embed_inline_images(message_id, html, &mut attachments)
                .await;
        }
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
            signals,
        })
    }

    async fn fetch_attachment(
        &self,
        message: &MessageLocator,
        attachment_id: &str,
    ) -> Result<Vec<u8>> {
        // A part of a clear-signed message's MIME.
        if let Some(section) = attachment_id.strip_prefix(MIME_ATTACHMENT_PREFIX) {
            let valid =
                !section.is_empty() && section.bytes().all(|b| b.is_ascii_digit() || b == b'.');
            if !valid {
                return Err(Error::not_found("attachment"));
            }
            let raw = self.mime(&message.remote_id).await?;
            return mime::attachment_content(&raw, section)
                .ok_or_else(|| Error::not_found("attachment"));
        }
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
        search: &ServerQuery<'_>,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<SearchPage> {
        self.search_page(search, cursor, limit).await
    }

    async fn send(&self, message: &OutgoingMessage, original: Option<&Message>) -> Result<()> {
        // What it asks for would be dropped here.
        if message.protect.is_some() || message.label_id.is_some() {
            return Err(Error::internal(
                "protected mail goes through send_requesting",
            ));
        }
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

    async fn send_requesting(
        &self,
        message: &OutgoingMessage,
        original: Option<&Message>,
        request: &HeaderRequest,
    ) -> Result<()> {
        let uploads = decode_attachments(message)?;
        let reply = message.reply_kind.zip(original);
        let kind = message.reply_kind;
        let draft_id = self.create_draft(message, reply).await?;
        // The x-cumulas-* properties, as Exchange keeps Internet headers;
        // `internetMessageHeaders` can only be set on a new message, and a
        // reply's draft doesn't keep them.
        let mut properties = Vec::new();
        if let Some(action) = request.protect {
            properties.push(json!({ "id": PROTECT_PROPERTY, "value": action.header_value() }));
        }
        if let Some(label) = &request.label_id {
            properties.push(json!({ "id": LABEL_PROPERTY, "value": label }));
        }
        let patched = self
            .client
            .send_no_content(
                Method::PATCH,
                &self.message_url(&draft_id, ""),
                Some(&json!({ "singleValueExtendedProperties": properties })),
                &[PREFER_IMMUTABLE_ID],
            )
            .await;
        let checked = match patched {
            Ok(()) => self.read_back(&draft_id, reply.is_some(), request).await,
            Err(Failure::Graph { status, .. }) if is_refusal(status) => {
                tracing::debug!("setting a draft's properties was refused ({status})");
                Err(headers_not_kept(kind))
            }
            Err(e) => Err(e.into()),
        };
        if let Err(e) = checked {
            self.discard_draft(&draft_id).await;
            if crate::protection::has_reason(&e, "headersNotKept") {
                tracing::debug!("a draft didn't keep what it asks for: {e}");
                return Err(headers_not_kept(kind));
            }
            // labelCopied, or Microsoft 365 didn't answer: said as it is.
            return Err(e);
        }
        if let Err(e) = self.add_attachments(&draft_id, &uploads).await {
            self.discard_draft(&draft_id).await;
            return Err(e);
        }
        // As in `send`: never discarded after this point.
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

    async fn web_link(&self, message: &MessageLocator) -> Result<Option<String>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct WebLink {
            web_link: Option<String>,
        }
        let url = self.client.url_with_query(
            &format!("/me/messages/{}", segment(&message.remote_id)),
            &[("$select", "webLink")],
        );
        let link: WebLink = self.client.get_json(&url, &[PREFER_IMMUTABLE_ID]).await?;
        Ok(https_only(link.web_link))
    }

    async fn transport_headers(&self, message: &MessageLocator) -> Result<Option<String>> {
        let url = self.client.url_with_query(
            &format!("/me/messages/{}", segment(&message.remote_id)),
            &[
                ("$select", "id"),
                ("$expand", &expand_property(TRANSPORT_HEADERS_PROPERTY)),
            ],
        );
        let read: WithProperties = self.client.get_json(&url, &[PREFER_IMMUTABLE_ID]).await?;
        Ok(read.value().map(|mut headers| {
            if headers.len() > MAX_TRANSPORT_HEADERS_BYTES {
                let mut end = MAX_TRANSPORT_HEADERS_BYTES;
                while !headers.is_char_boundary(end) {
                    end -= 1;
                }
                headers.truncate(end);
            }
            headers
        }))
    }

    async fn find_in_inbox(
        &self,
        tokens: &[String],
        since: DateTime<Utc>,
    ) -> Result<HashMap<String, MessageLocator>> {
        let since = since.to_rfc3339_opts(SecondsFormat::Secs, true);
        let mut next = Some(self.client.url_with_query(
            "/me/mailFolders/inbox/messages",
            &[
                ("$filter", &format!("receivedDateTime ge {since}")),
                ("$orderby", "receivedDateTime desc"),
                ("$select", INBOX_LOOKUP_FIELDS),
                ("$top", "50"),
            ],
        ));
        let wanted: Vec<(&String, String)> = tokens
            .iter()
            .map(|t| (t, format!("[{}]", t.to_ascii_lowercase())))
            .collect();
        let mut found = HashMap::new();
        for _ in 0..MAX_INBOX_LOOKUP_PAGES {
            let Some(url) = next.take() else {
                break;
            };
            let page: Page<GraphMessage> =
                self.client.get_page(&url, &[PREFER_IMMUTABLE_ID]).await?;
            // Newest first: the first match of a token is its newest.
            for message in page.value {
                let subject = message.subject.unwrap_or_default().to_ascii_lowercase();
                for (token, marker) in &wanted {
                    if !found.contains_key(*token) && subject.contains(marker.as_str()) {
                        found.insert(
                            (*token).clone(),
                            MessageLocator {
                                folder_remote_id: "inbox".into(),
                                remote_id: message.id.clone(),
                            },
                        );
                    }
                }
            }
            if found.len() == wanted.len() {
                break;
            }
            next = page.next_link;
        }
        Ok(found)
    }

    async fn own_address(&self) -> Result<Option<EmailAddress>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Me {
            mail: Option<String>,
            user_principal_name: Option<String>,
        }
        let address = self
            .own_address
            .get_or_try_init(|| async {
                let url = self
                    .client
                    .url_with_query("/me", &[("$select", "mail,userPrincipalName")]);
                let me: Me = self.client.get_json(&url, &[]).await?;
                Ok::<_, Error>(
                    [me.mail, me.user_principal_name]
                        .into_iter()
                        .flatten()
                        .map(|a| a.trim().to_owned())
                        .find(|a| a.contains('@'))
                        .map(|a| EmailAddress::new(None, &a)),
                )
            })
            .await?;
        Ok(address.clone())
    }
}
