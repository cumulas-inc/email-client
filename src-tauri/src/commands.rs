//! Commands invoked from the UI (`src/api/tauri.ts`). They are thin wrappers
//! around [`cumulas_core::CoreService`]; argument names arrive in camelCase.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use cumulas_core::model::{
    Account, AccountSignature, ActiveReminder, AdminConsentLink, Calendar, ComposeSignature,
    Contact, ContactDraft, EmailAddress, Event, EventDraft, EventResponse, Folder,
    ImapAccountUpdate, Message, MessagePage, MessageQuery, NewImapAccount, OutgoingMessage,
    PermissionRequest, ReplyKind, SearchHit, SearchOptions, ServerSearchResult, Settings, Snooze,
    SyncStatus, Task, TaskDraft, TaskList, TaskPatch,
};
use cumulas_core::push::PushStatus;
use cumulas_core::service::builtin_microsoft_client_id;
use cumulas_core::{Error, Result};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::AppState;
use crate::desktop::background::{self, BackgroundState};
use crate::desktop::router::Navigate;
use crate::desktop::update::{self, UpdateState};
use crate::desktop::{self as desktop_shell, Desktop, notify, tray};

/// Runs store work off the main thread and off the async workers: a sync
/// holding the store lock must not freeze the window.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| Error::internal(format!("background task failed: {e}")))?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: String,
    /// Passwords and tokens survive restarts (a Secret Service is available).
    persistent_secrets: bool,
    /// A Microsoft client ID is built in (the published registration's,
    /// unless the build named another or `none`), so Microsoft sign-in
    /// works without configuration.
    microsoft_client_id_built_in: bool,
    /// The host of the push service Microsoft accounts use (normally
    /// `sync.cumulas.co`), for the privacy note.
    sync_server: String,
    /// The host of the service company signatures come from (normally
    /// `signature.cumulas.co`); `None` when company signatures are off
    /// (`CUMULAS_SIGNATURE_CLIENT_ID` is `none`, or another service has no
    /// client ID).
    signature_server: Option<String>,
    /// Started by a restart into an update: the page says it's updated.
    restarted_to_update: bool,
}

#[tauri::command]
pub fn get_app_info(app: AppHandle, state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        persistent_secrets: state.persistent_secrets,
        microsoft_client_id_built_in: builtin_microsoft_client_id().is_some(),
        sync_server: cumulas_core::push::relay::sync_host(),
        signature_server: cumulas_core::signatures::config::signature_host(),
        restarted_to_update: state.restarted_to_update,
    }
}

/// Opens a link from a message or event in the user's browser / mail handler.
#[tauri::command]
pub fn open_link(app: AppHandle, url: String) -> Result<()> {
    let parsed = url::Url::parse(&url).map_err(|_| Error::invalid_input("Invalid link."))?;
    if !matches!(parsed.scheme(), "http" | "https" | "mailto") {
        return Err(Error::invalid_input(
            "Only web and email links can be opened.",
        ));
    }
    app.opener()
        .open_url(parsed.as_str(), None::<&str>)
        .map_err(|e| Error::internal(format!("could not open link: {e}")))
}

/// Quits, even while running in the background is on: the main menu's
/// Quit, and the page's answer to the tray's (`quit-requested`).
#[tauri::command]
pub fn quit_app(app: AppHandle, desktop: State<'_, Desktop>) {
    desktop.window.forget_quit();
    app.exit(0);
}

/// The user kept the unsent message when asked whether to quit (or
/// restart): the tray's next Quit asks again, rather than quitting at once.
#[tauri::command]
pub fn quit_canceled(desktop: State<'_, Desktop>) {
    desktop.window.forget_quit();
    desktop.update.take_activation_token();
}

// --- accounts & settings ------------------------------------------------------

#[tauri::command]
pub async fn list_accounts(state: State<'_, AppState>) -> Result<Vec<Account>> {
    let service = state.service.clone();
    blocking(move || service.list_accounts()).await
}

#[tauri::command]
pub async fn add_imap_account(
    state: State<'_, AppState>,
    input: NewImapAccount,
) -> Result<Account> {
    let account = state.service.add_imap_account(input).await?;
    spawn_sync(&state, &account.id);
    Ok(account)
}

