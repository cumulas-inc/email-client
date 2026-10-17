//! Pulls server state into the local store.

use std::sync::Arc;

use chrono::{Duration, Utc};

use super::permissions::is_listing_refusal;
use super::{CoreService, ProviderSet, SyncScope, alerts};
use crate::error::{Error, ErrorCode, Result};
use crate::model::{Account, AccountFeature, AccountKind, DataKind, Folder, SyncState, SyncStatus};
use crate::providers::SyncWindow;
use crate::providers::imap::is_server_busy;
use crate::store::BatchOutcome;

/// How far back the first sync of a folder reaches.
const MAIL_WINDOW_DAYS: i64 = 90;
const MAIL_MAX_PER_FOLDER: u32 = 500;
/// Calendar window kept in sync in the background (the UI can request others).
const CALENDAR_PAST_DAYS: i64 = 31;
const CALENDAR_FUTURE_DAYS: i64 = 180;
/// How long a feature Microsoft 365 refused stays refused before a full
/// sync tries it again, picking up a license or mailbox fixed meanwhile.
const REFUSED_RETRY: Duration = Duration::hours(24);

impl CoreService {
    /// Syncs every account; errors are recorded per account, not returned.
    pub async fn sync_all(&self) {
        let accounts = match self.store.list_accounts() {
            Ok(a) => a,
            Err(e) => {
                tracing::error!("listing accounts for sync failed: {e}");
                return;
            }
        };
        for account in accounts {
            if let Err(e) = self.sync_account(&account.id).await {
                log_sync_failure(&account.id, &e);
            }
        }
    }

    /// The lock held while an account syncs.
    pub(super) fn account_lock(&self, account_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.sync_locks.lock().unwrap();
        Arc::clone(locks.entry(account_id.to_owned()).or_default())
    }

    /// Syncs one account. If a sync of it is running already, perhaps of
    /// just what push reported, a full sync follows that one: queued,
    /// returning at once, while the background work runs, and waited for
    /// otherwise.
    pub async fn sync_account(&self, account_id: &str) -> Result<()> {
        let lock = self.account_lock(account_id);
        let Ok(_guard) = lock.try_lock() else {
            if self.push.request_for(account_id, SyncScope::full()) {
                return Ok(());
            }
            return self.sync_account_waiting(account_id).await;
        };
        self.sync_account_locked(account_id).await
    }

    /// Syncs one account, after the sync in progress if there is one.
    pub(super) async fn sync_account_waiting(&self, account_id: &str) -> Result<()> {
        let lock = self.account_lock(account_id);
        let _guard = lock.lock().await;
        self.sync_account_locked(account_id).await
    }

    /// Syncs one account; the caller holds its lock.
    async fn sync_account_locked(&self, account_id: &str) -> Result<()> {
        let account = self.store.get_account(account_id)?;

        let result = self.run_sync(&account).await;
        // Whatever became of the sync: a folder that keeps failing, or an
        // account whose sign-in expired, must not keep search results past
        // their time.
        self.prune_search_results(account_id);
        if let Err(e) = &result
            && is_server_busy(e)
        {
            self.yield_push(account_id, e, SyncScope::full());
        }
        self.resume_push(account_id, &result);
        let now = Utc::now();
        match &result {
            Ok(()) => {
                self.store
                    .set_account_sync_result(account_id, Some(now), None)?;
                self.set_sync_status(SyncStatus {
                    account_id: account_id.to_owned(),
                    state: SyncState::Idle,
                    message: None,
                    last_synced_at: Some(now),
                });
            }
            Err(e) => {
                let message = match e.code {
                    // Unless the error says why the sign-in is gone (the
                    // organization's settings keep the person out).
                    ErrorCode::AuthRequired
                        if e.details.as_ref().is_none_or(|d| d.get("reason").is_none()) =>
                    {
                        format!("Sign in again to keep {} up to date.", account.email)
                    }
                    _ => e.message.clone(),
                };
                self.store
                    .set_account_sync_result(account_id, None, Some(&message))?;
                // Drop cached providers so the next attempt reconnects.
                self.forget_providers(Some(account_id)).await;
                self.set_sync_status(SyncStatus {
                    account_id: account_id.to_owned(),
                    state: SyncState::Error,
                    message: Some(message),
                    last_synced_at: account.last_synced_at,
                });
            }
        }
        self.notify(&[DataKind::Accounts], Some(account_id));
        result
    }

    fn progress(&self, account: &Account, message: impl Into<String>) {
        self.set_sync_status(SyncStatus {
            account_id: account.id.clone(),
            state: SyncState::Syncing,
            message: Some(message.into()),
            last_synced_at: account.last_synced_at,
        });
    }

