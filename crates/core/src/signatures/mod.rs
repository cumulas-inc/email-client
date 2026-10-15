//! Company signatures from Cumulas Signatures, for Microsoft 365 work and
//! school accounts.
//!
//! Nothing to set up: for each such account the app asks Microsoft for a
//! token for the signature service's API (`api://<host>/<client
//! id>/access_as_user`, see [`config`]). In an organization that doesn't use
//! Cumulas Signatures, Microsoft refuses (the API isn't in the tenant) and
//! nothing reaches the service. Otherwise the app asks the service for the
//! person's signature with that token ([`client`]); the answer is the
//! signature, that the organization's mail server adds it on send, or that
//! there's none for them. Personal Microsoft accounts and IMAP accounts
//! never take part.
//!
//! [`OrganizationSignatures`] is the check as the service sees it;
//! `service::signatures` keeps the answers, when to ask again, and what a
//! message starts with.

pub mod client;
pub mod config;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use url::Url;

use crate::auth::{ApiTokenError, ApiTokens};
use client::{Fetched, ServerAnswer, SignatureClient};

/// The tenant of personal Microsoft accounts.
pub const CONSUMER_TENANT: &str = "9188040d-6c67-4c5b-b112-36a304b66dad";

/// Whether an account in `tenant_id` can have a company signature: a work
/// or school account, whose tenant is a GUID other than the personal
/// accounts' one.
pub fn is_organization_tenant(tenant_id: &str) -> bool {
    config::is_guid(tenant_id) && !tenant_id.eq_ignore_ascii_case(CONSUMER_TENANT)
}

/// What a check found out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    /// The organization's signature, which the app adds. Relative links in
    /// it point to `base`; `etag` names this answer.
    Signature {
        html: String,
        on_replies: bool,
        base: Option<Url>,
        etag: Option<String>,
    },
    /// The organization's mail server adds the signature on send, to
    /// replies and forwards too when `on_replies`.
    AddedOnSend {
        on_replies: bool,
        etag: Option<String>,
    },
    /// The organization doesn't use the service, or has no signature for
    /// the person; `etag` names the service's answer when it gave one.
    NotUsed { etag: Option<String> },
    /// The organization uses the service but hasn't allowed this app.
    NotAllowed,
    /// The answer named by the ETag sent still holds.
    Unchanged,
    /// No answer this time (no network, the service busy or unavailable,
    /// Microsoft's sign-in wanting the person): try again later, not before
    /// `retry_after` when the service said. Never says anything about the
    /// organization.
    Failed {
        retry_after: Option<Duration>,
        reason: String,
    },
}

/// Asks an account's organization for its signature.
#[async_trait]
pub trait OrganizationSignatures: Send + Sync {
    /// The API's scope, which names the service and its app registration:
    /// an answer kept from another is not used.
    fn scope(&self) -> &str;

    /// Asks now; `etag` is the last answer's.
    async fn check(&self, etag: Option<&str>) -> CheckOutcome;
}

/// The check over the network: a token from Microsoft, then the service.
pub struct HttpOrganizationSignatures {
    client: Arc<SignatureClient>,
    tokens: Arc<dyn ApiTokens>,
    scope: String,
}

impl HttpOrganizationSignatures {
    pub fn new(client: Arc<SignatureClient>, tokens: Arc<dyn ApiTokens>, scope: String) -> Self {
        Self {
            client,
            tokens,
            scope,
        }
    }
}

#[async_trait]
impl OrganizationSignatures for HttpOrganizationSignatures {
    fn scope(&self) -> &str {
        &self.scope
    }