/// Signs in to a Microsoft account in the browser, asking for `permissions`
/// (everything when `null`), then syncs it.
#[tauri::command]
pub async fn add_microsoft_account(
    app: AppHandle,
    state: State<'_, AppState>,
    login_hint: Option<String>,
    permissions: Option<PermissionRequest>,
) -> Result<Account> {
    let opener = app.clone();
    let open_browser = move |url: &str| {
        opener
            .opener()
            .open_url(url, None::<&str>)
            .map_err(|e| Error::internal(format!("could not open the browser: {e}")))
    };
    let account = state
        .service
        .add_microsoft_account(
            &open_browser,
            login_hint.as_deref(),
            permissions.unwrap_or_default(),
        )
        .await?;
    spawn_sync(&state, &account.id);
    Ok(account)
}

/// Stops waiting for a Microsoft sign-in the user abandoned in the browser.
#[tauri::command]
pub fn cancel_microsoft_sign_in(state: State<'_, AppState>) {
    state.service.cancel_microsoft_sign_in();
}

/// The link an administrator approves Cumulas Mail with, for an account's
/// organization or the one of the address being signed in with.
#[tauri::command]
pub async fn microsoft_admin_consent(
    state: State<'_, AppState>,
    account_id: Option<String>,
    login_hint: Option<String>,
) -> Result<AdminConsentLink> {
    let service = state.service.clone();
    blocking(move || service.microsoft_admin_consent(account_id.as_deref(), login_hint.as_deref()))
        .await
}

/// Checks a Microsoft account's refused features again ("Check again"),
/// then syncs it.
#[tauri::command]
pub async fn retry_microsoft_features(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<()> {
    let service = state.service.clone();
    let id = account_id.clone();
    blocking(move || service.retry_microsoft_features(&id)).await?;
    spawn_sync(&state, &account_id);
    Ok(())
}

/// Updates an IMAP account's password and/or servers, then syncs it.
#[tauri::command]
pub async fn update_imap_account(
    state: State<'_, AppState>,
    account_id: String,
    update: ImapAccountUpdate,
) -> Result<Account> {
    let account = state
        .service
        .update_imap_account(&account_id, update)
        .await?;
    spawn_sync(&state, &account.id);
    Ok(account)
}

#[tauri::command]
pub async fn update_account(
    state: State<'_, AppState>,
    account_id: String,
    display_name: Option<String>,
    color: Option<String>,
) -> Result<Account> {
    state
        .service
        .update_account(&account_id, display_name, color)
        .await
}

#[tauri::command]
pub async fn remove_account(state: State<'_, AppState>, account_id: String) -> Result<()> {
    state.service.remove_account(&account_id).await
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<Settings> {
    let service = state.service.clone();
    blocking(move || service.get_settings()).await
}

/// Installed font families for the font settings (scanned once, then cached).
#[tauri::command]
pub async fn list_fonts() -> Vec<String> {
    crate::fonts::installed_families().await
}

/// Saves the settings. Running in the background and starting at login
/// change only through their own commands (which also tell the system), so
/// those fields keep their saved values here.
#[tauri::command]
pub async fn update_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    mut settings: Settings,
) -> Result<Settings> {
    let before = state.service.get_settings()?;
    settings.run_in_background = before.run_in_background;
    settings.start_at_login = before.start_at_login;
    settings.start_hidden = before.start_hidden;
    let saved = state.service.update_settings(settings).await?;
    if saved.show_tray_icon != before.show_tray_icon {
        tray::sync(&app);
        background::announce(&app).await;
    }
    if saved.notifications_paused_until != before.notifications_paused_until {
        tray::pause_changed(&app);
    }
    Ok(saved)
}

fn spawn_sync(state: &State<'_, AppState>, account_id: &str) {
    let service = state.service.clone();
    let account_id = account_id.to_owned();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = service.sync_account(&account_id).await {
            tracing::warn!("sync failed: {e}");
        }
    });
}

/// Starts a sync in the background; progress arrives as `sync-status` events.
#[tauri::command]
pub fn sync_account(state: State<'_, AppState>, account_id: String) {
    spawn_sync(&state, &account_id);
}

