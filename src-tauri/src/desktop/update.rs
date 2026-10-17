//! Updates of the Flatpak. Software centers and `flatpak update` install
//! them; the Flatpak portal's update monitor says when a version other than
//! the running one is installed, and the portal starts that version for a
//! restart. Elsewhere there's nothing to say: the state stays `None`.
//!
//! A restart hands over through the single-instance name:
//! 1. The running instance asks the portal to start the installed version
//!    (`Spawn` with `LatestVersion`), naming the name's owner, itself
//!    (`--restarted-from=:1.42`).
//! 2. Before Tauri starts, the new instance checks that it can run: on the
//!    runtime it was built for (the portal keeps the running instance's)
//!    and with a display (before flatpak 1.15.6, the portal starts it with
//!    its own environment). It says how that went through the
//!    single-instance callback: `--restart-ready` or `--restart-refused`.
//! 3. Ready: the running instance shuts push down, lets go of the name and
//!    exits; the new one waits for that, then starts as usual, with a
//!    window. Otherwise, or when the new one exits early or doesn't answer,
//!    the running instance stays, and Restart can be tried again.

use std::ffi::OsString;
use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use cumulas_core::{Error, Result};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;

use super::notify::{Action, Toast, ToastKind};
use super::{Desktop, tray};
use crate::AppState;

/// The update notifications' id: the latest replaces the one before.
pub const NOTICE_ID: &str = "app:update";
/// Kept between runs: the installed commit the notification was shown for.
const NOTIFIED_KEY: &str = "shell.update_notified";
/// `--restarted-from=<the previous instance's unique bus name>`.
pub const RESTARTED_ARG: &str = "--restarted-from";
/// What the new instance tells the previous one, through the
/// single-instance callback.
pub const READY_ARG: &str = "--restart-ready";
pub const REFUSED_ARG: &str = "--restart-refused";
/// The manifest's `command`.
const COMMAND: &str = "cumulas-mail";
/// tauri-plugin-single-instance's name and object for this app id
/// (`platform_impl/linux.rs`: the identifier, then ".SingleInstance").
pub(crate) const SINGLE_INSTANCE_NAME: &str = "co.cumulas.Mail.SingleInstance";
pub(crate) const SINGLE_INSTANCE_PATH: &str = "/co/cumulas/Mail/SingleInstance";
#[cfg(target_os = "linux")]
const SINGLE_INSTANCE_INTERFACE: &str = "org.SingleInstance.DBus";
/// How long the running instance waits for the new one to answer.
#[cfg(target_os = "linux")]
const HANDOVER_LIMIT: Duration = Duration::from_secs(30);
/// How long the new instance waits for the previous one to let go of the
/// single-instance name.
#[cfg(target_os = "linux")]
const RELEASE_LIMIT: Duration = Duration::from_secs(60);
const RELEASE_POLL: Duration = Duration::from_millis(100);
/// The new instance's D-Bus calls to the previous one.
#[cfg(target_os = "linux")]
const CALLBACK_LIMIT: Duration = Duration::from_secs(10);
/// Between attempts to watch: 1 minute, 5 minutes, then every 30.
const RETRY: [Duration; 3] = [
    Duration::from_secs(60),
    Duration::from_secs(5 * 60),
    Duration::from_secs(30 * 60),
];
/// After the portal went away, before watching again (asking starts it).
#[cfg(target_os = "linux")]
const REWATCH_AFTER: Duration = Duration::from_secs(5);
#[cfg(target_os = "linux")]
const PORTAL: &str = "org.freedesktop.portal.Flatpak";
/// What flatpak-builder writes into every build: the manifest, which names
/// the runtime the build is for.
const MANIFEST: &str = "/app/manifest.json";
/// What `flatpak run` writes into the sandbox, which names the runtime it
/// started the app on.
const FLATPAK_INFO: &str = "/.flatpak-info";
/// A restart from the page or the tray carries no activation token, and
/// the ones the new instance would inherit are stale.
#[cfg(target_os = "linux")]
const ACTIVATION_ENV: [&str; 2] = ["XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"];
#[cfg(target_os = "linux")]
const SIGTERM: u32 = 15;

/// The page shows these (`errorMessage`). A refusal reads like any other
/// failure, since the remedy is the same; the log says which it was.
const RESTART_FAILED: &str = "Couldn't restart. Try again, or quit Cumulas Mail and open it again.";
const NOTHING_TO_RESTART: &str = "There's no new version to restart into.";

/// What the page shows about updates (`get_update_state` and the
/// `update-state` event). Always write `UpdateState::None` in full: the
/// bare name is Option's.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UpdateState {
    #[default]
    None,
    /// A version other than the running one is installed (its commit);
    /// `restart` says how restarting into it goes.
    #[serde(rename_all = "camelCase")]
    Installed { commit: String, restart: Restart },
}

/// Restarting into an installed update.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Restart {
    /// Not asked for yet.
    #[default]
    Idle,
    /// The new version is starting, to take over.
    Restarting,
    /// The last restart failed; it can be tried again.
    Failed,
}

/// What the new instance answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handover {
    Ready,
    Refused,
}

/// Why the new instance can't take over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// It was built for another runtime than the one it was started on.
    Runtime,
    /// It can't open the display.
    Display,
}

/// What the new instance does after answering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TakeOver {
    Continue,
    Exit,
}

/// Who owns the single-instance name, as far as a call could tell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Owner {
    None,
    Is(String),
    /// The call failed: it says nothing about the name.
    Unknown,
}

/// The update state, and what a restart waits for.
#[derive(Default)]
pub struct Updates {
    state: Mutex<UpdateState>,
    /// The restart running waits here for the new instance's answer.
    handover: Mutex<Option<oneshot::Sender<Handover>>>,
    /// A notification's Restart button's, kept while the page is asked
    /// (see `window::restart`): the new window takes the focus with it.
    activation_token: Mutex<Option<String>>,
}

impl Updates {
    pub fn state(&self) -> UpdateState {
        self.state.lock().unwrap().clone()
    }

    /// A version other than the running one is installed: Restart applies.
    pub fn installed(&self) -> bool {
        matches!(*self.state.lock().unwrap(), UpdateState::Installed { .. })
    }

    pub fn keep_activation_token(&self, token: Option<String>) {
        *self.activation_token.lock().unwrap() = token;
    }

