//! Protected mail: telling Microsoft Purview encryption, S/MIME and
//! Microsoft's encrypted message portal apart from ordinary mail, reading a
//! message's sensitivity label, and the diagnostics people copy for a bug
//! report. Pure: no I/O.
//!
//! Detection runs when a message is read, from the signals kept with its
//! content ([`MessageSignals`]), so a better rule in an update applies to
//! mail already on the device without fetching it again. No signal is
//! trusted on its own where a sender could plant it: a file counts as the
//! protected content only by its exact name, or by its type when a
//! message-level signal agrees (and, in a MIME structure, it's where
//! S/MIME keeps its content); a label shows only for the account's own
//! organization, and only by the name the organization gives it.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::{Error, ErrorCode};
use crate::html::html_to_text;
use crate::model::{
    AttachmentMeta, CatalogLabel, CheckItem, CheckNote, CheckResult, CheckState, FolderRole,
    LabelCatalog, ProtectAction, Protection, ProtectionKind, ReplyKind,
};
use crate::providers::{MessageContent, MessageSignals, SmimeContent};
use crate::signatures::config::is_guid;
use crate::store::{CheckRecord, LastControl};

/// Most test messages one check sends (plus its control), under Exchange
/// Online's 30 messages a minute. `MAX_CHECK_ITEMS` in
/// `src/modules/mail/protection.ts` must match.
pub const MAX_CHECK_ITEMS: usize = 19;
/// How long a passed check lets an option be offered.
pub const CHECK_VALID_DAYS: i64 = 90;
/// How long a check waits for its test messages to arrive.
pub const CHECK_TIMEOUT_MINUTES: i64 = 15;

/// The header (and named property) that asks the organization's mail server
/// to encrypt: `encrypt` or `do-not-forward`.
pub const PROTECT_HEADER: &str = "x-cumulas-protect";
/// The header (and named property) that asks for a sensitivity label: its
/// GUID, lowercase.
pub const LABEL_HEADER: &str = "x-cumulas-label";
/// PS_INTERNET_HEADERS, the named-property set of Internet headers.
pub const PS_INTERNET_HEADERS: &str = "{00020386-0000-0000-C000-000000000046}";
/// The MAPI message class (PidTagMessageClass).
pub const MESSAGE_CLASS_PROPERTY: &str = "String 0x001A";
pub const CONTENT_CLASS_PROPERTY: &str =
    "String {00020386-0000-0000-C000-000000000046} Name Content-Class";
pub const MSIP_LABELS_PROPERTY: &str =
    "String {00020386-0000-0000-C000-000000000046} Name msip_labels";
/// Exchange stores names in this set in lowercase (MS-OXCPRPT) and Graph's
/// Name filter is case-sensitive: always the header's lowercase name.
pub const PROTECT_PROPERTY: &str =
    "String {00020386-0000-0000-C000-000000000046} Name x-cumulas-protect";
pub const LABEL_PROPERTY: &str =
    "String {00020386-0000-0000-C000-000000000046} Name x-cumulas-label";
/// PR_TRANSPORT_MESSAGE_HEADERS: the whole header block transport delivered,
/// not only the part Graph parses into internetMessageHeaders.
pub const TRANSPORT_HEADERS_PROPERTY: &str = "String 0x007D";
/// Most of a message's transport headers kept: Exchange Online's limit for
/// a message's header.
pub const MAX_TRANSPORT_HEADERS_BYTES: usize = 256 * 1024;

/// The Cumulas Signatures relay adds it to every message it hands back.
pub const STAMP_HEADER: &str = "X-Signature-Stamped";
/// The earlier relay recipe's copy of the label request.
pub const LABEL_STAMPED_HEADER: &str = "x-cumulas-label-stamped";
/// How Exchange Online counted the sender: `Internal` for mail from inside
/// the organization.
pub const AUTH_AS_HEADER: &str = "X-MS-Exchange-Organization-AuthAs";
/// `Originating` or `Incoming`.
pub const DIRECTIONALITY_HEADER: &str = "X-MS-Exchange-Organization-MessageDirectionality";

/// Attachment ids of the parts of a message's MIME read on Graph:
/// `mime:<section>` (IMAP's are the bare section).
pub const MIME_ATTACHMENT_PREFIX: &str = "mime:";

/// Why sending protected mail, or checking it, isn't possible for an
/// account.
pub const PROTECTED_SENDING_UNSUPPORTED: &str =
    "Only Microsoft 365 work and school accounts can send protected mail.";

const RPMSG_NAMES: &[&str] = &[
    "message.rpmsg",
    "message_v2.rpmsg",
    "message_v3.rpmsg",
    "message_v4.rpmsg",
];
const SMIME_NAMES: &[&str] = &["smime.p7m", "smime.p7s"];
const RPMSG_TYPE: &str = "application/x-microsoft-rpmsg-message";
const SMIME_TYPES: &[&str] = &[
    "application/pkcs7-mime",
    "application/x-pkcs7-mime",
    "application/pkcs7-signature",
    "application/x-pkcs7-signature",
];
/// Hosts of Microsoft's encrypted message portal (public cloud).
const PORTAL_HOSTS: &[&str] = &["outlook.office365.com", "outlook.office.com"];
const SAFE_LINKS_SUFFIX: &str = ".safelinks.protection.outlook.com";

/// CMS ContentInfo content types (DER OID bytes): signedData,
/// envelopedData, authEnvelopedData.
const OID_SIGNED_DATA: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07, 0x02];
const OID_ENVELOPED_DATA: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07, 0x03];
const OID_AUTH_ENVELOPED_DATA: &[u8] = &[
    0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x09, 0x10, 0x01, 0x17,
];

// --- Errors ------------------------------------------------------------------

/// What a reply kind is called in messages: "message", "reply" or "forward".
pub fn kind_noun(reply_kind: Option<ReplyKind>) -> &'static str {
    match reply_kind {
        None => "message",
        Some(ReplyKind::Reply | ReplyKind::ReplyAll) => "reply",
        Some(ReplyKind::Forward) => "forward",
    }
}

/// Microsoft 365 didn't keep what the message asks for on its draft (or the
/// draft couldn't be made): nothing was sent.
pub fn headers_not_kept(reply_kind: Option<ReplyKind>) -> Error {
    Error::provider(format!(
        "Cumulas Mail couldn't add the label or encryption to this {}, so it wasn't sent. Send \
         it from Outlook on the web instead.",
        kind_noun(reply_kind)
    ))
    .with_details(serde_json::json!({ "reason": "headersNotKept" }))
}

/// A reply's or forward's draft already carries the organization's label
/// `label_id`, copied from the original. The service words the message.
pub fn label_copied(label_id: &str) -> Error {
    Error::invalid_input("labelCopied")
        .with_details(serde_json::json!({ "reason": "labelCopied", "labelId": label_id }))
}

/// Whether an error says this `reason` in its details.
pub fn has_reason(error: &Error, reason: &str) -> bool {
    error
        .details
        .as_ref()
        .and_then(|d| d.get("reason"))
        .and_then(|r| r.as_str())
        == Some(reason)
}

