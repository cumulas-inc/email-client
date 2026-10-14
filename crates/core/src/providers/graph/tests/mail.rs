//! Folders, delta sync, message bodies, flags, moves and sending.

use std::collections::HashMap;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use wiremock::matchers::{body_json, header, method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::{
    ACCOUNT, batch_responder, batched_requests, fixture, graph_error, ok_json, prefers, setup,
};
use crate::model::{
    EmailAddress, EntityKind, Folder, FolderRole, Importance, Message, MessageSummary,
    OutgoingAttachment, OutgoingMessage, ReplyKind, local_id,
};
use crate::providers::graph::client::{PREFER_HTML_BODY, PREFER_IMMUTABLE_ID, PREFER_PAGE_SIZE};
use crate::providers::{MailProvider, MessageLocator, SyncWindow};

const MSG_QUARTERLY: &str = "AAkALgAAAAAAHYQDEapmEc2byACqAC-EWg0AqP8jr2fTrESaK7Kv9g_EgwAAAWlAIgAA";
const MSG_OLD: &str = "AAkALgAAAAAAHYQDEapmEc2byACqAC-EWg0AqP8jr2fTrESaK7Kv9g_EgwAAAWlAIwAA";
const MSG_LUNCH: &str = "AAkALgAAAAAAHYQDEapmEc2byACqAC-EWg0AqP8jr2fTrESaK7Kv9g_EgwAAAWlAJAAA";
const MSG_NEW: &str = "AAkALgAAAAAAHYQDEapmEc2byACqAC-EWg0AqP8jr2fTrESaK7Kv9g_EgwAAAWlAJQAA";

fn inbox() -> Folder {
    Folder {
        id: local_id(ACCOUNT, EntityKind::Folder, "AAMkInbox"),
        account_id: ACCOUNT.into(),
        remote_id: "AAMkInbox".into(),
        parent_id: None,
        name: "Inbox".into(),
        role: FolderRole::Inbox,
        unread_count: 0,
        total_count: 0,
    }
}

fn window(max_messages: u32) -> SyncWindow {
    SyncWindow {
        since: Utc.with_ymd_and_hms(2026, 7, 16, 0, 0, 0).unwrap(),
        max_messages,
    }
}

fn locator(id: &str) -> MessageLocator {
    MessageLocator {
        folder_remote_id: "AAMkInbox".into(),
        remote_id: id.into(),
    }
}

fn well_known_answer(request: &Value) -> (u16, Value) {
    let url = request["url"].as_str().unwrap();
    let name = url
        .trim_start_matches("/me/mailFolders/")
        .split('?')
        .next()
        .unwrap();
    let id = match name {
        "inbox" => "AAMkInbox",
        "drafts" => "AAMkDrafts",
        "sentitems" => "AAMkSent",
        "junkemail" => "AAMkJunk",
        "deleteditems" => "AAMkDeleted",
        "outbox" => "AAMkOutbox",
        _ => {
            return (
                404,
                json!({ "error": { "code": "ErrorFolderNotFound", "message": "The specified folder could not be found in the store." } }),
            );
        }
    };
    (200, json!({ "id": id }))
}

#[tokio::test]
async fn folders_are_listed_recursively_with_roles() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me/mailFolders"))
        .and(query_param_is_missing("$skip"))
        .and(query_param("$top", "100"))
        .and(prefers(PREFER_IMMUTABLE_ID))
        .respond_with(ok_json(fixture("mail_folders_page1.json", &server)))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/me/mailFolders"))
        .and(query_param("$skip", "3"))
        .respond_with(ok_json(fixture("mail_folders_page2.json", &server)))
        .mount(&server)
        .await;
    Mock::given(path("/me/mailFolders/AAMkInbox/childFolders"))
        .respond_with(ok_json(json!({ "value": [{
            "id": "AAMkReceipts", "displayName": "Receipts", "parentFolderId": "AAMkInbox",
            "childFolderCount": 1, "unreadItemCount": 1, "totalItemCount": 9
        }] })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/me/mailFolders/AAMkReceipts/childFolders"))
        .respond_with(ok_json(json!({ "value": [{
            "id": "AAMkTravel", "displayName": "Travel", "parentFolderId": "AAMkReceipts",
            "childFolderCount": 0, "unreadItemCount": 0, "totalItemCount": 4
        }] })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/$batch"))
        .respond_with(batch_responder(well_known_answer))
        .expect(1)
        .mount(&server)
        .await;

    let folders = provider.list_folders().await.unwrap();
    let by_remote: HashMap<&str, &Folder> =
        folders.iter().map(|f| (f.remote_id.as_str(), f)).collect();
    assert_eq!(folders.len(), 9);

    let inbox = by_remote["AAMkInbox"];
    assert_eq!(inbox.id, local_id(ACCOUNT, EntityKind::Folder, "AAMkInbox"));
    assert_eq!(inbox.account_id, ACCOUNT);
    assert_eq!(inbox.name, "Inbox");
    assert_eq!(inbox.role, FolderRole::Inbox);
    assert_eq!((inbox.unread_count, inbox.total_count), (3, 120));
    assert_eq!(inbox.parent_id, None);

    let receipts = by_remote["AAMkReceipts"];
    assert_eq!(receipts.parent_id.as_deref(), Some(inbox.id.as_str()));
    assert_eq!(receipts.role, FolderRole::Other);
    let travel = by_remote["AAMkTravel"];
    assert_eq!(travel.parent_id.as_deref(), Some(receipts.id.as_str()));

    let roles: HashMap<&str, FolderRole> = folders
        .iter()
        .map(|f| (f.remote_id.as_str(), f.role))
        .collect();
    assert_eq!(roles["AAMkDrafts"], FolderRole::Drafts);
    assert_eq!(roles["AAMkSent"], FolderRole::Sent);
    assert_eq!(roles["AAMkDeleted"], FolderRole::Trash);
    assert_eq!(roles["AAMkJunk"], FolderRole::Junk);
    assert_eq!(roles["AAMkOutbox"], FolderRole::Outbox);
    assert_eq!(roles["AAMkProjects"], FolderRole::Other);
    assert!(folders.iter().all(|f| f.role != FolderRole::Archive));

    // The well-known names were resolved in one batch with immutable ids.
    let batch = batched_requests(&server).await;
    assert_eq!(batch.len(), 7);
    assert_eq!(batch[0]["method"], "GET");
    assert_eq!(batch[0]["url"], "/me/mailFolders/inbox?$select=id");
    assert_eq!(batch[0]["headers"]["Prefer"], PREFER_IMMUTABLE_ID);
}

fn summary<'a>(batch: &'a [MessageSummary], remote_id: &str) -> &'a MessageSummary {
    batch.iter().find(|m| m.remote_id == remote_id).unwrap()
}

#[tokio::test]
async fn initial_sync_reads_all_pages_and_keeps_the_newest() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me/mailFolders/AAMkInbox/messages/delta"))
        .and(query_param_is_missing("$skiptoken"))
        .and(query_param(
            "$filter",
            "receivedDateTime ge 2026-07-16T00:00:00Z",
        ))
        .and(|r: &Request| {
            r.url
                .query_pairs()
                .any(|(k, v)| k == "$select" && v.contains("bodyPreview") && v.contains("flag"))
        })
        .and(prefers(PREFER_IMMUTABLE_ID))
        .and(prefers(PREFER_PAGE_SIZE))
        .respond_with(ok_json(fixture(
            "messages_delta_initial_page1.json",
            &server,
        )))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/me/mailFolders/AAMkInbox/messages/delta"))
        .and(query_param("$skiptoken", "page-2"))
        .and(prefers(PREFER_IMMUTABLE_ID))
        .respond_with(ok_json(fixture(
            "messages_delta_initial_page2.json",
            &server,
        )))
        .expect(1)
        .mount(&server)
        .await;

    let batch = provider
        .sync_messages(&inbox(), None, window(2))
        .await
        .unwrap();
    assert!(batch.full_resync);
    assert!(batch.removed_remote_ids.is_empty());
    assert_eq!(
        batch.sync_state.as_deref(),
        Some(
            format!(
                "{}/me/mailFolders('AAMkInbox')/messages/delta?%24deltatoken=token-1",
                server.uri()
            )
            .as_str()
        )
    );
    // Capped at the two newest; the September message is dropped.
    let ids: Vec<&str> = batch.upserts.iter().map(|m| m.remote_id.as_str()).collect();
    assert_eq!(ids, vec![MSG_LUNCH, MSG_QUARTERLY]);

    let quarterly = summary(&batch.upserts, MSG_QUARTERLY);
    assert_eq!(
        quarterly.id,
        local_id(ACCOUNT, EntityKind::Message, MSG_QUARTERLY)
    );
    assert_eq!(quarterly.folder_id, inbox().id);
    assert_eq!(quarterly.account_id, ACCOUNT);
    assert_eq!(quarterly.subject, "Quarterly numbers");
    assert_eq!(
        quarterly.from,
        Some(EmailAddress::new(Some("Megan Bowen"), "MeganB@contoso.com"))
    );
    assert_eq!(
        quarterly.to,
        vec![EmailAddress::new(Some("Adele Vance"), "AdeleV@contoso.com")]
    );
    // The recipient without an address is skipped.
    assert_eq!(quarterly.cc.len(), 1);
    assert_eq!(
        quarterly.preview,
        "Hi team, The quarterly numbers are in. Revenue is up 12% on last year."
    );
    assert_eq!(
        quarterly.received_at,
        Utc.with_ymd_and_hms(2026, 10, 12, 8, 15, 0).unwrap()
    );
    assert!(!quarterly.is_read);
    assert!(quarterly.is_flagged);
    assert!(!quarterly.is_draft);
    assert!(quarterly.has_attachments);
    assert_eq!(quarterly.importance, Importance::High);
    assert_eq!(
        quarterly.conversation_id.as_deref(),
        Some("AAQkAGI2TG93AAA=")
    );

    let lunch = summary(&batch.upserts, MSG_LUNCH);
    // `from` is null: the sender is used.
    assert_eq!(lunch.from.as_ref().unwrap().address, "NestorW@contoso.com");
    assert!(lunch.cc.is_empty());
    assert!(!lunch.is_flagged, "completed flags are not flagged");
    assert_eq!(lunch.importance, Importance::Low);
}

#[tokio::test]
async fn incremental_sync_reports_changes_and_removals() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(query_param("$deltatoken", "token-1"))
        .and(prefers(PREFER_IMMUTABLE_ID))
        .respond_with(ok_json(fixture("messages_delta_incremental.json", &server)))
        .expect(1)
        .mount(&server)
        .await;
    // The item that came back with only `isRead` is fetched in full.
    Mock::given(method("POST"))
        .and(path("/$batch"))
        .respond_with(batch_responder(|r| {
            assert!(r["url"].as_str().unwrap().starts_with(&format!(
                "/me/messages/{MSG_QUARTERLY}?$select="
            )));
            (
                200,
                json!({
                    "id": MSG_QUARTERLY,
                    "subject": "Quarterly numbers",
                    "receivedDateTime": "2026-10-12T08:15:00Z",
                    "isRead": true,
                    "flag": { "flagStatus": "flagged" },
                    "from": { "emailAddress": { "name": "Megan Bowen", "address": "MeganB@contoso.com" } },
                    "toRecipients": [],
                    "bodyPreview": "Hi team"
                }),
            )
        }))
        .expect(1)
        .mount(&server)
        .await;

    let state = format!(
        "{}/me/mailFolders('AAMkInbox')/messages/delta?%24deltatoken=token-1",
        server.uri()
    );
    let batch = provider
        .sync_messages(&inbox(), Some(&state), window(1))
        .await
        .unwrap();
    assert!(!batch.full_resync);
    assert_eq!(batch.removed_remote_ids, vec![MSG_LUNCH, MSG_OLD]);
    assert_eq!(batch.upserts.len(), 2);
    assert_eq!(summary(&batch.upserts, MSG_NEW).subject, "New arrival");
    let refetched = summary(&batch.upserts, MSG_QUARTERLY);
    assert!(refetched.is_read);
    assert!(refetched.is_flagged);
    assert!(batch.sync_state.unwrap().ends_with("deltatoken=token-2"));
}

