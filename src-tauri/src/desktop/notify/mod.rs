//! Desktop notifications: what the app shows ([`Toast`]), what clicking one
//! or its buttons asks for ([`Action`]), and the two ways of showing them.
//!
//! Inside a Flatpak, notifications go through the Notification portal
//! ([`portal::PortalNotifier`]); a native install talks to
//! `org.freedesktop.Notifications` itself ([`fdo::FdoNotifier`]). Both give
//! notifications ids the app chooses, so a notification can be replaced or
//! withdrawn once its message is read elsewhere, and both report clicks as
//! [`Invoked`] actions for the router (`desktop::router`).
//!
//! Do Not Disturb is left to the desktop: no notification is urgent (which
//! would get past it), and the app never plays sounds itself.

#[cfg(target_os = "linux")]
pub mod fdo;
#[cfg(target_os = "linux")]
pub mod portal;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use cumulas_core::model::{Account, MessageSummary};
use cumulas_core::service::MailAlert;
use serde::Serialize;
use tokio::sync::mpsc;

/// What a notification, or one of its buttons, asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Show the message in the main window.
    OpenMessage(String),
    /// Show an account's Inbox (a summary of several new messages).
    OpenInbox(String),
    /// Mark the message read, without showing the window.
    MarkRead(String),
    /// Archive the message, without showing the window.
    Archive(String),
    /// Bring up the main window.
    ShowWindow,
}

impl Action {
    /// The action as a string, for a notification's button or target.
    pub fn encode(&self) -> String {
        match self {
            Action::OpenMessage(id) => format!("open:{id}"),
            Action::OpenInbox(account_id) => format!("inbox:{account_id}"),
            Action::MarkRead(id) => format!("read:{id}"),
            Action::Archive(id) => format!("archive:{id}"),
            Action::ShowWindow => "show".into(),
        }
    }

    /// The action [`Action::encode`] made this string from, if any.
    pub fn decode(encoded: &str) -> Option<Self> {
        if encoded == "show" {
            return Some(Action::ShowWindow);
        }
        let (verb, id) = encoded.split_once(':')?;
        if id.is_empty() {
            return None;
        }
        let id = id.to_owned();
        Some(match verb {
            "open" => Action::OpenMessage(id),
            "inbox" => Action::OpenInbox(id),
            "read" => Action::MarkRead(id),
            "archive" => Action::Archive(id),
            _ => return None,
        })
    }
}

/// A click on a notification or one of its buttons, with the activation
/// token the desktop gave for it (which lets the window take focus on
/// Wayland).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invoked {
    pub action: Action,
    pub activation_token: Option<String>,
}

/// What a notification is about, for hints that depend on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Mail,
    /// About the app itself (still running, a test).
    App,
}

/// A notification to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    /// Chosen by the app: `mail:{message}`, `mail-summary:{account}` or
    /// `app:{what}`. Showing a toast with the same id replaces it.
    pub id: String,
    pub title: String,
    pub body: String,
    pub kind: ToastKind,
    /// What clicking the notification does.
    pub default_action: Action,
    /// Buttons, by label.
    pub buttons: Vec<(String, Action)>,
    /// Let the desktop play its sound for it.
    pub sound: bool,
}

/// Shows and withdraws notifications.
#[async_trait]
pub trait Notifier: Send + Sync {
    fn kind(&self) -> NotifierKind;
    async fn show(&self, toast: &Toast) -> Result<(), String>;
    /// Withdraws the toast with this id, if it's still showing.
    async fn withdraw(&self, id: &str) -> Result<(), String>;
}

/// How notifications reach the desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NotifierKind {
    /// The Notification portal (Flatpak).
    Portal,
    /// `org.freedesktop.Notifications`.
    Freedesktop,
}

/// Where clicks go.
pub type InvokedSender = mpsc::UnboundedSender<Invoked>;

/// The app's notifications: connects to the desktop on first use (and again
/// later if that failed), and remembers which messages each notification
/// shown is about, so that it can go once they're read, moved or deleted.
pub struct Notifications {
    backend: tokio::sync::Mutex<Backend>,
    /// Toast id → the messages it's about.
    shown: Mutex<HashMap<String, Vec<String>>>,
    clicks: InvokedSender,
}