/// The refusal to open a file that holds a protected message's content.
pub fn container_refused() -> Error {
    Error::new(
        ErrorCode::Unsupported,
        "This file holds a protected message's content, which Cumulas Mail can't open. Open the \
         message in Outlook on the web.",
    )
}

// --- Detection ---------------------------------------------------------------

/// A content type without its parameters, lowercase.
fn base_type(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

fn is_named(name: &str, names: &[&str]) -> bool {
    let name = name.trim();
    names.iter().any(|n| n.eq_ignore_ascii_case(name))
}

fn is_rpmsg_name(name: &str) -> bool {
    is_named(name, RPMSG_NAMES)
}

fn is_smime_name(name: &str) -> bool {
    is_named(name, SMIME_NAMES)
}

fn is_container_name(name: &str) -> bool {
    is_rpmsg_name(name) || is_smime_name(name)
}

fn is_rpmsg_type(content_type: &str) -> bool {
    base_type(content_type) == RPMSG_TYPE
}

fn is_smime_type(content_type: &str) -> bool {
    SMIME_TYPES.contains(&base_type(content_type).as_str())
}

fn is_smime_signature_type(content_type: &str) -> bool {
    matches!(
        base_type(content_type).as_str(),
        "application/pkcs7-signature" | "application/x-pkcs7-signature"
    )
}

/// Whether a file of an S/MIME type is where the message's own S/MIME
/// keeps its signature or content, so that it counts as a container.
/// Read from a MIME structure (IMAP, or Graph's read of a signed message's
/// MIME), only a `multipart/signed` root's signature part or an opaque
/// S/MIME root itself is: a signed document attached to signed mail
/// (`contract.pdf.p7m`, `document.pdf.p7s`) is the sender's file. Graph's
/// own attachments have no structure; there the type and the message-level
/// signal agreeing is enough.
fn is_smime_part(signals: &MessageSignals, attachment: &AttachmentMeta) -> bool {
    let Some(root) = signals.mime_type.as_deref() else {
        return true;
    };
    let root = root.trim().to_ascii_lowercase();
    let section = attachment
        .id
        .strip_prefix(MIME_ATTACHMENT_PREFIX)
        .unwrap_or(&attachment.id);
    if root.starts_with("multipart/signed") {
        // The signed content is part 1, its signature part 2.
        !section.contains('.') && is_smime_signature_type(&attachment.content_type)
    } else if root.starts_with("application/pkcs7-mime")
        || root.starts_with("application/x-pkcs7-mime")
    {
        section == "1"
    } else {
        false
    }
}

fn lower(value: Option<&String>) -> String {
    value
        .map(|v| v.trim().to_ascii_lowercase())
        .unwrap_or_default()
}

/// Content-Class `rpmsg.message`, or an `IPM.Note.rpmsg*` message class.
fn rights_signal(signals: &MessageSignals) -> bool {
    lower(signals.content_class.as_ref()) == "rpmsg.message"
        || lower(signals.message_class.as_ref()).starts_with("ipm.note.rpmsg")
}

/// An `IPM.Note.SMIME*` message class, or an S/MIME top-level MIME type (a
/// `multipart/signed` with an OpenPGP signature isn't one).
fn smime_signal(signals: &MessageSignals) -> bool {
    let mime = lower(signals.mime_type.as_ref());
    lower(signals.message_class.as_ref()).starts_with("ipm.note.smime")
        || mime.starts_with("application/pkcs7-mime")
        || mime.starts_with("application/x-pkcs7-mime")
        || (mime.starts_with("multipart/signed") && mime.contains("pkcs7-signature"))
}

/// Whether the message's attachments should be listed whatever Graph says
/// of them: a message-level signal says it's protected or S/MIME.
pub fn wants_attachments(signals: &MessageSignals) -> bool {
    rights_signal(signals) || smime_signal(signals)
}

/// Whether neither body has any text.
pub fn is_blank(body_html: Option<&str>, body_text: Option<&str>) -> bool {
    body_html.is_none_or(|html| html_to_text(html).trim().is_empty())
        && body_text.is_none_or(|text| text.trim().is_empty())
}

/// How the message is protected, if it is. `body_html` is the raw HTML;
/// `own_mail` is set in Sent and Drafts, where a quoted portal notice
/// would otherwise count.
pub fn detect(
    signals: &MessageSignals,
    attachments: &[AttachmentMeta],
    body_html: Option<&str>,
    body_blank: bool,
    own_mail: bool,
) -> Option<Protection> {
    let rights = rights_signal(signals);
    let smime = smime_signal(signals);
    let rpmsg: Vec<&AttachmentMeta> = attachments
        .iter()
        .filter(|a| is_rpmsg_name(&a.name) || (rights && is_rpmsg_type(&a.content_type)))
        .collect();
    let smime_files: Vec<&AttachmentMeta> = attachments
        .iter()
        .filter(|a| {
            is_smime_name(&a.name)
                || (smime && is_smime_type(&a.content_type) && is_smime_part(signals, a))
        })
        .collect();
    let portal_url = if own_mail {
        None
    } else {
        body_html.and_then(portal_link_in)
    };
    let has_p7m = smime_files
        .iter()
        .any(|a| a.name.trim().eq_ignore_ascii_case("smime.p7m"));
    let (kind, containers) = if rights || !rpmsg.is_empty() {
        (ProtectionKind::RightsManaged, rpmsg)
    } else if smime {
        let class = lower(signals.message_class.as_ref());
        let mime = lower(signals.mime_type.as_ref());
        let kind = match signals.smime_content {
            Some(SmimeContent::Enveloped) => ProtectionKind::SmimeEncrypted,
            Some(SmimeContent::Signed) => ProtectionKind::SmimeSigned,
            None if class.starts_with("ipm.note.smime.multipartsigned")
                || mime.starts_with("multipart/signed")
                || mime.contains("smime-type=signed-data") =>
            {
                ProtectionKind::SmimeSigned
            }
            // Unknown: the safe side for replies.
            None => ProtectionKind::SmimeEncrypted,
        };
        (kind, smime_files)
    } else if !smime_files.is_empty() {
        // Exact names without a message-level signal: on Graph without a
        // class, an smime.p7m next to a body is a clear-signed message's.
        let kind = if has_p7m && body_blank && signals.smime_content != Some(SmimeContent::Signed) {
            ProtectionKind::SmimeEncrypted
        } else {
            ProtectionKind::SmimeSigned
        };
        (kind, smime_files)
    } else if portal_url.is_some() {
        (ProtectionKind::PortalLink, Vec::new())
    } else {
        return None;
    };
    let content_hidden = kind == ProtectionKind::SmimeSigned
        && (signals.smime_content == Some(SmimeContent::Signed)
            || (containers
                .iter()
                .any(|a| a.name.trim().eq_ignore_ascii_case("smime.p7m"))
                && !signals.body_from_mime));
    Some(Protection {
        kind,
        container_ids: containers.iter().map(|a| a.id.clone()).collect(),
        portal_url,
        content_hidden,
    })
}

/// The first link in `html` to Microsoft's encrypted message portal, as
/// written (after `&amp;` decoding), so that a Safe Links wrapper stays and
/// Safe Links still checks the link when it's opened. A wrapped link is
/// judged by the address it wraps.
pub fn portal_link_in(html: &str) -> Option<String> {
    hrefs(html).into_iter().find(|href| is_portal(href))
}

/// HTTPS on its default port, without a user name or password: the only
/// form of Microsoft's own addresses, and of their Safe Links wrappers.
fn is_plain_https(url: &Url) -> bool {
    url.scheme() == "https"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
}

fn is_portal(href: &str) -> bool {
    let Ok(mut url) = Url::parse(href) else {
        return false;
    };
    let wrapped = url
        .host_str()
        .is_some_and(|host| host.to_ascii_lowercase().ends_with(SAFE_LINKS_SUFFIX));
    if wrapped {
        // The wrapper is what opens: it must be as safe as what it wraps.
        if !is_plain_https(&url) {
            return false;
        }
        let Some(inner) = url
            .query_pairs()
            .find(|(key, _)| key == "url")
            .and_then(|(_, value)| Url::parse(&value).ok())
        else {
            return false;
        };
        url = inner;
    }
    is_plain_https(&url)
        && url
            .host_str()
            .is_some_and(|host| PORTAL_HOSTS.iter().any(|p| host.eq_ignore_ascii_case(p)))
        && url.path().to_ascii_lowercase().starts_with("/encryption/")
}

/// Every quoted `href` attribute value in `html`, in order.
fn hrefs(html: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let bytes = html.as_bytes();
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = lower[from..].find("href") {
        let start = from + at;
        from = start + 4;
        // An attribute of its own, not the end of another's name.
        let before = start.checked_sub(1).map(|i| bytes[i]);
        if !before.is_some_and(|b| b.is_ascii_whitespace() || matches!(b, b'"' | b'\'' | b'/')) {
            continue;
        }
        let mut i = from;
        while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        if bytes.get(i) != Some(&b'=') {
            continue;
        }
        i += 1;
        while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        let Some(&quote) = bytes.get(i).filter(|q| matches!(q, b'"' | b'\'')) else {
            continue;
        };
        let value_start = i + 1;
        let Some(len) = html[value_start..].find(char::from(quote)) else {
            break;
        };
        found.push(
            html[value_start..value_start + len]
                .trim()
                .replace("&amp;", "&"),
        );
        from = value_start + len + 1;
    }
    found
}

/// What an S/MIME `.p7m` holds, from its first bytes: the CMS ContentInfo's
/// content type. `None` for anything else, or too few bytes.
pub fn smime_content_of(prefix: &[u8]) -> Option<SmimeContent> {
    // ContentInfo ::= SEQUENCE { contentType OBJECT IDENTIFIER, ... }
    if *prefix.first()? != 0x30 {
        return None;
    }
    let mut at = 1;
    match *prefix.get(at)? {
        // Short form, or BER's indefinite length.
        length if length <= 0x80 => at += 1,
        length @ 0x81..=0x84 => at += 1 + usize::from(length - 0x80),
        _ => return None,
    }
    if *prefix.get(at)? != 0x06 {
        return None;
    }
    let length = usize::from(*prefix.get(at + 1)?);
    if length >= 0x80 {
        return None;
    }
    let oid = prefix.get(at + 2..at + 2 + length)?;
    if oid == OID_SIGNED_DATA {
        Some(SmimeContent::Signed)
    } else if oid == OID_ENVELOPED_DATA || oid == OID_AUTH_ENVELOPED_DATA {
        Some(SmimeContent::Enveloped)
    } else {
        None
    }
}

// --- Sensitivity labels ------------------------------------------------------

/// One label named in a message's `msip_labels` header.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MsipLabel {
    /// The label's GUID, lowercase.
    pub id: String,
    pub enabled: bool,
    /// The tenant of the organization whose label it is.
    pub site_id: Option<String>,
    /// As the message names it: any sender can write anything here, so
    /// it's never shown; labels are named from the organization's list.
    pub name: Option<String>,
    pub method: Option<String>,
}

