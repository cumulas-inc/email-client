//! MIME handling for incoming mail: summaries from headers and
//! `BODYSTRUCTURE`, previews from partial body fetches, and full message
//! bodies with attachments.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use async_imap::imap_proto::{BodyContentCommon, BodyStructure, ContentEncoding};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::{DateTime, Utc};
use mail_parser::decoders::base64::base64_decode;
use mail_parser::decoders::quoted_printable::quoted_printable_decode;
use mail_parser::{Encoding, HeaderValue, MessageParser, MessagePart, MimeHeaders, PartType};

use crate::html::{html_to_text, make_preview, text_to_html};
use crate::model::{AttachmentMeta, EmailAddress, Importance};
use crate::providers::MessageContent;

/// Length of message previews, in characters.
const PREVIEW_CHARS: usize = 200;

/// What the message list needs from a message's `BODYSTRUCTURE`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct StructureInfo {
    pub has_attachments: bool,
    /// The body part to build the preview from.
    pub preview: Option<PreviewPart>,
}

/// A text body part, located by its IMAP section (`TEXT`, `1`, `1.2`, ...).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct PreviewPart {
    pub section: String,
    pub is_html: bool,
    pub charset: Option<String>,
    pub encoding: String,
    /// Encoded size in bytes.
    pub size: u32,
}

impl PreviewPart {
    /// The `BODY.PEEK` item fetching enough of the part for a preview. Parts
    /// of typical size are fetched whole; partial fetches are kept for large
    /// ones because some servers (e.g. GreenMail) answer them malformed.
    pub(super) fn fetch_item(&self) -> String {
        const WHOLE_UP_TO: u32 = 32 * 1024;
        // HTML needs more bytes to get past markup and CSS.
        let partial = if self.is_html { 16 * 1024 } else { 4096 };
        if self.size <= WHOLE_UP_TO {
            format!("BODY.PEEK[{}]", self.section)
        } else {
            format!("BODY.PEEK[{}]<0.{partial}>", self.section)
        }
    }
}

/// Finds attachments and the best preview part (first plain-text body part,
/// else the first HTML one). Nested messages count as attachments and are not
/// descended into.
pub(super) fn analyze_structure(structure: &BodyStructure<'_>) -> StructureInfo {
    let mut walk = StructureWalk::default();
    walk.visit(structure, &mut Vec::new(), false);
    StructureInfo {
        has_attachments: walk.has_attachments,
        preview: walk.plain.or(walk.html),
    }
}

#[derive(Default)]
struct StructureWalk {
    has_attachments: bool,
    plain: Option<PreviewPart>,
    html: Option<PreviewPart>,
}

impl StructureWalk {
    fn visit(&mut self, part: &BodyStructure<'_>, path: &mut Vec<u32>, in_related: bool) {
        let (common, other) = match part {
            BodyStructure::Multipart { common, bodies, .. } => {
                let related = common.ty.subtype.eq_ignore_ascii_case("related");
                for (i, child) in bodies.iter().enumerate() {
                    path.push(u32::try_from(i + 1).unwrap_or(u32::MAX));
                    self.visit(child, path, in_related || related);
                    path.pop();
                }
                return;
            }
            BodyStructure::Message { .. } => {
                self.has_attachments = true;
                return;
            }
            BodyStructure::Text { common, other, .. }
            | BodyStructure::Basic { common, other, .. } => (common, other),
        };
        let ty = &common.ty;
        let is_attachment_disposition = common
            .disposition
            .as_ref()
            .is_some_and(|d| d.ty.eq_ignore_ascii_case("attachment"));
        let is_text = ty.ty.eq_ignore_ascii_case("text");
        let is_body_text = is_text
            && !is_attachment_disposition
            && (ty.subtype.eq_ignore_ascii_case("plain")
                || ty.subtype.eq_ignore_ascii_case("html"));
        if is_body_text {
            let is_html = ty.subtype.eq_ignore_ascii_case("html");
            let slot = if is_html {
                &mut self.html
            } else {
                &mut self.plain
            };
            if slot.is_none() {
                *slot = Some(PreviewPart {
                    section: if path.is_empty() {
                        "TEXT".to_owned()
                    } else {
                        path.iter()
                            .map(u32::to_string)
                            .collect::<Vec<_>>()
                            .join(".")
                    },
                    is_html,
                    charset: param(common, "charset"),
                    encoding: encoding_name(&other.transfer_encoding),
                    size: other.octets,
                });
            }
            return;
        }
        let is_signature = ty.ty.eq_ignore_ascii_case("application")
            && matches!(
                ty.subtype.to_ascii_lowercase().as_str(),
                "pgp-signature" | "pkcs7-signature" | "x-pkcs7-signature"
            );
        let is_other_text =
            is_text && !is_attachment_disposition && param(common, "name").is_none();
        // Images inside multipart/related are inline parts of the HTML body.
        if is_attachment_disposition || !(in_related || is_signature || is_other_text) {
            self.has_attachments = true;
        }
    }
}

