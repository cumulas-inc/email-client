//! The work account's own licenses (`/me/licenseDetails`): whether they
//! include automatic labeling, a hint for organization setup.

use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer};

use super::{authorized, graph_error, ok_json, setup};
use crate::error::ErrorCode;
use crate::providers::LabelsProvider;
use crate::providers::graph::{LicenseHint, MIP_PREMIUM_PLAN_ID, MIP_PREMIUM_PLAN_NAME};

const LICENSES: &str = "/me/licenseDetails";

fn plan(id: &str, name: &str, status: &str) -> Value {
    json!({
        "servicePlanId": id,
        "servicePlanName": name,
        "provisioningStatus": status,
        "appliesTo": "User",
    })
}

/// Information Protection for Office 365 - Standard, which E3 has.
fn standard() -> Value {
    plan(
        "5136a095-5cf0-4aff-bec3-e84448b38ea5",
        "MIP_S_CLP1",
        "Success",
    )
}

fn premium(status: &str) -> Value {
    plan(MIP_PREMIUM_PLAN_ID, MIP_PREMIUM_PLAN_NAME, status)
}

fn sku(part: &str, plans: Vec<Value>) -> Value {
    json!({
        "id": format!("{part}-id"),
        "skuId": "00000000-0000-0000-0000-000000000000",
        "skuPartNumber": part,
        "servicePlans": plans,
    })
}

async fn hint_for(skus: Vec<Value>) -> LicenseHint {
    let (server, provider) = setup().await;
    mount(&server, skus).await;
    provider.license_hint().await.unwrap().unwrap()
}

async fn mount(server: &MockServer, skus: Vec<Value>) {
    Mock::given(method("GET"))
        .and(path(LICENSES))
        .and(query_param("$select", "skuPartNumber,servicePlans"))
        .and(authorized)
        .respond_with(ok_json(json!({ "value": skus })))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn e5_includes_automatic_labeling() {
    let hint = hint_for(vec![sku("SPE_E5", vec![standard(), premium("Success")])]).await;
    assert_eq!(
        hint,
        LicenseHint {
            auto_labeling: true,
            from: vec!["SPE_E5".into()],
            pending: Vec::new(),
            all: vec!["SPE_E5".into()],
        }
    );
    assert_eq!(hint.provisioning(), None);
    assert_eq!(hint.summary(), "SPE_E5 · automatic labeling: included");
}

#[tokio::test]
async fn e3_and_business_premium_dont() {
    let e3 = hint_for(vec![sku("SPE_E3", vec![standard()])]).await;
    assert!(!e3.auto_labeling && e3.from.is_empty() && e3.pending.is_empty());
    assert_eq!(e3.summary(), "SPE_E3 · automatic labeling: not included");

    let business = hint_for(vec![sku(
        "SPB",
        vec![plan(
            "8c098270-9dd4-4350-9b30-ba4703f3b36b",
            "ADALLOM_S_O365",
            "Success",
        )],
    )])
    .await;
    assert!(!business.auto_labeling);
    assert_eq!(business.all, ["SPB"]);
}

#[tokio::test]
async fn an_add_on_to_e3_does() {
    let hint = hint_for(vec![
        sku("SPE_E3", vec![standard()]),
        // Matched by the plan's name when its ID is spelled otherwise.
        sku(
            "INFORMATION_PROTECTION_COMPLIANCE",
            vec![plan("not-the-usual-id", "mip_s_clp2", "Success")],
        ),
    ])
    .await;
    assert!(hint.auto_labeling);
    assert_eq!(hint.from, ["INFORMATION_PROTECTION_COMPLIANCE"]);
    assert_eq!(hint.all, ["SPE_E3", "INFORMATION_PROTECTION_COMPLIANCE"]);
}

#[tokio::test]
async fn a_plan_microsoft_hasnt_turned_on_doesnt_count() {
    let pending = hint_for(vec![sku("SPE_E5", vec![premium("PendingActivation")])]).await;
    assert!(!pending.auto_labeling && pending.from.is_empty());
    assert_eq!(
        pending.provisioning(),
        Some(&("SPE_E5".to_owned(), "PendingActivation".to_owned()))
    );
    assert_eq!(
        pending.summary(),
        "SPE_E5 · automatic labeling: pending (SPE_E5 PendingActivation)"
    );

    // Turned off: not included, and not on its way either.
    let disabled = hint_for(vec![sku("SPE_E5", vec![premium("Disabled")])]).await;
    assert!(!disabled.auto_labeling);
    assert_eq!(disabled.provisioning(), None);
    assert_eq!(
        disabled.summary(),
        "SPE_E5 · automatic labeling: not included (SPE_E5 Disabled)"
    );
}

#[tokio::test]
async fn no_licenses_and_refusals() {
    let none = hint_for(Vec::new()).await;
    assert_eq!(none, LicenseHint::default());
    assert_eq!(
        none.summary(),
        "no licenses · automatic labeling: not included"
    );

    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path(LICENSES))
        .respond_with(graph_error(
            403,
            "Authorization_RequestDenied",
            "Insufficient privileges",
        ))
        .mount(&server)
        .await;
    let error = provider.license_hint().await.unwrap_err();
    assert_eq!(error.code, ErrorCode::Provider);
    assert_eq!(error.http_status(), Some(403));
}
