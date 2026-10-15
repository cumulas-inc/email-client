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

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use chrono::{DateTime, Local};
use cumulas_core::model::{Account, ActiveReminder, FolderRole, Settings};
use cumulas_core::service::MailAlert;
use tauri::{AppHandle, Manager};
use tokio::sync::mpsc;

use self::background::Background;
use self::notify::{
    Action, MailToastOptions, Notifications, ReminderToastOptions, Toast, ToastKind, mail_toasts,
    reminder_toasts,
};
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
    /// Reminder changes, carried out one after another (see
    /// [`Desktop::reminders_changed`]); set by [`start`].
    reminders: OnceLock<mpsc::UnboundedSender<ReminderChange>>,
}

/// Reminders that came due, and keys of those no longer due.
type ReminderChange = (Vec<ActiveReminder>, Vec<String>);

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
            reminders: OnceLock::new(),
        };
        (desktop, invoked)
    }

    /// The reminders changed: their notifications follow, in the order the
    /// changes came (a reminder shown and dismissed at once mustn't be
    /// withdrawn before it's shown).
    pub fn reminders_changed(
        &self,
        app: &AppHandle,
        newly_due: Vec<ActiveReminder>,
        cleared: Vec<String>,
    ) {
        if let Some(queue) = self.reminders.get() {
            let _ = queue.send((newly_due, cleared));
            return;
        }
        // Before the desktop side started.
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            reminders_changed(&app, newly_due, cleared).await;
        });
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
                    .is_ok_and(background::Answer::background);
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
pub fn start(app: &AppHandle, invoked: mpsc::UnboundedReceiver<notify::Invoked>) {
    let (queue, mut changes) = mpsc::unbounded_channel::<ReminderChange>();
    if app.state::<Desktop>().reminders.set(queue).is_ok() {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            while let Some((newly_due, cleared)) = changes.recv().await {
                reminders_changed(&handle, newly_due, cleared).await;
            }
        });
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(serve_clicks(invoked, move |click| {
        let handle = handle.clone();
        async move { router::handle(&handle, click).await }
    }));
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        // Connects early, so that Settings knows how notifications go.
        let _ = handle.state::<Desktop>().notifications.notifier().await;
        background::reconcile_autostart(&handle).await;
        background::check_again(&handle).await;
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

