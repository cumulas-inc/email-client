//! Where notification clicks (and the tray's items) lead: opening a message,
//! an event or a task shows the window and tells the page to go there (the
//! `navigate` event); marking read, archiving, snoozing, dismissing and
//! completing a task happen without the window (and a new notification
//! says when they failed); Join opens the meeting in the browser.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Utc};
use cumulas_core::model::{Snooze, TaskPatch, TaskStatus};
use cumulas_core::store::Store;
use cumulas_core::{CoreService, ErrorCode};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;

use super::notify::{
    Action, Invoked, Notifications, failed_reminder_toast, failed_toast, is_https, mail_toast_id,
    reminder_toast_id,
};
use super::{Desktop, window};
use crate::AppState;

/// Where the page should go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Navigate {
    /// A message, in its folder.
    #[serde(rename_all = "camelCase")]
    Message {
        message_id: String,
        folder_id: String,
        account_id: String,
    },
    /// An account's Inbox, or every Inbox when `account_id` is `None`;
    /// `notice` says why, when the page should tell the user.
    #[serde(rename_all = "camelCase")]
    Inbox {
        account_id: Option<String>,
        notice: Option<String>,
    },
    /// A new message.
    Compose,
    /// An event, in Calendar: the day it starts on (the UTC date of
    /// `start` for an all-day event, the local one otherwise).
    #[serde(rename_all = "camelCase")]
    Event {
        event_id: String,
        start: DateTime<Utc>,
        is_all_day: bool,
    },
    /// A task, in its list.
    #[serde(rename_all = "camelCase")]
    Task { task_id: String, list_id: String },
    /// The Reminders panel; `notice` says why, when there's something to say.
    #[serde(rename_all = "camelCase")]
    Reminders { notice: Option<String> },
}

/// Navigation waiting for the page: the window was created for it, and its
/// page hadn't started listening yet.
#[derive(Default)]
pub struct Router {
    /// The page has started (it asked for pending navigation), so events
    /// reach it.
    page_ready: AtomicBool,
    pending: Mutex<Option<Navigate>>,
}

impl Router {
    /// The page started: hands over what's waiting for it.
    pub fn take_pending(&self) -> Option<Navigate> {
        self.page_ready.store(true, Ordering::SeqCst);
        self.pending.lock().unwrap().take()
    }

    /// The window and its page are gone.
    pub fn page_gone(&self) {
        self.page_ready.store(false, Ordering::SeqCst);
    }

    /// Sends the page to `to`, now or once it has started.
    pub fn navigate(&self, app: &AppHandle, to: Navigate) {
        if self.page_ready.load(Ordering::SeqCst) {
            if let Err(e) = app.emit("navigate", &to) {
                tracing::warn!("could not navigate: {e}");
            }
        } else {
            *self.pending.lock().unwrap() = Some(to);
        }
    }
}

/// Carries out a click on a notification.
pub async fn handle(app: &AppHandle, click: Invoked) {
    let Some(desktop) = app.try_state::<Desktop>() else {
        return;
    };
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let service = &state.service;
    match click.action {
        Action::OpenMessage(id) => {
            window::show_main(app, click.activation_token);
            let to = match service.store().get_message_summary(&id) {
                Ok(m) => Navigate::Message {
                    message_id: m.id,
                    folder_id: m.folder_id,
                    account_id: m.account_id,
                },
                Err(_) => Navigate::Inbox {
                    account_id: None,
                    notice: Some("That message was moved or deleted.".into()),
                },
            };
            desktop.router.navigate(app, to);
        }
        Action::OpenInbox(account_id) => {
            window::show_main(app, click.activation_token);
            desktop.router.navigate(
                app,
                Navigate::Inbox {
                    account_id: Some(account_id),
                    notice: None,
                },
            );
        }
        Action::MarkRead(id) => match service.set_read(std::slice::from_ref(&id), true).await {
            Ok(()) => desktop.notifications.withdraw(&mail_toast_id(&id)).await,
            Err(e) => {
                tracing::warn!("could not mark the message read from its notification: {e}");
                let title = "Couldn't mark the message as read";
                failed(&desktop.notifications, service.store(), &id, title).await;
            }
        },
        Action::Archive(id) => match service.archive_messages(std::slice::from_ref(&id)).await {
            Ok(()) => desktop.notifications.withdraw(&mail_toast_id(&id)).await,
            Err(e) => {
                tracing::warn!("could not archive the message from its notification: {e}");
                let title = "Couldn't archive the message";
                failed(&desktop.notifications, service.store(), &id, title).await;
            }
        },
        Action::ShowWindow => {
            window::show_main(app, click.activation_token);
        }
        Action::OpenEvent(id) => {
            window::show_main(app, click.activation_token);
            let to = match service.store().get_event(&id) {
                Ok(event) => Navigate::Event {
                    event_id: event.id,
                    start: event.start,
                    is_all_day: event.is_all_day,
                },
                Err(_) => Navigate::Reminders {
                    notice: Some("That event was moved or deleted.".into()),
                },
            };
            desktop.router.navigate(app, to);
        }
        Action::OpenTask(id) => {
            window::show_main(app, click.activation_token);
            let to = match service.store().get_task(&id) {
                Ok(task) => Navigate::Task {
                    task_id: task.id,
                    list_id: task.list_id,
                },
                Err(_) => Navigate::Reminders {
                    notice: Some("That task was deleted.".into()),
                },
            };
            desktop.router.navigate(app, to);
        }
        Action::OpenReminders => {
            window::show_main(app, click.activation_token);
            desktop
                .router
                .navigate(app, Navigate::Reminders { notice: None });
        }
        action @ (Action::Snooze { .. } | Action::Dismiss(_)) => {
            reminder_action(&desktop.notifications, service, action).await;
        }
        Action::Join(url) => {
            // Only the meeting's own web address: a Join target from
            // anywhere else is never opened.
            if !is_https(&url) {
                tracing::warn!("not opening a meeting address that isn't https");
                return;
            }
            if let Err(e) = app.opener().open_url(url.trim(), None::<&str>) {
                tracing::warn!("could not open the meeting: {e}");
            }
        }
        action @ Action::CompleteTask(_) => {
            reminder_action(&desktop.notifications, service, action).await;
        }
    }
}

