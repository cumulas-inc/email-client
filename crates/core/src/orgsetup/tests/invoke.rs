//! The InvokeCommand client against a mock server.

use std::fmt::Write as _;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use url::Url;
use wiremock::matchers::{body_json, header, method, path, path_regex};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::{ADMIN, TENANT, token_response, tokens};
use crate::auth::admin_setup::Resource;
use crate::orgsetup::invoke::{
    Anchor, CallKind, Cmdlet, ErrorDetail, HostRules, InvokeClient, InvokeError, InvokeTarget,
    SYSTEM_MAILBOX, classify, error_detail, http_client,
};
use crate::orgsetup::runlog::{CallRecord, Entry, RunLog};

const INVOKE: &str = "/adminapi/beta/85f67998-e79a-4173-95c2-90b0e6dc231d/InvokeCommand";
const UPN: &str = "UPN:admin@contoso.com";

fn system_mailbox() -> String {
    format!("APP:{SYSTEM_MAILBOX}@{TENANT}")
}

async fn client_with(
    server: &MockServer,
    log: &RunLog,
    resource: Resource,
    hosts: HostRules,
    cancel: CancellationToken,
) -> InvokeClient {
    let base = Url::parse(&server.uri()).unwrap();
    InvokeClient::new(
        http_client().unwrap(),
        InvokeTarget {
            resource,
            base,
            tenant_id: TENANT.into(),
            anchors: Anchor::exchange(ADMIN, TENANT),
        },
        Arc::new(tokens(server, log).await),
        hosts,
        log.clone(),
        cancel,
    )
}

async fn client(server: &MockServer, log: &RunLog) -> InvokeClient {
    let hosts = HostRules::also(&Url::parse(&server.uri()).unwrap());
    client_with(
        server,
        log,
        Resource::Exchange,
        hosts,
        CancellationToken::new(),
    )
    .await
}

fn rules() -> Cmdlet {
    Cmdlet::read("Get-TransportRule", "Read mail flow rules").with("ResultSize", "Unlimited")
}

fn create() -> Cmdlet {
    Cmdlet::write("New-TransportRule", "Create “Cumulas Mail: Encrypt”")
        .with("Name", "Cumulas Mail: Encrypt")
        .with("RemoveHeader", "x-cumulas-protect")
}

fn ok(values: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "@odata.context": "x", "value": values }))
}

fn calls(log: &RunLog) -> Vec<CallRecord> {
    log.entries()
        .into_iter()
        .filter_map(|e| match e {
            Entry::Call(call) => Some(call),
            _ => None,
        })
        .collect()
}

async fn invoke_requests(server: &MockServer) -> Vec<Request> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path().ends_with("/InvokeCommand"))
        .collect()
}

fn header_of<'r>(request: &'r Request, name: &str) -> Option<&'r str> {
    request.headers.get(name).and_then(|v| v.to_str().ok())
}

#[tokio::test]
async fn requests_carry_every_header_and_only_the_parameters_set() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(INVOKE))
        .and(body_json(json!({
            "CmdletInput": { "CmdletName": "Get-TransportRule", "Parameters": { "ResultSize": "Unlimited" } }
        })))
        .respond_with(ok(json!([{ "Name": "A" }])))
        .expect(2)
        .mount(&server)
        .await;
    let log = RunLog::default();
    let output = client(&server, &log).await.invoke(&rules()).await.unwrap();
    assert_eq!(output.values, vec![json!({ "Name": "A" })]);

    let request = &invoke_requests(&server).await[0];
    assert_eq!(
        header_of(request, "authorization"),
        Some("Bearer exchange-token")
    );
    assert_eq!(
        header_of(request, "content-type"),
        Some("application/json; charset=utf-8")
    );
    assert_eq!(header_of(request, "accept"), Some("application/json"));
    assert_eq!(header_of(request, "prefer"), Some("odata.maxpagesize=1000"));
    assert_eq!(header_of(request, "x-anchormailbox"), Some(UPN));
    assert_eq!(
        header_of(request, "x-cmdletname"),
        Some("Get-TransportRule")
    );
    let app = format!("CumulasMail/{}", env!("CARGO_PKG_VERSION"));
    assert_eq!(
        header_of(request, "x-clientapplication"),
        Some(app.as_str())
    );
    assert_eq!(header_of(request, "user-agent"), Some(app.as_str()));
    let id = header_of(request, "client-request-id").unwrap();
    assert!(uuid::Uuid::parse_str(id).is_ok(), "{id}");
    // Only reqwest's own gzip, never set by us.
    assert!(header_of(request, "accept-encoding").is_none_or(|v| v == "gzip"));
    assert!(header_of(request, "cookie").is_none());

    // Each attempt gets its own request ID.
    client(&server, &log).await.invoke(&rules()).await.ok();
    let ids: Vec<String> = invoke_requests(&server)
        .await
        .iter()
        .map(|r| header_of(r, "client-request-id").unwrap().to_owned())
        .collect();
    assert_ne!(ids[0], ids[1]);
}