    async fn run_sync(&self, account: &Account) -> Result<()> {
        self.progress(account, "Connecting…");
        if account.kind == AccountKind::Microsoft
            && self.store.expire_refused(&account.id, REFUSED_RETRY)?
        {
            self.notify(&[DataKind::Accounts], Some(&account.id));
        }
        let providers = self.providers_for(&account.id).await?;
        let mail = self.sync_mail(account, &providers).await;
        if let Err(e) = &mail {
            // The sign-in is gone, or the keyring can't be read: the rest
            // would fail the same way (and ask to unlock the keyring again).
            if matches!(e.code, ErrorCode::AuthRequired | ErrorCode::Storage) {
                return mail;
            }
            tracing::warn!("mail sync for account {} failed: {e}", account.id);
        }
        // The token refresh during the mail sync may have recorded a new grant.
        let account = &self.store.get_account(&account.id)?;
        let providers = self.providers_for(&account.id).await?;
        // Calendar, contacts and tasks failures should not hide mail errors and
        // vice versa, but one failing module must not stop the others.
        let mut first_error: Option<Error> = None;
        if let Err(e) = self.sync_calendar(account, &providers).await {
            tracing::warn!("calendar sync for account {} failed: {e}", account.id);
            first_error.get_or_insert(e);
        }
        if let Err(e) = self.sync_contacts(account, &providers).await {
            tracing::warn!("contacts sync for account {} failed: {e}", account.id);
            first_error.get_or_insert(e);
        }
        if let Err(e) = self.sync_tasks(account, &providers).await {
            tracing::warn!("tasks sync for account {} failed: {e}", account.id);
            first_error.get_or_insert(e);
        }
        self.prune_reminders();
        mail.and(first_error.map_or(Ok(()), Err))
    }

