//! Running without a window and starting at login.
//!
//! Inside a Flatpak both need the Background portal: it asks the user (or
//! the desktop decides), writes the autostart entry on the host, and may
//! refuse; a refused app is stopped when it has no window, so it then quits
//! when its window closes instead. A native install writes an XDG autostart
//! file itself.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use cumulas_core::model::Settings;
use cumulas_core::{Error, Result};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use super::notify::NotifierKind;
use super::{APP_ID, Desktop};
use crate::AppState;

/// Kept between runs: the Background portal's last answer.
const ALLOWED_KEY: &str = "shell.background_allowed";
/// Shown once: the notification saying the app still runs.
pub(super) const NOTICE_KEY: &str = "shell.background_notice_shown";
/// Why the app asks, for the portal's dialog.
const REASON: &str = "Show new mail while the window is closed";
/// Started at login without a window.
pub const BACKGROUND_ARG: &str = "--background";

/// How starting at login works here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AutostartBackend {
    /// The Background portal (Flatpak).
    Portal,
    /// A file in `~/.config/autostart`.
    XdgAutostartFile,
    Unsupported,
}

/// The Background portal's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// What it allows: running without a window, and starting at login.
    Portal { background: bool, autostart: bool },
    /// This desktop has no Background portal. Nothing stops an app without
    /// a window then (the portal does that), as outside a Flatpak, but
    /// nothing starts it at login either.
    NoPortal,
}

impl Answer {
    /// The app may run without a window.
    pub fn background(self) -> bool {
        match self {
            Answer::Portal { background, .. } => background,
            Answer::NoPortal => true,
        }
    }
}

/// What Settings shows about running in the background.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundState {
    pub run_in_background: bool,
    /// Whether the system lets the app run without a window: always outside
    /// a Flatpak; inside, the Background portal's answer, `None` until it
    /// was asked.
    pub background_allowed: Option<bool>,
    pub start_at_login: bool,
    pub start_hidden: bool,
    pub autostart: AutostartBackend,
    /// A tray icon can be shown (the library is installed and the desktop
    /// shows tray icons).
    pub tray_available: bool,
    /// How notifications reach the desktop; `None` when nothing answered.
    pub notifications: Option<NotifierKind>,
}

pub struct Background {
    backend: AutostartBackend,
    /// The portal's answer (Flatpak); `Some(true)` elsewhere.
    allowed: Mutex<Option<bool>>,
    /// The status last given to the Background portal.
    status: tokio::sync::Mutex<Option<String>>,
}

impl Background {
    pub fn new(app: &AppHandle) -> Self {
        let (backend, allowed) = if super::is_flatpak() {
            let stored = app
                .try_state::<AppState>()
                .and_then(|s| s.service.store().app_value(ALLOWED_KEY).ok().flatten());
            (AutostartBackend::Portal, stored.map(|v| v == "yes"))
        } else if cfg!(target_os = "linux") {
            (AutostartBackend::XdgAutostartFile, Some(true))
        } else {
            (AutostartBackend::Unsupported, Some(true))
        };
        Self {
            backend,
            allowed: Mutex::new(allowed),
            status: tokio::sync::Mutex::default(),
        }
    }

    pub fn allowed(&self) -> Option<bool> {
        *self.allowed.lock().unwrap()
    }

    fn remember_allowed(&self, app: &AppHandle, allowed: bool) {
        *self.allowed.lock().unwrap() = Some(allowed);
        if let Some(state) = app.try_state::<AppState>()
            && let Err(e) = state
                .service
                .store()
                .set_app_value(ALLOWED_KEY, if allowed { "yes" } else { "no" })
        {
            tracing::warn!("could not keep the background permission: {e}");
        }
    }
}

/// The state for Settings.
pub async fn state(app: &AppHandle) -> BackgroundState {
    let desktop = app.state::<Desktop>();
    let settings = settings(app).unwrap_or_default();
    BackgroundState {
        run_in_background: settings.run_in_background,
        background_allowed: desktop.background.allowed(),
        start_at_login: settings.start_at_login,
        start_hidden: settings.start_hidden,
        autostart: desktop.background.backend,
        tray_available: desktop.tray.available(),
        notifications: desktop.notifications.kind().await,
    }
}

/// Tells the page the state changed.
pub async fn announce(app: &AppHandle) {
    let state = state(app).await;
    if let Err(e) = app.emit("background-state", &state) {
        tracing::debug!("could not send the background state: {e}");
    }
}