/// The labels an `msip_labels` value names, in the order first seen.
/// Pieces that don't read are skipped.
pub fn parse_msip_labels(raw: &str) -> Vec<MsipLabel> {
    const PREFIX: &str = "msip_label_";
    let mut labels: Vec<MsipLabel> = Vec::new();
    for piece in raw.split(';') {
        let Some((key, value)) = piece.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        let Some(rest) = key
            .get(..PREFIX.len())
            .filter(|p| p.eq_ignore_ascii_case(PREFIX))
            .and_then(|_| key.get(PREFIX.len()..))
        else {
            continue;
        };
        let (Some(id), Some(field)) = (rest.get(..36), rest.get(36..)) else {
            continue;
        };
        let Some(field) = field.strip_prefix('_') else {
            continue;
        };
        if !is_guid(id) {
            continue;
        }
        let id = id.to_ascii_lowercase();
        let index = match labels.iter().position(|l| l.id == id) {
            Some(index) => index,
            None => {
                labels.push(MsipLabel {
                    id,
                    ..MsipLabel::default()
                });
                labels.len() - 1
            }
        };
        let label = &mut labels[index];
        let text = (!value.is_empty()).then(|| value.to_owned());
        match field.to_ascii_lowercase().as_str() {
            "enabled" => label.enabled = value.eq_ignore_ascii_case("true"),
            "siteid" => label.site_id = text,
            "name" => label.name = text,
            "method" => label.method = text,
            _ => {}
        }
    }
    labels
}

/// The first enabled label of the organization `tenant_id` an
/// `msip_labels` value names.
pub fn own_label(raw: &str, tenant_id: &str) -> Option<MsipLabel> {
    let tenant = tenant_id.trim();
    parse_msip_labels(raw).into_iter().find(|l| {
        l.enabled
            && l.site_id
                .as_deref()
                .is_some_and(|site| site.trim().eq_ignore_ascii_case(tenant))
    })
}

/// Bidirectional controls, which can reorder what's shown around them.
fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// What a message sent from Cumulas Mail asked for: the action, and the
/// label's GUID (lowercase) when it is one.
pub fn requested(signals: &MessageSignals) -> (Option<ProtectAction>, Option<String>) {
    let action = signals
        .requested_action
        .as_deref()
        .and_then(ProtectAction::from_header_value);
    let label = signals
        .requested_label
        .as_deref()
        .map(str::trim)
        .filter(|l| is_guid(l))
        .map(str::to_ascii_lowercase);
    (action, label)
}

// --- Setup check evidence ----------------------------------------------------

