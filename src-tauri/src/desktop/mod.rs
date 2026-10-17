//! The desktop around the core: notifications and where clicking them
//! leads, the main window's lifecycle, the tray icon, running in the
//! background and starting at login, the network monitor, and updates of
//! the Flatpak.

pub mod background;
#[cfg(target_os = "linux")]
mod network;
pub mod notify;
pub mod router;
pub mod tray;
pub mod update;
pub mod window;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use cumulas_core::model::{Account, ActiveReminder, FolderRole, Settings};
use cumulas_core::service::MailAlert;
use tauri::{AppHandle, Manager};
use tokio::sync::mpsc;
use tokio::time::Instant;

use self::background::Background;
use self::notify::{
    Action, MailToastOptions, Notifications, ReminderToastOptions, Toast, ToastKind, mail_toasts,
    reminder_toasts,
};
use self::router::Router;
use self::tray::Tray;
use self::update::Updates;
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
    pub update: Updates,
    /// Reminder changes, carried out one after another by [`start`] (see
    /// [`serve_reminders`]).
    reminders: mpsc::UnboundedSender<ReminderChange>,
}

/// Reminders that came due, and keys of those no longer due.
type ReminderChange = (Vec<ActiveReminder>, Vec<String>);

/// What [`start`] serves: clicks on notifications, and reminder changes
/// (those from before it started wait for it).
pub struct Inbound {
    invoked: mpsc::UnboundedReceiver<notify::Invoked>,
    reminders: mpsc::UnboundedReceiver<ReminderChange>,
}

/// How often reminders whose notification didn't show are shown again, and
/// for how long: the notification service may be restarting, or not reached
/// yet (it's tried again a minute after it wasn't), and reminders are about
/// now.
const RETRY_REMINDERS: Duration = Duration::from_secs(30);
const RETRY_REMINDERS_FOR: Duration = Duration::from_secs(5 * 60);

impl Desktop {
    /// The desktop state, and what [`start`] serves.
    pub fn new(app: &AppHandle) -> (Self, Inbound) {
        let (clicks, invoked) = mpsc::unbounded_channel();
        let (reminders, changes) = mpsc::unbounded_channel();
        let desktop = Self {
            notifications: Notifications::new(clicks),
            router: Router::default(),
            window: WindowState::default(),
            tray: Tray::default(),
            background: Background::new(app),
            update: Updates::default(),
            reminders,
        };
        let inbound = Inbound {
            invoked,
            reminders: changes,
        };
        (desktop, inbound)
    }