/// Carries out clicks as they come. Marking read, archiving, snoozing,
/// dismissing and completing wait on the server, so each runs on its own
/// and no other click waits for them; opening and showing wait on nothing,
/// and stay in order.
async fn serve_clicks<F>(
    mut invoked: mpsc::UnboundedReceiver<notify::Invoked>,
    handle: impl Fn(notify::Invoked) -> F,
) where
    F: Future<Output = ()> + Send + 'static,
{
    while let Some(click) = invoked.recv().await {
        if click.action.is_slow() {
            tauri::async_runtime::spawn(handle(click));
        } else {
            handle(click).await;
        }
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

/// The reminders changed: the notifications of those no longer due go, and
/// those that just came due show. Unlike new mail's, they show while the
/// window is in front too: they're about now, and the window may not be
/// the one in front for long. (The header's bell lists them as well.)
async fn reminders_changed(app: &AppHandle, newly_due: Vec<ActiveReminder>, cleared: Vec<String>) {
    let Some(desktop) = app.try_state::<Desktop>() else {
        return;
    };
    if !cleared.is_empty() {
        desktop.notifications.settle(&cleared).await;
    }
    if newly_due.is_empty() {
        return;
    }
    let settings = app
        .try_state::<AppState>()
        .and_then(|state| state.service.get_settings().ok())
        .unwrap_or_default();
    let options = ReminderToastOptions {
        snooze_minutes: settings.default_snooze_minutes,
        sound: settings.notify_sound,
    };
    for (toast, keys) in reminder_toasts(&newly_due, options, Local::now()) {
        if let Err(e) = desktop.notifications.show(&toast, keys).await {
            tracing::warn!("could not show a reminder: {e}");
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
    let tray = app
        .state::<AppState>()
        .service
        .get_settings()
        .is_ok_and(|s| desktop.tray.shown(s.show_tray_icon));
    let toast = Toast {
        id: "app:background".into(),
        title: "Still running in the background".into(),
        body: background_notice_body(tray).into(),
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

/// Where to quit from: the tray icon when it shows (wherever the desktop
/// puts it), else the window's main menu.
fn background_notice_body(tray: bool) -> &'static str {
    if tray {
        "New mail still shows up here. To quit, use the Cumulas Mail tray icon."
    } else {
        "New mail still shows up here. To quit, open Cumulas Mail and choose Quit in the main menu."
    }
}

/// The notification Settings sends to show what one looks like.
pub async fn test_notification(app: &AppHandle) -> Result<(), String> {
    let settings = app
        .state::<AppState>()
        .service
        .get_settings()
        .unwrap_or_default();
    app.state::<Desktop>()
        .notifications
        .show(&test_toast(&settings, Local::now()), Vec::new())
        .await
}

/// The test notification: true whatever the settings, so it says when new
/// mail wouldn't show up like this.
fn test_toast(settings: &Settings, now: DateTime<Local>) -> Toast {
    let paused = settings
        .notifications_paused_until
        .map(|until| until.with_timezone(&Local))
        .filter(|until| *until > now);
    let body = if !settings.notifications_enabled {
        "New message notifications are off, so new mail won't show up like this.".to_owned()
    } else if let Some(until) = paused {
        format!(
            "New message notifications are paused until {}.",
            until_label(until, now)
        )
    } else {
        "New message notifications are on.".to_owned()
    };
    Toast {
        id: "app:test".into(),
        title: "Test notification".into(),
        body,
        kind: ToastKind::App,
        default_action: Action::ShowWindow,
        buttons: Vec::new(),
        sound: settings.notify_sound,
    }
}

/// "9:30 AM", "tomorrow at 8:00 AM" or "Oct 17 at 8:00 AM", as Settings
/// says it.
fn until_label(until: DateTime<Local>, now: DateTime<Local>) -> String {
    let time = until.format("%-I:%M %p");
    let days = (until.date_naive() - now.date_naive()).num_days();
    match days {
        0 => time.to_string(),
        1 => format!("tomorrow at {time}"),
        _ => format!("{} at {time}", until.format("%b %-d")),
    }
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
    use chrono::TimeZone;

    use super::*;

    #[tokio::test]
    async fn a_slow_mark_as_read_holds_up_no_other_click() {
        let (clicks, invoked) = mpsc::unbounded_channel();
        let (handled, mut done) = mpsc::unbounded_channel();
        let (release, server) = tokio::sync::watch::channel(false);
        let serving = tokio::spawn(serve_clicks(invoked, move |click: notify::Invoked| {
            let (handled, mut server) = (handled.clone(), server.clone());
            async move {
                if matches!(click.action, Action::MarkRead(_) | Action::Archive(_)) {
                    // The server doesn't answer until released.
                    let _ = server.wait_for(|answered| *answered).await;
                }
                let _ = handled.send(click.action);
            }
        }));
        let click = |action| notify::Invoked {
            action,
            activation_token: None,
        };
        for action in [
            Action::MarkRead("a".into()),
            Action::Archive("b".into()),
            Action::OpenMessage("c".into()),
            Action::ShowWindow,
        ] {
            assert_eq!(
                action.is_slow(),
                matches!(action, Action::MarkRead(_) | Action::Archive(_))
            );
            clicks.send(click(action)).unwrap();
        }
        async fn next(done: &mut mpsc::UnboundedReceiver<Action>) -> Action {
            let next = tokio::time::timeout(std::time::Duration::from_secs(5), done.recv());
            next.await.expect("a click handled").unwrap()
        }
        // Opening and showing go first, while the server doesn't answer.
        assert_eq!(next(&mut done).await, Action::OpenMessage("c".into()));
        assert_eq!(next(&mut done).await, Action::ShowWindow);
        release.send(true).unwrap();
        let mut slow = vec![next(&mut done).await, next(&mut done).await];
        slow.sort_by_key(Action::encode);
        assert_eq!(
            slow,
            [Action::Archive("b".into()), Action::MarkRead("a".into())]
        );
        drop(clicks);
        serving.await.unwrap();
    }

    #[test]
    fn the_background_notice_says_where_to_quit() {
        let tray = background_notice_body(true);
        assert!(tray.contains("Cumulas Mail tray icon"), "{tray}");
        let menu = background_notice_body(false);
        assert!(menu.contains("Quit in the main menu"), "{menu}");
        for body in [tray, menu] {
            assert!(
                !body.contains("Ctrl+Q") && !body.contains("top bar"),
                "{body}"
            );
        }
    }

    #[test]
    fn the_test_notification_says_when_new_mail_wouldnt_show() {
        let now = Local.with_ymd_and_hms(2026, 10, 15, 22, 30, 0).unwrap();
        let on = Settings::default();
        assert!(on.notifications_enabled);
        let toast = test_toast(&on, now);
        assert_eq!(toast.title, "Test notification");
        assert_eq!(toast.body, "New message notifications are on.");
        assert_eq!(toast.sound, on.notify_sound);

        let off = Settings {
            notifications_enabled: false,
            ..Settings::default()
        };
        assert_eq!(
            test_toast(&off, now).body,
            "New message notifications are off, so new mail won't show up like this."
        );

        let paused = |until: DateTime<Local>| Settings {
            notifications_paused_until: Some(until.with_timezone(&chrono::Utc)),
            ..Settings::default()
        };
        let tomorrow = Local.with_ymd_and_hms(2026, 10, 16, 8, 0, 0).unwrap();
        assert_eq!(
            test_toast(&paused(tomorrow), now).body,
            "New message notifications are paused until tomorrow at 8:00 AM."
        );
        let soon = Local.with_ymd_and_hms(2026, 10, 15, 23, 15, 0).unwrap();
        assert_eq!(
            test_toast(&paused(soon), now).body,
            "New message notifications are paused until 11:15 PM."
        );
        let later = Local.with_ymd_and_hms(2026, 10, 17, 8, 0, 0).unwrap();
        assert_eq!(until_label(later, now), "Oct 17 at 8:00 AM");
        // A pause that ended doesn't count.
        let ended = Local.with_ymd_and_hms(2026, 10, 15, 21, 0, 0).unwrap();
        assert_eq!(
            test_toast(&paused(ended), now).body,
            "New message notifications are on."
        );
    }

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
