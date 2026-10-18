//! Protected mail on Graph: the signals a message is read with, S/MIME
//! content, sending through a draft that carries the x-cumulas-*
//! properties, and the setup check's lookups.

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{ACCOUNT, batch_responder, batched_requests, graph_error, ok_json, prefers, setup};
use crate::error::ErrorCode;
use crate::model::{
    EmailAddress, EntityKind, Importance, Message, MessageSummary, OutgoingMessage, ProtectAction,
    ProtectionKind, ReplyKind, local_id,
};
use crate::protection::{detect, has_reason};
use crate::providers::graph::client::PREFER_IMMUTABLE_ID;
use crate::providers::{HeaderRequest, MailProvider, MessageLocator, SmimeContent};

const TENANT: &str = "72f988bf-86f1-41af-91ab-2d7cd011db47";
const LABEL: &str = "6c1b3b5e-0d4f-4b1e-9d3a-2f0e6a7b8c9d";
const OTHER_LABEL: &str = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
const PROTECT_ID: &str = "String {00020386-0000-0000-C000-000000000046} Name x-cumulas-protect";
const LABEL_ID: &str = "String {00020386-0000-0000-C000-000000000046} Name x-cumulas-label";

fn locator(id: &str) -> MessageLocator {
    MessageLocator {
        folder_remote_id: "AAMkInbox".into(),
        remote_id: id.into(),
    }
}

/// The named property a batched request reads (`Content-Class`,
/// `msip_labels`, `x-cumulas-protect`, `x-cumulas-label`), if it reads one.
fn property_of(request: &Value) -> Option<String> {
    let url = request["url"].as_str()?;
    let (_, expand) = url.split_once("&$expand=")?;
    let name = expand
        .strip_prefix("singleValueExtendedProperties($filter=id eq 'String {00020386-0000-0000-C000-000000000046} Name ")?
        .strip_suffix("')")?;
    Some(name.to_owned())
}

/// Answers property reads with `values` (by property name); others are
/// left without the array, as Graph does.
fn properties(values: &[(&str, &str)]) -> impl Fn(&Value) -> (u16, Value) + Send + Sync + 'static {
    let values: Vec<(String, String)> = values
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    move |request: &Value| {
        let name = property_of(request).unwrap_or_else(|| panic!("not a property read: {request}"));
        match values.iter().find(|(k, _)| *k == name) {
            Some((_, value)) => (
                200,
                json!({
                    "id": "msg",
                    "singleValueExtendedProperties": [
                        // Graph may spell the id otherwise; only the value counts.
                        { "id": format!("String {{00020386-0000-0000-c000-000000000046}} Name {name}"), "value": value },
                    ],
                }),
            ),
            None => (200, json!({ "id": "msg" })),
        }
    }
}

/// A message's detail with this body, class and attachment flag.
fn detail(body: &str, class: Option<&str>, has_attachments: bool) -> Value {
    let mut detail = json!({
        "id": "msg-1",
        "hasAttachments": has_attachments,
        "body": { "contentType": "html", "content": body },
        "bccRecipients": [],
        "replyTo": [],
        "internetMessageId": "<a@contoso.com>",
        "internetMessageHeaders": [],
    });
    if let Some(class) = class {
        detail["singleValueExtendedProperties"] = json!([{ "id": "String 0x1a", "value": class }]);
    }
    detail
}