    pub fn take_activation_token(&self) -> Option<String> {
        self.activation_token.lock().unwrap().take()
    }

    /// Changes the state with `change`; the state before and after.
    fn change(
        &self,
        change: impl FnOnce(&UpdateState) -> UpdateState,
    ) -> (UpdateState, UpdateState) {
        let mut state = self.state.lock().unwrap();
        let after = change(&state);
        (std::mem::replace(&mut *state, after.clone()), after)
    }

    /// Ends a restart the new version didn't take over: nothing waits for
    /// its answer anymore, and Restart can be tried again.
    fn end_restart(&self) -> (UpdateState, UpdateState) {
        self.handover.lock().unwrap().take();
        self.change(after_restart_failed)
    }
}

/// A restart until the new version takes over. Dropped before that,
/// however the restart ended (it failed, panicked, or its task was
/// dropped), it calls `ended`.
struct Unfinished<F: FnOnce()>(Option<F>);

impl<F: FnOnce()> Unfinished<F> {
    fn new(ended: F) -> Self {
        Self(Some(ended))
    }

    fn took_over(mut self) {
        self.0 = None;
    }
}

impl<F: FnOnce()> Drop for Unfinished<F> {
    fn drop(&mut self) {
        if let Some(ended) = self.0.take() {
            ended();
        }
    }
}

fn installed(commit: &str, restart: Restart) -> UpdateState {
    UpdateState::Installed {
        commit: commit.to_owned(),
        restart,
    }
}

/// The state after the update monitor's report: an installed commit other
/// than the running one is an update to restart into, whatever the remote
/// has (software centers install updates, the app doesn't).
pub(crate) fn after_report(before: &UpdateState, running: &str, local: &str) -> UpdateState {
    if local.is_empty() || local == running {
        return UpdateState::None;
    }
    match before {
        UpdateState::Installed { commit, .. } if commit == local => before.clone(),
        // A newer one while restarting: the restart starts the newest.
        UpdateState::Installed {
            restart: Restart::Restarting,
            ..
        } => installed(local, Restart::Restarting),
        _ => installed(local, Restart::Idle),
    }
}

/// The state once a restart is asked for: unchanged when one runs already,
/// or there's nothing to restart into.
pub(crate) fn after_restart_asked(before: &UpdateState) -> UpdateState {
    match before {
        UpdateState::Installed {
            commit,
            restart: Restart::Idle | Restart::Failed,
        } => installed(commit, Restart::Restarting),
        _ => before.clone(),
    }
}

/// The state once a restart failed: it can be tried again.
pub(crate) fn after_restart_failed(before: &UpdateState) -> UpdateState {
    match before {
        UpdateState::Installed {
            commit,
            restart: Restart::Restarting,
        } => installed(commit, Restart::Failed),
        _ => before.clone(),
    }
}

/// How long to wait before the `attempt`th retry (from 0) to watch.
pub(crate) fn retry_after(attempt: u32) -> Duration {
    RETRY[(attempt as usize).min(RETRY.len() - 1)]
}

/// The new instance's arguments. No `--background`: it always shows its
/// window, even when this one started without one.
pub(crate) fn restart_args(previous: &str) -> Vec<String> {
    vec![COMMAND.to_owned(), format!("{RESTARTED_ARG}={previous}")]
}

/// This instance's language and input method, for the new one: before
/// flatpak 1.15.6, the portal starts it with its own environment (the
/// session's activation environment), which may differ. A variable that
/// isn't UTF-8 stays behind (`std::env::vars` would panic on it).
pub(crate) fn carried_env(
    vars: impl IntoIterator<Item = (OsString, OsString)>,
) -> Vec<(String, String)> {
    let mut carried: Vec<(String, String)> = vars
        .into_iter()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .filter(|(name, _)| {
            matches!(
                name.as_str(),
                "LANG" | "LANGUAGE" | "GTK_IM_MODULE" | "QT_IM_MODULE" | "XMODIFIERS"
            ) || name.starts_with("LC_")
        })
        .collect();
    carried.sort();
    carried
}

/// The previous instance's unique bus name, from `--restarted-from=…`:
/// ":" and then letters, digits, ".", "_" or "-". `None` for a normal
/// launch, and for anything else.
pub fn restarted_from(args: impl IntoIterator<Item = String>) -> Option<String> {
    args.into_iter()
        .find_map(|arg| {
            arg.strip_prefix(RESTARTED_ARG)?
                .strip_prefix('=')
                .map(str::to_owned)
        })
        .filter(|name| {
            name.strip_prefix(':').is_some_and(|rest| {
                !rest.is_empty()
                    && rest
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
            })
        })
}

/// The new instance's answer, from the arguments the single-instance
/// callback got; `None` for a launch.
pub fn handover(argv: &[String]) -> Option<Handover> {
    argv.iter().skip(1).find_map(|arg| match arg.as_str() {
        READY_ARG => Some(Handover::Ready),
        REFUSED_ARG => Some(Handover::Refused),
        _ => None,
    })
}

/// The runtime `/.flatpak-info` says the app runs on, as its id and branch:
/// `[Application] runtime=runtime/<id>/<arch>/<branch>`.
pub(crate) fn running_runtime(flatpak_info: &str) -> Option<(&str, &str)> {
    let mut section = "";
    for line in flatpak_info.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name;
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if section != "Application" || key.trim() != "runtime" {
            continue;
        }
        let mut parts = value.trim().split('/');
        return match (parts.next(), parts.next(), parts.next(), parts.next()) {
            (Some("runtime"), Some(id), Some(_arch), Some(branch))
                if !id.is_empty() && !branch.is_empty() =>
            {
                Some((id, branch))
            }
            _ => None,
        };
    }
    None
}

/// The runtime the build is for, as its id (when named) and branch, from
/// the manifest flatpak-builder keeps in `/app/manifest.json`.
pub(crate) fn built_for(manifest: &str) -> Option<(Option<String>, String)> {
    let manifest: serde_json::Value = serde_json::from_str(manifest).ok()?;
    let branch = manifest.get("runtime-version")?.as_str()?.to_owned();
    let id = manifest
        .get("runtime")
        .and_then(|r| r.as_str())
        .map(str::to_owned);
    Some((id, branch))
}

