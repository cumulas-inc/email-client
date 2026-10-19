//! Organization setup for administrators ([`crate::orgsetup`]): the setup
//! app's session, what the setup dialog reads and changes, and "Copy
//! details for IT" for people's checks.
//!
//! Not built yet: signing in, reading, planning, applying and the script
//! answer `NotConfigured`, and the dialog falls back on the guide.

use std::sync::Arc;

use super::protection::work_tenant;
use super::{BrowserOpener, CoreService};
use crate::auth::admin_setup::{setup_admin_consent_url, setup_not_available};
use crate::error::{Error, ErrorCode, Result};
use crate::model::{
    OrgSetupAdmin, OrgSetupChoices, OrgSetupInfo, OrgSetupPlan, OrgSetupResult, OrgSetupScript,
    OrgSetupScriptRequest, OrgSetupStatus,
};
use crate::orgsetup::{self, Session};
use crate::protection::{Settle, org_setup_changed_key};

/// Why an account has no organization setup.
const WORK_ACCOUNTS_ONLY: &str =
    "Only Microsoft 365 work and school accounts have an organization to set up.";

impl CoreService {
    /// The organization (tenant ID) of a work or school account; others
    /// have none to set up.
    fn setup_tenant(&self, account_id: &str) -> Result<String> {
        let account = self.store.get_account(account_id)?;
        work_tenant(&account)
            .map(str::to_owned)
            .ok_or_else(|| Error::new(ErrorCode::Unsupported, WORK_ACCOUNTS_ONLY))
    }

    /// The live setup session, if it's for the organization `tenant`.
    fn org_setup_session(&self, tenant: &str) -> Option<Arc<Session>> {
        self.org_setup
            .lock()
            .unwrap()
            .as_ref()
            .filter(|s| s.tenant_id.eq_ignore_ascii_case(tenant) && !s.is_cancelled())
            .cloned()
    }

    /// Whether setup can run from here for the account's organization, who
    /// is signed in for it, and when this computer last changed it. No
    /// network, and no wait for whatever setup is doing.
    pub fn org_setup_info(&self, account_id: &str) -> Result<OrgSetupInfo> {
        let tenant = self.setup_tenant(account_id)?;
        let endpoints = self.connector.admin_setup_endpoints();
        let settle = self.settle(&tenant);
        Ok(OrgSetupInfo {
            automatic: endpoints.is_some(),
            signed_in: self.org_setup_session(&tenant).map(|s| s.admin.clone()),
            approval_url: endpoints.map(|e| setup_admin_consent_url(&e.client_id, Some(&tenant))),
            rules_changed_at: settle.rules_changed_at,
            policies_changed_at: settle.policies_changed_at,
            tenant_id: tenant,
        })
    }

    /// Signs an administrator of the account's organization in with the
    /// setup app, in the browser.
    pub async fn org_setup_sign_in(
        &self,
        _open_browser: &BrowserOpener,
        account_id: &str,
    ) -> Result<OrgSetupAdmin> {
        self.setup_tenant(account_id)?;
        Err(setup_not_available())
    }

    /// Abandons the setup sign-in waiting for the browser.
    pub fn cancel_org_setup_sign_in(&self) {
        if let Some(cancel) = self.pending_org_setup_sign_in.lock().unwrap().take() {
            let _ = cancel.send(());
        }
    }

    /// Reads everything setup needs from Microsoft 365, and what it would
    /// change.
    pub async fn read_org_setup(&self, account_id: &str) -> Result<OrgSetupStatus> {
        self.setup_tenant(account_id)?;
        Err(setup_not_available())
    }

    /// The changes `choices` make, from the last read.
    pub async fn plan_org_setup(
        &self,
        account_id: &str,
        _choices: &OrgSetupChoices,
    ) -> Result<OrgSetupPlan> {
        self.setup_tenant(account_id)?;
        Err(setup_not_available())
    }

    /// Reads again, and makes the changes only if they're still the ones
    /// `confirmed`.
    pub async fn apply_org_setup(
        &self,
        account_id: &str,
        _choices: &OrgSetupChoices,
        _confirmed: &[String],
    ) -> Result<OrgSetupResult> {
        self.setup_tenant(account_id)?;
        Err(setup_not_available())
    }

    /// The PowerShell script and the Microsoft Purview portal steps, filled
    /// in for the account's organization; no sign-in needed.
    pub fn org_setup_script(
        &self,
        account_id: &str,
        _request: &OrgSetupScriptRequest,
    ) -> Result<OrgSetupScript> {
        self.setup_tenant(account_id)?;
        Err(setup_not_available())
    }

    /// "Copy setup details": the sign-in attempts and the calls setup made.
    /// Works in every state, also after the session ended.
    pub fn org_setup_details(&self) -> String {
        orgsetup::details_text(&self.org_setup_log)
    }

    /// Forgets the setup sign-in at once (its tokens go with it); the run
    /// log stays.
    pub fn end_org_setup(&self) {
        if let Some(session) = self.org_setup.lock().unwrap().take() {
            session.cancel();
        }
    }

    /// "Copy details for IT": what the account's last checks found.
    pub fn protection_check_details(&self, account_id: &str) -> Result<String> {
        self.check_details_text(account_id)
    }

    /// When setup last changed the organization `tenant` from this
    /// computer; neither when that can't be read.
    pub(super) fn settle(&self, tenant: &str) -> Settle {
        self.store
            .app_value(&org_setup_changed_key(tenant))
            .ok()
            .flatten()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests;