async fn mount_detail(server: &MockServer, detail: Value) {
    Mock::given(method("GET"))
        .and(path("/me/messages/msg-1"))
        .and(query_param(
            "$expand",
            "singleValueExtendedProperties($filter=id eq 'String 0x001A')",
        ))
        .respond_with(ok_json(detail))
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_attachments(server: &MockServer, attachments: Value) {
    Mock::given(method("GET"))
        .and(path("/me/messages/msg-1/attachments"))
        .respond_with(ok_json(json!({ "value": attachments })))
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_properties(server: &MockServer, values: &[(&str, &str)]) {
    Mock::given(method("POST"))
        .and(path("/$batch"))
        .respond_with(batch_responder(properties(values)))
        .mount(server)
        .await;
}

// --- Reading ------------------------------------------------------------------------

#[tokio::test]
async fn the_named_properties_are_read_one_after_the_other() {
    let (server, provider) = setup().await;
    let msip = format!("MSIP_Label_{LABEL}_Enabled=true; MSIP_Label_{LABEL}_SiteId={TENANT}");
    mount_detail(&server, detail("<p>Hi</p>", Some("IPM.Note"), false)).await;
    mount_properties(
        &server,
        &[
            ("Content-Class", "urn:content-classes:message"),
            ("msip_labels", &msip),
            ("x-cumulas-protect", " encrypt "),
            ("x-cumulas-label", LABEL),
        ],
    )
    .await;
    let content = provider.fetch_message(&locator("msg-1")).await.unwrap();
    let signals = content.signals;
    assert_eq!(signals.message_class.as_deref(), Some("IPM.Note"));
    assert_eq!(
        signals.content_class.as_deref(),
        Some("urn:content-classes:message")
    );
    assert_eq!(signals.msip_labels.as_deref(), Some(msip.as_str()));
    assert_eq!(signals.msip_labels_source.as_deref(), Some("property"));
    assert_eq!(signals.requested_action.as_deref(), Some("encrypt"));
    assert_eq!(signals.requested_label.as_deref(), Some(LABEL));
    assert_eq!(signals.has_attachments_flag, Some(false));
    assert!(!signals.properties_unavailable);

    // Sequential: each after the one before, in one batch.
    let sent = batched_requests(&server).await;
    assert_eq!(sent.len(), 4);
    let ids: Vec<&str> = sent.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        [
            "contentClass",
            "msipLabels",
            "requestedProtect",
            "requestedLabel"
        ]
    );
    assert_eq!(sent[0].get("dependsOn"), None);
    for i in 1..4 {
        assert_eq!(sent[i]["dependsOn"], json!([ids[i - 1]]));
    }
    assert_eq!(
        sent[3]["url"],
        "/me/messages/msg-1?$select=id&$expand=singleValueExtendedProperties($filter=id eq \
         'String {00020386-0000-0000-C000-000000000046} Name x-cumulas-label')"
    );
    assert_eq!(sent[0]["headers"]["Prefer"], PREFER_IMMUTABLE_ID);
}

#[tokio::test]
async fn headers_come_first_for_labels_and_last_for_requests() {
    let (server, provider) = setup().await;
    let mut message = detail("<p>Hi</p>", None, false);
    message["internetMessageHeaders"] = json!([
        { "name": "msip_labels", "value": "MSIP_Label_header" },
        { "name": "Content-Class", "value": "rpmsg.message" },
        { "name": "x-cumulas-protect", "value": "do-not-forward" },
        { "name": "X-Cumulas-Label", "value": OTHER_LABEL },
    ]);
    mount_detail(&server, message).await;
    mount_properties(
        &server,
        &[
            ("msip_labels", "MSIP_Label_property"),
            ("x-cumulas-protect", "encrypt"),
        ],
    )
    .await;
    // rpmsg: the attachments are listed whatever hasAttachments says.
    mount_attachments(&server, json!([])).await;
    let signals = provider
        .fetch_message(&locator("msg-1"))
        .await
        .unwrap()
        .signals;
    assert_eq!(signals.msip_labels.as_deref(), Some("MSIP_Label_header"));
    assert_eq!(signals.msip_labels_source.as_deref(), Some("header"));
    assert_eq!(signals.content_class.as_deref(), Some("rpmsg.message"));
    assert_eq!(signals.requested_action.as_deref(), Some("encrypt"));
    assert_eq!(signals.requested_label.as_deref(), Some(OTHER_LABEL));
}

#[tokio::test]
async fn failed_property_reads_dont_hold_up_reading() {
    for (status, sub_status) in [(400, 200), (500, 200), (200, 424), (200, 500)] {
        let (server, provider) = setup().await;
        mount_detail(&server, detail("<p>Hi</p>", Some("IPM.Note"), false)).await;
        if status == 200 {
            Mock::given(method("POST"))
                .and(path("/$batch"))
                .respond_with(batch_responder(move |r| {
                    if r["id"] == "msipLabels" {
                        (
                            sub_status,
                            json!({ "error": { "code": "x", "message": "x" } }),
                        )
                    } else {
                        properties(&[("Content-Class", "urn:content-classes:message")])(r)
                    }
                }))
                .mount(&server)
                .await;
        } else {
            Mock::given(method("POST"))
                .and(path("/$batch"))
                .respond_with(graph_error(status, "BadRequest", "Bad request"))
                .mount(&server)
                .await;
        }
        let content = provider.fetch_message(&locator("msg-1")).await.unwrap();
        assert_eq!(content.body_html.as_deref(), Some("<p>Hi</p>"));
        assert_eq!(content.signals.message_class.as_deref(), Some("IPM.Note"));
        assert!(
            content.signals.properties_unavailable,
            "{status} {sub_status}"
        );
        if status == 200 {
            // What could be read is kept.
            assert_eq!(
                content.signals.content_class.as_deref(),
                Some("urn:content-classes:message")
            );
        }
    }
}

#[tokio::test]
async fn a_slow_property_batch_is_given_up() {
    let (server, mut provider) = setup().await;
    provider.property_budget = Duration::from_millis(200);
    mount_detail(&server, detail("<p>Hi</p>", None, false)).await;
    Mock::given(method("POST"))
        .and(path("/$batch"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "responses": [] }))
                .set_delay(Duration::from_secs(3)),
        )
        .mount(&server)
        .await;
    let started = std::time::Instant::now();
    let content = provider.fetch_message(&locator("msg-1")).await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(content.signals.properties_unavailable);
    assert_eq!(content.body_html.as_deref(), Some("<p>Hi</p>"));
}

