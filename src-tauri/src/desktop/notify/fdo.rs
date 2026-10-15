//! Notifications through `org.freedesktop.Notifications`, for native
//! packages (deb, rpm, AppImage), where the app talks to the desktop's
//! notification server directly.
//!
//! The server numbers notifications itself, so the app keeps a small map
//! from its own toast ids to the server's (to replace and withdraw them)
//! and back (to know which of the clicks it hears about are its own: the
//! server's signals go to every client). A server that starts again (after
//! a crash, or another taking over) numbers from scratch, so the map is
//! forgotten then.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::StreamExt;
use zbus::zvariant::Value;

use super::{Action, Invoked, InvokedSender, Notifier, NotifierKind, Toast, ToastKind};

/// The most notifications the app remembers; older ones can no longer be
/// replaced, withdrawn or clicked through.
const REMEMBERED: usize = 256;

/// `NotificationClosed`'s reason for a notification that timed out.
const EXPIRED: u32 = 1;

#[zbus::proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications",
    gen_blocking = false
)]
pub trait Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;

    fn close_notification(&self, id: u32) -> zbus::Result<()>;

    fn get_capabilities(&self) -> zbus::Result<Vec<String>>;

    #[zbus(signal)]
    fn action_invoked(&self, id: u32, action_key: String) -> zbus::Result<()>;

    #[zbus(signal)]
    fn activation_token(&self, id: u32, activation_token: String) -> zbus::Result<()>;

    #[zbus(signal)]
    fn notification_closed(&self, id: u32, reason: u32) -> zbus::Result<()>;
}

/// Which desktop file the notifications belong to: its id (the file name
/// without `.desktop`) and its icon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopEntry {
    pub id: String,
    pub icon: String,
}

/// The app's notifications on the server, both ways.
#[derive(Debug, Default)]
pub(crate) struct IdMap {
    by_toast: HashMap<String, u32>,
    by_server: HashMap<u32, Shown>,
    /// Server ids, oldest first, to forget the oldest beyond [`REMEMBERED`].
    order: VecDeque<u32>,
    /// Activation tokens that arrived ahead of their click.
    tokens: HashMap<u32, String>,
    /// The server the ids are from: its unique name on the bus, when known.
    server: Option<String>,
}

#[derive(Debug, Clone)]
struct Shown {
    toast_id: String,
    default_action: Action,
    /// The server closed it as timed out, which can leave it in the
    /// desktop's history (KDE Plasma's) until the app closes it.
    expired: bool,
}

impl IdMap {
    /// The server id a new toast with this id replaces (0 for none).
    pub(crate) fn replaces(&self, toast_id: &str) -> u32 {
        self.by_toast.get(toast_id).copied().unwrap_or(0)
    }

    /// The server showed `toast` as `server_id`; returns the server ids of
    /// the toast's earlier showings that expired, to close now.
    pub(crate) fn shown(&mut self, toast: &Toast, server_id: u32) -> Vec<u32> {
        if let Some(old) = self.by_toast.insert(toast.id.clone(), server_id)
            && old != server_id
        {
            self.by_server.remove(&old);
        }
        let mut expired = self.take_expired(&toast.id);
        expired.retain(|id| *id != server_id);
        self.by_server.insert(
            server_id,
            Shown {
                toast_id: toast.id.clone(),
                default_action: toast.default_action.clone(),
                expired: false,
            },
        );
        self.order.retain(|id| *id != server_id);
        self.order.push_back(server_id);
        while self.order.len() > REMEMBERED {
            if let Some(oldest) = self.order.pop_front()
                && let Some(shown) = self.by_server.remove(&oldest)
                && self.by_toast.get(&shown.toast_id) == Some(&oldest)
            {
                self.by_toast.remove(&shown.toast_id);
            }
        }
        expired
    }

    /// Forgets a toast the app withdraws; returns the server ids to close:
    /// its own, and its earlier showings' that expired.
    pub(crate) fn withdrawn(&mut self, toast_id: &str) -> Vec<u32> {
        let mut server_ids = self.take_expired(toast_id);
        if let Some(server_id) = self.by_toast.remove(toast_id) {
            self.by_server.remove(&server_id);
            self.tokens.remove(&server_id);
            server_ids.push(server_id);
        }
        server_ids
    }

    /// The server closed one (dismissed, expired, clicked): it can't be
    /// replaced any more (Plasma would keep a replacement for an expired one
    /// in its history, without a popup), and only an expired one can still
    /// be withdrawn. What a click on it does is kept until it's forgotten,
    /// in case the click is reported after the close.
    pub(crate) fn closed(&mut self, server_id: u32, reason: u32) {
        if let Some(shown) = self.by_server.get_mut(&server_id)
            && self.by_toast.get(&shown.toast_id) == Some(&server_id)
        {
            self.by_toast.remove(&shown.toast_id);
            shown.expired = reason == EXPIRED;
        }
        self.tokens.remove(&server_id);
    }