#[tokio::test]
async fn expired_delta_tokens_fall_back_to_a_full_resync() {
    for (status, code) in [(410, "SyncStateNotFound"), (400, "syncStateInvalid")] {
        let (server, provider) = setup().await;
        Mock::given(method("GET"))
            .and(query_param("$deltatoken", "stale"))
            .respond_with(graph_error(
                status,
                code,
                "The sync state is not valid or has expired.",
            ))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/me/mailFolders/AAMkInbox/messages/delta"))
            .and(query_param_is_missing("$deltatoken"))
            .respond_with(ok_json(json!({
                "value": [],
                "@odata.deltaLink": format!("{}/me/mailFolders/AAMkInbox/messages/delta?$deltatoken=fresh", server.uri()),
            })))
            .expect(1)
            .mount(&server)
            .await;
        let state = format!(
            "{}/me/mailFolders/AAMkInbox/messages/delta?$deltatoken=stale",
            server.uri()
        );
        let batch = provider
            .sync_messages(&inbox(), Some(&state), window(10))
            .await
            .unwrap();
        assert!(batch.full_resync, "{code}");
        assert!(batch.upserts.is_empty());
        assert!(batch.sync_state.unwrap().ends_with("deltatoken=fresh"));
    }
}

#[tokio::test]
async fn other_delta_errors_are_returned() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .respond_with(graph_error(
            500,
            "ErrorInternalServerError",
            "Something broke.",
        ))
        .mount(&server)
        .await;
    let state = format!(
        "{}/me/mailFolders/AAMkInbox/messages/delta?$deltatoken=t",
        server.uri()
    );
    let error = provider
        .sync_messages(&inbox(), Some(&state), window(10))
        .await
        .unwrap_err();
    assert_eq!(error.message, "Something broke.");
}

