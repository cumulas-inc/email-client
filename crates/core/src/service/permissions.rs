//! The optional features of Microsoft accounts (calendar, contacts, tasks,
//! the mailbox's time zone), each with a Graph permission of its own. One
//! the account lacks the permission for, or that Microsoft 365 refused,
//! answers every call with `PermissionRequired` and keeps nothing cached;
//! an administrator's approval, signing in again or checking again brings
//! it back.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::json;

use super::CoreService;
use crate::auth::microsoft::{is_consumer_domain, is_personal_tenant};
use crate::error::{Error, ErrorCode, Result};
use crate::model::{
    Account, AccountFeature, AccountKind, AccountSettings, AdminConsentLink, Calendar, Contact,
    ContactDraft, DataKind, Event, EventDraft, EventResponse, Task, TaskDraft, TaskList, TaskPatch,
};
use crate::providers::{CalendarProvider, ContactsProvider, TasksProvider};

/// Stands in for the provider of a feature the account can't use: every
/// call fails with `PermissionRequired`, except reading the server's
/// reminders, which reads as none kept (the default), so nothing reminds of
/// a calendar the app can't see.
pub(crate) struct NeedsPermission {
    pub feature: AccountFeature,
    pub account_id: String,
    pub email: String,
    /// Microsoft 365 refused the feature, rather than its permission
    /// missing.
    pub refused: bool,
}

impl NeedsPermission {
    fn error(&self) -> Error {
        let (noun, email) = (self.feature.noun(), &self.email);
        let (message, reason) = if self.refused {
            (
                format!(
                    "Microsoft 365 refused access to the {noun} of {email}. See Settings → Accounts."
                ),
                "refused",
            )
        } else {
            (
                format!(
                    "Cumulas Mail needs permission to use the {noun} of {email}. Turn it on in \
                     Settings → Accounts."
                ),
                "notGranted",
            )
        };
        Error::new(ErrorCode::PermissionRequired, message).with_details(json!({
            "feature": self.feature,
            "accountId": self.account_id,
            "reason": reason,
        }))
    }
}

#[async_trait]
impl CalendarProvider for NeedsPermission {
    async fn list_calendars(&self) -> Result<Vec<Calendar>> {
        Err(self.error())
    }

    async fn list_events(
        &self,
        _: &Calendar,
        _: DateTime<Utc>,
        _: DateTime<Utc>,
    ) -> Result<Vec<Event>> {
        Err(self.error())
    }

    async fn create_event(&self, _: &Calendar, _: &EventDraft) -> Result<Event> {
        Err(self.error())
    }

    async fn update_event(&self, _: &Calendar, _: &str, _: &EventDraft) -> Result<Event> {
        Err(self.error())
    }

    async fn delete_event(&self, _: &Calendar, _: &str) -> Result<()> {
        Err(self.error())
    }

    async fn respond_to_event(
        &self,
        _: &Calendar,
        _: &str,
        _: EventResponse,
        _: Option<&str>,
        _: bool,
    ) -> Result<()> {
        Err(self.error())
    }

    async fn dismiss_reminder(&self, _: &str) -> Result<()> {
        Err(self.error())
    }

    async fn snooze_reminder(&self, _: &str, _: DateTime<Utc>) -> Result<()> {
        Err(self.error())
    }
}

#[async_trait]
impl ContactsProvider for NeedsPermission {
    async fn list_contacts(&self) -> Result<Vec<Contact>> {
        Err(self.error())
    }

    async fn create_contact(&self, _: &ContactDraft) -> Result<Contact> {
        Err(self.error())
    }

    async fn update_contact(&self, _: &str, _: &ContactDraft) -> Result<Contact> {
        Err(self.error())
    }

    async fn delete_contact(&self, _: &str) -> Result<()> {
        Err(self.error())
    }
}

#[async_trait]
impl TasksProvider for NeedsPermission {
    async fn list_task_lists(&self) -> Result<Vec<TaskList>> {
        Err(self.error())
    }

    async fn list_tasks(&self, _: &TaskList) -> Result<Vec<Task>> {
        Err(self.error())
    }

    async fn create_task(&self, _: &TaskList, _: &TaskDraft) -> Result<Task> {
        Err(self.error())
    }

    async fn update_task(&self, _: &TaskList, _: &Task, _: &TaskPatch) -> Result<Task> {
        Err(self.error())
    }