/// Whether this build runs on the runtime it was built for: true unless
/// both are known and differ. The portal starts a restart's new version on
/// the previous one's runtime, so a version built for another would run on
/// the old one; a normal launch uses the right one.
pub(crate) fn runtime_matches(manifest: Option<&str>, flatpak_info: Option<&str>) -> bool {
    let (Some((wanted_id, wanted_branch)), Some((id, branch))) = (
        manifest.and_then(built_for),
        flatpak_info.and_then(running_runtime),
    ) else {
        return true;
    };
    wanted_branch == branch && wanted_id.is_none_or(|wanted| wanted == id)
}

/// What a new instance that can't take over does: it runs anyway only when
/// the previous one is gone already and just the runtime is wrong (the new
/// version on the old runtime beats no app); without a display nothing
/// could run.
pub(crate) fn after_refusal(refusal: Refusal, previous_gone: bool) -> TakeOver {
    match (refusal, previous_gone) {
        (Refusal::Runtime, true) => TakeOver::Continue,
        _ => TakeOver::Exit,
    }
}

/// Waits until `owner` says the single-instance name has no owner, or
/// another than `previous`; false after `limit`. A failed call (`Unknown`)
/// doesn't count as let go.
pub(crate) async fn wait_for_release<F, Fut>(mut owner: F, previous: &str, limit: Duration) -> bool
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Owner>,
{
    let released = async {
        loop {
            match owner().await {
                Owner::None => return,
                Owner::Is(name) if name != previous => return,
                Owner::Is(_) | Owner::Unknown => {}
            }
            tokio::time::sleep(RELEASE_POLL).await;
        }
    };
    tokio::time::timeout(limit, released).await.is_ok()
}

/// Changes the state with `change`, and tells the page and the tray, and
/// the desktop when a version was newly installed; returns the state from
/// before.
fn transition(app: &AppHandle, change: impl FnOnce(&UpdateState) -> UpdateState) -> UpdateState {
    let Some(desktop) = app.try_state::<Desktop>() else {
        return UpdateState::None;
    };
    let (before, after) = desktop.update.change(change);
    announce(app, &before, &after);
    before
}

/// Tells the page and the tray that the state changed, and the desktop
/// when a version was newly installed.
fn announce(app: &AppHandle, before: &UpdateState, after: &UpdateState) {
    if before == after {
        return;
    }
    if let Err(e) = app.emit("update-state", after) {
        tracing::debug!("could not send the update state: {e}");
    }
    tray::update_changed(app);
    if let UpdateState::Installed { commit, .. } = after
        && !matches!(before, UpdateState::Installed { commit: was, .. } if was == commit)
    {
        let (app, commit) = (app.clone(), commit.clone());
        tauri::async_runtime::spawn(async move { notify_installed(&app, &commit).await });
    }
}

/// Says once per installed version that it's there, while the window isn't
/// visible (hidden to the background, or not opened since a start at
/// login): the app may run like that for days, and many desktops show no
/// tray icons. A visible window has the page's card.
async fn notify_installed(app: &AppHandle, commit: &str) {
    let desktop = app.state::<Desktop>();
    if desktop.window.visible() {
        return;
    }
    let store = app.state::<AppState>().service.store().clone();
    if store.app_value(NOTIFIED_KEY).ok().flatten().as_deref() == Some(commit) {
        return;
    }
    match desktop
        .notifications
        .show(&update_toast(), Vec::new())
        .await
    {
        Ok(()) => {
            if let Err(e) = store.set_app_value(NOTIFIED_KEY, commit) {
                tracing::warn!("could not remember the update notification: {e}");
            }
        }
        Err(e) => tracing::info!("could not say an update is installed: {e}"),
    }
}

fn update_toast() -> Toast {
    Toast {
        id: NOTICE_ID.into(),
        title: "Update installed".into(),
        body: "Restart Cumulas Mail to use the new version.".into(),
        kind: ToastKind::App,
        default_action: Action::ShowWindow,
        buttons: vec![("Restart".into(), Action::RestartToUpdate)],
        sound: false,
    }
}

fn restart_failed_toast() -> Toast {
    Toast {
        id: NOTICE_ID.into(),
        title: "Couldn't restart".into(),
        body: "Try again, or quit Cumulas Mail and open it again.".into(),
        kind: ToastKind::App,
        default_action: Action::ShowWindow,
        buttons: vec![("Try again".into(), Action::RestartToUpdate)],
        sound: false,
    }
}

/// Follows the Flatpak portal's update monitor for as long as the app
/// runs, in a Flatpak. A missing or old portal isn't an error: it's tried
/// again later (ashpd reads the portal's version as 1 whenever it can't
/// read it, at login say), and at once when the portal starts.
#[cfg(target_os = "linux")]
pub async fn watch(app: AppHandle) {
    if !super::is_flatpak() {
        return;
    }
    let mut owners = follow_portal().await;
    let mut attempt = 0;
    loop {
        let monitored = async {
            // A connection of its own: when it goes, the portal closes the
            // monitor made on it.
            let connection = zbus::Connection::session().await?;
            let portal = ashpd::flatpak::Flatpak::with_connection(connection).await?;
            portal.create_update_monitor(Default::default()).await
        };
        let monitor = match monitored.await {
            Ok(monitor) => monitor,
            Err(e) if gives_up(&e) => {
                tracing::info!("not watching for updates: {e}");
                return;
            }
            Err(e) => {
                if attempt == 0 {
                    tracing::info!("could not watch for updates yet: {e}");
                } else {
                    tracing::debug!("could not watch for updates yet: {e}");
                }
                tokio::select! {
                    () = tokio::time::sleep(retry_after(attempt)) => {}
                    () = portal_changed(&mut owners) => {}
                }
                attempt = attempt.saturating_add(1);
                continue;
            }
        };
        attempt = 0;
        tracing::info!("watching for updates");
        follow_reports(&app, &monitor, &mut owners).await;
        tokio::time::sleep(REWATCH_AFTER).await;
    }
}

#[cfg(not(target_os = "linux"))]
pub async fn watch(_app: AppHandle) {}