#[tokio::test]
async fn attachments_are_listed_for_protected_and_blank_messages() {
    for (body, class, content_class) in [
        ("<p>Read it in Outlook.</p>", None, Some("rpmsg.message")),
        (
            "<p>Signed.</p>",
            Some("IPM.Note.SMIME.MultipartSigned"),
            None,
        ),
        ("<div><br></div>", None, None),
    ] {
        let (server, provider) = setup().await;
        mount_detail(&server, detail(body, class, false)).await;
        let values: Vec<(&str, &str)> = content_class
            .map(|c| ("Content-Class", c))
            .into_iter()
            .collect();
        mount_properties(&server, &values).await;
        mount_attachments(
            &server,
            json!([{
                "@odata.type": "#microsoft.graph.fileAttachment",
                "id": "att-1",
                "name": "message.rpmsg",
                "contentType": "application/x-microsoft-rpmsg-message",
                "size": 48213,
                "isInline": false,
            }]),
        )
        .await;
        if class.is_some() {
            // A clear-signed message's MIME is read too; here it can't be.
            Mock::given(method("GET"))
                .and(path("/me/messages/msg-1/$value"))
                .respond_with(graph_error(500, "InternalServerError", "x"))
                .mount(&server)
                .await;
        }
        let content = provider.fetch_message(&locator("msg-1")).await.unwrap();
        assert_eq!(content.attachments.len(), 1, "{body}");
    }
}

/// The start of a CMS ContentInfo: signedData (0x02) or envelopedData
/// (0x03).
fn content_info(last: u8) -> Vec<u8> {
    let mut bytes = vec![0x30, 0x82, 0x40, 0x00, 0x06, 0x09];
    bytes.extend_from_slice(&[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07, last]);
    bytes.extend(std::iter::repeat_n(0xA5, 4000));
    bytes
}

#[tokio::test]
async fn the_p7m_of_an_smime_message_says_whether_its_signed_or_encrypted() {
    for (last, expected, kind) in [
        (0x02, SmimeContent::Signed, ProtectionKind::SmimeSigned),
        (
            0x03,
            SmimeContent::Enveloped,
            ProtectionKind::SmimeEncrypted,
        ),
    ] {
        let (server, provider) = setup().await;
        mount_detail(&server, detail("", Some("IPM.Note.SMIME"), true)).await;
        mount_properties(&server, &[]).await;
        mount_attachments(
            &server,
            json!([{
                "@odata.type": "#microsoft.graph.fileAttachment",
                "id": "att-p7m",
                "name": "smime.p7m",
                "contentType": "application/pkcs7-mime",
                "size": 4012,
            }]),
        )
        .await;
        Mock::given(method("GET"))
            .and(path("/me/messages/msg-1/attachments/att-p7m/$value"))
            .and(prefers(PREFER_IMMUTABLE_ID))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(content_info(last)))
            .expect(1)
            .mount(&server)
            .await;
        let content = provider.fetch_message(&locator("msg-1")).await.unwrap();
        assert_eq!(content.signals.smime_content, Some(expected));
        let protection = detect(
            &content.signals,
            &content.attachments,
            content.body_html.as_deref(),
            true,
            false,
        )
        .unwrap();
        assert_eq!(protection.kind, kind);
        assert_eq!(protection.container_ids, ["att-p7m"]);
    }
}

/// A clear-signed message: text and a PDF, signed.
fn signed_mime() -> String {
    let pdf = STANDARD.encode(b"%PDF-1.7 certificate");
    let signature = STANDARD.encode(content_info(0x02));
    format!(
        "From: it@contoso.com\r\nTo: adele@contoso.com\r\nSubject: VPN\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/signed; protocol=\"application/pkcs7-signature\"; \
         micalg=sha-256; boundary=\"s\"\r\n\r\n\
         --s\r\nContent-Type: multipart/mixed; boundary=\"m\"\r\n\r\n\
         --m\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Renew your certificate.</p>\r\n\
         --m\r\nContent-Type: application/pdf; name=\"certificate.pdf\"\r\n\
         Content-Disposition: attachment; filename=\"certificate.pdf\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\n{pdf}\r\n--m--\r\n\
         --s\r\nContent-Type: application/pkcs7-signature; name=\"smime.p7s\"\r\n\
         Content-Disposition: attachment; filename=\"smime.p7s\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\n{signature}\r\n--s--\r\n"
    )
}

