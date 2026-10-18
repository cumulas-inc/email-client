//! Sensitivity labels and the label policy (Graph beta).

use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{graph_error, ok_json, setup};
use crate::error::ErrorCode;
use crate::model::{CatalogLabel, LabelPolicy};
use crate::providers::LabelsProvider;

const LABELS: &str = "/beta/me/security/informationProtection/sensitivityLabels";
const POLICY: &str = "/beta/me/security/informationProtection/labelPolicySettings";
const GENERAL: &str = "6c1b3b5e-0d4f-4b1e-9d3a-000000000001";
const ALL_EMPLOYEES: &str = "6c1b3b5e-0d4f-4b1e-9d3a-000000000002";
const PARENT: &str = "6c1b3b5e-0d4f-4b1e-9d3a-000000000003";

fn label(id: &str, name: &str) -> Value {
    json!({
        "id": id,
        "name": name,
        "description": "",
        "color": "",
        "sensitivity": 1,
        "tooltip": format!("{name} information"),
        "isActive": true,
        "isAppliable": true,
        "contentFormats": ["file", "email"],
        "hasProtection": false,
    })
}

async fn mount_labels(server: &MockServer, labels: Value) {
    Mock::given(method("GET"))
        .and(path(LABELS))
        .respond_with(ok_json(json!({ "value": labels })))
        .mount(server)
        .await;
}

async fn mount_policy(server: &MockServer, expanded: ResponseTemplate, plain: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path(POLICY))
        .and(query_param("$expand", "defaultLabel"))
        .respond_with(expanded)
        .expect(1)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(POLICY))
        .and(query_param_is_missing("$expand"))
        .respond_with(plain)
        .mount(server)
        .await;
}

#[tokio::test]
async fn only_labels_people_may_apply_to_email_are_kept() {
    let (server, provider) = setup().await;
    let mut child = label(ALL_EMPLOYEES, "AllEmployees");
    child["displayName"] = json!("All Employees");
    child["color"] = json!("#B35F26");
    child["hasProtection"] = json!(true);
    child["sensitivity"] = json!(3);
    child["parent"] =
        json!({ "id": PARENT, "name": "Confidential", "displayName": "Confidential" });
    let mut general = label(GENERAL, "General");
    general["color"] = json!("blue");
    general["contentFormats"] = json!(null);
    let mut parent = label(PARENT, "Confidential");
    parent["isAppliable"] = false.into();
    let mut inactive = label("6c1b3b5e-0d4f-4b1e-9d3a-000000000004", "Old");
    inactive["isActive"] = false.into();
    let mut disabled = label("6c1b3b5e-0d4f-4b1e-9d3a-000000000005", "Off");
    disabled["isEnabled"] = false.into();
    let mut files = label("6c1b3b5e-0d4f-4b1e-9d3a-000000000006", "Files only");
    files["contentFormats"] = json!(["file"]);
    let mut by_target = label("6c1b3b5e-0d4f-4b1e-9d3a-000000000007", "Mail");
    by_target["contentFormats"] = json!([]);
    by_target["applicableTo"] = json!("email,site");
    let mut not_a_guid = label("not-a-guid", "Odd");
    not_a_guid["id"] = json!("not-a-guid");
    let mut nameless = label("6c1b3b5e-0d4f-4b1e-9d3a-000000000008", " ");
    nameless["displayName"] = json!("");
    let mut unreadable = label("6c1b3b5e-0d4f-4b1e-9d3a-000000000009", "Odd");
    unreadable["sensitivity"] = json!("high");
    mount_labels(
        &server,
        json!([
            general, parent, child, inactive, disabled, files, by_target, not_a_guid, nameless,
            unreadable
        ]),
    )
    .await;
    mount_policy(
        &server,
        ok_json(json!({
            "id": "policy",
            "isMandatory": true,
            "isDowngradeJustificationRequired": true,
            "moreInfoUrl": "https://contoso.com/labels",
            "defaultLabel": { "id": ALL_EMPLOYEES.to_uppercase(), "name": "AllEmployees" },
        })),
        ResponseTemplate::new(500),
    )
    .await;
    let catalog = provider.label_catalog().await.unwrap();
    assert_eq!(
        catalog.labels,
        [
            CatalogLabel {
                id: GENERAL.into(),
                name: "General".into(),
                parent_name: None,
                tooltip: Some("General information".into()),
                color: None,
                has_protection: false,
                sensitivity: Some(1),
            },
            CatalogLabel {
                id: ALL_EMPLOYEES.into(),
                name: "All Employees".into(),
                parent_name: Some("Confidential".into()),
                tooltip: Some("AllEmployees information".into()),
                color: Some("#b35f26".into()),
                has_protection: true,
                sensitivity: Some(3),
            },
            CatalogLabel {
                id: "6c1b3b5e-0d4f-4b1e-9d3a-000000000007".into(),
                name: "Mail".into(),
                parent_name: None,
                tooltip: Some("Mail information".into()),
                color: None,
                has_protection: false,
                sensitivity: Some(1),
            },
        ]
    );
    assert_eq!(catalog.labels[1].title(), "Confidential › All Employees");
    assert_eq!(
        catalog.policy,
        LabelPolicy {
            required: true,
            default_label_id: Some(ALL_EMPLOYEES.into()),
            justify_downgrade: true,
            more_info_url: Some("https://contoso.com/labels".into()),
        }
    );
}

