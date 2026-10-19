//! Finding the Security & Compliance host.

use serde_json::json;
use url::Url;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{ADMIN, TENANT, token_response, tokens};
use crate::orgsetup::discovery::compliance_host;
use crate::orgsetup::invoke::{Anchor, InvokeError, http_client};
use crate::orgsetup::runlog::{Entry, RunLog};

const BANNER: &str =
    "/adminapi/beta/85f67998-e79a-4173-95c2-90b0e6dc231d/EXOBanner('AutogenSession')";

async fn server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path(format!("/{TENANT}/oauth2/v2.0/token")))
        .respond_with(token_response("compliance-token"))
        .mount(&server)
        .await;
    server
}

async fn discover(
    server: &MockServer,
    log: &RunLog,
    initial: Option<&str>,
) -> Result<Url, InvokeError> {
    let tokens = tokens(server, log).await;
    let anchors = Anchor::compliance(ADMIN, initial);
    let discovery = Url::parse(&server.uri()).unwrap();
    compliance_host(
        &http_client().unwrap(),
        &tokens,
        TENANT,
        &anchors,
        &discovery,
        log,
    )
    .await
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
async fn a_redirect_names_the_regional_host() {
    let server = server().await;
    Mock::given(method("GET"))
        .and(path(BANNER))
        .and(wiremock::matchers::query_param("Version", "3.9.2"))
        .and(header("authorization", "Bearer compliance-token"))
        .and(header("X-AnchorMailbox", "UPN:admin@contoso.com"))
        .respond_with(ResponseTemplate::new(302).insert_header(
            "Location",
            "https://eur03b.ps.compliance.protection.outlook.com/adminapi/beta/x/EXOBanner('AutogenSession')?secret=1",
        ))
        .expect(1)
        .mount(&server)
        .await;
    let log = RunLog::default();
    let host = discover(&server, &log, None).await.unwrap();
    assert_eq!(
        host.as_str(),
        "https://eur03b.ps.compliance.protection.outlook.com/"
    );
    let notes = notes(&log);
    assert!(
        notes.contains(&"Security & Compliance host: eur03b.ps.compliance.protection.outlook.com (redirect, anchor UPN)".to_owned()),
        "{notes:?}"
    );
    // The host only: never the Location's path or query.
    let text = crate::orgsetup::details_text(&log);
    assert!(
        !text.contains("secret=1") && !text.contains("/adminapi/beta/x"),
        "{text}"
    );
}

#[tokio::test]
async fn the_admin_protection_form_is_read_as_its_rest_host() {
    let server = server().await;
    Mock::given(path(BANNER))
        .respond_with(ResponseTemplate::new(302).insert_header(
            "Location",
            "https://nam12b.admin.protection.outlook.com:446/powershell-liveid?x=y",
        ))
        .mount(&server)
        .await;
    let log = RunLog::default();
    let host = discover(&server, &log, None).await.unwrap();
    assert_eq!(
        host.as_str(),
        "https://nam12b.ps.compliance.protection.outlook.com/"
    );
}

#[tokio::test]
async fn a_success_means_the_global_host() {
    let server = server().await;
    Mock::given(path(BANNER))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    let log = RunLog::default();
    let host = discover(&server, &log, None).await.unwrap();
    assert_eq!(host.origin(), Url::parse(&server.uri()).unwrap().origin());
    assert!(
        notes(&log)
            .iter()
            .any(|n| n.contains("(global, anchor UPN)"))
    );
}

#[tokio::test]
async fn a_refused_mailbox_tries_the_system_mailbox_at_the_initial_domain() {
    let server = server().await;
    Mock::given(header("X-AnchorMailbox", "UPN:admin@contoso.com"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    Mock::given(header(
        "X-AnchorMailbox",
        "UPN:SystemMailbox{bb558c35-97f1-4cb9-8ff7-d53741dc928c}@contoso.onmicrosoft.com",
    ))
    .respond_with(ResponseTemplate::new(302).insert_header(
        "Location",
        "https://eur03b.ps.compliance.protection.outlook.com/adminapi/beta/x",
    ))
    .mount(&server)
    .await;
    let log = RunLog::default();
    let host = discover(&server, &log, Some("contoso.onmicrosoft.com"))
        .await
        .unwrap();
    assert_eq!(
        host.host_str(),
        Some("eur03b.ps.compliance.protection.outlook.com")
    );
    assert!(
        notes(&log)
            .iter()
            .any(|n| n.contains("anchor SystemMailbox"))
    );
}

#[tokio::test]
async fn refused_with_both_anchors_is_an_error_with_its_detail() {
    let server = server().await;
    Mock::given(path(BANNER))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "error": { "code": "Forbidden", "message": "Access denied" }
        })))
        .expect(2)
        .mount(&server)
        .await;
    let log = RunLog::default();
    let error = discover(&server, &log, Some("contoso.onmicrosoft.com"))
        .await
        .unwrap_err();
    match error {
        InvokeError::Forbidden(detail) => assert_eq!(detail.message, "Access denied"),
        other => panic!("{other:?}"),
    }
    let call = log
        .entries()
        .into_iter()
        .find_map(|e| match e {
            Entry::Call(call) => Some(call),
            _ => None,
        })
        .unwrap();
    assert_eq!(call.cmdlet, "EXOBanner");
    assert_eq!(
        call.attempts,
        [(Some("UPN"), Some(403)), (Some("SystemMailbox"), Some(403))]
    );
}

#[tokio::test]
async fn a_redirect_outside_microsofts_protection_hosts_is_refused() {
    let server = server().await;
    Mock::given(path(BANNER))
        .respond_with(
            ResponseTemplate::new(302).insert_header("Location", "https://evil.example/x"),
        )
        .mount(&server)
        .await;
    let log = RunLog::default();
    let error = discover(&server, &log, None).await.unwrap_err();
    assert_eq!(error, InvokeError::Redirect("evil.example".into()));
}
