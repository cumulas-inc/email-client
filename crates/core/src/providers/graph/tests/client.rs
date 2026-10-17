//! Error mapping, retries, paging and batching.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{authorized, batch_responder, batched_requests, graph_error, ok_json, setup};
use crate::auth::microsoft::{MicrosoftAuthConfig, MicrosoftTokenSource};
use crate::auth::{NoGrantRecord, StaticToken};
use crate::error::ErrorCode;
use crate::providers::graph::GraphProvider;
use crate::providers::{MailProvider, MessageLocator, TasksProvider};
use crate::secrets::{MemorySecretStore, SecretStore, keys};

fn lists_body() -> serde_json::Value {
    json!({ "value": [{ "id": "list-1", "displayName": "Tasks", "wellknownListName": "defaultList" }] })
}

#[tokio::test]
async fn sends_the_bearer_token() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me/todo/lists"))
        .and(authorized)
        .respond_with(ok_json(lists_body()))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(provider.list_task_lists().await.unwrap().len(), 1);
}

/// A Graph provider whose real Microsoft token source refreshes against the
/// mock server, with a cached token that Graph will reject.
async fn provider_with_revoked_token(server: &MockServer) -> GraphProvider {
    Mock::given(method("POST"))
        .and(path("/common/oauth2/v2.0/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(ok_json(json!({
            "token_type": "Bearer",
            "expires_in": 3599,
            "access_token": "fresh-token",
            "refresh_token": "refresh-0",
        })))
        .expect(1)
        .named("token refresh")
        .mount(server)
        .await;
    let secrets = Arc::new(MemorySecretStore::default());
    secrets
        .set(&keys::refresh_token("acct"), "refresh-0")
        .await
        .unwrap();
    let config = MicrosoftAuthConfig {
        client_id: "test-client-id".into(),
        tenant: "common".into(),
        authority_host: server.uri(),
        consent_scopes: Vec::new(),
        graph_scopes: Vec::new(),
        published: false,
    };
    let tokens = Arc::new(MicrosoftTokenSource::new(
        config,
        "acct",
        secrets,
        false,
        Arc::new(NoGrantRecord),
    ));
    tokens
        .prime("revoked-token", Duration::from_secs(3600))
        .await;
    GraphProvider::with_base_url("acct", tokens, server.uri())
}

fn token_rejected() -> ResponseTemplate {
    graph_error(
        401,
        "InvalidAuthenticationToken",
        "Access token has expired or is not yet valid.",
    )
}

