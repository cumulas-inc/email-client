//! The setup sign-in's tokens: the mail app's token machinery on a memory
//! store of the session's own.

use std::collections::HashMap;
use std::time::Duration;

use serde_json::json;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{CLIENT, TENANT, clock, config, token_response};
use crate::auth::admin_setup::{AdminTokens, COMPLIANCE_SCOPES, Resource};
use crate::orgsetup::runlog::{Entry, RunLog};

fn token_path() -> String {
    format!("/{TENANT}/oauth2/v2.0/token")
}

fn refused(code: u32) -> ResponseTemplate {
    ResponseTemplate::new(400).set_body_json(json!({
        "error": "invalid_scope",
        "error_description": format!("AADSTS{code}: The provided value for scope isn't valid."),
        "error_codes": [code],
    }))
}

async fn forms(server: &MockServer) -> Vec<HashMap<String, String>> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| url::form_urlencoded::parse(&r.body).into_owned().collect())
        .collect()
}

fn notes(log: &RunLog) -> Vec<String> {
    log.entries()
        .into_iter()
        .filter_map(|e| match e {
            Entry::Note { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn exchange_tokens_refresh_with_the_in_memory_refresh_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(token_path()))
        .respond_with(token_response("exchange-2"))
        .mount(&server)
        .await;
    let (_, wall) = clock();
    let tokens = AdminTokens::new(&config(&server), "refresh-0", RunLog::default(), wall);
    assert_eq!(
        tokens.token(Resource::Exchange).await.unwrap(),
        "exchange-2"
    );
    // Cached until it's within the margin.
    assert_eq!(
        tokens.token(Resource::Exchange).await.unwrap(),
        "exchange-2"
    );
    tokens.invalidate(Resource::Exchange).await;
    tokens.token(Resource::Exchange).await.unwrap();
    let forms = forms(&server).await;
    assert_eq!(forms.len(), 2);
    assert_eq!(forms[0]["client_id"], CLIENT);
    assert_eq!(forms[0]["grant_type"], "refresh_token");
    assert_eq!(forms[0]["refresh_token"], "refresh-0");
    assert_eq!(
        forms[0]["scope"],
        "https://outlook.office365.com/Exchange.Manage offline_access"
    );
    // Microsoft rotated it: the new one is kept, in memory.
    assert_eq!(forms[1]["refresh_token"], "refresh-rotated");
}

#[tokio::test]
async fn a_primed_token_is_used_until_its_margin_and_expires_by_the_wall_clock() {
    let server = MockServer::start().await;
    Mock::given(path(token_path()))
        .respond_with(token_response("exchange-2"))
        .mount(&server)
        .await;
    let (wall, clock) = clock();
    let tokens = AdminTokens::new(&config(&server), "refresh-0", RunLog::default(), clock);
    tokens
        .prime(Resource::Exchange, "signed-in", Duration::from_secs(3600))
        .await;
    assert_eq!(tokens.token(Resource::Exchange).await.unwrap(), "signed-in");
    assert!(server.received_requests().await.unwrap().is_empty());
    // A night asleep: `Instant` stood still, the wall clock didn't.
    *wall.lock().unwrap() += Duration::from_secs(8 * 3600);
    assert_eq!(
        tokens.token(Resource::Exchange).await.unwrap(),
        "exchange-2"
    );

    // One that lasts less than the refresh margin is refreshed at once.
    tokens
        .prime(Resource::Exchange, "nearly-gone", Duration::from_secs(60))
        .await;
    assert_eq!(
        tokens.token(Resource::Exchange).await.unwrap(),
        "exchange-2"
    );
}

#[tokio::test]
async fn compliance_tries_each_scope_form_and_keeps_the_one_that_worked() {
    let server = MockServer::start().await;
    Mock::given(path(token_path()))
        .and(body_string_contains("%2F.default"))
        .respond_with(refused(70011))
        .mount(&server)
        .await;
    Mock::given(path(token_path()))
        .and(body_string_contains(
            "ps.compliance.protection.outlook.com%2FExchange.Manage",
        ))
        .respond_with(token_response("compliance-1"))
        .mount(&server)
        .await;
    let log = RunLog::default();
    let (_, clock) = clock();
    let tokens = AdminTokens::new(&config(&server), "refresh-0", log.clone(), clock);
    assert_eq!(
        tokens.token(Resource::Compliance).await.unwrap(),
        "compliance-1"
    );
    tokens.invalidate(Resource::Compliance).await;
    assert_eq!(
        tokens.token(Resource::Compliance).await.unwrap(),
        "compliance-1"
    );
    let scopes: Vec<String> = forms(&server)
        .await
        .into_iter()
        .map(|f| f["scope"].clone())
        .collect();
    assert_eq!(
        scopes,
        [
            format!("{} offline_access", COMPLIANCE_SCOPES[0]),
            format!("{} offline_access", COMPLIANCE_SCOPES[1]),
            // Remembered: the refused form isn't tried again.
            format!("{} offline_access", COMPLIANCE_SCOPES[1]),
        ]
    );
    assert_eq!(
        notes(&log),
        [
            format!(
                "Security & Compliance token: {} refused (invalid_scope, AADSTS70011)",
                COMPLIANCE_SCOPES[0]
            ),
            format!(
                "Security & Compliance token: {} issued",
                COMPLIANCE_SCOPES[1]
            ),
        ]
    );
}

#[tokio::test]
async fn both_scope_forms_refused_is_an_error_naming_both_codes() {
    let server = MockServer::start().await;
    Mock::given(path(token_path()))
        .and(body_string_contains("%2F.default"))
        .respond_with(refused(70011))
        .mount(&server)
        .await;
    Mock::given(path(token_path()))
        .respond_with(refused(65001))
        .mount(&server)
        .await;
    let (_, clock) = clock();
    let tokens = AdminTokens::new(&config(&server), "refresh-0", RunLog::default(), clock);
    let error = tokens.token(Resource::Compliance).await.unwrap_err();
    assert!(error.message.contains("AADSTS70011"), "{}", error.message);
    assert!(error.message.contains("AADSTS65001"), "{}", error.message);
}

#[tokio::test]
async fn a_refused_refresh_ends_the_setup_sign_in() {
    let server = MockServer::start().await;
    Mock::given(path(token_path()))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": "invalid_grant",
            "error_description": "AADSTS700082: The refresh token has expired due to inactivity.",
            "error_codes": [700082],
        })))
        .mount(&server)
        .await;
    let (_, clock) = clock();
    let tokens = AdminTokens::new(&config(&server), "refresh-0", RunLog::default(), clock);
    let error = tokens.token(Resource::Exchange).await.unwrap_err();
    assert_eq!(error.code, crate::ErrorCode::AuthRequired);
    assert_eq!(error.details.unwrap()["reason"], "setupSignedOut");
}

#[tokio::test]
async fn dropping_the_tokens_drops_the_refresh_token() {
    let server = MockServer::start().await;
    let (_, clock) = clock();
    let tokens = AdminTokens::new(&config(&server), "refresh-0", RunLog::default(), clock);
    let store = tokens.store();
    assert!(store.upgrade().is_some());
    drop(tokens);
    assert!(store.upgrade().is_none());
}
