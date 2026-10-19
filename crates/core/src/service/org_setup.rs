//! Organization setup for administrators ([`crate::orgsetup`]): the setup
//! app's session, what the setup dialog reads and changes, and "Copy
//! details for IT" for people's checks.
//!
//! The session lives in memory only, at most one at a time; it ends when
//! the dialog closes or unmounts, at the next sign-in, after 60 minutes
//! unused by either clock, and when the app quits. Its run log stays until
//! the next sign-in attempt, so "Copy setup details" works in every state.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::SystemTime;

use chrono::Utc;

use super::protection::work_tenant;
use super::{BrowserOpener, CoreService};
use crate::auth::admin_setup::{other_tenant, setup_admin_consent_url, setup_not_available};
use crate::auth::microsoft::WallClock;
use crate::error::{Error, ErrorCode, Result};
use crate::model::{
    Account, DataKind, OrgSetupAdmin, OrgSetupChoices, OrgSetupInfo, OrgSetupPlan, OrgSetupResult,
    OrgSetupScript, OrgSetupScriptRequest, OrgSetupStatus,
};
use crate::orgsetup::invoke::ErrorDetail;
use crate::orgsetup::objects::LabelView;
use crate::orgsetup::runlog::{RunHeader, SignInRecord};
use crate::orgsetup::script::{ScriptInput, ScriptLabel, script, template_or_default};
use crate::orgsetup::status::{
    FailureKind, Part, ReadFailure, SessionInfo, SetupContext, read_snapshot, status,
};
use crate::orgsetup::{self, IDLE_TICK, Session, apply, signed_out, wanted};
use crate::protection::{Settle, org_setup_changed_key};
use crate::providers::graph::LicenseHint;
use crate::signatures::config::{SignatureService, is_guid};

/// Why an account has no organization setup.
const WORK_ACCOUNTS_ONLY: &str =
    "Only Microsoft 365 work and school accounts have an organization to set up.";

/// What the run log notes when Microsoft 365 ended the sign-in.
const REFUSED_NOTE: &str = "Signed out: Microsoft 365 didn't accept the sign-in any more.";

/// The wall clock sessions read (tests move theirs).
fn system_clock() -> WallClock {
    Arc::new(SystemTime::now)
}

impl CoreService {
    /// The organization (tenant ID) of a work or school account; others
    /// have none to set up.
    fn setup_tenant(&self, account_id: &str) -> Result<String> {
        let account = self.store.get_account(account_id)?;
        work_tenant(&account)
            .map(str::to_owned)
            .ok_or_else(|| Error::new(ErrorCode::Unsupported, WORK_ACCOUNTS_ONLY))
    }

    /// The live setup session, if it's for the organization `tenant`. One
    /// unused for too long is signed out here too (the watcher may not
    /// have woken since a resume).
    fn org_setup_session(&self, tenant: &str) -> Option<Arc<Session>> {
        let session = self
            .org_setup
            .lock()
            .unwrap()
            .as_ref()
            .filter(|s| s.tenant_id.eq_ignore_ascii_case(tenant) && !s.is_cancelled())
            .cloned()?;
        if session.is_idle() {
            orgsetup::sign_out(&self.org_setup, &session, orgsetup::IDLE_NOTE);
            return None;
        }
        Some(session)
    }

