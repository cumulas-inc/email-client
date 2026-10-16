//! Microsoft To Do task lists and tasks on Microsoft Graph.

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::GraphProvider;
use super::client::segment;
use super::types::{
    DateTimeTimeZone, ItemBody, date_time_json, midnight, nearest_date, non_empty, null_default,
};
use crate::error::Result;
use crate::html::html_to_text;
use crate::model::{
    EntityKind, Importance, Task, TaskDraft, TaskList, TaskPatch, TaskStatus, local_id,
};
use crate::providers::TasksProvider;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphTaskList {
    id: String,
    display_name: Option<String>,
    /// `none`, `defaultList` or `flaggedEmails`.
    wellknown_list_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphTask {
    id: String,
    title: Option<String>,
    body: Option<ItemBody>,
    status: Option<String>,
    importance: Option<String>,
    due_date_time: Option<DateTimeTimeZone>,
    #[serde(default, deserialize_with = "null_default")]
    is_reminder_on: bool,
    reminder_date_time: Option<DateTimeTimeZone>,
    completed_date_time: Option<DateTimeTimeZone>,
    created_date_time: Option<DateTime<Utc>>,
    last_modified_date_time: Option<DateTime<Utc>>,
}

/// To Do has two states the model lacks: `waitingOnOthers` is work under way
/// (in progress); `deferred` has been put off (not started).
fn task_status(status: Option<&str>) -> TaskStatus {
    match status {
        Some("completed") => TaskStatus::Completed,
        Some("inProgress" | "waitingOnOthers") => TaskStatus::InProgress,
        _ => TaskStatus::NotStarted,
    }
}

impl GraphTask {
    fn into_task(self, account_id: &str, list_id: &str) -> Task {
        let notes = self.body.and_then(|body| {
            let is_html = body.is_html();
            body.content
                .map(|c| if is_html { html_to_text(&c) } else { c })
                .and_then(|c| non_empty(Some(c)))
        });
        let created_at = self.created_date_time.unwrap_or_else(Utc::now);
        Task {
            id: local_id(account_id, EntityKind::Task, &self.id),
            account_id: account_id.to_owned(),
            list_id: list_id.to_owned(),
            remote_id: self.id,
            title: self.title.unwrap_or_default(),
            notes,
            status: task_status(self.status.as_deref()),
            importance: Importance::parse(self.importance.as_deref().unwrap_or("normal")),
            // Due dates are midnight in the zone they were set in.
            due_date: self.due_date_time.and_then(|d| d.naive()).map(nearest_date),
            reminder_at: self
                .is_reminder_on
                .then(|| self.reminder_date_time.as_ref()?.instant())
                .flatten(),
            completed_at: self.completed_date_time.and_then(|d| d.utc()),
            created_at,
            updated_at: self.last_modified_date_time.unwrap_or(created_at),
        }
    }
}

fn body_json(notes: &str) -> Value {
    json!({ "content": notes, "contentType": "text" })
}

/// A reminder goes out in UTC.
fn reminder_json(at: DateTime<Utc>) -> Value {
    date_time_json(at.naive_utc(), "UTC")
}

impl GraphProvider {
    fn tasks_url(&self, list: &TaskList) -> String {
        self.client.url(&format!(
            "/me/todo/lists/{}/tasks",
            segment(&list.remote_id)
        ))
    }

    fn task_url(&self, list: &TaskList, remote_id: &str) -> String {
        format!("{}/{}", self.tasks_url(list), segment(remote_id))
    }

    /// Due dates go out as midnight in the mailbox time zone, which is how
    /// To Do itself stores them.
    async fn due_json(&self, date: NaiveDate) -> Value {
        date_time_json(midnight(date), &self.mailbox_time_zone().await)
    }
}

#[async_trait]
impl TasksProvider for GraphProvider {
    async fn list_task_lists(&self) -> Result<Vec<TaskList>> {
        let lists: Vec<GraphTaskList> = self
            .client
            .get_all(&self.client.url("/me/todo/lists"), &[])
            .await?;
        Ok(lists
            .into_iter()
            // "Flagged email" mirrors flagged messages, which the mail view
            // already shows, and does not accept new tasks.
            .filter(|l| l.wellknown_list_name.as_deref() != Some("flaggedEmails"))
            .map(|l| TaskList {
                id: local_id(&self.account_id, EntityKind::TaskList, &l.id),
                account_id: self.account_id.clone(),
                name: l.display_name.unwrap_or_default(),
                is_default: l.wellknown_list_name.as_deref() == Some("defaultList"),
                remote_id: l.id,
            })
            .collect())
    }

    async fn list_tasks(&self, list: &TaskList) -> Result<Vec<Task>> {
        let tasks: Vec<GraphTask> = self.client.get_all(&self.tasks_url(list), &[]).await?;
        Ok(tasks
            .into_iter()
            .map(|t| t.into_task(&self.account_id, &list.id))
            .collect())
    }

    async fn create_task(&self, list: &TaskList, draft: &TaskDraft) -> Result<Task> {
        let mut body = json!({ "title": draft.title });
        if let Some(notes) = draft.notes.as_deref().filter(|n| !n.trim().is_empty()) {
            body["body"] = body_json(notes);
        }
        if let Some(importance) = draft.importance {
            body["importance"] = importance.as_str().into();
        }
        if let Some(due) = draft.due_date {
            body["dueDateTime"] = self.due_json(due).await;
        }
        if let Some(at) = draft.reminder_at {
            body["isReminderOn"] = true.into();
            body["reminderDateTime"] = reminder_json(at);
        }
        let created: GraphTask = self
            .client
            .send_json(Method::POST, &self.tasks_url(list), &body, &[])
            .await?;
        Ok(created.into_task(&self.account_id, &list.id))
    }

    async fn update_task(
        &self,
        list: &TaskList,
        current: &Task,
        patch: &TaskPatch,
    ) -> Result<Task> {
        let mut body = Map::new();
        if let Some(title) = &patch.title {
            body.insert("title".into(), title.as_str().into());
        }
        if patch.clear_notes {
            body.insert("body".into(), body_json(""));
        } else if let Some(notes) = &patch.notes {
            body.insert("body".into(), body_json(notes));
        }
        if let Some(status) = patch.status {
            body.insert("status".into(), status.as_str().into());
        }
        if let Some(importance) = patch.importance {
            body.insert("importance".into(), importance.as_str().into());
        }
        if patch.clear_due_date {
            body.insert("dueDateTime".into(), Value::Null);
        } else if let Some(due) = patch.due_date {
            body.insert("dueDateTime".into(), self.due_json(due).await);
        }
        if patch.clear_reminder {
            body.insert("isReminderOn".into(), false.into());
        } else if let Some(at) = patch.reminder_at {
            body.insert("isReminderOn".into(), true.into());
            body.insert("reminderDateTime".into(), reminder_json(at));
        }
        if body.is_empty() {
            return Ok(current.clone());
        }
        let updated: GraphTask = self
            .client
            .send_json(
                Method::PATCH,
                &self.task_url(list, &current.remote_id),
                &Value::Object(body),
                &[],
            )
            .await?;
        Ok(updated.into_task(&self.account_id, &list.id))
    }

    async fn delete_task(&self, list: &TaskList, remote_id: &str) -> Result<()> {
        self.client
            .send_no_content(Method::DELETE, &self.task_url(list, remote_id), None, &[])
            .await?;
        Ok(())
    }
}