/// What a setup check's arrived message showed, kept with its verdict (the
/// store) and printed by "Copy details for IT". Values pass through
/// [`clean_value`]: control and bidirectional characters removed, `None`
/// when empty; header values at most 200 characters, the Message-ID at most
/// 998 (RFC 5322's line limit), so it can be pasted into
/// Get-MailDetailTransportRuleReport.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CheckEvidence {
    /// The internetMessageId of the arrived copy.
    pub message_id: Option<String>,
    /// The request (x-cumulas-protect or x-cumulas-label, header or its
    /// property) was still on the arrived copy. On a delivered copy the
    /// property exists only if the header did, so either means the header
    /// reached delivery. Always false for the control.
    pub request_arrived: bool,
    /// X-Signature-Stamped: the Cumulas Signatures relay handled it.
    pub relay_stamped: bool,
    /// x-cumulas-label-stamped (the earlier relay recipe's copy of the
    /// label request).
    pub label_stamped: Option<String>,
    /// X-MS-Exchange-Organization-AuthAs; `None` when absent or empty,
    /// which never counts as "not internal".
    pub auth_as: Option<String>,
    /// X-MS-Exchange-Organization-MessageDirectionality.
    pub directionality: Option<String>,
    pub detected: Option<ProtectionKind>,
    /// This organization's labels on it.
    pub own_labels: Vec<SeenLabel>,
    /// Labels of other organizations (counted, never named).
    pub foreign_labels: u32,
    pub content_class: Option<String>,
    pub message_class: Option<String>,
    pub rpmsg_attachment: bool,
    pub headers_from: HeadersFrom,
    /// From the check's send to when a refresh found it (an upper bound:
    /// refreshes are 10 seconds apart).
    pub found_after_secs: Option<i64>,
}

/// A label of the account's organization seen on an arrived message.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeenLabel {
    /// GUID, lowercase.
    pub id: String,
    pub enabled: bool,
}

/// Where an arrived message's header values were read from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HeadersFrom {
    #[default]
    Unread,
    /// The headers Microsoft Graph parses (internetMessageHeaders).
    Graph,
    /// The transport headers as delivered (PR_TRANSPORT_MESSAGE_HEADERS).
    Transport,
}

/// A value from a message or from Microsoft 365 fit to keep and copy: control
/// and bidirectional characters removed, trimmed, at most `max` characters;
/// `None` when nothing is left, so an empty header never counts as a value.
pub(crate) fn clean_value(value: &str, max: usize) -> Option<String> {
    let cleaned: String = value
        .chars()
        .filter(|c| !c.is_control() && !is_bidi_control(*c))
        .collect();
    let trimmed = cleaned.trim();
    (!trimmed.is_empty()).then(|| trimmed.chars().take(max).collect())
}

/// Most characters kept of a header value in the evidence.
const EVIDENCE_VALUE_CHARS: usize = 200;
/// Most characters kept of a Message-ID: RFC 5322's line limit, so that it
/// can be pasted whole into Get-MailDetailTransportRuleReport.
const MESSAGE_ID_CHARS: usize = 998;
/// Most labels kept of one message's msip_labels.
const MAX_SEEN_LABELS: usize = 20;

/// The values of header `name` (ASCII case ignored) in a raw header block,
/// unfolded (a line starting with a space or tab continues the one before)
/// and trimmed, in order. Reading stops at the first empty line, where a
/// body would start.
pub fn header_values(raw: &str, name: &str) -> Vec<String> {
    let mut values = Vec::new();
    // The value of a field named `name` while its lines are read.
    let mut current: Option<String> = None;
    for line in raw.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.starts_with([' ', '\t']) {
            if let Some(value) = current.as_mut() {
                value.push_str(line);
            }
            continue;
        }
        if let Some(value) = current.take() {
            values.push(value.trim().to_owned());
        }
        if line.is_empty() {
            break;
        }
        current = line
            .split_once(':')
            .filter(|(field, _)| field.trim().eq_ignore_ascii_case(name))
            .map(|(_, value)| value.to_owned());
    }
    if let Some(value) = current {
        values.push(value.trim().to_owned());
    }
    values
}

/// What a setup check's test message (or control) showed when it arrived,
/// `sent_at` and found `now`. With its transport headers (`transport`), the
/// relay's stamp, AuthAs, the direction and Content-Class come from them:
/// the first copy of each, since every hop adds its own on top, so it's the
/// delivering hop's. Without, from the headers Graph parsed (the signals).
/// The request counts as arrived when either kept it.
pub fn check_evidence(
    content: &MessageContent,
    transport: Option<&str>,
    tenant: &str,
    sent_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> CheckEvidence {
    let signals = &content.signals;
    let value = |v: Option<&str>| v.and_then(|v| clean_value(v, EVIDENCE_VALUE_CHARS));
    let first = |name: &str| transport.and_then(|raw| header_values(raw, name).into_iter().next());
    let requested = signals.requested_action.is_some() || signals.requested_label.is_some();
    let mut evidence = match transport {
        Some(raw) => CheckEvidence {
            relay_stamped: value(first(STAMP_HEADER).as_deref()).is_some(),
            label_stamped: value(first(LABEL_STAMPED_HEADER).as_deref()),
            auth_as: value(first(AUTH_AS_HEADER).as_deref()),
            directionality: value(first(DIRECTIONALITY_HEADER).as_deref()),
            request_arrived: requested
                || [PROTECT_HEADER, LABEL_HEADER].iter().any(|name| {
                    header_values(raw, name)
                        .iter()
                        .any(|v| value(Some(v)).is_some())
                }),
            headers_from: HeadersFrom::Transport,
            ..CheckEvidence::default()
        },
        None => CheckEvidence {
            relay_stamped: signals.relay_stamped,
            label_stamped: value(signals.label_stamped.as_deref()),
            auth_as: value(signals.auth_as.as_deref()),
            directionality: value(signals.directionality.as_deref()),
            request_arrived: requested,
            headers_from: HeadersFrom::Graph,
            ..CheckEvidence::default()
        },
    };
    // A Content-Class Graph missed (property and header) is taken from the
    // transport headers, so that it no longer hides Purview's encryption.
    let transport_class = first("Content-Class");
    let patched = MessageSignals {
        content_class: signals
            .content_class
            .clone()
            .or_else(|| transport_class.clone()),
        ..signals.clone()
    };
    let body_html = content.body_html.as_deref();
    evidence.detected = detect(
        &patched,
        &content.attachments,
        body_html,
        is_blank(body_html, content.body_text.as_deref()),
        false,
    )
    .map(|p| p.kind);
    evidence.content_class = value(
        transport_class
            .or_else(|| signals.content_class.clone())
            .as_deref(),
    );
    evidence.message_class = value(signals.message_class.as_deref());
    evidence.message_id = content
        .internet_message_id
        .as_deref()
        .and_then(|id| clean_value(id, MESSAGE_ID_CHARS));
    evidence.rpmsg_attachment = content
        .attachments
        .iter()
        .any(|a| is_rpmsg_name(&a.name) || is_rpmsg_type(&a.content_type));
    let msip = signals.msip_labels.clone().or_else(|| first("msip_labels"));
    let tenant = tenant.trim();
    for label in parse_msip_labels(msip.as_deref().unwrap_or_default()) {
        let own = label
            .site_id
            .as_deref()
            .is_some_and(|site| site.trim().eq_ignore_ascii_case(tenant));
        if !own {
            evidence.foreign_labels = evidence.foreign_labels.saturating_add(1);
        } else if evidence.own_labels.len() < MAX_SEEN_LABELS {
            evidence.own_labels.push(SeenLabel {
                id: label.id,
                enabled: label.enabled,
            });
        }
    }
    evidence.found_after_secs = Some((now - sent_at).num_seconds().max(0));
    evidence
}

/// The first enabled label of the organization the evidence saw, as
/// [`own_label`] finds it.
pub fn first_own_label(evidence: &CheckEvidence) -> Option<&str> {
    evidence
        .own_labels
        .iter()
        .find(|l| l.enabled)
        .map(|l| l.id.as_str())
}

/// Whether the message went through the Cumulas Signatures relay and came
/// back counted as mail from outside the organization: stamped, with an
/// AuthAs that is set and isn't `Internal`. An unset AuthAs never counts.
pub fn relay_not_internal(evidence: &CheckEvidence) -> bool {
    evidence.relay_stamped
        && evidence
            .auth_as
            .as_deref()
            .is_some_and(|a| !a.trim().eq_ignore_ascii_case("Internal"))
}

/// A PowerShell single-quoted string: `'…'`, with `'` and the typographic
/// single quotes PowerShell also ends such a string at doubled, so that any
/// value stays one string.
pub fn ps_quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');
    for c in value.chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            quoted.push(c);
        }
        quoted.push(c);
    }
    quoted.push('\'');
    quoted
}