#[tokio::test]
async fn values_single_objects_empty_bodies_and_warnings() {
    let server = MockServer::start().await;
    Mock::given(header("X-CmdletName", "Get-IRMConfiguration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "@odata.context": "x", "AzureRMSLicensingEnabled": true,
            "@adminapi.warnings": ["The policy takes effect within an hour."],
        })))
        .mount(&server)
        .await;
    Mock::given(header("X-CmdletName", "Enable-TransportRule"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(header("X-CmdletName", "Get-TransportRule"))
        .respond_with(ok(json!([{ "Name": "A" }, { "Name": "B" }])))
        .mount(&server)
        .await;
    let log = RunLog::default();
    let client = client(&server, &log).await;
    let irm = client
        .invoke(&Cmdlet::read("Get-IRMConfiguration", "Read"))
        .await
        .unwrap();
    assert_eq!(
        irm.values,
        vec![json!({ "AzureRMSLicensingEnabled": true })]
    );
    assert_eq!(irm.warnings, ["The policy takes effect within an hour."]);
    let enabled = client
        .invoke(&Cmdlet::write("Enable-TransportRule", "Turn on").with("Identity", "g"))
        .await
        .unwrap();
    assert!(enabled.values.is_empty());
    assert_eq!(client.invoke(&rules()).await.unwrap().values.len(), 2);
    let logged = calls(&log);
    assert_eq!(
        logged[0].warnings,
        ["The policy takes effect within an hour."]
    );
    assert_eq!(logged[2].results, Some(2));
}

#[tokio::test]
async fn a_success_that_isnt_json_is_unexpected() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>sign in</html>"))
        .mount(&server)
        .await;
    let log = RunLog::default();
    let error = client(&server, &log)
        .await
        .invoke(&rules())
        .await
        .unwrap_err();
    assert!(matches!(error, InvokeError::Unexpected(_)), "{error:?}");
}

#[tokio::test]
async fn next_pages_are_posted_the_same_body_on_allowed_origins_only() {
    let server = MockServer::start().await;
    let next = format!(
        "{}/adminapi/beta/{TENANT}/InvokeCommand?$skiptoken=2",
        server.uri()
    );
    Mock::given(path(INVOKE))
        .and(wiremock::matchers::query_param("$skiptoken", "2"))
        .respond_with(ok(json!([{ "Name": "B" }])))
        .mount(&server)
        .await;
    Mock::given(path(INVOKE))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "value": [{ "Name": "A" }], "@odata.nextLink": next,
        })))
        .mount(&server)
        .await;
    let log = RunLog::default();
    let output = client(&server, &log).await.invoke(&rules()).await.unwrap();
    assert_eq!(
        output.values,
        vec![json!({ "Name": "A" }), json!({ "Name": "B" })]
    );
    let requests = invoke_requests(&server).await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].body, requests[1].body);
    assert_eq!(
        header_of(&requests[1], "authorization"),
        Some("Bearer exchange-token")
    );

    // A next page elsewhere is never asked for.
    let server = MockServer::start().await;
    Mock::given(path(INVOKE))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "value": [], "@odata.nextLink": "https://evil.example/adminapi/beta/t/InvokeCommand",
        })))
        .expect(1)
        .mount(&server)
        .await;
    let error = client(&server, &log)
        .await
        .invoke(&rules())
        .await
        .unwrap_err();
    assert_eq!(error, InvokeError::Redirect("evil.example".into()));

    // Nor a 21st page.
    let server = MockServer::start().await;
    let again = format!("{}{INVOKE}?page=again", server.uri());
    Mock::given(path(INVOKE))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "value": [{ "Name": "A" }], "@odata.nextLink": again,
        })))
        .expect(20)
        .mount(&server)
        .await;
    let error = client(&server, &log)
        .await
        .invoke(&rules())
        .await
        .unwrap_err();
    assert_eq!(error, InvokeError::Unexpected("too many pages".into()));
}

