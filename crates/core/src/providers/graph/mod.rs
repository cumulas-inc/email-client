//! Microsoft Graph provider (mail, calendar, contacts, Microsoft To Do).
//!
//! STUB: the public API below is the contract used by `service.rs`; the
//! implementation is filled in separately.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use super::{
    CalendarProvider, ContactsProvider, MailProvider, MessageBatch, MessageContent, MessageLocator,
    SyncWindow, TasksProvider,
};
use crate::auth::TokenSource;
use crate::error::{Error, Result};
use crate::model::{
    Calendar, Contact, ContactDraft, Event, EventDraft, EventResponse, Folder, Message,
    OutgoingMessage, Task, TaskDraft, TaskList, TaskPatch,
};

pub const GRAPH_BASE_URL: &str = "https://graph.microsoft.com/v1.0";

pub struct GraphProvider {
    _account_id: String,
    _base_url: String,
    _tokens: Arc<dyn TokenSource>,
}

impl GraphProvider {
    pub fn new(account_id: impl Into<String>, tokens: Arc<dyn TokenSource>) -> Self {
        Self::with_base_url(account_id, tokens, GRAPH_BASE_URL)
    }

    /// For tests against a mock server.
    pub fn with_base_url(
        account_id: impl Into<String>,
        tokens: Arc<dyn TokenSource>,
        base_url: impl Into<String>,
    ) -> Self {
        Self {
            _account_id: account_id.into(),
            _base_url: base_url.into(),
            _tokens: tokens,
        }
    }
}

fn todo() -> Error {
    Error::unsupported("Microsoft Graph provider is not implemented yet")
}

#[async_trait]
impl MailProvider for GraphProvider {
    async fn list_folders(&self) -> Result<Vec<Folder>> {
        Err(todo())
    }
    async fn sync_messages(
        &self,
        _: &Folder,
        _: Option<&str>,
        _: SyncWindow,
    ) -> Result<MessageBatch> {
        Err(todo())
    }
    async fn fetch_message(&self, _: &MessageLocator) -> Result<MessageContent> {
        Err(todo())
    }
    async fn fetch_attachment(&self, _: &MessageLocator, _: &str) -> Result<Vec<u8>> {
        Err(todo())
    }
    async fn set_read(&self, _: &[MessageLocator], _: bool) -> Result<()> {
        Err(todo())
    }
    async fn set_flagged(&self, _: &[MessageLocator], _: bool) -> Result<()> {
        Err(todo())
    }
    async fn move_messages(&self, _: &[MessageLocator], _: &str) -> Result<Vec<Option<String>>> {
        Err(todo())
    }
    async fn delete_messages(&self, _: &[MessageLocator]) -> Result<()> {
        Err(todo())
    }
    async fn send(&self, _: &OutgoingMessage, _: Option<&Message>) -> Result<()> {
        Err(todo())
    }
}

#[async_trait]
impl CalendarProvider for GraphProvider {
    async fn list_calendars(&self) -> Result<Vec<Calendar>> {
        Err(todo())
    }
    async fn list_events(
        &self,
        _: &Calendar,
        _: DateTime<Utc>,
        _: DateTime<Utc>,
    ) -> Result<Vec<Event>> {
        Err(todo())
    }
    async fn create_event(&self, _: &Calendar, _: &EventDraft) -> Result<Event> {
        Err(todo())
    }
    async fn update_event(&self, _: &Calendar, _: &str, _: &EventDraft) -> Result<Event> {
        Err(todo())
    }
    async fn delete_event(&self, _: &Calendar, _: &str) -> Result<()> {
        Err(todo())
    }
    async fn respond_to_event(
        &self,
        _: &Calendar,
        _: &str,
        _: EventResponse,
        _: Option<&str>,
        _: bool,
    ) -> Result<()> {
        Err(todo())
    }
}

#[async_trait]
impl ContactsProvider for GraphProvider {
    async fn list_contacts(&self) -> Result<Vec<Contact>> {
        Err(todo())
    }
    async fn create_contact(&self, _: &ContactDraft) -> Result<Contact> {
        Err(todo())
    }
    async fn update_contact(&self, _: &str, _: &ContactDraft) -> Result<Contact> {
        Err(todo())
    }
    async fn delete_contact(&self, _: &str) -> Result<()> {
        Err(todo())
    }
}

#[async_trait]
impl TasksProvider for GraphProvider {
    async fn list_task_lists(&self) -> Result<Vec<TaskList>> {
        Err(todo())
    }
    async fn list_tasks(&self, _: &TaskList) -> Result<Vec<Task>> {
        Err(todo())
    }
    async fn create_task(&self, _: &TaskList, _: &TaskDraft) -> Result<Task> {
        Err(todo())
    }
    async fn update_task(&self, _: &TaskList, _: &Task, _: &TaskPatch) -> Result<Task> {
        Err(todo())
    }
    async fn delete_task(&self, _: &TaskList, _: &str) -> Result<()> {
        Err(todo())
    }
}
