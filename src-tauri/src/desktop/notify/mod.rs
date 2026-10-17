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
//!
//! Subjects and senders' names say whatever the sender wants (and so do
//! the titles and locations of meetings others invite to), so every
//! toast's text is cleaned on its way out (see `clean`).
//!
//! Reminders show the same way, with Snooze and Dismiss (and Join for an
//! online meeting) buttons, and at a higher priority than mail; unlike
//! mail, they show while the window is in front too.

#[cfg(target_os = "linux")]
pub mod fdo;
#[cfg(target_os = "linux")]
pub mod portal;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Local};
use cumulas_core::model::{Account, ActiveReminder, MessageSummary, ReminderKind};
use cumulas_core::reminders::SUMMARY_THRESHOLD as REMINDER_SUMMARY_THRESHOLD;
use cumulas_core::service::MailAlert;
use futures::future::BoxFuture;
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
    /// Show the event in Calendar.
    OpenEvent(String),
    /// Show the task in Tasks.
    OpenTask(String),
    /// Show the Reminders panel (a summary of several reminders).
    OpenReminders,
    /// Snooze the reminder with this key, without showing the window.
    Snooze { key: String, minutes: u32 },
    /// Dismiss the reminder with this key, without showing the window.
    Dismiss(String),
    /// Join the online meeting at this (https) address.
    Join(String),
    /// Mark the task complete, without showing the window.
    CompleteTask(String),
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
            Action::OpenEvent(id) => format!("open-event:{id}"),
            Action::OpenTask(id) => format!("open-task:{id}"),
            Action::OpenReminders => "reminders".into(),
            Action::Snooze { key, minutes } => format!("snooze:{minutes}:{key}"),
            Action::Dismiss(key) => format!("dismiss:{key}"),
            Action::Join(url) => format!("join:{url}"),
            Action::CompleteTask(id) => format!("complete:{id}"),
        }
    }

    /// The action [`Action::encode`] made this string from, if any.
    pub fn decode(encoded: &str) -> Option<Self> {
        match encoded {
            "show" => return Some(Action::ShowWindow),
            "reminders" => return Some(Action::OpenReminders),
            _ => {}
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
            "open-event" => Action::OpenEvent(id),
            "open-task" => Action::OpenTask(id),
            "snooze" => {
                let (minutes, key) = id.split_once(':')?;
                let minutes = minutes.parse().ok().filter(|m| *m > 0)?;
                if key.is_empty() {
                    return None;
                }
                Action::Snooze {
                    key: key.to_owned(),
                    minutes,
                }
            }
            "dismiss" => Action::Dismiss(id),
            "join" => Action::Join(id),
            "complete" => Action::CompleteTask(id),
            _ => return None,
        })
    }

    /// Waits on a server: runs on its own, so that no other click waits
    /// for it.
    pub fn is_slow(&self) -> bool {
        matches!(
            self,
            Action::MarkRead(_)
                | Action::Archive(_)
                | Action::Snooze { .. }
                | Action::Dismiss(_)
                | Action::CompleteTask(_)
        )
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
    /// An event's or task's reminder: shown at a higher priority.
    Reminder,
    /// About the app itself (still running, a test, a button that failed).
    App,
}

/// A notification to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    /// Chosen by the app: `mail:{message}`, `mail-summary:{account}`,
    /// `reminder:{key}`, `reminder-summary` or `app:{what}`. Showing a toast
    /// with the same id replaces it.
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
    /// Its connection to the desktop is gone: it can't show anything any
    /// more, nor hear clicks.
    fn is_closed(&self) -> bool;
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
/// later if that failed, or the connection was lost), and remembers which
/// messages (or reminders) each notification shown is about, so that it can
/// go once they're read, moved or deleted (or dismissed, snoozed, done).
pub struct Notifications {
    backend: tokio::sync::Mutex<Backend>,
    connect: Connect,
    /// Toast id → when it was shown (a count of shows) and the messages
    /// it's about.
    shown: Mutex<HashMap<String, (u64, Vec<String>)>>,
    shows: AtomicU64,
    clicks: InvokedSender,
}

/// Reaches the desktop's notification service, which sends clicks to the
/// sender given.
pub(crate) type Connect = Box<
    dyn Fn(InvokedSender) -> BoxFuture<'static, Result<Arc<dyn Notifier>, String>> + Send + Sync,
>;

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

/// The most characters of a notification's title (and buttons), and of its
/// text: more than a desktop shows, where a subject can be any length.
const MAX_TITLE_CHARS: usize = 200;
const MAX_TEXT_CHARS: usize = 1000;

