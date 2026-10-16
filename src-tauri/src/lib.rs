//! Tauri shell: wires [`CoreService`] to the webview through commands and
//! events, starts and stops the core's background work (sync and push), and
//! fits the app into the desktop (`desktop`): notifications, the tray,
//! running in the background, one instance per user.

mod commands;
mod desktop;
mod fonts;

use std::sync::Arc;
use std::time::Duration;

use cumulas_core::CoreService;
use cumulas_core::model::{Account, ActiveReminder, DataKind, SyncStatus};
use cumulas_core::push::PushStatus;
use cumulas_core::secrets::{KeyringSecretStore, MemorySecretStore, SecretStore};
use cumulas_core::service::{EventSink, LiveConnector, MailAlert};
use cumulas_core::store::Store;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, RunEvent};

use crate::desktop::Desktop;
use crate::desktop::background::BACKGROUND_ARG;

/// The most the app waits on exit for push connections to close.
const SHUTDOWN_LIMIT: Duration = Duration::from_secs(2);

pub(crate) struct AppState {
    pub service: Arc<CoreService>,
    /// False when no Secret Service was reachable and secrets only live in
    /// memory for this session.
    pub persistent_secrets: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DataChanged<'a> {
    pub kinds: &'a [DataKind],
    pub account_id: Option<&'a str>,
}

/// Forwards core events to the webview and the desktop.
struct TauriSink {
    app: AppHandle,
}

impl EventSink for TauriSink {
    fn data_changed(&self, kinds: &[DataKind], account_id: Option<&str>) {
        let _ = self
            .app
            .emit("data-changed", DataChanged { kinds, account_id });
        if self.app.try_state::<Desktop>().is_none() {
            return;
        }
        if kinds.contains(&DataKind::Mail) {
            desktop::tray::mail_changed(&self.app);
        }
        if kinds.contains(&DataKind::Accounts) {
            let app = self.app.clone();
            tauri::async_runtime::spawn(async move {
                desktop::background::set_status(&app).await;
            });
        }
    }

    fn sync_status(&self, status: &SyncStatus) {
        let _ = self.app.emit("sync-status", status);
    }

    /// The core decided these should be shown (the alert policy).
    fn new_mail(&self, account: &Account, alert: &MailAlert) {
        let (app, account, alert) = (self.app.clone(), account.clone(), alert.clone());
        tauri::async_runtime::spawn(async move {
            desktop::new_mail(&app, &account, alert).await;
        });
    }

    /// Read, moved or deleted: their notifications go.
    fn mail_settled(&self, message_ids: &[String]) {
        let (app, ids) = (self.app.clone(), message_ids.to_vec());
        tauri::async_runtime::spawn(async move {
            if let Some(desktop) = app.try_state::<Desktop>() {
                desktop.notifications.settle(&ids).await;
            }
        });
    }

    fn push_status(&self, status: &PushStatus) {
        let _ = self.app.emit("push-status", status);
    }

    /// For the page's Reminders panel, and the desktop's notifications.
    fn reminders_changed(
        &self,
        active: &[ActiveReminder],
        newly_due: &[ActiveReminder],
        cleared: &[String],
    ) {
        let _ = self.app.emit(
            "reminders-changed",
            RemindersChanged {
                active,
                newly_due,
                cleared,
            },
        );
        if let Some(desktop) = self.app.try_state::<Desktop>() {
            desktop.reminders_changed(newly_due.to_vec(), cleared.to_vec());
        }
    }
}

/// The `reminders-changed` event.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemindersChanged<'a> {
    pub active: &'a [ActiveReminder],
    pub newly_due: &'a [ActiveReminder],
    pub cleared: &'a [String],
}

async fn open_secret_store() -> (Arc<dyn SecretStore>, bool) {
    match KeyringSecretStore::new().await {
        Ok(store) => (Arc::new(store), true),
        Err(e) => {
            tracing::error!(
                "{e}; passwords and sign-ins will only be kept until the app is closed"
            );
            (Arc::new(MemorySecretStore::default()), false)
        }
    }
}