enum Backend {
    NotTried,
    Connected(Arc<dyn Notifier>),
    /// Nothing answered; try again after a while.
    Failed(Instant),
}

/// The most notifications the app remembers the messages of.
const REMEMBERED_TOASTS: usize = 1000;

/// How long after failing to reach a notification service the app tries
/// again (the desktop's shell may have been restarting).
const RETRY_CONNECT: Duration = Duration::from_secs(60);

impl Notifications {
    pub fn new(clicks: InvokedSender) -> Self {
        Self {
            backend: tokio::sync::Mutex::new(Backend::NotTried),
            shown: Mutex::default(),
            clicks,
        }
    }

    /// For tests: notifications through `notifier`.
    #[cfg(test)]
    pub fn with_notifier(notifier: Arc<dyn Notifier>, clicks: InvokedSender) -> Self {
        Self {
            backend: tokio::sync::Mutex::new(Backend::Connected(notifier)),
            shown: Mutex::default(),
            clicks,
        }
    }

    /// The connected notifier, connecting first if needed.
    pub async fn notifier(&self) -> Option<Arc<dyn Notifier>> {
        let mut backend = self.backend.lock().await;
        match &*backend {
            Backend::Connected(notifier) => return Some(Arc::clone(notifier)),
            Backend::Failed(at) if at.elapsed() < RETRY_CONNECT => return None,
            _ => {}
        }
        match connect(self.clicks.clone()).await {
            Ok(notifier) => {
                tracing::info!("notifications go through {:?}", notifier.kind());
                *backend = Backend::Connected(Arc::clone(&notifier));
                Some(notifier)
            }
            Err(e) => {
                tracing::warn!("no notification service: {e}");
                *backend = Backend::Failed(Instant::now());
                None
            }
        }
    }

    /// How notifications are shown, once connected; `None` until then or
    /// when no notification service answered.
    pub async fn kind(&self) -> Option<NotifierKind> {
        self.notifier().await.map(|n| n.kind())
    }

    /// Shows a toast about `messages` (none for the app's own).
    pub async fn show(&self, toast: &Toast, messages: Vec<String>) -> Result<(), String> {
        let notifier = self
            .notifier()
            .await
            .ok_or_else(|| "No notification service is running.".to_owned())?;
        notifier.show(toast).await?;
        let mut shown = self.shown.lock().unwrap();
        // Unread mail the app runs alongside for weeks: past this many, an
        // old notification is no longer withdrawn when its mail is read.
        if shown.len() >= REMEMBERED_TOASTS
            && let Some(old) = shown.keys().find(|k| **k != toast.id).cloned()
        {
            shown.remove(&old);
        }
        shown.insert(toast.id.clone(), messages);
        Ok(())
    }

    /// Withdraws a toast.
    pub async fn withdraw(&self, id: &str) {
        let known = self.shown.lock().unwrap().remove(id).is_some();
        if !known {
            return;
        }
        if let Some(notifier) = self.notifier().await
            && let Err(e) = notifier.withdraw(id).await
        {
            tracing::debug!("could not withdraw notification {id}: {e}");
        }
    }

    /// These messages were read, moved or deleted: withdraws the
    /// notifications that were only about them.
    pub async fn settle(&self, message_ids: &[String]) {
        let done = settled(&mut self.shown.lock().unwrap(), message_ids);
        if done.is_empty() {
            return;
        }
        let Some(notifier) = self.notifier().await else {
            return;
        };
        for id in done {
            if let Err(e) = notifier.withdraw(&id).await {
                tracing::debug!("could not withdraw notification {id}: {e}");
            }
        }
    }
}

/// Takes the settled messages out of the shown toasts, and those left
/// about nothing out of `shown`; returns their ids. The app's own toasts
/// (about no messages) stay.
fn settled(shown: &mut HashMap<String, Vec<String>>, message_ids: &[String]) -> Vec<String> {
    let mut done = Vec::new();
    shown.retain(|toast, messages| {
        if messages.is_empty() {
            return true;
        }
        messages.retain(|m| !message_ids.contains(m));
        if messages.is_empty() {
            done.push(toast.clone());
            false
        } else {
            true
        }
    });
    done.sort();
    done
}

