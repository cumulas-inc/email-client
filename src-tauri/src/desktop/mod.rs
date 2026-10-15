//! The desktop around the core: notifications and where clicking them
//! leads, the main window's lifecycle, the tray icon, running in the
//! background and starting at login, and the network monitor.

pub mod background;
#[cfg(target_os = "linux")]
mod network;
pub mod notify;
pub mod router;
pub mod tray;
pub mod window;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cumulas_core::model::{Account, FolderRole};
use cumulas_core::service::MailAlert;
use tauri::{AppHandle, Manager};
use tokio::sync::mpsc;

use self::background::Background;
use self::notify::{Action, MailToastOptions, Notifications, Toast, ToastKind, mail_toasts};
use self::router::Router;
use self::tray::Tray;
use self::window::WindowState;
use crate::AppState;

/// The app id: the Flatpak's, the desktop file's and the D-Bus names'.
pub const APP_ID: &str = "co.cumulas.Mail";

/// Inside a Flatpak sandbox.
pub fn is_flatpak() -> bool {
    Path::new("/.flatpak-info").exists()
}

/// The desktop's side of the app, kept as Tauri state.
pub struct Desktop {
    pub notifications: Notifications,
    pub router: Router,
    pub window: WindowState,
    pub tray: Tray,
    pub background: Background,
}

impl Desktop {
    /// The desktop state, and where notification clicks arrive (for
    /// [`start`]).
    pub fn new(app: &AppHandle) -> (Self, mpsc::UnboundedReceiver<notify::Invoked>) {
        let (clicks, invoked) = mpsc::unbounded_channel();
        let desktop = Self {
            notifications: Notifications::new(clicks),
            router: Router::default(),
            window: WindowState::default(),
            tray: Tray::default(),
            background: Background::new(app),
        };
        (desktop, invoked)
    }

    /// Whether closing the window keeps the app running: the setting is on,
    /// the system allows it, and there's an account to watch.
    pub fn stays_alive(&self, app: &AppHandle) -> bool {
        let Some(state) = app.try_state::<AppState>() else {
            return false;
        };
        let enabled = state
            .service
            .get_settings()
            .is_ok_and(|s| s.run_in_background);
        let watching = state
            .service
            .list_accounts()
            .is_ok_and(|accounts| !accounts.is_empty());
        enabled && watching && self.background.allowed() != Some(false)
    }

    /// The window was closed and the app keeps running: in a Flatpak, asks
    /// for that the first time (quitting if refused); then says so, once.
    pub fn closed_to_background(&self, app: &AppHandle) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let desktop = app.state::<Desktop>();
            if desktop.background.allowed().is_none() {
                let settings = app.state::<AppState>().service.get_settings();
                let (login, hidden) = settings
                    .map(|s| (s.start_at_login, s.start_hidden))
                    .unwrap_or((false, true));
                let allowed = background::request(&app, login, hidden)
                    .await
                    .is_ok_and(|(allowed, _)| allowed);
                background::announce(&app).await;
                if !allowed {
                    tracing::info!("not allowed to run in the background; quitting");
                    app.exit(0);
                    return;
                }
            }
            background_notice(&app).await;
        });
    }
}

/// Starts the desktop side once the core runs: notification clicks, the
/// tray, the network monitor and the background status.
pub fn start(app: &AppHandle, mut invoked: mpsc::UnboundedReceiver<notify::Invoked>) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(click) = invoked.recv().await {
            router::handle(&handle, click).await;
        }
    });
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        // Connects early, so that Settings knows how notifications go.
        let _ = handle.state::<Desktop>().notifications.notifier().await;
        background::reconcile_autostart(&handle).await;
        background::set_status(&handle).await;
        background::announce(&handle).await;
    });
    tray::mail_changed(app);
    tray::sync(app);
    tauri::async_runtime::spawn(tray::watch(app.clone()));
    #[cfg(target_os = "linux")]
    {
        let service = Arc::clone(&app.state::<AppState>().service);
        tauri::async_runtime::spawn(network::watch(service));
    }
}

/// Shows new mail the core decided to notify about: unless the user is
/// looking at the app, and leaving out messages read meanwhile.
pub async fn new_mail(app: &AppHandle, account: &Account, alert: MailAlert) {
    let desktop = app.state::<Desktop>();
    if desktop.window.in_front() {
        return;
    }
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let service = &state.service;
    let still_unread = |messages: &[cumulas_core::model::MessageSummary]| {
        messages
            .iter()
            .filter(|m| {
                service
                    .store()
                    .get_message_summary(&m.id)
                    .is_ok_and(|now| !now.is_read)
            })
            .cloned()
            .collect::<Vec<_>>()
    };
    let alert = match alert {
        MailAlert::Each(messages) => MailAlert::Each(still_unread(&messages)),
        MailAlert::Summary(messages) => MailAlert::Summary(still_unread(&messages)),
    };
    if alert.messages().is_empty() {
        return;
    }
    let settings = service.get_settings().unwrap_or_default();
    let options = MailToastOptions {
        preview: settings.notify_show_preview,
        sound: settings.notify_sound,
        can_archive: service
            .store()
            .find_folder_by_role(&account.id, FolderRole::Archive)
            .is_ok_and(|f| f.is_some()),
    };
    for (toast, messages) in mail_toasts(account, &alert, options) {
        if let Err(e) = desktop.notifications.show(&toast, messages).await {
            tracing::warn!("could not show a notification: {e}");
        }
    }
}

