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
use crate::providers::graph::client::{BatchRequest, GraphClient};
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

#[tokio::test]
async fn beta_urls_replace_the_version_root() {
    let (server, provider) = setup().await;
    assert_eq!(
        provider.client.beta_url("/me/x"),
        format!("{}/beta/me/x", server.uri())
    );
    let client = GraphClient::new(
        crate::providers::graph::GRAPH_BASE_URL.into(),
        Arc::new(StaticToken("t".into())),
    );
    assert_eq!(
        client.beta_url("/me/security/informationProtection/sensitivityLabels"),
        "https://graph.microsoft.com/beta/me/security/informationProtection/sensitivityLabels"
    );
    assert!(client.is_own_link("https://graph.microsoft.com/beta/me?$skiptoken=1"));
}

/// A server that answers one request with `body` in chunks of `chunk`
/// bytes, without a Content-Length.
async fn chunked_server(body: Vec<u8>, chunk: usize) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buf = [0u8; 1024];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = socket.read(&mut buf).await.unwrap();
            if n == 0 {
                return;
            }
            request.extend_from_slice(&buf[..n]);
        }
        let mut response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        for part in body.chunks(chunk) {
            response.extend_from_slice(format!("{:x}\r\n", part.len()).as_bytes());
            response.extend_from_slice(part);
            response.extend_from_slice(b"\r\n");
        }
        response.extend_from_slice(b"0\r\n\r\n");
        // The reader may hang up early.
        let _ = socket.write_all(&response).await;
    });
    format!("http://{address}")
}

#[tokio::test]
async fn limited_downloads_are_refused_by_length_or_as_read() {
    let (server, provider) = setup().await;
    Mock::given(path("/big"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![7u8; 3000]))
        .mount(&server)
        .await;
    let url = format!("{}/big", server.uri());
    let client = &provider.client;
    assert_eq!(
        client
            .get_bytes_limited(&url, &[], 3000)
            .await
            .unwrap()
            .len(),
        3000
    );
    let error = crate::Error::from(client.get_bytes_limited(&url, &[], 2999).await.unwrap_err());
    assert_eq!(error.code, ErrorCode::Provider);

    // Without a length: refused once more than the limit has come.
    let base = chunked_server(vec![7u8; 3000], 500).await;
    let chunked = GraphClient::new(base.clone(), Arc::new(StaticToken("t".into())));
    let error = crate::Error::from(
        chunked
            .get_bytes_limited(&format!("{base}/big"), &[], 1200)
            .await
            .unwrap_err(),
    );
    assert_eq!(error.code, ErrorCode::Provider);
    let base = chunked_server(vec![7u8; 3000], 500).await;
    let chunked = GraphClient::new(base.clone(), Arc::new(StaticToken("t".into())));
    let read = chunked
        .get_bytes_limited(&format!("{base}/big"), &[], 3000)
        .await
        .unwrap();
    assert_eq!(read.len(), 3000);
}

#[tokio::test]
async fn prefixes_stop_at_their_length() {
    let body: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
    let base = chunked_server(body.clone(), 300).await;
    let client = GraphClient::new(base.clone(), Arc::new(StaticToken("t".into())));
    let prefix = client
        .get_bytes_prefix(&format!("{base}/p7m"), &[], 512)
        .await
        .unwrap();
    assert_eq!(prefix, body[..512]);
    // Shorter than asked: all of it.
    let (server, provider) = setup().await;
    Mock::given(path("/short"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1u8; 100]))
        .mount(&server)
        .await;
    let short = provider
        .client
        .get_bytes_prefix(&format!("{}/short", server.uri()), &[], 512)
        .await
        .unwrap();
    assert_eq!(short.len(), 100);
}

#[tokio::test]
async fn sequential_batches_name_only_requests_sent_with_them() {
    let (server, provider) = setup().await;
    // b is throttled once; c, after it, fails as Graph fails a dependent.
    Mock::given(method("POST"))
        .and(path("/$batch"))
        .respond_with(batch_responder(|r| match r["id"].as_str().unwrap() {
            "a" => (200, json!({ "id": "a" })),
            "b" => (
                429,
                json!({ "error": { "code": "TooManyRequests", "message": "slow down" } }),
            ),
            _ => (
                424,
                json!({ "error": { "code": "FailedDependency", "message": "b failed" } }),
            ),
        }))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/$batch"))
        .respond_with(batch_responder(|_| (200, json!({ "id": "b" }))))
        .mount(&server)
        .await;
    let requests = [
        BatchRequest::get("a", "/me/messages/a".into()),
        BatchRequest::get("b", "/me/messages/b".into()).after("a"),
        BatchRequest::get("c", "/me/messages/c".into()).after("b"),
    ];
    let responses = provider.client.batch(&requests, &[]).await.unwrap();
    assert!(responses["a"].is_success() && responses["b"].is_success());
    assert_eq!(responses["c"].status, 424);
    let sent = batched_requests(&server).await;
    assert_eq!(sent.len(), 4);
    assert_eq!(sent[0].get("dependsOn"), None);
    assert_eq!(sent[1]["dependsOn"], json!(["a"]));
    assert_eq!(sent[2]["dependsOn"], json!(["b"]));
    // Retried alone: after nothing.
    assert_eq!(sent[3]["id"], "b");
    assert_eq!(sent[3].get("dependsOn"), None);
}