/// Carries out a reminder's Snooze, Dismiss or Mark complete button, without
/// the window.
pub(crate) async fn reminder_action(
    notifications: &Notifications,
    service: &CoreService,
    action: Action,
) {
    match action {
        Action::Snooze { key, minutes } => {
            let result = service
                .snooze_reminder(&key, Snooze::Minutes(minutes))
                .await;
            reminder_done(notifications, service, &key, result.map(drop), "snooze").await;
        }
        Action::Dismiss(key) => {
            let result = service.dismiss_reminders(std::slice::from_ref(&key)).await;
            reminder_done(notifications, service, &key, result.map(drop), "dismiss").await;
        }
        Action::CompleteTask(id) => {
            let patch = TaskPatch {
                status: Some(TaskStatus::Completed),
                ..TaskPatch::default()
            };
            // Its reminder clears once it's done (see `reminders_changed`).
            if let Err(e) = service.update_task(&id, patch).await {
                tracing::warn!("could not complete the task from its notification: {e}");
                let toast = failed_reminder_toast("Couldn't complete the task", None);
                if let Err(e) = notifications.show(&toast, Vec::new()).await {
                    tracing::warn!("could not say a notification's button failed: {e}");
                }
            }
        }
        _ => {}
    }
}

/// A reminder's Snooze or Dismiss (`what`) button was carried out: its
/// notification goes, or says that it failed (the desktop closed it on
/// the click, so the reminder would look handled).
async fn reminder_done(
    notifications: &Notifications,
    service: &CoreService,
    key: &str,
    result: cumulas_core::Result<()>,
    what: &str,
) {
    match result {
        Ok(()) => notifications.withdraw(&reminder_toast_id(key)).await,
        // No longer due: nothing left to do.
        Err(e) if e.code == ErrorCode::NotFound => {
            notifications.withdraw(&reminder_toast_id(key)).await;
        }
        Err(e) => {
            tracing::warn!("could not {what} the reminder from its notification: {e}");
            let reminder = service
                .list_active_reminders()
                .ok()
                .and_then(|active| active.into_iter().find(|r| r.key == key));
            let toast =
                failed_reminder_toast(&format!("Couldn't {what} the reminder"), reminder.as_ref());
            if let Err(e) = notifications.show(&toast, vec![key.to_owned()]).await {
                tracing::warn!("could not say a notification's button failed: {e}");
            }
        }
    }
}