#[test]
fn error_bodies_are_read_defensively() {
    // A detail's message holding JSON.
    let body = json!({ "error": { "code": "BadRequest", "message": "outer", "details": [
        { "message": r#"{"message":"Rule name already exists.","exceptionType":"ArgumentException"}"# }
    ]}});
    let detail = error_detail(400, body.to_string().as_bytes(), None);
    assert_eq!(detail.message, "Rule name already exists.");
    assert_eq!(detail.exception_type.as_deref(), Some("ArgumentException"));
    assert_eq!(detail.code.as_deref(), Some("BadRequest"));
    assert_eq!(detail.status, Some(400));

    // `_|type|message`.
    let body = json!({ "error": { "details": [
        { "message": "|Microsoft.Exchange.Management.Tasks.ThrottlingException|Too many calls" }
    ]}});
    let detail = error_detail(400, body.to_string().as_bytes(), None);
    assert_eq!(detail.message, "Too many calls");
    assert_eq!(
        detail.exception_type.as_deref(),
        Some("Microsoft.Exchange.Management.Tasks.ThrottlingException")
    );

    // The inner exception, then the inner error, then the error.
    let body = json!({ "error": { "message": "outer", "innererror": {
        "message": "inner", "internalException": { "message": "innermost" }
    }}});
    assert_eq!(
        error_detail(500, body.to_string().as_bytes(), None).message,
        "innermost"
    );
    let body = json!({ "error": { "message": "outer", "innererror": { "message": "inner" } } });
    assert_eq!(
        error_detail(500, body.to_string().as_bytes(), None).message,
        "inner"
    );
    let body = json!({ "error": { "code": "Forbidden", "message": "User is not allowed to call Get-RoleGroup" } });
    let detail = error_detail(403, body.to_string().as_bytes(), None);
    assert_eq!(detail.message, "User is not allowed to call Get-RoleGroup");

    // Plain text; X-ExceptionType when nothing else names a type; one line,
    // control characters out, capped.
    let detail = error_detail(
        429,
        b"You have reached the maximum number of concurrent requests per tenant.\r\nPlease wait",
        Some("ThrottlingException"),
    );
    assert_eq!(
        detail.message,
        "You have reached the maximum number of concurrent requests per tenant.  Please wait"
    );
    assert_eq!(
        detail.exception_type.as_deref(),
        Some("ThrottlingException")
    );
    assert!(!detail.excerpt.contains('\n'));
    let long = "x".repeat(5000);
    let detail = error_detail(400, long.as_bytes(), None);
    assert_eq!(detail.message.chars().count(), 500);
    assert_eq!(detail.excerpt.chars().count(), 2000);
    // An empty body: the status.
    assert_eq!(error_detail(502, b"", None).message, "HTTP 502");
}

#[test]
fn failures_are_classified_first_match_wins() {
    let detail = |message: &str| ErrorDetail {
        message: message.into(),
        ..ErrorDetail::default()
    };
    let exchange = |status, message: &str| classify(status, detail(message), Resource::Exchange);
    let compliance =
        |status, message: &str| classify(status, detail(message), Resource::Compliance);
    assert!(matches!(
        exchange(401, "licensing"),
        InvokeError::Unauthorized(_)
    ));
    assert!(matches!(
        compliance(400, "The tenant isn't licensed for auto-labeling"),
        InvokeError::LicenseRequired(_)
    ));
    // In Exchange Online, licensing words are IRM's: a plain refusal.
    assert!(matches!(
        exchange(400, "IRM licensing isn't enabled"),
        InvokeError::Rejected(_)
    ));
    for forbidden in [
        "The term 'New-TransportRule' is not recognized as the name of a cmdlet",
        "User is not allowed to call Get-RoleGroup",
        "Access denied",
        "Access is denied.",
        "The operation isn't within your current write scope",
        "AccessDenied",
    ] {
        assert!(
            matches!(exchange(400, forbidden), InvokeError::Forbidden(_)),
            "{forbidden}"
        );
    }
    assert!(matches!(exchange(403, ""), InvokeError::Forbidden(_)));
    assert!(matches!(
        exchange(400, "A rule named 'X' already exists."),
        InvokeError::AlreadyExists(_)
    ));
    assert!(matches!(
        exchange(404, "The object couldn't be found on the server"),
        InvokeError::NotFound(_)
    ));
    assert!(matches!(
        classify(
            404,
            ErrorDetail {
                exception_type: Some("ManagementObjectNotFoundException".into()),
                ..detail("x")
            },
            Resource::Exchange
        ),
        InvokeError::NotFound(_)
    ));
    assert!(matches!(exchange(429, ""), InvokeError::Throttled(_)));
    assert!(matches!(
        exchange(
            400,
            "You have reached the maximum number of concurrent requests per tenant"
        ),
        InvokeError::Throttled(_)
    ));
    assert!(matches!(
        exchange(500, "Something"),
        InvokeError::Rejected(_)
    ));
}

#[tokio::test]
async fn throttling_is_retried_as_retry_after_says_then_given_up() {
    let server = MockServer::start().await;
    Mock::given(path(INVOKE))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "1"))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(path(INVOKE))
        .respond_with(ok(json!([])))
        .mount(&server)
        .await;
    let log = RunLog::default();
    let started = Instant::now();
    client(&server, &log).await.invoke(&create()).await.unwrap();
    assert!(started.elapsed() >= Duration::from_secs(1));
    assert_eq!(invoke_requests(&server).await.len(), 2);

    // Always throttled: three retries, then Throttled.
    let server = MockServer::start().await;
    Mock::given(path(INVOKE))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
        .expect(4)
        .mount(&server)
        .await;
    let error = client(&server, &log)
        .await
        .invoke(&create())
        .await
        .unwrap_err();
    assert!(matches!(error, InvokeError::Throttled(_)), "{error:?}");
}