    async fn check(&self, etag: Option<&str>) -> CheckOutcome {
        let mut fresh = false;
        loop {
            let token = match self.tokens.api_token(fresh).await {
                Ok(token) => token,
                Err(ApiTokenError::NotInTenant) => return CheckOutcome::NotUsed { etag: None },
                Err(ApiTokenError::NotAllowed) => return CheckOutcome::NotAllowed,
                Err(ApiTokenError::Failed(e)) => {
                    return CheckOutcome::Failed {
                        retry_after: None,
                        reason: e.message,
                    };
                }
            };
            return match self.client.fetch(&token, etag).await {
                // Once with a new token: the cached one may have been
                // revoked, or the clocks disagree.
                Fetched::Unauthorized if !fresh => {
                    fresh = true;
                    continue;
                }
                Fetched::Unauthorized => CheckOutcome::Failed {
                    retry_after: None,
                    reason: "The signature service didn't accept the sign-in.".into(),
                },
                Fetched::Answer { answer, etag } => match answer {
                    ServerAnswer::Signature { html, on_replies } => CheckOutcome::Signature {
                        html,
                        on_replies,
                        base: Some(self.client.base().clone()),
                        etag,
                    },
                    ServerAnswer::AddedOnSend { on_replies } => {
                        CheckOutcome::AddedOnSend { on_replies, etag }
                    }
                    ServerAnswer::None => CheckOutcome::NotUsed { etag },
                },
                Fetched::NotModified => CheckOutcome::Unchanged,
                Fetched::NotAllowed => CheckOutcome::NotAllowed,
                Fetched::NotUsed => CheckOutcome::NotUsed { etag: None },
                Fetched::Failed {
                    retry_after,
                    reason,
                } => CheckOutcome::Failed {
                    retry_after,
                    reason,
                },
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use wiremock::matchers::{header, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::error::Error;

    /// Hands out `token-1`, `token-2`, …: a new one when `fresh`.
    #[derive(Default)]
    struct CountingTokens {
        issued: Mutex<Vec<bool>>,
        refusal: Mutex<Option<ApiTokenError>>,
    }

    #[async_trait]
    impl ApiTokens for CountingTokens {
        async fn api_token(&self, fresh: bool) -> Result<String, ApiTokenError> {
            if let Some(refusal) = self.refusal.lock().unwrap().clone() {
                return Err(refusal);
            }
            let mut issued = self.issued.lock().unwrap();
            issued.push(fresh);
            Ok(format!(
                "token-{}",
                issued.iter().filter(|f| **f).count() + 1
            ))
        }
    }

    fn checker(server: &MockServer, tokens: Arc<CountingTokens>) -> HttpOrganizationSignatures {
        HttpOrganizationSignatures::new(
            Arc::new(SignatureClient::new(&server.uri()).unwrap()),
            tokens,
            "api://x/y/access_as_user".into(),
        )
    }

    #[tokio::test]
    async fn a_refused_token_is_replaced_once() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(header("authorization", "Bearer token-1"))
            .respond_with(
                ResponseTemplate::new(401)
                    .set_body_json(serde_json::json!({ "error": "x", "code": "token-invalid" })),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(header("authorization", "Bearer token-2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "version": 1, "source": "app", "applyToReplies": false,
                "signature": { "html": "<p>Ada</p>", "text": "Ada" },
            })))
            .mount(&server)
            .await;
        let tokens = Arc::new(CountingTokens::default());
        let outcome = checker(&server, tokens.clone()).check(None).await;
        assert_eq!(
            outcome,
            CheckOutcome::Signature {
                html: "<p>Ada</p>".into(),
                on_replies: false,
                base: Some(Url::parse(&format!("{}/", server.uri())).unwrap()),
                etag: None,
            }
        );
        assert_eq!(*tokens.issued.lock().unwrap(), [false, true]);

        // Refused twice: tried again later, not a third time now.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401))
            .expect(2)
            .mount(&server)
            .await;
        let outcome = checker(&server, Arc::new(CountingTokens::default()))
            .check(Some("\"e\""))
            .await;
        assert!(
            matches!(outcome, CheckOutcome::Failed { .. }),
            "{outcome:?}"
        );
    }

    #[tokio::test]
    async fn microsofts_refusals_never_reach_the_service() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        let cases = [
            (
                ApiTokenError::NotInTenant,
                CheckOutcome::NotUsed { etag: None },
            ),
            (ApiTokenError::NotAllowed, CheckOutcome::NotAllowed),
            (
                ApiTokenError::Failed(Error::network("offline")),
                CheckOutcome::Failed {
                    retry_after: None,
                    reason: "offline".into(),
                },
            ),
        ];
        for (refusal, expected) in cases {
            let tokens = Arc::new(CountingTokens::default());
            *tokens.refusal.lock().unwrap() = Some(refusal);
            assert_eq!(checker(&server, tokens).check(None).await, expected);
        }
    }

    #[tokio::test]
    async fn the_services_answers_become_outcomes() {
        let cases = [
            (
                ResponseTemplate::new(200)
                    .insert_header("etag", "\"r\"")
                    .set_body_json(
                        serde_json::json!({ "version": 1, "source": "relay", "signature": null }),
                    ),
                CheckOutcome::AddedOnSend {
                    on_replies: false,
                    etag: Some("\"r\"".into()),
                },
            ),
            (
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "version": 1, "source": "relay", "applyToReplies": true, "signature": null,
                })),
                CheckOutcome::AddedOnSend {
                    on_replies: true,
                    etag: None,
                },
            ),
            (
                ResponseTemplate::new(200)
                    .insert_header("etag", "\"n\"")
                    .set_body_json(
                        serde_json::json!({ "version": 1, "source": "none", "signature": null }),
                    ),
                CheckOutcome::NotUsed {
                    etag: Some("\"n\"".into()),
                },
            ),
            (ResponseTemplate::new(304), CheckOutcome::Unchanged),
            (
                ResponseTemplate::new(403)
                    .set_body_json(serde_json::json!({ "error": "x", "code": "app-not-allowed" })),
                CheckOutcome::NotAllowed,
            ),
            (
                ResponseTemplate::new(404)
                    .set_body_json(serde_json::json!({ "error": "x", "code": "not-managed" })),
                CheckOutcome::NotUsed { etag: None },
            ),
        ];
        for (response, expected) in cases {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(response)
                .mount(&server)
                .await;
            let outcome = checker(&server, Arc::new(CountingTokens::default()))
                .check(Some("\"r\""))
                .await;
            assert_eq!(outcome, expected);
        }

        // A 404 that isn't the service's says nothing of the organization.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404).set_body_string("Not Found"))
            .mount(&server)
            .await;
        let outcome = checker(&server, Arc::new(CountingTokens::default()))
            .check(Some("\"r\""))
            .await;
        assert!(
            matches!(
                outcome,
                CheckOutcome::Failed {
                    retry_after: None,
                    ..
                }
            ),
            "{outcome:?}"
        );
    }

    #[test]
    fn only_work_and_school_tenants_take_part() {
        assert!(is_organization_tenant(
            "11111111-2222-3333-4444-555555555555"
        ));
        assert!(!is_organization_tenant(CONSUMER_TENANT));
        assert!(!is_organization_tenant(
            "9188040D-6C67-4C5B-B112-36A304B66DAD"
        ));
        assert!(!is_organization_tenant("consumers"));
        assert!(!is_organization_tenant(""));
    }
}
