//! Tests of the Graph provider against a mock Graph endpoint. Larger response
//! bodies live in `tests/fixtures/graph/`; `{{base}}` in them is replaced by
//! the mock server's URL so paging links point back at it.

mod calendar;
mod client;
mod contacts;
mod mail;
mod tasks;

use std::sync::Arc;

use serde_json::{Value, json};
use wiremock::{MockServer, Request, ResponseTemplate};

use super::GraphProvider;
use crate::auth::StaticToken;

pub(super) const ACCOUNT: &str = "acct-1";

async fn setup() -> (MockServer, GraphProvider) {
    let server = MockServer::start().await;
    let provider = GraphProvider::with_base_url(
        ACCOUNT,
        Arc::new(StaticToken("test-token".into())),
        server.uri(),
    );
    (server, provider)
}

fn fixture(name: &str, server: &MockServer) -> Value {
    let path = format!("{}/tests/fixtures/graph/{name}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text.replace("{{base}}", &server.uri())).unwrap()
}

fn ok_json(body: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(body)
}

/// Matches requests that carry `preference` in a `Prefer` header.
fn prefers(preference: &'static str) -> impl Fn(&Request) -> bool + Send + Sync {
    move |request: &Request| {
        request.headers.get_all("prefer").iter().any(|v| {
            v.to_str()
                .is_ok_and(|v| v.split(',').any(|p| p.trim() == preference))
        })
    }
}

/// Matches requests with a bearer token.
fn authorized(request: &Request) -> bool {
    request
        .headers
        .get("authorization")
        .is_some_and(|v| v == "Bearer test-token")
}

/// Answers a JSON batch by calling `answer` for each sub-request; it returns
/// `(status, body)`.
fn batch_responder(
    answer: impl Fn(&Value) -> (u16, Value) + Send + Sync + 'static,
) -> impl Fn(&Request) -> ResponseTemplate + Send + Sync {
    move |request: &Request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let responses: Vec<Value> = body["requests"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                let (status, body) = answer(r);
                json!({
                    "id": r["id"],
                    "status": status,
                    "headers": { "Content-Type": "application/json" },
                    "body": body,
                })
            })
            .collect();
        ok_json(json!({ "responses": responses }))
    }
}

/// The sub-requests of every batch the server received, in order.
async fn batched_requests(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/$batch")
        .flat_map(|r| {
            let body: Value = serde_json::from_slice(&r.body).unwrap();
            body["requests"].as_array().unwrap().clone()
        })
        .collect()
}

fn graph_error(status: u16, code: &str, message: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(json!({
        "error": {
            "code": code,
            "message": message,
            "innerError": {
                "date": "2026-10-14T10:00:00",
                "request-id": "8d1a1a2c-0000-0000-0000-000000000000",
                "client-request-id": "8d1a1a2c-0000-0000-0000-000000000000"
            }
        }
    }))
}