#[tokio::test]
async fn the_concurrency_limit_is_retried_whatever_the_status() {
    let server = MockServer::start().await;
    Mock::given(path(INVOKE))
        .respond_with(
            ResponseTemplate::new(400)
                .insert_header("Retry-After", "0")
                .set_body_string(
                    "You have reached the maximum number of concurrent requests per tenant. \
                     Please wait and try again",
                ),
        )
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(path(INVOKE))
        .respond_with(ok(json!([])))
        .mount(&server)
        .await;
    let log = RunLog::default();
    client(&server, &log).await.invoke(&create()).await.unwrap();
    assert_eq!(invoke_requests(&server).await.len(), 2);
}

#[tokio::test]
async fn unavailable_is_retried_for_reads_but_never_for_writes() {
    let server = MockServer::start().await;
    Mock::given(path(INVOKE))
        .respond_with(ResponseTemplate::new(503).insert_header("Retry-After", "0"))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(path(INVOKE))
        .respond_with(ok(json!([])))
        .mount(&server)
        .await;
    let log = RunLog::default();
    client(&server, &log).await.invoke(&rules()).await.unwrap();
    assert_eq!(invoke_requests(&server).await.len(), 2);

    let server = MockServer::start().await;
    Mock::given(path(INVOKE))
        .respond_with(ResponseTemplate::new(503).insert_header("Retry-After", "0"))
        .expect(1)
        .mount(&server)
        .await;
    let error = client(&server, &log)
        .await
        .invoke(&create())
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            InvokeError::Rejected(ErrorDetail {
                status: Some(503),
                ..
            })
        ),
        "{error:?}"
    );
}

