//! Sending: building outgoing MIME messages and submitting them over SMTP.

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use lettre::message::header::{ContentTransferEncoding, ContentType, HeaderName, HeaderValue};
use lettre::message::{Attachment, Body, Mailbox, MultiPart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{Address, AsyncSmtpTransport, AsyncTransport, Message as MimeMessage, Tokio1Executor};

use super::ImapCredentials;
use super::connection::APP_PASSWORD_HINT;
use super::mime::angle_brackets;
use crate::error::{Error, Result};
use crate::html::html_to_text;
use crate::model::{EmailAddress, Importance, Message, OutgoingMessage, ReplyKind, TlsMode};

/// Connection and per-command timeout for SMTP.
const SMTP_TIMEOUT: Duration = Duration::from_secs(60);
/// Long threads are cut to their most recent ancestors, like other clients do.
const MAX_REFERENCES: usize = 20;

/// Builds the MIME message for `message`. Bcc recipients are only in the
/// envelope, never in the headers, so the copy saved to Sent does not reveal
/// them either.
pub(super) fn build_message(
    credentials: &ImapCredentials,
    message: &OutgoingMessage,
    original: Option<&Message>,
) -> Result<MimeMessage> {
    let from_address = parse_address(&credentials.email)?;
    let domain = from_address.domain().to_owned();
    let display_name = credentials.display_name.trim();
    let mut builder = MimeMessage::builder()
        .from(Mailbox::new(
            (!display_name.is_empty()).then(|| display_name.to_owned()),
            from_address,
        ))
        .subject(message.subject.trim())
        .date_now()
        .message_id(Some(format!("<{}@{domain}>", uuid::Uuid::new_v4())));
    for to in &message.to {
        builder = builder.to(mailbox(to)?);
    }
    for cc in &message.cc {
        builder = builder.cc(mailbox(cc)?);
    }
    for bcc in &message.bcc {
        builder = builder.bcc(mailbox(bcc)?);
    }

    if let Some(original) = original {
        let parent = original.internet_message_id.as_deref().map(angle_brackets);
        let mut references: Vec<String> = original
            .references
            .iter()
            .map(|r| angle_brackets(r))
            .collect();
        references.extend(parent.clone());
        references.dedup();
        let skip = references.len().saturating_sub(MAX_REFERENCES);
        if !references.is_empty() {
            builder = builder.references(references[skip..].join(" "));
        }
        // Forwards start a new conversation; only replies answer the parent.
        if message.reply_kind != Some(ReplyKind::Forward)
            && let Some(parent) = parent
        {
            builder = builder.in_reply_to(parent);
        }
    }

    match message.importance {
        Some(Importance::High) => {
            builder = builder
                .raw_header(raw_header("Importance", "high"))
                .raw_header(raw_header("X-Priority", "1 (Highest)"));
        }
        Some(Importance::Low) => {
            builder = builder
                .raw_header(raw_header("Importance", "low"))
                .raw_header(raw_header("X-Priority", "5 (Lowest)"));
        }
        Some(Importance::Normal) | None => {}
    }

    let body = MultiPart::alternative_plain_html(
        html_to_text(&message.body_html),
        html_document(&message.body_html),
    );
    let body = if message.attachments.is_empty() {
        body
    } else {
        let mut mixed = MultiPart::mixed().multipart(body);
        for attachment in &message.attachments {
            let data = BASE64.decode(attachment.data_base64.trim()).map_err(|_| {
                Error::invalid_input(format!(
                    "The attachment \"{}\" could not be read.",
                    attachment.name
                ))
            })?;
            let content_type = ContentType::parse(&attachment.content_type)
                .or_else(|_| ContentType::parse("application/octet-stream"))
                .map_err(|e| Error::internal(e.to_string()))?;
            // lettre would send ASCII files as 7bit with their line endings
            // untouched; base64 keeps every attachment byte-exact.
            let body = Body::new_with_encoding(data, ContentTransferEncoding::Base64)
                .map_err(|_| Error::internal("base64 encoding failed"))?;
            mixed =
                mixed.singlepart(Attachment::new(attachment.name.clone()).body(body, content_type));
        }
        mixed
    };
    builder
        .multipart(body)
        .map_err(|e| Error::invalid_input(format!("The message could not be built: {e}")))
}

fn raw_header(name: &'static str, value: &str) -> HeaderValue {
    HeaderValue::new(HeaderName::new_from_ascii_str(name), value.to_owned())
}

fn parse_address(address: &str) -> Result<Address> {
    address
        .trim()
        .parse()
        .map_err(|_| Error::invalid_input(format!("\"{address}\" is not a valid email address.")))
}

fn mailbox(address: &EmailAddress) -> Result<Mailbox> {
    Ok(Mailbox::new(
        address
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_owned),
        parse_address(&address.address)?,
    ))
}