#[tokio::test]
async fn clear_signed_mail_is_read_from_its_mime() {
    let (server, provider) = setup().await;
    // Exchange's form: the promoted text, and the whole signed entity in an
    // smime.p7m. No class here: the attachment's type says it.
    mount_detail(
        &server,
        detail("<p>Renew your certificate (promoted).</p>", None, true),
    )
    .await;
    mount_properties(&server, &[]).await;
    mount_attachments(
        &server,
        json!([{
            "@odata.type": "#microsoft.graph.fileAttachment",
            "id": "att-p7m",
            "name": "smime.p7m",
            "contentType": "multipart/signed",
            "size": 9000,
        }]),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/me/messages/msg-1/$value"))
        .respond_with(ResponseTemplate::new(200).set_body_string(signed_mime()))
        // The message, its two parts, and a part it doesn't have.
        .expect(4)
        .mount(&server)
        .await;
    let content = provider.fetch_message(&locator("msg-1")).await.unwrap();
    assert!(content.signals.body_from_mime);
    assert_eq!(
        content.signals.mime_type.as_deref(),
        Some("multipart/signed; protocol=application/pkcs7-signature")
    );
    assert!(
        content
            .body_html
            .as_deref()
            .unwrap()
            .contains("Renew your certificate.</p>")
    );
    let names: Vec<(&str, &str)> = content
        .attachments
        .iter()
        .map(|a| (a.id.as_str(), a.name.as_str()))
        .collect();
    assert_eq!(
        names,
        [("mime:1.2", "certificate.pdf"), ("mime:2", "smime.p7s")]
    );
    let protection = detect(
        &content.signals,
        &content.attachments,
        content.body_html.as_deref(),
        false,
        false,
    )
    .unwrap();
    assert_eq!(protection.kind, ProtectionKind::SmimeSigned);
    assert!(!protection.content_hidden);
    assert_eq!(protection.container_ids, ["mime:2"]);

    // Its parts are fetched from the MIME again.
    let pdf = provider
        .fetch_attachment(&locator("msg-1"), "mime:1.2")
        .await
        .unwrap();
    assert_eq!(pdf, b"%PDF-1.7 certificate");
    let signature = provider
        .fetch_attachment(&locator("msg-1"), "mime:2")
        .await
        .unwrap();
    assert_eq!(signature, content_info(0x02));
    for bad in ["mime:", "mime:1/../2", "mime:9"] {
        let error = provider
            .fetch_attachment(&locator("msg-1"), bad)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::NotFound, "{bad}");
    }
}