/// Applies the monitor's reports until the portal goes (it's restarted or
/// replaced, and the monitor with it) or stops sending them.
#[cfg(target_os = "linux")]
async fn follow_reports(
    app: &AppHandle,
    monitor: &ashpd::flatpak::update_monitor::UpdateMonitor,
    owners: &mut Option<zbus::fdo::NameOwnerChangedStream>,
) {
    use futures::StreamExt;

    let mut reports = match monitor.receive_update_available().await {
        Ok(reports) => reports,
        Err(e) => {
            tracing::info!("could not watch for updates: {e}");
            return;
        }
    };
    loop {
        tokio::select! {
            report = reports.next() => match report {
                Some(info) => {
                    let (running, local) = (info.running_commit(), info.local_commit());
                    tracing::debug!("update report: running {running}, installed {local}");
                    transition(app, |state| after_report(state, running, local));
                }
                None => {
                    tracing::info!("the Flatpak portal stopped reporting updates; watching again");
                    let _ = monitor.close().await;
                    return;
                }
            },
            () = portal_changed(owners) => {
                tracing::info!("the Flatpak portal restarted; watching for updates again");
                return;
            }
        }
    }
}

/// Changes of the Flatpak portal's owner, as `tray::watch` follows the
/// StatusNotifier host; `None` when they can't be followed.
#[cfg(target_os = "linux")]
async fn follow_portal() -> Option<zbus::fdo::NameOwnerChangedStream> {
    let follow = async {
        let connection = zbus::Connection::session().await?;
        let bus = zbus::fdo::DBusProxy::new(&connection).await?;
        bus.receive_name_owner_changed_with_args(&[(0, PORTAL)])
            .await
    };
    match follow.await {
        Ok(changes) => Some(changes),
        Err(e) => {
            tracing::info!("can't follow the Flatpak portal ({e})");
            None
        }
    }
}

/// Returns when the Flatpak portal's owner changes: it stopped, started or
/// was replaced. Never, when that can't be followed.
#[cfg(target_os = "linux")]
async fn portal_changed(owners: &mut Option<zbus::fdo::NameOwnerChangedStream>) {
    use futures::StreamExt;

    if let Some(changes) = owners {
        if changes.next().await.is_some() {
            return;
        }
        *owners = None;
    }
    std::future::pending().await
}

/// The error says watching can never work here: there's no Flatpak
/// portal, or it doesn't watch this app (not a Flatpak app as far as it
/// can tell, as under `flatpak-builder --run`).
#[cfg(target_os = "linux")]
fn gives_up(e: &ashpd::Error) -> bool {
    use zbus::fdo::Error as Fdo;

    if super::background::no_portal(e) {
        return true;
    }
    let incomplete = |detail: &str| detail.starts_with("Incomplete instance info");
    let e = match e {
        ashpd::Error::Zbus(e) | ashpd::Error::Portal(ashpd::PortalError::ZBus(e)) => e,
        _ => return false,
    };
    match e {
        zbus::Error::MethodError(name, detail, _) => match name.as_str() {
            "org.freedesktop.DBus.Error.NotSupported" => true,
            "org.freedesktop.DBus.Error.Failed" => detail.as_deref().is_some_and(incomplete),
            _ => false,
        },
        zbus::Error::FDO(e) => match &**e {
            Fdo::NotSupported(_) => true,
            Fdo::Failed(detail) => incomplete(detail),
            _ => false,
        },
        _ => false,
    }
}

/// Restarts into the installed update (see the module's doc): the app
/// exits once the new version has taken over. Asked again while it runs,
/// does nothing. When it fails, the app keeps running, Restart can be tried
/// again, and a notification says so if the window isn't visible.
pub async fn restart(app: &AppHandle, activation_token: Option<String>) -> Result<()> {
    match transition(app, after_restart_asked) {
        UpdateState::None => return Err(Error::unsupported(NOTHING_TO_RESTART)),
        UpdateState::Installed {
            restart: Restart::Restarting,
            ..
        } => return Ok(()),
        UpdateState::Installed { .. } => {}
    }
    // Whatever stops it from here, even a panic, ends it the same way.
    let unfinished = Unfinished::new(|| restart_failed(app));
    if let Err(reason) = hand_over(app, activation_token).await {
        tracing::warn!("couldn't restart into the update: {reason}");
        return Err(Error::unsupported(RESTART_FAILED));
    }
    unfinished.took_over();
    tracing::info!("the new version took over; exiting");
    let desktop = app.state::<Desktop>();
    desktop.notifications.withdraw(NOTICE_ID).await;
    // Push first, then the name: the new instance starts its own once it
    // has the name, and two never run at once.
    if let Some(state) = app.try_state::<AppState>() {
        let _ = tokio::time::timeout(crate::SHUTDOWN_LIMIT, state.service.shutdown()).await;
    }
    let handle = app.clone();
    let left = app.run_on_main_thread(move || {
        handle.state::<Desktop>().window.forget_quit();
        // Now rather than on exit, which may take a while: the new instance
        // waits for it. (zbus's blocking calls panic in an async task.)
        tauri_plugin_single_instance::destroy(&handle);
        handle.exit(0);
    });
    if let Err(e) = left {
        tracing::error!("could not exit for the new version: {e}");
        app.exit(0);
    }
    Ok(())
}

/// Ends a restart the new version didn't take over: Restart can be tried
/// again, and a notification says so while the window isn't visible.
fn restart_failed(app: &AppHandle) {
    let Some(desktop) = app.try_state::<Desktop>() else {
        return;
    };
    let (before, after) = desktop.update.end_restart();
    announce(app, &before, &after);
    if desktop.window.visible() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let desktop = app.state::<Desktop>();
        let shown = desktop
            .notifications
            .show(&restart_failed_toast(), Vec::new())
            .await;
        if let Err(e) = shown {
            tracing::info!("could not say the restart failed: {e}");
        }
    });
}

