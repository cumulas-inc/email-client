//! Commands invoked from the UI (`src/api/tauri.ts`). They are thin wrappers
//! around [`cumulas_core::CoreService`]; argument names arrive in camelCase.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use cumulas_core::model::{
    Account, Calendar, Contact, ContactDraft, EmailAddress, Event, EventDraft, EventResponse,
    Folder, ImapAccountUpdate, Message, MessagePage, MessageQuery, MessageSummary, NewImapAccount,
    OutgoingMessage, Settings, SyncStatus, Task, TaskDraft, TaskList, TaskPatch,
};
use cumulas_core::service::BUILTIN_MICROSOFT_CLIENT_ID;
use cumulas_core::{Error, Result};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: String,
    /// Passwords and tokens survive restarts (a Secret Service is available).
    persistent_secrets: bool,
    /// A Microsoft client id was compiled in, so Microsoft sign-in works
    /// without configuration.
    microsoft_client_id_built_in: bool,
}

#[tauri::command]
pub fn get_app_info(app: AppHandle, state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        persistent_secrets: state.persistent_secrets,
        microsoft_client_id_built_in: BUILTIN_MICROSOFT_CLIENT_ID.is_some(),
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

// --- accounts & settings ------------------------------------------------------

#[tauri::command]
pub fn list_accounts(state: State<'_, AppState>) -> Result<Vec<Account>> {
    state.service.list_accounts()
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

#[tauri::command]
pub async fn add_microsoft_account(
    app: AppHandle,
    state: State<'_, AppState>,
    login_hint: Option<String>,
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
        .add_microsoft_account(&open_browser, login_hint.as_deref())
        .await?;
    spawn_sync(&state, &account.id);
    Ok(account)
}

/// Stops waiting for a Microsoft sign-in the user abandoned in the browser.
#[tauri::command]
pub fn cancel_microsoft_sign_in(state: State<'_, AppState>) {
    state.service.cancel_microsoft_sign_in();
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
pub fn get_settings(state: State<'_, AppState>) -> Result<Settings> {
    state.service.get_settings()
}

#[tauri::command]
pub async fn update_settings(state: State<'_, AppState>, settings: Settings) -> Result<Settings> {
    state.service.update_settings(settings).await
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
pub fn get_sync_statuses(state: State<'_, AppState>) -> Result<Vec<SyncStatus>> {
    state.service.sync_statuses()
}

// --- mail -----------------------------------------------------------------------

#[tauri::command]
pub fn list_folders(state: State<'_, AppState>, account_id: Option<String>) -> Result<Vec<Folder>> {
    state.service.list_folders(account_id.as_deref())
}

#[tauri::command]
pub fn list_messages(
    state: State<'_, AppState>,
    folder_id: String,
    query: MessageQuery,
) -> Result<MessagePage> {
    state.service.list_messages(&folder_id, &query)
}

#[tauri::command]
pub fn search_messages(
    state: State<'_, AppState>,
    text: String,
    account_id: Option<String>,
) -> Result<Vec<MessageSummary>> {
    state.service.search_messages(&text, account_id.as_deref())
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
    state.service.send_message(message).await
}

#[tauri::command]
pub fn suggest_addresses(state: State<'_, AppState>, text: String) -> Result<Vec<EmailAddress>> {
    state.service.suggest_addresses(&text)
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
/// writes it there. Returns the saved path, or `None` if cancelled.
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
pub fn list_calendars(state: State<'_, AppState>) -> Result<Vec<Calendar>> {
    state.service.list_calendars()
}

#[tauri::command]
pub fn list_events(
    state: State<'_, AppState>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    calendar_ids: Option<Vec<String>>,
) -> Result<Vec<Event>> {
    state
        .service
        .list_events(start, end, calendar_ids.as_deref())
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
pub fn list_contacts(
    state: State<'_, AppState>,
    account_id: Option<String>,
    text: Option<String>,
) -> Result<Vec<Contact>> {
    state
        .service
        .list_contacts(account_id.as_deref(), text.as_deref())
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
pub fn list_task_lists(state: State<'_, AppState>) -> Result<Vec<TaskList>> {
    state.service.list_task_lists()
}

#[tauri::command]
pub fn list_tasks(
    state: State<'_, AppState>,
    list_id: Option<String>,
    include_completed: bool,
) -> Result<Vec<Task>> {
    state
        .service
        .list_tasks(list_id.as_deref(), include_completed)
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
    use super::safe_file_name;

    #[test]
    fn attachment_names_cannot_escape() {
        assert_eq!(safe_file_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(safe_file_name("..."), "attachment");
        assert_eq!(safe_file_name("report.pdf"), "report.pdf");
    }
}