    /// The live session for the account's organization, or `AuthRequired`
    /// (`setupSignedOut`).
    fn live_session(&self, account_id: &str) -> Result<(Account, String, Arc<Session>)> {
        let tenant = self.setup_tenant(account_id)?;
        if self.connector.admin_setup_endpoints().is_none() {
            return Err(setup_not_available());
        }
        let account = self.store.get_account(account_id)?;
        let session = self.org_setup_session(&tenant).ok_or_else(signed_out)?;
        session.touch();
        Ok((account, tenant, session))
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
    /// setup app, in the browser. Ends any session and pending sign-in
    /// first; every attempt goes to the run log, failed or not.
    pub async fn org_setup_sign_in(
        &self,
        open_browser: &BrowserOpener,
        account_id: &str,
    ) -> Result<OrgSetupAdmin> {
        self.org_setup_sign_in_with(open_browser, account_id, system_clock(), IDLE_TICK)
            .await
    }

    /// The same, reading the wall clock with `clock` and looking for an
    /// unused session every `idle_tick` (tests move the one and shorten the
    /// other).
    async fn org_setup_sign_in_with(
        &self,
        open_browser: &BrowserOpener,
        account_id: &str,
        clock: WallClock,
        idle_tick: std::time::Duration,
    ) -> Result<OrgSetupAdmin> {
        let tenant = self.setup_tenant(account_id)?;
        let account = self.store.get_account(account_id)?;
        let Some(endpoints) = self.connector.admin_setup_endpoints() else {
            return Err(setup_not_available());
        };
        self.cancel_org_setup_sign_in();
        self.end_org_setup();
        // Closing the dialog from here on ends this sign-in too, also once
        // it can no longer be canceled.
        let generation = self.org_setup_generation.load(Ordering::SeqCst);
        let log = Arc::clone(&self.org_setup_log);
        log.start(RunHeader {
            tenant_id: Some(tenant.clone()),
            client_id: Some(endpoints.client_id.clone()),
            admin: None,
        });
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        *self.pending_org_setup_sign_in.lock().unwrap() = Some(cancel);
        let signed_in = tokio::select! {
            result = self.connector.admin_setup_sign_in(open_browser, &tenant, &log) => result,
            _ = cancelled => {
                let error = Error::new(ErrorCode::SignInCancelled, "Sign-in was canceled.");
                log.sign_in(SignInRecord::new(endpoints.authority_host.clone(), "").failed(&error));
                Err(error)
            }
        };
        self.pending_org_setup_sign_in.lock().unwrap().take();
        let sign_in = signed_in.map_err(|e| match e.details.as_ref().map(|d| &d["reason"]) {
            Some(reason) if reason == "otherTenant" => other_tenant(
                account
                    .email
                    .rsplit_once('@')
                    .map_or("your organization", |(_, d)| d),
            ),
            _ => e,
        })?;
        if !sign_in.tenant_id.eq_ignore_ascii_case(&tenant) {
            return Err(other_tenant(
                account
                    .email
                    .rsplit_once('@')
                    .map_or("your organization", |(_, d)| d),
            ));
        }
        log.set_admin(&sign_in.username);
        log.note(format!("Signed in; granted \"{}\"", sign_in.granted));
        let session = Arc::new(Session::new(
            sign_in,
            account_id,
            &endpoints,
            (*log).clone(),
            clock,
        )?);
        {
            let mut slot = self.org_setup.lock().unwrap();
            if self.org_setup_generation.load(Ordering::SeqCst) != generation {
                // The dialog closed as the sign-in finished: its tokens go
                // with the session now.
                drop(slot);
                session.cancel();
                log.note("Signed out: the setup window closed as the sign-in finished.");
                return Err(Error::new(
                    ErrorCode::SignInCancelled,
                    "Sign-in was canceled.",
                ));
            }
            *slot = Some(Arc::clone(&session));
        }
        orgsetup::watch_idle(Arc::clone(&self.org_setup), &session, idle_tick);
        Ok(session.admin.clone())
    }

    /// Abandons the setup sign-in waiting for the browser, or finishing.
    pub fn cancel_org_setup_sign_in(&self) {
        self.org_setup_generation.fetch_add(1, Ordering::SeqCst);
        if let Some(cancel) = self.pending_org_setup_sign_in.lock().unwrap().take() {
            let _ = cancel.send(());
        }
    }

    /// What setup knows without asking Microsoft, and the work account's
    /// license hint when `licensing`.
    async fn setup_context(
        &self,
        account: &Account,
        tenant: &str,
        licensing: bool,
    ) -> Result<SetupContext> {
        let mut ctx = self.setup_context_now(account, tenant)?;
        if licensing {
            ctx.licensing = self.license_hint(account).await;
        }
        Ok(ctx)
    }

    fn setup_context_now(&self, account: &Account, tenant: &str) -> Result<SetupContext> {
        Ok(SetupContext {
            account_email: account.email.clone(),
            tenant_id: tenant.to_owned(),
            needs_permission: account.needs_permission.clone(),
            refused_features: account.refused_features.clone(),
            catalog: self.label_list(account)?,
            signatures_manage_url: SignatureService::configured()
                .map(|s| format!("{}/manage", s.url.trim_end_matches('/'))),
            settle: self.settle(tenant),
            now: Utc::now(),
            licensing: Part::Skipped,
        })
    }

    /// The work account's own licenses (call 12), through its provider: not
    /// the setup app's.
    async fn license_hint(&self, account: &Account) -> Part<LicenseHint> {
        let log = &self.org_setup_log;
        let from = &account.email;
        // The account's providers before gating: reading its licenses needs
        // only User.Read, which every account has.
        if let Err(e) = self.providers_for(&account.id).await {
            log.note(format!(
                "Licensing (from {from}): couldn't read ({})",
                e.message
            ));
            return failed_part(&e.message);
        }
        let labels = self
            .providers
            .lock()
            .await
            .get(&account.id)
            .and_then(|cached| cached.raw.labels.clone());
        let Some(labels) = labels else {
            return Part::Skipped;
        };
        match labels.license_hint().await {
            Ok(Some(hint)) => {
                log.note(format!("Licensing (from {from}): {}", hint.summary()));
                Part::Read(hint)
            }
            Ok(None) => Part::Skipped,
            Err(e) => {
                log.note(format!(
                    "Licensing (from {from}): couldn't read ({})",
                    e.message
                ));
                failed_part(&e.message)
            }
        }
    }

    /// Reads everything setup needs from Microsoft 365, and what it would
    /// change. Errors: `AuthRequired` (`setupSignedOut`) without a live
    /// session, or when it ends during the read.
    pub async fn read_org_setup(&self, account_id: &str) -> Result<OrgSetupStatus> {
        let (account, tenant, session) = self.live_session(account_id)?;
        let _calls = tokio::select! {
            calls = session.calls.lock() => calls,
            _ = session.cancelled() => return Err(signed_out()),
        };
        let ctx = self.setup_context(&account, &tenant, true).await?;
        let snapshot = read_snapshot(&session, &ctx)
            .await
            .inspect_err(|e| self.end_if_signed_out(&session, e))?;
        let status = status(&snapshot, &ctx, &session_info(&session));
        session.keep_snapshot(snapshot);
        session.touch();
        Ok(status)
    }

    /// The changes `choices` make, from the last read (no new one).
    pub async fn plan_org_setup(
        &self,
        account_id: &str,
        choices: &OrgSetupChoices,
    ) -> Result<OrgSetupPlan> {
        let (account, tenant, session) = self.live_session(account_id)?;
        let snapshot = session.snapshot().ok_or_else(|| {
            Error::invalid_input("Read your organization's setup first: select Read again.")
        })?;
        let mut ctx = self.setup_context(&account, &tenant, false).await?;
        ctx.licensing = snapshot.licensing.clone();
        let choices = wanted::validate(choices, Some(&snapshot), &ctx)?;
        Ok(orgsetup::plan::plan(&snapshot, &ctx, &choices))
    }

    /// Reads again, and makes the changes only if they're still the ones
    /// `confirmed`; records when it changed what, and tells open check rows.
    pub async fn apply_org_setup(
        &self,
        account_id: &str,
        choices: &OrgSetupChoices,
        confirmed: &[String],
    ) -> Result<OrgSetupResult> {
        let (account, tenant, session) = self.live_session(account_id)?;
        let _calls = tokio::select! {
            calls = session.calls.lock() => calls,
            _ = session.cancelled() => return Err(signed_out()),
        };
        let ctx = self.setup_context(&account, &tenant, true).await?;
        let outcome = apply::apply(&session, &ctx, &session_info(&session), choices, confirmed)
            .await
            .inspect_err(|e| self.end_if_signed_out(&session, e))?;
        if outcome.result.signed_out {
            // As the result says: sign in again to go on.
            orgsetup::sign_out(&self.org_setup, &session, REFUSED_NOTE);
        }
        if outcome.rules_changed || outcome.policies_changed {
            let settle = apply::settled(
                self.settle(&tenant),
                outcome.rules_changed,
                outcome.policies_changed,
            );
            self.record_settle(&tenant, &settle);
        }
        session.touch();
        self.notify(&[DataKind::Protection], None);
        Ok(outcome.result)
    }

    /// Ends `session` when `error` says its sign-in ended (Microsoft
    /// refused to renew it), so the next call doesn't try it again.
    fn end_if_signed_out(&self, session: &Arc<Session>, error: &Error) {
        if orgsetup::invoke::is_signed_out(error) {
            orgsetup::sign_out(&self.org_setup, session, REFUSED_NOTE);
        }
    }

    fn record_settle(&self, tenant: &str, settle: &Settle) {
        let saved = serde_json::to_string(settle)
            .map_err(Error::from)
            .and_then(|json| {
                self.store
                    .set_app_value(&org_setup_changed_key(tenant), &json)
            });
        if let Err(e) = saved {
            tracing::warn!("could not record when organization setup changed things: {e}");
        }
    }

    /// The PowerShell script and the Microsoft Purview portal steps, filled
    /// in for the account's organization; no sign-in needed. With a session
    /// that has read, labels and templates are checked against what it read.
    pub fn org_setup_script(
        &self,
        account_id: &str,
        request: &OrgSetupScriptRequest,
    ) -> Result<OrgSetupScript> {
        let tenant = self.setup_tenant(account_id)?;
        let account = self.store.get_account(account_id)?;
        let snapshot = self.org_setup_session(&tenant).and_then(|s| s.snapshot());
        let mut ctx = self.setup_context_now(&account, &tenant)?;
        if let Some(snapshot) = &snapshot {
            ctx.licensing = snapshot.licensing.clone();
        }
        let mut choices = request.choices.clone().unwrap_or_default();
        if !request.labels {
            choices.label_ids.clear();
        }
        if snapshot.is_none() {
            // Purview's list went with the sign-in: the labels chosen from it
            // that the account's own list doesn't have are left out (anything
            // that isn't a label ID is still refused).
            choices.label_ids.retain(|id| {
                let id = id.trim().to_ascii_lowercase();
                !is_guid(&id) || ctx.catalog.as_ref().is_some_and(|c| c.label(&id).is_some())
            });
        }
        let choices = wanted::validate(&choices, snapshot.as_deref(), &ctx)?;
        // With labels but none chosen (or none left): the account's list,
        // as before a sign-in.
        let ids: Vec<String> = if !request.labels {
            Vec::new()
        } else if !choices.label_ids.is_empty() {
            choices.label_ids.clone()
        } else {
            ctx.catalog
                .as_ref()
                .map(|c| c.labels.iter().map(|l| l.id.to_ascii_lowercase()).collect())
                .unwrap_or_default()
        };
        let purview = snapshot
            .as_ref()
            .and_then(|s| s.compliance.read())
            .map(|c| c.labels.clone())
            .unwrap_or_default();
        let labels = ids
            .into_iter()
            .filter(|id| is_guid(id))
            .filter_map(|id| {
                let listed = ctx.catalog.as_ref().and_then(|c| c.label(&id));
                let view = purview.iter().find(|l| l.is(&id)).cloned().or_else(|| {
                    listed.map(|label| LabelView {
                        guid: id.clone(),
                        immutable_id: None,
                        name: label.name.clone(),
                        display_name: None,
                        parent_id: None,
                        content_type: None,
                        raw: serde_json::Value::Null,
                    })
                })?;
                let title = listed.map_or_else(|| view.title(), |l| l.title());
                Some(ScriptLabel { id, title, view })
            })
            .collect();
        let input = ScriptInput {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            generated: Utc::now(),
            organization: ctx.organization().to_owned(),
            tenant_id: tenant,
            actions: request.actions,
            encrypt_template: template_or_default(choices.encrypt_template.as_deref(), true),
            dnf_template: template_or_default(choices.do_not_forward_template.as_deref(), false),
            labels,
        };
        Ok(script(&input))
    }

    /// "Copy setup details": the sign-in attempts and the calls setup made.
    /// Works in every state, also after the session ended.
    pub fn org_setup_details(&self) -> String {
        orgsetup::details_text(&self.org_setup_log)
    }

    /// Forgets the setup sign-in at once (its tokens go with it once a call
    /// in flight returns); the run log stays.
    pub fn end_org_setup(&self) {
        self.org_setup_generation.fetch_add(1, Ordering::SeqCst);
        let session = self.org_setup.lock().unwrap().take();
        if let Some(session) = session {
            orgsetup::sign_out(&self.org_setup, &session, "Signed out.");
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

/// Who a session is for, for the rows about the setup app.
fn session_info(session: &Session) -> SessionInfo {
    SessionInfo {
        admin: session.admin.clone(),
        client_id: session.client_id.clone(),
        admin_object_id: session.admin_object_id.clone(),
    }
}

fn failed_part(message: &str) -> Part<LicenseHint> {
    Part::Failed(ReadFailure {
        kind: FailureKind::Other,
        detail: ErrorDetail {
            message: message.trim().to_owned(),
            ..ErrorDetail::default()
        },
    })
}

#[cfg(test)]
mod tests;
