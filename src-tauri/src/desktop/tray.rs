//! The tray icon: shown when the setting is on and the desktop can show it.
//!
//! The icon needs a StatusNotifier host (KDE, most other desktops, GNOME
//! with the AppIndicator extension) and the libayatana-appindicator (or
//! libappindicator) library, which tray-icon loads at run time and panics
//! without. So the library is looked for first, and the host followed on
//! the session bus: the icon comes and goes with it.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use chrono::{Duration, Local, NaiveTime, TimeZone, Utc};
use cumulas_core::model::{DataKind, FolderRole};
use cumulas_core::push::WakeReason;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager};

use super::router::Navigate;
use super::{Desktop, window};
use crate::AppState;

const TRAY_ID: &str = "main";
/// The StatusNotifier host's bus name.
#[cfg(target_os = "linux")]
const WATCHER: &str = "org.kde.StatusNotifierWatcher";

#[derive(Default)]
pub struct Tray {
    /// The library loads (checked once).
    library: std::sync::OnceLock<bool>,
    /// A StatusNotifier host is running.
    host: AtomicBool,
    unread: AtomicU32,
    items: Mutex<Option<TrayItems>>,
}

/// The menu items that change.
struct TrayItems {
    unread: MenuItem<tauri::Wry>,
    resume: MenuItem<tauri::Wry>,
}

/// Whether to show the icon: the setting is on, there's a host, and the
/// library loads. The library is only looked for when it's needed.
pub(crate) fn wanted(enabled: bool, host: bool, library: impl FnOnce() -> bool) -> bool {
    enabled && host && library()
}

impl Tray {
    /// A tray icon can be shown, whatever the setting.
    pub fn available(&self) -> bool {
        self.host.load(Ordering::Relaxed) && self.library()
    }

    fn library(&self) -> bool {
        *self.library.get_or_init(library_loads)
    }
}

/// Whether tray-icon will find its library (it would panic otherwise).
#[cfg(target_os = "linux")]
fn library_loads() -> bool {
    let found = ["libayatana-appindicator3.so.1", "libappindicator3.so.1"]
        .iter()
        // SAFETY: these libraries run no code of interest when loaded; the
        // same ones are loaded by tray-icon right after.
        .any(|name| unsafe { libloading::Library::new(name) }.is_ok());
    if !found {
        tracing::info!("no tray icon: libayatana-appindicator3 isn't installed");
    }
    found
}

#[cfg(not(target_os = "linux"))]
fn library_loads() -> bool {
    false
}

/// Follows the StatusNotifier host, adding or removing the icon as it comes
/// and goes.
#[cfg(target_os = "linux")]
pub async fn watch(app: AppHandle) {
    use futures::StreamExt;

    let follow = async {
        let connection = zbus::Connection::session().await?;
        let bus = zbus::fdo::DBusProxy::new(&connection).await?;
        let mut changes = bus
            .receive_name_owner_changed_with_args(&[(0, WATCHER)])
            .await?;
        let name = zbus::names::BusName::try_from(WATCHER)?;
        let present = bus.name_has_owner(name).await.unwrap_or(false);
        host_changed(&app, present).await;
        while let Some(change) = changes.next().await {
            if let Ok(args) = change.args() {
                host_changed(&app, args.new_owner().is_some()).await;
            }
        }
        Ok::<_, zbus::Error>(())
    };
    if let Err(e) = follow.await {
        tracing::info!("no tray icon: can't follow the StatusNotifier host ({e})");
    }
}

#[cfg(not(target_os = "linux"))]
pub async fn watch(_app: AppHandle) {}

async fn host_changed(app: &AppHandle, present: bool) {
    let desktop = app.state::<Desktop>();
    let was = desktop.tray.host.swap(present, Ordering::Relaxed);
    if was == present {
        return;
    }
    tracing::info!(
        "StatusNotifier host {}",
        if present { "available" } else { "gone" }
    );
    sync(app);
    super::background::announce(app).await;
}

