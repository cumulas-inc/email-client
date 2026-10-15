//! Where notification clicks (and the tray's items) lead: opening a message
//! shows the window and tells the page to go there (the `navigate` event);
//! marking read and archiving happen without the window.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use super::notify::{Action, Invoked, mail_toast_id};
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
        Action::MarkRead(id) => {
            if let Err(e) = service.set_read(std::slice::from_ref(&id), true).await {
                tracing::warn!("could not mark the message read from its notification: {e}");
            }
            desktop.notifications.withdraw(&mail_toast_id(&id)).await;
        }
        Action::Archive(id) => {
            if let Err(e) = service.archive_messages(std::slice::from_ref(&id)).await {
                tracing::warn!("could not archive the message from its notification: {e}");
            }
            desktop.notifications.withdraw(&mail_toast_id(&id)).await;
        }
        Action::ShowWindow => {
            window::show_main(app, click.activation_token);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
