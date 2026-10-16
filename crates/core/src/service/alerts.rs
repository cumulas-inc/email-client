//! Which new messages raise a notification.

use chrono::{DateTime, Duration, Utc};

use crate::model::{Folder, FolderRole, MessageSummary, NotifyScope, Settings};

/// Older messages arriving now (a folder catching up, mail moved in from
/// elsewhere) are not news.
pub const MAX_AGE_HOURS: i64 = 48;
/// More messages than this in one sync give a single summary for the account.
pub const SUMMARY_THRESHOLD: usize = 3;

/// What new mail should show, newest message first.
#[derive(Debug, Clone, PartialEq)]
pub enum MailAlert {
    /// One notification per message.
    Each(Vec<MessageSummary>),
    /// One notification for the account that replaces its previous summary.
    Summary(Vec<MessageSummary>),
}

impl MailAlert {
    pub fn messages(&self) -> &[MessageSummary] {
        match self {
            MailAlert::Each(messages) | MailAlert::Summary(messages) => messages,
        }
    }
}

/// The alert for messages a sync of `folder` inserted, if any should be
/// raised. A message raises one only when all of these hold: notifications
/// are on and not paused, the account isn't muted, the folder is covered by
/// the notify scope, this isn't the folder's first sync, and the message is
/// unread, not a draft and received in the last [`MAX_AGE_HOURS`].
pub(crate) fn mail_alert(
    settings: &Settings,
    account_id: &str,
    folder: &Folder,
    first_sync: bool,
    inserted: &[MessageSummary],
    now: DateTime<Utc>,
) -> Option<MailAlert> {
    let in_scope = match settings.notify_scope {
        NotifyScope::Inbox => folder.role == FolderRole::Inbox,
        NotifyScope::AllFolders => !matches!(
            folder.role,
            FolderRole::Junk
                | FolderRole::Trash
                | FolderRole::Sent
                | FolderRole::Drafts
                | FolderRole::Outbox
        ),
    };
    let paused = settings
        .notifications_paused_until
        .is_some_and(|until| until > now);
    let muted = settings.muted_account_ids.iter().any(|id| id == account_id);
    if !settings.notifications_enabled || paused || muted || !in_scope || first_sync {
        return None;
    }
    let oldest = now - Duration::hours(MAX_AGE_HOURS);
    let mut fresh: Vec<MessageSummary> = inserted
        .iter()
        .filter(|m| !m.is_read && !m.is_draft && m.received_at >= oldest)
        .cloned()
        .collect();
    if fresh.is_empty() {
        return None;
    }
    fresh.sort_by_key(|m| std::cmp::Reverse(m.received_at));
    Some(if fresh.len() > SUMMARY_THRESHOLD {
        MailAlert::Summary(fresh)
    } else {
        MailAlert::Each(fresh)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EmailAddress, EntityKind, Importance, local_id};

    fn folder(role: FolderRole) -> Folder {
        Folder {
            id: local_id("a", EntityKind::Folder, role.as_str()),
            account_id: "a".into(),
            remote_id: role.as_str().into(),
            parent_id: None,
            name: role.as_str().into(),
            role,
            unread_count: 0,
            total_count: 0,
        }
    }

    fn message(subject: &str, minutes_ago: i64) -> MessageSummary {
        MessageSummary {
            id: local_id("a", EntityKind::Message, subject),
            account_id: "a".into(),
            folder_id: String::new(),
            remote_id: subject.into(),
            conversation_id: None,
            subject: subject.into(),
            from: Some(EmailAddress::new(Some("Ada"), "ada@example.com")),
            to: vec![],
            cc: vec![],
            preview: String::new(),
            received_at: now() - Duration::minutes(minutes_ago),
            is_read: false,
            is_flagged: false,
            is_draft: false,
            has_attachments: false,
            importance: Importance::Normal,
        }
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-15T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn subjects(alert: Option<MailAlert>) -> Vec<String> {
        alert
            .map(|a| a.messages().iter().map(|m| m.subject.clone()).collect())
            .unwrap_or_default()
    }

    fn alert(
        settings: &Settings,
        role: FolderRole,
        messages: &[MessageSummary],
    ) -> Option<MailAlert> {
        mail_alert(settings, "a", &folder(role), false, messages, now())
    }

    #[test]
    fn only_the_inbox_by_default() {
        let settings = Settings::default();
        let new = [message("Hello", 1)];
        assert_eq!(
            subjects(alert(&settings, FolderRole::Inbox, &new)),
            ["Hello"]
        );
        for role in [FolderRole::Archive, FolderRole::Other, FolderRole::Junk] {
            assert_eq!(alert(&settings, role, &new), None, "{role:?}");
        }
    }

    #[test]
    fn all_folders_leaves_out_junk_trash_sent_drafts_and_outbox() {
        let settings = Settings {
            notify_scope: NotifyScope::AllFolders,
            ..Settings::default()
        };
        let new = [message("Hello", 1)];
        for role in [FolderRole::Inbox, FolderRole::Archive, FolderRole::Other] {
            assert!(alert(&settings, role, &new).is_some(), "{role:?}");
        }
        for role in [
            FolderRole::Junk,
            FolderRole::Trash,
            FolderRole::Sent,
            FolderRole::Drafts,
            FolderRole::Outbox,
        ] {
            assert_eq!(alert(&settings, role, &new), None, "{role:?}");
        }
    }

    #[test]
    fn muted_accounts_paused_and_disabled_notifications_stay_quiet() {
        let new = [message("Hello", 1)];
        let muted = Settings {
            muted_account_ids: vec!["b".into(), "a".into()],
            ..Settings::default()
        };
        assert_eq!(alert(&muted, FolderRole::Inbox, &new), None);
        let other_muted = Settings {
            muted_account_ids: vec!["b".into()],
            ..Settings::default()
        };
        assert!(alert(&other_muted, FolderRole::Inbox, &new).is_some());

        let paused = Settings {
            notifications_paused_until: Some(now() + Duration::minutes(5)),
            ..Settings::default()
        };
        assert_eq!(alert(&paused, FolderRole::Inbox, &new), None);
        let pause_over = Settings {
            notifications_paused_until: Some(now() - Duration::minutes(5)),
            ..Settings::default()
        };
        assert!(alert(&pause_over, FolderRole::Inbox, &new).is_some());

        let off = Settings {
            notifications_enabled: false,
            ..Settings::default()
        };
        assert_eq!(alert(&off, FolderRole::Inbox, &new), None);
    }

    #[test]
    fn not_on_a_folders_first_sync() {
        let new = [message("Hello", 1)];
        let first = mail_alert(
            &Settings::default(),
            "a",
            &folder(FolderRole::Inbox),
            true,
            &new,
            now(),
        );
        assert_eq!(first, None);
    }

    #[test]
    fn only_unread_non_draft_mail_from_the_last_two_days() {
        let mut read = message("Read", 1);
        read.is_read = true;
        let mut draft = message("Draft", 1);
        draft.is_draft = true;
        let old = message("Old", MAX_AGE_HOURS * 60 + 1);
        let edge = message("Edge", MAX_AGE_HOURS * 60);
        let alerted = subjects(alert(
            &Settings::default(),
            FolderRole::Inbox,
            &[read, draft, old, edge, message("New", 2)],
        ));
        assert_eq!(alerted, ["New", "Edge"]);
        let mut read = message("Read", 1);
        read.is_read = true;
        assert_eq!(
            alert(&Settings::default(), FolderRole::Inbox, &[read]),
            None
        );
    }

    #[test]
    fn more_than_three_become_one_summary_newest_first() {
        let settings = Settings::default();
        let three: Vec<_> = (1..=3).map(|i| message(&format!("m{i}"), i)).collect();
        assert!(matches!(
            alert(&settings, FolderRole::Inbox, &three),
            Some(MailAlert::Each(m)) if m.len() == 3
        ));
        let four: Vec<_> = (1..=4).map(|i| message(&format!("m{i}"), 5 - i)).collect();
        let summary = alert(&settings, FolderRole::Inbox, &four);
        assert!(matches!(summary, Some(MailAlert::Summary(_))));
        assert_eq!(subjects(summary), ["m4", "m3", "m2", "m1"]);
    }
}