#[tokio::test]
async fn foreign_sync_state_is_never_followed() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me/mailFolders/AAMkInbox/messages/delta"))
        .and(query_param_is_missing("$deltatoken"))
        .respond_with(ok_json(json!({
            "value": [],
            "@odata.deltaLink": format!("{}/me/mailFolders/AAMkInbox/messages/delta?$deltatoken=fresh", server.uri()),
        })))
        .expect(1)
        .mount(&server)
        .await;
    let batch = provider
        .sync_messages(
            &inbox(),
            Some("https://attacker.example/delta?$deltatoken=x"),
            window(10),
        )
        .await
        .unwrap();
    assert!(batch.full_resync);
}

#[tokio::test]
async fn fetch_message_reads_body_headers_and_embeds_inline_images() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path(format!("/me/messages/{MSG_QUARTERLY}")))
        .and(query_param(
            "$select",
            "body,bccRecipients,replyTo,internetMessageId,internetMessageHeaders,hasAttachments",
        ))
        .and(prefers(PREFER_IMMUTABLE_ID))
        .and(prefers(PREFER_HTML_BODY))
        .respond_with(ok_json(fixture("message_detail.json", &server)))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/me/messages/{MSG_QUARTERLY}/attachments")))
        .and(query_param("$select", "id,name,contentType,size,isInline"))
        .respond_with(ok_json(fixture("message_attachments.json", &server)))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/$batch"))
        .respond_with(batch_responder(|r| {
            let url = r["url"].as_str().unwrap();
            let (cid, bytes) = if url.ends_with("AAMkAtt-image") {
                ("image001.png@01DA1F2B.3C4D5E60", "iVBORw0KGgo=")
            } else if url.ends_with("AAMkAtt-logo") {
                ("<logo@contoso>", "R0lGODlh")
            } else {
                panic!("unexpected attachment fetch {url}");
            };
            (
                200,
                json!({
                    "@odata.type": "#microsoft.graph.fileAttachment",
                    "id": url.rsplit('/').next().unwrap(),
                    "contentId": cid,
                    "contentBytes": bytes,
                }),
            )
        }))
        .expect(1)
        .mount(&server)
        .await;

    let content = provider
        .fetch_message(&locator(MSG_QUARTERLY))
        .await
        .unwrap();
    let html = content.body_html.unwrap();
    assert!(html.contains(r#"<img src="data:image/png;base64,iVBORw0KGgo=" alt="chart">"#));
    // Typed application/octet-stream but named .gif: embedded as a GIF.
    assert!(html.contains(r#"<img src="data:image/gif;base64,R0lGODlh">"#));
    assert!(!html.contains("cid:"));
    assert_eq!(content.body_text, None);
    assert_eq!(
        content.bcc,
        vec![EmailAddress::new(Some("Audit"), "audit@contoso.com")]
    );
    assert_eq!(
        content.reply_to,
        vec![EmailAddress::new(Some("Sales"), "sales@contoso.com")]
    );
    assert_eq!(
        content.internet_message_id.as_deref(),
        Some("<CAF4b2Xx=QzE@mail.contoso.com>")
    );
    assert_eq!(
        content.in_reply_to.as_deref(),
        Some("<parent-1@mail.contoso.com>")
    );
    assert_eq!(
        content.references,
        vec!["<root-1@mail.contoso.com>", "<parent-1@mail.contoso.com>"]
    );

    let attachments: HashMap<&str, _> = content
        .attachments
        .iter()
        .map(|a| (a.id.as_str(), a))
        .collect();
    assert_eq!(attachments.len(), 5);
    let image = attachments["AAMkAtt-image"];
    assert!(image.is_inline);
    assert_eq!(
        image.content_id.as_deref(),
        Some("image001.png@01DA1F2B.3C4D5E60")
    );
    let logo = attachments["AAMkAtt-logo"];
    assert!(logo.is_inline, "embedded images count as inline");
    assert_eq!(logo.content_id.as_deref(), Some("logo@contoso"));
    let report = attachments["AAMkAtt-report"];
    assert_eq!(report.name, "report.pdf");
    assert_eq!(report.content_type, "application/pdf");
    assert_eq!(report.size, 52000);
    assert!(!report.is_inline);
    // Too large to embed: never fetched.
    assert_eq!(attachments["AAMkAtt-huge"].content_id, None);
    assert_eq!(attachments["AAMkAtt-item"].content_type, "message/rfc822");

    // Only the two small images were fetched (inline one first).
    let fetched: Vec<String> = batched_requests(&server)
        .await
        .iter()
        .map(|r| r["url"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        fetched,
        vec![
            format!("/me/messages/{MSG_QUARTERLY}/attachments/AAMkAtt-image"),
            format!("/me/messages/{MSG_QUARTERLY}/attachments/AAMkAtt-logo"),
        ]
    );
}

#[tokio::test]
async fn plain_messages_without_attachments_need_one_request() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me/messages/msg-1"))
        .respond_with(ok_json(json!({
            "id": "msg-1",
            "hasAttachments": false,
            "body": { "contentType": "text", "content": "Hello" },
            "bccRecipients": [],
            "replyTo": [],
            "internetMessageId": "<a@b>"
        })))
        .expect(1)
        .mount(&server)
        .await;
    let content = provider.fetch_message(&locator("msg-1")).await.unwrap();
    assert_eq!(content.body_text.as_deref(), Some("Hello"));
    assert_eq!(content.body_html, None);
    assert!(content.attachments.is_empty());
    assert!(content.references.is_empty());
    assert_eq!(content.in_reply_to, None);
}

#[tokio::test]
async fn fetch_attachment_downloads_the_raw_value() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me/messages/msg-1/attachments/att-1/$value"))
        .and(prefers(PREFER_IMMUTABLE_ID))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"%PDF-1.7".to_vec()))
        .expect(1)
        .mount(&server)
        .await;
    let bytes = provider
        .fetch_attachment(&locator("msg-1"), "att-1")
        .await
        .unwrap();
    assert_eq!(bytes, b"%PDF-1.7");
}

