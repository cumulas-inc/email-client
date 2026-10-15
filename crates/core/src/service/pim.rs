//! Calendar, contacts and tasks. Microsoft accounts go through Graph; IMAP
//! accounts keep these in the local store only.

use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

use super::CoreService;
use crate::error::{Error, Result};
use crate::model::{
    Attendee, AttendeeKind, Calendar, Contact, ContactDraft, DataKind, EntityKind, Event,
    EventDraft, EventResponse, Importance, ResponseStatus, Task, TaskDraft, TaskList, TaskPatch,
    TaskStatus, local_id,
};

fn validate_event(draft: &EventDraft) -> Result<()> {
    if draft.subject.trim().is_empty() {
        return Err(Error::invalid_input("Add a title for the event."));
    }
    if draft.end < draft.start {
        return Err(Error::invalid_input("The event ends before it starts."));
    }
    Ok(())
}

fn local_event(calendar: &Calendar, remote_id: &str, draft: &EventDraft) -> Event {
    Event {
        id: local_id(&calendar.account_id, EntityKind::Event, remote_id),
        account_id: calendar.account_id.clone(),
        calendar_id: calendar.id.clone(),
        remote_id: remote_id.to_owned(),
        subject: draft.subject.trim().to_owned(),
        location: draft.location.clone().filter(|l| !l.trim().is_empty()),
        body_preview: draft
            .body_html
            .as_deref()
            .map(|h| crate::html::make_preview(&crate::html::html_to_text(h), 200)),
        body_html: draft.body_html.clone(),
        start: draft.start,
        end: draft.end,
        is_all_day: draft.is_all_day,
        organizer: None,
        attendees: draft
            .attendees
            .iter()
            .map(|e| Attendee {
                email: e.clone(),
                kind: AttendeeKind::Required,
                response: ResponseStatus::None,
            })
            .collect(),
        show_as: draft.show_as,
        response_status: ResponseStatus::Organizer,
        is_online_meeting: false,
        online_meeting_url: None,
        reminder_minutes: draft.reminder_minutes,
        is_recurring: false,
        is_cancelled: false,
    }
}

fn contact_from_draft(remote_id: &str, draft: &ContactDraft) -> Contact {
    Contact {
        id: local_id(&draft.account_id, EntityKind::Contact, remote_id),
        account_id: draft.account_id.clone(),
        remote_id: remote_id.to_owned(),
        display_name: draft.display_name.trim().to_owned(),
        given_name: draft.given_name.clone(),
        surname: draft.surname.clone(),
        emails: draft.emails.clone(),
        phones: draft.phones.clone(),
        company: draft.company.clone(),
        job_title: draft.job_title.clone(),
        department: draft.department.clone(),
        address: draft.address.clone(),
        birthday: draft.birthday,
        notes: draft.notes.clone(),
    }
}

/// Applies a patch to a task in memory (used for local tasks and to validate).
pub(crate) fn apply_task_patch(task: &Task, patch: &TaskPatch) -> Task {
    let mut t = task.clone();
    if let Some(title) = &patch.title {
        t.title = title.trim().to_owned();
    }
    if patch.clear_notes {
        t.notes = None;
    } else if let Some(notes) = &patch.notes {
        t.notes = Some(notes.clone());
    }
    if let Some(importance) = patch.importance {
        t.importance = importance;
    }
    if patch.clear_due_date {
        t.due_date = None;
    } else if let Some(due) = patch.due_date {
        t.due_date = Some(due);
    }
    if patch.clear_reminder {
        t.reminder_at = None;
    } else if let Some(at) = patch.reminder_at {
        t.reminder_at = Some(at);
    }
    if let Some(status) = patch.status {
        if status == TaskStatus::Completed && t.status != TaskStatus::Completed {
            t.completed_at = Some(Utc::now());
        } else if status != TaskStatus::Completed {
            t.completed_at = None;
        }
        t.status = status;
    }
    t.updated_at = Utc::now();
    t
}

impl CoreService {
    // --- calendar -------------------------------------------------------------

    /// The server keeps the account's reminders, and an event was just
    /// made or changed there: its reminders (and calendars) are read again
    /// soon, so that the server's word covers the event. Until then the
    /// event's own reminder time counts, and a reminder already due waits
    /// for that word a minute at most.
    fn request_server_reminders(&self, account_id: &str) {
        self.push.request_for(
            account_id,
            super::SyncScope {
                calendar: true,
                ..Default::default()
            },
        );
    }

    pub fn list_calendars(&self) -> Result<Vec<Calendar>> {
        self.store.list_calendars()
    }