// --- Organization setup changes ----------------------------------------------

/// When organization setup last changed the organization's mail flow rules
/// and auto-labeling policies from this computer: the app value
/// [`org_setup_changed_key`], as `{ "rulesChangedAt", "policiesChangedAt" }`.
/// Checks judged soon after say it's too soon to tell.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settle {
    pub rules_changed_at: Option<DateTime<Utc>>,
    pub policies_changed_at: Option<DateTime<Utc>>,
}

/// How long new or changed mail flow rules can take to start working.
pub const RULES_SETTLE: Duration = Duration::minutes(60);
/// How long auto-labeling policies can take to start working, once on.
pub const POLICIES_SETTLE: Duration = Duration::hours(24);

/// The app value that keeps the organization's [`Settle`]:
/// `orgsetup.changed.<tenant id, lowercase>`.
pub fn org_setup_changed_key(tenant: &str) -> String {
    format!("orgsetup.changed.{}", tenant.trim().to_ascii_lowercase())
}

// --- Copy details for IT -----------------------------------------------------

/// How every test message's subject starts (a token follows).
pub const CHECK_SUBJECT_PREFIX: &str = "Cumulas Mail setup check";
/// Where the guide says what the check details mean.
const CHECK_GUIDE: &str = "https://github.com/cumulas-inc/email-client/blob/main/docs/microsoft-365.md#troubleshooting-the-check";

/// One option's check, for [`check_details`].
#[derive(Debug, Clone)]
pub struct CheckDetailsItem {
    /// "Encrypt", "Do Not Forward", or "Label Confidential (<id>)".
    pub title: String,
    pub item: CheckItem,
    /// As Settings shows it now.
    pub result: CheckResult,
    /// What its last decided check kept.
    pub record: CheckRecord,
}

/// What [`check_details`] describes: an account's last checks.
pub struct CheckDetailsInput<'a> {
    /// The account's own address.
    pub email: &'a str,
    pub tenant: &'a str,
    pub version: &'a str,
    pub control: Option<&'a LastControl>,
    pub items: Vec<CheckDetailsItem>,
    pub catalog: Option<&'a LabelCatalog>,
    pub settle: &'a Settle,
}

fn utc(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

fn utc_day(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%d %H:%M UTC").to_string()
}

/// How long `span` is, in words: "10 minutes", "3 hours".
fn span_words(span: Duration) -> String {
    let minutes = span.num_minutes().max(0);
    let plural = |n: i64, unit: &str| {
        if n == 1 {
            format!("1 {unit}")
        } else {
            format!("{n} {unit}s")
        }
    };
    match minutes {
        0 => "less than a minute".into(),
        1..60 => plural(minutes, "minute"),
        60..2880 => plural(minutes / 60, "hour"),
        _ => plural(minutes / 1440, "day"),
    }
}

fn said(value: Option<&str>) -> &str {
    value.unwrap_or("not said")
}

/// What a check found, as diagnostics name it.
fn kind_words(kind: Option<ProtectionKind>) -> &'static str {
    match kind {
        Some(ProtectionKind::RightsManaged) => "Microsoft Purview encryption",
        Some(ProtectionKind::SmimeEncrypted) => "S/MIME encryption",
        Some(ProtectionKind::SmimeSigned) => "S/MIME signature",
        Some(ProtectionKind::PortalLink) => "encrypted message portal link",
        None => "nothing",
    }
}

/// A label of the organization: "Title (id)" when it's listed, else its id.
fn label_words(catalog: Option<&LabelCatalog>, id: &str) -> String {
    match catalog.and_then(|c| c.label(id)) {
        Some(label) => format!("{} ({id})", label.title()),
        None => id.to_owned(),
    }
}

/// A check's row in Settings (`describeCheck`), with times in UTC.
fn check_row_words(result: &CheckResult, label: bool) -> String {
    let on = result.at.map(utc_day);
    let dated = |text: &str| {
        on.as_ref()
            .map_or(text.to_owned(), |on| format!("{text} · {on}"))
    };
    let checked = |text: &str| {
        on.as_ref()
            .map_or(text.to_owned(), |on| format!("{text} · checked {on}"))
    };
    match (result.state, result.note) {
        (CheckState::NotChecked, _) => "Not checked yet".into(),
        (CheckState::Waiting, _) => "Checking…".into(),
        (CheckState::Applied, _) if label => checked("Arrived labeled"),
        (CheckState::Applied, _) => checked("Arrived encrypted"),
        (CheckState::Expired, Some(CheckNote::RelayChanged)) => "Check again to use it".into(),
        (CheckState::Expired, _) => format!(
            "Checked over {CHECK_VALID_DAYS} days ago{}. Check again to use it.",
            on.as_deref()
                .map(|on| format!(" ({on})"))
                .unwrap_or_default()
        ),
        (CheckState::NotApplied, Some(CheckNote::HeaderNotKept)) => dated("Couldn't send the test"),
        (CheckState::NotApplied, Some(CheckNote::OtherLabel)) => {
            checked("Arrived with another label")
        }
        (CheckState::NotApplied, _) => checked("Arrived without it"),
        (CheckState::Unknown, Some(CheckNote::NotArrived)) => dated("Couldn't check"),
        (CheckState::Unknown, _) => dated("Couldn't tell"),
    }
}

