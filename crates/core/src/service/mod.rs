//! The application service: everything the UI can do, independent of Tauri.
//!
//! Reads are served from the local [`Store`]; writes go to the server first
//! and are then reflected locally, so the cache never claims something the
//! server rejected.

mod alerts;
mod background;
mod connector;
mod mail;
mod pim;
mod sync;
pub(crate) mod targeted;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use uuid::Uuid;

pub use alerts::{MAX_AGE_HOURS, MailAlert, SUMMARY_THRESHOLD};
pub use connector::{
    BUILTIN_MICROSOFT_CLIENT_ID, BrowserOpener, Connector, LiveConnector, ProviderSet, PushSource,
};
pub use mail::Attachment;
pub use targeted::SyncScope;

use crate::error::{Error, ErrorCode, Result};
use crate::model::{
    Account, AccountKind, AccountSettings, ActiveReminder, Calendar, DataKind, EntityKind,
    ImapAccountUpdate, ImapSettings, MicrosoftSettings, NewImapAccount, Settings, SyncState,
    SyncStatus, TaskList, TlsMode, is_valid_font_family, local_id,
};
use crate::providers::imap::ImapCredentials;
use crate::push::PushStatus;
use crate::push::hub::PushHub;
use crate::secrets::{SecretStore, keys};
use crate::store::Store;

/// Receives notifications for the UI and the desktop.
pub trait EventSink: Send + Sync {
    /// Cached data changed; the UI should re-query the given kinds.
    fn data_changed(&self, kinds: &[DataKind], account_id: Option<&str>);
    fn sync_status(&self, status: &SyncStatus);
    /// New mail that passed the alert policy (see [`MailAlert`]).
    fn new_mail(&self, account: &Account, alert: &MailAlert);
    /// These messages were read, moved or deleted, here or elsewhere: their
    /// notifications can go.
    fn mail_settled(&self, _message_ids: &[String]) {}
    /// The due reminders changed: all `active` ones, the ones that just
    /// became due, and the keys of those no longer due.
    fn reminders_changed(
        &self,
        _active: &[ActiveReminder],
        _newly_due: &[ActiveReminder],
        _cleared: &[String],
    ) {
    }
    /// An account's push status changed.
    fn push_status(&self, _status: &PushStatus) {}
}

/// Sink that drops everything; for tests and headless use.
pub struct NullSink;

impl EventSink for NullSink {
    fn data_changed(&self, _: &[DataKind], _: Option<&str>) {}
    fn sync_status(&self, _: &SyncStatus) {}
    fn new_mail(&self, _: &Account, _: &MailAlert) {}
}

/// Accent colors assigned to new accounts in turn: teal, sky, apricot, plum,
/// moss, gold, rose and slate. Each reads as a dot or stripe and carries white
/// bold text as a fill.
const ACCOUNT_COLORS: &[&str] = &[
    "#0b7a75", "#2b7aa8", "#b35f26", "#8a4f7d", "#4d7a2e", "#8c6a10", "#b8434f", "#5e6b7a",
];

/// Remote ids of the calendar and task list created for IMAP accounts.
const LOCAL_CALENDAR_ID: &str = "local:calendar";
const LOCAL_TASK_LIST_ID: &str = "local:tasks";

pub struct CoreService {
    store: Arc<Store>,
    secrets: Arc<dyn SecretStore>,
    connector: Arc<dyn Connector>,
    sink: Arc<dyn EventSink>,
    providers: tokio::sync::Mutex<HashMap<String, ProviderSet>>,
    sync_status: Mutex<HashMap<String, SyncStatus>>,
    sync_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Cancels the Microsoft sign-in in progress, tagged with its sequence number.
    pending_sign_in: Mutex<Option<(u64, tokio::sync::oneshot::Sender<()>)>>,
    sign_in_seq: std::sync::atomic::AtomicU64,
    /// Push and the other background work, once started.
    push: PushHub,
}