#[cfg(target_os = "linux")]
async fn connect(clicks: InvokedSender) -> Result<Arc<dyn Notifier>, String> {
    if super::is_flatpak() {
        portal::PortalNotifier::connect(clicks)
            .await
            .map(|n| Arc::new(n) as Arc<dyn Notifier>)
    } else {
        let entry = super::desktop_entry();
        fdo::FdoNotifier::connect(clicks, entry)
            .await
            .map(|n| Arc::new(n) as Arc<dyn Notifier>)
    }
}

#[cfg(not(target_os = "linux"))]
async fn connect(_clicks: InvokedSender) -> Result<Arc<dyn Notifier>, String> {
    Err("notifications are only implemented for Linux".into())
}

// --- what new mail shows ------------------------------------------------------

/// A toast's id for a message.
pub fn mail_toast_id(message_id: &str) -> String {
    format!("mail:{message_id}")
}

/// What new mail's notifications show, from the settings and the account.
#[derive(Debug, Clone, Copy)]
pub struct MailToastOptions {
    /// The sender and subject (else "New message" and the account).
    pub preview: bool,
    pub sound: bool,
    /// The account has an Archive folder, so "Archive" can work.
    pub can_archive: bool,
}

/// The notifications for new mail: one per message with the sender as the
/// title and the subject as the text, or "New message" and the account
/// when previews are off; a summary for the account when there are many.
pub fn mail_toasts(
    account: &Account,
    alert: &MailAlert,
    options: MailToastOptions,
) -> Vec<(Toast, Vec<String>)> {
    let MailToastOptions {
        preview,
        sound,
        can_archive,
    } = options;
    match alert {
        MailAlert::Each(messages) => messages
            .iter()
            .map(|m| {
                let (title, body) = if preview {
                    (sender(m, account), subject(m))
                } else {
                    ("New message".to_owned(), account.email.clone())
                };
                let toast = Toast {
                    id: mail_toast_id(&m.id),
                    title,
                    body,
                    kind: ToastKind::Mail,
                    default_action: Action::OpenMessage(m.id.clone()),
                    buttons: std::iter::once((
                        "Mark as read".into(),
                        Action::MarkRead(m.id.clone()),
                    ))
                    .chain(can_archive.then(|| ("Archive".into(), Action::Archive(m.id.clone()))))
                    .collect(),
                    sound,
                };
                (toast, vec![m.id.clone()])
            })
            .collect(),
        MailAlert::Summary(messages) => {
            let body = if preview {
                summary_senders(messages, account)
            } else {
                account.email.clone()
            };
            let toast = Toast {
                id: format!("mail-summary:{}", account.id),
                title: format!("{} new messages", messages.len()),
                body,
                kind: ToastKind::Mail,
                default_action: Action::OpenInbox(account.id.clone()),
                buttons: Vec::new(),
                sound,
            };
            vec![(toast, messages.iter().map(|m| m.id.clone()).collect())]
        }
    }
}

/// Who a message is from, for a notification title.
fn sender(message: &MessageSummary, account: &Account) -> String {
    message
        .from
        .as_ref()
        .map(|f| {
            f.name
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .unwrap_or(&f.address)
                .to_owned()
        })
        .unwrap_or_else(|| account.display_name.clone())
}

fn subject(message: &MessageSummary) -> String {
    let subject = message.subject.trim();
    if subject.is_empty() {
        "(no subject)".into()
    } else {
        subject.to_owned()
    }
}

