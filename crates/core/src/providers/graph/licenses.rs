//! A work account's own licenses (`/me/licenseDetails`, which `User.Read`
//! covers): whether they include automatic labeling, which sending with a
//! label needs. A hint about one person, not the organization: the setup
//! app has no permission to read the organization's plans.

use serde::Deserialize;

use super::GraphProvider;
use super::types::null_default;
use crate::error::Result;

/// Information Protection for Office 365 - Premium: the service plan
/// automatic labeling needs.
pub const MIP_PREMIUM_PLAN_ID: &str = "efb0351d-3b08-4503-993d-383af8de41e3";
pub const MIP_PREMIUM_PLAN_NAME: &str = "MIP_S_CLP2";

const LICENSES_PATH: &str = "/me/licenseDetails";

/// What a person's licenses say about automatic labeling.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LicenseHint {
    /// Whether this person's licenses include the plan, provisioned.
    pub auto_labeling: bool,
    /// skuPartNumber of the SKUs that include it, provisioned ("SPE_E5",
    /// "INFORMATION_PROTECTION_COMPLIANCE").
    pub from: Vec<String>,
    /// SKUs that include it in a state other than Success (Error,
    /// PendingInput, PendingActivation, PendingProvisioning, Disabled), and
    /// that state.
    pub pending: Vec<(String, String)>,
    /// Every skuPartNumber, for the run log.
    pub all: Vec<String>,
}

impl LicenseHint {
    /// A license that includes automatic labeling and that Microsoft 365 is
    /// still turning on (`PendingInput`, `PendingActivation`,
    /// `PendingProvisioning`): its SKU and that state. One in `Error` or
    /// `Disabled` doesn't count.
    pub fn provisioning(&self) -> Option<&(String, String)> {
        self.pending
            .iter()
            .find(|(_, state)| state.to_ascii_lowercase().starts_with("pending"))
    }

    /// For the run log: every SKU, and whether automatic labeling is
    /// included ("SPE_E3 · automatic labeling: not included").
    pub fn summary(&self) -> String {
        let all = if self.all.is_empty() {
            "no licenses".to_owned()
        } else {
            self.all.join(", ")
        };
        let states: Vec<String> = self
            .pending
            .iter()
            .map(|(sku, state)| format!("{sku} {state}"))
            .collect();
        let included = if self.auto_labeling {
            "included".to_owned()
        } else if self.provisioning().is_some() {
            format!("pending ({})", states.join(", "))
        } else if states.is_empty() {
            "not included".to_owned()
        } else {
            format!("not included ({})", states.join(", "))
        };
        format!("{all} · automatic labeling: {included}")
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LicenseDetails {
    sku_part_number: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    service_plans: Vec<ServicePlan>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServicePlan {
    service_plan_id: Option<String>,
    service_plan_name: Option<String>,
    provisioning_status: Option<String>,
}

impl ServicePlan {
    /// Whether it's the plan automatic labeling needs.
    fn is_premium(&self) -> bool {
        self.service_plan_id
            .as_deref()
            .is_some_and(|id| id.trim().eq_ignore_ascii_case(MIP_PREMIUM_PLAN_ID))
            || self
                .service_plan_name
                .as_deref()
                .is_some_and(|name| name.trim().eq_ignore_ascii_case(MIP_PREMIUM_PLAN_NAME))
    }
}

/// The hint from the person's license details: a plan counts only when
/// Microsoft 365 has provisioned it (`Success`).
fn license_hint(details: Vec<LicenseDetails>) -> LicenseHint {
    let mut hint = LicenseHint::default();
    for license in details {
        let sku = license
            .sku_part_number
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "(unnamed)".into());
        for plan in license.service_plans.iter().filter(|p| p.is_premium()) {
            let status = plan
                .provisioning_status
                .as_deref()
                .map(str::trim)
                .unwrap_or("Unknown");
            if status.eq_ignore_ascii_case("Success") {
                hint.auto_labeling = true;
                if !hint.from.contains(&sku) {
                    hint.from.push(sku.clone());
                }
            } else {
                hint.pending.push((sku.clone(), status.to_owned()));
            }
        }
        if !hint.all.contains(&sku) {
            hint.all.push(sku);
        }
    }
    hint
}

impl GraphProvider {
    /// The person's own licenses, through the account's Graph client (its
    /// retries and errors).
    pub(super) async fn read_license_hint(&self) -> Result<LicenseHint> {
        let url = self
            .client
            .url_with_query(LICENSES_PATH, &[("$select", "skuPartNumber,servicePlans")]);
        let details: Vec<LicenseDetails> = self.client.get_all(&url, &[]).await?;
        Ok(license_hint(details))
    }
}