#[tauri::command]
pub fn sync_all(state: State<'_, AppState>) {
    let service = state.service.clone();
    tauri::async_runtime::spawn(async move { service.sync_all().await });
}

#[tauri::command]
pub async fn get_sync_statuses(state: State<'_, AppState>) -> Result<Vec<SyncStatus>> {
    let service = state.service.clone();
    blocking(move || service.sync_statuses()).await
}

/// How each account hears about new mail; changes arrive as `push-status`
/// events.
#[tauri::command]
pub async fn get_push_statuses(state: State<'_, AppState>) -> Result<Vec<PushStatus>> {
    let service = state.service.clone();
    blocking(move || service.push_statuses()).await
}

// --- desktop --------------------------------------------------------------------

/// Running in the background, starting at login, the tray and how
/// notifications are shown; changes arrive as `background-state` events.
#[tauri::command]
pub async fn get_background_state(app: AppHandle) -> BackgroundState {
    background::state(&app).await
}

/// Keeps the app running when its window closes (asking the system first
/// in a Flatpak).
#[tauri::command]
pub async fn set_run_in_background(app: AppHandle, enabled: bool) -> Result<BackgroundState> {
    background::set_run_in_background(&app, enabled).await
}

/// Starts the app at login; `hidden` starts it without a window.
#[tauri::command]
pub async fn set_start_at_login(
    app: AppHandle,
    enabled: bool,
    hidden: bool,
) -> Result<BackgroundState> {
    background::set_start_at_login(&app, enabled, hidden).await
}

/// Shows a notification like new mail's, so the user can see what they
/// look like (and whether the desktop lets them through).
#[tauri::command]
pub async fn send_test_notification(app: AppHandle) -> Result<()> {
    desktop_shell::test_notification(&app)
        .await
        .map_err(|e| Error::unsupported(format!("Couldn't show a notification. {e}")))
}

/// What the Flatpak portal said about updates; changes arrive as
/// `update-state` events.
#[tauri::command]
pub fn get_update_state(desktop: State<'_, Desktop>) -> UpdateState {
    desktop.update.state()
}

/// Restarts into the installed update: the page's Restart, and its answer
/// to the tray's or a notification's (`restart-requested`). The app exits
/// once the new version has taken over; when it couldn't, it keeps running.
#[tauri::command]
pub async fn restart_app(app: AppHandle) -> Result<()> {
    let desktop = app.state::<Desktop>();
    desktop.window.forget_quit();
    let activation_token = desktop.update.take_activation_token();
    update::restart(&app, activation_token).await
}

/// Navigation that waited for the page (a notification was clicked while
/// the app ran without a window). The page calls this once it listens for
/// `navigate` events.
#[tauri::command]
pub fn take_pending_navigation(desktop: State<'_, Desktop>) -> Option<Navigate> {
    desktop.router.take_pending()
}

// --- reminders --------------------------------------------------------------

/// The reminders due now, or snoozed.
#[tauri::command]
pub async fn list_active_reminders(state: State<'_, AppState>) -> Result<Vec<ActiveReminder>> {
    let service = state.service.clone();
    blocking(move || service.list_active_reminders()).await
}

/// Snoozes a reminder for `minutes`, or until `until`; its notification
/// goes. Returns the reminders then active.
#[tauri::command]
pub async fn snooze_reminder(
    state: State<'_, AppState>,
    desktop: State<'_, Desktop>,
    reminder_key: String,
    minutes: Option<u32>,
    until: Option<DateTime<Utc>>,
) -> Result<Vec<ActiveReminder>> {
    let snooze = match (minutes, until) {
        (Some(minutes), None) => Snooze::Minutes(minutes),
        (None, Some(until)) => Snooze::Until(until),
        _ => {
            return Err(Error::invalid_input(
                "Snooze for some minutes, or until a time.",
            ));
        }
    };
    let active = state.service.snooze_reminder(&reminder_key, snooze).await?;
    desktop
        .notifications
        .withdraw(&notify::reminder_toast_id(&reminder_key))
        .await;
    Ok(active)
}

