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
//! message-level signal agrees; a label shows only for the account's own
//! organization.

use url::Url;

use crate::error::{Error, ErrorCode};
use crate::html::html_to_text;
use crate::model::{
    AttachmentMeta, FolderRole, LabelCatalog, ProtectAction, Protection, ProtectionKind, ReplyKind,
};
use crate::providers::{MessageContent, MessageSignals, SmimeContent};
use crate::signatures::config::is_guid;

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
/// The `Name` part is case-sensitive: exactly the header's lowercase name.
pub const PROTECT_PROPERTY: &str =
    "String {00020386-0000-0000-C000-000000000046} Name x-cumulas-protect";
pub const LABEL_PROPERTY: &str =
    "String {00020386-0000-0000-C000-000000000046} Name x-cumulas-label";

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

/// Most characters of a label's own name shown.
const MAX_LABEL_NAME_CHARS: usize = 64;

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
        .filter(|a| is_smime_name(&a.name) || (smime && is_smime_type(&a.content_type)))
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

fn is_portal(href: &str) -> bool {
    let Ok(mut url) = Url::parse(href) else {
        return false;
    };
    let wrapped = url
        .host_str()
        .is_some_and(|host| host.to_ascii_lowercase().ends_with(SAFE_LINKS_SUFFIX));
    if wrapped {
        let Some(inner) = url
            .query_pairs()
            .find(|(key, _)| key == "url")
            .and_then(|(_, value)| Url::parse(&value).ok())
        else {
            return false;
        };
        url = inner;
    }
    url.scheme() == "https"
        && url.port().is_none()
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
    /// As the message names it: any sender can write anything here.
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

/// A label's own name, fit to show: without control and bidi characters,
/// at most 64 characters; `None` when blank or GUID-shaped (Purview names
/// labels by GUID).
pub fn shown_label_name(name: &str) -> Option<String> {
    let cleaned: String = name
        .chars()
        .filter(|c| !c.is_control() && !is_bidi_control(*c))
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() || is_guid(cleaned) {
        return None;
    }
    if cleaned.chars().count() <= MAX_LABEL_NAME_CHARS {
        return Some(cleaned.to_owned());
    }
    let cut: String = cleaned.chars().take(MAX_LABEL_NAME_CHARS - 1).collect();
    Some(format!("{}…", cut.trim_end()))
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
        match protection.map(|p| p.kind) {
            Some(ProtectionKind::RightsManaged) => "Microsoft Purview encryption",
            Some(ProtectionKind::SmimeEncrypted) => "S/MIME encryption",
            Some(ProtectionKind::SmimeSigned) => "S/MIME signature",
            Some(ProtectionKind::PortalLink) => "encrypted message portal link",
            None => "nothing",
        }
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
    let mut text = out.join("\n");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests;