/// One process per user: a second launch shows the first one's window
/// (unless it's another start at login) and exits. Registered first, so
/// the second launch stops before anything else starts.
fn single_instance(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    // The plugin can't run without a usable session bus address (and its
    // version for this Tauri panics then); every launch then runs on its own.
    #[cfg(target_os = "linux")]
    if let Err(e) = zbus::Address::session() {
        tracing::warn!("no session bus ({e}); not checking for another running copy");
        return builder;
    }
    builder.plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
        if argv.iter().any(|a| a == BACKGROUND_ARG) {
            return;
        }
        desktop::window::show_main(app, None);
    }))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,cumulas_core=debug".into()),
        )
        .init();
    // Started at login without a window: none is created until asked for.
    let in_background = std::env::args().any(|a| a == BACKGROUND_ARG);

    single_instance(tauri::Builder::default())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let data_dir = app.path().app_data_dir()?;
            let store = Arc::new(Store::open(&data_dir.join("mail.db"))?);
            let (secrets, persistent_secrets) = tauri::async_runtime::block_on(open_secret_store());
            let connector = Arc::new(LiveConnector::new(Arc::clone(&store), Arc::clone(&secrets)));
            let sink = Arc::new(TauriSink {
                app: app.handle().clone(),
            });
            let service = Arc::new(CoreService::new(store, secrets, connector, sink));
            app.manage(AppState {
                service: Arc::clone(&service),
                persistent_secrets,
            });
            let (desktop, inbound) = Desktop::new(app.handle());
            app.manage(desktop);
            // Without an account there's nothing to watch in the background.
            let watching = service.list_accounts().is_ok_and(|a| !a.is_empty());
            if !in_background || !watching {
                desktop::window::create_main(app.handle())?;
            }
            // Periodic sync, push and the resume detector; inside the
            // runtime, where their tasks live.
            tauri::async_runtime::block_on(async { service.start_background() });
            desktop::start(app.handle(), inbound);
            Ok(())
        })
        .on_window_event(desktop::window::on_window_event)
        .invoke_handler(tauri::generate_handler![
            commands::get_app_info,
            commands::open_link,
            commands::quit_app,
            commands::quit_canceled,
            // accounts & settings
            commands::list_accounts,
            commands::add_imap_account,
            commands::add_microsoft_account,
            commands::cancel_microsoft_sign_in,
            commands::update_imap_account,
            commands::update_account,
            commands::remove_account,
            commands::get_settings,
            commands::update_settings,
            commands::list_fonts,
            commands::sync_account,
            commands::sync_all,
            commands::get_sync_statuses,
            commands::get_push_statuses,
            // desktop
            commands::get_background_state,
            commands::set_run_in_background,
            commands::set_start_at_login,
            commands::send_test_notification,
            commands::take_pending_navigation,
            // reminders
            commands::list_active_reminders,
            commands::snooze_reminder,
            commands::dismiss_reminders,
            // signatures
            commands::get_account_signature,
            commands::update_own_signature,
            commands::check_organization_signature,
            commands::get_compose_signature,
            // mail
            commands::list_folders,
            commands::list_messages,
            commands::search_messages,
            commands::get_message,
            commands::set_read,
            commands::set_flagged,
            commands::move_messages,
            commands::archive_messages,
            commands::delete_messages,
            commands::send_message,
            commands::suggest_addresses,
            commands::save_attachment,
            commands::open_attachment,
            // calendar
            commands::list_calendars,
            commands::list_events,
            commands::refresh_events,
            commands::create_event,
            commands::update_event,
            commands::delete_event,
            commands::respond_to_event,
            // contacts
            commands::list_contacts,
            commands::create_contact,
            commands::update_contact,
            commands::delete_contact,
            // tasks
            commands::list_task_lists,
            commands::list_tasks,
            commands::create_task,
            commands::update_task,
            commands::delete_task,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Cumulas Mail")
        .run(|app, event| match event {
            // The last window closed (it's hidden instead while the app
            // stays in the background, and none exists after a start at
            // login): keep running when that's what the user chose.
            // `app.exit` (Quit) comes with a code and always exits.
            RunEvent::ExitRequested {
                code: None, api, ..
            } if app
                .try_state::<Desktop>()
                .is_some_and(|d| d.stays_alive(app)) =>
            {
                api.prevent_exit();
            }
            RunEvent::Exit => {
                if let Some(state) = app.try_state::<AppState>() {
                    // Close push connections (IMAP LOGOUT) and stop syncing.
                    let service = Arc::clone(&state.service);
                    tauri::async_runtime::block_on(async move {
                        let _ = tokio::time::timeout(SHUTDOWN_LIMIT, service.shutdown()).await;
                    });
                }
            }
            _ => {}
        });
}