/// Starts the installed version through the Flatpak portal, and waits for
/// it to say it's ready; why not, when it doesn't.
#[cfg(target_os = "linux")]
async fn hand_over(
    app: &AppHandle,
    activation_token: Option<String>,
) -> std::result::Result<(), String> {
    use std::collections::HashMap;
    use std::os::fd::BorrowedFd;

    use ashpd::flatpak::{Flatpak, SpawnFlags, SpawnOptions};
    use futures::StreamExt;

    // Its owner is this instance: the new one waits for it to let go. If
    // the name weren't the plugin's, the new one would hand off to nobody
    // and exit, after this one had gone: so no owner, no restart.
    let connection = zbus::Connection::session()
        .await
        .map_err(|e| format!("no session bus ({e})"))?;
    let previous = match owner_of(&connection).await {
        Owner::Is(name) => name,
        Owner::None | Owner::Unknown => return Err(format!("nobody owns {SINGLE_INSTANCE_NAME}")),
    };
    let (answered, answer) = oneshot::channel();
    *app.state::<Desktop>().update.handover.lock().unwrap() = Some(answered);

    let portal = Flatpak::with_connection(connection)
        .await
        .map_err(|e| format!("no Flatpak portal ({e})"))?;
    // Before Spawn: a version that can't even start exits at once.
    let exits = portal
        .receive_spawn_exited()
        .await
        .map_err(|e| format!("can't follow the new version ({e})"))?;
    let mut env = carried_env(std::env::vars_os());
    let options = match &activation_token {
        Some(token) => {
            env.extend(ACTIVATION_ENV.map(|name| (name.to_owned(), token.clone())));
            SpawnOptions::default()
        }
        None => SpawnOptions::default().unset_env(ACTIVATION_ENV),
    };
    let envs: HashMap<&str, &str> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    // Not WatchBus, which would stop the new version when this one exits.
    let pid = portal
        .spawn(
            "",
            &restart_args(&previous),
            HashMap::<u32, BorrowedFd<'_>>::new(),
            envs,
            SpawnFlags::LatestVersion.into(),
            options,
        )
        .await
        .map_err(|e| format!("the Flatpak portal didn't start it ({e})"))?;
    tracing::info!("started the new version (process {pid}); waiting for it to take over");
    let mut exited = exits.filter(move |(exited, _)| std::future::ready(*exited == pid));
    tokio::select! {
        biased;
        answer = answer => match answer {
            Ok(Handover::Ready) => Ok(()),
            Ok(Handover::Refused) => Err("the new version can't run here (see its log)".into()),
            Err(_) => Err("its answer was lost".into()),
        },
        exit = exited.next() => Err(match exit {
            Some((_, status)) => format!("the new version exited ({status}) before taking over"),
            None => "lost the Flatpak portal".into(),
        }),
        () = tokio::time::sleep(HANDOVER_LIMIT) => {
            if let Err(e) = portal.spawn_signal(pid, SIGTERM, true).await {
                tracing::debug!("could not stop the new version: {e}");
            }
            Err(format!("the new version didn't answer in {} s", HANDOVER_LIMIT.as_secs()))
        }
    }
}

#[cfg(not(target_os = "linux"))]
async fn hand_over(
    _app: &AppHandle,
    _activation_token: Option<String>,
) -> std::result::Result<(), String> {
    Err("restarting into an update needs the Flatpak portal".into())
}

/// The new instance answered, through the single-instance callback. On
/// zbus's thread: this mustn't block.
pub fn answered(app: &AppHandle, handover: Handover) {
    let waiting = app
        .try_state::<Desktop>()
        .and_then(|desktop| desktop.update.handover.lock().unwrap().take());
    match waiting {
        Some(restart) => {
            let _ = restart.send(handover);
        }
        None => tracing::info!("a restart answered ({handover:?}) that nobody waits for"),
    }
}

/// Whether this process, started by a restart, can run here (see the
/// module's doc). On the main thread, before Tauri starts: GTK is set up
/// here, and Tauri's own setup then finds it done.
pub fn startable() -> std::result::Result<(), Refusal> {
    let manifest = std::fs::read_to_string(MANIFEST).ok();
    let flatpak_info = std::fs::read_to_string(FLATPAK_INFO).ok();
    if !runtime_matches(manifest.as_deref(), flatpak_info.as_deref()) {
        tracing::warn!(
            "built for runtime {:?}, but started on {:?}: leaving the previous version running",
            manifest.as_deref().and_then(built_for),
            flatpak_info.as_deref().and_then(running_runtime),
        );
        return Err(Refusal::Runtime);
    }
    #[cfg(target_os = "linux")]
    if let Err(e) = gtk::init() {
        tracing::warn!("can't open the display ({e}): leaving the previous version running");
        return Err(Refusal::Display);
    }
    Ok(())
}

/// Tells the previous instance how `startable` went, and when ready, waits
/// for it to let go of the single-instance name: then this one continues
/// as a normal launch, which takes the name.
#[cfg(target_os = "linux")]
pub async fn take_over(previous: &str, answer: std::result::Result<(), Refusal>) -> TakeOver {
    let connection = async {
        zbus::connection::Builder::session()?
            .method_timeout(CALLBACK_LIMIT)
            .build()
            .await
    };
    let connection = match connection.await {
        Ok(connection) => connection,
        Err(e) => {
            // The previous one gives up waiting, and keeps running.
            tracing::warn!("can't reach the previous version ({e}); exiting");
            return TakeOver::Exit;
        }
    };
    let said = connection
        .call_method(
            Some(SINGLE_INSTANCE_NAME),
            SINGLE_INSTANCE_PATH,
            Some(SINGLE_INSTANCE_INTERFACE),
            "ExecuteCallback",
            &(
                vec![
                    COMMAND,
                    if answer.is_ok() {
                        READY_ARG
                    } else {
                        REFUSED_ARG
                    },
                ],
                "",
            ),
        )
        .await;
    if let Err(refusal) = answer {
        let gone = said.as_ref().is_err_and(no_owner);
        let next = after_refusal(refusal, gone);
        match (&said, next) {
            (Err(_), TakeOver::Continue) => {
                tracing::warn!("the previous version is gone; running here anyway");
            }
            (Err(e), TakeOver::Exit) => {
                tracing::warn!("could not tell the previous version ({e}); exiting");
            }
            (Ok(_), _) => tracing::info!("told the previous version to keep running; exiting"),
        }
        return next;
    }
    if let Err(e) = said {
        // It returns at once when the previous one is gone; otherwise it
        // either lets go or stops this one.
        tracing::warn!("could not tell the previous version ({e}); waiting for it anyway");
    }
    let owner = || owner_of(&connection);
    if wait_for_release(owner, previous, RELEASE_LIMIT).await {
        tracing::info!("the previous version let go; taking over");
        TakeOver::Continue
    } else {
        tracing::warn!(
            "the previous version didn't let go in {} s; exiting",
            RELEASE_LIMIT.as_secs()
        );
        TakeOver::Exit
    }
}