fn settings(app: &AppHandle) -> Result<Settings> {
    app.state::<AppState>().service.get_settings()
}

async fn save(app: &AppHandle, change: impl FnOnce(&mut Settings)) -> Result<()> {
    let service = &app.state::<AppState>().service;
    let mut settings = service.get_settings()?;
    change(&mut settings);
    service.update_settings(settings).await?;
    Ok(())
}

/// Turns running in the background on or off. In a Flatpak, turning it on
/// asks the Background portal the first time.
pub async fn set_run_in_background(app: &AppHandle, enabled: bool) -> Result<BackgroundState> {
    let desktop = app.state::<Desktop>();
    if enabled && desktop.background.backend == AutostartBackend::Portal {
        let current = settings(app)?;
        request(app, current.start_at_login, current.start_hidden).await?;
    }
    save(app, |s| s.run_in_background = enabled).await?;
    announce(app).await;
    Ok(state(app).await)
}

/// Turns starting at login on or off; `hidden` starts without a window.
pub async fn set_start_at_login(
    app: &AppHandle,
    enabled: bool,
    hidden: bool,
) -> Result<BackgroundState> {
    let desktop = app.state::<Desktop>();
    match desktop.background.backend {
        AutostartBackend::Portal => match request(app, enabled, hidden).await? {
            Answer::Portal {
                autostart: false, ..
            } if enabled => {
                announce(app).await;
                return Err(Error::unsupported(
                    "Your system didn't allow Cumulas Mail to start at login. You can allow it \
                     in your system's settings for apps.",
                ));
            }
            Answer::NoPortal if enabled => {
                announce(app).await;
                return Err(Error::unsupported(
                    "Starting at login isn't available on this desktop.",
                ));
            }
            _ => {}
        },
        AutostartBackend::XdgAutostartFile => {
            let path = autostart_path(&config_home());
            if enabled {
                write_autostart(&path, &executable(), hidden)?;
            } else {
                remove_autostart(&path)?;
            }
        }
        AutostartBackend::Unsupported => {
            return Err(Error::unsupported(
                "Starting at login isn't available on this system.",
            ));
        }
    }
    save(app, |s| {
        s.start_at_login = enabled;
        s.start_hidden = hidden;
    })
    .await?;
    announce(app).await;
    Ok(state(app).await)
}

/// Asks the Background portal to let the app run without a window, and to
/// start it at login when `autostart`; returns what it allowed and keeps it.
#[cfg(target_os = "linux")]
pub async fn request(app: &AppHandle, autostart: bool, hidden: bool) -> Result<Answer> {
    use ashpd::desktop::ResponseError;
    use ashpd::desktop::background::Background as Portal;

    let mut command = vec!["cumulas-mail"];
    if hidden {
        command.push(BACKGROUND_ARG);
    }
    let response = Portal::request()
        .reason(REASON)
        .auto_start(autostart)
        .command(command)
        .dbus_activatable(false)
        .send()
        .await
        .and_then(|request| request.response());
    let (background, autostart) = match response {
        Ok(answer) => (answer.run_in_background(), answer.auto_start()),
        // How the portal refuses.
        Err(ashpd::Error::Response(ResponseError::Cancelled)) => (false, false),
        Err(e) if no_portal(&e) => {
            // Only kept for this run: another desktop may have the portal.
            tracing::info!("no Background portal; running without a window as natively ({e})");
            *app.state::<Desktop>().background.allowed.lock().unwrap() = Some(true);
            return Ok(Answer::NoPortal);
        }
        Err(e) => {
            tracing::warn!("the Background portal didn't answer: {e}");
            return Err(Error::unsupported(
                "Your system didn't answer the request to run in the background.",
            ));
        }
    };
    tracing::info!("background: allowed {background}, at login {autostart}");
    app.state::<Desktop>()
        .background
        .remember_allowed(app, background);
    Ok(Answer::Portal {
        background,
        autostart,
    })
}

#[cfg(not(target_os = "linux"))]
pub async fn request(_app: &AppHandle, _autostart: bool, _hidden: bool) -> Result<Answer> {
    Err(Error::unsupported(
        "Running in the background isn't available on this system.",
    ))
}