/// Dismisses reminders; their notifications go. Returns the reminders then
/// active.
#[tauri::command]
pub async fn dismiss_reminders(
    state: State<'_, AppState>,
    desktop: State<'_, Desktop>,
    reminder_keys: Vec<String>,
) -> Result<Vec<ActiveReminder>> {
    let active = state.service.dismiss_reminders(&reminder_keys).await?;
    for key in &reminder_keys {
        desktop
            .notifications
            .withdraw(&notify::reminder_toast_id(key))
            .await;
    }
    Ok(active)
}

// --- signatures -------------------------------------------------------------------

/// The account's own signature and what its organization last said about
/// its own; changes arrive as `data-changed` events with `signatures`.
#[tauri::command]
pub async fn get_account_signature(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<AccountSignature> {
    let service = state.service.clone();
    blocking(move || service.get_account_signature(&account_id)).await
}

/// Saves the account's own signature, sanitized; more than 512 KB is
/// refused (`invalidInput`).
#[tauri::command]
pub async fn update_own_signature(
    state: State<'_, AppState>,
    account_id: String,
    html: String,
    on_replies: bool,
) -> Result<AccountSignature> {
    state
        .service
        .update_own_signature(&account_id, &html, on_replies)
}

/// Asks the account's organization for its signature now, unless it was
/// asked in the last minute (or the service asked to wait): the result's
/// `organization.checkableAt` says until when. A failure for the network
/// or the service isn't an error: the result says what was known, marked
/// stale.
#[tauri::command]
pub async fn check_organization_signature(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<AccountSignature> {
    state
        .service
        .check_organization_signature(&account_id)
        .await
}

/// The signature a new message (`replyKind` null), reply or forward from
/// the account starts with.
#[tauri::command]
pub async fn get_compose_signature(
    state: State<'_, AppState>,
    account_id: String,
    reply_kind: Option<ReplyKind>,
) -> Result<ComposeSignature> {
    state
        .service
        .get_compose_signature(&account_id, reply_kind)
        .await
}

// --- mail -----------------------------------------------------------------------

#[tauri::command]
pub async fn list_folders(
    state: State<'_, AppState>,
    account_id: Option<String>,
) -> Result<Vec<Folder>> {
    let service = state.service.clone();
    blocking(move || service.list_folders(account_id.as_deref())).await
}

#[tauri::command]
pub async fn list_messages(
    state: State<'_, AppState>,
    folder_id: String,
    query: MessageQuery,
) -> Result<MessagePage> {
    let service = state.service.clone();
    blocking(move || service.list_messages(&folder_id, &query)).await
}

/// Messages on this device matching `text` (operators and filters such as
/// `from:` or `is:unread` included), and the messages `includeIds` names
/// (those server searches found) that pass its filters. Best first, unless
/// `options.sort` is newest or the query has no words to rank by; rows
/// whose text matched carry a snippet of it.
#[tauri::command]
pub async fn search_messages(
    state: State<'_, AppState>,
    text: String,
    account_id: Option<String>,
    include_ids: Option<Vec<String>>,
    options: Option<SearchOptions>,
) -> Result<Vec<SearchHit>> {
    let service = state.service.clone();
    blocking(move || {
        service.search_messages(
            &text,
            account_id.as_deref(),
            include_ids.as_deref().unwrap_or_default(),
            options.unwrap_or_default(),
        )
    })
    .await
}

/// Searches one account's whole mailbox on its server (the first pages, or
/// the next with `more`) and stores what it finds that passes the query's
/// filters; a call may read several pages to find enough. A new `searchId`
/// cancels the previous search. The server's failures come back in the
/// result; only an unknown account or a bad search id is an error.
#[tauri::command]
pub async fn search_server(
    state: State<'_, AppState>,
    search_id: String,
    text: String,
    account_id: String,
    more: Option<bool>,
    options: Option<SearchOptions>,
) -> Result<ServerSearchResult> {
    state
        .service
        .search_server(
            &search_id,
            &text,
            &account_id,
            more.unwrap_or(false),
            options.unwrap_or_default(),
        )
        .await
}

/// Cancels the server search if it is the current one. Only takes the
/// search registry's lock, never the store's.
#[tauri::command]
pub fn cancel_server_search(state: State<'_, AppState>, search_id: String) {
    state.service.cancel_server_search(&search_id);
}

#[tauri::command]
pub async fn get_message(
    state: State<'_, AppState>,
    message_id: String,
    allow_remote: Option<bool>,
) -> Result<Message> {
    state
        .service
        .get_message(&message_id, allow_remote.unwrap_or(false))
        .await
}

#[tauri::command]
pub async fn set_read(
    state: State<'_, AppState>,
    message_ids: Vec<String>,
    read: bool,
) -> Result<()> {
    state.service.set_read(&message_ids, read).await
}

#[tauri::command]
pub async fn set_flagged(
    state: State<'_, AppState>,
    message_ids: Vec<String>,
    flagged: bool,
) -> Result<()> {
    state.service.set_flagged(&message_ids, flagged).await
}

#[tauri::command]
pub async fn move_messages(
    state: State<'_, AppState>,
    message_ids: Vec<String>,
    folder_id: String,
) -> Result<()> {
    state.service.move_messages(&message_ids, &folder_id).await
}

#[tauri::command]
pub async fn archive_messages(state: State<'_, AppState>, message_ids: Vec<String>) -> Result<()> {
    state.service.archive_messages(&message_ids).await
}

#[tauri::command]
pub async fn delete_messages(state: State<'_, AppState>, message_ids: Vec<String>) -> Result<()> {
    state.service.delete_messages(&message_ids).await
}

#[tauri::command]
pub async fn send_message(state: State<'_, AppState>, message: OutgoingMessage) -> Result<()> {
    let account_id = message.account_id.clone();
    state.service.send_message(message).await?;
    // Bring the copy in Sent into view without waiting for the next sync.
    spawn_sync(&state, &account_id);
    Ok(())
}

/// People to suggest for `text`, contacts first; without text, the people
/// who wrote most lately.
#[tauri::command]
pub async fn suggest_addresses(
    state: State<'_, AppState>,
    text: String,
) -> Result<Vec<EmailAddress>> {
    let service = state.service.clone();
    blocking(move || service.suggest_addresses(&text)).await
}

/// Strips anything that could make an attachment name escape its directory.
fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_start_matches('.');
    if trimmed.is_empty() {
        "attachment".into()
    } else {
        trimmed.chars().take(200).collect()
    }
}

/// Asks where to save an attachment (through the file chooser portal) and
/// writes it there. Returns the saved path, or `None` if canceled.
#[tauri::command]
pub async fn save_attachment(
    app: AppHandle,
    state: State<'_, AppState>,
    message_id: String,
    attachment_id: String,
) -> Result<Option<String>> {
    let attachment = state
        .service
        .get_attachment(&message_id, &attachment_id)
        .await?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(safe_file_name(&attachment.meta.name))
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(path) = rx.await.ok().flatten() else {
        return Ok(None);
    };
    let path: PathBuf = path
        .into_path()
        .map_err(|e| Error::internal(format!("unsupported save location: {e}")))?;
    tokio::fs::write(&path, &attachment.data).await?;
    Ok(Some(path.display().to_string()))
}

/// Opens an attachment with the default application for its type.
#[tauri::command]
pub async fn open_attachment(
    app: AppHandle,
    state: State<'_, AppState>,
    message_id: String,
    attachment_id: String,
) -> Result<()> {
    let attachment = state
        .service
        .get_attachment(&message_id, &attachment_id)
        .await?;
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| Error::internal(e.to_string()))?
        .join("attachments")
        .join(&message_id);
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join(safe_file_name(&attachment.meta.name));
    tokio::fs::write(&path, &attachment.data).await?;
    app.opener()
        .open_path(path.display().to_string(), None::<&str>)
        .map_err(|e| Error::internal(format!("could not open the attachment: {e}")))
}