    /// Takes out the server ids of a toast's showings that expired.
    fn take_expired(&mut self, toast_id: &str) -> Vec<u32> {
        let server_ids: Vec<u32> = self
            .by_server
            .iter()
            .filter(|(_, shown)| shown.expired && shown.toast_id == toast_id)
            .map(|(server_id, _)| *server_id)
            .collect();
        for server_id in &server_ids {
            self.by_server.remove(server_id);
        }
        server_ids
    }

    /// The server is now `server` (its unique name; `None` for none). Another
    /// one than before numbers its notifications afresh: none of the ids are
    /// its.
    pub(crate) fn served_by(&mut self, server: Option<&str>) {
        if self.server.as_deref() != server {
            *self = Self {
                server: server.map(str::to_owned),
                ..Self::default()
            };
        }
    }

    pub(crate) fn activation_token(&mut self, server_id: u32, token: String) {
        if self.by_server.contains_key(&server_id) {
            self.tokens.insert(server_id, token);
        }
    }

    /// What a click on one of the app's notifications asks for: `default`
    /// is the click on the notification itself, other keys are the buttons'
    /// encoded actions. `None` for other apps' notifications.
    pub(crate) fn invoked(&mut self, server_id: u32, action_key: &str) -> Option<Invoked> {
        let shown = self.by_server.get(&server_id)?;
        let action = if action_key == "default" {
            shown.default_action.clone()
        } else {
            Action::decode(action_key)?
        };
        Some(Invoked {
            action,
            activation_token: self.tokens.remove(&server_id),
        })
    }
}

/// What the server can do, from `GetCapabilities`.
#[derive(Debug, Clone, Copy, Default)]
struct Capabilities {
    actions: bool,
    /// The body is markup, so text in it needs escaping.
    body_markup: bool,
}

pub struct FdoNotifier {
    proxy: NotificationsProxy<'static>,
    ids: Arc<Mutex<IdMap>>,
    capabilities: Capabilities,
    entry: DesktopEntry,
}

impl FdoNotifier {
    /// Connects to the session bus's notification server.
    pub async fn connect(clicks: InvokedSender, entry: DesktopEntry) -> Result<Self, String> {
        let connection = zbus::Connection::session()
            .await
            .map_err(|e| format!("no session bus: {e}"))?;
        Self::with_connection(&connection, clicks, entry).await
    }

    /// Uses `connection`'s notification server, and listens there for
    /// clicks until the connection closes.
    pub async fn with_connection(
        connection: &zbus::Connection,
        clicks: InvokedSender,
        entry: DesktopEntry,
    ) -> Result<Self, String> {
        let proxy = NotificationsProxy::new(connection)
            .await
            .map_err(|e| e.to_string())?;
        let capabilities = proxy
            .get_capabilities()
            .await
            .map_err(|e| format!("the notification server didn't answer: {e}"))?;
        let capabilities = Capabilities {
            actions: capabilities.iter().any(|c| c == "actions"),
            body_markup: capabilities.iter().any(|c| c == "body-markup"),
        };
        let ids = Arc::new(Mutex::new(IdMap::default()));
        listen(&proxy, Arc::clone(&ids), clicks).await?;
        Ok(Self {
            proxy,
            ids,
            capabilities,
            entry,
        })
    }
}

/// Follows the server's signals (activation tokens, clicks and closes), and
/// which server it is.
async fn listen(
    proxy: &NotificationsProxy<'static>,
    ids: Arc<Mutex<IdMap>>,
    clicks: InvokedSender,
) -> Result<(), String> {
    let mut owner = proxy
        .inner()
        .receive_owner_changed()
        .await
        .map_err(|e| e.to_string())?;
    let mut invoked = proxy
        .receive_action_invoked()
        .await
        .map_err(|e| e.to_string())?;
    let mut tokens = proxy
        .receive_activation_token()
        .await
        .map_err(|e| e.to_string())?;
    let mut closed = proxy
        .receive_notification_closed()
        .await
        .map_err(|e| e.to_string())?;
    tokio::spawn(async move {
        loop {
            tokio::select! {
                // The old server's ids go before the new one's signals; the
                // token comes before its click.
                biased;
                Some(owner) = owner.next() => {
                    ids.lock().unwrap().served_by(owner.as_ref().map(|o| o.as_str()));
                }
                Some(signal) = tokens.next() => {
                    if let Ok(args) = signal.args() {
                        ids.lock()
                            .unwrap()
                            .activation_token(args.id, args.activation_token);
                    }
                }
                Some(signal) = invoked.next() => {
                    let Ok(args) = signal.args() else { continue };
                    let click = ids.lock().unwrap().invoked(args.id, &args.action_key);
                    if let Some(click) = click
                        && clicks.send(click).is_err()
                    {
                        return;
                    }
                }
                Some(signal) = closed.next() => {
                    if let Ok(args) = signal.args() {
                        ids.lock().unwrap().closed(args.id, args.reason);
                    }
                }
                else => return,
            }
        }
    });
    Ok(())
}