/// The note under a check's row in Settings, as its words for people say
/// it (`CHECK_NOTES` in `src/modules/settings/orgsetup/copy.ts`), without
/// the pointer to these details.
fn check_note_words(
    result: &CheckResult,
    item: &CheckItem,
    catalog: Option<&LabelCatalog>,
) -> Option<String> {
    let action = match item {
        CheckItem::Action { action } => Some(*action),
        CheckItem::Label { .. } => None,
    };
    // How long before the check was decided the setup changed.
    let changed = result
        .changed_at
        .zip(result.at)
        .map(|(changed, at)| span_words(at - changed));
    let note = match (result.state, result.note) {
        (CheckState::Waiting, _) => {
            return result.passed_at.map(|at| {
                format!(
                    "Until this check is done, the one from {} still applies.",
                    utc_day(at)
                )
            });
        }
        (CheckState::Applied, Some(CheckNote::NotArrived)) => format!(
            "The latest test message didn't arrive in your Inbox within {CHECK_TIMEOUT_MINUTES} \
             minutes, so this earlier check still applies."
        ),
        (CheckState::Expired, Some(CheckNote::RelayChanged)) => {
            "Your organization's mail now goes through the Cumulas Signatures relay, which it \
             didn't when this was checked."
                .into()
        }
        (CheckState::NotApplied, Some(CheckNote::HeaderNotKept)) => {
            "Microsoft 365 didn't keep the request on the test message, so Cumulas Mail can't \
             ask for this."
                .into()
        }
        (CheckState::NotApplied, Some(CheckNote::RequestIgnored)) => match action {
            Some(action) => format!(
                "Your organization's mail server got the request (x-cumulas-protect was still on \
                 the test message), but no rule acted on it. The rule “Cumulas Mail: {}” may be \
                 missing, turned off, in test mode, or new: rules can take 30 minutes or more to \
                 start working.",
                action.name()
            ),
            None => "Your organization's mail server got the request (x-cumulas-label was still \
                     on the test message), but no auto-labeling policy applied the label. The \
                     policy may be missing, still in simulation, not include you, not be in \
                     effect yet (allow up to a day), or need a license your plan doesn't have."
                .into(),
        },
        (CheckState::NotApplied, Some(CheckNote::RequestRemoved)) => format!(
            "The request was removed on the way, but the message wasn't {}. Usually a rule \
             removed it, such as “Cumulas Mail: ignore label requests from outside” when the \
             message didn't count as from inside your organization.",
            if action.is_some() {
                "encrypted"
            } else {
                "labeled"
            }
        ),
        (CheckState::NotApplied, Some(CheckNote::RelayNotInternal)) if action.is_some() => {
            "Your mail goes through the Cumulas Signatures relay, and Microsoft 365 treats it as \
             outside mail when it comes back. Cumulas Mail's encryption rules then need to come \
             before the relay's rule: Set up or check for your organization does that."
                .into()
        }
        (CheckState::NotApplied, Some(CheckNote::RelayNotInternal)) => {
            "Your mail goes through the Cumulas Signatures relay, and Microsoft 365 treats it as \
             outside mail when it comes back, so labels can't work yet. Your IT team needs to \
             finish setting up “Count mail coming back from the signature server as mail from \
             inside your organization” in Cumulas Signatures."
                .into()
        }
        (CheckState::NotApplied, Some(CheckNote::TooSoon)) => {
            return Some(format!(
                "Your organization's setup changed {} before this check. New rules can take 30 \
                 minutes or more to start working, and auto-labeling policies up to a day. \
                 Check again later.",
                changed.as_deref().unwrap_or("shortly")
            ));
        }
        (CheckState::NotApplied, Some(CheckNote::OtherLabel)) => {
            let title = result
                .other_label_id
                .as_deref()
                .and_then(|id| catalog.and_then(|c| c.label(id)))
                .map_or_else(
                    || "a different label of your organization".to_owned(),
                    CatalogLabel::title,
                );
            format!(
                "It arrived labeled {title} instead.{}",
                if result.other_label_on_control {
                    " Your organization puts that label on all mail."
                } else {
                    ""
                }
            )
        }
        (CheckState::NotApplied, _) => "The test message arrived without it, as far as Cumulas \
                                        Mail can tell. New mail flow rules can take 30 minutes \
                                        or more to start working, and auto-labeling policies up \
                                        to a day."
            .into(),
        (CheckState::Unknown, Some(CheckNote::OtherRule)) => {
            "Another rule in your organization already protects or labels this mail, so Cumulas \
             Mail can't tell whether its own rule works."
                .into()
        }
        (CheckState::Unknown, Some(CheckNote::HeaderKept)) => {
            "The test message arrived encrypted, but your organization's rule didn't remove the \
             x-cumulas-protect header, so another rule may have encrypted it. Ask your IT team \
             to check the rule."
                .into()
        }
        (CheckState::Unknown, Some(CheckNote::NotArrived)) => format!(
            "The test message didn't arrive in your Inbox within {CHECK_TIMEOUT_MINUTES} minutes."
        ),
        (CheckState::Unknown, Some(CheckNote::OtherProtection)) => {
            "The test message arrived protected another way (S/MIME, or as a link to Microsoft's \
             encrypted message portal), so Cumulas Mail can't tell whether your organization's \
             rule did it. Open the test message to see."
                .into()
        }
        (CheckState::Unknown, Some(CheckNote::LabelUnreadable)) => {
            "The test message arrived encrypted, but Cumulas Mail couldn't read its label. Open \
             the test message to see which label it has."
                .into()
        }
        _ => return None,
    };
    Some(match changed {
        Some(ago) if matches!(result.state, CheckState::NotApplied | CheckState::Unknown) => {
            format!(
                "{note} Your organization's setup changed {ago} before this check; if that's \
                 under an hour (a day for labels), check again later."
            )
        }
        _ => note,
    })
}

/// The lines of what an arrived message showed, after its first.
fn evidence_lines(out: &mut Vec<String>, evidence: &CheckEvidence, catalog: Option<&LabelCatalog>) {
    out.push(format!(
        "  Message-ID: {}",
        said(evidence.message_id.as_deref())
    ));
    out.push(format!(
        "  Request still on the message when it arrived: {}",
        yes_no(evidence.request_arrived)
    ));
    if let Some(stamped) = &evidence.label_stamped {
        out.push(format!("  Arrived with x-cumulas-label-stamped: {stamped}"));
    }
    out.push(format!(
        "  Relay: {} · AuthAs: {} · Direction: {}",
        yes_no(evidence.relay_stamped),
        said(evidence.auth_as.as_deref()),
        said(evidence.directionality.as_deref())
    ));
    out.push(format!(
        "  Arrived: {} · Content-Class: {} · Message class: {} · message.rpmsg: {}",
        kind_words(evidence.detected),
        said(evidence.content_class.as_deref()),
        said(evidence.message_class.as_deref()),
        yes_no(evidence.rpmsg_attachment)
    ));
    let labels: Vec<String> = evidence
        .own_labels
        .iter()
        .map(|l| {
            format!(
                "{}, {}",
                label_words(catalog, &l.id),
                if l.enabled { "enabled" } else { "not enabled" }
            )
        })
        .collect();
    out.push(format!(
        "  Labels on it: {} · other organizations' labels: {}",
        if labels.is_empty() {
            "none".into()
        } else {
            labels.join("; ")
        },
        evidence.foreign_labels
    ));
}