fn param(common: &BodyContentCommon<'_>, name: &str) -> Option<String> {
    common
        .ty
        .params
        .as_ref()?
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.to_string())
}

fn encoding_name(encoding: &ContentEncoding<'_>) -> String {
    match encoding {
        ContentEncoding::SevenBit => "7bit".to_owned(),
        ContentEncoding::EightBit => "8bit".to_owned(),
        ContentEncoding::Binary => "binary".to_owned(),
        ContentEncoding::Base64 => "base64".to_owned(),
        ContentEncoding::QuotedPrintable => "quoted-printable".to_owned(),
        ContentEncoding::Other(other) => other.to_ascii_lowercase(),
    }
}

/// Builds a preview from the first bytes of a body part.
pub(super) fn decode_preview(part: &PreviewPart, data: &[u8]) -> String {
    let mut entity = format!(
        "Content-Type: text/{}",
        if part.is_html { "html" } else { "plain" }
    );
    if let Some(charset) = &part.charset {
        entity.push_str(&format!(
            "; charset=\"{}\"",
            charset.replace(['"', '\r', '\n'], "")
        ));
    }
    entity.push_str(&format!(
        "\r\nContent-Transfer-Encoding: {}\r\n\r\n",
        part.encoding.replace(['\r', '\n'], "")
    ));
    let mut entity = entity.into_bytes();
    entity.extend_from_slice(&complete_prefix(data, &part.encoding));
    let Some(message) = MessageParser::default().parse(&entity) else {
        return String::new();
    };
    let text = match message.parts.first().map(|p| &p.body) {
        Some(PartType::Html(html)) => html_to_text(html),
        Some(PartType::Text(text)) => text.to_string(),
        _ => return String::new(),
    };
    preview_from_text(&text)
}

/// Cuts a truncated transfer-encoded prefix to something that decodes.
fn complete_prefix<'a>(data: &'a [u8], encoding: &str) -> Cow<'a, [u8]> {
    match encoding {
        "base64" => {
            let mut chars: Vec<u8> = data
                .iter()
                .copied()
                .filter(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
                .collect();
            chars.truncate(chars.len() - chars.len() % 4);
            Cow::Owned(chars)
        }
        "quoted-printable" => {
            // Drop an escape sequence cut in half.
            let tail = data.len().saturating_sub(2);
            match data[tail..].iter().position(|b| *b == b'=') {
                Some(at) => Cow::Borrowed(&data[..tail + at]),
                None => Cow::Borrowed(data),
            }
        }
        _ => Cow::Borrowed(data),
    }
}

/// One-line preview of a plain-text body, without quoted lines.
fn preview_from_text(text: &str) -> String {
    let unquoted: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim_start().starts_with('>'))
        .collect();
    let preview = make_preview(&unquoted.join("\n"), PREVIEW_CHARS);
    // A multi-byte character cut by the partial fetch decodes as U+FFFD.
    preview.trim_end_matches(['\u{fffd}', ' ']).to_owned()
}

/// Header fields shown in the message list and reading pane.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct HeaderInfo {
    pub subject: String,
    pub from: Option<EmailAddress>,
    pub to: Vec<EmailAddress>,
    pub cc: Vec<EmailAddress>,
    pub bcc: Vec<EmailAddress>,
    pub reply_to: Vec<EmailAddress>,
    pub date: Option<DateTime<Utc>>,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub importance: Importance,
    /// `multipart/mixed` messages usually carry attachments; used when the
    /// structure is unavailable.
    pub is_mixed: bool,
}