// --- calendar -------------------------------------------------------------------

#[tauri::command]
pub async fn list_calendars(state: State<'_, AppState>) -> Result<Vec<Calendar>> {
    let service = state.service.clone();
    blocking(move || service.list_calendars()).await
}

#[tauri::command]
pub async fn list_events(
    state: State<'_, AppState>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    calendar_ids: Option<Vec<String>>,
) -> Result<Vec<Event>> {
    let service = state.service.clone();
    blocking(move || service.list_events(start, end, calendar_ids.as_deref())).await
}

#[tauri::command]
pub async fn refresh_events(
    state: State<'_, AppState>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<()> {
    state.service.refresh_events(start, end).await
}

#[tauri::command]
pub async fn create_event(state: State<'_, AppState>, draft: EventDraft) -> Result<Event> {
    state.service.create_event(draft).await
}

#[tauri::command]
pub async fn update_event(
    state: State<'_, AppState>,
    event_id: String,
    draft: EventDraft,
) -> Result<Event> {
    state.service.update_event(&event_id, draft).await
}

#[tauri::command]
pub async fn delete_event(state: State<'_, AppState>, event_id: String) -> Result<()> {
    state.service.delete_event(&event_id).await
}

#[tauri::command]
pub async fn respond_to_event(
    state: State<'_, AppState>,
    event_id: String,
    response: EventResponse,
    comment: Option<String>,
    send_response: bool,
) -> Result<Event> {
    state
        .service
        .respond_to_event(&event_id, response, comment, send_response)
        .await
}

