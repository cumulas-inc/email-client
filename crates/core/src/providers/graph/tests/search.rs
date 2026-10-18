//! Searching the mailbox with `$search`, or `$filter` for filters alone.

use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, Request};

use super::{ACCOUNT, batch_responder, batched_requests, graph_error, ok_json, prefers, setup};
use crate::error::{ErrorCode, Result};
use crate::model::{EntityKind, Folder, FolderRole, local_id};
use crate::providers::graph::GraphProvider;
use crate::providers::graph::client::PREFER_IMMUTABLE_ID;
use crate::providers::{MailProvider, SearchPage, ServerQuery};
use crate::search::SearchQuery;
use crate::search::dates::Received;

fn folder(remote: &str, role: FolderRole) -> Folder {
    Folder {
        id: local_id(ACCOUNT, EntityKind::Folder, remote),
        account_id: ACCOUNT.into(),
        remote_id: remote.into(),
        parent_id: None,
        name: remote.into(),
        role,
        unread_count: 0,
        total_count: 0,
    }
}

fn folders() -> Vec<Folder> {
    vec![
        folder("AAMkInbox", FolderRole::Inbox),
        folder("AAMkArchive", FolderRole::Archive),
    ]
}

fn found(id: &str, received: &str, parent: Option<&str>) -> Value {
    let mut message = json!({
        "id": id,
        "subject": format!("Found {id}"),
        "from": { "emailAddress": { "name": "Megan Bowen", "address": "MeganB@contoso.com" } },
        "toRecipients": [],
        "bodyPreview": "Hello",
        "receivedDateTime": received,
        "isRead": false,
    });
    if let Some(parent) = parent {
        message["parentFolderId"] = parent.into();
    }
    message
}

/// Searches the whole mailbox for `text`, at any time.
async fn search(
    provider: &GraphProvider,
    text: &str,
    cursor: Option<&str>,
    limit: u32,
) -> Result<SearchPage> {
    let query = SearchQuery::parse(text);
    let folders = folders();
    provider
        .search(&ServerQuery::whole_mailbox(&query, &folders), cursor, limit)
        .await
}

/// The value of query parameter `name`, decoded.
fn param(request: &Request, name: &str) -> Option<String> {
    request
        .url
        .query_pairs()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
}