    async fn delete_task(&self, _: &TaskList, _: &str) -> Result<()> {
        Err(self.error())
    }
}

/// Whether Graph refused to list a feature's items (HTTP 403): its
/// permission is granted, but Microsoft 365 doesn't let the account use it
/// (no license, or a mailbox without it).
pub(crate) fn is_listing_refusal(e: &Error) -> bool {
    e.http_status() == Some(403)
}

impl CoreService {
    /// Forgets what's cached of a feature the account can't use, so that
    /// nothing old looks current and no reminder goes off for a calendar
    /// the app can't see. It comes back with the feature.
    pub(super) fn drop_feature_data(
        &self,
        account: &Account,
        feature: AccountFeature,
    ) -> Result<()> {
        let id = account.id.as_str();
        let removed = match feature {
            AccountFeature::Calendar => {
                let had = self
                    .store
                    .list_calendars()?
                    .iter()
                    .any(|c| c.account_id == id);
                self.store.replace_calendars(id, &[])?;
                self.store.forget_server_reminders(id)?;
                had.then_some(DataKind::Calendar)
            }
            AccountFeature::Contacts => {
                let had = !self.store.list_contacts(Some(id), None)?.is_empty();
                self.store.replace_contacts(id, &[])?;
                had.then_some(DataKind::Contacts)
            }
            AccountFeature::Tasks => {
                let had = self
                    .store
                    .list_task_lists()?
                    .iter()
                    .any(|l| l.account_id == id);
                self.store.replace_task_lists(id, &[])?;
                had.then_some(DataKind::Tasks)
            }
            // Nothing is kept of it.
            AccountFeature::MailboxSettings => None,
        };
        if let Some(kind) = removed {
            self.notify(&[kind], Some(id));
        }
        Ok(())
    }

    /// Microsoft 365 refused to list a feature's items although its
    /// permission is granted: it is marked refused until checked again (a
    /// sign-in, Check again, its permission granted anew, or a day later),
    /// and its cache goes.
    pub(super) fn refuse_feature(&self, account: &Account, feature: AccountFeature) -> Result<()> {
        self.store.refuse_feature(&account.id, feature)?;
        self.drop_feature_data(account, feature)?;
        tracing::info!(
            "Microsoft 365 refused the {} of account {}",
            feature.noun(),
            account.id
        );
        self.notify(&[DataKind::Accounts], Some(&account.id));
        Ok(())
    }

    /// Where an administrator approves the app for an account's
    /// organization (`account_id`), or for the organization of the address
    /// being signed in with (`login_hint`); for `organizations` without
    /// either. Personal accounts have no administrator.
    pub fn microsoft_admin_consent(
        &self,
        account_id: Option<&str>,
        login_hint: Option<&str>,
    ) -> Result<AdminConsentLink> {
        let personal = || {
            Error::invalid_input(
                "Personal Microsoft accounts don't need an administrator's approval.",
            )
        };
        let tenant = match account_id {
            Some(id) => {
                let account = self.store.get_account(id)?;
                let AccountSettings::Microsoft(settings) = &account.settings else {
                    return Err(Error::invalid_input("Only Microsoft accounts need this."));
                };
                if is_personal_tenant(&settings.tenant_id) {
                    return Err(personal());
                }
                Some(settings.tenant_id.clone())
            }
            None => {
                let domain = login_hint
                    .and_then(|hint| hint.rsplit_once('@'))
                    .map(|(_, domain)| domain.trim())
                    .filter(|domain| !domain.is_empty());
                if domain.is_some_and(is_consumer_domain) {
                    return Err(personal());
                }
                domain.map(str::to_owned)
            }
        };
        self.connector.microsoft_admin_consent(tenant.as_deref())
    }

    /// Checks a Microsoft account's refused features again ("Check
    /// again"): they are no longer marked refused, and the next sync tries
    /// them.
    pub fn retry_microsoft_features(&self, account_id: &str) -> Result<()> {
        let account = self.store.get_account(account_id)?;
        if account.kind != AccountKind::Microsoft {
            return Err(Error::invalid_input("Only Microsoft accounts need this."));
        }
        if self.store.clear_refused(account_id)? {
            self.notify(&[DataKind::Accounts], Some(account_id));
        }
        Ok(())
    }
}