/// The portal's error says it isn't there: xdg-desktop-portal offers the
/// Background portal only with a desktop backend for it (Sway, Hyprland,
/// XFCE and others have none), or no portals run at all.
#[cfg(target_os = "linux")]
pub(super) fn no_portal(e: &ashpd::Error) -> bool {
    use zbus::fdo::Error as Fdo;

    let e = match e {
        ashpd::Error::PortalNotFound(_) => return true,
        ashpd::Error::Zbus(e) | ashpd::Error::Portal(ashpd::PortalError::ZBus(e)) => e,
        _ => return false,
    };
    match e {
        zbus::Error::MethodError(name, ..) => matches!(
            name.as_str(),
            "org.freedesktop.DBus.Error.UnknownMethod"
                | "org.freedesktop.DBus.Error.UnknownInterface"
                | "org.freedesktop.DBus.Error.UnknownObject"
                | "org.freedesktop.DBus.Error.ServiceUnknown"
        ),
        zbus::Error::FDO(e) => matches!(
            **e,
            Fdo::UnknownMethod(_)
                | Fdo::UnknownInterface(_)
                | Fdo::UnknownObject(_)
                | Fdo::ServiceUnknown(_)
        ),
        _ => false,
    }
}

/// Flatpak: asks the Background portal again at startup when it answered
/// in an earlier run, since the user may have changed that in the system's
/// settings for apps meanwhile. Once it has an answer, the portal gives it
/// at once, without asking the user.
pub async fn check_again(app: &AppHandle) {
    let desktop = app.state::<Desktop>();
    let Ok(current) = settings(app) else {
        return;
    };
    if !worth_asking_again(
        desktop.background.backend,
        desktop.background.allowed(),
        &current,
    ) {
        return;
    }
    if let Err(e) = request(app, current.start_at_login, current.start_hidden).await {
        tracing::info!("could not check the background permission again: {e}");
    }
}

/// The portal answered in an earlier run (else the first close asks), and
/// the answer matters: the app is to run in the background or at login.
fn worth_asking_again(
    backend: AutostartBackend,
    allowed: Option<bool>,
    settings: &Settings,
) -> bool {
    backend == AutostartBackend::Portal
        && allowed.is_some()
        && (settings.run_in_background || settings.start_at_login)
}

/// Tells the desktop what the app does while it runs without a window
/// (GNOME lists it under Background Apps). Flatpak only.
#[cfg(target_os = "linux")]
pub async fn set_status(app: &AppHandle) {
    use ashpd::desktop::background::{BackgroundProxy, SetStatusOptions};

    let desktop = app.state::<Desktop>();
    if desktop.background.backend != AutostartBackend::Portal {
        return;
    }
    let accounts = app
        .state::<AppState>()
        .service
        .list_accounts()
        .map(|a| a.len())
        .unwrap_or(0);
    let message = match accounts {
        0 => "Waiting for an account".to_owned(),
        1 => "Watching 1 account".to_owned(),
        n => format!("Watching {n} accounts"),
    };
    // Accounts change at every sync (when they were last synced); the
    // status only when their number does.
    let mut last = desktop.background.status.lock().await;
    if last.as_deref() == Some(message.as_str()) {
        return;
    }
    let set = async {
        let proxy = BackgroundProxy::new().await?;
        proxy
            .set_status(SetStatusOptions::default().set_message(&message))
            .await
    };
    match set.await {
        Ok(()) => *last = Some(message),
        Err(e) => tracing::debug!("could not set the background status: {e}"),
    }
}

#[cfg(not(target_os = "linux"))]
pub async fn set_status(_app: &AppHandle) {}

/// Native installs: brings the setting in line with the autostart file
/// (which the user may have removed elsewhere), and points the file at
/// this executable (an AppImage may have moved).
pub async fn reconcile_autostart(app: &AppHandle) {
    let desktop = app.state::<Desktop>();
    if desktop.background.backend != AutostartBackend::XdgAutostartFile {
        return;
    }
    let Ok(current) = settings(app) else {
        return;
    };
    let path = autostart_path(&config_home());
    let enabled = autostart_enabled(&path);
    if enabled && let Err(e) = write_autostart(&path, &executable(), current.start_hidden) {
        tracing::warn!("could not update the autostart file: {e}");
    }
    if enabled != current.start_at_login
        && let Err(e) = save(app, |s| s.start_at_login = enabled).await
    {
        tracing::warn!("could not save the start at login setting: {e}");
    }
}

// --- the XDG autostart file -----------------------------------------------------

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"))
}

pub(crate) fn autostart_path(config_home: &Path) -> PathBuf {
    config_home
        .join("autostart")
        .join(format!("{APP_ID}.desktop"))
}

