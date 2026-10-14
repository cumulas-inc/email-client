//! Pulls server state into the local store.

use std::sync::Arc;

use chrono::{Duration, Utc};

use super::{CoreService, ProviderSet};
use crate::error::{Error, ErrorCode, Result};
use crate::model::{Account, DataKind, FolderRole, SyncState, SyncStatus};
use crate::providers::SyncWindow;

/// How far back the first sync of a folder reaches.
const MAIL_WINDOW_DAYS: i64 = 90;
const MAIL_MAX_PER_FOLDER: u32 = 500;
/// Calendar window kept in sync in the background (the UI can request others).
const CALENDAR_PAST_DAYS: i64 = 31;
const CALENDAR_FUTURE_DAYS: i64 = 180;

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
                tracing::warn!("sync of {} failed: {e}", account.email);
            }
        }
    }

    /// Syncs one account. Returns immediately if it is already syncing.
    pub async fn sync_account(&self, account_id: &str) -> Result<()> {
        let lock = {
            let mut locks = self.sync_locks.lock().unwrap();
            Arc::clone(locks.entry(account_id.to_owned()).or_default())
        };
        let Ok(_guard) = lock.try_lock() else {
            return Ok(());
        };
        let account = self.store.get_account(account_id)?;

        let result = self.run_sync(&account).await;
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
                    ErrorCode::AuthRequired => {
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
        let providers = self.providers_for(&account.id).await?;
        self.sync_mail(account, &providers).await?;
        // Calendar, contacts and tasks failures should not hide mail errors and
        // vice versa, but one failing module must not stop the others.
        let mut first_error: Option<Error> = None;
        if let Err(e) = self.sync_calendar(account, &providers).await {
            tracing::warn!("calendar sync for {} failed: {e}", account.email);
            first_error.get_or_insert(e);
        }
        if let Err(e) = self.sync_contacts(account, &providers).await {
            tracing::warn!("contacts sync for {} failed: {e}", account.email);
            first_error.get_or_insert(e);
        }
        if let Err(e) = self.sync_tasks(account, &providers).await {
            tracing::warn!("tasks sync for {} failed: {e}", account.email);
            first_error.get_or_insert(e);
        }
        first_error.map_or(Ok(()), Err)
    }

    async fn sync_mail(&self, account: &Account, providers: &ProviderSet) -> Result<()> {
        self.progress(account, "Updating folders…");
        let folders = providers.mail.list_folders().await?;
        self.store.replace_folders(&account.id, &folders)?;
        self.notify(&[DataKind::Mail], Some(&account.id));

        let mut ordered = self.store.list_folders(Some(&account.id))?;
        // Inbox first, then other well-known folders, then the rest.
        ordered.sort_by_key(|f| f.role.sort_rank());
        let window = SyncWindow {
            since: Utc::now() - Duration::days(MAIL_WINDOW_DAYS),
            max_messages: MAIL_MAX_PER_FOLDER,
        };
        let mut first_error = None;
        for folder in ordered {
            self.progress(account, format!("Syncing {}…", folder.name));
            let state = self.store.folder_sync_state(&folder.id)?;
            let batch = match providers
                .mail
                .sync_messages(&folder, state.as_deref(), window)
                .await
            {
                Ok(batch) => batch,
                Err(e) if e.code == ErrorCode::AuthRequired => return Err(e),
                Err(e) => {
                    tracing::warn!("syncing folder {} failed: {e}", folder.name);
                    first_error.get_or_insert(e);
                    continue;
                }
            };
            let inserted = self.store.apply_message_batch(&folder, &batch)?;
            // The first sync of a folder is not "new mail".
            if folder.role == FolderRole::Inbox && state.is_some() {
                let unread: Vec<_> = inserted.into_iter().filter(|m| !m.is_read).collect();
                if !unread.is_empty() {
                    self.sink.new_mail(account, &unread);
                }
            }
            self.notify(&[DataKind::Mail], Some(&account.id));
        }
        first_error.map_or(Ok(()), Err)
    }

    async fn sync_calendar(&self, account: &Account, providers: &ProviderSet) -> Result<()> {
        let Some(provider) = &providers.calendar else {
            return Ok(());
        };
        self.progress(account, "Syncing calendars…");
        let calendars = provider.list_calendars().await?;
        self.store.replace_calendars(&account.id, &calendars)?;
        let today = Utc::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .expect("midnight is valid")
            .and_utc();
        let start = today - Duration::days(CALENDAR_PAST_DAYS);
        let end = today + Duration::days(CALENDAR_FUTURE_DAYS);
        for calendar in &calendars {
            let events = provider.list_events(calendar, start, end).await?;
            self.store
                .replace_events_in_range(&calendar.id, start, end, &events)?;
        }
        self.notify(&[DataKind::Calendar], Some(&account.id));
        Ok(())
    }

    async fn sync_contacts(&self, account: &Account, providers: &ProviderSet) -> Result<()> {
        let Some(provider) = &providers.contacts else {
            return Ok(());
        };
        self.progress(account, "Syncing contacts…");
        let contacts = provider.list_contacts().await?;
        self.store.replace_contacts(&account.id, &contacts)?;
        self.notify(&[DataKind::Contacts], Some(&account.id));
        Ok(())
    }

    async fn sync_tasks(&self, account: &Account, providers: &ProviderSet) -> Result<()> {
        let Some(provider) = &providers.tasks else {
            return Ok(());
        };
        self.progress(account, "Syncing tasks…");
        let lists = provider.list_task_lists().await?;
        self.store.replace_task_lists(&account.id, &lists)?;
        for list in &lists {
            let tasks = provider.list_tasks(list).await?;
            self.store.replace_tasks(&list.id, &tasks)?;
        }
        self.notify(&[DataKind::Tasks], Some(&account.id));
        Ok(())
    }
}