#[tokio::test]
async fn search_sends_kql_with_select_and_top() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me/messages"))
        .and(|r: &Request| {
            param(r, "$search").as_deref()
                == Some(
                    r#""from:megan* AND subject:\"q3 plan\" AND invoice* AND hasAttachments:true""#,
                )
        })
        .and(|r: &Request| {
            param(r, "$select")
                .is_some_and(|s| s.contains("receivedDateTime") && s.ends_with(",parentFolderId"))
        })
        .and(query_param("$top", "50"))
        .and(|r: &Request| param(r, "$orderby").is_none() && param(r, "$filter").is_none())
        .and(prefers(PREFER_IMMUTABLE_ID))
        .respond_with(ok_json(json!({
            "value": [
                found("old", "2023-03-01T09:00:00Z", Some("AAMkArchive")),
                found("new", "2024-05-02T09:00:00Z", Some("AAMkInbox")),
                // Without a folder or a date: dropped.
                found("nowhere", "2024-05-03T09:00:00Z", None),
                { "id": "undated", "parentFolderId": "AAMkInbox" },
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let page = search(
        &provider,
        "from:megan subject:\"q3 plan\" invoice has:attachment",
        None,
        50,
    )
    .await
    .unwrap();
    assert_eq!(page.next, None);
    let found: Vec<(&str, &str)> = page
        .messages
        .iter()
        .map(|m| (m.remote_id.as_str(), m.folder_id.as_str()))
        .collect();
    let inbox = local_id(ACCOUNT, EntityKind::Folder, "AAMkInbox");
    let archive = local_id(ACCOUNT, EntityKind::Folder, "AAMkArchive");
    // Newest first, each in its folder.
    assert_eq!(found, [("new", inbox.as_str()), ("old", archive.as_str())]);
    let new = &page.messages[0];
    assert_eq!(new.id, local_id(ACCOUNT, EntityKind::Message, "new"));
    assert_eq!(new.account_id, ACCOUNT);
    assert_eq!(new.subject, "Found new");
    assert_eq!(
        new.received_at,
        Utc.with_ymd_and_hms(2024, 5, 2, 9, 0, 0).unwrap()
    );
    // Every stored folder was known: nothing looked up.
    assert!(batched_requests(&server).await.is_empty());
}

#[tokio::test]
async fn search_escapes_phrases() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me/messages"))
        .and(|r: &Request| param(r, "$search").as_deref() == Some(r#""subject:\"q3 plan\"""#))
        .respond_with(ok_json(json!({ "value": [] })))
        .expect(1)
        .mount(&server)
        .await;
    let page = search(&provider, "subject:\"Q3   plan\"", None, 50)
        .await
        .unwrap();
    assert!(page.messages.is_empty());
}

#[tokio::test]
async fn an_empty_query_asks_nothing() {
    let (server, provider) = setup().await;
    for text in ["has:pdf", "in:inbox account:x", ""] {
        let page = search(&provider, text, None, 50).await.unwrap();
        assert!(page.messages.is_empty() && page.next.is_none());
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn search_follows_next_link_and_refuses_foreign_ones() {
    let (server, provider) = setup().await;
    let next = format!(
        "{}/me/messages?%24search=%22invoice*%22&%24top=1&%24skip=1",
        server.uri()
    );
    Mock::given(method("GET"))
        .and(path("/me/messages"))
        .and(|r: &Request| param(r, "$skip").is_none())
        .respond_with(ok_json(json!({
            "value": [found("first", "2024-05-02T09:00:00Z", Some("AAMkInbox"))],
            "@odata.nextLink": next,
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/me/messages"))
        .and(query_param("$skip", "1"))
        .and(prefers(PREFER_IMMUTABLE_ID))
        .respond_with(ok_json(json!({
            "value": [found("second", "2024-05-01T09:00:00Z", Some("AAMkInbox"))],
        })))
        .expect(1)
        .mount(&server)
        .await;

    let first = search(&provider, "invoice", None, 1).await.unwrap();
    assert_eq!(first.messages[0].remote_id, "first");
    assert_eq!(first.next.as_deref(), Some(next.as_str()));
    let second = search(&provider, "invoice", first.next.as_deref(), 1)
        .await
        .unwrap();
    assert_eq!(second.messages[0].remote_id, "second");
    assert_eq!(second.next, None);

    // A link elsewhere would get the token: never followed.
    let error = search(
        &provider,
        "invoice",
        Some("https://attacker.example/me/messages?$skip=2"),
        1,
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::Provider);
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn search_canonicalizes_unknown_folder_ids() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me/messages"))
        .respond_with(ok_json(json!({
            "value": [
                found("aliased", "2024-05-02T09:00:00Z", Some("AQMkInboxOtherForm")),
                found("hidden", "2024-05-01T09:00:00Z", Some("AAMkHidden")),
                found("again", "2024-04-01T09:00:00Z", Some("AQMkInboxOtherForm")),
            ]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/$batch"))
        .respond_with(batch_responder(|r| {
            match r["url"].as_str().unwrap() {
                "/me/mailFolders/AQMkInboxOtherForm?$select=id" => {
                    (200, json!({ "id": "AAMkInbox" }))
                }
                // A folder the app doesn't list (or one Graph won't show).
                _ => (200, json!({ "id": "AAMkHidden" })),
            }
        }))
        .expect(1)
        .mount(&server)
        .await;
    let page = search(&provider, "invoice", None, 50).await.unwrap();
    let inbox = local_id(ACCOUNT, EntityKind::Folder, "AAMkInbox");
    let hidden = local_id(ACCOUNT, EntityKind::Folder, "AAMkHidden");
    let folders: Vec<&str> = page.messages.iter().map(|m| m.folder_id.as_str()).collect();
    assert_eq!(folders, [inbox.as_str(), hidden.as_str(), inbox.as_str()]);
    // Each unknown folder looked up once.
    let lookups = batched_requests(&server).await;
    assert_eq!(lookups.len(), 2);
    assert_eq!(lookups[0]["headers"]["Prefer"], PREFER_IMMUTABLE_ID);
}

#[tokio::test]
async fn search_errors_map_like_other_calls() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me/messages"))
        .and(|r: &Request| param(r, "$search").is_some_and(|s| s.contains("bad")))
        .respond_with(graph_error(
            400,
            "BadRequest",
            "Syntax error: character '(' is not valid.",
        ))
        // Without dates, a refusal isn't asked again.
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/me/messages"))
        .and(|r: &Request| param(r, "$search").is_some_and(|s| s.contains("expired")))
        .respond_with(graph_error(401, "InvalidAuthenticationToken", "Expired."))
        .mount(&server)
        .await;
    let error = search(&provider, "bad", None, 50).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::Provider);
    assert_eq!(error.message, "Syntax error: character '(' is not valid.");
    assert_eq!(error.http_status(), Some(400));
    let error = search(&provider, "expired", None, 50).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::AuthRequired);
}

/// 1 October 2026 in UTC−4.
fn october_first() -> Received {
    Received {
        since: Some(Utc.with_ymd_and_hms(2026, 10, 1, 4, 0, 0).unwrap()),
        before: Some(Utc.with_ymd_and_hms(2026, 10, 2, 4, 0, 0).unwrap()),
    }
}

#[tokio::test]
async fn a_search_scoped_to_one_folder_asks_that_folder() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me/mailFolders/AAMkArchive/messages"))
        .and(|r: &Request| param(r, "$search").as_deref() == Some(r#""invoice*""#))
        .and(query_param("$top", "50"))
        .and(|r: &Request| param(r, "$select").is_some_and(|s| s.ends_with(",parentFolderId")))
        .and(prefers(PREFER_IMMUTABLE_ID))
        .respond_with(ok_json(json!({
            "value": [found("old", "2023-03-01T09:00:00Z", Some("AAMkArchive"))]
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/me/messages"))
        .and(|r: &Request| param(r, "$search").as_deref() == Some(r#""invoice*""#))
        .respond_with(ok_json(json!({ "value": [] })))
        .expect(1)
        .mount(&server)
        .await;
    let query = SearchQuery::parse("invoice in:archive");
    let folders = folders();
    let one = ServerQuery {
        scope: Some(&folders[1..]),
        ..ServerQuery::whole_mailbox(&query, &folders)
    };
    let page = provider.search(&one, None, 50).await.unwrap();
    assert_eq!(
        page.messages[0].folder_id,
        local_id(ACCOUNT, EntityKind::Folder, "AAMkArchive")
    );
    // Graph can't say two folders: the whole mailbox, which the caller
    // checks.
    let two = ServerQuery {
        scope: Some(&folders[..]),
        ..ServerQuery::whole_mailbox(&query, &folders)
    };
    provider.search(&two, None, 50).await.unwrap();
}

#[tokio::test]
async fn filters_alone_go_to_filter_newest_first() {
    let (server, provider) = setup().await;
    let next = format!(
        "{}/me/messages?%24filter=x&%24orderby=receivedDateTime+desc&%24skip=1",
        server.uri()
    );
    Mock::given(method("GET"))
        .and(path("/me/messages"))
        .and(|r: &Request| {
            param(r, "$filter").as_deref()
                == Some(
                    "receivedDateTime ge 2026-10-01T04:00:00Z and receivedDateTime lt \
                     2026-10-02T04:00:00Z and isRead eq false and flag/flagStatus eq 'flagged'",
                )
        })
        .and(query_param("$orderby", "receivedDateTime desc"))
        .and(query_param("$top", "50"))
        .and(|r: &Request| param(r, "$search").is_none() && param(r, "$skip").is_none())
        .and(|r: &Request| param(r, "$select").is_some_and(|s| s.ends_with(",parentFolderId")))
        .and(prefers(PREFER_IMMUTABLE_ID))
        .respond_with(ok_json(json!({
            "value": [found("first", "2026-10-01T09:00:00Z", Some("AAMkInbox"))],
            "@odata.nextLink": next,
        })))
        .expect(1)
        .mount(&server)
        .await;
    // Its pages are followed like any.
    Mock::given(method("GET"))
        .and(path("/me/messages"))
        .and(query_param("$skip", "1"))
        .respond_with(ok_json(json!({
            "value": [found("second", "2026-10-01T08:00:00Z", Some("AAMkInbox"))],
        })))
        .expect(1)
        .mount(&server)
        .await;
    let query = SearchQuery::parse("is:unread is:flagged received:2026-10-01");
    let folders = folders();
    let search = ServerQuery {
        received: october_first(),
        ..ServerQuery::whole_mailbox(&query, &folders)
    };
    let first = provider.search(&search, None, 50).await.unwrap();
    assert_eq!(first.messages[0].remote_id, "first");
    let second = provider
        .search(&search, first.next.as_deref(), 50)
        .await
        .unwrap();
    assert_eq!(second.messages[0].remote_id, "second");
    assert_eq!(second.next, None);
}

#[tokio::test]
async fn filters_alone_in_one_folder() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me/mailFolders/AAMkInbox/messages"))
        .and(|r: &Request| {
            param(r, "$filter").as_deref()
                == Some("receivedDateTime ge 1900-01-01T00:00:00Z and hasAttachments eq true")
        })
        .and(query_param("$orderby", "receivedDateTime desc"))
        .respond_with(ok_json(json!({ "value": [] })))
        .expect(1)
        .mount(&server)
        .await;
    let query = SearchQuery::parse("has:attachment in:inbox");
    let folders = folders();
    let search = ServerQuery {
        scope: Some(&folders[..1]),
        ..ServerQuery::whole_mailbox(&query, &folders)
    };
    provider.search(&search, None, 50).await.unwrap();
}

/// The `$search` a request carried, or "".
fn searched(request: &Request) -> String {
    param(request, "$search").unwrap_or_default()
}

#[tokio::test]
async fn dates_go_with_the_words_until_the_mailbox_refuses_them() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me/messages"))
        .and(|r: &Request| searched(r).contains("received"))
        .respond_with(graph_error(400, "BadRequest", "Unsupported property."))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/me/messages"))
        .and(|r: &Request| !searched(r).contains("received"))
        .respond_with(ok_json(json!({
            "value": [found("hit", "2026-10-01T09:00:00Z", Some("AAMkInbox"))]
        })))
        .expect(2)
        .mount(&server)
        .await;
    let query = SearchQuery::parse("invoice received:2026-10-01");
    let folders = folders();
    let search = ServerQuery {
        received: october_first(),
        ..ServerQuery::whole_mailbox(&query, &folders)
    };
    // Refused with dates: asked again without them, and found.
    let page = provider.search(&search, None, 50).await.unwrap();
    assert_eq!(page.messages[0].remote_id, "hit");
    // From then on, never sent them.
    provider.search(&search, None, 50).await.unwrap();
    let asked: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(searched)
        .collect();
    assert_eq!(
        asked,
        [
            r#""invoice* AND received>=09/30/2026 AND received<=10/03/2026""#,
            r#""invoice*""#,
            r#""invoice*""#,
        ]
    );
}

#[tokio::test]
async fn other_failures_with_dates_are_not_asked_again() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me/messages"))
        .respond_with(graph_error(403, "ErrorAccessDenied", "Access is denied."))
        .expect(1)
        .mount(&server)
        .await;
    let query = SearchQuery::parse("invoice received:2026-10-01");
    let folders = folders();
    let search = ServerQuery {
        received: october_first(),
        ..ServerQuery::whole_mailbox(&query, &folders)
    };
    let error = provider.search(&search, None, 50).await.unwrap_err();
    assert_eq!(error.http_status(), Some(403));
}
