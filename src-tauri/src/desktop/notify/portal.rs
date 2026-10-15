//! Notifications through the Notification portal, for the Flatpak: the
//! sandbox has no access to `org.freedesktop.Notifications`, and the portal
//! attributes notifications to the app by itself.
//!
//! Actions are named without the `app.` prefix, so that the portal sends
//! clicks to the running app as `ActionInvoked` signals rather than through
//! `org.freedesktop.Application`. Each carries its [`Action`] as its target.

use std::collections::HashMap;

use ashpd::desktop::notification::NotificationProxy;
use async_trait::async_trait;
use futures::StreamExt;
use zbus::zvariant::{OwnedValue, Value};

use super::{Action, Invoked, InvokedSender, Notifier, NotifierKind, Toast};

pub struct PortalNotifier {
    proxy: NotificationProxy,
}

impl PortalNotifier {
    pub async fn connect(clicks: InvokedSender) -> Result<Self, String> {
        let proxy = NotificationProxy::new()
            .await
            .map_err(|e| format!("no Notification portal: {e}"))?;
        // The signal through the proxy's D-Bus side, whose stream owns
        // what it needs.
        let mut invoked = proxy
            .receive_signal("ActionInvoked")
            .await
            .map_err(|e| e.to_string())?;
        tokio::spawn(async move {
            while let Some(signal) = invoked.next().await {
                let Ok((_id, name, parameters)) = signal
                    .body()
                    .deserialize::<(String, String, Vec<OwnedValue>)>()
                else {
                    continue;
                };
                let parameters: Vec<&Value<'_>> = parameters.iter().map(|v| &**v).collect();
                let Some(click) = decode_click(&parameters) else {
                    tracing::debug!("ignoring notification action {name:?}");
                    continue;
                };
                if clicks.send(click).is_err() {
                    return;
                }
            }
        });
        Ok(Self { proxy })
    }
}

/// The click an `ActionInvoked` signal's parameters describe: the action's
/// target, then (from version 2 of the portal) platform data with the
/// activation token.
fn decode_click(parameters: &[&Value<'_>]) -> Option<Invoked> {
    let action = Action::decode(&string(parameters.first()?)?)?;
    let activation_token = parameters.get(1).and_then(|data| token(data));
    Some(Invoked {
        action,
        activation_token,
    })
}

fn string(value: &Value<'_>) -> Option<String> {
    match value {
        Value::Str(s) => Some(s.to_string()),
        Value::Value(inner) => string(inner),
        _ => None,
    }
}

fn token(platform_data: &Value<'_>) -> Option<String> {
    match platform_data {
        Value::Dict(dict) => dict.iter().find_map(|(key, value)| {
            (string(key)? == "activation-token")
                .then(|| string(value))
                .flatten()
        }),
        Value::Value(inner) => token(inner),
        _ => None,
    }
}

/// The notification as the portal takes it: `a{sv}`.
fn notification(toast: &Toast, version: u32) -> HashMap<&str, Value<'_>> {
    let mut n: HashMap<&str, Value<'_>> = HashMap::new();
    n.insert("title", Value::from(toast.title.as_str()));
    n.insert("body", Value::from(toast.body.as_str()));
    // Never "urgent": that gets past Do Not Disturb.
    n.insert("priority", Value::from("normal"));
    n.insert("default-action", Value::from("open"));
    n.insert(
        "default-action-target",
        Value::from(toast.default_action.encode()),
    );
    if !toast.buttons.is_empty() {
        let buttons: Vec<HashMap<&str, Value<'_>>> = toast
            .buttons
            .iter()
            .map(|(label, action)| {
                HashMap::from([
                    ("label", Value::from(label.as_str())),
                    ("action", Value::from(button_action(action))),
                    ("target", Value::from(action.encode())),
                ])
            })
            .collect();
        n.insert("buttons", Value::from(buttons));
    }
    // Sounds are up to the desktop; the portal only learned to be told
    // "silent" in version 2.
    if !toast.sound && version >= 2 {
        n.insert("sound", Value::from("silent"));
    }
    n
}

fn button_action(action: &Action) -> &'static str {
    match action {
        Action::MarkRead(_) => "mark-read",
        Action::Archive(_) => "archive",
        _ => "open",
    }
}

#[async_trait]
impl Notifier for PortalNotifier {
    fn kind(&self) -> NotifierKind {
        NotifierKind::Portal
    }

    async fn show(&self, toast: &Toast) -> Result<(), String> {
        let n = notification(toast, self.proxy.version());
        self.proxy
            .call_method("AddNotification", &(toast.id.as_str(), n))
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn withdraw(&self, id: &str) -> Result<(), String> {
        self.proxy
            .remove_notification(id)
            .await
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use zbus::zvariant::{Dict, Signature};

    use super::*;
    use crate::desktop::notify::ToastKind;

    fn toast(sound: bool) -> Toast {
        Toast {
            id: "mail:m1".into(),
            title: "Ada".into(),
            body: "Engines".into(),
            kind: ToastKind::Mail,
            default_action: Action::OpenMessage("m1".into()),
            buttons: vec![
                ("Mark as read".into(), Action::MarkRead("m1".into())),
                ("Archive".into(), Action::Archive("m1".into())),
            ],
            sound,
        }
    }

    #[test]
    fn the_notification_carries_its_actions_as_targets() {
        let toast = toast(true);
        let n = notification(&toast, 2);
        assert_eq!(n["title"], Value::from("Ada"));
        assert_eq!(n["priority"], Value::from("normal"));
        assert_eq!(n["default-action"], Value::from("open"));
        assert_eq!(n["default-action-target"], Value::from("open:m1"));
        assert!(!n.contains_key("sound"));
        let Value::Array(buttons) = &n["buttons"] else {
            panic!("{:?}", n["buttons"]);
        };
        assert_eq!(buttons.len(), 2);
        let Value::Dict(archive) = &buttons[1] else {
            panic!();
        };
        assert_eq!(
            archive.get::<&str, Value<'_>>(&"target").unwrap(),
            Some(Value::from("archive:m1"))
        );
        // The whole thing goes over D-Bus as (sa{sv}).
        zbus::zvariant::to_bytes(
            zbus::zvariant::serialized::Context::new_dbus(zbus::zvariant::LE, 0),
            &(toast.id.as_str(), n),
        )
        .unwrap();
    }

    #[test]
    fn quiet_notifications_ask_for_silence_where_the_portal_can() {
        assert_eq!(
            notification(&toast(false), 2)["sound"],
            Value::from("silent")
        );
        assert!(!notification(&toast(false), 1).contains_key("sound"));
    }

    #[test]
    fn clicks_decode_their_target_and_token() {
        let mut data = Dict::new(&Signature::Str, &Signature::Variant);
        data.add("activation-token", Value::from("xdg-token"))
            .unwrap();
        let target = Value::from("read:m1");
        let platform = Value::Dict(data);
        assert_eq!(
            decode_click(&[&target, &platform]),
            Some(Invoked {
                action: Action::MarkRead("m1".into()),
                activation_token: Some("xdg-token".into()),
            })
        );
        // Version 1 sends the target alone.
        let open = Value::Value(Box::new(Value::from("open:m1")));
        assert_eq!(
            decode_click(&[&open]),
            Some(Invoked {
                action: Action::OpenMessage("m1".into()),
                activation_token: None,
            })
        );
        assert_eq!(decode_click(&[]), None);
        assert_eq!(decode_click(&[&Value::U32(3)]), None);
    }
}