/// Says once that closing the window left the app running.
async fn background_notice(app: &AppHandle) {
    let store = app.state::<AppState>().service.store().clone();
    if store
        .app_value(background::NOTICE_KEY)
        .ok()
        .flatten()
        .is_some()
    {
        return;
    }
    let desktop = app.state::<Desktop>();
    let body = if desktop.tray.available() {
        "New mail still shows up here. To quit, use the Cumulas Mail icon in the top bar."
    } else {
        "New mail still shows up here. To quit, open Cumulas Mail and press Ctrl+Q."
    };
    let toast = Toast {
        id: "app:background".into(),
        title: "Still running in the background".into(),
        body: body.into(),
        kind: ToastKind::App,
        default_action: Action::ShowWindow,
        buttons: Vec::new(),
        sound: false,
    };
    match desktop.notifications.show(&toast, Vec::new()).await {
        Ok(()) => {
            if let Err(e) = store.set_app_value(background::NOTICE_KEY, "yes") {
                tracing::warn!("could not remember the background notice: {e}");
            }
        }
        Err(e) => tracing::info!("could not say the app runs in the background: {e}"),
    }
}

/// The notification Settings sends to show what one looks like.
pub async fn test_notification(app: &AppHandle) -> Result<(), String> {
    let sound = app
        .state::<AppState>()
        .service
        .get_settings()
        .map(|s| s.notify_sound)
        .unwrap_or(true);
    let toast = Toast {
        id: "app:test".into(),
        title: "Notifications are on".into(),
        body: "New mail will show up like this.".into(),
        kind: ToastKind::App,
        default_action: Action::ShowWindow,
        buttons: Vec::new(),
        sound,
    };
    app.state::<Desktop>()
        .notifications
        .show(&toast, Vec::new())
        .await
}

/// The desktop file notifications belong to: the Flatpak's, else the first
/// of the names packages install that exists, else the app id.
pub fn desktop_entry() -> notify::fdo::DesktopEntry {
    if is_flatpak() {
        let id = std::env::var("FLATPAK_ID").unwrap_or_else(|_| APP_ID.into());
        return notify::fdo::DesktopEntry {
            icon: id.clone(),
            id,
        };
    }
    find_desktop_entry(&data_dirs()).unwrap_or_else(|| notify::fdo::DesktopEntry {
        id: APP_ID.into(),
        icon: APP_ID.into(),
    })
}

fn data_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    let system = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    home.into_iter()
        .chain(system.split(':').map(PathBuf::from))
        .filter(|p| p.is_absolute())
        .collect()
}

/// The names the packages install the desktop file under: the app id
/// (Flatpak, distributions), the binary's name (Tauri's deb and rpm), the
/// product name (older Tauri bundles).
const DESKTOP_IDS: &[&str] = &[APP_ID, "cumulas-mail", "Cumulas Mail"];

pub(crate) fn find_desktop_entry(dirs: &[PathBuf]) -> Option<notify::fdo::DesktopEntry> {
    DESKTOP_IDS.iter().find_map(|id| {
        dirs.iter().find_map(|dir| {
            let text =
                std::fs::read_to_string(dir.join("applications").join(format!("{id}.desktop")))
                    .ok()?;
            let icon = text
                .lines()
                .find_map(|l| l.trim().strip_prefix("Icon="))
                .map(str::trim)
                .filter(|i| !i.is_empty())
                .unwrap_or(id);
            Some(notify::fdo::DesktopEntry {
                id: (*id).to_owned(),
                icon: icon.to_owned(),
            })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktop_entry_is_found_under_any_package_name() {
        let home = tempfile::tempdir().unwrap();
        let system = tempfile::tempdir().unwrap();
        let dirs = [home.path().to_owned(), system.path().to_owned()];
        assert_eq!(find_desktop_entry(&dirs), None);

        let apps = system.path().join("applications");
        std::fs::create_dir_all(&apps).unwrap();
        std::fs::write(
            apps.join("cumulas-mail.desktop"),
            "[Desktop Entry]\nName=Cumulas Mail\nIcon=cumulas-mail\n",
        )
        .unwrap();
        assert_eq!(
            find_desktop_entry(&dirs),
            Some(notify::fdo::DesktopEntry {
                id: "cumulas-mail".into(),
                icon: "cumulas-mail".into(),
            })
        );

        // The app id wins wherever it is.
        let local = home.path().join("applications");
        std::fs::create_dir_all(&local).unwrap();
        std::fs::write(
            local.join("co.cumulas.Mail.desktop"),
            "[Desktop Entry]\nName=Cumulas Mail\n",
        )
        .unwrap();
        assert_eq!(
            find_desktop_entry(&dirs),
            Some(notify::fdo::DesktopEntry {
                id: APP_ID.into(),
                icon: APP_ID.into(),
            })
        );
    }
}