#[tokio::test]
async fn rejected_tokens_are_refreshed_once_and_the_request_retried() {
    let server = MockServer::start().await;
    let provider = provider_with_revoked_token(&server).await;
    Mock::given(path("/me/todo/lists"))
        .and(header("authorization", "Bearer revoked-token"))
        .respond_with(token_rejected())
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/me/todo/lists"))
        .and(header("authorization", "Bearer fresh-token"))
        .respond_with(ok_json(lists_body()))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(provider.list_task_lists().await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_second_rejection_requires_sign_in() {
    let server = MockServer::start().await;
    let provider = provider_with_revoked_token(&server).await;
    Mock::given(path("/me/todo/lists"))
        .respond_with(token_rejected())
        .expect(2)
        .mount(&server)
        .await;
    let error = provider.list_task_lists().await.unwrap_err();
    assert_eq!(error.code, ErrorCode::AuthRequired);
    assert!(error.message.contains("Access token has expired"));
}

#[tokio::test]
async fn not_found_and_other_errors_carry_graph_messages() {
    let (server, provider) = setup().await;
    Mock::given(path("/me/todo/lists"))
        .respond_with(graph_error(
            404,
            "ErrorItemNotFound",
            "The specified object was not found in the store.",
        ))
        .mount(&server)
        .await;
    let error = provider.list_task_lists().await.unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);
    assert_eq!(
        error.message,
        "The specified object was not found in the store."
    );

    let (server, provider) = setup().await;
    Mock::given(path("/me/todo/lists"))
        .respond_with(graph_error(
            403,
            "ErrorAccessDenied",
            "Access is denied. Check credentials and try again.",
        ))
        .mount(&server)
        .await;
    let error = provider.list_task_lists().await.unwrap_err();
    assert_eq!(error.code, ErrorCode::Provider);
    assert_eq!(
        error.message,
        "Access is denied. Check credentials and try again."
    );
    let details = error.details.unwrap();
    assert_eq!(details["status"], 403);
    assert_eq!(details["code"], "ErrorAccessDenied");

    let (server, provider) = setup().await;
    Mock::given(path("/me/todo/lists"))
        .respond_with(ResponseTemplate::new(500).set_body_string("<html>oops</html>"))
        .mount(&server)
        .await;
    let error = provider.list_task_lists().await.unwrap_err();
    assert_eq!(error.code, ErrorCode::Provider);
    assert!(error.message.contains("500"));
}

#[tokio::test]
async fn throttled_requests_are_retried_after_the_given_delay() {
    let (server, provider) = setup().await;
    Mock::given(path("/me/todo/lists"))
        .respond_with(
            graph_error(429, "TooManyRequests", "Too many requests")
                .insert_header("Retry-After", "1"),
        )
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/me/todo/lists"))
        .respond_with(ok_json(lists_body()))
        .expect(1)
        .mount(&server)
        .await;
    let started = Instant::now();
    assert_eq!(provider.list_task_lists().await.unwrap().len(), 1);
    assert!(started.elapsed() >= Duration::from_millis(900));
}

#[tokio::test]
async fn retries_are_bounded() {
    let (server, provider) = setup().await;
    Mock::given(path("/me/todo/lists"))
        .respond_with(
            graph_error(503, "ServiceUnavailable", "Service unavailable")
                .insert_header("Retry-After", "0"),
        )
        .expect(4)
        .mount(&server)
        .await;
    let error = provider.list_task_lists().await.unwrap_err();
    assert_eq!(error.code, ErrorCode::Provider);
    assert_eq!(error.message, "Service unavailable");
}

#[tokio::test]
async fn transport_errors_are_network_errors() {
    // Nothing listens on the discard port.
    let provider = GraphProvider::with_base_url(
        "acct",
        Arc::new(StaticToken("t".into())),
        "http://127.0.0.1:9",
    );
    let error = provider.list_task_lists().await.unwrap_err();
    assert_eq!(error.code, ErrorCode::Network);
}

#[tokio::test]
async fn paging_links_to_other_servers_are_not_followed() {
    let (server, provider) = setup().await;
    Mock::given(path("/me/todo/lists"))
        .respond_with(ok_json(json!({
            "value": [{ "id": "list-1", "displayName": "Tasks" }],
            "@odata.nextLink": "https://attacker.example/me/todo/lists?$skip=1",
        })))
        .mount(&server)
        .await;
    let error = provider.list_task_lists().await.unwrap_err();
    assert_eq!(error.code, ErrorCode::Provider);
}

#[tokio::test]
async fn throttled_batch_parts_are_retried_alone() {
    let (server, provider) = setup().await;
    // First round: message 1 is throttled.
    Mock::given(method("POST"))
        .and(path("/$batch"))
        .respond_with(batch_responder(|r| {
            if r["url"].as_str().unwrap().contains("msg-1") {
                (
                    429,
                    json!({ "error": { "code": "TooManyRequests", "message": "slow down" } }),
                )
            } else {
                (200, json!({ "id": "msg-0", "isRead": true }))
            }
        }))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/$batch"))
        .respond_with(batch_responder(|_| (200, json!({ "isRead": true }))))
        .mount(&server)
        .await;
    let messages: Vec<_> = (0..2)
        .map(|i| MessageLocator {
            folder_remote_id: "inbox".into(),
            remote_id: format!("msg-{i}"),
        })
        .collect();
    provider.set_read(&messages, true).await.unwrap();

    let sent = batched_requests(&server).await;
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[2]["url"], "/me/messages/msg-1");
}

#[tokio::test]
async fn batch_part_failures_are_reported() {
    let (server, provider) = setup().await;
    Mock::given(path("/$batch"))
        .respond_with(batch_responder(|_| {
            (
                403,
                json!({ "error": { "code": "ErrorAccessDenied", "message": "Access is denied." } }),
            )
        }))
        .mount(&server)
        .await;
    let error = provider
        .set_flagged(
            &[MessageLocator {
                folder_remote_id: "inbox".into(),
                remote_id: "msg-0".into(),
            }],
            true,
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Provider);
    assert_eq!(error.message, "Access is denied.");
}
