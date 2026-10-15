//! Tauri shell: wires [`CoreService`] to the webview through commands and
//! events, and runs periodic background sync.

mod commands;
mod fonts;

use std::sync::Arc;
use std::time::Duration;

use cumulas_core::CoreService;
use cumulas_core::model::{Account, DataKind, MessageSummary, SyncStatus};
use cumulas_core::secrets::{KeyringSecretStore, MemorySecretStore, SecretStore};
use cumulas_core::service::{EventSink, LiveConnector, MailAlert};
use cumulas_core::store::Store;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

pub(crate) struct AppState {
    pub service: Arc<CoreService>,
    /// False when no Secret Service was reachable and secrets only live in
    /// memory for this session.
    pub persistent_secrets: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DataChanged<'a> {
    kinds: &'a [DataKind],
    account_id: Option<&'a str>,
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
    }

    fn sync_status(&self, status: &SyncStatus) {
        let _ = self.app.emit("sync-status", status);
    }

    /// The core decided these should be shown (the alert policy).
    fn new_mail(&self, account: &Account, alert: &MailAlert) {
        let show_preview = self
            .app
            .try_state::<AppState>()
            .and_then(|s| s.service.get_settings().ok())
            .is_none_or(|s| s.notify_show_preview);
        let toasts: Vec<(String, String)> = match alert {
            MailAlert::Each(messages) if show_preview => messages
                .iter()
                .map(|m| (sender(m, account), m.subject.clone()))
                .collect(),
            MailAlert::Each(messages) => messages
                .iter()
                .map(|_| ("New message".to_owned(), account.email.clone()))
                .collect(),
            MailAlert::Summary(messages) => vec![(
                format!("{} new messages", messages.len()),
                format!("in {}", account.email),
            )],
        };
        for (title, body) in toasts {
            if let Err(e) = self
                .app
                .notification()
                .builder()
                .title(title)
                .body(body)
                .show()
            {
                tracing::warn!("could not show notification: {e}");
            }
        }
    }
}

/// Who a message is from, for a notification title.
fn sender(message: &MessageSummary, account: &Account) -> String {
    message
        .from
        .as_ref()
        .map(|f| f.name.clone().unwrap_or_else(|| f.address.clone()))
        .unwrap_or_else(|| account.display_name.clone())
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

fn spawn_background_sync(service: Arc<CoreService>) {
    tauri::async_runtime::spawn(async move {
        // Give the window a moment to render before the first sync.
        tokio::time::sleep(Duration::from_secs(2)).await;
        loop {
            service.sync_all().await;
            let minutes = service
                .get_settings()
                .map(|s| s.sync_interval_minutes)
                .unwrap_or(5)
                .max(1);
            tokio::time::sleep(Duration::from_secs(u64::from(minutes) * 60)).await;
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,cumulas_core=debug".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let store = Arc::new(Store::open(&data_dir.join("mail.db"))?);
            let (secrets, persistent_secrets) = tauri::async_runtime::block_on(open_secret_store());
            let connector = Arc::new(LiveConnector::new(Arc::clone(&store), Arc::clone(&secrets)));
            let sink = Arc::new(TauriSink {
                app: app.handle().clone(),
            });
            let service = Arc::new(CoreService::new(store, secrets, connector, sink));
            // The UI sets the zoom when the text size changes; setting the
            // saved one now means the first frame is already the right size.
            if let (Ok(settings), Some(window)) =
                (service.get_settings(), app.get_webview_window("main"))
            {
                let zoom = settings.text_size.zoom();
                if let Err(e) = window.set_zoom(zoom) {
                    tracing::warn!("could not set the zoom to {zoom}: {e}");
                }
            }
            spawn_background_sync(Arc::clone(&service));
            app.manage(AppState {
                service,
                persistent_secrets,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_info,
            commands::open_link,
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
        .run(tauri::generate_context!())
        .expect("error while running Cumulas Mail");
}