impl Default for HeaderInfo {
    fn default() -> Self {
        Self {
            subject: String::new(),
            from: None,
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            reply_to: Vec::new(),
            date: None,
            message_id: None,
            in_reply_to: None,
            references: Vec::new(),
            importance: Importance::Normal,
            is_mixed: false,
        }
    }
}

impl HeaderInfo {
    /// The thread a message belongs to: the first message it references, else
    /// the message it replies to, else itself.
    pub(super) fn conversation_id(&self) -> Option<String> {
        self.references
            .first()
            .or(self.in_reply_to.as_ref())
            .or(self.message_id.as_ref())
            .cloned()
    }
}

/// Parses a header block (`BODY[HEADER.FIELDS (...)]`).
pub(super) fn parse_headers(raw: &[u8]) -> HeaderInfo {
    MessageParser::default()
        .parse_headers(raw)
        .map(|message| header_info(&message))
        .unwrap_or_default()
}

fn header_info(message: &mail_parser::Message<'_>) -> HeaderInfo {
    HeaderInfo {
        subject: message.subject().unwrap_or_default().trim().to_owned(),
        from: addresses(message.from()).into_iter().next(),
        to: addresses(message.to()),
        cc: addresses(message.cc()),
        bcc: addresses(message.bcc()),
        reply_to: addresses(message.reply_to()),
        date: message
            .date()
            .and_then(|d| DateTime::from_timestamp(d.to_timestamp(), 0)),
        message_id: message.message_id().map(angle_brackets),
        in_reply_to: ids(message.in_reply_to()).into_iter().next(),
        references: ids(message.references()),
        importance: importance(message),
        is_mixed: message.content_type().is_some_and(|ct| {
            ct.ctype().eq_ignore_ascii_case("multipart")
                && ct
                    .subtype()
                    .is_some_and(|s| s.eq_ignore_ascii_case("mixed"))
        }),
    }
}