/// Wraps an HTML fragment from the composer in a complete document.
fn html_document(body: &str) -> String {
    if body
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("<!doctype")
        || body.to_ascii_lowercase().contains("<html")
    {
        return body.to_owned();
    }
    format!(
        "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"></head><body>{body}</body></html>"
    )
}

fn transport(credentials: &ImapCredentials) -> Result<AsyncSmtpTransport<Tokio1Executor>> {
    let settings = &credentials.settings;
    let host = settings.smtp_host.trim();
    let parameters = || {
        TlsParameters::new(host.to_owned())
            .map_err(|e| Error::internal(format!("Could not set up TLS: {e}")))
    };
    let tls = match settings.smtp_tls {
        TlsMode::Tls => Tls::Wrapper(parameters()?),
        TlsMode::StartTls => Tls::Required(parameters()?),
        TlsMode::None => Tls::None,
    };
    Ok(
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host)
            .port(settings.smtp_port)
            .tls(tls)
            .credentials(Credentials::new(
                settings.username.clone(),
                credentials.password.clone(),
            ))
            .timeout(Some(SMTP_TIMEOUT))
            .build(),
    )
}

/// Submits a message built by [`build_message`].
pub(super) async fn send(
    credentials: &ImapCredentials,
    message: &MimeMessage,
    raw: &[u8],
) -> Result<()> {
    transport(credentials)?
        .send_raw(message.envelope(), raw)
        .await
        .map_err(|e| smtp_error(credentials, e))?;
    Ok(())
}

/// Connects and authenticates (lettre logs in as part of connecting when
/// credentials are set), then quits without sending anything.
pub(super) async fn verify(credentials: &ImapCredentials) -> Result<()> {
    let connected = transport(credentials)?
        .test_connection()
        .await
        .map_err(|e| smtp_error(credentials, e))?;
    if connected {
        Ok(())
    } else {
        Err(Error::network(format!(
            "The SMTP server {} closed the connection.",
            credentials.settings.smtp_host
        )))
    }
}

