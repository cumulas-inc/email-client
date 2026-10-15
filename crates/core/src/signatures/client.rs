//! The client of Cumulas Signatures' API for mail apps:
//! `GET <service>/apps/signature`, with a token for the service's API that
//! names the person to their organization's service. The answer is their
//! signature, or that the organization adds it on send, or that it has
//! none for them.

use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::header::{ACCEPT, CONTENT_LENGTH, ETAG, HeaderMap, IF_NONE_MATCH, RETRY_AFTER};
use reqwest::{Response, StatusCode};
use serde::Deserialize;
use url::Url;

use crate::html::text_to_html;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// The most of an answer that's read; a signature is far smaller.
pub(crate) const MAX_BODY_BYTES: usize = 1024 * 1024;
/// The most a `Retry-After` makes the app wait.
pub(crate) const MAX_RETRY_AFTER: Duration = Duration::from_secs(6 * 3600);
/// The longest ETag kept to send back.
const MAX_ETAG_LEN: usize = 256;

/// What the organization's service answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerAnswer {
    /// The person's signature, which the app adds (`source: "app"`).
    Signature { html: String, on_replies: bool },
    /// The organization's mail server adds it on send (`source: "relay"`).
    AddedOnSend,
    /// No signature for this person right now (`source: "none"`).
    None,
}

/// The outcome of one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fetched {
    /// 200, with the answer's ETag to send back next time.
    Answer {
        answer: ServerAnswer,
        etag: Option<String>,
    },
    /// 304: the answer whose ETag was sent still holds.
    NotModified,
    /// 401: the token was refused.
    Unauthorized,
    /// 403 `app-not-allowed`: the organization hasn't allowed this app.
    NotAllowed,
    /// 404: the service doesn't serve the person's organization, or isn't
    /// set up for mail apps.
    NotUsed,
    /// Anything else: try again later, not before `retry_after` when the
    /// service said.
    Failed {
        retry_after: Option<Duration>,
        reason: String,
    },
}

/// The signature service over HTTPS. One serves every account.
pub struct SignatureClient {
    http: reqwest::Client,
    base: Url,
    endpoint: String,
}