// --- contacts -------------------------------------------------------------------

#[tauri::command]
pub async fn list_contacts(
    state: State<'_, AppState>,
    account_id: Option<String>,
    text: Option<String>,
) -> Result<Vec<Contact>> {
    let service = state.service.clone();
    blocking(move || service.list_contacts(account_id.as_deref(), text.as_deref())).await
}

#[tauri::command]
pub async fn create_contact(state: State<'_, AppState>, draft: ContactDraft) -> Result<Contact> {
    state.service.create_contact(draft).await
}

#[tauri::command]
pub async fn update_contact(
    state: State<'_, AppState>,
    contact_id: String,
    draft: ContactDraft,
) -> Result<Contact> {
    state.service.update_contact(&contact_id, draft).await
}

#[tauri::command]
pub async fn delete_contact(state: State<'_, AppState>, contact_id: String) -> Result<()> {
    state.service.delete_contact(&contact_id).await
}

// --- tasks ----------------------------------------------------------------------

#[tauri::command]
pub async fn list_task_lists(state: State<'_, AppState>) -> Result<Vec<TaskList>> {
    let service = state.service.clone();
    blocking(move || service.list_task_lists()).await
}

#[tauri::command]
pub async fn list_tasks(
    state: State<'_, AppState>,
    list_id: Option<String>,
    include_completed: bool,
) -> Result<Vec<Task>> {
    let service = state.service.clone();
    blocking(move || service.list_tasks(list_id.as_deref(), include_completed)).await
}

#[tauri::command]
pub async fn create_task(state: State<'_, AppState>, draft: TaskDraft) -> Result<Task> {
    state.service.create_task(draft).await
}

#[tauri::command]
pub async fn update_task(
    state: State<'_, AppState>,
    task_id: String,
    patch: TaskPatch,
) -> Result<Task> {
    state.service.update_task(&task_id, patch).await
}

#[tauri::command]
pub async fn delete_task(state: State<'_, AppState>, task_id: String) -> Result<()> {
    state.service.delete_task(&task_id).await
}

#[cfg(test)]
mod tests {
    use super::{AppInfo, safe_file_name};

    #[test]
    fn attachment_names_cannot_escape() {
        assert_eq!(safe_file_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(safe_file_name("..."), "attachment");
        assert_eq!(safe_file_name("report.pdf"), "report.pdf");
    }

    /// The shape `src/api/types.ts` mirrors.
    #[test]
    fn app_info_names_the_signature_server_or_null() {
        let info = |signature_server: Option<&str>| {
            serde_json::to_value(AppInfo {
                version: "0.1.0".into(),
                persistent_secrets: true,
                microsoft_client_id_built_in: false,
                sync_server: "sync.cumulas.co".into(),
                signature_server: signature_server.map(str::to_owned),
                restarted_to_update: true,
            })
            .unwrap()
        };
        assert_eq!(
            info(Some("signature.cumulas.co"))["signatureServer"],
            "signature.cumulas.co"
        );
        assert_eq!(info(None)["signatureServer"], serde_json::Value::Null);
        assert_eq!(info(None)["restartedToUpdate"], true);
    }
}