impl CoreService {
    pub fn new(
        store: Arc<Store>,
        secrets: Arc<dyn SecretStore>,
        connector: Arc<dyn Connector>,
        sink: Arc<dyn EventSink>,
    ) -> Self {
        Self {
            store,
            secrets,
            connector,
            sink,
            providers: tokio::sync::Mutex::new(HashMap::new()),
            sync_status: Mutex::new(HashMap::new()),
            sync_locks: Mutex::new(HashMap::new()),
            pending_sign_in: Mutex::new(None),
            sign_in_seq: std::sync::atomic::AtomicU64::new(0),
            push: PushHub::default(),
        }
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    /// Identifies this installation to the push service.
    pub fn installation_id(&self) -> Result<String> {
        self.store.installation_id()
    }

    /// Providers for an account, created on first use and then cached so that
    /// connections and tokens are reused.
    async fn providers_for(&self, account_id: &str) -> Result<ProviderSet> {
        let mut cache = self.providers.lock().await;
        if let Some(set) = cache.get(account_id) {
            return Ok(set.clone());
        }
        let account = self.store.get_account(account_id)?;
        let set = self.connector.providers(&account).await?;
        cache.insert(account_id.to_owned(), set.clone());
        Ok(set)
    }

    async fn forget_providers(&self, account_id: Option<&str>) {
        let mut cache = self.providers.lock().await;
        match account_id {
            Some(id) => {
                cache.remove(id);
            }
            None => cache.clear(),
        }
    }

    fn notify(&self, kinds: &[DataKind], account_id: Option<&str>) {
        self.sink.data_changed(kinds, account_id);
    }

    // --- accounts -----------------------------------------------------------

    pub fn list_accounts(&self) -> Result<Vec<Account>> {
        self.store.list_accounts()
    }

    fn next_color(&self) -> Result<String> {
        let n = self.store.list_accounts()?.len();
        Ok(ACCOUNT_COLORS[n % ACCOUNT_COLORS.len()].to_owned())
    }

    /// Validates the settings against the servers, then saves the account.
    /// The password goes to the system keyring.
    pub async fn add_imap_account(&self, input: NewImapAccount) -> Result<Account> {
        let email = input.email.trim().to_owned();
        if !email.contains('@') {
            return Err(Error::invalid_input("Enter a valid email address."));
        }
        validate_imap_settings(&input.settings)?;
        if self
            .store
            .list_accounts()?
            .iter()
            .any(|a| a.email.eq_ignore_ascii_case(&email))
        {
            return Err(Error::invalid_input(format!(
                "{email} has already been added."
            )));
        }

        let display_name = if input.display_name.trim().is_empty() {
            email.clone()
        } else {
            input.display_name.trim().to_owned()
        };
        let credentials = ImapCredentials {
            settings: input.settings.clone(),
            password: input.password.clone(),
            display_name: display_name.clone(),
            email: email.clone(),
        };
        self.connector.verify_imap(&credentials).await?;

        let account = Account {
            id: Uuid::new_v4().to_string(),
            kind: AccountKind::Imap,
            display_name,
            email,
            color: self.next_color()?,
            settings: AccountSettings::Imap(input.settings),
            created_at: Utc::now(),
            last_synced_at: None,
            sync_error: None,
        };
        self.secrets
            .set(&keys::imap_password(&account.id), &input.password)
            .await?;
        self.store.insert_account(&account)?;
        self.create_local_pim_containers(&account)?;
        self.restart_push(&account.id).await;
        self.notify(&[DataKind::Accounts], Some(&account.id));
        Ok(account)
    }

    /// Changes the password and/or server settings of an IMAP account, after
    /// checking them against the servers.
    pub async fn update_imap_account(
        &self,
        account_id: &str,
        update: ImapAccountUpdate,
    ) -> Result<Account> {
        let mut account = self.store.get_account(account_id)?;
        let AccountSettings::Imap(current) = &account.settings else {
            return Err(Error::unsupported(
                "Only IMAP accounts have server settings.",
            ));
        };
        let settings = update.settings.unwrap_or_else(|| current.clone());
        validate_imap_settings(&settings)?;
        let password = match update.password.filter(|p| !p.is_empty()) {
            Some(password) => password,
            None => self
                .secrets
                .get(&keys::imap_password(account_id))
                .await?
                .ok_or_else(|| Error::invalid_input("Enter the password for this account."))?,
        };
        self.connector
            .verify_imap(&ImapCredentials {
                settings: settings.clone(),
                password: password.clone(),
                display_name: account.display_name.clone(),
                email: account.email.clone(),
            })
            .await?;

        self.secrets
            .set(&keys::imap_password(account_id), &password)
            .await?;
        account.settings = AccountSettings::Imap(settings);
        account.sync_error = None;
        self.store.update_account(&account)?;
        self.store.set_account_sync_result(account_id, None, None)?;
        self.forget_providers(Some(account_id)).await;
        self.restart_push(account_id).await;
        self.notify(&[DataKind::Accounts], Some(account_id));
        Ok(account)
    }

    /// IMAP accounts get a local calendar and task list so every account can
    /// use every module.
    fn create_local_pim_containers(&self, account: &Account) -> Result<()> {
        self.store.upsert_calendar(&Calendar {
            id: local_id(&account.id, EntityKind::Calendar, LOCAL_CALENDAR_ID),
            account_id: account.id.clone(),
            remote_id: LOCAL_CALENDAR_ID.into(),
            name: "Calendar".into(),
            color: account.color.clone(),
            can_edit: true,
            is_default: true,
        })?;
        self.store.upsert_task_list(&TaskList {
            id: local_id(&account.id, EntityKind::TaskList, LOCAL_TASK_LIST_ID),
            account_id: account.id.clone(),
            remote_id: LOCAL_TASK_LIST_ID.into(),
            name: "Tasks".into(),
            is_default: true,
        })?;
        Ok(())
    }

    /// Signs in to a Microsoft account in the browser and adds it, or refreshes
    /// the credentials of an account that was already added.
    pub async fn add_microsoft_account(
        &self,
        open_browser: &BrowserOpener,
        login_hint: Option<&str>,
    ) -> Result<Account> {
        let seq = self
            .sign_in_seq
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        // Starting a new sign-in cancels one still waiting for the browser.
        if let Some((_, previous)) = self
            .pending_sign_in
            .lock()
            .unwrap()
            .replace((seq, cancel_tx))
        {
            let _ = previous.send(());
        }
        let outcome = tokio::select! {
            result = self.connector.microsoft_sign_in(open_browser, login_hint) => result,
            _ = cancel_rx => Err(Error::new(ErrorCode::SignInCancelled, "Sign-in was canceled.")),
        };
        {
            let mut pending = self.pending_sign_in.lock().unwrap();
            if pending.as_ref().is_some_and(|(s, _)| *s == seq) {
                *pending = None;
            }
        }
        let result = outcome?;
        let existing = self.store.list_accounts()?.into_iter().find(
            |a| matches!(&a.settings, AccountSettings::Microsoft(m) if m.user_id == result.user_id),
        );
        let account = match existing {
            Some(mut account) => {
                account.display_name = result.display_name.clone();
                account.email = result.email.clone();
                self.store.update_account(&account)?;
                self.store
                    .set_account_sync_result(&account.id, None, None)?;
                account
            }
            None => {
                let account = Account {
                    id: Uuid::new_v4().to_string(),
                    kind: AccountKind::Microsoft,
                    display_name: result.display_name.clone(),
                    email: result.email.clone(),
                    color: self.next_color()?,
                    settings: AccountSettings::Microsoft(MicrosoftSettings {
                        tenant_id: result.tenant_id.clone(),
                        user_id: result.user_id.clone(),
                    }),
                    created_at: Utc::now(),
                    last_synced_at: None,
                    sync_error: None,
                };
                self.store.insert_account(&account)?;
                account
            }
        };
        self.secrets
            .set(&keys::refresh_token(&account.id), &result.refresh_token)
            .await?;
        self.forget_providers(Some(&account.id)).await;
        self.restart_push(&account.id).await;
        self.notify(&[DataKind::Accounts], Some(&account.id));
        Ok(account)
    }

    /// Abandons a Microsoft sign-in that is waiting for the browser; the
    /// pending `add_microsoft_account` call fails with `SignInCancelled`.
    pub fn cancel_microsoft_sign_in(&self) {
        if let Some((_, cancel)) = self.pending_sign_in.lock().unwrap().take() {
            let _ = cancel.send(());
        }
    }

    pub async fn update_account(
        &self,
        account_id: &str,
        display_name: Option<String>,
        color: Option<String>,
    ) -> Result<Account> {
        let mut account = self.store.get_account(account_id)?;
        if let Some(name) = display_name.filter(|n| !n.trim().is_empty()) {
            account.display_name = name.trim().to_owned();
        }
        if let Some(color) = color {
            if !is_hex_color(&color) {
                return Err(Error::invalid_input("Color must look like #1a2b3c."));
            }
            account.color = color;
        }
        self.store.update_account(&account)?;
        self.forget_providers(Some(account_id)).await;
        self.notify(&[DataKind::Accounts], Some(account_id));
        Ok(account)
    }

    /// Removes the account, its cached data and its stored credentials.
    pub async fn remove_account(&self, account_id: &str) -> Result<()> {
        self.store.get_account(account_id)?;
        // Before the data goes, so no sync writes to it meanwhile.
        self.push.stop(Some(&[account_id.to_owned()])).await;
        self.push.forget_status(account_id);
        self.forget_providers(Some(account_id)).await;
        self.store.delete_account(account_id)?;
        for key in [
            keys::imap_password(account_id),
            keys::refresh_token(account_id),
        ] {
            if let Err(e) = self.secrets.delete(&key).await {
                tracing::warn!("could not delete secret {key}: {e}");
            }
        }
        self.sync_status.lock().unwrap().remove(account_id);
        self.notify(
            &[
                DataKind::Accounts,
                DataKind::Mail,
                DataKind::Calendar,
                DataKind::Contacts,
                DataKind::Tasks,
            ],
            None,
        );
        Ok(())
    }

    // --- settings -----------------------------------------------------------

    pub fn get_settings(&self) -> Result<Settings> {
        self.store.get_settings()
    }

    pub async fn update_settings(&self, settings: Settings) -> Result<Settings> {
        validate_settings(&settings)?;
        let previous = self.store.get_settings()?;
        self.store.save_settings(&settings)?;
        let microsoft_app_changed = previous.microsoft_client_id != settings.microsoft_client_id
            || previous.microsoft_tenant != settings.microsoft_tenant;
        if microsoft_app_changed {
            self.forget_providers(None).await;
        }
        if microsoft_app_changed
            || previous.instant_push != settings.instant_push
            || previous.use_push_relay != settings.use_push_relay
        {
            self.restart_all_push().await;
        }
        Ok(settings)
    }

    // --- sync status ----------------------------------------------------------

    pub fn sync_statuses(&self) -> Result<Vec<SyncStatus>> {
        let live = self.sync_status.lock().unwrap().clone();
        Ok(self
            .store
            .list_accounts()?
            .into_iter()
            .map(|a| {
                live.get(&a.id).cloned().unwrap_or(SyncStatus {
                    account_id: a.id.clone(),
                    state: if a.sync_error.is_some() {
                        SyncState::Error
                    } else {
                        SyncState::Idle
                    },
                    message: a.sync_error.clone(),
                    last_synced_at: a.last_synced_at,
                })
            })
            .collect())
    }

    fn set_sync_status(&self, status: SyncStatus) {
        self.sink.sync_status(&status);
        self.sync_status
            .lock()
            .unwrap()
            .insert(status.account_id.clone(), status);
    }
}

fn validate_settings(s: &Settings) -> Result<()> {
    if s.sync_interval_minutes == 0 {
        return Err(Error::invalid_input(
            "Sync interval must be at least one minute.",
        ));
    }
    if s.default_snooze_minutes == 0 {
        return Err(Error::invalid_input(
            "Reminders must snooze for at least one minute.",
        ));
    }
    let families = [s.interface_font.family(), s.reading_font.family()];
    if families
        .into_iter()
        .flatten()
        .any(|f| !is_valid_font_family(f))
    {
        return Err(Error::invalid_input(
            "That font name can't be used. Choose a font from the list.",
        ));
    }
    Ok(())
}

fn validate_imap_settings(s: &ImapSettings) -> Result<()> {
    if s.imap_host.trim().is_empty() || s.smtp_host.trim().is_empty() {
        return Err(Error::invalid_input(
            "Enter the IMAP and SMTP server names.",
        ));
    }
    if s.imap_port == 0 || s.smtp_port == 0 {
        return Err(Error::invalid_input(
            "Enter the IMAP and SMTP port numbers.",
        ));
    }
    for (host, tls) in [(&s.imap_host, s.imap_tls), (&s.smtp_host, s.smtp_tls)] {
        if tls == TlsMode::None && !is_loopback(host) {
            return Err(Error::invalid_input(format!(
                "Unencrypted connections are only allowed to this computer, not {host}."
            )));
        }
    }
    Ok(())
}

fn is_loopback(host: &str) -> bool {
    matches!(host.trim(), "localhost" | "127.0.0.1" | "::1" | "[::1]")
}

fn is_hex_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
pub(crate) mod tests;