#[tokio::test]
async fn read_and_flag_changes_are_batched_patches() {
    let (server, provider) = setup().await;
    Mock::given(method("POST"))
        .and(path("/$batch"))
        .respond_with(batch_responder(|r| {
            if r["url"] == "/me/messages/gone" {
                (
                    404,
                    json!({ "error": { "code": "ErrorItemNotFound", "message": "Not found." } }),
                )
            } else {
                (200, json!({ "id": "x" }))
            }
        }))
        .mount(&server)
        .await;
    provider
        .set_read(&[locator("msg-1"), locator("gone")], true)
        .await
        .unwrap();
    provider
        .set_flagged(&[locator("msg-2")], false)
        .await
        .unwrap();
    let sent = batched_requests(&server).await;
    assert_eq!(
        sent[0],
        json!({
            "id": "0",
            "method": "PATCH",
            "url": "/me/messages/msg-1",
            "body": { "isRead": true },
            "headers": { "Prefer": PREFER_IMMUTABLE_ID, "Content-Type": "application/json" },
        })
    );
    assert_eq!(sent[1]["url"], "/me/messages/gone");
    assert_eq!(
        sent[2]["body"],
        json!({ "flag": { "flagStatus": "notFlagged" } })
    );
}

#[tokio::test]
async fn moves_return_new_ids_and_deletes_are_permanent() {
    let (server, provider) = setup().await;
    Mock::given(method("POST"))
        .and(path("/$batch"))
        .respond_with(batch_responder(|r| match r["url"].as_str().unwrap() {
            "/me/messages/msg-1/move" => (201, json!({ "id": "msg-1" })),
            "/me/messages/msg-2/move" => (201, json!({ "id": "msg-2-new" })),
            "/me/messages/msg-3/permanentDelete" => (204, Value::Null),
            "/me/messages/gone/permanentDelete" => (
                404,
                json!({ "error": { "code": "ErrorItemNotFound", "message": "Not found." } }),
            ),
            other => panic!("unexpected {other}"),
        }))
        .mount(&server)
        .await;
    let ids = provider
        .move_messages(&[locator("msg-1"), locator("msg-2")], "AAMkArchive")
        .await
        .unwrap();
    assert_eq!(ids, vec![Some("msg-1".into()), Some("msg-2-new".into())]);
    provider
        .delete_messages(&[locator("msg-3"), locator("gone")])
        .await
        .unwrap();
    let sent = batched_requests(&server).await;
    assert_eq!(sent[0]["method"], "POST");
    assert_eq!(sent[0]["body"], json!({ "destinationId": "AAMkArchive" }));
    assert_eq!(sent[2]["method"], "POST");
    assert!(sent[2].get("body").is_none());
}