#[tokio::test]
async fn a_signed_message_whose_mime_cant_be_read_keeps_its_container() {
    let (server, provider) = setup().await;
    mount_detail(
        &server,
        detail(
            "<p>Promoted.</p>",
            Some("IPM.Note.SMIME.MultipartSigned"),
            true,
        ),
    )
    .await;
    mount_properties(&server, &[]).await;
    mount_attachments(
        &server,
        json!([{ "id": "att-p7m", "name": "smime.p7m", "contentType": "multipart/signed", "size": 9 }]),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/me/messages/msg-1/$value"))
        .respond_with(graph_error(503, "ServiceUnavailable", "x").insert_header("Retry-After", "0"))
        .mount(&server)
        .await;
    let content = provider.fetch_message(&locator("msg-1")).await.unwrap();
    assert!(!content.signals.body_from_mime);
    assert_eq!(content.body_html.as_deref(), Some("<p>Promoted.</p>"));
    let protection = detect(
        &content.signals,
        &content.attachments,
        content.body_html.as_deref(),
        false,
        false,
    )
    .unwrap();
    assert_eq!(protection.kind, ProtectionKind::SmimeSigned);
    assert!(protection.content_hidden);
    assert_eq!(protection.container_ids, ["att-p7m"]);
}

// --- Sending ------------------------------------------------------------------------

fn outgoing() -> OutgoingMessage {
    OutgoingMessage {
        account_id: ACCOUNT.into(),
        to: vec![EmailAddress::new(None, "pat@fabrikam.com")],
        cc: vec![],
        bcc: vec![],
        subject: "Atlas pricing".into(),
        body_html: "<p>Attached.</p>".into(),
        attachments: vec![],
        importance: None,
        reply_to_message_id: None,
        reply_kind: None,
        protect: Some(ProtectAction::Encrypt),
        label_id: Some(LABEL.into()),
    }
}

fn request(protect: Option<ProtectAction>, label: Option<&str>, encrypts: bool) -> HeaderRequest {
    HeaderRequest {
        protect,
        label_id: label.map(str::to_owned),
        tenant_id: TENANT.into(),
        label_encrypts: encrypts,
    }
}

fn original() -> Message {
    Message {
        summary: MessageSummary {
            id: local_id(ACCOUNT, EntityKind::Message, "orig-1"),
            account_id: ACCOUNT.into(),
            folder_id: "inbox".into(),
            remote_id: "orig-1".into(),
            conversation_id: None,
            subject: "Atlas".into(),
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
        internet_message_id: None,
        in_reply_to: None,
        references: vec![],
        protection: None,
        label: None,
        requested: None,
    }
}

/// `METHOD path` of every request, in order.
async fn request_log(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| format!("{} {}", r.method, r.url.path()))
        .collect()
}

/// A draft `draft-1` that keeps what it's given, read back with `values`.
async fn draft_keeping(server: &MockServer, values: &[(&str, &str)]) {
    Mock::given(method("PATCH"))
        .and(path("/me/messages/draft-1"))
        .respond_with(ok_json(json!({ "id": "draft-1" })))
        .mount(server)
        .await;
    mount_properties(server, values).await;
    Mock::given(method("DELETE"))
        .and(path("/me/messages/draft-1"))
        .respond_with(ResponseTemplate::new(204))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/me/messages/draft-1/send"))
        .respond_with(ResponseTemplate::new(202))
        .mount(server)
        .await;
}

#[tokio::test]
async fn new_messages_asking_for_protection_go_through_a_checked_draft() {
    let (server, provider) = setup().await;
    Mock::given(method("POST"))
        .and(path("/me/messages"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": "draft-1" })))
        .expect(1)
        .mount(&server)
        .await;
    draft_keeping(
        &server,
        &[("x-cumulas-protect", "encrypt"), ("x-cumulas-label", LABEL)],
    )
    .await;
    provider
        .send_requesting(
            &outgoing(),
            None,
            &request(Some(ProtectAction::Encrypt), Some(LABEL), false),
        )
        .await
        .unwrap();
    assert_eq!(
        request_log(&server).await,
        [
            "POST /me/messages",
            "PATCH /me/messages/draft-1",
            "POST /$batch",
            "POST /me/messages/draft-1/send",
        ]
    );
    let requests = server.received_requests().await.unwrap();
    // The new message's JSON never carries the properties or headers...
    let created: Value = requests[0].body_json().unwrap();
    assert_eq!(created["subject"], "Atlas pricing");
    assert!(created.get("singleValueExtendedProperties").is_none());
    assert!(created.get("internetMessageHeaders").is_none());
    // ...one PATCH sets them, by their exact names.
    let patch: Value = requests[1].body_json().unwrap();
    assert_eq!(
        patch,
        json!({ "singleValueExtendedProperties": [
            { "id": PROTECT_ID, "value": "encrypt" },
            { "id": LABEL_ID, "value": LABEL },
        ] })
    );
    // Read back, in order, before sending: no msip_labels for a new message.
    let read: Vec<Option<String>> = batched_requests(&server)
        .await
        .iter()
        .map(property_of)
        .collect();
    assert_eq!(
        read,
        [
            Some("x-cumulas-protect".into()),
            Some("x-cumulas-label".into())
        ]
    );
    assert_eq!(
        batched_requests(&server).await[1]["dependsOn"],
        json!(["protect"])
    );
}

#[tokio::test]
async fn replies_asking_for_a_label_are_drafted_patched_checked_and_sent() {
    let (server, provider) = setup().await;
    Mock::given(method("POST"))
        .and(path("/me/messages/orig-1/createReply"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": "draft-1" })))
        .expect(1)
        .mount(&server)
        .await;
    // The original's label, copied: the same one.
    let copied = format!("MSIP_Label_{LABEL}_Enabled=true; MSIP_Label_{LABEL}_SiteId={TENANT}");
    draft_keeping(
        &server,
        &[("x-cumulas-label", LABEL), ("msip_labels", &copied)],
    )
    .await;
    let message = OutgoingMessage {
        protect: None,
        reply_kind: Some(ReplyKind::Reply),
        reply_to_message_id: Some(original().summary.id),
        attachments: vec![crate::model::OutgoingAttachment {
            name: "notes.txt".into(),
            content_type: "text/plain".into(),
            data_base64: STANDARD.encode("notes"),
        }],
        ..outgoing()
    };
    Mock::given(method("POST"))
        .and(path("/me/messages/draft-1/attachments"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": "att" })))
        .expect(1)
        .mount(&server)
        .await;
    provider
        .send_requesting(
            &message,
            Some(&original()),
            &request(None, Some(LABEL), false),
        )
        .await
        .unwrap();
    assert_eq!(
        request_log(&server).await,
        [
            "POST /me/messages/orig-1/createReply",
            "PATCH /me/messages/draft-1",
            "PATCH /me/messages/draft-1",
            "POST /$batch",
            "POST /me/messages/draft-1/attachments",
            "POST /me/messages/draft-1/send",
        ]
    );
    let requests = server.received_requests().await.unwrap();
    let content: Value = requests[1].body_json().unwrap();
    assert_eq!(content["subject"], "Atlas pricing");
    let properties: Value = requests[2].body_json().unwrap();
    assert_eq!(
        properties,
        json!({ "singleValueExtendedProperties": [{ "id": LABEL_ID, "value": LABEL }] })
    );
    let read: Vec<Option<String>> = batched_requests(&server)
        .await
        .iter()
        .map(property_of)
        .collect();
    assert_eq!(
        read,
        [Some("x-cumulas-label".into()), Some("msip_labels".into())]
    );
}

/// Sends a reply asking for `request` from a draft read back with
/// `values`; returns the result and whether the draft was sent.
async fn reply_read_back(
    values: &[(&str, &str)],
    request: HeaderRequest,
) -> (crate::Result<()>, bool) {
    let (server, provider) = setup().await;
    Mock::given(method("POST"))
        .and(path("/me/messages/orig-1/createForward"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": "draft-1" })))
        .mount(&server)
        .await;
    draft_keeping(&server, values).await;
    let message = OutgoingMessage {
        protect: request.protect,
        label_id: request.label_id.clone(),
        reply_kind: Some(ReplyKind::Forward),
        reply_to_message_id: Some(original().summary.id),
        ..outgoing()
    };
    let result = provider
        .send_requesting(&message, Some(&original()), &request)
        .await;
    let log = request_log(&server).await;
    let sent = log.iter().any(|l| l.ends_with("/send"));
    let deleted = log.iter().any(|l| l == "DELETE /me/messages/draft-1");
    assert_eq!(sent, !deleted, "{log:?}");
    (result, sent)
}

#[tokio::test]
async fn nothing_is_sent_unless_the_draft_kept_what_it_asks_for() {
    let both = request(Some(ProtectAction::DoNotForward), Some(LABEL), false);
    for values in [
        vec![("x-cumulas-label", LABEL)],
        vec![("x-cumulas-protect", "encrypt"), ("x-cumulas-label", LABEL)],
        vec![
            ("x-cumulas-protect", "do-not-forward"),
            ("x-cumulas-label", OTHER_LABEL),
        ],
        vec![("x-cumulas-protect", "do-not-forward")],
    ] {
        let (result, sent) = reply_read_back(&values, both.clone()).await;
        assert!(!sent, "{values:?}");
        let error = result.unwrap_err();
        assert_eq!(error.code, ErrorCode::Provider);
        assert!(has_reason(&error, "headersNotKept"));
        assert_eq!(
            error.message,
            "Cumulas Mail couldn't add the label or encryption to this forward, so it wasn't \
             sent. Send it from Outlook on the web instead."
        );
    }
    let (result, sent) = reply_read_back(
        &[
            ("x-cumulas-protect", "do-not-forward"),
            ("x-cumulas-label", LABEL),
        ],
        both,
    )
    .await;
    result.unwrap();
    assert!(sent);
}

#[tokio::test]
async fn a_label_copied_from_the_original_must_be_the_one_asked_for() {
    let msip = |id: &str, tenant: &str| {
        format!("MSIP_Label_{id}_Enabled=true; MSIP_Label_{id}_SiteId={tenant}")
    };
    let asked = |encrypts| request(None, Some(LABEL), encrypts);
    // Another of the organization's labels: refused, naming it.
    let copied = msip(OTHER_LABEL, TENANT);
    let (result, sent) = reply_read_back(
        &[("x-cumulas-label", LABEL), ("msip_labels", &copied)],
        asked(false),
    )
    .await;
    assert!(!sent);
    let error = result.unwrap_err();
    assert!(has_reason(&error, "labelCopied"));
    assert_eq!(error.details.unwrap()["labelId"], OTHER_LABEL);
    // The same label: sent, unless it encrypts.
    let same = msip(LABEL, &TENANT.to_uppercase());
    let (result, sent) = reply_read_back(
        &[("x-cumulas-label", LABEL), ("msip_labels", &same)],
        asked(false),
    )
    .await;
    result.unwrap();
    assert!(sent);
    let (result, sent) = reply_read_back(
        &[("x-cumulas-label", LABEL), ("msip_labels", &same)],
        asked(true),
    )
    .await;
    assert!(!sent);
    assert_eq!(result.unwrap_err().details.unwrap()["labelId"], LABEL);
    // Another organization's label isn't this one's to keep.
    let foreign = msip(OTHER_LABEL, "11111111-2222-3333-4444-555555555555");
    let (result, sent) = reply_read_back(
        &[("x-cumulas-label", LABEL), ("msip_labels", &foreign)],
        asked(true),
    )
    .await;
    result.unwrap();
    assert!(sent);
}

#[tokio::test]
async fn a_draft_that_refuses_the_properties_is_discarded() {
    let (server, provider) = setup().await;
    Mock::given(method("POST"))
        .and(path("/me/messages"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": "draft-1" })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/me/messages/draft-1"))
        .respond_with(graph_error(
            400,
            "ErrorInvalidProperty",
            "Invalid property.",
        ))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/me/messages/draft-1"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let error = provider
        .send_requesting(
            &outgoing(),
            None,
            &request(Some(ProtectAction::Encrypt), None, false),
        )
        .await
        .unwrap_err();
    assert!(has_reason(&error, "headersNotKept"));
    assert!(error.message.contains("this message,"));
    assert!(
        !request_log(&server)
            .await
            .iter()
            .any(|l| l.ends_with("/send"))
    );
}

/// A new message's draft `draft-1` whose PATCH is answered by `patch` and
/// whose read-back batches by `read_back`; returns the result and the
/// requests' log.
async fn checked_draft(
    patch: ResponseTemplate,
    read_back: impl Fn(&wiremock::Request) -> ResponseTemplate + Send + Sync + 'static,
) -> (crate::Result<()>, Vec<String>) {
    let (server, provider) = setup().await;
    Mock::given(method("POST"))
        .and(path("/me/messages"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": "draft-1" })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/me/messages/draft-1"))
        .respond_with(patch)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/$batch"))
        .respond_with(read_back)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/me/messages/draft-1"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/me/messages/draft-1/send"))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;
    let result = provider
        .send_requesting(
            &outgoing(),
            None,
            &request(Some(ProtectAction::Encrypt), Some(LABEL), false),
        )
        .await;
    (result, request_log(&server).await)
}

#[tokio::test]
async fn a_draft_microsoft_365_didnt_answer_for_isnt_said_to_have_dropped_it() {
    let kept = batch_responder(properties(&[
        ("x-cumulas-protect", "encrypt"),
        ("x-cumulas-label", LABEL),
    ]));
    // The PATCH keeps failing on the server's side.
    let unavailable = graph_error(503, "ServiceUnavailable", "Service unavailable")
        .insert_header("Retry-After", "0");
    let (result, log) = checked_draft(unavailable, kept).await;
    let error = result.unwrap_err();
    assert!(!has_reason(&error, "headersNotKept"), "{error:?}");
    assert_eq!(error.message, "Service unavailable");
    assert!(log.contains(&"DELETE /me/messages/draft-1".to_owned()));
    assert!(!log.iter().any(|l| l.ends_with("/send")));

    // The read-back fails on the server's side, and what runs after it
    // fails with it (424).
    let failing = batch_responder(|r| match r["id"].as_str().unwrap() {
        "protect" => (
            500,
            json!({ "error": { "code": "InternalServerError", "message": "oops" } }),
        ),
        _ => (
            424,
            json!({ "error": { "code": "FailedDependency", "message": "failed" } }),
        ),
    });
    let (result, log) = checked_draft(ok_json(json!({ "id": "draft-1" })), failing).await;
    let error = result.unwrap_err();
    assert_eq!(error.code, ErrorCode::Provider);
    assert!(!has_reason(&error, "headersNotKept"), "{error:?}");
    assert!(error.message.starts_with("Microsoft 365 didn't answer"));
    assert!(log.contains(&"DELETE /me/messages/draft-1".to_owned()));
    assert!(!log.iter().any(|l| l.ends_with("/send")));
}

#[tokio::test]
async fn a_throttled_read_back_is_retried_with_what_runs_after_it() {
    // The first round throttles the first read and fails the second with
    // 424; both go again, and the draft is sent.
    let answered = std::sync::atomic::AtomicBool::new(false);
    let kept = properties(&[("x-cumulas-protect", "encrypt"), ("x-cumulas-label", LABEL)]);
    let read_back = move |request: &wiremock::Request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let first = !answered.swap(true, std::sync::atomic::Ordering::SeqCst);
        let responses: Vec<Value> = body["requests"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                let (status, body) = match r["id"].as_str().unwrap() {
                    "protect" if first => (429, json!({ "error": { "code": "TooManyRequests" } })),
                    _ if first => (424, json!({ "error": { "code": "FailedDependency" } })),
                    _ => kept(r),
                };
                json!({
                    "id": r["id"],
                    "status": status,
                    "headers": { "Content-Type": "application/json", "Retry-After": "0" },
                    "body": body,
                })
            })
            .collect();
        ok_json(json!({ "responses": responses }))
    };
    let (result, log) = checked_draft(ok_json(json!({ "id": "draft-1" })), read_back).await;
    result.unwrap();
    assert_eq!(
        log,
        [
            "POST /me/messages",
            "PATCH /me/messages/draft-1",
            "POST /$batch",
            "POST /$batch",
            "POST /me/messages/draft-1/send",
        ]
    );
}

#[tokio::test]
async fn plain_sending_never_drops_a_request() {
    let (server, provider) = setup().await;
    let error = provider.send(&outgoing(), None).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::Internal);
    assert!(server.received_requests().await.unwrap().is_empty());
}

// --- Outlook on the web and the setup check --------------------------------------------

#[tokio::test]
async fn web_links_are_https_only() {
    let (server, provider) = setup().await;
    for (id, link) in [
        (
            "msg-1",
            json!("https://outlook.office365.com/owa/?ItemID=msg-1&exvsurl=1"),
        ),
        ("msg-2", json!("javascript:alert(1)")),
        ("msg-3", Value::Null),
    ] {
        Mock::given(method("GET"))
            .and(path(format!("/me/messages/{id}")))
            .and(query_param("$select", "webLink"))
            .and(prefers(PREFER_IMMUTABLE_ID))
            .respond_with(ok_json(json!({ "id": id, "webLink": link })))
            .mount(&server)
            .await;
    }
    assert_eq!(
        provider
            .web_link(&locator("msg-1"))
            .await
            .unwrap()
            .as_deref(),
        Some("https://outlook.office365.com/owa/?ItemID=msg-1&exvsurl=1")
    );
    assert_eq!(provider.web_link(&locator("msg-2")).await.unwrap(), None);
    assert_eq!(provider.web_link(&locator("msg-3")).await.unwrap(), None);
}

#[tokio::test]
async fn the_inbox_is_searched_for_each_tokens_newest_message() {
    let (server, provider) = setup().await;
    let since = Utc.with_ymd_and_hms(2026, 10, 18, 9, 0, 0).unwrap();
    Mock::given(method("GET"))
        .and(path("/me/mailFolders/inbox/messages"))
        .and(query_param(
            "$filter",
            "receivedDateTime ge 2026-10-18T09:00:00Z",
        ))
        .and(query_param("$orderby", "receivedDateTime desc"))
        .and(query_param("$select", "id,subject,receivedDateTime"))
        .and(query_param("$top", "50"))
        .and(prefers(PREFER_IMMUTABLE_ID))
        .respond_with(ok_json(json!({
            "value": [
                { "id": "m-3", "subject": "Cumulas Mail setup check [aaaaaaaa]" },
                { "id": "m-2", "subject": "RE: Cumulas Mail setup check [AAAAAAAA]" },
                { "id": "m-1", "subject": "Lunch?" },
            ],
            "@odata.nextLink": format!("{}/me/mailFolders/inbox/messages?$skip=3", server.uri()),
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/me/mailFolders/inbox/messages"))
        .and(query_param("$skip", "3"))
        .respond_with(ok_json(json!({
            "value": [{ "id": "m-0", "subject": "Cumulas Mail setup check [bbbbbbbb]" }],
        })))
        .mount(&server)
        .await;
    let tokens = [
        "aaaaaaaa".to_owned(),
        "bbbbbbbb".to_owned(),
        "cccccccc".to_owned(),
    ];
    let found = provider.find_in_inbox(&tokens, since).await.unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!(found["aaaaaaaa"].remote_id, "m-3");
    assert_eq!(found["bbbbbbbb"].remote_id, "m-0");
    assert!(!found.contains_key("cccccccc"));
}

#[tokio::test]
async fn the_own_address_is_asked_for_once() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me"))
        .and(query_param("$select", "mail,userPrincipalName"))
        .respond_with(ok_json(
            json!({ "mail": null, "userPrincipalName": "adele@contoso.com" }),
        ))
        .expect(1)
        .mount(&server)
        .await;
    for _ in 0..2 {
        assert_eq!(
            provider.own_address().await.unwrap(),
            Some(EmailAddress::new(None, "adele@contoso.com"))
        );
    }
}

#[tokio::test]
async fn the_draft_is_created_as_usual_for_requests() {
    // `body_json` pins the new message's JSON: the same as `send`'s draft.
    let (server, provider) = setup().await;
    Mock::given(method("POST"))
        .and(path("/me/messages"))
        .and(body_json(json!({
            "subject": "Atlas pricing",
            "body": { "contentType": "HTML", "content": "<p>Attached.</p>" },
            "toRecipients": [{ "emailAddress": { "address": "pat@fabrikam.com" } }],
            "ccRecipients": [],
            "bccRecipients": [],
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": "draft-1" })))
        .expect(1)
        .mount(&server)
        .await;
    draft_keeping(&server, &[("x-cumulas-protect", "encrypt")]).await;
    let message = OutgoingMessage {
        label_id: None,
        ..outgoing()
    };
    provider
        .send_requesting(
            &message,
            None,
            &request(Some(ProtectAction::Encrypt), None, false),
        )
        .await
        .unwrap();
}