/// Adds or removes the icon to match the setting and the desktop.
pub fn sync(app: &AppHandle) {
    let desktop = app.state::<Desktop>();
    let enabled = app
        .state::<AppState>()
        .service
        .get_settings()
        .map(|s| s.show_tray_icon)
        .unwrap_or(true);
    let show = wanted(enabled, desktop.tray.host.load(Ordering::Relaxed), || {
        desktop.tray.library()
    });
    let handle = app.clone();
    let result = app.run_on_main_thread(move || {
        let present = handle.tray_by_id(TRAY_ID).is_some();
        if show && !present {
            if let Err(e) = build(&handle) {
                tracing::warn!("could not add the tray icon: {e}");
            }
        } else if !show && present {
            handle.remove_tray_by_id(TRAY_ID);
            *handle.state::<Desktop>().tray.items.lock().unwrap() = None;
        }
    });
    if let Err(e) = result {
        tracing::warn!("could not update the tray icon: {e}");
    }
}

fn icon(unread: u32) -> Image<'static> {
    if unread > 0 {
        tauri::include_image!("icons/tray-unread.png")
    } else {
        tauri::include_image!("icons/tray.png")
    }
}

fn unread_label(unread: u32) -> String {
    match unread {
        0 => "No unread messages".into(),
        1 => "1 unread message".into(),
        n => format!("{n} unread messages"),
    }
}

/// On the main thread.
fn build(app: &AppHandle) -> tauri::Result<()> {
    let desktop = app.state::<Desktop>();
    let unread = desktop.tray.unread.load(Ordering::Relaxed);
    let paused = paused_until(app).is_some();

    let unread_item = MenuItem::with_id(app, "unread", unread_label(unread), false, None::<&str>)?;
    let resume = MenuItem::with_id(app, "pause-resume", "Resume", paused, None::<&str>)?;
    let pause = Submenu::with_items(
        app,
        "Pause notifications",
        true,
        &[
            &MenuItem::with_id(app, "pause-hour", "For 1 hour", true, None::<&str>)?,
            &MenuItem::with_id(app, "pause-tomorrow", "Until tomorrow", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &resume,
        ],
    )?;
    let menu = Menu::with_items(
        app,
        &[
            &unread_item,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "open", "Open Cumulas Mail", true, None::<&str>)?,
            &MenuItem::with_id(app, "compose", "New message", true, None::<&str>)?,
            &MenuItem::with_id(app, "check", "Check for new mail", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &pause,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", "Quit Cumulas Mail", true, None::<&str>)?,
        ],
    )?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon(unread))
        .tooltip(tooltip(unread))
        .menu(&menu)
        .on_menu_event(|app, event| on_menu(app, event.id.as_ref()));
    // In the sandbox, the icon must be written where the host can read it.
    if super::is_flatpak()
        && let (Some(runtime), Ok(id)) = (
            std::env::var_os("XDG_RUNTIME_DIR"),
            std::env::var("FLATPAK_ID"),
        )
    {
        builder = builder.temp_dir_path(
            std::path::PathBuf::from(runtime)
                .join("app")
                .join(id)
                .join("tray-icon"),
        );
    }
    builder.build(app)?;
    *desktop.tray.items.lock().unwrap() = Some(TrayItems {
        unread: unread_item,
        resume,
    });
    Ok(())
}

/// Hosts that show tooltips show the unread count; the menu's first line
/// shows it everywhere (appindicator has no tooltips).
fn tooltip(unread: u32) -> String {
    format!("Cumulas Mail — {}", unread_label(unread).to_lowercase())
}

/// Unread mail in the Inboxes changed, maybe: updates the icon and menu.
pub fn mail_changed(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let unread: u32 = match state.service.list_folders(None) {
        Ok(folders) => folders
            .iter()
            .filter(|f| f.role == FolderRole::Inbox)
            .map(|f| f.unread_count)
            .sum(),
        Err(_) => return,
    };
    let desktop = app.state::<Desktop>();
    let before = desktop.tray.unread.swap(unread, Ordering::Relaxed);
    if before == unread {
        return;
    }
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(tray) = handle.tray_by_id(TRAY_ID) else {
            return;
        };
        if (before == 0) != (unread == 0) {
            let _ = tray.set_icon(Some(icon(unread)));
        }
        let _ = tray.set_tooltip(Some(tooltip(unread)));
        if let Some(items) = &*handle.state::<Desktop>().tray.items.lock().unwrap() {
            let _ = items.unread.set_text(unread_label(unread));
        }
    });
}