/// "From Ada, Grace and 3 others", newest first, each sender once.
fn summary_senders(messages: &[MessageSummary], account: &Account) -> String {
    let mut names: Vec<String> = Vec::new();
    for m in messages {
        let name = sender(m, account);
        if !names.contains(&name) {
            names.push(name);
        }
    }
    match names.as_slice() {
        [] => account.email.clone(),
        [one] => format!("From {one}"),
        [a, b] => format!("From {a} and {b}"),
        [a, b, rest @ ..] if rest.len() == 1 => format!("From {a}, {b} and {}", rest[0]),
        [a, b, rest @ ..] => format!("From {a}, {b} and {} others", rest.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use cumulas_core::model::{
        AccountKind, AccountSettings, EmailAddress, ImapSettings, Importance, TlsMode,
    };

    fn account() -> Account {
        Account {
            id: "acct".into(),
            kind: AccountKind::Imap,
            display_name: "Alice".into(),
            email: "alice@example.com".into(),
            color: "#0b7a75".into(),
            settings: AccountSettings::Imap(ImapSettings {
                imap_host: "imap.example.com".into(),
                imap_port: 993,
                imap_tls: TlsMode::Tls,
                smtp_host: "smtp.example.com".into(),
                smtp_port: 465,
                smtp_tls: TlsMode::Tls,
                username: "alice".into(),
            }),
            created_at: Utc::now(),
            last_synced_at: None,
            sync_error: None,
        }
    }

    fn message(id: &str, from: Option<(&str, &str)>, subject: &str) -> MessageSummary {
        MessageSummary {
            id: id.into(),
            account_id: "acct".into(),
            folder_id: "inbox".into(),
            remote_id: id.into(),
            conversation_id: None,
            subject: subject.into(),
            from: from.map(|(name, address)| {
                EmailAddress::new((!name.is_empty()).then_some(name), address)
            }),
            to: vec![],
            cc: vec![],
            preview: String::new(),
            received_at: Utc::now(),
            is_read: false,
            is_flagged: false,
            is_draft: false,
            has_attachments: false,
            importance: Importance::Normal,
        }
    }

    fn options(preview: bool, sound: bool) -> MailToastOptions {
        MailToastOptions {
            preview,
            sound,
            can_archive: true,
        }
    }

    #[test]
    fn archive_is_offered_only_with_an_archive_folder() {
        let alert = MailAlert::Each(vec![message("m1", None, "Hello")]);
        let without = MailToastOptions {
            can_archive: false,
            ..options(true, true)
        };
        let (toast, _) = &mail_toasts(&account(), &alert, without)[0];
        assert_eq!(
            toast.buttons,
            [("Mark as read".to_owned(), Action::MarkRead("m1".into()))]
        );
    }

    #[test]
    fn actions_survive_encoding() {
        for action in [
            Action::OpenMessage("5f0c-1:2".into()),
            Action::OpenInbox("acct".into()),
            Action::MarkRead("m1".into()),
            Action::Archive("m1".into()),
            Action::ShowWindow,
        ] {
            assert_eq!(Action::decode(&action.encode()), Some(action));
        }
        for junk in ["", "default", "open:", "delete:m1", "show:x", "open"] {
            assert_eq!(Action::decode(junk), None, "{junk:?}");
        }
    }

    #[test]
    fn one_notification_per_message_with_sender_subject_and_buttons() {
        let alert = MailAlert::Each(vec![
            message("m1", Some(("Ada Lovelace", "ada@example.com")), "Engines"),
            message("m2", Some(("", "grace@example.com")), "  "),
            message("m3", None, "Hello"),
        ]);
        let toasts = mail_toasts(&account(), &alert, options(true, true));
        let titles: Vec<_> = toasts
            .iter()
            .map(|(t, _)| (t.title.as_str(), t.body.as_str()))
            .collect();
        assert_eq!(
            titles,
            [
                ("Ada Lovelace", "Engines"),
                ("grace@example.com", "(no subject)"),
                ("Alice", "Hello"),
            ]
        );
        let (first, messages) = &toasts[0];
        assert_eq!(first.id, "mail:m1");
        assert_eq!(messages, &["m1"]);
        assert_eq!(first.default_action, Action::OpenMessage("m1".into()));
        assert_eq!(
            first.buttons,
            [
                ("Mark as read".to_owned(), Action::MarkRead("m1".into())),
                ("Archive".to_owned(), Action::Archive("m1".into())),
            ]
        );
        assert!(first.sound);
    }

    #[test]
    fn without_previews_only_the_account_shows() {
        let alert = MailAlert::Each(vec![message(
            "m1",
            Some(("Ada", "ada@example.com")),
            "Secret plans",
        )]);
        let (toast, _) = &mail_toasts(&account(), &alert, options(false, false))[0];
        assert_eq!(toast.title, "New message");
        assert_eq!(toast.body, "alice@example.com");
        assert!(!toast.sound);

        let summary = MailAlert::Summary(vec![
            message("m1", Some(("Ada", "ada@example.com")), "a"),
            message("m2", Some(("Bob", "bob@example.com")), "b"),
            message("m3", Some(("Cy", "cy@example.com")), "c"),
            message("m4", Some(("Di", "di@example.com")), "d"),
        ]);
        let (toast, _) = &mail_toasts(&account(), &summary, options(false, true))[0];
        assert_eq!(toast.body, "alice@example.com");
    }

    #[test]
    fn many_messages_give_one_summary_per_account() {
        let summary = MailAlert::Summary(vec![
            message("m1", Some(("Ada", "ada@example.com")), "a"),
            message("m2", Some(("Bob", "bob@example.com")), "b"),
            message("m3", Some(("Ada", "ada@example.com")), "c"),
            message("m4", Some(("Cy", "cy@example.com")), "d"),
            message("m5", Some(("Di", "di@example.com")), "e"),
        ]);
        let toasts = mail_toasts(&account(), &summary, options(true, true));
        assert_eq!(toasts.len(), 1);
        let (toast, messages) = &toasts[0];
        assert_eq!(toast.id, "mail-summary:acct");
        assert_eq!(toast.title, "5 new messages");
        assert_eq!(toast.body, "From Ada, Bob and 2 others");
        assert_eq!(toast.default_action, Action::OpenInbox("acct".into()));
        assert!(toast.buttons.is_empty());
        assert_eq!(messages.len(), 5);

        let three = MailAlert::Summary(vec![
            message("m1", Some(("Ada", "a@x")), "a"),
            message("m2", Some(("Bob", "b@x")), "b"),
            message("m3", Some(("Cy", "c@x")), "c"),
        ]);
        assert_eq!(
            mail_toasts(&account(), &three, options(true, true))[0]
                .0
                .body,
            "From Ada, Bob and Cy"
        );
    }

    /// Records what it's asked to do.
    #[derive(Default)]
    struct Recording {
        shown: Mutex<Vec<String>>,
        withdrawn: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl Notifier for Recording {
        fn kind(&self) -> NotifierKind {
            NotifierKind::Freedesktop
        }

        async fn show(&self, toast: &Toast) -> Result<(), String> {
            self.shown.lock().unwrap().push(toast.id.clone());
            Ok(())
        }

        async fn withdraw(&self, id: &str) -> Result<(), String> {
            self.withdrawn.lock().unwrap().push(id.to_owned());
            Ok(())
        }
    }

    #[tokio::test]
    async fn notifications_go_once_their_mail_is_read_elsewhere() {
        let recording = Arc::new(Recording::default());
        let (clicks, _) = mpsc::unbounded_channel();
        let notifications = Notifications::with_notifier(recording.clone(), clicks);
        let alert = MailAlert::Each(vec![
            message("m1", Some(("Ada", "ada@example.com")), "One"),
            message("m2", Some(("Bob", "bob@example.com")), "Two"),
        ]);
        for (toast, messages) in mail_toasts(&account(), &alert, options(true, true)) {
            notifications.show(&toast, messages).await.unwrap();
        }
        assert_eq!(*recording.shown.lock().unwrap(), ["mail:m1", "mail:m2"]);

        notifications.settle(&["m2".into(), "m9".into()]).await;
        assert_eq!(*recording.withdrawn.lock().unwrap(), ["mail:m2"]);
        // Withdrawn already: not again.
        notifications.withdraw("mail:m2").await;
        notifications.withdraw("mail:m1").await;
        assert_eq!(*recording.withdrawn.lock().unwrap(), ["mail:m2", "mail:m1"]);
    }

    #[test]
    fn settling_withdraws_only_toasts_left_about_nothing() {
        let mut shown = HashMap::from([
            ("mail:m1".to_owned(), vec!["m1".to_owned()]),
            ("mail:m2".to_owned(), vec!["m2".to_owned()]),
            (
                "mail-summary:a".to_owned(),
                vec!["m3".to_owned(), "m4".to_owned()],
            ),
            ("app:background".to_owned(), vec![]),
        ]);
        assert_eq!(
            settled(&mut shown, &["m1".into(), "m3".into()]),
            ["mail:m1"]
        );
        assert_eq!(settled(&mut shown, &["m4".into()]), ["mail-summary:a"]);
        let mut left: Vec<_> = shown.keys().cloned().collect();
        left.sort();
        assert_eq!(left, ["app:background", "mail:m2"]);
    }
}