    async fn sync_mail(&self, account: &Account, providers: &ProviderSet) -> Result<()> {
        self.progress(account, "Updating folders…");
        let folders = providers.mail.list_folders().await?;
        self.store.replace_folders(&account.id, &folders)?;
        self.notify(&[DataKind::Mail], Some(&account.id));

        let mut ordered = self.store.list_folders(Some(&account.id))?;
        // Inbox first, then other well-known folders, then the rest.
        ordered.sort_by_key(|f| f.role.sort_rank());
        let mut first_error = None;
        for folder in ordered {
            self.progress(account, format!("Syncing {}…", folder.name));
            match self
                .sync_one_folder(account, providers, &folder, false)
                .await
            {
                Ok(_) => {}
                Err(e) if e.code == ErrorCode::AuthRequired => return Err(e),
                Err(e) => {
                    tracing::warn!("syncing folder {} failed: {e}", folder.name);
                    first_error.get_or_insert(e);
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Syncs one folder's messages into the store, raises notifications for
    /// new mail and withdraws those for mail read, moved or removed
    /// elsewhere. A `targeted` sync also refreshes the folder's counts when
    /// messages changed, and those of folders messages moved in from or out
    /// to, since it doesn't list the folders.
    pub(super) async fn sync_one_folder(
        &self,
        account: &Account,
        providers: &ProviderSet,
        folder: &Folder,
        targeted: bool,
    ) -> Result<BatchOutcome> {
        let window = SyncWindow {
            since: Utc::now() - Duration::days(MAIL_WINDOW_DAYS),
            max_messages: MAIL_MAX_PER_FOLDER,
        };
        let state = self.store.folder_sync_state(&folder.id)?;
        let batch = providers
            .mail
            .sync_messages(folder, state.as_deref(), window)
            .await?;
        let outcome = self.store.apply_message_batch(folder, &batch)?;
        if targeted && outcome.changes_counts() {
            let mut folders = vec![folder.clone()];
            for id in outcome.moved_from.iter().chain(&outcome.moved_to) {
                if folders.iter().all(|f| &f.id != id) {
                    folders.push(self.store.get_folder(id)?);
                }
            }
            for folder in &folders {
                match providers.mail.folder_counts(folder).await {
                    Ok(Some(counts)) => {
                        self.store
                            .set_folder_counts(&folder.id, counts.unread, counts.total)?;
                    }
                    Ok(None) => {}
                    Err(e) => tracing::debug!("reading folder counts failed ({:?})", e.code),
                }
            }
        }

        if !outcome.inserted.is_empty() {
            let settings = self.store.get_settings()?;
            // The first sync of a folder is not "new mail".
            let first_sync = state.is_none();
            if let Some(alert) = alerts::mail_alert(
                &settings,
                &account.id,
                folder,
                first_sync,
                &outcome.inserted,
                Utc::now(),
            ) {
                self.sink.new_mail(account, &alert);
            }
        }
        let settled: Vec<String> = outcome
            .became_read
            .iter()
            .chain(&outcome.removed)
            .chain(&outcome.moved_out)
            .cloned()
            .collect();
        if !settled.is_empty() {
            self.sink.mail_settled(&settled);
        }

        // Targeted syncs run often; only bother the UI when something changed.
        let changed = !batch.upserts.is_empty()
            || !batch.flag_updates.is_empty()
            || !outcome.removed.is_empty()
            || !batch.moved_out.is_empty()
            || batch.full_resync;
        if !targeted || changed {
            self.notify(&[DataKind::Mail], Some(&account.id));
        }
        Ok(outcome)
    }

    async fn sync_calendar(&self, account: &Account, providers: &ProviderSet) -> Result<()> {
        if providers.calendar.is_some() && can_use(account, AccountFeature::Calendar) {
            self.progress(account, "Syncing calendars…");
        }
        self.refresh_calendars(account, providers).await
    }

    /// Syncs the calendars without reporting progress, as targeted syncs
    /// don't: nothing would report the end.
    pub(super) async fn refresh_calendars(
        &self,
        account: &Account,
        providers: &ProviderSet,
    ) -> Result<()> {
        let Some(provider) = &providers.calendar else {
            return Ok(());
        };
        let listed = provider.list_calendars().await;
        let Some(calendars) = self.listed(account, AccountFeature::Calendar, listed)? else {
            return Ok(());
        };
        self.store.replace_calendars(&account.id, &calendars)?;
        let today = Utc::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .expect("midnight is valid")
            .and_utc();
        let start = today - Duration::days(CALENDAR_PAST_DAYS);
        let end = today + Duration::days(CALENDAR_FUTURE_DAYS);
        for calendar in &calendars {
            let events = match provider.list_events(calendar, start, end).await {
                Ok(events) => events,
                // About this calendar alone, such as one shared with the
                // account that it may no longer read.
                Err(e) if is_listing_refusal(&e) => {
                    tracing::info!(
                        "skipping a calendar of account {} that Microsoft 365 refused to read",
                        account.id
                    );
                    continue;
                }
                Err(e) => return Err(e),
            };
            self.store
                .replace_events_in_range(&calendar.id, start, end, &events)?;
        }
        self.refresh_server_reminders(account, provider).await;
        self.notify(&[DataKind::Calendar], Some(&account.id));
        Ok(())
    }

    async fn sync_contacts(&self, account: &Account, providers: &ProviderSet) -> Result<()> {
        let Some(provider) = &providers.contacts else {
            return Ok(());
        };
        if can_use(account, AccountFeature::Contacts) {
            self.progress(account, "Syncing contacts…");
        }
        let listed = provider.list_contacts().await;
        let Some(contacts) = self.listed(account, AccountFeature::Contacts, listed)? else {
            return Ok(());
        };
        self.store.replace_contacts(&account.id, &contacts)?;
        self.notify(&[DataKind::Contacts], Some(&account.id));
        Ok(())
    }

    async fn sync_tasks(&self, account: &Account, providers: &ProviderSet) -> Result<()> {
        let Some(provider) = &providers.tasks else {
            return Ok(());
        };
        if can_use(account, AccountFeature::Tasks) {
            self.progress(account, "Syncing tasks…");
        }
        let listed = provider.list_task_lists().await;
        let Some(lists) = self.listed(account, AccountFeature::Tasks, listed)? else {
            return Ok(());
        };
        self.store.replace_task_lists(&account.id, &lists)?;
        for list in &lists {
            let tasks = match provider.list_tasks(list).await {
                Ok(tasks) => tasks,
                // About this list alone, such as one shared with the account.
                Err(e) if is_listing_refusal(&e) => {
                    tracing::info!(
                        "skipping a task list of account {} that Microsoft 365 refused to read",
                        account.id
                    );
                    continue;
                }
                Err(e) => return Err(e),
            };
            self.store.replace_tasks(&list.id, &tasks)?;
        }
        self.notify(&[DataKind::Tasks], Some(&account.id));
        Ok(())
    }

    /// A feature's top-level listing, or `None` when the account can't use
    /// the feature: without its permission its cache goes, and when
    /// Microsoft 365 refuses the listing (HTTP 403) the feature is marked
    /// refused. Neither is a sync error.
    fn listed<T>(
        &self,
        account: &Account,
        feature: AccountFeature,
        listed: Result<Vec<T>>,
    ) -> Result<Option<Vec<T>>> {
        match listed {
            Ok(items) => Ok(Some(items)),
            Err(e) if e.code == ErrorCode::PermissionRequired => {
                self.drop_feature_data(account, feature)?;
                Ok(None)
            }
            Err(e) if account.kind == AccountKind::Microsoft && is_listing_refusal(&e) => {
                self.refuse_feature(account, feature)?;
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }
}

/// Whether the account may use a feature now (it says nothing of
/// providers the account doesn't have).
fn can_use(account: &Account, feature: AccountFeature) -> bool {
    !account.needs_permission.contains(&feature) && !account.refused_features.contains(&feature)
}

/// Logs a failed sync without the user name an authentication error names.
pub(super) fn log_sync_failure(account_id: &str, error: &Error) {
    match error.code {
        ErrorCode::AuthRequired => tracing::warn!(
            "sync of account {account_id} failed: the server rejected the credentials"
        ),
        _ => tracing::warn!("sync of account {account_id} failed: {error}"),
    }
}
