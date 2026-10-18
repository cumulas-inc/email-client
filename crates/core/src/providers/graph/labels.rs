//! Sensitivity labels on Microsoft Graph (beta, Information Protection):
//! the labels a work or school account may apply to email, and its label
//! policy. Read leniently: a beta API's shape can change, and a label that
//! doesn't read is left out rather than failing the list.

use async_trait::async_trait;
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::Value;

use super::GraphProvider;
use super::client::Failure;
use super::types::null_default;
use crate::error::Result;
use crate::model::{CatalogLabel, LabelCatalog, LabelPolicy};
use crate::providers::LabelsProvider;
use crate::signatures::config::is_guid;

const LABELS_PATH: &str = "/me/security/informationProtection/sensitivityLabels";
const POLICY_PATH: &str = "/me/security/informationProtection/labelPolicySettings";
/// Most labels kept.
const MAX_LABELS: usize = 200;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphLabel {
    id: Option<String>,
    name: Option<String>,
    display_name: Option<String>,
    color: Option<String>,
    sensitivity: Option<i32>,
    tooltip: Option<String>,
    is_active: Option<bool>,
    is_appliable: Option<bool>,
    is_enabled: Option<bool>,
    is_default: Option<bool>,
    has_protection: Option<bool>,
    #[serde(default, deserialize_with = "null_default")]
    content_formats: Vec<Value>,
    /// Comma-separated targets, such as `email,file`.
    applicable_to: Option<Value>,
    parent: Option<GraphParent>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphParent {
    name: Option<String>,
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphPolicy {
    is_mandatory: Option<bool>,
    is_downgrade_justification_required: Option<bool>,
    more_info_url: Option<String>,
    default_label: Option<GraphDefault>,
}

#[derive(Debug, Deserialize)]
struct GraphDefault {
    id: Option<String>,
}

fn text(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

/// Whether a value names email: a string with `email` among its
/// comma-separated targets, or a list with it.
fn names_email(value: &Value) -> bool {
    match value {
        Value::String(s) => s.split(',').any(|t| t.trim().eq_ignore_ascii_case("email")),
        Value::Array(items) => items.iter().any(names_email),
        _ => false,
    }
}

fn is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Array(items) => items.is_empty(),
        _ => false,
    }
}

/// `#rrggbb`, lowercase; anything else is no color.
fn color(value: Option<String>) -> Option<String> {
    let value = value?.trim().to_ascii_lowercase();
    (value.len() == 7
        && value.starts_with('#')
        && value[1..].bytes().all(|b| b.is_ascii_hexdigit()))
    .then_some(value)
}

/// The label as the catalog keeps it, if the person may apply it to email:
/// active, enabled and appliable (not a parent of others), for email or
/// for anything.
fn catalog_label(label: GraphLabel) -> Option<(CatalogLabel, bool)> {
    let id = text(label.id)
        .filter(|id| is_guid(id))?
        .to_ascii_lowercase();
    if label.is_active == Some(false)
        || label.is_appliable == Some(false)
        || label.is_enabled == Some(false)
    {
        return None;
    }
    let formats = Value::Array(label.content_formats);
    let applicable = label.applicable_to.unwrap_or(Value::Null);
    let for_email = names_email(&formats)
        || names_email(&applicable)
        || (is_empty(&formats) && is_empty(&applicable));
    if !for_email {
        return None;
    }
    let name = text(label.display_name).or_else(|| text(label.name))?;
    let parent_name = label
        .parent
        .and_then(|p| text(p.display_name).or_else(|| text(p.name)));
    Some((
        CatalogLabel {
            id,
            name,
            parent_name,
            tooltip: text(label.tooltip),
            color: color(label.color),
            has_protection: label.has_protection.unwrap_or(false),
            sensitivity: label.sensitivity,
        },
        label.is_default.unwrap_or(false),
    ))
}

impl GraphProvider {
    /// The label policy; `None` when Microsoft 365 has none to give.
    async fn label_policy(&self) -> Result<Option<GraphPolicy>> {
        // The default label is a relationship, only there when expanded.
        let expanded = self
            .client
            .beta_url(&format!("{POLICY_PATH}?$expand=defaultLabel"));
        let plain = self.client.beta_url(POLICY_PATH);
        let mut answer = self.client.get_json::<GraphPolicy>(&expanded, &[]).await;
        if let Err(Failure::Graph {
            status: StatusCode::BAD_REQUEST,
            ..
        }) = &answer
        {
            answer = self.client.get_json(&plain, &[]).await;
        }
        match answer {
            Ok(policy) => Ok(Some(policy)),
            Err(Failure::Graph { status, .. })
                if matches!(status, StatusCode::NOT_FOUND | StatusCode::BAD_REQUEST) =>
            {
                tracing::info!("Microsoft 365 gave no label policy ({status}); using none");
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }
}

#[async_trait]
impl LabelsProvider for GraphProvider {
    async fn label_catalog(&self) -> Result<LabelCatalog> {
        let listed: Vec<Value> = self
            .client
            .get_all(&self.client.beta_url(LABELS_PATH), &[])
            .await?;
        let mut defaults = Vec::new();
        let mut labels = Vec::new();
        for value in listed {
            let label = match serde_json::from_value::<GraphLabel>(value) {
                Ok(label) => label,
                Err(e) => {
                    tracing::debug!("skipping a sensitivity label that doesn't read: {e}");
                    continue;
                }
            };
            if let Some((label, is_default)) = catalog_label(label) {
                if labels.iter().any(|l: &CatalogLabel| l.id == label.id) {
                    continue;
                }
                if is_default {
                    defaults.push(label.id.clone());
                }
                labels.push(label);
            }
            if labels.len() == MAX_LABELS {
                break;
            }
        }
        let policy = match self.label_policy().await? {
            Some(policy) => {
                let listed = |id: &str| labels.iter().any(|l| l.id.eq_ignore_ascii_case(id));
                let expanded_default = policy
                    .default_label
                    .and_then(|d| text(d.id))
                    .map(|id| id.to_ascii_lowercase())
                    .filter(|id| listed(id));
                LabelPolicy {
                    required: policy.is_mandatory.unwrap_or(false),
                    default_label_id: expanded_default.or_else(|| defaults.first().cloned()),
                    justify_downgrade: policy.is_downgrade_justification_required.unwrap_or(false),
                    more_info_url: text(policy.more_info_url)
                        .filter(|u| url::Url::parse(u).is_ok_and(|u| u.scheme() == "https")),
                }
            }
            None => LabelPolicy::default(),
        };
        Ok(LabelCatalog { labels, policy })
    }
}