fn addresses(value: Option<&mail_parser::Address<'_>>) -> Vec<EmailAddress> {
    value
        .map(|list| {
            list.iter()
                .filter_map(|addr| {
                    let address = addr.address()?.trim();
                    (!address.is_empty()).then(|| EmailAddress::new(addr.name(), address))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn ids(value: &HeaderValue<'_>) -> Vec<String> {
    match value {
        HeaderValue::Text(id) => vec![angle_brackets(id)],
        HeaderValue::TextList(list) => list.iter().map(|id| angle_brackets(id)).collect(),
        _ => Vec::new(),
    }
}

/// Message ids are kept in their header form, `<local@domain>`.
pub(super) fn angle_brackets(id: &str) -> String {
    let id = id.trim().trim_start_matches('<').trim_end_matches('>');
    format!("<{id}>")
}

fn importance(message: &mail_parser::Message<'_>) -> Importance {
    let raw = |name: &str| {
        message
            .header_raw(name)
            .map(|v| v.trim().to_ascii_lowercase())
    };
    if let Some(value) = raw("Importance") {
        match value.as_str() {
            "high" => return Importance::High,
            "low" => return Importance::Low,
            _ => {}
        }
    }
    if let Some(value) = raw("X-Priority") {
        match value.chars().next() {
            Some('1' | '2') => return Importance::High,
            Some('4' | '5') => return Importance::Low,
            _ => {}
        }
    }
    match raw("Priority").as_deref() {
        Some("urgent") => Importance::High,
        Some("non-urgent") => Importance::Low,
        _ => match raw("X-MSMail-Priority").as_deref() {
            Some("high") => Importance::High,
            Some("low") => Importance::Low,
            _ => Importance::Normal,
        },
    }
}

/// Parses a complete message (`BODY[]`) for the reading pane. Attachment ids
/// are IMAP section paths (`2`, `1.3`), usable with [`attachment_content`].
pub(super) fn parse_message(raw: &[u8]) -> MessageContent {
    let Some(message) = MessageParser::default().parse(raw) else {
        // Not MIME at all: show it as text rather than failing.
        return MessageContent {
            body_text: Some(String::from_utf8_lossy(raw).into_owned()),
            ..Default::default()
        };
    };
    let headers = header_info(&message);
    let paths = section_paths(&message);

    let html_parts: Vec<&MessagePart<'_>> = message.html_bodies().collect();
    let has_html = html_parts
        .iter()
        .any(|p| matches!(p.body, PartType::Html(_)));
    let mut body_html = has_html.then(|| {
        html_parts
            .iter()
            .filter_map(|part| match &part.body {
                PartType::Html(html) => Some(html.to_string()),
                PartType::Text(text) => Some(text_to_html(text)),
                // Inline images without a Content-ID are listed as attachments.
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    });

    let plain: Vec<&str> = message
        .text_bodies()
        .filter_map(|part| match &part.body {
            PartType::Text(text) => Some(text.as_ref()),
            _ => None,
        })
        .collect();
    let body_text = if !plain.is_empty() {
        Some(plain.join("\n\n"))
    } else {
        body_html.as_deref().map(html_to_text)
    };

    let mut inlined = HashSet::new();
    if let Some(html) = &mut body_html {
        *html = inline_cid_images(html, &message, &mut inlined);
    }

    let attachments = message
        .attachments
        .iter()
        .filter_map(|&id| {
            let part = message.parts.get(id as usize)?;
            let content_id = part.content_id().map(strip_angle_brackets);
            Some(AttachmentMeta {
                id: paths.get(&id)?.clone(),
                name: attachment_name(part),
                content_type: content_type(part),
                size: part.len() as u64,
                is_inline: content_id
                    .as_ref()
                    .is_some_and(|cid| inlined.contains(&cid.to_ascii_lowercase())),
                content_id,
            })
        })
        .collect();

    MessageContent {
        body_html,
        body_text,
        bcc: headers.bcc,
        reply_to: headers.reply_to,
        attachments,
        internet_message_id: headers.message_id,
        in_reply_to: headers.in_reply_to,
        references: headers.references,
    }
}

fn strip_angle_brackets(id: &str) -> String {
    id.trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .to_owned()
}

fn attachment_name(part: &MessagePart<'_>) -> String {
    if let Some(name) = part.attachment_name().filter(|n| !n.trim().is_empty()) {
        return name.trim().to_owned();
    }
    match &part.body {
        PartType::Message(nested) => {
            let subject = nested.subject().unwrap_or("message").trim();
            let safe: String = subject
                .chars()
                .map(|c| {
                    if matches!(c, '/' | '\\' | ':') {
                        '_'
                    } else {
                        c
                    }
                })
                .collect();
            format!("{}.eml", if safe.is_empty() { "message" } else { &safe })
        }
        _ => "attachment".to_owned(),
    }
}

fn content_type(part: &MessagePart<'_>) -> String {
    match part.content_type() {
        Some(ct) => match ct.subtype() {
            Some(sub) => format!("{}/{}", ct.ctype(), sub).to_ascii_lowercase(),
            None => ct.ctype().to_ascii_lowercase(),
        },
        None => match part.body {
            PartType::Message(_) => "message/rfc822".to_owned(),
            PartType::Text(_) => "text/plain".to_owned(),
            PartType::Html(_) => "text/html".to_owned(),
            _ => "application/octet-stream".to_owned(),
        },
    }
}

/// IMAP section path of every part below the root (`1`, `2.1`, ...). A
/// non-multipart message has the single part `1`. Nested messages are leaves.
fn section_paths(message: &mail_parser::Message<'_>) -> HashMap<u32, String> {
    fn walk(
        message: &mail_parser::Message<'_>,
        children: &[u32],
        prefix: &str,
        paths: &mut HashMap<u32, String>,
    ) {
        for (i, &id) in children.iter().enumerate() {
            let path = if prefix.is_empty() {
                (i + 1).to_string()
            } else {
                format!("{prefix}.{}", i + 1)
            };
            if let Some(PartType::Multipart(grandchildren)) =
                message.parts.get(id as usize).map(|p| &p.body)
            {
                walk(message, grandchildren, &path, paths);
            }
            paths.insert(id, path);
        }
    }

    let mut paths = HashMap::new();
    match message.parts.first().map(|p| &p.body) {
        Some(PartType::Multipart(children)) => walk(message, children, "", &mut paths),
        Some(_) => {
            paths.insert(0, "1".to_owned());
        }
        None => {}
    }
    paths
}

/// Replaces `cid:` references to images of the message with `data:` URIs and
/// records the (lower-cased) Content-IDs that were inlined.
fn inline_cid_images(
    html: &str,
    message: &mail_parser::Message<'_>,
    inlined: &mut HashSet<String>,
) -> String {
    let images: HashMap<String, &MessagePart<'_>> = message
        .parts
        .iter()
        .filter(|part| {
            part.content_type()
                .is_some_and(|ct| ct.ctype().eq_ignore_ascii_case("image"))
        })
        .filter_map(|part| {
            let cid = strip_angle_brackets(part.content_id()?);
            Some((cid.to_ascii_lowercase(), part))
        })
        .collect();
    if images.is_empty() {
        return html.to_owned();
    }

    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut pos = 0;
    while let Some(found) = lower[pos..].find("cid:") {
        let start = pos + found;
        let id_start = start + 4;
        let id_end = html[id_start..]
            .find(|c: char| matches!(c, '"' | '\'' | ')' | '>' | '<') || c.is_whitespace())
            .map_or(html.len(), |end| id_start + end);
        let cid = percent_decode(&html[id_start..id_end]).to_ascii_lowercase();
        out.push_str(&html[pos..start]);
        match images.get(&cid) {
            Some(part) => {
                out.push_str(&format!(
                    "data:{};base64,{}",
                    content_type(part),
                    BASE64.encode(part.contents())
                ));
                inlined.insert(cid);
            }
            None => out.push_str(&html[start..id_end]),
        }
        pos = id_end;
    }
    out.push_str(&html[pos..]);
    out
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(byte) = value
                .get(i + 1..i + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The decoded content of the attachment `id` (a section path from
/// [`parse_message`]) of a complete message.
pub(super) fn attachment_content(raw: &[u8], id: &str) -> Option<Vec<u8>> {
    let message = MessageParser::default().parse(raw)?;
    let part_id = section_paths(&message)
        .into_iter()
        .find_map(|(part_id, path)| (path == id).then_some(part_id))?;
    let part = message.parts.get(part_id as usize)?;
    Some(part_content(&message, part))
}

/// The decoded content of a single MIME entity (`BODY[n.MIME]` followed by
/// `BODY[n]`).
pub(super) fn entity_content(entity: &[u8]) -> Option<Vec<u8>> {
    let message = MessageParser::default().parse(entity)?;
    let part = message.parts.first()?;
    Some(part_content(&message, part))
}

/// Attachment bytes. Text parts are only transfer-decoded so they keep their
/// original charset, as a downloaded file should.
fn part_content(message: &mail_parser::Message<'_>, part: &MessagePart<'_>) -> Vec<u8> {
    if let PartType::Text(_) | PartType::Html(_) = part.body {
        let raw = message
            .raw_message
            .get(part.offset_body as usize..part.offset_end as usize)
            .unwrap_or_default();
        let decoded = match part.encoding {
            Encoding::Base64 => base64_decode(raw),
            Encoding::QuotedPrintable => quoted_printable_decode(raw),
            Encoding::None => Some(raw.to_vec()),
        };
        if let Some(decoded) = decoded {
            return decoded;
        }
    }
    part.contents().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crlf(text: &str) -> Vec<u8> {
        text.replace('\n', "\r\n").into_bytes()
    }

    const ALTERNATIVE: &str = "From: \"Ann Example\" <ann@example.com>
To: Bob <bob@example.com>, carol@example.com
Cc: =?UTF-8?Q?J=C3=BCrgen?= <juergen@example.de>
Reply-To: replies@example.com
Subject: =?UTF-8?B?UXVhcnRhbHNiZXJpY2h0IMOcYmVyc2ljaHQ=?=
Date: Tue, 01 Oct 2024 09:30:00 +0200
Message-ID: <abc123@example.com>
In-Reply-To: <parent@example.com>
References: <root@example.com> <parent@example.com>
Importance: High
MIME-Version: 1.0
Content-Type: multipart/alternative; boundary=\"b1\"

--b1
Content-Type: text/plain; charset=utf-8

Hello Bob,
> quoted line
the numbers are in.
--b1
Content-Type: text/html; charset=utf-8

<html><body><p>Hello <b>Bob</b>,</p><p>the numbers are in.</p></body></html>
--b1--
";

    #[test]
    fn parses_multipart_alternative() {
        let raw = crlf(ALTERNATIVE);
        let h = parse_headers(&raw);
        assert_eq!(h.subject, "Quartalsbericht Übersicht");
        assert_eq!(
            h.from,
            Some(EmailAddress::new(Some("Ann Example"), "ann@example.com"))
        );
        assert_eq!(h.to.len(), 2);
        assert_eq!(h.to[1], EmailAddress::new(None, "carol@example.com"));
        assert_eq!(h.cc[0].name.as_deref(), Some("Jürgen"));
        assert_eq!(h.importance, Importance::High);
        assert_eq!(h.date.unwrap().to_rfc3339(), "2024-10-01T07:30:00+00:00");
        assert_eq!(h.conversation_id().as_deref(), Some("<root@example.com>"));

        let c = parse_message(&raw);
        assert!(c.body_html.as_deref().unwrap().contains("<b>Bob</b>"));
        assert!(c.body_text.as_deref().unwrap().starts_with("Hello Bob,"));
        assert_eq!(
            c.internet_message_id.as_deref(),
            Some("<abc123@example.com>")
        );
        assert_eq!(c.in_reply_to.as_deref(), Some("<parent@example.com>"));
        assert_eq!(
            c.references,
            vec!["<root@example.com>", "<parent@example.com>"]
        );
        assert_eq!(
            c.reply_to,
            vec![EmailAddress::new(None, "replies@example.com")]
        );
        assert!(c.attachments.is_empty());
    }

    #[test]
    fn plain_text_only_has_no_html() {
        let raw = crlf(
            "From: a@example.com\nSubject: plain\nContent-Type: text/plain; charset=us-ascii\n\nJust text.\n",
        );
        let content = parse_message(&raw);
        assert_eq!(content.body_html, None);
        assert_eq!(
            content.body_text.as_deref().map(str::trim),
            Some("Just text.")
        );
        assert_eq!(parse_headers(&raw).importance, Importance::Normal);
    }

    const LATIN1_QP: &str = "From: =?ISO-8859-1?Q?Andr=E9?= <andre@example.fr>
Subject: =?ISO-8859-1?Q?R=E9sum=E9?=
X-Priority: 5 (Lowest)
Content-Type: text/plain; charset=ISO-8859-1
Content-Transfer-Encoding: quoted-printable

Voil=E0 le r=E9sum=E9 de la r=E9union, tr=E8s long pour v=E9rifier les lign=
es coup=E9es.
";

    #[test]
    fn decodes_latin1_quoted_printable() {
        let raw = crlf(LATIN1_QP);
        let headers = parse_headers(&raw);
        assert_eq!(headers.subject, "Résumé");
        assert_eq!(
            headers.from.as_ref().unwrap().name.as_deref(),
            Some("André")
        );
        assert_eq!(headers.importance, Importance::Low);
        let text = parse_message(&raw).body_text.unwrap();
        assert!(text.contains("Voilà le résumé de la réunion"));
        assert!(text.contains("lignes coupées"));
    }

    const WITH_ATTACHMENTS: &str = "From: a@example.com
Subject: Report
Content-Type: multipart/mixed; boundary=\"outer\"

--outer
Content-Type: multipart/related; boundary=\"rel\"

--rel
Content-Type: text/html; charset=utf-8

<p>Chart: <img src=\"cid:chart@x\"></p><img src='CID:missing@x'>
--rel
Content-Type: image/png
Content-ID: <chart@x>
Content-Transfer-Encoding: base64

iVBORw0KGgo=
--rel--
--outer
Content-Type: application/pdf; name=\"report.pdf\"
Content-Disposition: attachment; filename=\"report.pdf\"
Content-Transfer-Encoding: base64

JVBERi0xLjQK
--outer
Content-Type: text/plain; charset=ISO-8859-1; name=\"notes.txt\"
Content-Disposition: attachment; filename=\"notes.txt\"
Content-Transfer-Encoding: quoted-printable

caf=E9
--outer
Content-Type: message/rfc822

From: c@example.com
Subject: Forwarded thing

Inner body.
--outer--
";

    #[test]
    fn inlines_cid_images_and_lists_attachments() {
        let content = parse_message(&crlf(WITH_ATTACHMENTS));
        let html = content.body_html.unwrap();
        assert!(
            html.contains("src=\"data:image/png;base64,iVBORw0KGgo=\""),
            "{html}"
        );
        // Unknown references are left alone.
        assert!(html.contains("CID:missing@x"));

        let a = &content.attachments;
        let names: Vec<_> = a.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "attachment",
                "report.pdf",
                "notes.txt",
                "Forwarded thing.eml"
            ]
        );
        assert!(a[0].is_inline);
        assert_eq!(a[0].content_id.as_deref(), Some("chart@x"));
        assert_eq!(a[0].id, "1.2");
        assert_eq!(a[1].id, "2");
        assert_eq!(a[1].content_type, "application/pdf");
        assert!(!a[1].is_inline);
        assert_eq!(a[3].content_type, "message/rfc822");

        let raw = crlf(WITH_ATTACHMENTS);
        assert_eq!(attachment_content(&raw, "2").unwrap(), b"%PDF-1.4\n");
        // Text attachments keep their charset.
        assert_eq!(attachment_content(&raw, "3").unwrap(), b"caf\xe9");
        let eml = attachment_content(&raw, "4").unwrap();
        assert!(String::from_utf8_lossy(&eml).contains("Subject: Forwarded thing"));
        assert_eq!(
            attachment_content(&raw, "1.2").unwrap(),
            b"\x89PNG\r\n\x1a\n"
        );
        assert!(attachment_content(&raw, "9").is_none());
    }

    #[test]
    fn decodes_single_entities() {
        let entity = crlf(
            "Content-Type: application/pdf\nContent-Transfer-Encoding: base64\n\nJVBERi0xLjQK\n",
        );
        assert_eq!(entity_content(&entity).unwrap(), b"%PDF-1.4\n");
    }

    #[test]
    fn nested_multipart_section_paths() {
        let raw = crlf(
            "Content-Type: multipart/mixed; boundary=a\n\n--a\nContent-Type: multipart/alternative; boundary=b\n\n--b\nContent-Type: text/plain\n\nx\n--b\nContent-Type: text/html\n\n<p>x</p>\n--b--\n--a\nContent-Type: image/gif; name=a.gif\nContent-Transfer-Encoding: base64\n\nR0lGODlh\n--a--\n",
        );
        let attachments = parse_message(&raw).attachments;
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].id, "2");
        assert_eq!(attachments[0].name, "a.gif");
        assert_eq!(attachment_content(&raw, "2").unwrap(), b"GIF89a");
    }

    fn preview_part(encoding: &str, charset: Option<&str>, is_html: bool) -> PreviewPart {
        PreviewPart {
            section: "1".into(),
            is_html,
            charset: charset.map(str::to_owned),
            encoding: encoding.into(),
            size: 100,
        }
    }

    #[test]
    fn fetches_large_parts_partially() {
        let mut part = preview_part("7bit", None, true);
        assert_eq!(part.fetch_item(), "BODY.PEEK[1]");
        part.size = 500_000;
        assert_eq!(part.fetch_item(), "BODY.PEEK[1]<0.16384>");
    }

    /// `BODYSTRUCTURE` as sent by a server, parsed like `async-imap` does.
    fn structure_of(bodystructure: &str) -> StructureInfo {
        use async_imap::imap_proto::{AttributeValue, Response};
        let line = format!("* 1 FETCH (BODYSTRUCTURE {bodystructure})\r\n");
        let (_, response) = Response::parse(line.as_bytes()).expect("parsable");
        let Response::Fetch(_, attributes) = response else {
            panic!("not a fetch response")
        };
        let Some(AttributeValue::BodyStructure(structure)) = attributes.first() else {
            panic!("no structure")
        };
        analyze_structure(structure)
    }

    #[test]
    fn analyzes_body_structures() {
        // multipart/mixed { alternative { plain, html }, csv attachment }
        let info = structure_of(
            r#"((("text" "plain" ("charset" "utf-8") NIL NIL "7bit" 114 3 NIL NIL NIL)("text" "html" ("charset" "utf-8") NIL NIL "quoted-printable" 222 5 NIL NIL NIL) "alternative" ("boundary" "a") NIL NIL)("text" "csv" NIL NIL NIL "7bit" 138 6 NIL ("attachment" ("filename" "numbers.csv")) NIL) "mixed" ("boundary" "b") NIL NIL)"#,
        );
        assert!(info.has_attachments);
        let preview = info.preview.unwrap();
        assert_eq!(preview.section, "1.1");
        assert!(!preview.is_html);
        assert_eq!(preview.charset.as_deref(), Some("utf-8"));
        assert_eq!(preview.size, 114);

        // related { html, inline image }: no attachments, HTML preview.
        let info = structure_of(
            r#"(("text" "html" ("charset" "iso-8859-1") NIL NIL "quoted-printable" 300 8 NIL NIL NIL)("image" "png" NIL "<c@x>" NIL "base64" 900 NIL ("inline" NIL) NIL) "related" ("boundary" "r") NIL NIL)"#,
        );
        assert!(!info.has_attachments);
        let preview = info.preview.unwrap();
        assert_eq!((preview.section.as_str(), preview.is_html), ("1", true));
        assert_eq!(preview.encoding, "quoted-printable");

        // Single part: the preview is the message text.
        let info = structure_of(
            r#"("text" "plain" ("charset" "us-ascii") NIL NIL "7bit" 20 1 NIL NIL NIL)"#,
        );
        assert!(!info.has_attachments);
        assert_eq!(info.preview.unwrap().section, "TEXT");

        // Signed message: the signature is not an attachment.
        let info = structure_of(
            r#"(("text" "plain" NIL NIL NIL "7bit" 20 1 NIL NIL NIL)("application" "pgp-signature" ("name" "signature.asc") NIL NIL "7bit" 400 NIL NIL NIL) "signed" ("boundary" "s") NIL NIL)"#,
        );
        assert!(!info.has_attachments);
    }

    #[test]
    fn previews_from_truncated_parts() {
        let plain = preview_part("7bit", Some("utf-8"), false);
        assert_eq!(
            decode_preview(&plain, b"Hi there,\r\n> old quote\r\nsee   you\r\n"),
            "Hi there, see you"
        );

        // Base64 cut in the middle of a quantum.
        let encoded = BASE64.encode("Grüße aus Köln, bis bald!");
        let b64 = preview_part("base64", Some("utf-8"), false);
        assert_eq!(decode_preview(&b64, &encoded.as_bytes()[..18]), "Grüße aus");
        // The first quantum ends inside "ü".
        assert_eq!(decode_preview(&b64, &encoded.as_bytes()[..6]), "Gr");

        // Quoted-printable cut inside an escape sequence.
        let qp = preview_part("quoted-printable", Some("iso-8859-1"), false);
        assert_eq!(decode_preview(&qp, b"Caf=E9 cr=E"), "Café cr");

        let html = preview_part("7bit", None, true);
        assert_eq!(
            decode_preview(
                &html,
                b"<html><head><style>p{color:red}</style></head><body><p>Hello&nbsp;world</p>"
            ),
            "Hello world"
        );

        let long = "word ".repeat(100);
        let preview = decode_preview(&plain, long.as_bytes());
        assert!(preview.chars().count() <= PREVIEW_CHARS + 1);
        assert!(preview.ends_with('…'));
    }

    #[test]
    fn header_fields_for_summaries() {
        let h = parse_headers(&crlf(
            "Subject: =?utf-8?q?Caf=C3=A9?=\nFrom: <x@example.com>\nX-Priority: 1\nContent-Type: multipart/mixed; boundary=x\n\n",
        ));
        assert_eq!(h.subject, "Café");
        assert_eq!(h.from, Some(EmailAddress::new(None, "x@example.com")));
        assert_eq!(h.importance, Importance::High);
        assert!(h.is_mixed);
        assert_eq!(h.conversation_id(), None);
    }
}