#[tokio::test]
async fn moving_a_missing_message_fails() {
    let (server, provider) = setup().await;
    Mock::given(path("/$batch"))
        .respond_with(batch_responder(|_| {
            (
                404,
                json!({ "error": { "code": "ErrorItemNotFound", "message": "The specified object was not found in the store." } }),
            )
        }))
        .mount(&server)
        .await;
    let error = provider
        .move_messages(&[locator("gone")], "AAMkArchive")
        .await
        .unwrap_err();
    assert_eq!(error.code, crate::error::ErrorCode::NotFound);
}

fn outgoing() -> OutgoingMessage {
    OutgoingMessage {
        account_id: ACCOUNT.into(),
        to: vec![EmailAddress::new(Some("Pat Kim"), "pat@fabrikam.com")],
        cc: vec![EmailAddress::new(None, "lee@contoso.com")],
        bcc: vec![],
        subject: "Hello".into(),
        body_html: "<p>Hi Pat</p>".into(),
        attachments: vec![OutgoingAttachment {
            name: "notes.txt".into(),
            content_type: "text/plain".into(),
            data_base64: STANDARD.encode("some notes"),
        }],
        importance: Some(Importance::High),
        reply_to_message_id: None,
        reply_kind: None,
    }
}

#[tokio::test]
async fn new_messages_go_through_send_mail() {
    let (server, provider) = setup().await;
    Mock::given(method("POST"))
        .and(path("/me/sendMail"))
        .and(body_json(json!({
            "message": {
                "subject": "Hello",
                "body": { "contentType": "HTML", "content": "<p>Hi Pat</p>" },
                "toRecipients": [{ "emailAddress": { "name": "Pat Kim", "address": "pat@fabrikam.com" } }],
                "ccRecipients": [{ "emailAddress": { "address": "lee@contoso.com" } }],
                "bccRecipients": [],
                "importance": "high",
                "attachments": [{
                    "@odata.type": "#microsoft.graph.fileAttachment",
                    "name": "notes.txt",
                    "contentType": "text/plain",
                    "contentBytes": "c29tZSBub3Rlcw==",
                }],
            },
            "saveToSentItems": true,
        })))
        .respond_with(ResponseTemplate::new(202))
        .expect(1)
        .mount(&server)
        .await;
    provider.send(&outgoing(), None).await.unwrap();
}