impl Notifications {
    pub fn new(clicks: InvokedSender) -> Self {
        Self::with_connect(Box::new(|clicks| Box::pin(connect(clicks))), clicks)
    }

    /// Notifications through whatever `connect` reaches.
    pub(crate) fn with_connect(connect: Connect, clicks: InvokedSender) -> Self {
        Self {
            backend: tokio::sync::Mutex::new(Backend::NotTried),
            connect,
            shown: Mutex::default(),
            shows: AtomicU64::new(0),
            clicks,
        }
    }

    /// For tests: notifications through `notifier`.
    #[cfg(test)]
    pub fn with_notifier(notifier: Arc<dyn Notifier>, clicks: InvokedSender) -> Self {
        let connect: Connect = Box::new(move |_| {
            let notifier = Arc::clone(&notifier);
            Box::pin(async move { Ok(notifier) })
        });
        Self::with_connect(connect, clicks)
    }

    /// The connected notifier, connecting first if needed.
    pub async fn notifier(&self) -> Option<Arc<dyn Notifier>> {
        let mut backend = self.backend.lock().await;
        match &*backend {
            Backend::Connected(notifier) if !notifier.is_closed() => {
                return Some(Arc::clone(notifier));
            }
            // The bus dropped the app, or went away: a new connection, which
            // also hears clicks again.
            Backend::Connected(_) => tracing::warn!("lost the notification service; reconnecting"),
            Backend::Failed(at) if at.elapsed() < RETRY_CONNECT => return None,
            _ => {}
        }
        match (self.connect)(self.clicks.clone()).await {
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
        notifier.show(&cleaned(toast)).await?;
        let mut shown = self.shown.lock().unwrap();
        let count = self.shows.fetch_add(1, Ordering::Relaxed);
        // Unread mail the app runs alongside for weeks: past this many, the
        // oldest notification is no longer withdrawn when its mail is read.
        if shown.len() >= REMEMBERED_TOASTS
            && !shown.contains_key(&toast.id)
            && let Some(oldest) = shown
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(id, _)| id.clone())
        {
            shown.remove(&oldest);
        }
        shown.insert(toast.id.clone(), (count, messages));
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
fn settled(shown: &mut HashMap<String, (u64, Vec<String>)>, message_ids: &[String]) -> Vec<String> {
    let mut done = Vec::new();
    shown.retain(|toast, (_, messages)| {
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

/// The toast as it goes to the desktop: its text [`clean`]ed.
fn cleaned(toast: &Toast) -> Toast {
    Toast {
        id: toast.id.clone(),
        title: clean(&toast.title, MAX_TITLE_CHARS),
        body: clean(&toast.body, MAX_TEXT_CHARS),
        kind: toast.kind,
        default_action: toast.default_action.clone(),
        buttons: toast
            .buttons
            .iter()
            .map(|(label, action)| (clean(label, MAX_TITLE_CHARS), action.clone()))
            .collect(),
        sound: toast.sound,
    }
}

/// Text that can go to the desktop, whatever a sender put in it:
/// - no control characters but line breaks and tabs (a NUL gets the app cut
///   off the bus, and with it every later notification and click);
/// - no bidi embeddings or overrides, and isolates only in pairs, so that
///   none reorders the text after it;
/// - at most `max` characters.
fn clean(text: &str, max: usize) -> String {
    let mut chars: Vec<char> = text
        .chars()
        .filter(|&c| {
            matches!(c, '\n' | '\t') || !(c.is_control() || matches!(c, '\u{202A}'..='\u{202E}'))
        })
        .collect();
    if chars.len() > max {
        chars.truncate(max - 1);
        chars.push('…');
    }
    // An isolate still open at the end of its line, or a close with none
    // open, goes.
    let mut keep = vec![true; chars.len()];
    let mut open = Vec::new();
    for (i, c) in chars.iter().enumerate() {
        match c {
            '\u{2066}'..='\u{2068}' => open.push(i),
            '\u{2069}' => keep[i] = open.pop().is_some(),
            '\n' | '\u{2029}' => {
                for j in open.drain(..) {
                    keep[j] = false;
                }
            }
            _ => {}
        }
    }
    for j in open {
        keep[j] = false;
    }
    chars
        .into_iter()
        .zip(keep)
        .filter_map(|(c, keep)| keep.then_some(c))
        .collect()
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

/// The notification that takes a message's place when its "Mark as read"
/// or "Archive" button failed (the desktop closed the notification on the
/// click, and the message would look handled): `title` says what failed,
/// the text which message, or the account without previews. Clicking it
/// opens the message, to try again there.
pub fn failed_toast(
    title: &str,
    message: &MessageSummary,
    account: &Account,
    preview: bool,
) -> Toast {
    Toast {
        id: mail_toast_id(&message.id),
        title: title.to_owned(),
        body: if preview {
            subject(message)
        } else {
            account.email.clone()
        },
        kind: ToastKind::App,
        default_action: Action::OpenMessage(message.id.clone()),
        buttons: Vec::new(),
        sound: false,
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

/// "From Ada, Grace and 3 others", newest first, each sender once. Each name
/// is isolated, so that it can't reorder the rest (a name in another script
/// would move the commas; an override in one, the whole line).
fn summary_senders(messages: &[MessageSummary], account: &Account) -> String {
    let mut names: Vec<String> = Vec::new();
    for m in messages {
        let name = sender(m, account);
        if !names.contains(&name) {
            names.push(name);
        }
    }
    // The name's own isolates go first: one left open would pair with the
    // close put around the name.
    let names: Vec<String> = names
        .iter()
        .map(|n| {
            let n = n.replace(['\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}'], "");
            format!("\u{2068}{n}\u{2069}")
        })
        .collect();
    match names.as_slice() {
        [] => account.email.clone(),
        [one] => format!("From {one}"),
        [a, b] => format!("From {a} and {b}"),
        [a, b, rest @ ..] if rest.len() == 1 => format!("From {a}, {b} and {}", rest[0]),
        [a, b, rest @ ..] => format!("From {a}, {b} and {} others", rest.len()),
    }
}

// --- what reminders show ------------------------------------------------------

/// A toast's id for a reminder.
pub fn reminder_toast_id(key: &str) -> String {
    format!("reminder:{key}")
}

/// What reminders' notifications offer, from the settings.
#[derive(Debug, Clone, Copy)]
pub struct ReminderToastOptions {
    /// For how long the Snooze button snoozes.
    pub snooze_minutes: u32,
    pub sound: bool,
}

/// The notifications for reminders that just came due: one each, with
/// Snooze and Dismiss (and Join for an online meeting, or Mark complete for
/// a task: never more than three buttons, which is all GNOME shows), or one
/// for them all when there are many (after the computer slept, say). Each
/// comes with the keys of the reminders it's about.
pub fn reminder_toasts(
    reminders: &[ActiveReminder],
    options: ReminderToastOptions,
    now: DateTime<Local>,
) -> Vec<(Toast, Vec<String>)> {
    if reminders.len() > REMINDER_SUMMARY_THRESHOLD {
        let toast = Toast {
            id: "reminder-summary".into(),
            title: format!("{} reminders", reminders.len()),
            body: summary_titles(reminders),
            kind: ToastKind::Reminder,
            default_action: Action::OpenReminders,
            buttons: Vec::new(),
            sound: options.sound,
        };
        return vec![(toast, reminders.iter().map(|r| r.key.clone()).collect())];
    }
    reminders
        .iter()
        .map(|r| (reminder_toast(r, options, now), vec![r.key.clone()]))
        .collect()
}

fn reminder_toast(
    r: &ActiveReminder,
    options: ReminderToastOptions,
    now: DateTime<Local>,
) -> Toast {
    let minutes = options.snooze_minutes.max(1);
    let mut buttons = vec![
        (
            snooze_label(minutes),
            Action::Snooze {
                key: r.key.clone(),
                minutes,
            },
        ),
        ("Dismiss".to_owned(), Action::Dismiss(r.key.clone())),
    ];
    let default_action = match r.kind {
        ReminderKind::Event => {
            if let Some(url) = r.online_meeting_url.as_deref().filter(|u| is_https(u)) {
                buttons.push(("Join".into(), Action::Join(url.to_owned())));
            }
            Action::OpenEvent(r.item_id.clone())
        }
        ReminderKind::Task => {
            buttons.push((
                "Mark complete".into(),
                Action::CompleteTask(r.item_id.clone()),
            ));
            Action::OpenTask(r.item_id.clone())
        }
    };
    Toast {
        id: reminder_toast_id(&r.key),
        title: reminder_title(r),
        body: reminder_when(r, now),
        kind: ToastKind::Reminder,
        default_action,
        buttons,
        sound: options.sound,
    }
}

/// The notification that takes a reminder's place when its button failed
/// (the desktop closed the notification on the click). Clicking it opens
/// the Reminders panel, to try again there.
pub fn failed_reminder_toast(title: &str, reminder: Option<&ActiveReminder>) -> Toast {
    Toast {
        id: reminder.map_or_else(|| "app:reminder".into(), |r| reminder_toast_id(&r.key)),
        title: title.to_owned(),
        body: reminder.map(reminder_title).unwrap_or_default(),
        kind: ToastKind::App,
        default_action: Action::OpenReminders,
        buttons: Vec::new(),
        sound: false,
    }
}

/// An address the Join button can open: a web address over https.
pub fn is_https(url: &str) -> bool {
    url::Url::parse(url.trim()).is_ok_and(|u| u.scheme() == "https" && u.host().is_some())
}

/// "Snooze 5 min", "Snooze 1 hour".
fn snooze_label(minutes: u32) -> String {
    match minutes {
        60 => "Snooze 1 hour".into(),
        m if m % 60 == 0 => format!("Snooze {} hours", m / 60),
        m => format!("Snooze {m} min"),
    }
}

fn reminder_title(r: &ActiveReminder) -> String {
    let title = r.title.trim();
    if !title.is_empty() {
        return title.to_owned();
    }
    match r.kind {
        ReminderKind::Event => "(no title)".into(),
        ReminderKind::Task => "(untitled task)".into(),
    }
}

/// When the event is, and where: "Today at 10:00 AM · Room 3", "Started
/// at 9:30 AM", "Tomorrow at 9:00 AM", "Fri, Oct 16 at 2:00 PM", "All day
/// tomorrow"; "Task" for a task.
fn reminder_when(r: &ActiveReminder, now: DateTime<Local>) -> String {
    let Some(start) = r.starts_at.map(|s| s.with_timezone(&Local)) else {
        return "Task".into();
    };
    let days = (start.date_naive() - now.date_naive()).num_days();
    let all_day = r.ends_at.is_some_and(|end| {
        let end = end.with_timezone(&Local);
        start.time() == chrono::NaiveTime::MIN
            && end.time() == chrono::NaiveTime::MIN
            && end > start
    });
    let when = if all_day {
        match days {
            0 => "All day today".to_owned(),
            1 => "All day tomorrow".to_owned(),
            _ => format!("All day {}", start.format("%a, %b %-d")),
        }
    } else {
        let time = start.format("%-I:%M %p");
        match days {
            0 if start <= now => format!("Started at {time}"),
            0 => format!("Today at {time}"),
            1 => format!("Tomorrow at {time}"),
            -1 => format!("Started yesterday at {time}"),
            _ => format!("{} at {time}", start.format("%a, %b %-d")),
        }
    };
    match r
        .location
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty())
    {
        Some(location) => format!("{when} · {location}"),
        None => when,
    }
}

/// "Standup, Design review and 3 more", each title isolated (see
/// `summary_senders`).
fn summary_titles(reminders: &[ActiveReminder]) -> String {
    let titles: Vec<String> = reminders
        .iter()
        .map(|r| {
            let t = reminder_title(r).replace(['\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}'], "");
            format!("\u{2068}{t}\u{2069}")
        })
        .collect();
    match titles.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [a, b] => format!("{a} and {b}"),
        [a, b, rest @ ..] => format!("{a}, {b} and {} more", rest.len()),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::atomic::AtomicBool;

    use super::*;
    use chrono::Utc;
    use cumulas_core::model::{
        AccountKind, AccountSettings, EmailAddress, ImapSettings, Importance, TlsMode,
    };

    pub(crate) fn account() -> Account {
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
            needs_permission: vec![],
            refused_features: vec![],
        }
    }

    pub(crate) fn message(id: &str, from: Option<(&str, &str)>, subject: &str) -> MessageSummary {
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
            Action::OpenEvent("e1".into()),
            Action::OpenTask("t1".into()),
            Action::OpenReminders,
            Action::Snooze {
                key: "event:5f0c-1:1792040700".into(),
                minutes: 15,
            },
            Action::Dismiss("task:7a:1792040700".into()),
            Action::Join("https://teams.example/l/meetup-join/19%3a?context=%7b%7d".into()),
            Action::CompleteTask("t1".into()),
        ] {
            assert_eq!(Action::decode(&action.encode()), Some(action));
        }
        for junk in [
            "",
            "default",
            "open:",
            "delete:m1",
            "show:x",
            "open",
            "snooze:5",
            "snooze:0:event:e:1",
            "snooze:five:event:e:1",
            "snooze:5:",
            "dismiss:",
            "reminders:x",
        ] {
            assert_eq!(Action::decode(junk), None, "{junk:?}");
        }
    }

    pub(crate) fn reminder(
        kind: ReminderKind,
        title: &str,
        starts_at: Option<DateTime<Local>>,
    ) -> ActiveReminder {
        let fire_at = starts_at.map_or_else(chrono::Utc::now, |s| {
            s.with_timezone(&chrono::Utc) - chrono::Duration::minutes(15)
        });
        ActiveReminder {
            key: format!("{}:{title}:{}", kind.as_str(), fire_at.timestamp()),
            kind,
            item_id: title.to_lowercase(),
            account_id: "acct".into(),
            title: title.into(),
            starts_at: starts_at.map(|s| s.with_timezone(&chrono::Utc)),
            ends_at: starts_at
                .map(|s| s.with_timezone(&chrono::Utc) + chrono::Duration::minutes(30)),
            location: None,
            online_meeting_url: None,
            fire_at,
            snoozed_until: None,
            overdue: false,
            missed: false,
        }
    }

    fn local(day: u32, hour: u32, minute: u32) -> DateTime<Local> {
        use chrono::TimeZone;
        Local
            .with_ymd_and_hms(2026, 10, day, hour, minute, 0)
            .unwrap()
    }

    #[test]
    fn a_reminder_snoozes_dismisses_and_joins_from_its_notification() {
        let now = local(15, 9, 45);
        let mut standup = reminder(ReminderKind::Event, "Standup", Some(local(15, 10, 0)));
        standup.location = Some("Room 3".into());
        standup.online_meeting_url = Some("https://teams.example/join/1".into());
        let options = ReminderToastOptions {
            snooze_minutes: 10,
            sound: true,
        };
        let toasts = reminder_toasts(std::slice::from_ref(&standup), options, now);
        let [(toast, keys)] = toasts.as_slice() else {
            panic!("{toasts:?}");
        };
        assert_eq!(keys, std::slice::from_ref(&standup.key));
        assert_eq!(toast.id, format!("reminder:{}", standup.key));
        assert_eq!(toast.title, "Standup");
        assert_eq!(toast.body, "Today at 10:00 AM · Room 3");
        assert_eq!(toast.kind, ToastKind::Reminder);
        assert_eq!(toast.default_action, Action::OpenEvent("standup".into()));
        assert_eq!(
            toast.buttons,
            [
                (
                    "Snooze 10 min".to_owned(),
                    Action::Snooze {
                        key: standup.key.clone(),
                        minutes: 10
                    }
                ),
                ("Dismiss".to_owned(), Action::Dismiss(standup.key.clone())),
                (
                    "Join".to_owned(),
                    Action::Join("https://teams.example/join/1".into())
                ),
            ]
        );
        assert!(toast.sound);

        // Only https meetings get a Join button.
        for url in [
            "http://meet.example/1",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "https://",
        ] {
            standup.online_meeting_url = Some(url.into());
            let (toast, _) = &reminder_toasts(std::slice::from_ref(&standup), options, now)[0];
            assert_eq!(toast.buttons.len(), 2, "{url}");
        }
    }

    #[test]
    fn a_task_reminder_can_be_completed_from_its_notification() {
        let task = reminder(ReminderKind::Task, "Pay rent", None);
        let options = ReminderToastOptions {
            snooze_minutes: 60,
            sound: false,
        };
        let (toast, _) = &reminder_toasts(std::slice::from_ref(&task), options, local(15, 9, 0))[0];
        assert_eq!(toast.body, "Task");
        assert_eq!(toast.default_action, Action::OpenTask("pay rent".into()));
        let labels: Vec<&str> = toast.buttons.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["Snooze 1 hour", "Dismiss", "Mark complete"]);
        assert_eq!(toast.buttons[2].1, Action::CompleteTask("pay rent".into()));
        assert!(!toast.sound);
    }

    #[test]
    fn reminders_say_when() {
        let now = local(15, 9, 45);
        let when = |start| reminder_when(&reminder(ReminderKind::Event, "x", Some(start)), now);
        assert_eq!(when(local(15, 9, 30)), "Started at 9:30 AM");
        assert_eq!(when(local(15, 14, 0)), "Today at 2:00 PM");
        assert_eq!(when(local(16, 9, 0)), "Tomorrow at 9:00 AM");
        assert_eq!(when(local(14, 23, 0)), "Started yesterday at 11:00 PM");
        assert_eq!(when(local(20, 9, 0)), "Tue, Oct 20 at 9:00 AM");
        let mut all_day = reminder(ReminderKind::Event, "Trip", Some(local(16, 0, 0)));
        all_day.ends_at = Some(local(18, 0, 0).with_timezone(&chrono::Utc));
        assert_eq!(reminder_when(&all_day, now), "All day tomorrow");
        assert_eq!(snooze_label(5), "Snooze 5 min");
        assert_eq!(snooze_label(90), "Snooze 90 min");
        assert_eq!(snooze_label(120), "Snooze 2 hours");
        let mut untitled = reminder(ReminderKind::Event, " ", None);
        untitled.title = "  ".into();
        assert_eq!(reminder_title(&untitled), "(no title)");
    }

    #[test]
    fn many_reminders_at_once_give_one_notification() {
        let reminders: Vec<ActiveReminder> = ["Standup", "Review", "Lunch", "1:1"]
            .into_iter()
            .map(|t| reminder(ReminderKind::Event, t, Some(local(15, 10, 0))))
            .collect();
        let options = ReminderToastOptions {
            snooze_minutes: 5,
            sound: true,
        };
        let toasts = reminder_toasts(&reminders, options, local(15, 9, 50));
        let [(toast, keys)] = toasts.as_slice() else {
            panic!("{toasts:?}");
        };
        assert_eq!(toast.id, "reminder-summary");
        assert_eq!(toast.title, "4 reminders");
        assert_eq!(
            toast.body,
            "\u{2068}Standup\u{2069}, \u{2068}Review\u{2069} and 2 more"
        );
        assert_eq!(toast.default_action, Action::OpenReminders);
        assert!(toast.buttons.is_empty());
        assert_eq!(keys.len(), 4);
        // Three still go one by one.
        assert_eq!(
            reminder_toasts(&reminders[..3], options, local(15, 9, 50)).len(),
            3
        );
    }

    #[tokio::test]
    async fn a_reminder_s_notification_goes_once_it_s_no_longer_due() {
        let recording = Arc::new(Recording::default());
        let (clicks, _) = mpsc::unbounded_channel();
        let notifications = Notifications::with_notifier(recording.clone(), clicks);
        let options = ReminderToastOptions {
            snooze_minutes: 5,
            sound: true,
        };
        let one = reminder(ReminderKind::Event, "Standup", Some(local(15, 10, 0)));
        let many: Vec<ActiveReminder> = ["A", "B", "C", "D"]
            .into_iter()
            .map(|t| reminder(ReminderKind::Task, t, None))
            .collect();
        for reminders in [std::slice::from_ref(&one), many.as_slice()] {
            for (toast, keys) in reminder_toasts(reminders, options, local(15, 9, 50)) {
                notifications.show(&toast, keys).await.unwrap();
            }
        }
        // Mail read elsewhere touches no reminder.
        notifications.settle(&["m1".into()]).await;
        assert!(recording.withdrawn.lock().unwrap().is_empty());
        notifications
            .settle(&[one.key.clone(), many[0].key.clone()])
            .await;
        assert_eq!(
            *recording.withdrawn.lock().unwrap(),
            [format!("reminder:{}", one.key)]
        );
        let rest: Vec<String> = many[1..].iter().map(|r| r.key.clone()).collect();
        notifications.settle(&rest).await;
        assert_eq!(recording.withdrawn.lock().unwrap()[1], "reminder-summary");
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
        assert_eq!(
            toast.body,
            "From \u{2068}Ada\u{2069}, \u{2068}Bob\u{2069} and 2 others"
        );
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
            "From \u{2068}Ada\u{2069}, \u{2068}Bob\u{2069} and \u{2068}Cy\u{2069}"
        );
    }

    #[test]
    fn a_sender_s_name_can_t_reorder_the_rest_of_the_summary() {
        // A right-to-left override, and an isolate that would take the
        // place of the one around the name, left open.
        let summary = MailAlert::Summary(vec![
            message("m1", Some(("Mallory\u{202E}\u{2067}", "m@x")), "a"),
            message("m2", Some(("Bob", "b@x")), "b"),
            message("m3", Some(("Cy", "c@x")), "c"),
            message("m4", Some(("Di", "d@x")), "d"),
        ]);
        let (toast, _) = &mail_toasts(&account(), &summary, options(true, true))[0];
        assert_eq!(
            toast.body,
            "From \u{2068}Mallory\u{202E}\u{2069}, \u{2068}Bob\u{2069} and 2 others"
        );
    }

    /// Records what it's asked to do.
    #[derive(Default)]
    pub(crate) struct Recording {
        pub(crate) shown: Mutex<Vec<Toast>>,
        pub(crate) withdrawn: Mutex<Vec<String>>,
        closed: AtomicBool,
        /// Shows nothing: the server is gone (restarting, say).
        pub(crate) failing: AtomicBool,
    }

    impl Recording {
        pub(crate) fn shown_ids(&self) -> Vec<String> {
            self.shown
                .lock()
                .unwrap()
                .iter()
                .map(|t| t.id.clone())
                .collect()
        }
    }

    #[async_trait]
    impl Notifier for Recording {
        fn kind(&self) -> NotifierKind {
            NotifierKind::Freedesktop
        }

        fn is_closed(&self) -> bool {
            self.closed.load(Ordering::SeqCst)
        }

        async fn show(&self, toast: &Toast) -> Result<(), String> {
            if self.failing.load(Ordering::SeqCst) {
                return Err("The name is not activatable".into());
            }
            self.shown.lock().unwrap().push(toast.clone());
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
        assert_eq!(recording.shown_ids(), ["mail:m1", "mail:m2"]);

        notifications.settle(&["m2".into(), "m9".into()]).await;
        assert_eq!(*recording.withdrawn.lock().unwrap(), ["mail:m2"]);
        // Withdrawn already: not again.
        notifications.withdraw("mail:m2").await;
        notifications.withdraw("mail:m1").await;
        assert_eq!(*recording.withdrawn.lock().unwrap(), ["mail:m2", "mail:m1"]);
    }

    #[tokio::test]
    async fn text_from_mail_is_cleaned_before_it_reaches_the_desktop() {
        let recording = Arc::new(Recording::default());
        let (clicks, _) = mpsc::unbounded_channel();
        let notifications = Notifications::with_notifier(recording.clone(), clicks);
        // What `Subject: =?UTF-8?B?SGkAdGhlcmU=?=` and a From name of
        // `=?UTF-8?B?QWRhAA==?=` decode to. A NUL gets the app cut off the
        // bus, and with it every later notification and click.
        let alert = MailAlert::Each(vec![message(
            "m1",
            Some(("Ada\0", "ada@example.com")),
            "Hi\0there\u{7}\r\n\u{9b}again\u{202E}!",
        )]);
        for (toast, messages) in mail_toasts(&account(), &alert, options(true, true)) {
            notifications.show(&toast, messages).await.unwrap();
        }
        {
            let shown = recording.shown.lock().unwrap();
            assert_eq!(shown[0].title, "Ada");
            assert_eq!(shown[0].body, "Hithere\nagain!");
            assert_eq!(shown[0].default_action, Action::OpenMessage("m1".into()));
        }

        let long = message("m2", None, &"x".repeat(5000));
        let alert = MailAlert::Each(vec![long]);
        for (toast, messages) in mail_toasts(&account(), &alert, options(true, true)) {
            notifications.show(&toast, messages).await.unwrap();
        }
        let body = &recording.shown.lock().unwrap()[1].body;
        assert!(body.chars().count() <= 1000, "{}", body.len());
        assert!(body.ends_with('…'));
    }

    #[test]
    fn cleaning_keeps_lines_tabs_and_isolates_in_pairs() {
        assert_eq!(clean("Q1\tplan\nNext", 100), "Q1\tplan\nNext");
        assert_eq!(clean("a\0b\u{1b}[31mc\u{7f}\u{85}d", 100), "ab[31mcd");
        // Embeddings and overrides go; marks stay.
        assert_eq!(
            clean("\u{202A}a\u{202B}b\u{202C}c\u{202D}d\u{202E}e\u{200F}", 100),
            "abcde\u{200F}"
        );
        // Isolates that close stay; an open one or a stray close goes,
        // within each line.
        let summary = "From \u{2068}Ada\u{2069}, \u{2068}Bob\u{2069}";
        assert_eq!(clean(summary, 100), summary);
        let nested = "\u{2067}a\u{2068}b\u{2069}\u{2069}c";
        assert_eq!(clean(&format!("{nested}\u{2066}"), 100), nested);
        assert_eq!(clean("a\u{2069}b\u{2067}c", 100), "abc");
        assert_eq!(clean("\u{2066}a\nb\u{2069}", 100), "a\nb");
        // At most so many characters, an isolate cut open included.
        assert_eq!(clean("abcdef", 4), "abc…");
        assert_eq!(clean("añb", 3), "añb");
        assert_eq!(clean("ab\u{2068}cdef\u{2069}", 5), "abc…");
    }

    #[tokio::test]
    async fn a_lost_connection_is_made_again() {
        let first = Arc::new(Recording::default());
        let second = Arc::new(Recording::default());
        let connections = Arc::new(AtomicU64::new(0));
        let connect: Connect = {
            let (first, second) = (Arc::clone(&first), Arc::clone(&second));
            let connections = Arc::clone(&connections);
            Box::new(move |_| {
                let notifier: Arc<dyn Notifier> = match connections.fetch_add(1, Ordering::SeqCst) {
                    0 => first.clone(),
                    _ => second.clone(),
                };
                Box::pin(async move { Ok(notifier) })
            })
        };
        let (clicks, _) = mpsc::unbounded_channel();
        let notifications = Notifications::with_connect(connect, clicks);
        let alert = MailAlert::Each(vec![
            message("m1", Some(("Ada", "ada@example.com")), "One"),
            message("m2", Some(("Bob", "bob@example.com")), "Two"),
        ]);
        let toasts = mail_toasts(&account(), &alert, options(true, true));
        notifications
            .show(&toasts[0].0, toasts[0].1.clone())
            .await
            .unwrap();
        notifications
            .show(&toasts[0].0, toasts[0].1.clone())
            .await
            .unwrap();
        assert_eq!(connections.load(Ordering::SeqCst), 1);

        // The bus dropped the app.
        first.closed.store(true, Ordering::SeqCst);
        notifications
            .show(&toasts[1].0, toasts[1].1.clone())
            .await
            .unwrap();
        assert_eq!(connections.load(Ordering::SeqCst), 2);
        assert_eq!(first.shown_ids(), ["mail:m1", "mail:m1"]);
        assert_eq!(second.shown_ids(), ["mail:m2"]);
        assert_eq!(notifications.kind().await, Some(NotifierKind::Freedesktop));
        assert_eq!(connections.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_failed_button_says_so_about_its_message() {
        let mail = message("m1", Some(("Ada", "ada@example.com")), "Engines");
        let toast = failed_toast("Couldn't archive the message", &mail, &account(), true);
        assert_eq!(toast.id, "mail:m1");
        assert_eq!(toast.title, "Couldn't archive the message");
        assert_eq!(toast.body, "Engines");
        assert_eq!(toast.default_action, Action::OpenMessage("m1".into()));
        assert!(toast.buttons.is_empty());
        assert!(!toast.sound);
        let private = failed_toast("Couldn't archive the message", &mail, &account(), false);
        assert_eq!(private.body, "alice@example.com");
    }

    #[tokio::test]
    async fn past_the_cap_the_oldest_toast_is_forgotten() {
        let recording = Arc::new(Recording::default());
        let (clicks, _) = mpsc::unbounded_channel();
        let notifications = Notifications::with_notifier(recording.clone(), clicks);
        let show = |n: usize| {
            let alert = MailAlert::Each(vec![message(&format!("m{n}"), None, "Hello")]);
            mail_toasts(&account(), &alert, options(true, true)).remove(0)
        };
        // One more than the cap: the first one goes.
        for n in 0..=REMEMBERED_TOASTS {
            let (toast, messages) = show(n);
            notifications.show(&toast, messages).await.unwrap();
        }
        // Shown again, so not one more: nothing goes.
        let (toast, messages) = show(REMEMBERED_TOASTS);
        notifications.show(&toast, messages).await.unwrap();

        let all: Vec<String> = (0..=REMEMBERED_TOASTS).map(|n| format!("m{n}")).collect();
        notifications.settle(&all).await;
        let withdrawn = recording.withdrawn.lock().unwrap();
        assert_eq!(withdrawn.len(), REMEMBERED_TOASTS);
        assert!(!withdrawn.contains(&"mail:m0".to_owned()));
    }

    #[test]
    fn settling_withdraws_only_toasts_left_about_nothing() {
        let mut shown = HashMap::from([
            ("mail:m1".to_owned(), (0, vec!["m1".to_owned()])),
            ("mail:m2".to_owned(), (1, vec!["m2".to_owned()])),
            (
                "mail-summary:a".to_owned(),
                (2, vec!["m3".to_owned(), "m4".to_owned()]),
            ),
            ("app:background".to_owned(), (3, vec![])),
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