/// "Copy details for IT": what an account's last checks found, for its
/// organization's IT team. Never a message's text or subject; the test
/// messages' Message-IDs and what Microsoft 365 said of them. Every value in
/// a command is quoted with [`ps_quote`].
pub fn check_details(input: &CheckDetailsInput<'_>) -> String {
    let catalog = input.catalog;
    // An address is far shorter than a Message-ID's limit.
    let email = clean_value(input.email, MESSAGE_ID_CHARS).unwrap_or_default();
    let mut out = vec![
        format!(
            "Cumulas Mail {}: setup check details for {email}",
            input.version
        ),
        format!(
            "Organization (tenant): {}",
            clean_value(input.tenant, EVIDENCE_VALUE_CHARS).unwrap_or_default()
        ),
        "These details include no message text. They include the test messages' Message-IDs \
         and what Microsoft 365 said about how it handled them."
            .into(),
        String::new(),
    ];
    let mut message_ids: Vec<&str> = Vec::new();
    let control = input.control.and_then(|c| c.outcome.evidence.as_ref());
    match input.control {
        Some(last) => {
            let found = control
                .and_then(|e| e.found_after_secs)
                .map(|secs| format!(", found within {secs} s"))
                .unwrap_or_default();
            out.push(format!(
                "Control message (no request), sent {}{found}",
                utc(last.sent_at)
            ));
            match control {
                Some(evidence) => {
                    out.push(format!(
                        "  Message-ID: {}",
                        said(evidence.message_id.as_deref())
                    ));
                    out.push(format!(
                        "  Went through the Cumulas Signatures relay (X-Signature-Stamped): {}",
                        yes_no(evidence.relay_stamped)
                    ));
                    out.push(format!(
                        "  Counted as from inside your organization \
                         (X-MS-Exchange-Organization-AuthAs): {}",
                        said(evidence.auth_as.as_deref())
                    ));
                    out.push(format!(
                        "  Direction (X-MS-Exchange-Organization-MessageDirectionality): {}",
                        said(evidence.directionality.as_deref())
                    ));
                    out.push(format!(
                        "  Arrived: {}; label of your organization: {}",
                        kind_words(evidence.detected),
                        first_own_label(evidence)
                            .map_or_else(|| "none".into(), |id| label_words(catalog, id))
                    ));
                    out.push(format!(
                        "  Headers read from: {}",
                        match evidence.headers_from {
                            HeadersFrom::Transport => "the message's transport headers",
                            HeadersFrom::Graph => "Microsoft Graph",
                            HeadersFrom::Unread => "not read",
                        }
                    ));
                }
                None => out.push("  Arrived: not kept".into()),
            }
        }
        None => out.push("Control message (no request): not found".into()),
    }
    out.push(format!(
        "Path: {}",
        match control {
            None => "not known",
            Some(e) if !e.relay_stamped => "no relay",
            Some(e) if relay_not_internal(e) => "relay present, return not internal",
            Some(e) if e.auth_as.is_some() => "relay present, return internal",
            Some(_) => "relay present, return not known",
        }
    ));
    if let Some(id) = control.and_then(|e| e.message_id.as_deref()) {
        message_ids.push(id);
    }
    out.push(String::new());

    for item in &input.items {
        let label = matches!(item.item, CheckItem::Label { .. });
        out.push(format!(
            "{}: {}",
            item.title,
            check_row_words(&item.result, label)
        ));
        if let Some(note) = check_note_words(&item.result, &item.item, catalog) {
            out.push(format!("  Note: {note}"));
        }
        let record = &item.record;
        // A check on its way: what the one before it kept isn't its.
        if item.result.state == CheckState::Waiting
            || (record.at.is_none() && record.sent_at.is_none())
        {
            continue;
        }
        let sent = record.sent_at.map_or_else(|| "not said".into(), utc);
        out.push(match &item.item {
            CheckItem::Action { action } => format!(
                "  Sent: {PROTECT_HEADER}: {}, {sent}",
                action.header_value()
            ),
            CheckItem::Label { label_id } => format!("  Sent: {LABEL_HEADER}: {label_id}, {sent}"),
        });
        match &record.evidence {
            Some(evidence) => {
                evidence_lines(&mut out, evidence, catalog);
                if let Some(id) = evidence.message_id.as_deref() {
                    message_ids.push(id);
                }
            }
            None => out.push("  Arrived: not kept".into()),
        }
    }

    let changed = |at: Option<DateTime<Utc>>| at.map_or_else(|| "never".into(), utc);
    out.push(String::new());
    out.push(format!(
        "Setup changed on this computer: rules {}, auto-labeling policies {}",
        changed(input.settle.rules_changed_at),
        changed(input.settle.policies_changed_at)
    ));
    out.push(String::new());
    out.push("For your IT team, in Exchange Online PowerShell:".into());
    let address = ps_quote(&email);
    out.push(format!(
        "  Get-MessageTraceV2 -SenderAddress {address} -RecipientAddress {address} -Subject {} \
         -SubjectFilterType StartsWith -StartDate (Get-Date).AddDays(-2) -EndDate (Get-Date)",
        ps_quote(CHECK_SUBJECT_PREFIX)
    ));
    let mut listed: Vec<&str> = Vec::new();
    for id in message_ids {
        if listed.contains(&id) {
            continue;
        }
        listed.push(id);
        out.push(format!(
            "  Get-MailDetailTransportRuleReport -MessageId {} -EventType \
             TransportRuleHits,TransportRuleActionHits -StartDate (Get-Date).AddDays(-2) \
             -EndDate (Get-Date)",
            ps_quote(id)
        ));
    }
    out.push(format!("Guide: {CHECK_GUIDE}"));
    let mut text = out.join("\n");
    text.push('\n');
    text
}

// --- Diagnostics -------------------------------------------------------------

/// The kind of account a message is in, as diagnostics name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticsAccount {
    MicrosoftWork,
    MicrosoftPersonal,
    Imap,
}

/// What [`diagnostics`] describes: one message's cached content and what
/// was made of it.
pub struct DiagnosticsInput<'a> {
    pub account: DiagnosticsAccount,
    pub folder_role: FolderRole,
    /// As cached: the raw body, the attachments' metadata and the signals.
    pub content: &'a MessageContent,
    pub protection: Option<&'a Protection>,
    /// The account's organization (work and school accounts).
    pub tenant_id: Option<&'a str>,
    /// The account's labels, once listed.
    pub catalog: Option<&'a LabelCatalog>,
}