#[tokio::test]
async fn without_an_expandable_default_the_default_label_says_so_itself() {
    let (server, provider) = setup().await;
    let mut general = label(GENERAL, "General");
    general["isDefault"] = true.into();
    mount_labels(&server, json!([label(ALL_EMPLOYEES, "All"), general])).await;
    mount_policy(
        &server,
        graph_error(
            400,
            "BadRequest",
            "Could not find a property named 'defaultLabel'.",
        ),
        ok_json(json!({
            "isMandatory": false,
            "isDowngradeJustificationRequired": false,
            "moreInfoUrl": "http://contoso.com/labels",
        })),
    )
    .await;
    let policy = provider.label_catalog().await.unwrap().policy;
    assert_eq!(
        policy,
        LabelPolicy {
            required: false,
            default_label_id: Some(GENERAL.into()),
            justify_downgrade: false,
            // https only.
            more_info_url: None,
        }
    );
}

#[tokio::test]
async fn a_policy_microsoft_365_doesnt_have_is_no_policy() {
    for (expanded, plain) in [(404, 500), (400, 400)] {
        let (server, provider) = setup().await;
        mount_labels(&server, json!([label(GENERAL, "General")])).await;
        mount_policy(
            &server,
            graph_error(expanded, "x", "x"),
            graph_error(plain, "x", "x"),
        )
        .await;
        let catalog = provider.label_catalog().await.unwrap();
        assert_eq!(catalog.labels.len(), 1);
        assert_eq!(catalog.policy, LabelPolicy::default());
    }
    // Any other failure fails the listing.
    let (server, provider) = setup().await;
    mount_labels(&server, json!([label(GENERAL, "General")])).await;
    mount_policy(
        &server,
        graph_error(500, "InternalServerError", "Something went wrong."),
        ResponseTemplate::new(500),
    )
    .await;
    let error = provider.label_catalog().await.unwrap_err();
    assert_eq!(error.message, "Something went wrong.");
}

#[tokio::test]
async fn every_page_is_read_and_a_refusal_keeps_its_status() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path(LABELS))
        .and(query_param_is_missing("$skiptoken"))
        .respond_with(ok_json(json!({
            "value": [label(GENERAL, "General")],
            "@odata.nextLink": format!("{}{LABELS}?$skiptoken=2", server.uri()),
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(LABELS))
        .and(query_param("$skiptoken", "2"))
        .respond_with(ok_json(
            json!({ "value": [label(ALL_EMPLOYEES, "All Employees")] }),
        ))
        .mount(&server)
        .await;
    mount_policy(&server, ok_json(json!({})), ResponseTemplate::new(500)).await;
    let catalog = provider.label_catalog().await.unwrap();
    assert_eq!(catalog.labels.len(), 2);
    assert_eq!(catalog.policy, LabelPolicy::default());

    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path(LABELS))
        .respond_with(graph_error(403, "Forbidden", "Access denied."))
        .mount(&server)
        .await;
    let error = provider.label_catalog().await.unwrap_err();
    assert_eq!(error.code, ErrorCode::Provider);
    assert_eq!(error.http_status(), Some(403));
}