#[cfg(not(target_os = "linux"))]
pub async fn take_over(_previous: &str, _answer: std::result::Result<(), Refusal>) -> TakeOver {
    TakeOver::Continue
}

/// The single-instance name's owner.
#[cfg(target_os = "linux")]
async fn owner_of(connection: &zbus::Connection) -> Owner {
    let asked = async {
        let bus = zbus::fdo::DBusProxy::new(connection).await?;
        let name = zbus::names::BusName::try_from(SINGLE_INSTANCE_NAME)?;
        bus.get_name_owner(name).await.map_err(zbus::Error::from)
    };
    match asked.await {
        Ok(owner) => Owner::Is(owner.to_string()),
        Err(e) if no_owner(&e) => Owner::None,
        Err(e) => {
            tracing::debug!("could not ask who owns {SINGLE_INSTANCE_NAME}: {e}");
            Owner::Unknown
        }
    }
}

/// The error says the name has no owner.
#[cfg(target_os = "linux")]
fn no_owner(e: &zbus::Error) -> bool {
    use zbus::fdo::Error as Fdo;

    match e {
        zbus::Error::MethodError(name, ..) => matches!(
            name.as_str(),
            "org.freedesktop.DBus.Error.NameHasNoOwner"
                | "org.freedesktop.DBus.Error.ServiceUnknown"
        ),
        zbus::Error::FDO(e) => matches!(**e, Fdo::NameHasNoOwner(_) | Fdo::ServiceUnknown(_)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::APP_ID;

    fn state(commit: &str, restart: Restart) -> UpdateState {
        installed(commit, restart)
    }

    #[test]
    fn a_report_says_what_was_installed() {
        let none = UpdateState::None;
        // Nothing new installed (the remote may have something: software
        // centers install it, and the next report says so).
        assert_eq!(after_report(&none, "a", "a"), UpdateState::None);
        assert_eq!(after_report(&none, "a", "b"), state("b", Restart::Idle));
        // Rolled back to the running version.
        let b = state("b", Restart::Idle);
        assert_eq!(after_report(&b, "a", "a"), UpdateState::None);
        // A portal that left the commit out says nothing new.
        assert_eq!(after_report(&none, "a", ""), UpdateState::None);
    }

    #[test]
    fn an_installed_update_stays_until_another_comes() {
        // The same commit again (the portal reports the remote's changes
        // too, and a new monitor reports again): unchanged, how the
        // restart went included.
        for restart in [Restart::Idle, Restart::Restarting, Restart::Failed] {
            let b = state("b", restart);
            assert_eq!(after_report(&b, "a", "b"), b);
        }
        // A newer one: not tried yet, unless a restart runs (which starts
        // the newest).
        assert_eq!(
            after_report(&state("b", Restart::Failed), "a", "c"),
            state("c", Restart::Idle)
        );
        assert_eq!(
            after_report(&state("b", Restart::Restarting), "a", "c"),
            state("c", Restart::Restarting)
        );
    }

    #[test]
    fn one_restart_at_a_time_and_another_after_a_failure() {
        let idle = state("b", Restart::Idle);
        let restarting = after_restart_asked(&idle);
        assert_eq!(restarting, state("b", Restart::Restarting));
        // Asked again while it runs: unchanged, so nothing more starts.
        assert_eq!(after_restart_asked(&restarting), restarting);
        let failed = after_restart_failed(&restarting);
        assert_eq!(failed, state("b", Restart::Failed));
        // It can be tried again.
        assert_eq!(after_restart_asked(&failed), restarting);
        // Nothing to restart into.
        assert_eq!(after_restart_asked(&UpdateState::None), UpdateState::None);
        // A failure only ends a restart.
        assert_eq!(after_restart_failed(&idle), idle);
        assert_eq!(after_restart_failed(&UpdateState::None), UpdateState::None);
    }

    #[test]
    fn a_restart_that_breaks_off_can_be_tried_again() {
        use futures::FutureExt;

        let updates = Updates::default();
        updates.change(|_| state("b", Restart::Idle));
        // As `restart` starts one: the state, the answer waited for.
        let start = || {
            updates.change(after_restart_asked);
            let (answered, answer) = oneshot::channel();
            *updates.handover.lock().unwrap() = Some(answered);
            (
                Unfinished::new(|| {
                    updates.end_restart();
                }),
                answer,
            )
        };
        let ended = |answer: &mut oneshot::Receiver<Handover>| {
            assert_eq!(updates.state(), state("b", Restart::Failed));
            assert!(updates.handover.lock().unwrap().is_none());
            assert!(answer.try_recv().is_err());
        };

        // A panic on the way (as a variable that isn't UTF-8 once caused).
        let mut answer = None;
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let (_unfinished, waiting) = start();
            answer = Some(waiting);
            panic!("on the way");
        }));
        assert!(panicked.is_err());
        ended(answer.as_mut().unwrap());

        // Its task dropped on the way.
        let mut answer = None;
        let restarting = async {
            let (_unfinished, waiting) = start();
            answer = Some(waiting);
            std::future::pending::<()>().await;
        };
        assert_eq!(restarting.now_or_never(), None);
        ended(answer.as_mut().unwrap());

        // Taken over: it's the new version's now.
        let (unfinished, _answer) = start();
        unfinished.took_over();
        assert_eq!(updates.state(), state("b", Restart::Restarting));
    }

    #[test]
    fn retrying_to_watch_backs_off() {
        let minutes = |attempt| retry_after(attempt).as_secs() / 60;
        assert_eq!(minutes(0), 1);
        assert_eq!(minutes(1), 5);
        assert_eq!(minutes(2), 30);
        assert_eq!(minutes(9), 30);
        assert_eq!(minutes(u32::MAX), 30);
    }

    #[test]
    fn the_restart_arguments_round_trip() {
        let args = restart_args(":1.42");
        assert_eq!(args, ["cumulas-mail", "--restarted-from=:1.42"]);
        assert!(
            !args
                .iter()
                .any(|a| a == crate::desktop::background::BACKGROUND_ARG)
        );
        let launched = |args: &[&str]| restarted_from(args.iter().map(|a| a.to_string()));
        assert_eq!(
            launched(&args.iter().map(String::as_str).collect::<Vec<_>>()).as_deref(),
            Some(":1.42")
        );
        assert_eq!(
            launched(&["cumulas-mail", "--verbose", "--restarted-from=:1.7_x-y"]).as_deref(),
            Some(":1.7_x-y")
        );
        for normal in [
            &["cumulas-mail"][..],
            &["cumulas-mail", "--background"],
            &["cumulas-mail", "--restarted-from"],
            &["cumulas-mail", "--restarted-from="],
            &["cumulas-mail", "--restarted-from=co.cumulas.Mail"],
            &["cumulas-mail", "--restarted-from=:"],
            &["cumulas-mail", "--restarted-from=:1 2"],
            &["cumulas-mail", "--restarted-from=:1.42;rm"],
            &["cumulas-mail", "--restarted-fromage=:1.42"],
        ] {
            assert_eq!(launched(normal), None, "{normal:?}");
        }
    }

    #[test]
    fn handover_reads_only_its_own_arguments() {
        let argv = |args: &[&str]| args.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        assert_eq!(
            handover(&argv(&["cumulas-mail", "--restart-ready"])),
            Some(Handover::Ready)
        );
        assert_eq!(
            handover(&argv(&["cumulas-mail", "--restart-refused"])),
            Some(Handover::Refused)
        );
        for launch in [
            &[][..],
            &["cumulas-mail"],
            &["cumulas-mail", "--background"],
            &["cumulas-mail", "--restarted-from=:1.42"],
            &["--restart-ready"],
            &["cumulas-mail", "--restart-ready=1"],
        ] {
            assert_eq!(handover(&argv(launch)), None, "{launch:?}");
        }
    }

    #[test]
    fn the_new_instance_keeps_the_language_and_input_method() {
        let vars = [
            ("PATH", "/app/bin:/usr/bin"),
            ("LANG", "de_DE.UTF-8"),
            ("DISPLAY", ":0"),
            ("LC_TIME", "en_GB.UTF-8"),
            ("WAYLAND_DISPLAY", "wayland-0"),
            ("LANGUAGE", "de:en"),
            ("XDG_ACTIVATION_TOKEN", "stale"),
            ("GTK_IM_MODULE", "ibus"),
            ("LCX", "no"),
            ("QT_IM_MODULE", "ibus"),
            ("XMODIFIERS", "@im=ibus"),
        ]
        .map(|(k, v)| (OsString::from(k), OsString::from(v)));
        let carried = carried_env(vars);
        let names: Vec<&str> = carried.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            names,
            [
                "GTK_IM_MODULE",
                "LANG",
                "LANGUAGE",
                "LC_TIME",
                "QT_IM_MODULE",
                "XMODIFIERS"
            ]
        );
        assert!(carried.contains(&("LANG".into(), "de_DE.UTF-8".into())));
    }

    #[cfg(unix)]
    #[test]
    fn a_variable_that_isnt_utf8_stays_behind() {
        use std::os::unix::ffi::OsStringExt;

        // A Latin-1 value, say, from an old shell profile.
        let latin1 = |text: &[u8]| OsString::from_vec(text.to_vec());
        let carried = carried_env([
            (OsString::from("LANG"), latin1(b"de_DE.ISO-8859-1\xe4")),
            (latin1(b"LC_\xff"), OsString::from("C")),
            (OsString::from("PATH"), latin1(b"/home/j\xf6rg/bin")),
            (OsString::from("LC_TIME"), OsString::from("en_GB.UTF-8")),
        ]);
        assert_eq!(carried, [("LC_TIME".to_owned(), "en_GB.UTF-8".to_owned())]);
    }

    #[test]
    fn the_single_instance_name_is_the_plugins() {
        // The plugin names it after the identifier in tauri.conf.json.
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        assert_eq!(config["identifier"], APP_ID);
        assert_eq!(SINGLE_INSTANCE_NAME, format!("{APP_ID}.SingleInstance"));
        let path = format!(
            "/{}",
            SINGLE_INSTANCE_NAME.replace('.', "/").replace('-', "_")
        );
        assert_eq!(SINGLE_INSTANCE_PATH, path);
    }

    #[test]
    fn the_gtk_app_id_stays_off() {
        // With it on, GTK would make the new instance a remote of the
        // previous one's GApplication, which returns at once.
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        let app_id = &config["app"]["enableGTKAppId"];
        assert!(app_id.is_null() || app_id == false, "{app_id}");
    }

    #[tokio::test(start_paused = true)]
    async fn waiting_for_the_previous_instance() {
        use std::sync::atomic::{AtomicU32, Ordering};

        /// Answers `first` `times` times, then `then`.
        fn owner(
            first: Owner,
            times: u32,
            then: Owner,
        ) -> impl FnMut() -> std::future::Ready<Owner> {
            let asked = AtomicU32::new(0);
            move || {
                let n = asked.fetch_add(1, Ordering::Relaxed);
                std::future::ready(if n < times {
                    first.clone()
                } else {
                    then.clone()
                })
            }
        }
        let limit = Duration::from_secs(60);
        let previous = || Owner::Is(":1.42".into());
        let start = tokio::time::Instant::now();
        assert!(wait_for_release(owner(previous(), 3, Owner::None), ":1.42", limit).await);
        assert_eq!(start.elapsed(), RELEASE_POLL * 3);
        // A failed call doesn't count as let go.
        assert!(wait_for_release(owner(Owner::Unknown, 5, Owner::None), ":1.42", limit).await);
        // Someone else has it (a launch in between): this one hands off to it.
        let start = tokio::time::Instant::now();
        let other = Owner::Is(":1.99".into());
        assert!(wait_for_release(owner(other.clone(), 1, other), ":1.42", limit).await);
        assert_eq!(start.elapsed(), Duration::ZERO);
        // Never let go.
        let start = tokio::time::Instant::now();
        assert!(!wait_for_release(owner(previous(), u32::MAX, previous()), ":1.42", limit).await);
        assert!(start.elapsed() >= limit);
    }

    #[test]
    fn the_runtime_comes_from_flatpak_info() {
        let info = "[Application]\n\
                    name=co.cumulas.Mail\n\
                    runtime=runtime/org.gnome.Platform/x86_64/50\n\
                    \n\
                    [Instance]\n\
                    instance-id=1\n\
                    runtime=runtime/org.gnome.Platform/x86_64/49\n";
        assert_eq!(running_runtime(info), Some(("org.gnome.Platform", "50")));
        // Only under [Application].
        let elsewhere = "[Instance]\nruntime=runtime/org.gnome.Platform/x86_64/49\n";
        assert_eq!(running_runtime(elsewhere), None);
        assert_eq!(
            running_runtime("[Application]\nruntime=org.gnome.Platform\n"),
            None
        );
        assert_eq!(running_runtime(""), None);
    }

    #[test]
    fn the_build_says_which_runtime_it_is_for() {
        let manifest = r#"{"id": "co.cumulas.Mail", "runtime": "org.gnome.Platform",
                           "runtime-version": "50", "sdk": "org.gnome.Sdk"}"#;
        let info = |branch: &str| {
            format!("[Application]\nruntime=runtime/org.gnome.Platform/x86_64/{branch}\n")
        };
        assert_eq!(
            built_for(manifest),
            Some((Some("org.gnome.Platform".into()), "50".into()))
        );
        assert!(runtime_matches(Some(manifest), Some(&info("50"))));
        // Started on the previous version's runtime.
        assert!(!runtime_matches(Some(manifest), Some(&info("49"))));
        let other = info("50").replace("org.gnome.Platform", "org.kde.Platform");
        assert!(!runtime_matches(Some(manifest), Some(&other)));
        // Without a runtime id, the branch decides.
        let branch_only = r#"{"runtime-version": "50"}"#;
        assert!(runtime_matches(Some(branch_only), Some(&other)));
        assert!(!runtime_matches(Some(branch_only), Some(&info("49"))));
        // Whatever isn't known isn't checked: no manifest (a build without
        // flatpak-builder), none of its runtime version, no Flatpak.
        assert!(runtime_matches(None, Some(&info("49"))));
        assert!(runtime_matches(
            Some(r#"{"runtime": "org.gnome.Platform"}"#),
            Some(&info("49"))
        ));
        assert!(runtime_matches(Some("not json"), Some(&info("49"))));
        assert!(runtime_matches(Some(manifest), None));
    }

    #[test]
    fn a_refused_restart_runs_only_on_the_wrong_runtime_and_only_alone() {
        assert_eq!(after_refusal(Refusal::Runtime, false), TakeOver::Exit);
        assert_eq!(after_refusal(Refusal::Display, false), TakeOver::Exit);
        assert_eq!(after_refusal(Refusal::Runtime, true), TakeOver::Continue);
        assert_eq!(after_refusal(Refusal::Display, true), TakeOver::Exit);
    }

    #[test]
    fn the_update_notification_restarts() {
        let toast = update_toast();
        assert_eq!(toast.id, "app:update");
        assert_eq!(toast.kind, ToastKind::App);
        assert_eq!(toast.title, "Update installed");
        assert_eq!(toast.body, "Restart Cumulas Mail to use the new version.");
        assert_eq!(toast.default_action, Action::ShowWindow);
        assert_eq!(
            toast.buttons,
            [("Restart".to_owned(), Action::RestartToUpdate)]
        );
        assert!(!toast.sound);

        let failed = restart_failed_toast();
        assert_eq!(failed.id, toast.id);
        assert_eq!(failed.title, "Couldn't restart");
        assert_eq!(
            failed.body,
            "Try again, or quit Cumulas Mail and open it again."
        );
        assert_eq!(
            failed.buttons,
            [("Try again".to_owned(), Action::RestartToUpdate)]
        );
        assert!(!failed.sound);
    }

    /// The shapes `src/api/types.ts` mirrors.
    #[test]
    fn the_state_reads_as_the_page_expects() {
        let json = |state: UpdateState| serde_json::to_value(state).unwrap();
        assert_eq!(
            json(UpdateState::None),
            serde_json::json!({ "kind": "none" })
        );
        assert_eq!(
            json(state("b", Restart::Idle)),
            serde_json::json!({ "kind": "installed", "commit": "b", "restart": "idle" })
        );
        assert_eq!(
            json(state("b", Restart::Restarting))["restart"],
            "restarting"
        );
        assert_eq!(json(state("b", Restart::Failed))["restart"], "failed");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn watching_gives_up_only_where_it_can_never_work() {
        use zbus::fdo::Error as Fdo;

        let method_error = |name: &str, detail: &str| {
            let call = zbus::Message::method_call("/", "CreateUpdateMonitor")
                .unwrap()
                .build(&())
                .unwrap();
            let reply = zbus::Message::error(&call.header(), name)
                .unwrap()
                .build(&(detail,))
                .unwrap();
            ashpd::Error::Portal(ashpd::PortalError::ZBus(zbus::Error::from(reply)))
        };
        let fdo = |e: Fdo| ashpd::Error::Zbus(zbus::Error::FDO(Box::new(e)));
        let never = [
            ashpd::Error::PortalNotFound(PORTAL.try_into().unwrap()),
            fdo(Fdo::ServiceUnknown("The name is not activatable".into())),
            // Not a Flatpak app, as far as the portal can tell.
            method_error(
                "org.freedesktop.DBus.Error.NotSupported",
                "Updates only supported by flatpak apps",
            ),
            method_error(
                "org.freedesktop.DBus.Error.Failed",
                "Incomplete instance info for update monitor",
            ),
            fdo(Fdo::NotSupported(
                "Updates only supported by flatpak apps".into(),
            )),
        ];
        for e in &never {
            assert!(gives_up(e), "{e}");
        }
        let later = [
            // ashpd reads the version as 1 when reading it fails.
            ashpd::Error::RequiresVersion(2, 1),
            method_error("org.freedesktop.DBus.Error.Failed", "Something else"),
            method_error("org.freedesktop.DBus.Error.NoReply", "Timeout"),
            fdo(Fdo::Failed("Something else".into())),
            ashpd::Error::Zbus(zbus::Error::InputOutput(std::sync::Arc::new(
                std::io::Error::other("broken pipe"),
            ))),
        ];
        for e in &later {
            assert!(!gives_up(e), "{e}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_name_without_owner_is_told_apart() {
        use zbus::fdo::Error as Fdo;

        assert!(no_owner(&zbus::Error::FDO(Box::new(Fdo::NameHasNoOwner(
            "no".into()
        )))));
        assert!(no_owner(&zbus::Error::FDO(Box::new(Fdo::ServiceUnknown(
            "no".into()
        )))));
        assert!(!no_owner(&zbus::Error::FDO(Box::new(Fdo::NoReply(
            "slow".into()
        )))));
    }
}