    /// The reminders changed: their notifications follow, in the order the
    /// changes came (a reminder shown and dismissed at once mustn't be
    /// withdrawn before it's shown).
    pub fn reminders_changed(&self, newly_due: Vec<ActiveReminder>, cleared: Vec<String>) {
        let _ = self.reminders.send((newly_due, cleared));
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

/// Starts the desktop side once the core runs: notification clicks,
/// reminders, the tray, the network monitor, the background status and
/// the update monitor.
pub fn start(app: &AppHandle, inbound: Inbound) {
    let handle = app.clone();
    let show = move |due: Vec<ActiveReminder>, cleared: Vec<String>, now: DateTime<Utc>| {
        let handle = handle.clone();
        async move { reminders_changed(&handle, due, &cleared, now).await }
    };
    tauri::async_runtime::spawn(serve_reminders(inbound.reminders, Utc::now, show));
    let handle = app.clone();
    tauri::async_runtime::spawn(serve_clicks(inbound.invoked, move |click| {
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
    tauri::async_runtime::spawn(update::watch(app.clone()));
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

/// Carries out reminder changes in the order they came, with `show`, which
/// returns the reminders whose notification didn't show. Those are shown
/// again with the next change and every [`RETRY_REMINDERS`], until they
/// show, are no longer due, or are no longer news: the core shows a
/// reminder once.
async fn serve_reminders<F, Fut>(
    mut changes: mpsc::UnboundedReceiver<ReminderChange>,
    clock: impl Fn() -> DateTime<Utc>,
    show: F,
) where
    F: Fn(Vec<ActiveReminder>, Vec<String>, DateTime<Utc>) -> Fut,
    Fut: Future<Output = Vec<ActiveReminder>>,
{
    let mut unshown: Vec<Unshown> = Vec::new();
    loop {
        let retry = !unshown.is_empty();
        let (newly_due, cleared) = tokio::select! {
            change = changes.recv() => match change {
                Some(change) => change,
                None => return,
            },
            () = tokio::time::sleep(RETRY_REMINDERS), if retry => (Vec::new(), Vec::new()),
        };
        let now = clock();
        let awake = Instant::now();
        unshown.retain(|u| {
            let key = &u.reminder.key;
            if cleared.contains(key) || newly_due.iter().any(|n| &n.key == key) {
                return false;
            }
            let stale = u.stale(now, awake);
            if stale {
                tracing::warn!("gave up showing a reminder's notification");
            }
            !stale
        });
        let mut due: Vec<ActiveReminder> = unshown.iter().map(|u| u.reminder.clone()).collect();
        due.extend(newly_due);
        let failed = show(due, cleared, now).await;
        unshown = failed
            .into_iter()
            .map(|reminder| {
                let earlier = unshown.iter().find(|u| u.reminder.key == reminder.key);
                Unshown {
                    since: earlier.map_or(now, |u| u.since),
                    awake_since: earlier.map_or(awake, |u| u.awake_since),
                    reminder,
                }
            })
            .collect();
    }
}

/// A reminder whose notification didn't show yet, and since when: by the
/// wall clock, which runs on while the computer sleeps (tokio's timers
/// don't), and by tokio's, which isn't set back.
struct Unshown {
    reminder: ActiveReminder,
    since: DateTime<Utc>,
    awake_since: Instant,
}

impl Unshown {
    /// No longer news: its event is over (the core lists a reminder it
    /// couldn't show before then as missed; it doesn't show it), or it's
    /// been [`RETRY_REMINDERS_FOR`], asleep or not.
    fn stale(&self, now: DateTime<Utc>, awake: Instant) -> bool {
        let over = self.reminder.ends_at.is_some_and(|end| end <= now);
        let age = (now - self.since).to_std().unwrap_or_default();
        let awake_for = awake.duration_since(self.awake_since);
        over || age.max(awake_for) >= RETRY_REMINDERS_FOR
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
/// those due show. Unlike new mail's, they show while the window is in
/// front too: they're about now, and the window may not be the one in
/// front for long. (The header's bell lists them as well.) Returns the
/// reminders whose notification didn't show.
async fn reminders_changed(
    app: &AppHandle,
    due: Vec<ActiveReminder>,
    cleared: &[String],
    now: DateTime<Utc>,
) -> Vec<ActiveReminder> {
    let Some(desktop) = app.try_state::<Desktop>() else {
        return Vec::new();
    };
    let settings = app
        .try_state::<AppState>()
        .and_then(|state| state.service.get_settings().ok())
        .unwrap_or_default();
    let options = ReminderToastOptions {
        snooze_minutes: settings.default_snooze_minutes,
        sound: settings.notify_sound,
    };
    let now = now.with_timezone(&Local);
    show_reminders(&desktop.notifications, due, cleared, options, now).await
}

/// Withdraws the notifications of reminders no longer due, and shows those
/// of `due`; returns the reminders whose notification didn't show.
async fn show_reminders(
    notifications: &Notifications,
    due: Vec<ActiveReminder>,
    cleared: &[String],
    options: ReminderToastOptions,
    now: DateTime<Local>,
) -> Vec<ActiveReminder> {
    if !cleared.is_empty() {
        notifications.settle(cleared).await;
    }
    let mut unshown = Vec::new();
    for (toast, keys) in reminder_toasts(&due, options, now) {
        if let Err(e) = notifications.show(&toast, keys.clone()).await {
            tracing::warn!("could not show a reminder: {e}");
            unshown.extend(due.iter().filter(|r| keys.contains(&r.key)).cloned());
        }
    }
    unshown
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
    use std::sync::atomic::Ordering;

    use chrono::TimeZone;
    use cumulas_core::model::ReminderKind;

    use super::*;
    use crate::desktop::notify::tests::{Recording, reminder};

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

    /// The wall clock, which runs on while the computer sleeps, while
    /// tokio's (paused in these tests) stands still; and which can be set
    /// back.
    #[derive(Clone)]
    struct WallClock {
        base: DateTime<Utc>,
        awake_since: Instant,
        moved: Arc<std::sync::Mutex<chrono::Duration>>,
    }

    impl WallClock {
        fn new(base: DateTime<Local>) -> Self {
            Self {
                base: base.with_timezone(&Utc),
                awake_since: Instant::now(),
                moved: Arc::default(),
            }
        }

        fn now(&self) -> DateTime<Utc> {
            let awake = chrono::Duration::from_std(self.awake_since.elapsed()).unwrap();
            self.base + awake + *self.moved.lock().unwrap()
        }

        fn sleep(&self, how_long: chrono::Duration) {
            *self.moved.lock().unwrap() += how_long;
        }

        fn set_back(&self, by: chrono::Duration) {
            *self.moved.lock().unwrap() -= by;
        }
    }

    /// Serves reminder changes with `clock` onto a recording notification
    /// server: where changes go, the server, and the serving task.
    fn serving_reminders(
        clock: &WallClock,
    ) -> (
        mpsc::UnboundedSender<ReminderChange>,
        Arc<Recording>,
        tokio::task::JoinHandle<()>,
    ) {
        let recording = Arc::new(Recording::default());
        let (clicks, _) = mpsc::unbounded_channel();
        let notifications = Arc::new(Notifications::with_notifier(recording.clone(), clicks));
        let options = ReminderToastOptions {
            snooze_minutes: 5,
            sound: true,
        };
        let (changes, queue) = mpsc::unbounded_channel();
        let clock = clock.clone();
        let serving = tokio::spawn(serve_reminders(
            queue,
            move || clock.now(),
            move |due, cleared, now| {
                let notifications = Arc::clone(&notifications);
                async move {
                    let now = now.with_timezone(&Local);
                    show_reminders(&notifications, due, &cleared, options, now).await
                }
            },
        ));
        (changes, recording, serving)
    }

    #[tokio::test(start_paused = true)]
    async fn a_reminder_that_didn_t_show_shows_once_the_server_is_back() {
        let start = Local.with_ymd_and_hms(2026, 10, 15, 16, 45, 0).unwrap();
        let clock = WallClock::new(start);
        let (changes, recording, serving) = serving_reminders(&clock);
        let server_gone = |gone| recording.failing.store(gone, Ordering::SeqCst);
        let wait = tokio::time::sleep;
        let at = |minutes| Some(start + chrono::Duration::minutes(minutes));
        let standup = reminder(ReminderKind::Event, "Standup", at(15));
        let review = reminder(ReminderKind::Task, "Review", None);
        let lunch = reminder(ReminderKind::Task, "Lunch", None);

        // The notification server is gone (restarting, say) when two come
        // due, and one of them is dismissed meanwhile.
        server_gone(true);
        let due = vec![standup.clone(), review.clone()];
        changes.send((due, vec![])).unwrap();
        wait(Duration::from_secs(1)).await;
        changes.send((vec![], vec![review.key.clone()])).unwrap();
        wait(RETRY_REMINDERS * 2).await;
        assert!(recording.shown_ids().is_empty());

        // It's back: the one still due shows, once.
        server_gone(false);
        wait(RETRY_REMINDERS).await;
        let standup_id = notify::reminder_toast_id(&standup.key);
        assert_eq!(recording.shown_ids(), std::slice::from_ref(&standup_id));
        wait(RETRY_REMINDERS * 4).await;
        assert_eq!(recording.shown_ids(), std::slice::from_ref(&standup_id));

        // Gone for too long: given up on, even with the clock set back
        // meanwhile.
        server_gone(true);
        changes.send((vec![lunch], vec![])).unwrap();
        wait(Duration::from_secs(1)).await;
        clock.set_back(chrono::Duration::hours(1));
        wait(RETRY_REMINDERS_FOR + RETRY_REMINDERS).await;
        server_gone(false);
        changes.send((vec![], vec![])).unwrap();
        wait(RETRY_REMINDERS * 2).await;
        assert_eq!(recording.shown_ids(), [standup_id]);

        drop(changes);
        serving.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_reminder_that_didn_t_show_before_the_computer_slept_doesn_t_after() {
        let start = Local.with_ymd_and_hms(2026, 10, 15, 16, 45, 0).unwrap();
        let clock = WallClock::new(start);
        let (changes, recording, serving) = serving_reminders(&clock);
        let server_gone = |gone| recording.failing.store(gone, Ordering::SeqCst);
        let wait = tokio::time::sleep;
        let at = |minutes| Some(start + chrono::Duration::minutes(minutes));

        // A call that ends in two minutes and a task come due while the
        // server is gone; the computer sleeps for three minutes. The call
        // is over by then (the core lists such a reminder as missed, it
        // doesn't show it), and the task still shows.
        let call = reminder(ReminderKind::Event, "Call", at(-28));
        let lunch = reminder(ReminderKind::Task, "Lunch", None);
        assert_eq!(call.ends_at, at(2).map(|end| end.with_timezone(&Utc)));
        server_gone(true);
        changes.send((vec![call, lunch.clone()], vec![])).unwrap();
        wait(Duration::from_secs(5)).await;
        clock.sleep(chrono::Duration::minutes(3));
        server_gone(false);
        wait(RETRY_REMINDERS).await;
        let lunch_id = notify::reminder_toast_id(&lunch.key);
        assert_eq!(recording.shown_ids(), std::slice::from_ref(&lunch_id));

        // A meeting coming up and a task come due while the server is
        // gone, and the computer sleeps until the next day: a few seconds
        // went by for tokio, but neither is news by now.
        server_gone(true);
        let standup = reminder(ReminderKind::Event, "Standup", at(20));
        let review = reminder(ReminderKind::Task, "Review", None);
        changes.send((vec![standup, review], vec![])).unwrap();
        wait(Duration::from_secs(5)).await;
        clock.sleep(chrono::Duration::hours(16));
        server_gone(false);
        wait(RETRY_REMINDERS * 2).await;
        changes.send((vec![], vec![])).unwrap();
        wait(RETRY_REMINDERS * 2).await;
        assert_eq!(recording.shown_ids(), std::slice::from_ref(&lunch_id));

        // Time awake and asleep add up: a meeting's reminder that failed
        // for two minutes awake and four asleep is given up on, though the
        // meeting hasn't started.
        server_gone(true);
        let planning = reminder(ReminderKind::Event, "Planning", at(16 * 60 + 120));
        changes.send((vec![planning], vec![])).unwrap();
        wait(RETRY_REMINDERS * 4).await;
        clock.sleep(chrono::Duration::minutes(4));
        server_gone(false);
        wait(RETRY_REMINDERS).await;
        assert_eq!(recording.shown_ids(), [lunch_id]);

        drop(changes);
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