#[tokio::test]
async fn no_answer_is_tried_once_more_for_a_read_and_never_for_a_write() {
    let server = MockServer::start().await;
    // A port nothing listens on.
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = Url::parse(&format!("http://{}", closed.local_addr().unwrap())).unwrap();
    drop(closed);
    let log = RunLog::default();
    let client = InvokeClient::new(
        http_client().unwrap(),
        InvokeTarget {
            resource: Resource::Exchange,
            base: base.clone(),
            tenant_id: TENANT.into(),
            anchors: Anchor::exchange(ADMIN, TENANT),
        },
        Arc::new(tokens(&server, &log).await),
        HostRules::also(&base),
        log.clone(),
        CancellationToken::new(),
    );

    let started = Instant::now();
    let error = client.invoke(&rules()).await.unwrap_err();
    assert!(matches!(&error, InvokeError::Network(_)), "{error:?}");
    // Its message never names where it went.
    let port = base.port().unwrap().to_string();
    assert!(!error.message().contains(&port), "{error:?}");
    assert!(started.elapsed() >= Duration::from_secs(2));
    // Both tries with the admin's mailbox: no answer says nothing about it.
    assert_eq!(
        calls(&log)[0].attempts,
        [(Some("UPN"), None), (Some("UPN"), None)]
    );

    let error = client.invoke(&create()).await.unwrap_err();
    assert!(matches!(error, InvokeError::Network(_)), "{error:?}");
    assert_eq!(calls(&log)[1].attempts, [(Some("UPN"), None)]);
}