/// The program to start: the AppImage when running from one (its
/// executable is in a temporary mount), else this executable where it's
/// installed.
fn executable() -> PathBuf {
    executable_from(
        std::env::var_os("APPIMAGE").map(PathBuf::from),
        std::env::current_exe().ok(),
    )
}

pub(crate) fn executable_from(appimage: Option<PathBuf>, current: Option<PathBuf>) -> PathBuf {
    appimage
        .filter(|p| p.is_absolute())
        .or_else(|| current.map(installed))
        .unwrap_or_else(|| PathBuf::from("cumulas-mail"))
}

/// Where the running executable is installed. Once a package upgrade has
/// replaced it there, Linux names it "<path> (deleted)"; the new one is at
/// the path.
fn installed(current: PathBuf) -> PathBuf {
    match current.to_str().and_then(|p| p.strip_suffix(" (deleted)")) {
        Some(path) => PathBuf::from(path),
        None => current,
    }
}

pub(crate) fn write_autostart(path: &Path, executable: &Path, hidden: bool) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, autostart_entry(executable, hidden))?;
    Ok(())
}

pub(crate) fn remove_autostart(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// Whether the autostart file exists and isn't turned off in it.
pub(crate) fn autostart_enabled(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    !text.lines().any(|line| {
        let line = line.trim();
        line.eq_ignore_ascii_case("Hidden=true")
            || line.eq_ignore_ascii_case("X-GNOME-Autostart-enabled=false")
    })
}

pub(crate) fn autostart_entry(executable: &Path, hidden: bool) -> String {
    let mut exec = exec_arg(&executable.to_string_lossy());
    if hidden {
        exec.push(' ');
        exec.push_str(BACKGROUND_ARG);
    }
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Cumulas Mail\n\
         Comment=Shows new mail while the window is closed\n\
         Exec={}\n\
         Icon={APP_ID}\n\
         Terminal=false\n\
         NoDisplay=true\n\
         X-GNOME-Autostart-enabled=true\n\
         X-GNOME-UsesNotifications=true\n",
        escape_string(&exec)
    )
}

/// One argument of an `Exec` key, quoted when it needs to be (desktop entry
/// spec, "The Exec key"): `"`, `` ` ``, `$` and `\` are escaped inside the
/// quotes, and `%` is doubled.
fn exec_arg(arg: &str) -> String {
    const RESERVED: &[char] = &[
        ' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(',
        ')', '`',
    ];
    let arg = arg.replace('%', "%%");
    if !arg.is_empty() && !arg.contains(RESERVED) {
        return arg;
    }
    let mut quoted = String::with_capacity(arg.len() + 2);
    quoted.push('"');
    for c in arg.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted
}