#[tokio::test]
async fn invalid_attachment_data_is_rejected() {
    let (_server, provider) = setup().await;
    let mut message = outgoing();
    message.attachments[0].data_base64 = "not base64!".into();
    let error = provider.send(&message, None).await.unwrap_err();
    assert_eq!(error.code, crate::error::ErrorCode::InvalidInput);
}

fn original() -> Message {
    Message {
        summary: MessageSummary {
            id: local_id(ACCOUNT, EntityKind::Message, MSG_QUARTERLY),
            account_id: ACCOUNT.into(),
            folder_id: inbox().id,
            remote_id: MSG_QUARTERLY.into(),
            conversation_id: None,
            subject: "Quarterly numbers".into(),
            from: Some(EmailAddress::new(Some("Megan Bowen"), "MeganB@contoso.com")),
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
    }
}

/// Paths of the requests received, in order, as `METHOD path`.
async fn request_log(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| format!("{} {}", r.method, r.url.path()))
        .collect()
}

#[tokio::test]
async fn replies_are_drafted_patched_and_sent() {
    let (server, provider) = setup().await;
    Mock::given(method("POST"))
        .and(path(format!("/me/messages/{MSG_QUARTERLY}/createReplyAll")))
        .and(prefers(PREFER_IMMUTABLE_ID))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(json!({ "id": "draft-1", "isDraft": true })),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/me/messages/draft-1"))
        .respond_with(ok_json(json!({ "id": "draft-1" })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/me/messages/draft-1/attachments"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": "att" })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/me/messages/draft-1/send"))
        .and(header("content-length", "0"))
        .respond_with(ResponseTemplate::new(202))
        .expect(1)
        .mount(&server)
        .await;

    let mut message = outgoing();
    message.subject = "RE: Quarterly numbers".into();
    message.reply_kind = Some(ReplyKind::ReplyAll);
    message.reply_to_message_id = Some(original().summary.id);
    provider.send(&message, Some(&original())).await.unwrap();

    assert_eq!(
        request_log(&server).await,
        vec![
            format!("POST /me/messages/{MSG_QUARTERLY}/createReplyAll"),
            "PATCH /me/messages/draft-1".to_owned(),
            "POST /me/messages/draft-1/attachments".to_owned(),
            "POST /me/messages/draft-1/send".to_owned(),
        ]
    );
    let requests = server.received_requests().await.unwrap();
    let patch: Value = requests[1].body_json().unwrap();
    assert_eq!(patch["subject"], "RE: Quarterly numbers");
    assert_eq!(patch["body"]["content"], "<p>Hi Pat</p>");
    assert_eq!(
        patch["toRecipients"][0]["emailAddress"]["address"],
        "pat@fabrikam.com"
    );
    let attachment: Value = requests[2].body_json().unwrap();
    assert_eq!(attachment["name"], "notes.txt");
    assert_eq!(attachment["contentBytes"], "c29tZSBub3Rlcw==");
}

