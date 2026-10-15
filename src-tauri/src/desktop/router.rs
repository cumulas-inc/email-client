//! Where notification clicks (and the tray's items) lead: opening a message
//! shows the window and tells the page to go there (the `navigate` event);
//! marking read and archiving happen without the window (and a new
//! notification says when they failed).

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use cumulas_core::store::Store;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use super::notify::{Action, Invoked, Notifications, failed_toast, mail_toast_id};
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