/// Notifications were paused or resumed: enables "Resume" to match.
pub fn pause_changed(app: &AppHandle) {
    let paused = paused_until(app).is_some();
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(items) = &*handle.state::<Desktop>().tray.items.lock().unwrap() {
            let _ = items.resume.set_enabled(paused);
        }
    });
}

fn paused_until(app: &AppHandle) -> Option<chrono::DateTime<Utc>> {
    app.state::<AppState>()
        .service
        .get_settings()
        .ok()?
        .notifications_paused_until
        .filter(|until| *until > Utc::now())
}

/// Tomorrow at 8 in the morning, local time.
pub(crate) fn tomorrow_morning(now: chrono::DateTime<Local>) -> chrono::DateTime<Utc> {
    let date = now.date_naive() + Duration::days(1);
    let morning = date.and_time(NaiveTime::from_hms_opt(8, 0, 0).unwrap_or_default());
    Local
        .from_local_datetime(&morning)
        .earliest()
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|| (now + Duration::days(1)).with_timezone(&Utc))
}

/// Tray menu clicks, on the main thread.
fn on_menu(app: &AppHandle, id: &str) {
    match id {
        "open" => {
            window::show_main(app, None);
        }
        "compose" => {
            window::show_main(app, None);
            app.state::<Desktop>()
                .router
                .navigate(app, Navigate::Compose);
        }
        "check" => {
            app.state::<AppState>().service.wake(WakeReason::User);
        }
        "pause-hour" => pause(app, Some(Utc::now() + Duration::hours(1))),
        "pause-tomorrow" => pause(app, Some(tomorrow_morning(Local::now()))),
        "pause-resume" => pause(app, None),
        "quit" => app.exit(0),
        _ => {}
    }
}

fn pause(app: &AppHandle, until: Option<chrono::DateTime<Utc>>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let service = &app.state::<AppState>().service;
        let saved = match service.get_settings() {
            Ok(mut settings) => {
                settings.notifications_paused_until = until;
                service.update_settings(settings).await
            }
            Err(e) => Err(e),
        };
        match saved {
            Ok(_) => {
                pause_changed(&app);
                // Settings in the page follow.
                let _ = app.emit(
                    "data-changed",
                    crate::DataChanged {
                        kinds: &[DataKind::Accounts],
                        account_id: None,
                    },
                );
            }
            Err(e) => tracing::warn!("could not pause notifications: {e}"),
        }
    });
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use chrono::{Datelike, Timelike};

    use super::*;

    #[test]
    fn the_icon_needs_the_setting_a_host_and_the_library() {
        let probed = Cell::new(0);
        let library = |loads: bool| {
            let probed = &probed;
            move || {
                probed.set(probed.get() + 1);
                loads
            }
        };
        assert!(wanted(true, true, library(true)));
        assert!(!wanted(true, true, library(false)));
        assert_eq!(probed.get(), 2);
        // Without a host or the setting, the library isn't even looked for.
        assert!(!wanted(true, false, library(true)));
        assert!(!wanted(false, true, library(true)));
        assert_eq!(probed.get(), 2);
    }

    #[test]
    fn labels_count_unread_mail() {
        assert_eq!(unread_label(0), "No unread messages");
        assert_eq!(unread_label(1), "1 unread message");
        assert_eq!(unread_label(12), "12 unread messages");
        assert_eq!(tooltip(3), "Cumulas Mail — 3 unread messages");
    }

    #[test]
    fn until_tomorrow_means_tomorrow_morning() {
        let now = Local.with_ymd_and_hms(2026, 10, 15, 22, 30, 0).unwrap();
        let until = tomorrow_morning(now).with_timezone(&Local);
        assert_eq!((until.day(), until.hour(), until.minute()), (16, 8, 0));
    }
}