/// Escapes `&`, `<` and `>` for servers that read the body as markup.
pub(crate) fn escape_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

#[async_trait]
impl Notifier for FdoNotifier {
    fn kind(&self) -> NotifierKind {
        NotifierKind::Freedesktop
    }

    fn is_closed(&self) -> bool {
        self.proxy.inner().connection().is_closed()
    }

    async fn show(&self, toast: &Toast) -> Result<(), String> {
        let replaces = self.ids.lock().unwrap().replaces(&toast.id);
        let encoded: Vec<(String, &str)> = toast
            .buttons
            .iter()
            .map(|(label, action)| (action.encode(), label.as_str()))
            .collect();
        let mut actions: Vec<&str> = Vec::new();
        if self.capabilities.actions {
            actions.extend(["default", "Open"]);
            for (key, label) in &encoded {
                actions.extend([key.as_str(), label]);
            }
        }
        let mut hints: HashMap<&str, Value<'_>> = HashMap::new();
        hints.insert("desktop-entry", Value::from(self.entry.id.as_str()));
        // Normal: critical notifications get past Do Not Disturb.
        hints.insert("urgency", Value::U8(1));
        if toast.kind == ToastKind::Mail {
            hints.insert("category", Value::from("email.arrived"));
        }
        match (toast.sound, toast.kind) {
            (true, ToastKind::Mail) => {
                hints.insert("sound-name", Value::from("message-new-email"));
            }
            (true, ToastKind::Reminder) => {
                hints.insert("sound-name", Value::from("alarm-clock-elapsed"));
            }
            (true, ToastKind::App) => {}
            (false, _) => {
                hints.insert("suppress-sound", Value::Bool(true));
            }
        }
        let body = if self.capabilities.body_markup {
            escape_markup(&toast.body)
        } else {
            toast.body.clone()
        };
        // Called by hand rather than through the proxy, to learn which server
        // answered.
        let reply = self
            .proxy
            .inner()
            .call_method(
                "Notify",
                &(
                    "Cumulas Mail",
                    replaces,
                    self.entry.icon.as_str(),
                    toast.title.as_str(),
                    body.as_str(),
                    &actions,
                    hints,
                    -1i32,
                ),
            )
            .await
            .map_err(|e| e.to_string())?;
        let server_id: u32 = reply.body().deserialize().map_err(|e| e.to_string())?;
        let expired = {
            let mut ids = self.ids.lock().unwrap();
            // A server started since (by the bus, for this very notification,
            // say) numbers afresh. Its reply can come before the listener
            // hears of it, or after.
            ids.served_by(reply.header().sender().map(|s| s.as_str()));
            ids.shown(toast, server_id)
        };
        // Superseded: out of the desktop's history.
        if let Err(e) = self.close(&expired).await {
            tracing::debug!("could not close expired notifications: {e}");
        }
        Ok(())
    }

    async fn withdraw(&self, id: &str) -> Result<(), String> {
        let server_ids = self.ids.lock().unwrap().withdrawn(id);
        self.close(&server_ids).await
    }
}