/// A notification's button failed. The desktop closed the notification on
/// the click, so a new one says so; otherwise the message would look
/// handled.
async fn failed(notifications: &Notifications, store: &Store, id: &str, title: &str) {
    let message = store.get_message_summary(id);
    let account = message
        .as_ref()
        .ok()
        .and_then(|m| store.get_account(&m.account_id).ok());
    let (Ok(message), Some(account)) = (message, account) else {
        // Moved or deleted meanwhile: nothing is left to do.
        notifications.withdraw(&mail_toast_id(id)).await;
        return;
    };
    let preview = store.get_settings().unwrap_or_default().notify_show_preview;
    let toast = failed_toast(title, &message, &account, preview);
    if let Err(e) = notifications.show(&toast, vec![message.id]).await {
        tracing::warn!("could not say a notification's button failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cumulas_core::model::{Folder, FolderRole};
    use cumulas_core::providers::MessageBatch;
    use tokio::sync::mpsc;

    use super::*;
    use crate::desktop::notify::tests::{Recording, account, message};

    #[test]
    fn navigation_serializes_for_the_page() {
        let message = Navigate::Message {
            message_id: "m1".into(),
            folder_id: "f1".into(),
            account_id: "a1".into(),
        };
        assert_eq!(
            serde_json::to_value(&message).unwrap(),
            serde_json::json!({
                "kind": "message",
                "messageId": "m1",
                "folderId": "f1",
                "accountId": "a1",
            })
        );
        assert_eq!(
            serde_json::to_value(Navigate::Inbox {
                account_id: None,
                notice: Some("gone".into()),
            })
            .unwrap(),
            serde_json::json!({ "kind": "inbox", "accountId": null, "notice": "gone" })
        );
        assert_eq!(
            serde_json::to_value(Navigate::Compose).unwrap(),
            serde_json::json!({ "kind": "compose" })
        );
        assert_eq!(
            serde_json::to_value(Navigate::Event {
                event_id: "e1".into(),
                start: chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 10, 15, 9, 0, 0).unwrap(),
                is_all_day: false,
            })
            .unwrap(),
            serde_json::json!({
                "kind": "event",
                "eventId": "e1",
                "start": "2026-10-15T09:00:00Z",
                "isAllDay": false,
            })
        );
        assert_eq!(
            serde_json::to_value(Navigate::Task {
                task_id: "t1".into(),
                list_id: "l1".into(),
            })
            .unwrap(),
            serde_json::json!({ "kind": "task", "taskId": "t1", "listId": "l1" })
        );
        assert_eq!(
            serde_json::to_value(Navigate::Reminders { notice: None }).unwrap(),
            serde_json::json!({ "kind": "reminders", "notice": null })
        );
    }

    #[test]
    fn navigation_waits_for_the_page() {
        let router = Router::default();
        *router.pending.lock().unwrap() = Some(Navigate::Compose);
        assert_eq!(router.take_pending(), Some(Navigate::Compose));
        assert_eq!(router.take_pending(), None);
        assert!(router.page_ready.load(Ordering::SeqCst));
        router.page_gone();
        assert!(!router.page_ready.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn a_failed_button_says_so() {
        let store = Store::open_in_memory().unwrap();
        store.insert_account(&account()).unwrap();
        let inbox = Folder {
            id: "inbox".into(),
            account_id: "acct".into(),
            remote_id: "inbox".into(),
            parent_id: None,
            name: "Inbox".into(),
            role: FolderRole::Inbox,
            unread_count: 0,
            total_count: 0,
        };
        store
            .replace_folders("acct", std::slice::from_ref(&inbox))
            .unwrap();
        let batch = MessageBatch {
            upserts: vec![message("m1", Some(("Ada", "ada@example.com")), "Engines")],
            ..MessageBatch::default()
        };
        store.apply_message_batch(&inbox, &batch).unwrap();
        let recording = Arc::new(Recording::default());
        let (clicks, _) = mpsc::unbounded_channel();
        let notifications = Notifications::with_notifier(recording.clone(), clicks);

        failed(&notifications, &store, "m1", "Couldn't archive the message").await;
        {
            let shown = recording.shown.lock().unwrap();
            let [toast] = shown.as_slice() else {
                panic!("{shown:?}");
            };
            assert_eq!(toast.id, "mail:m1");
            assert_eq!(toast.title, "Couldn't archive the message");
            assert_eq!(toast.body, "Engines");
            assert_eq!(toast.default_action, Action::OpenMessage("m1".into()));
        }
        // It goes once the message is read, like the one it took the place of.
        notifications.settle(&["m1".into()]).await;
        assert_eq!(*recording.withdrawn.lock().unwrap(), ["mail:m1"]);

        // Moved or deleted meanwhile: nothing to say.
        failed(
            &notifications,
            &store,
            "gone",
            "Couldn't archive the message",
        )
        .await;
        assert_eq!(recording.shown.lock().unwrap().len(), 1);
    }
}

#[cfg(test)]
mod reminder_tests {
    use std::sync::Arc;

    use chrono::Duration;
    use cumulas_core::model::{Calendar, EventDraft, ShowAs, TaskDraft, TaskList};
    use cumulas_core::secrets::{MemorySecretStore, SecretStore, keys};
    use cumulas_core::service::{LiveConnector, NullSink};
    use tokio::sync::mpsc;

    use super::*;
    use crate::desktop::notify::tests::{Recording, account};
    use crate::desktop::notify::{ReminderToastOptions, reminder_toasts};

    /// A service with an IMAP account's local calendar and tasks (which
    /// need no server): two events whose reminders are due, and a task's.
    async fn service() -> (CoreService, String) {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::default());
        store.insert_account(&account()).unwrap();
        secrets
            .set(&keys::imap_password("acct"), "secret")
            .await
            .unwrap();
        let calendar = Calendar {
            id: "cal".into(),
            account_id: "acct".into(),
            remote_id: "local:calendar".into(),
            name: "Calendar".into(),
            color: "#0b7a75".into(),
            can_edit: true,
            is_default: true,
        };
        store.upsert_calendar(&calendar).unwrap();
        store
            .upsert_task_list(&TaskList {
                id: "list".into(),
                account_id: "acct".into(),
                remote_id: "local:tasks".into(),
                name: "Tasks".into(),
                is_default: true,
            })
            .unwrap();
        let connector = Arc::new(LiveConnector::new(Arc::clone(&store), Arc::clone(&secrets)));
        let service = CoreService::new(store, secrets, connector, Arc::new(NullSink));
        let soon = Utc::now() + Duration::minutes(5);
        for subject in ["Standup", "Review"] {
            service
                .create_event(EventDraft {
                    calendar_id: calendar.id.clone(),
                    subject: subject.into(),
                    location: None,
                    body_html: None,
                    start: soon,
                    end: soon + Duration::minutes(30),
                    is_all_day: false,
                    attendees: vec![],
                    reminder_minutes: Some(15),
                    show_as: ShowAs::Busy,
                    is_online_meeting: false,
                })
                .await
                .unwrap();
        }
        let task = service
            .create_task(TaskDraft {
                list_id: "list".into(),
                title: "Pay rent".into(),
                notes: None,
                importance: None,
                due_date: None,
                reminder_at: Some(Utc::now() - Duration::minutes(1)),
            })
            .await
            .unwrap();
        (service, task.id)
    }

    #[tokio::test]
    async fn snooze_dismiss_and_complete_work_without_the_window() {
        let (service, task_id) = service().await;
        let active = service.list_active_reminders().unwrap();
        assert_eq!(active.len(), 3);
        let key = |title: &str| {
            active
                .iter()
                .find(|r| r.title == title)
                .unwrap()
                .key
                .clone()
        };
        let recording = Arc::new(Recording::default());
        let (clicks, _) = mpsc::unbounded_channel();
        let notifications = Notifications::with_notifier(recording.clone(), clicks);
        let options = ReminderToastOptions {
            snooze_minutes: 5,
            sound: true,
        };
        for (toast, keys) in reminder_toasts(&active, options, chrono::Local::now()) {
            notifications.show(&toast, keys).await.unwrap();
        }

        // What each button carries, decoded as a click delivers it.
        let toasts = recording.shown.lock().unwrap().clone();
        let button = |title: &str, label: &str| {
            let toast = toasts.iter().find(|t| t.title == title).unwrap();
            let (_, action) = toast
                .buttons
                .iter()
                .find(|(l, _)| l.starts_with(label))
                .unwrap();
            Action::decode(&action.encode()).unwrap()
        };

        reminder_action(&notifications, &service, button("Standup", "Snooze")).await;
        reminder_action(&notifications, &service, button("Review", "Dismiss")).await;
        reminder_action(
            &notifications,
            &service,
            button("Pay rent", "Mark complete"),
        )
        .await;
        assert_eq!(
            *recording.withdrawn.lock().unwrap(),
            [
                reminder_toast_id(&key("Standup")),
                reminder_toast_id(&key("Review"))
            ]
        );
        let active = service.list_active_reminders().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].title, "Standup");
        assert!(active[0].snoozed_until.is_some());
        assert_eq!(
            service.store().get_task(&task_id).unwrap().status,
            TaskStatus::Completed
        );

        // A reminder no longer due (its notification went already): no
        // complaint.
        let gone = Action::Snooze {
            key: key("Review"),
            minutes: 5,
        };
        reminder_action(&notifications, &service, gone).await;
        assert_eq!(recording.withdrawn.lock().unwrap().len(), 2);
        assert_eq!(recording.shown.lock().unwrap().len(), 3);

        // A button that failed says so.
        reminder_action(
            &notifications,
            &service,
            Action::CompleteTask("missing".into()),
        )
        .await;
        let shown = recording.shown.lock().unwrap();
        assert_eq!(shown.last().unwrap().title, "Couldn't complete the task");
        assert_eq!(shown.last().unwrap().default_action, Action::OpenReminders);
    }
}