/// The escapes every string value gets, applied after the `Exec` quoting:
/// so a backslash in a quoted argument ends up as four.
fn escape_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exec_line(entry: &str) -> &str {
        entry.lines().find_map(|l| l.strip_prefix("Exec=")).unwrap()
    }

    #[test]
    fn plain_paths_stay_plain() {
        let entry = autostart_entry(Path::new("/usr/bin/cumulas-mail"), true);
        assert_eq!(exec_line(&entry), "/usr/bin/cumulas-mail --background");
        assert!(entry.contains("\nNoDisplay=true\n"));
        assert!(entry.contains("\nX-GNOME-Autostart-enabled=true\n"));
        let shown = autostart_entry(Path::new("/usr/bin/cumulas-mail"), false);
        assert_eq!(exec_line(&shown), "/usr/bin/cumulas-mail");
    }

    #[test]
    fn paths_with_spaces_and_specials_are_quoted() {
        let entry = autostart_entry(Path::new("/home/ada/My Apps/Cumulas Mail.AppImage"), true);
        assert_eq!(
            exec_line(&entry),
            r#""/home/ada/My Apps/Cumulas Mail.AppImage" --background"#
        );
        // `"`, `$` and `` ` `` are escaped in the quotes, `%` doubled, and a
        // backslash ends up as four.
        let odd = autostart_entry(Path::new(r#"/opt/a"b$c`d\e 100%/app"#), false);
        assert_eq!(exec_line(&odd), r#""/opt/a\\"b\\$c\\`d\\\\e 100%%/app""#);
        assert_eq!(exec_arg("/opt/50%/app"), "/opt/50%%/app");
    }

    #[test]
    fn the_appimage_is_started_rather_than_its_mount() {
        let mount = PathBuf::from("/tmp/.mount_CumulaXYZ/usr/bin/cumulas-mail");
        assert_eq!(
            executable_from(
                Some(PathBuf::from("/home/ada/Cumulas.AppImage")),
                Some(mount.clone())
            ),
            PathBuf::from("/home/ada/Cumulas.AppImage")
        );
        assert_eq!(
            executable_from(
                Some(PathBuf::from("relative.AppImage")),
                Some(mount.clone())
            ),
            mount
        );
        assert_eq!(executable_from(None, Some(mount.clone())), mount);
        assert_eq!(executable_from(None, None), PathBuf::from("cumulas-mail"));
    }

    #[test]
    fn an_upgraded_executable_is_started_where_it_was_installed() {
        // What /proc/self/exe reads once dpkg or rpm replaced the file.
        let replaced = PathBuf::from("/usr/bin/cumulas-mail (deleted)");
        let executable = executable_from(None, Some(replaced));
        assert_eq!(executable, PathBuf::from("/usr/bin/cumulas-mail"));
        assert_eq!(
            exec_line(&autostart_entry(&executable, true)),
            "/usr/bin/cumulas-mail --background"
        );
        let spaced = PathBuf::from("/opt/Cumulas Mail/cumulas-mail");
        assert_eq!(executable_from(None, Some(spaced.clone())), spaced);
    }

    #[test]
    fn a_kept_answer_is_checked_again_at_startup() {
        let wants = Settings::default();
        assert!(wants.run_in_background);
        // Refused or allowed in an earlier run: either may have changed in
        // the system's settings since.
        for kept in [Some(false), Some(true)] {
            assert!(worth_asking_again(AutostartBackend::Portal, kept, &wants));
        }
        // Never asked: the first close asks.
        assert!(!worth_asking_again(AutostartBackend::Portal, None, &wants));
        // Native installs don't ask.
        let native = AutostartBackend::XdgAutostartFile;
        assert!(!worth_asking_again(native, Some(true), &wants));
        // Neither running in the background nor starting at login.
        let neither = Settings {
            run_in_background: false,
            start_at_login: false,
            ..Settings::default()
        };
        let portal = AutostartBackend::Portal;
        assert!(!worth_asking_again(portal, Some(false), &neither));
        let login = Settings {
            start_at_login: true,
            ..neither
        };
        assert!(worth_asking_again(portal, Some(false), &login));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_missing_portal_is_not_a_refusal() {
        use ashpd::desktop::ResponseError;
        use zbus::fdo::Error as Fdo;

        let missing = [
            // Its interface isn't exported (no backend for it).
            ashpd::Error::PortalNotFound("org.freedesktop.portal.Background".try_into().unwrap()),
            ashpd::Error::Portal(ashpd::PortalError::ZBus(zbus::Error::FDO(Box::new(
                Fdo::UnknownMethod("No such interface".into()),
            )))),
            // No portals at all.
            ashpd::Error::Zbus(zbus::Error::FDO(Box::new(Fdo::ServiceUnknown(
                "The name is not activatable".into(),
            )))),
        ];
        for e in &missing {
            assert!(no_portal(e), "{e}");
        }
        let answered = [
            // How the portal refuses.
            ashpd::Error::Response(ResponseError::Cancelled),
            ashpd::Error::Response(ResponseError::Other),
            ashpd::Error::Portal(ashpd::PortalError::NotAllowed("no".into())),
            ashpd::Error::Zbus(zbus::Error::FDO(Box::new(Fdo::AccessDenied("no".into())))),
            ashpd::Error::NoResponse,
        ];
        for e in &answered {
            assert!(!no_portal(e), "{e}");
        }
        assert!(Answer::NoPortal.background());
        assert!(
            !Answer::Portal {
                background: false,
                autostart: false
            }
            .background()
        );
    }

    #[test]
    fn the_file_is_written_read_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = autostart_path(dir.path());
        assert_eq!(
            path,
            dir.path().join("autostart").join("co.cumulas.Mail.desktop")
        );
        assert!(!autostart_enabled(&path));
        remove_autostart(&path).unwrap();

        write_autostart(&path, Path::new("/usr/bin/cumulas-mail"), true).unwrap();
        assert!(autostart_enabled(&path));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("[Desktop Entry]\nType=Application\n"));

        // Turned off by another tool.
        std::fs::write(&path, text.replace("enabled=true", "enabled=false")).unwrap();
        assert!(!autostart_enabled(&path));

        remove_autostart(&path).unwrap();
        assert!(!path.exists());
    }
}