impl FdoNotifier {
    /// Closes these notifications, all of them even when the server already
    /// dropped some (an expired one, on most desktops); reports the first
    /// error.
    async fn close(&self, server_ids: &[u32]) -> Result<(), String> {
        let mut result = Ok(());
        for &server_id in server_ids {
            if let Err(e) = self.proxy.close_notification(server_id).await
                && result.is_ok()
            {
                result = Err(e.to_string());
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::mpsc;
    use zbus::object_server::SignalEmitter;
    use zbus::zvariant::OwnedValue;

    use super::super::{Connect, Notifications};
    use super::*;

    fn toast(id: &str, default_action: Action) -> Toast {
        Toast {
            id: id.into(),
            title: "Ada Lovelace".into(),
            body: "Engines & <gears>".into(),
            kind: ToastKind::Mail,
            default_action,
            buttons: vec![
                ("Mark as read".into(), Action::MarkRead("m1".into())),
                ("Archive".into(), Action::Archive("m1".into())),
            ],
            sound: true,
        }
    }

    #[test]
    fn the_map_replaces_withdraws_and_decodes_clicks() {
        let mut ids = IdMap::default();
        let first = toast("mail:m1", Action::OpenMessage("m1".into()));
        assert_eq!(ids.replaces("mail:m1"), 0);
        ids.shown(&first, 7);
        assert_eq!(ids.replaces("mail:m1"), 7);

        // Clicks on the app's own notifications, with their tokens.
        ids.activation_token(7, "token-7".into());
        ids.activation_token(99, "someone else's".into());
        assert_eq!(
            ids.invoked(7, "default"),
            Some(Invoked {
                action: Action::OpenMessage("m1".into()),
                activation_token: Some("token-7".into()),
            })
        );
        assert_eq!(
            ids.invoked(7, "read:m1"),
            Some(Invoked {
                action: Action::MarkRead("m1".into()),
                activation_token: None,
            })
        );
        assert_eq!(ids.invoked(7, "made-up"), None);
        // Another app's notification.
        assert_eq!(ids.invoked(99, "default"), None);

        // A replacement under a new server id drops the old one.
        ids.shown(&first, 8);
        assert_eq!(ids.invoked(7, "default"), None);
        assert_eq!(ids.withdrawn("mail:m1"), [8]);
        assert!(ids.withdrawn("mail:m1").is_empty());
    }

    #[test]
    fn closed_notifications_are_not_replaced_but_still_clickable() {
        let mut ids = IdMap::default();
        ids.shown(&toast("mail:m1", Action::OpenMessage("m1".into())), 3);
        // Dismissed.
        ids.closed(3, 2);
        assert_eq!(ids.replaces("mail:m1"), 0);
        assert!(ids.withdrawn("mail:m1").is_empty());
        assert_eq!(
            ids.invoked(3, "default").map(|i| i.action),
            Some(Action::OpenMessage("m1".into()))
        );
    }

    #[test]
    fn expired_notifications_are_withdrawn_but_not_replaced() {
        let mut ids = IdMap::default();
        let first = toast("mail:m1", Action::OpenMessage("m1".into()));
        ids.shown(&first, 3);
        ids.closed(3, EXPIRED);
        assert_eq!(ids.replaces("mail:m1"), 0);
        assert_eq!(
            ids.invoked(3, "default").map(|i| i.action),
            Some(Action::OpenMessage("m1".into()))
        );
        assert_eq!(ids.withdrawn("mail:m1"), [3]);
        assert!(ids.withdrawn("mail:m1").is_empty());

        // Shown anew: the expired one is closed then, the new one later.
        ids.shown(&first, 4);
        ids.closed(4, EXPIRED);
        assert_eq!(ids.shown(&first, 5), [4]);
        assert_eq!(ids.replaces("mail:m1"), 5);
        assert_eq!(ids.withdrawn("mail:m1"), [5]);
    }

    #[test]
    fn a_new_server_s_ids_are_not_the_app_s() {
        let mut ids = IdMap::default();
        let first = toast("mail:m1", Action::OpenMessage("m1".into()));
        ids.served_by(Some(":1.5"));
        ids.shown(&first, 1);
        ids.shown(&toast("mail:m2", Action::OpenMessage("m2".into())), 2);
        ids.closed(2, EXPIRED);
        ids.served_by(Some(":1.5"));
        assert_eq!(ids.replaces("mail:m1"), 1);

        // The server went.
        ids.served_by(None);
        assert_eq!(ids.replaces("mail:m1"), 0);
        assert_eq!(ids.invoked(1, "default"), None);
        assert!(ids.withdrawn("mail:m1").is_empty());
        assert!(ids.withdrawn("mail:m2").is_empty());

        // A new one, which the app heard from before the bus said so.
        ids.served_by(Some(":1.9"));
        ids.shown(&first, 1);
        ids.served_by(Some(":1.9"));
        assert_eq!(ids.replaces("mail:m1"), 1);
    }

    #[test]
    fn the_map_forgets_the_oldest() {
        let mut ids = IdMap::default();
        for n in 0..(REMEMBERED as u32 + 10) {
            let id = format!("mail:{n}");
            ids.shown(&toast(&id, Action::OpenMessage(n.to_string())), n + 1);
        }
        assert_eq!(ids.by_server.len(), REMEMBERED);
        assert_eq!(ids.by_toast.len(), REMEMBERED);
        assert_eq!(ids.replaces("mail:0"), 0);
        assert_eq!(ids.replaces(&format!("mail:{}", REMEMBERED + 9)), 266);
    }

    #[test]
    fn markup_is_escaped() {
        assert_eq!(
            escape_markup("Q&A <b>now</b>"),
            "Q&amp;A &lt;b&gt;now&lt;/b&gt;"
        );
    }

    // --- against a fake notification server ----------------------------------

    #[derive(Debug, Clone, PartialEq)]
    struct Notified {
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    }

    #[derive(Default)]
    struct Recorded {
        notified: Vec<Notified>,
        closed: Vec<u32>,
    }

    struct FakeServer {
        recorded: Arc<Mutex<Recorded>>,
        capabilities: Vec<String>,
        next_id: u32,
    }

    #[zbus::interface(name = "org.freedesktop.Notifications")]
    impl FakeServer {
        #[allow(clippy::too_many_arguments)]
        fn notify(
            &mut self,
            app_name: String,
            replaces_id: u32,
            app_icon: String,
            summary: String,
            body: String,
            actions: Vec<String>,
            hints: HashMap<String, OwnedValue>,
            expire_timeout: i32,
        ) -> u32 {
            self.recorded.lock().unwrap().notified.push(Notified {
                app_name,
                replaces_id,
                app_icon,
                summary,
                body,
                actions,
                hints,
                expire_timeout,
            });
            if replaces_id != 0 {
                return replaces_id;
            }
            self.next_id += 1;
            self.next_id
        }

        fn close_notification(&mut self, id: u32) {
            self.recorded.lock().unwrap().closed.push(id);
        }

        fn get_capabilities(&self) -> Vec<String> {
            self.capabilities.clone()
        }

        fn get_server_information(&self) -> (String, String, String, String) {
            ("fake".into(), "cumulas".into(), "1".into(), "1.2".into())
        }

        #[zbus(signal)]
        async fn action_invoked(
            emitter: &SignalEmitter<'_>,
            id: u32,
            action_key: &str,
        ) -> zbus::Result<()>;

        #[zbus(signal)]
        async fn activation_token(
            emitter: &SignalEmitter<'_>,
            id: u32,
            activation_token: &str,
        ) -> zbus::Result<()>;

        #[zbus(signal)]
        async fn notification_closed(
            emitter: &SignalEmitter<'_>,
            id: u32,
            reason: u32,
        ) -> zbus::Result<()>;
    }

    struct Harness {
        server: zbus::Connection,
        recorded: Arc<Mutex<Recorded>>,
        notifier: FdoNotifier,
        clicks: mpsc::UnboundedReceiver<Invoked>,
    }

    const PATH: &str = "/org/freedesktop/Notifications";

    fn entry() -> DesktopEntry {
        DesktopEntry {
            id: "co.cumulas.Mail".into(),
            icon: "co.cumulas.Mail".into(),
        }
    }

    /// The notifier and a fake server, talking peer to peer.
    async fn harness(capabilities: &[&str]) -> Harness {
        let (ours, theirs) = tokio::net::UnixStream::pair().unwrap();
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let fake = FakeServer {
            recorded: Arc::clone(&recorded),
            capabilities: capabilities.iter().map(|c| (*c).to_owned()).collect(),
            next_id: 40,
        };
        let guid = zbus::Guid::generate();
        let server = zbus::connection::Builder::unix_stream(theirs)
            .server(guid)
            .unwrap()
            .p2p()
            .serve_at(PATH, fake)
            .unwrap()
            .build();
        let client = zbus::connection::Builder::unix_stream(ours).p2p().build();
        let (server, client) = tokio::join!(server, client);
        let (server, client) = (server.unwrap(), client.unwrap());
        let (tx, clicks) = mpsc::unbounded_channel();
        // The notifier's proxy keeps the client connection open.
        let notifier = FdoNotifier::with_connection(&client, tx, entry())
            .await
            .unwrap();
        Harness {
            server,
            recorded,
            notifier,
            clicks,
        }
    }

    impl Harness {
        fn emit(&self) -> SignalEmitter<'static> {
            SignalEmitter::new(&self.server, PATH).unwrap()
        }

        async fn next_click(&mut self) -> Invoked {
            tokio::time::timeout(Duration::from_secs(5), self.clicks.recv())
                .await
                .expect("no click within 5 s")
                .unwrap()
        }
    }

    fn hint_str(hints: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
        hints
            .get(key)
            .and_then(|v| String::try_from(v.try_clone().unwrap()).ok())
    }

    #[tokio::test]
    async fn notify_sends_actions_and_hints_and_replaces() {
        let h = harness(&["actions", "body", "body-markup"]).await;
        let first = toast("mail:m1", Action::OpenMessage("m1".into()));
        h.notifier.show(&first).await.unwrap();
        h.notifier.show(&first).await.unwrap();

        let recorded = h.recorded.lock().unwrap();
        let [one, two] = recorded.notified.as_slice() else {
            panic!("{:?}", recorded.notified);
        };
        assert_eq!(one.app_name, "Cumulas Mail");
        assert_eq!(one.app_icon, "co.cumulas.Mail");
        assert_eq!(one.summary, "Ada Lovelace");
        assert_eq!(one.body, "Engines &amp; &lt;gears&gt;");
        assert_eq!(one.expire_timeout, -1);
        assert_eq!(
            one.actions,
            [
                "default",
                "Open",
                "read:m1",
                "Mark as read",
                "archive:m1",
                "Archive"
            ]
        );
        assert_eq!(
            hint_str(&one.hints, "desktop-entry").as_deref(),
            Some("co.cumulas.Mail")
        );
        assert_eq!(
            hint_str(&one.hints, "category").as_deref(),
            Some("email.arrived")
        );
        assert_eq!(
            hint_str(&one.hints, "sound-name").as_deref(),
            Some("message-new-email")
        );
        assert_eq!(
            u8::try_from(one.hints["urgency"].try_clone().unwrap()).unwrap(),
            1
        );
        assert_eq!(one.replaces_id, 0);
        // Shown again: the same notification, replaced.
        assert_eq!(two.replaces_id, 41);
    }

    #[tokio::test]
    async fn quiet_toasts_and_servers_without_actions_or_markup() {
        let h = harness(&["body"]).await;
        let mut quiet = toast("mail:m1", Action::OpenMessage("m1".into()));
        quiet.sound = false;
        h.notifier.show(&quiet).await.unwrap();
        let recorded = h.recorded.lock().unwrap();
        let shown = &recorded.notified[0];
        assert!(shown.actions.is_empty());
        assert_eq!(shown.body, "Engines & <gears>");
        assert!(!shown.hints.contains_key("sound-name"));
        assert!(bool::try_from(shown.hints["suppress-sound"].try_clone().unwrap()).unwrap());
    }

    #[tokio::test]
    async fn clicks_reach_the_router_with_their_tokens() {
        let mut h = harness(&["actions"]).await;
        h.notifier
            .show(&toast("mail:m1", Action::OpenMessage("m1".into())))
            .await
            .unwrap();
        let emitter = h.emit();
        // Another app's notification, then ours with a token.
        FakeServer::action_invoked(&emitter, 7, "default")
            .await
            .unwrap();
        FakeServer::activation_token(&emitter, 41, "xdg-token")
            .await
            .unwrap();
        FakeServer::action_invoked(&emitter, 41, "default")
            .await
            .unwrap();
        assert_eq!(
            h.next_click().await,
            Invoked {
                action: Action::OpenMessage("m1".into()),
                activation_token: Some("xdg-token".into()),
            }
        );
        FakeServer::action_invoked(&emitter, 41, "archive:m1")
            .await
            .unwrap();
        assert_eq!(h.next_click().await.action, Action::Archive("m1".into()));
    }

    #[tokio::test]
    async fn reminders_carry_their_buttons_and_route_their_clicks() {
        let mut h = harness(&["actions", "body"]).await;
        let key = "event:5f0c-1:1792040700";
        let reminder = Toast {
            id: format!("reminder:{key}"),
            title: "Standup".into(),
            body: "Today at 10:00 AM · Room 3".into(),
            kind: ToastKind::Reminder,
            default_action: Action::OpenEvent("5f0c-1".into()),
            buttons: vec![
                (
                    "Snooze 5 min".into(),
                    Action::Snooze {
                        key: key.into(),
                        minutes: 5,
                    },
                ),
                ("Dismiss".into(), Action::Dismiss(key.into())),
                (
                    "Join".into(),
                    Action::Join("https://teams.example/l/meetup-join/1?a=b".into()),
                ),
            ],
            sound: true,
        };
        h.notifier.show(&reminder).await.unwrap();
        {
            let recorded = h.recorded.lock().unwrap();
            let shown = &recorded.notified[0];
            assert_eq!(
                shown.actions,
                [
                    "default",
                    "Open",
                    "snooze:5:event:5f0c-1:1792040700",
                    "Snooze 5 min",
                    "dismiss:event:5f0c-1:1792040700",
                    "Dismiss",
                    "join:https://teams.example/l/meetup-join/1?a=b",
                    "Join",
                ]
            );
            assert_eq!(
                hint_str(&shown.hints, "sound-name").as_deref(),
                Some("alarm-clock-elapsed")
            );
            assert!(!shown.hints.contains_key("category"));
            // Never critical: that would get past Do Not Disturb.
            assert_eq!(
                u8::try_from(shown.hints["urgency"].try_clone().unwrap()).unwrap(),
                1
            );
        }
        let emitter = h.emit();
        for (button, action) in [
            (
                "snooze:5:event:5f0c-1:1792040700",
                Action::Snooze {
                    key: key.into(),
                    minutes: 5,
                },
            ),
            (
                "dismiss:event:5f0c-1:1792040700",
                Action::Dismiss(key.into()),
            ),
            ("default", Action::OpenEvent("5f0c-1".into())),
        ] {
            FakeServer::action_invoked(&emitter, 41, button)
                .await
                .unwrap();
            assert_eq!(h.next_click().await.action, action);
        }
    }

    #[tokio::test]
    async fn withdrawing_closes_the_notification_once() {
        let h = harness(&["actions"]).await;
        h.notifier
            .show(&toast("mail:m1", Action::OpenMessage("m1".into())))
            .await
            .unwrap();
        h.notifier.withdraw("mail:m1").await.unwrap();
        h.notifier.withdraw("mail:m1").await.unwrap();
        h.notifier.withdraw("mail:unknown").await.unwrap();
        assert_eq!(h.recorded.lock().unwrap().closed, [41]);

        // Once the server closed it, a new one isn't a replacement.
        h.notifier
            .show(&toast("mail:m2", Action::OpenMessage("m2".into())))
            .await
            .unwrap();
        let emitter = h.emit();
        FakeServer::notification_closed(&emitter, 42, 2)
            .await
            .unwrap();
        // The close arrives asynchronously.
        for _ in 0..100 {
            if h.notifier.ids.lock().unwrap().replaces("mail:m2") == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        h.notifier
            .show(&toast("mail:m2", Action::OpenMessage("m2".into())))
            .await
            .unwrap();
        assert_eq!(h.recorded.lock().unwrap().notified[2].replaces_id, 0);
    }

    /// Waits for what a signal does, which happens asynchronously.
    async fn until(done: impl Fn() -> bool) {
        for _ in 0..200 {
            if done() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("not done within 2 s");
    }

    #[tokio::test]
    async fn expired_notifications_still_go_once_read() {
        let h = harness(&["actions"]).await;
        let summary = toast("mail-summary:acct", Action::OpenInbox("acct".into()));
        h.notifier
            .show(&toast("mail:m1", Action::OpenMessage("m1".into())))
            .await
            .unwrap();
        h.notifier.show(&summary).await.unwrap();
        // The popups timed out; KDE Plasma keeps them in its history.
        let emitter = h.emit();
        FakeServer::notification_closed(&emitter, 41, EXPIRED)
            .await
            .unwrap();
        FakeServer::notification_closed(&emitter, 42, EXPIRED)
            .await
            .unwrap();
        until(|| h.notifier.ids.lock().unwrap().replaces("mail-summary:acct") == 0).await;

        // The mail was read.
        h.notifier.withdraw("mail:m1").await.unwrap();
        assert_eq!(h.recorded.lock().unwrap().closed, [41]);
        // A new summary is a new notification, and the old one goes.
        h.notifier.show(&summary).await.unwrap();
        let recorded = h.recorded.lock().unwrap();
        assert_eq!(recorded.notified[2].replaces_id, 0);
        assert_eq!(recorded.closed, [41, 42]);
    }

    #[tokio::test]
    async fn a_lost_connection_shows() {
        let h = harness(&["actions"]).await;
        assert!(!h.notifier.is_closed());
        h.server.clone().close().await.unwrap();
        until(|| h.notifier.is_closed()).await;
    }

    // --- on a real bus -------------------------------------------------------

    /// A private `dbus-daemon`, for what only a real bus does: it drops a
    /// client that sends it a malformed message, and tells who owns a name.
    struct PrivateBus {
        daemon: std::process::Child,
        address: String,
    }

    impl PrivateBus {
        /// `None` (and the test passes untried) without `dbus-daemon`.
        fn start() -> Option<Self> {
            let daemon = std::process::Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--print-address"])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .inspect_err(|e| eprintln!("not tried on a real bus, no dbus-daemon: {e}"))
                .ok()?;
            let mut bus = Self {
                daemon,
                address: String::new(),
            };
            let stdout = bus.daemon.stdout.take()?;
            std::io::BufRead::read_line(&mut std::io::BufReader::new(stdout), &mut bus.address)
                .ok()?;
            bus.address = bus.address.trim().to_owned();
            Some(bus)
        }

        async fn connection(&self) -> zbus::Result<zbus::Connection> {
            zbus::connection::Builder::address(self.address.as_str())?
                .build()
                .await
        }

        /// A notification server on the bus, numbering from 41.
        async fn serve(&self) -> (zbus::Connection, Arc<Mutex<Recorded>>) {
            let recorded = Arc::new(Mutex::new(Recorded::default()));
            let fake = FakeServer {
                recorded: Arc::clone(&recorded),
                capabilities: vec!["actions".into(), "body".into()],
                next_id: 40,
            };
            let server = zbus::connection::Builder::address(self.address.as_str())
                .unwrap()
                .serve_at(PATH, fake)
                .unwrap()
                .name("org.freedesktop.Notifications")
                .unwrap()
                .build()
                .await
                .unwrap();
            (server, recorded)
        }

        /// How the app's notifications reach the bus.
        fn connect(&self) -> Connect {
            let address = self.address.clone();
            Box::new(move |clicks| {
                let address = address.clone();
                Box::pin(async move {
                    let connection = zbus::connection::Builder::address(address.as_str())
                        .map_err(|e| e.to_string())?
                        .build()
                        .await
                        .map_err(|e| e.to_string())?;
                    let notifier =
                        FdoNotifier::with_connection(&connection, clicks, entry()).await?;
                    Ok(Arc::new(notifier) as Arc<dyn Notifier>)
                })
            })
        }
    }

    impl Drop for PrivateBus {
        fn drop(&mut self) {
            let _ = self.daemon.kill();
            let _ = self.daemon.wait();
        }
    }

    async fn click(clicks: &mut mpsc::UnboundedReceiver<Invoked>) -> Action {
        tokio::time::timeout(Duration::from_secs(5), clicks.recv())
            .await
            .expect("no click within 5 s")
            .unwrap()
            .action
    }

    #[tokio::test]
    async fn on_a_real_bus_a_nul_in_mail_is_harmless() {
        let Some(bus) = PrivateBus::start() else {
            return;
        };
        let (server, recorded) = bus.serve().await;
        let (tx, mut clicks) = mpsc::unbounded_channel();
        let notifications = Notifications::with_connect(bus.connect(), tx);
        // What `Subject: =?UTF-8?B?SGkAdGhlcmU=?=` from `=?UTF-8?B?QWRhAA==?=`
        // decodes to.
        let mut nul = toast("mail:m1", Action::OpenMessage("m1".into()));
        nul.title = "Ada\0".into();
        nul.body = "Hi\0there".into();
        notifications.show(&nul, vec!["m1".into()]).await.unwrap();
        notifications
            .show(
                &toast("mail:m2", Action::OpenMessage("m2".into())),
                vec!["m2".into()],
            )
            .await
            .unwrap();
        // Still connected, and hearing clicks.
        let emitter = SignalEmitter::new(&server, PATH).unwrap();
        FakeServer::action_invoked(&emitter, 41, "default")
            .await
            .unwrap();
        assert_eq!(click(&mut clicks).await, Action::OpenMessage("m1".into()));
        let recorded = recorded.lock().unwrap();
        assert_eq!(recorded.notified.len(), 2);
        assert_eq!(recorded.notified[0].summary, "Ada");
        assert_eq!(recorded.notified[0].body, "Hithere");
    }

    #[tokio::test]
    async fn on_a_real_bus_a_dropped_app_connects_again() {
        let Some(bus) = PrivateBus::start() else {
            return;
        };
        let (server, recorded) = bus.serve().await;
        let (tx, mut clicks) = mpsc::unbounded_channel();
        let notifications = Notifications::with_connect(bus.connect(), tx);
        let first = notifications.notifier().await.unwrap();
        // What a NUL did before the text was cleaned: the bus drops the app.
        let mut nul = toast("mail:m1", Action::OpenMessage("m1".into()));
        nul.body = "Hi\0there".into();
        assert!(first.show(&nul).await.is_err());
        until(|| first.is_closed()).await;

        notifications
            .show(
                &toast("mail:m2", Action::OpenMessage("m2".into())),
                vec!["m2".into()],
            )
            .await
            .unwrap();
        assert_eq!(recorded.lock().unwrap().notified.len(), 1);
        let emitter = SignalEmitter::new(&server, PATH).unwrap();
        FakeServer::action_invoked(&emitter, 41, "default")
            .await
            .unwrap();
        assert_eq!(click(&mut clicks).await, Action::OpenMessage("m2".into()));
    }

    #[tokio::test]
    async fn on_a_real_bus_a_restarted_server_s_ids_are_its_own() {
        let Some(bus) = PrivateBus::start() else {
            return;
        };
        let (old, _) = bus.serve().await;
        let connection = bus.connection().await.unwrap();
        let (tx, mut clicks) = mpsc::unbounded_channel();
        let notifier = FdoNotifier::with_connection(&connection, tx, entry())
            .await
            .unwrap();
        let summary = toast("mail-summary:acct", Action::OpenInbox("acct".into()));
        notifier
            .show(&toast("mail:m1", Action::OpenMessage("m1".into())))
            .await
            .unwrap();
        notifier.show(&summary).await.unwrap();

        // The server restarts, numbering from 41 again.
        old.close().await.unwrap();
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let mut new = None;
        for _ in 0..200 {
            let fake = FakeServer {
                recorded: Arc::clone(&recorded),
                capabilities: vec!["actions".into()],
                next_id: 40,
            };
            let built = zbus::connection::Builder::address(bus.address.as_str())
                .unwrap()
                .serve_at(PATH, fake)
                .unwrap()
                .name("org.freedesktop.Notifications")
                .unwrap()
                .build()
                .await;
            if let Ok(server) = built {
                new = Some(server);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let new = new.expect("the name wasn't released");
        until(|| notifier.ids.lock().unwrap().replaces("mail:m1") == 0).await;

        // Its 41 is another app's.
        let emitter = SignalEmitter::new(&new, PATH).unwrap();
        FakeServer::action_invoked(&emitter, 41, "default")
            .await
            .unwrap();
        // Not a replacement for its 42, another app's too.
        notifier.show(&summary).await.unwrap();
        assert_eq!(recorded.lock().unwrap().notified[0].replaces_id, 0);
        FakeServer::action_invoked(&emitter, 41, "default")
            .await
            .unwrap();
        assert_eq!(click(&mut clicks).await, Action::OpenInbox("acct".into()));
    }
}