/// A value from the message fit for the diagnostics: one line, at most 100
/// characters. Only values that name a format or a mechanism go through
/// here, never the message's text or addresses.
fn shown_value(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .filter(|c| !c.is_control() && !is_bidi_control(*c))
        .take(100)
        .collect();
    match cleaned.trim() {
        "" => "none".into(),
        v => v.to_owned(),
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

/// `N bytes` below 1 KiB, whole KB below 1 MiB, MB with one decimal above.
fn shown_size(size: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    if size < KIB {
        format!("{size} bytes")
    } else if size < MIB {
        format!("{} KB", size / KIB)
    } else {
        format!("{:.1} MB", size as f64 / MIB as f64)
    }
}

/// What the app knows of a message's protection and label, for a bug
/// report: from the cached signals and the attachments' metadata only.
/// Never the subject, addresses, text, links, or the names of files other
/// than the protection's own.
pub fn diagnostics(input: &DiagnosticsInput<'_>) -> String {
    let content = input.content;
    let signals = &content.signals;
    let unread = signals.properties_unavailable;
    let property = |value: Option<&String>| match value {
        Some(v) => shown_value(v),
        None if unread => "not read".into(),
        None => "none".into(),
    };
    let mut out = Vec::new();
    out.push(format!(
        "Cumulas Mail {}: protection details of one message",
        env!("CARGO_PKG_VERSION")
    ));
    out.push(
        "These details don't include the message's subject, addresses, text, links, or the \
         names of its files other than the protection's own."
            .into(),
    );
    out.push(String::new());
    out.push(format!(
        "Account: {}",
        match input.account {
            DiagnosticsAccount::MicrosoftWork => "Microsoft 365 (work or school)",
            DiagnosticsAccount::MicrosoftPersonal => "Microsoft 365 (personal)",
            DiagnosticsAccount::Imap => "IMAP",
        }
    ));
    out.push(format!("Folder: {}", input.folder_role.as_str()));
    let protection = input.protection;
    out.push(format!(
        "Detected: {}",
        kind_words(protection.map(|p| p.kind))
    ));
    out.push(format!(
        "Content hidden in a container: {}",
        yes_no(protection.is_some_and(|p| p.content_hidden))
    ));
    let portal_host = protection
        .and_then(|p| p.portal_url.as_deref())
        .and_then(|u| Url::parse(u).ok())
        .and_then(|u| u.host_str().map(str::to_owned));
    out.push(match portal_host {
        Some(host) => format!("Portal link: yes ({})", shown_value(&host)),
        None => "Portal link: no".into(),
    });
    out.push(format!(
        "Content-Class: {}",
        property(signals.content_class.as_ref())
    ));
    out.push(format!(
        "Message class: {}",
        signals
            .message_class
            .as_deref()
            .map_or_else(|| "none".into(), shown_value)
    ));
    let smime = smime_signal(signals)
        || content.attachments.iter().any(|a| is_smime_name(&a.name))
        || protection.is_some_and(|p| {
            matches!(
                p.kind,
                ProtectionKind::SmimeEncrypted | ProtectionKind::SmimeSigned
            )
        });
    out.push(format!(
        "S/MIME content: {}",
        match (smime, signals.smime_content) {
            (_, Some(SmimeContent::Signed)) => "signed",
            (_, Some(SmimeContent::Enveloped)) => "enveloped",
            (true, None) => "not read",
            (false, None) => "not applicable",
        }
    ));
    out.push(format!(
        "MIME type: {}",
        signals
            .mime_type
            .as_deref()
            .map_or_else(|| "none".into(), shown_value)
    ));
    out.push(format!(
        "Server says it has attachments: {}",
        signals.has_attachments_flag.map_or("not said", yes_no)
    ));
    let html = content
        .body_html
        .as_deref()
        .filter(|h| !h.trim().is_empty());
    let text = content
        .body_text
        .as_deref()
        .filter(|t| !t.trim().is_empty());
    out.push(match (html, text) {
        (Some(html), _) => format!("Body: HTML, {} characters", html.chars().count()),
        (None, Some(text)) => format!("Body: text, {} characters", text.chars().count()),
        (None, None) => "Body: empty".into(),
    });
    out.push(format!(
        "Body read from the message's MIME: {}",
        yes_no(signals.body_from_mime)
    ));
    out.push(if unread {
        format!(
            "Properties read: no ({} tries)",
            signals.properties_attempts
        )
    } else {
        "Properties read: yes".into()
    });
    out.push(format!("Attachments: {}", content.attachments.len()));
    let containers: &[String] = protection.map_or(&[], |p| &p.container_ids);
    for attachment in &content.attachments {
        let container = containers.contains(&attachment.id);
        let name = if container && is_container_name(&attachment.name) {
            shown_value(&attachment.name)
        } else {
            "(name left out)".into()
        };
        let mut line = format!(
            "  - {name} · {} · {}",
            shown_value(&base_type(&attachment.content_type)),
            shown_size(attachment.size)
        );
        if container {
            line.push_str(" · protection file");
        }
        out.push(line);
    }
    out.push(format!(
        "Label header: {}",
        match (
            signals.msip_labels.as_deref(),
            signals.msip_labels_source.as_deref()
        ) {
            (Some(_), Some("property")) => "from the message property",
            (Some(_), _) => "from the header",
            (None, _) if unread => "not read",
            (None, _) => "none",
        }
    ));
    for label in parse_msip_labels(signals.msip_labels.as_deref().unwrap_or_default()) {
        let organization = match (input.tenant_id, label.site_id.as_deref()) {
            (Some(tenant), Some(site)) => yes_no(site.trim().eq_ignore_ascii_case(tenant)),
            (Some(_), None) => "no",
            (None, _) => "unknown",
        };
        let listed = match input.catalog {
            Some(catalog) => yes_no(catalog.label(&label.id).is_some()),
            None => "no list",
        };
        out.push(format!(
            "  - {} · this organization: {organization} · enabled: {} · method: {} · in your \
             labels: {listed}",
            label.id,
            yes_no(label.enabled),
            label
                .method
                .as_deref()
                .map_or_else(|| "not said".into(), shown_value)
        ));
    }
    let action = match signals.requested_action.as_deref() {
        Some(value) => ProtectAction::from_header_value(value)
            .map_or("unrecognized", ProtectAction::header_value),
        None if unread => "not read",
        None => "none",
    };
    let label = match signals.requested_label.as_deref().map(str::trim) {
        Some(value) if is_guid(value) => value.to_ascii_lowercase(),
        Some(_) => "unrecognized".into(),
        None if unread => "not read".into(),
        None => "none".into(),
    };
    out.push(format!(
        "Requested by Cumulas Mail: {action} · label: {label}"
    ));
    let said = |value: Option<&String>| value.map_or_else(|| "not said".into(), |v| shown_value(v));
    out.push(format!(
        "Relay (X-Signature-Stamped): {}",
        yes_no(signals.relay_stamped)
    ));
    out.push(format!(
        "Counted as internal (AuthAs): {}",
        said(signals.auth_as.as_ref())
    ));
    out.push(format!(
        "Direction: {}",
        said(signals.directionality.as_ref())
    ));
    let mut sources: Vec<&str> = Vec::new();
    for source in [
        signals.requested_action_source.as_deref(),
        signals.requested_label_source.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        let source = if source == "property" {
            "the message property"
        } else {
            "the header"
        };
        if !sources.contains(&source) {
            sources.push(source);
        }
    }
    out.push(format!(
        "Request read from: {}",
        match sources.as_slice() {
            [] if unread => "not read".into(),
            [] => "none".into(),
            sources => sources.join(" and "),
        }
    ));
    let mut text = out.join("\n");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests;