#[tokio::test]
async fn failed_drafts_are_discarded() {
    let (server, provider) = setup().await;
    Mock::given(method("POST"))
        .and(path(format!("/me/messages/{MSG_QUARTERLY}/createForward")))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": "draft-2" })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/me/messages/draft-2"))
        .respond_with(ok_json(json!({ "id": "draft-2" })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/me/messages/draft-2/attachments"))
        .respond_with(graph_error(413, "ErrorRequestEntityTooLarge", "Too large."))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/me/messages/draft-2"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/me/messages/draft-2/send"))
        .respond_with(ResponseTemplate::new(202))
        .expect(0)
        .mount(&server)
        .await;
    let mut message = outgoing();
    message.reply_kind = Some(ReplyKind::Forward);
    let error = provider
        .send(&message, Some(&original()))
        .await
        .unwrap_err();
    assert_eq!(error.message, "Too large.");
}

#[tokio::test]
async fn large_attachments_use_an_upload_session() {
    let (server, provider) = setup().await;
    let size = 4 * 1024 * 1024 + 10;
    Mock::given(method("POST"))
        .and(path("/me/messages"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": "draft-3" })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/me/messages/draft-3/attachments/createUploadSession"))
        .and(body_json(json!({
            "AttachmentItem": {
                "attachmentType": "file",
                "name": "video.mp4",
                "size": size,
                "contentType": "video/mp4",
            }
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "uploadUrl": format!("{}/upload/session-1?authtoken=opaque", server.uri()),
            "expirationDateTime": "2026-10-14T12:00:00Z",
            "nextExpectedRanges": ["0-"],
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/upload/session-1"))
        .and(header("content-range", "bytes 0-3145727/4194314"))
        .and(|r: &Request| !r.headers.contains_key("authorization"))
        .respond_with(ok_json(json!({ "nextExpectedRanges": ["3145728-"] })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/upload/session-1"))
        .and(header("content-range", "bytes 3145728-4194313/4194314"))
        .and(header("content-type", "application/octet-stream"))
        .respond_with(ResponseTemplate::new(201))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/me/messages/draft-3/send"))
        .respond_with(ResponseTemplate::new(202))
        .expect(1)
        .mount(&server)
        .await;

    let mut message = outgoing();
    message.attachments = vec![OutgoingAttachment {
        name: "video.mp4".into(),
        content_type: "video/mp4".into(),
        data_base64: STANDARD.encode(vec![7u8; size]),
    }];
    provider.send(&message, None).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let draft: Value = requests[0].body_json().unwrap();
    assert_eq!(draft["subject"], "Hello");
    assert!(draft.get("attachments").is_none());
    let uploaded: usize = requests
        .iter()
        .filter(|r| r.method.as_str() == "PUT")
        .map(|r| r.body.len())
        .sum();
    assert_eq!(uploaded, size);
}