impl SignatureClient {
    /// A client for the service at `base` (see
    /// [`super::config::signature_url`]); `None` when that isn't a URL.
    pub fn new(base: &str) -> Option<Self> {
        let base = base.trim_end_matches('/');
        let http = reqwest::Client::builder()
            // The token goes with the request: never anywhere else.
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .user_agent(concat!("CumulasMail/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();
        Some(Self {
            http,
            base: Url::parse(&format!("{base}/")).ok()?,
            endpoint: format!("{base}/apps/signature"),
        })
    }

    /// Where relative links in the service's signatures point.
    pub fn base(&self) -> &Url {
        &self.base
    }

    /// Asks for the person's signature with `token`; `etag` (from the last
    /// answer) lets the service say it hasn't changed.
    pub async fn fetch(&self, token: &str, etag: Option<&str>) -> Fetched {
        let mut request = self
            .http
            .get(&self.endpoint)
            .bearer_auth(token)
            .header(ACCEPT, "application/json");
        if let Some(etag) = etag.filter(|e| usable_etag(e)) {
            request = request.header(IF_NONE_MATCH, etag);
        }
        let mut response = match request.send().await {
            Ok(response) => response,
            Err(e) => {
                return failed(format!(
                    "Could not reach the signature service: {}",
                    e.without_url()
                ));
            }
        };
        let status = response.status();
        if status == StatusCode::NOT_MODIFIED {
            return Fetched::NotModified;
        }
        let retry_after = retry_after(response.headers(), Utc::now());
        let etag = response
            .headers()
            .get(ETAG)
            .and_then(|v| v.to_str().ok())
            .filter(|e| usable_etag(e))
            .map(str::to_owned);
        let body = match read_capped(&mut response).await {
            Ok(body) => body,
            Err(reason) => {
                return Fetched::Failed {
                    retry_after,
                    reason,
                };
            }
        };
        match status {
            StatusCode::OK => match parse_answer(&body) {
                Ok(answer) => Fetched::Answer { answer, etag },
                Err(reason) => failed(reason),
            },
            StatusCode::UNAUTHORIZED => Fetched::Unauthorized,
            StatusCode::FORBIDDEN if error_code(&body) == "app-not-allowed" => Fetched::NotAllowed,
            StatusCode::NOT_FOUND => Fetched::NotUsed,
            _ => Fetched::Failed {
                retry_after,
                reason: match error_code(&body) {
                    code if code.is_empty() => {
                        format!("The signature service answered HTTP {}.", status.as_u16())
                    }
                    code => format!(
                        "The signature service answered HTTP {} ({code}).",
                        status.as_u16()
                    ),
                },
            },
        }
    }
}

fn failed(reason: String) -> Fetched {
    Fetched::Failed {
        retry_after: None,
        reason,
    }
}

/// An ETag that can go back in a header: short, visible ASCII.
fn usable_etag(etag: &str) -> bool {
    !etag.is_empty()
        && etag.len() <= MAX_ETAG_LEN
        && etag.bytes().all(|b| (0x20..0x7f).contains(&b))
}

/// Reads the body, stopping past [`MAX_BODY_BYTES`].
async fn read_capped(response: &mut Response) -> Result<Vec<u8>, String> {
    let too_large = || "The signature service sent more than a signature.".to_owned();
    let announced = response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if announced.is_some_and(|length| length > MAX_BODY_BYTES as u64) {
        return Err(too_large());
    }
    let mut body = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if body.len() + chunk.len() > MAX_BODY_BYTES {
                    return Err(too_large());
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => return Ok(body),
            Err(e) => {
                return Err(format!(
                    "Could not read the signature service's answer: {}",
                    e.without_url()
                ));
            }
        }
    }
}

/// `Retry-After` in seconds or as a date, capped at [`MAX_RETRY_AFTER`].
pub(crate) fn retry_after(headers: &HeaderMap, now: DateTime<Utc>) -> Option<Duration> {
    let value = headers.get(RETRY_AFTER)?.to_str().ok()?.trim();
    let wait = match value.parse::<u64>() {
        Ok(seconds) => Duration::from_secs(seconds),
        Err(_) => {
            let at = DateTime::parse_from_rfc2822(value).ok()?;
            (at.with_timezone(&Utc) - now).to_std().unwrap_or_default()
        }
    };
    Some(wait.min(MAX_RETRY_AFTER))
}

/// The `code` of an error answer (`{ "error", "code" }`), or "".
fn error_code(body: &[u8]) -> String {
    #[derive(Deserialize)]
    struct ErrorBody {
        #[serde(default)]
        code: String,
    }
    serde_json::from_slice::<ErrorBody>(body)
        .map(|b| b.code)
        .unwrap_or_default()
}

/// Reads a 200 answer. Fields this app doesn't know are ignored; a source
/// it doesn't know is an answer it can't act on.
pub(crate) fn parse_answer(body: &[u8]) -> Result<ServerAnswer, String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Answer {
        #[serde(default)]
        source: String,
        #[serde(default)]
        apply_to_replies: bool,
        #[serde(default)]
        signature: Option<Signature>,
    }
    #[derive(Deserialize)]
    struct Signature {
        #[serde(default)]
        html: String,
        #[serde(default)]
        text: String,
    }
    let answer: Answer = serde_json::from_slice(body)
        .map_err(|_| "The signature service sent an answer this app can't read.".to_owned())?;
    match answer.source.as_str() {
        "app" => {
            let signature = answer.signature.ok_or_else(|| {
                "The signature service said there's a signature but sent none.".to_owned()
            })?;
            let html = if !signature.html.trim().is_empty() {
                signature.html
            } else if !signature.text.trim().is_empty() {
                text_to_html(&signature.text)
            } else {
                return Err("The signature service sent an empty signature.".into());
            };
            Ok(ServerAnswer::Signature {
                html,
                on_replies: answer.apply_to_replies,
            })
        }
        "relay" => Ok(ServerAnswer::AddedOnSend),
        "none" => Ok(ServerAnswer::None),
        other => {
            tracing::warn!("the signature service answered with an unknown source ({other:.40})");
            Err("The signature service sent an answer this app doesn't know.".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use reqwest::header::HeaderValue;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn client(server: &MockServer) -> SignatureClient {
        SignatureClient::new(&server.uri()).unwrap()
    }

    fn signature_answer() -> serde_json::Value {
        serde_json::json!({
            "version": 1,
            "source": "app",
            "applyToReplies": true,
            "signature": { "html": "<p>Ada</p>", "text": "Ada" },
            "somethingNew": [1, 2],
        })
    }

    #[tokio::test]
    async fn asks_with_the_token_and_reads_the_signature() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/apps/signature"))
            .and(header("authorization", "Bearer token-1"))
            .and(header("accept", "application/json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("etag", "\"abc\"")
                    .set_body_json(signature_answer()),
            )
            .expect(1)
            .mount(&server)
            .await;
        let fetched = client(&server).fetch("token-1", None).await;
        assert_eq!(
            fetched,
            Fetched::Answer {
                answer: ServerAnswer::Signature {
                    html: "<p>Ada</p>".into(),
                    on_replies: true,
                },
                etag: Some("\"abc\"".into()),
            }
        );
        let request = &server.received_requests().await.unwrap()[0];
        assert!(request.url.query().is_none());
        assert!(request.body.is_empty());
        assert!(
            request.headers["user-agent"]
                .to_str()
                .unwrap()
                .starts_with("CumulasMail/")
        );
        assert!(!request.headers.contains_key("if-none-match"));
    }

    #[tokio::test]
    async fn sends_the_etag_back_and_reads_not_modified() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(header("if-none-match", "\"abc\""))
            .respond_with(ResponseTemplate::new(304))
            .mount(&server)
            .await;
        assert_eq!(
            client(&server).fetch("t", Some("\"abc\"")).await,
            Fetched::NotModified
        );
    }

    #[tokio::test]
    async fn maps_each_answer() {
        let error = |status: u16, code: &str| {
            ResponseTemplate::new(status)
                .set_body_json(serde_json::json!({ "error": "Nope.", "code": code }))
        };
        let cases = [
            (
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "version": 1, "source": "relay", "applyToReplies": false, "signature": null,
                })),
                Fetched::Answer {
                    answer: ServerAnswer::AddedOnSend,
                    etag: None,
                },
            ),
            (
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "version": 1, "source": "none", "applyToReplies": false, "signature": null,
                })),
                Fetched::Answer {
                    answer: ServerAnswer::None,
                    etag: None,
                },
            ),
            (error(401, "token-invalid"), Fetched::Unauthorized),
            (error(401, "token-required"), Fetched::Unauthorized),
            (error(403, "app-not-allowed"), Fetched::NotAllowed),
            (error(404, "not-managed"), Fetched::NotUsed),
            (error(404, "not-set-up"), Fetched::NotUsed),
        ];
        for (response, expected) in cases {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(response)
                .mount(&server)
                .await;
            assert_eq!(client(&server).fetch("t", None).await, expected);
        }
    }

    #[tokio::test]
    async fn busy_and_broken_answers_are_tried_again_later() {
        let cases = [
            (
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "120")
                    .set_body_json(
                        serde_json::json!({ "error": "Slow down.", "code": "rate-limited" }),
                    ),
                Some(Duration::from_secs(120)),
            ),
            (
                ResponseTemplate::new(503)
                    .insert_header("retry-after", "86400")
                    .set_body_json(serde_json::json!({ "error": "Later.", "code": "unavailable" })),
                Some(MAX_RETRY_AFTER),
            ),
            (ResponseTemplate::new(500), None),
            // A 403 that isn't the service's refusal (a proxy's, say).
            (
                ResponseTemplate::new(403).set_body_string("Forbidden"),
                None,
            ),
            // Redirects aren't followed: the token stays with the service.
            (
                ResponseTemplate::new(302).insert_header("location", "https://example.com/"),
                None,
            ),
            (
                ResponseTemplate::new(200).set_body_string("<html>not json</html>"),
                None,
            ),
            (
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "version": 2, "source": "elsewhere" })),
                None,
            ),
            (
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "source": "app", "signature": null })),
                None,
            ),
            (
                ResponseTemplate::new(200).set_body_string("x".repeat(MAX_BODY_BYTES + 1)),
                None,
            ),
        ];
        for (response, expected) in cases {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(response)
                .mount(&server)
                .await;
            match client(&server).fetch("t", None).await {
                Fetched::Failed { retry_after, .. } => assert_eq!(retry_after, expected),
                other => panic!("{other:?}"),
            }
        }

        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let client = SignatureClient::new(&format!("http://{}", closed.local_addr().unwrap()));
        drop(closed);
        assert!(matches!(
            client.unwrap().fetch("t", None).await,
            Fetched::Failed {
                retry_after: None,
                ..
            }
        ));
    }

    #[test]
    fn retry_after_reads_seconds_and_dates() {
        let now = DateTime::parse_from_rfc3339("2026-10-15T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let headers = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(RETRY_AFTER, HeaderValue::from_str(value).unwrap());
            headers
        };
        assert_eq!(
            retry_after(&headers("90"), now),
            Some(Duration::from_secs(90))
        );
        assert_eq!(
            retry_after(&headers("Thu, 15 Oct 2026 10:05:00 GMT"), now),
            Some(Duration::from_secs(300))
        );
        assert_eq!(
            retry_after(&headers("Thu, 15 Oct 2026 09:00:00 GMT"), now),
            Some(Duration::ZERO)
        );
        assert_eq!(retry_after(&headers("soon"), now), None);
        assert_eq!(retry_after(&HeaderMap::new(), now), None);
    }

    #[test]
    fn a_text_only_signature_becomes_html() {
        let answer = parse_answer(
            br#"{"source":"app","applyToReplies":false,"signature":{"html":"","text":"Ada <CEO>"}}"#,
        )
        .unwrap();
        assert_eq!(
            answer,
            ServerAnswer::Signature {
                html: "<div style=\"white-space: pre-wrap\">Ada &lt;CEO&gt;</div>".into(),
                on_replies: false,
            }
        );
    }
}
