//! Tests of organization setup: the InvokeCommand client, discovery and
//! tokens against mock servers, the views, status, plan and script on
//! fixtures, and apply against a modeled organization ([`tenant`]).

mod apply;
mod discovery;
mod invoke;
mod objects;
mod plan;
mod runlog;
mod script;
mod status;
pub(crate) mod tenant;
mod tokens;

use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};
use url::Url;
use wiremock::{MockServer, ResponseTemplate};

use super::objects::{
    AcceptedDomainView, AutoPolicyView, AutoRuleView, InboundConnectorView, IrmView, LabelView,
    OutboundConnectorView, RmsTemplateView, TransportRuleView,
};
use super::status::{ComplianceSnapshot, Snapshot};
use super::{HostRules, RunLog, Session};
use crate::auth::admin_setup::{AdminSetupConfig, AdminSignIn, AdminTokens, Resource};
use crate::auth::microsoft::WallClock;
use crate::model::{CatalogLabel, LabelCatalog, LabelPolicy};
use crate::orgsetup::status::{Part, SetupContext};
use crate::protection::Settle;
use crate::providers::graph::LicenseHint;
use crate::service::AdminEndpoints;

pub(crate) const TENANT: &str = "85f67998-e79a-4173-95c2-90b0e6dc231d";
pub(crate) const CLIENT: &str = "b4f17c55-b9f6-4a84-958f-afe3842da327";
pub(crate) const ADMIN: &str = "admin@contoso.com";
pub(crate) const ADMIN_OID: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
pub(crate) const EMAIL: &str = "adele@contoso.com";
pub(crate) const CONFIDENTIAL: &str = "6c1b0f3e-1111-4a8b-9c2d-000000000001";
pub(crate) const GENERAL: &str = "6c1b0f3e-1111-4a8b-9c2d-000000000002";
pub(crate) const SECRET: &str = "6c1b0f3e-1111-4a8b-9c2d-000000000003";

/// A change's ID without the digest of its calls.
pub(crate) fn base_id(id: &str) -> &str {
    id.split_once('#').map_or(id, |(base, _)| base)
}

/// A token endpoint's answer.
pub(crate) fn token_response(access: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "token_type": "Bearer",
        "expires_in": 3599,
        "access_token": access,
        "refresh_token": "refresh-rotated",
    }))
}

/// A wall clock tests move.
pub(crate) fn clock() -> (Arc<StdMutex<SystemTime>>, WallClock) {
    let now = Arc::new(StdMutex::new(SystemTime::now()));
    let read = now.clone();
    (now, Arc::new(move || *read.lock().unwrap()))
}

pub(crate) fn config(server: &MockServer) -> AdminSetupConfig {
    AdminSetupConfig {
        client_id: CLIENT.into(),
        tenant_id: TENANT.into(),
        authority_host: server.uri(),
    }
}

/// Tokens whose Exchange token is primed and whose refreshes go to the
/// mock server.
pub(crate) async fn tokens(server: &MockServer, log: &RunLog) -> AdminTokens {
    let tokens = AdminTokens::new(
        &config(server),
        "refresh-0",
        log.clone(),
        Arc::new(SystemTime::now),
    );
    tokens
        .prime(
            Resource::Exchange,
            "exchange-token",
            Duration::from_secs(3600),
        )
        .await;
    tokens
}

pub(crate) fn endpoints(server: &MockServer) -> AdminEndpoints {
    let base = Url::parse(&server.uri()).unwrap();
    AdminEndpoints {
        client_id: CLIENT.into(),
        authority_host: server.uri(),
        exchange: base.clone(),
        compliance_discovery: base.clone(),
        hosts: HostRules::also(&base),
    }
}

/// A signed-in session against the mock server.
pub(crate) async fn session(server: &MockServer) -> Session {
    let log = RunLog::default();
    let sign_in = AdminSignIn {
        username: ADMIN.into(),
        display_name: "Admin".into(),
        tenant_id: TENANT.into(),
        object_id: ADMIN_OID.into(),
        tokens: tokens(server, &log).await,
        granted: "https://outlook.office365.com/Exchange.Manage".into(),
    };
    let mut session = Session::new(
        sign_in,
        "acct",
        &endpoints(server),
        log,
        Arc::new(SystemTime::now),
    )
    .unwrap();
    session.relist_delay = Duration::from_millis(1);
    session
}

pub(crate) fn catalog_label(id: &str, name: &str, encrypts: bool) -> CatalogLabel {
    CatalogLabel {
        id: id.into(),
        name: name.into(),
        parent_name: None,
        tooltip: None,
        color: None,
        has_protection: encrypts,
        sensitivity: None,
    }
}

/// The work account's label list: Confidential (encrypts), General,
/// Highly Confidential.
pub(crate) fn catalog() -> LabelCatalog {
    LabelCatalog {
        labels: vec![
            catalog_label(CONFIDENTIAL, "Confidential", true),
            catalog_label(GENERAL, "General", false),
            catalog_label(SECRET, "Highly Confidential", true),
        ],
        policy: LabelPolicy::default(),
    }
}

/// A license hint that includes automatic labeling.
pub(crate) fn e5() -> LicenseHint {
    LicenseHint {
        auto_labeling: true,
        from: vec!["SPE_E5".into()],
        pending: Vec::new(),
        all: vec!["SPE_E5".into()],
    }
}

/// What the service would know for adele@contoso.com.
pub(crate) fn ctx() -> SetupContext {
    SetupContext {
        account_email: EMAIL.into(),
        tenant_id: TENANT.into(),
        needs_permission: Vec::new(),
        refused_features: Vec::new(),
        catalog: Some(catalog()),
        signatures_manage_url: Some("https://signature.cumulas.co/manage".into()),
        settle: Settle::default(),
        now: chrono::Utc::now(),
        licensing: Part::Read(e5()),
    }
}

/// A fixture file of `fixtures/`.
pub(crate) fn fixture(name: &str) -> Value {
    let path = format!(
        "{}/src/orgsetup/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).unwrap()
}

/// What a read of `tenant` would find, without a mock server.
pub(crate) fn snapshot_of(tenant: &tenant::FakeTenant) -> Snapshot {
    tenant.with(|s| {
        let rules = s
            .rules
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let mut r = r.clone();
                r["Priority"] = json!(i);
                TransportRuleView::read(&r).unwrap()
            })
            .collect();
        Snapshot {
            read_at: chrono::Utc::now(),
            accepted_domains: Part::Read(
                s.accepted
                    .iter()
                    .filter_map(AcceptedDomainView::read)
                    .collect(),
            ),
            irm: Part::Read(IrmView::read(&s.irm)),
            templates: Part::Read(
                s.templates
                    .iter()
                    .filter_map(RmsTemplateView::read)
                    .collect(),
            ),
            rules: Part::Read(rules),
            inbound: Part::Read(
                s.inbound
                    .iter()
                    .filter_map(InboundConnectorView::read)
                    .collect(),
            ),
            outbound: Part::Read(
                s.outbound
                    .iter()
                    .filter_map(OutboundConnectorView::read)
                    .collect(),
            ),
            compliance: Part::Read(ComplianceSnapshot {
                labels: s.labels.iter().filter_map(LabelView::read).collect(),
                policies: s.policies.iter().filter_map(AutoPolicyView::read).collect(),
                rules: s
                    .policy_rules
                    .iter()
                    .filter_map(AutoRuleView::read)
                    .collect(),
            }),
            licensing: Part::Read(e5()),
        }
    })
}