#[tokio::test]
async fn a_rejected_token_is_refreshed_once() {
    let server = MockServer::start().await;
    Mock::given(path(format!("/{TENANT}/oauth2/v2.0/token")))
        .respond_with(token_response("exchange-token-2"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path(INVOKE))
        .respond_with(ResponseTemplate::new(401).set_body_string("token expired"))
        .expect(2)
        .mount(&server)
        .await;
    let log = RunLog::default();
    let error = client(&server, &log)
        .await
        .invoke(&create())
        .await
        .unwrap_err();
    assert!(matches!(error, InvokeError::Unauthorized(_)), "{error:?}");
    let requests = invoke_requests(&server).await;
    assert_eq!(
        header_of(&requests[0], "authorization"),
        Some("Bearer exchange-token")
    );
    assert_eq!(
        header_of(&requests[1], "authorization"),
        Some("Bearer exchange-token-2")
    );
}

#[tokio::test]
async fn redirects_to_allowed_origins_are_followed_with_the_same_post() {
    for status in [302, 307] {
        let server = MockServer::start().await;
        let elsewhere = format!("{}/regional{INVOKE}", server.uri());
        Mock::given(path(format!("/regional{INVOKE}")))
            .respond_with(ok(json!([{ "Name": "A" }])))
            .mount(&server)
            .await;
        Mock::given(path(INVOKE))
            .respond_with(
                ResponseTemplate::new(status)
                    .insert_header("Location", elsewhere.as_str())
                    .insert_header("Set-Cookie", "affinity=abc123; Path=/; Secure"),
            )
            .mount(&server)
            .await;
        let log = RunLog::default();
        let output = client(&server, &log).await.invoke(&rules()).await.unwrap();
        assert_eq!(output.values.len(), 1, "{status}");
        let requests = invoke_requests(&server).await;
        assert_eq!(requests.len(), 2);
        let followed = &requests[1];
        assert_eq!(followed.method.as_str(), "POST");
        assert_eq!(followed.body, requests[0].body);
        assert_eq!(
            header_of(followed, "authorization"),
            Some("Bearer exchange-token")
        );
        assert_eq!(header_of(followed, "cookie"), Some("affinity=abc123"));
        // The run log has the host, never the cookie.
        let logged = calls(&log);
        assert_eq!(logged[0].redirects.len(), 1);
        assert!(!format!("{logged:?}").contains("abc123"));
    }
}

#[tokio::test]
async fn redirects_elsewhere_are_refused_before_anything_is_sent_there() {
    let server = MockServer::start().await;
    Mock::given(path(INVOKE))
        .respond_with(ResponseTemplate::new(302).insert_header(
            "Location",
            "https://evil.example/adminapi/beta/t/InvokeCommand",
        ))
        .expect(1)
        .mount(&server)
        .await;
    let log = RunLog::default();
    let error = client(&server, &log)
        .await
        .invoke(&rules())
        .await
        .unwrap_err();
    assert_eq!(error, InvokeError::Redirect("evil.example".into()));

    // An http Location to the mock server, under production rules: refused
    // too (the mock's http origin isn't allowed at all).
    let server = MockServer::start().await;
    Mock::given(path(INVOKE))
        .respond_with(ok(json!([])))
        .expect(0)
        .mount(&server)
        .await;
    let production = client_with(
        &server,
        &log,
        Resource::Exchange,
        HostRules::production(),
        CancellationToken::new(),
    )
    .await;
    let error = production.invoke(&rules()).await.unwrap_err();
    assert!(matches!(error, InvokeError::Redirect(_)), "{error:?}");
}

#[tokio::test]
async fn more_than_three_hops_are_refused() {
    let server = MockServer::start().await;
    let again = format!("{}{INVOKE}", server.uri());
    Mock::given(path(INVOKE))
        .respond_with(ResponseTemplate::new(307).insert_header("Location", again.as_str()))
        .expect(4)
        .mount(&server)
        .await;
    let log = RunLog::default();
    let error = client(&server, &log)
        .await
        .invoke(&rules())
        .await
        .unwrap_err();
    assert!(
        matches!(&error, InvokeError::Redirect(m) if m.contains("more than 3 redirects")),
        "{error:?}"
    );
}

#[tokio::test]
async fn reads_try_the_system_mailbox_and_pin_the_anchor_that_answers() {
    for failing in [404, 403] {
        let server = MockServer::start().await;
        Mock::given(header("X-AnchorMailbox", UPN))
            .respond_with(ResponseTemplate::new(failing).set_body_string("mailbox not found"))
            .mount(&server)
            .await;
        Mock::given(header("X-AnchorMailbox", system_mailbox().as_str()))
            .respond_with(ok(json!([])))
            .mount(&server)
            .await;
        let log = RunLog::default();
        let client = client(&server, &log).await;
        client.invoke(&rules()).await.unwrap();
        assert_eq!(client.pinned_anchor(), Some("SystemMailbox"));
        // The next call goes straight to the pinned anchor, writes too.
        client.invoke(&create()).await.unwrap();
        let anchors: Vec<String> = invoke_requests(&server)
            .await
            .iter()
            .map(|r| header_of(r, "x-anchormailbox").unwrap().to_owned())
            .collect();
        assert_eq!(
            anchors,
            [UPN.to_owned(), system_mailbox(), system_mailbox()]
        );
        // Each attempt is in the run log.
        let logged = calls(&log);
        assert_eq!(
            logged[0].attempts,
            [
                (Some("UPN"), Some(failing)),
                (Some("SystemMailbox"), Some(200))
            ]
        );
        assert_eq!(logged[0].anchor, Some("SystemMailbox"));
    }
}

#[tokio::test]
async fn writes_never_switch_anchors() {
    let server = MockServer::start().await;
    Mock::given(path(INVOKE))
        .respond_with(ResponseTemplate::new(404).set_body_string("mailbox not found"))
        .expect(1)
        .mount(&server)
        .await;
    let log = RunLog::default();
    let error = client(&server, &log)
        .await
        .invoke(&create())
        .await
        .unwrap_err();
    assert!(
        matches!(error, InvokeError::Rejected(_) | InvokeError::NotFound(_)),
        "{error:?}"
    );
}

#[tokio::test]
async fn refused_with_every_anchor_gives_the_first_anchors_answer() {
    let server = MockServer::start().await;
    Mock::given(header("X-AnchorMailbox", UPN))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "error": { "code": "Forbidden", "message": "User is not allowed to call Get-TransportRule" }
        })))
        .mount(&server)
        .await;
    Mock::given(header("X-AnchorMailbox", system_mailbox().as_str()))
        .respond_with(ResponseTemplate::new(403).set_body_string("Access denied"))
        .mount(&server)
        .await;
    let log = RunLog::default();
    let error = client(&server, &log)
        .await
        .invoke(&rules())
        .await
        .unwrap_err();
    match error {
        InvokeError::Forbidden(detail) => {
            assert_eq!(
                detail.message,
                "User is not allowed to call Get-TransportRule"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_read_ends_at_once_when_the_session_does() {
    let server = MockServer::start().await;
    Mock::given(path(INVOKE))
        .respond_with(ok(json!([])).set_delay(Duration::from_secs(30)))
        .mount(&server)
        .await;
    let log = RunLog::default();
    let cancel = CancellationToken::new();
    let hosts = HostRules::also(&Url::parse(&server.uri()).unwrap());
    let client = client_with(&server, &log, Resource::Exchange, hosts, cancel.clone()).await;
    let started = Instant::now();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel.cancel();
    });
    assert_eq!(
        client.invoke(&rules()).await.unwrap_err(),
        InvokeError::Cancelled
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn the_run_log_has_values_only_for_our_objects() {
    let server = MockServer::start().await;
    Mock::given(path(INVOKE))
        .respond_with(ok(json!([])))
        .mount(&server)
        .await;
    let log = RunLog::default();
    let client = client(&server, &log).await;
    client.invoke(&rules()).await.unwrap();
    client.invoke(&create()).await.unwrap();
    let logged = calls(&log);
    assert_eq!(logged[0].parameters, [("ResultSize".to_owned(), None)]);
    assert_eq!(
        logged[1].parameters,
        [
            (
                "Name".to_owned(),
                Some("'Cumulas Mail: Encrypt'".to_owned())
            ),
            (
                "RemoveHeader".to_owned(),
                Some("'x-cumulas-protect'".to_owned())
            ),
        ]
    );
    assert_eq!(logged[1].step, "Create “Cumulas Mail: Encrypt”");
    assert_eq!(logged[1].status, Some(200));
    assert_eq!(logged[1].cmdlet, "New-TransportRule");
    assert!(!format!("{logged:?}").contains("exchange-token"));
    assert_eq!(rules().kind, CallKind::Read);
}

/// Collects the formatted events of this crate's `tracing`.
struct Collect(Arc<StdMutex<Vec<String>>>);

struct Fields(String);

impl tracing::field::Visit for Fields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let _ = write!(self.0, "{}={value:?} ", field.name());
    }
}

impl tracing::Subscriber for Collect {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        metadata.target().starts_with("orgsetup") || metadata.target().starts_with("cumulas_core")
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut fields = Fields(format!("{} ", event.metadata().target()));
        event.record(&mut fields);
        self.0.lock().unwrap().push(fields.0);
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

#[tokio::test]
async fn tracing_never_carries_a_token_a_cookie_or_a_value() {
    let events = Arc::new(StdMutex::new(Vec::new()));
    let _guard = tracing::subscriber::set_default(Collect(events.clone()));
    // With one dispatcher registered, tracing works out the interest of a
    // callsite another test thread hits first from that thread's default
    // (none) alone; a second keeps it asking every registered one.
    let _second = tracing::Dispatch::new(Collect(Arc::default()));
    let server = MockServer::start().await;
    Mock::given(path(format!("/{TENANT}/oauth2/v2.0/token")))
        .respond_with(token_response("exchange-token-2"))
        .mount(&server)
        .await;
    Mock::given(path_regex("InvokeCommand$"))
        .and(header("authorization", "Bearer exchange-token"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    Mock::given(path_regex("InvokeCommand$"))
        .respond_with(
            ok(json!([{ "Name": "Cumulas Mail: Encrypt", "Comments": "secret-body" }]))
                .insert_header("Set-Cookie", "affinity=cookie-value"),
        )
        .mount(&server)
        .await;
    let log = RunLog::default();
    let client = client(&server, &log).await;
    client.invoke(&create()).await.unwrap();
    client.invoke(&rules()).await.unwrap();
    let events = events.lock().unwrap().join("\n");
    assert!(events.contains("InvokeCommand"), "{events}");
    assert!(events.contains("New-TransportRule"), "{events}");
    for secret in [
        "exchange-token",
        "refresh-0",
        "refresh-rotated",
        "cookie-value",
        "secret-body",
        "x-cumulas-protect",
        "Cumulas Mail: Encrypt",
    ] {
        assert!(!events.contains(secret), "{secret} in {events}");
    }
}