fn smtp_error(credentials: &ImapCredentials, e: lettre::transport::smtp::Error) -> Error {
    let settings = &credentials.settings;
    let host = settings.smtp_host.trim();
    if let Some(code) = e.status() {
        // 530: authentication required, 534: mechanism too weak (Gmail without
        // an app password), 535: credentials invalid.
        if matches!(code.to_string().as_str(), "530" | "534" | "535") {
            return Error::auth_required(format!(
                "The SMTP server {host} rejected the user name or password for {}. \
                 {APP_PASSWORD_HINT}",
                settings.username
            ));
        }
        return Error::provider(format!("The SMTP server {host} refused the message: {e}"));
    }
    if e.is_timeout() {
        return Error::network(format!("The SMTP server {host} did not respond in time."));
    }
    if e.is_tls() {
        return Error::network(format!(
            "Could not establish a secure connection to the SMTP server {host}: {e}"
        ));
    }
    if e.is_client() || e.is_response() {
        return Error::provider(format!("Sending through {host} failed: {e}"));
    }
    Error::network(format!(
        "Could not connect to the SMTP server {host}:{}: {e}",
        settings.smtp_port
    ))
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::super::mime::{attachment_content, parse_headers, parse_message};
    use super::*;
    use crate::model::{ImapSettings, MessageSummary, OutgoingAttachment};

    /// Plain text with bare line feeds, which must survive unchanged.
    const AGENDA: &[u8] = b"1. Coffee\n2. Plans\n";

    fn credentials() -> ImapCredentials {
        ImapCredentials {
            settings: ImapSettings {
                imap_host: "imap.example.com".into(),
                imap_port: 993,
                imap_tls: TlsMode::Tls,
                smtp_host: "smtp.example.com".into(),
                smtp_port: 465,
                smtp_tls: TlsMode::Tls,
                username: "ann@example.com".into(),
            },
            password: "secret".into(),
            display_name: "Ann Müller".into(),
            email: "ann@example.com".into(),
        }
    }

    fn outgoing() -> OutgoingMessage {
        OutgoingMessage {
            account_id: "acct".into(),
            to: vec![EmailAddress::new(Some("Bob"), "bob@example.com")],
            cc: vec![EmailAddress::new(None, "carol@example.com")],
            bcc: vec![EmailAddress::new(Some("Secret"), "dave@example.com")],
            subject: "Re: Plans für Freitag".into(),
            body_html: "<p>See you <b>Friday</b>!</p>".into(),
            attachments: vec![OutgoingAttachment {
                name: "agenda.txt".into(),
                content_type: "text/plain".into(),
                data_base64: BASE64.encode(AGENDA),
            }],
            importance: Some(Importance::High),
            reply_to_message_id: Some("local".into()),
            reply_kind: Some(ReplyKind::Reply),
        }
    }

    fn original() -> Message {
        Message {
            summary: MessageSummary {
                id: "local".into(),
                account_id: "acct".into(),
                folder_id: "f".into(),
                remote_id: "r".into(),
                conversation_id: None,
                subject: "Plans".into(),
                from: None,
                to: vec![],
                cc: vec![],
                preview: String::new(),
                received_at: Utc::now(),
                is_read: true,
                is_flagged: false,
                is_draft: false,
                has_attachments: false,
                importance: Importance::Normal,
            },
            bcc: vec![],
            reply_to: vec![],
            body_html: None,
            body_text: None,
            has_blocked_content: false,
            attachments: vec![],
            internet_message_id: Some("<parent@example.com>".into()),
            in_reply_to: None,
            references: vec!["<root@example.com>".into()],
        }
    }

    #[test]
    fn builds_reply_with_threading_headers_and_no_bcc() {
        let built = build_message(&credentials(), &outgoing(), Some(&original())).unwrap();
        let raw = built.formatted();
        let h = parse_headers(&raw);
        assert_eq!(h.subject, "Re: Plans für Freitag");
        assert_eq!(
            h.from,
            Some(EmailAddress::new(Some("Ann Müller"), "ann@example.com"))
        );
        assert_eq!(
            h.to,
            vec![EmailAddress::new(Some("Bob"), "bob@example.com")]
        );
        assert_eq!(h.cc, vec![EmailAddress::new(None, "carol@example.com")]);
        assert!(h.bcc.is_empty());
        assert!(!String::from_utf8_lossy(&raw).contains("dave@example.com"));
        assert_eq!(h.in_reply_to.as_deref(), Some("<parent@example.com>"));
        assert_eq!(h.references, ["<root@example.com>", "<parent@example.com>"]);
        assert_eq!(h.importance, Importance::High);
        assert!(h.message_id.as_deref().unwrap().ends_with("@example.com>"));
        assert!(h.date.is_some());

        let c = parse_message(&raw);
        assert!(c.body_html.as_deref().unwrap().contains("<b>Friday</b>"));
        assert_eq!(
            c.body_text.as_deref().map(str::trim),
            Some("See you Friday!")
        );
        assert_eq!(c.attachments.len(), 1);
        assert_eq!(c.attachments[0].name, "agenda.txt");
        assert_eq!(
            attachment_content(&raw, &c.attachments[0].id).unwrap(),
            AGENDA
        );

        // Bcc recipients still receive the message.
        let recipients: Vec<String> = built
            .envelope()
            .to()
            .iter()
            .map(|a| a.to_string())
            .collect();
        assert!(recipients.contains(&"dave@example.com".to_owned()));
        assert_eq!(recipients.len(), 3);
    }

    #[test]
    fn forwards_reference_but_do_not_reply() {
        let mut message = outgoing();
        message.reply_kind = Some(ReplyKind::Forward);
        message.importance = Some(Importance::Low);
        message.attachments.clear();
        let raw = build_message(&credentials(), &message, Some(&original()))
            .unwrap()
            .formatted();
        let h = parse_headers(&raw);
        assert_eq!(h.in_reply_to, None);
        assert_eq!(h.references.len(), 2);
        assert_eq!(h.importance, Importance::Low);
    }

    #[test]
    fn rejects_invalid_input() {
        let mut message = outgoing();
        message.to[0].address = "not an address".into();
        let err = build_message(&credentials(), &message, None).unwrap_err();
        assert_eq!(err.code, crate::error::ErrorCode::InvalidInput);

        let mut message = outgoing();
        message.attachments[0].data_base64 = "***".into();
        let err = build_message(&credentials(), &message, None).unwrap_err();
        assert_eq!(err.code, crate::error::ErrorCode::InvalidInput);
    }

    #[test]
    fn wraps_html_fragments() {
        assert!(html_document("<p>x</p>").starts_with("<!DOCTYPE html>"));
        assert_eq!(
            html_document("<html><body>x</body></html>"),
            "<html><body>x</body></html>"
        );
    }
}