    pub fn list_events(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        calendar_ids: Option<&[String]>,
    ) -> Result<Vec<Event>> {
        if end <= start || end - start > Duration::days(400) {
            return Err(Error::invalid_input("Invalid date range."));
        }
        self.store.list_events(start, end, calendar_ids)
    }

    /// Fetches events in a date range from the servers into the cache, e.g.
    /// when the user navigates outside the regularly synced window.
    pub async fn refresh_events(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<()> {
        if end <= start || end - start > Duration::days(400) {
            return Err(Error::invalid_input("Invalid date range."));
        }
        for calendar in self.store.list_calendars()? {
            let Some(provider) = self.providers_for(&calendar.account_id).await?.calendar else {
                continue;
            };
            let events = provider.list_events(&calendar, start, end).await?;
            self.store
                .replace_events_in_range(&calendar.id, start, end, &events)?;
        }
        self.notify(&[DataKind::Calendar], None);
        Ok(())
    }

    pub async fn create_event(&self, draft: EventDraft) -> Result<Event> {
        validate_event(&draft)?;
        let calendar = self.store.get_calendar(&draft.calendar_id)?;
        if !calendar.can_edit {
            return Err(Error::invalid_input("This calendar is read-only."));
        }
        let event = match self.providers_for(&calendar.account_id).await?.calendar {
            Some(provider) => {
                let event = provider.create_event(&calendar, &draft).await?;
                self.request_server_reminders(&calendar.account_id);
                event
            }
            None => local_event(&calendar, &Uuid::new_v4().to_string(), &draft),
        };
        self.store.upsert_event(&event)?;
        self.notify(&[DataKind::Calendar], Some(&calendar.account_id));
        Ok(event)
    }

    pub async fn update_event(&self, event_id: &str, draft: EventDraft) -> Result<Event> {
        validate_event(&draft)?;
        let existing = self.store.get_event(event_id)?;
        if existing.calendar_id != draft.calendar_id {
            return Err(Error::unsupported(
                "Moving an event to another calendar is not supported yet.",
            ));
        }
        let calendar = self.store.get_calendar(&existing.calendar_id)?;
        let event = match self.providers_for(&calendar.account_id).await?.calendar {
            Some(provider) => {
                let event = provider
                    .update_event(&calendar, &existing.remote_id, &draft)
                    .await?;
                self.request_server_reminders(&calendar.account_id);
                event
            }
            None => local_event(&calendar, &existing.remote_id, &draft),
        };
        self.store.upsert_event(&event)?;
        self.notify(&[DataKind::Calendar], Some(&calendar.account_id));
        Ok(event)
    }

    pub async fn delete_event(&self, event_id: &str) -> Result<()> {
        let event = self.store.get_event(event_id)?;
        let calendar = self.store.get_calendar(&event.calendar_id)?;
        if let Some(provider) = self.providers_for(&calendar.account_id).await?.calendar {
            provider.delete_event(&calendar, &event.remote_id).await?;
        }
        self.store.delete_event(event_id)?;
        self.notify(&[DataKind::Calendar], Some(&calendar.account_id));
        Ok(())
    }

    pub async fn respond_to_event(
        &self,
        event_id: &str,
        response: EventResponse,
        comment: Option<String>,
        send_response: bool,
    ) -> Result<Event> {
        let mut event = self.store.get_event(event_id)?;
        let calendar = self.store.get_calendar(&event.calendar_id)?;
        let provider = self
            .providers_for(&calendar.account_id)
            .await?
            .calendar
            .ok_or_else(|| Error::unsupported("Meeting responses need a Microsoft account."))?;
        provider
            .respond_to_event(
                &calendar,
                &event.remote_id,
                response,
                comment.as_deref(),
                send_response,
            )
            .await?;
        if response == EventResponse::Decline {
            self.store.delete_event(event_id)?;
        } else {
            event.response_status = match response {
                EventResponse::Accept => ResponseStatus::Accepted,
                _ => ResponseStatus::Tentative,
            };
            self.store.upsert_event(&event)?;
        }
        self.notify(&[DataKind::Calendar], Some(&calendar.account_id));
        Ok(event)
    }

    // --- contacts -------------------------------------------------------------

    pub fn list_contacts(
        &self,
        account_id: Option<&str>,
        text: Option<&str>,
    ) -> Result<Vec<Contact>> {
        self.store.list_contacts(account_id, text)
    }

    fn validate_contact(draft: &ContactDraft) -> Result<()> {
        if draft.display_name.trim().is_empty() {
            return Err(Error::invalid_input("Add a name for the contact."));
        }
        Ok(())
    }

    pub async fn create_contact(&self, draft: ContactDraft) -> Result<Contact> {
        Self::validate_contact(&draft)?;
        self.store.get_account(&draft.account_id)?;
        let contact = match self.providers_for(&draft.account_id).await?.contacts {
            Some(provider) => provider.create_contact(&draft).await?,
            None => contact_from_draft(&Uuid::new_v4().to_string(), &draft),
        };
        self.store.upsert_contact(&contact)?;
        self.notify(&[DataKind::Contacts], Some(&draft.account_id));
        Ok(contact)
    }

    pub async fn update_contact(&self, contact_id: &str, draft: ContactDraft) -> Result<Contact> {
        Self::validate_contact(&draft)?;
        let existing = self.store.get_contact(contact_id)?;
        if existing.account_id != draft.account_id {
            return Err(Error::unsupported(
                "Contacts cannot be moved between accounts.",
            ));
        }
        let contact = match self.providers_for(&existing.account_id).await?.contacts {
            Some(provider) => provider.update_contact(&existing.remote_id, &draft).await?,
            None => contact_from_draft(&existing.remote_id, &draft),
        };
        self.store.upsert_contact(&contact)?;
        self.notify(&[DataKind::Contacts], Some(&existing.account_id));
        Ok(contact)
    }

    pub async fn delete_contact(&self, contact_id: &str) -> Result<()> {
        let existing = self.store.get_contact(contact_id)?;
        if let Some(provider) = self.providers_for(&existing.account_id).await?.contacts {
            provider.delete_contact(&existing.remote_id).await?;
        }
        self.store.delete_contact(contact_id)?;
        self.notify(&[DataKind::Contacts], Some(&existing.account_id));
        Ok(())
    }

    // --- tasks ----------------------------------------------------------------

    pub fn list_task_lists(&self) -> Result<Vec<TaskList>> {
        self.store.list_task_lists()
    }

    pub fn list_tasks(&self, list_id: Option<&str>, include_completed: bool) -> Result<Vec<Task>> {
        self.store.list_tasks(list_id, include_completed)
    }

    pub async fn create_task(&self, draft: TaskDraft) -> Result<Task> {
        if draft.title.trim().is_empty() {
            return Err(Error::invalid_input("Add a title for the task."));
        }
        let list = self.store.get_task_list(&draft.list_id)?;
        let task = match self.providers_for(&list.account_id).await?.tasks {
            Some(provider) => provider.create_task(&list, &draft).await?,
            None => {
                let remote_id = Uuid::new_v4().to_string();
                let now = Utc::now();
                Task {
                    id: local_id(&list.account_id, EntityKind::Task, &remote_id),
                    account_id: list.account_id.clone(),
                    list_id: list.id.clone(),
                    remote_id,
                    title: draft.title.trim().to_owned(),
                    notes: draft.notes.clone().filter(|n| !n.trim().is_empty()),
                    status: TaskStatus::NotStarted,
                    importance: draft.importance.unwrap_or(Importance::Normal),
                    due_date: draft.due_date,
                    reminder_at: draft.reminder_at,
                    completed_at: None,
                    created_at: now,
                    updated_at: now,
                }
            }
        };
        self.store.upsert_task(&task)?;
        self.notify(&[DataKind::Tasks], Some(&list.account_id));
        Ok(task)
    }

    pub async fn update_task(&self, task_id: &str, patch: TaskPatch) -> Result<Task> {
        if patch.title.as_deref().is_some_and(|t| t.trim().is_empty()) {
            return Err(Error::invalid_input("A task needs a title."));
        }
        let current = self.store.get_task(task_id)?;
        let list = self.store.get_task_list(&current.list_id)?;
        let task = match self.providers_for(&list.account_id).await?.tasks {
            Some(provider) => provider.update_task(&list, &current, &patch).await?,
            None => apply_task_patch(&current, &patch),
        };
        self.store.upsert_task(&task)?;
        self.notify(&[DataKind::Tasks], Some(&list.account_id));
        Ok(task)
    }

    pub async fn delete_task(&self, task_id: &str) -> Result<()> {
        let task = self.store.get_task(task_id)?;
        let list = self.store.get_task_list(&task.list_id)?;
        if let Some(provider) = self.providers_for(&list.account_id).await?.tasks {
            provider.delete_task(&list, &task.remote_id).await?;
        }
        self.store.delete_task(task_id)?;
        self.notify(&[DataKind::Tasks], Some(&list.account_id));
        Ok(())
    }
}
