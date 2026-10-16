//! Mailbox listing: hierarchy, display names, roles and counts.

use std::collections::{HashMap, HashSet};

use async_imap::imap_proto::{
    MailboxDatum, MailboxListData, NameAttribute, Response, StatusAttribute,
};

use super::connection::{ImapSession, quote};
use super::utf7;
use crate::error::Result;
use crate::model::{EntityKind, Folder, FolderRole, local_id};

/// STATUS commands sent per pipelined batch.
const STATUS_BATCH: usize = 50;

/// A mailbox as returned by `LIST`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ListedMailbox {
    /// Raw (modified UTF-7) name, used in commands and as the remote id.
    pub name: String,
    pub delimiter: Option<String>,
    /// Lower-cased attributes, e.g. `\noselect`, `\sent`.
    pub attributes: Vec<String>,
    /// `(MESSAGES, UNSEEN)` once known from LIST-STATUS or STATUS.
    pub counts: Option<(u32, u32)>,
}

impl ListedMailbox {
    fn from_list(data: &MailboxListData<'_>) -> Self {
        Self {
            name: data.name.to_string(),
            delimiter: data.delimiter.as_ref().map(|d| d.to_string()),
            attributes: data.name_attributes.iter().map(attribute_name).collect(),
            counts: None,
        }
    }

    fn has(&self, attribute: &str) -> bool {
        self.attributes.iter().any(|a| a == attribute)
    }

    /// Mailboxes that cannot hold messages, and Gmail's virtual views
    /// (All Mail, Starred, Important) whose messages all live elsewhere too.
    fn is_listed(&self) -> bool {
        ![
            "\\noselect",
            "\\nonexistent",
            "\\all",
            "\\flagged",
            "\\important",
        ]
        .iter()
        .any(|a| self.has(a))
    }

    /// Path segments of the raw name.
    fn segments(&self) -> Vec<&str> {
        match self.delimiter.as_deref().filter(|d| !d.is_empty()) {
            Some(delimiter) => self.name.split(delimiter).collect(),
            None => vec![self.name.as_str()],
        }
    }

    fn parent_name(&self) -> Option<&str> {
        let delimiter = self.delimiter.as_deref().filter(|d| !d.is_empty())?;
        self.name.rsplit_once(delimiter).map(|(parent, _)| parent)
    }
}

fn attribute_name(attribute: &NameAttribute<'_>) -> String {
    match attribute {
        NameAttribute::NoInferiors => "\\noinferiors".to_owned(),
        NameAttribute::NoSelect => "\\noselect".to_owned(),
        NameAttribute::Marked => "\\marked".to_owned(),
        NameAttribute::Unmarked => "\\unmarked".to_owned(),
        NameAttribute::All => "\\all".to_owned(),
        NameAttribute::Archive => "\\archive".to_owned(),
        NameAttribute::Drafts => "\\drafts".to_owned(),
        NameAttribute::Flagged => "\\flagged".to_owned(),
        NameAttribute::Junk => "\\junk".to_owned(),
        NameAttribute::Sent => "\\sent".to_owned(),
        NameAttribute::Trash => "\\trash".to_owned(),
        NameAttribute::Extension(name) => name.to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// Lists all folders of the account with their message counts.
pub(super) async fn list(session: &mut ImapSession, account_id: &str) -> Result<Vec<Folder>> {
    let mut mailboxes = list_mailboxes(session, true).await?;
    let missing: Vec<usize> = mailboxes
        .iter()
        .enumerate()
        .filter(|(_, m)| m.is_listed() && m.counts.is_none())
        .map(|(i, _)| i)
        .collect();
    for batch in missing.chunks(STATUS_BATCH) {
        let mut commands = Vec::with_capacity(batch.len());
        for &i in batch {
            commands.push(format!(
                "STATUS {} (MESSAGES UNSEEN)",
                quote(&mailboxes[i].name)?
            ));
        }
        let mut counts = HashMap::new();
        // A mailbox the server refuses to report on simply shows no counts.
        session
            .run(&commands, |response| collect_status(response, &mut counts))
            .await?;
        apply_counts(&mut mailboxes, &counts);
    }
    Ok(build_folders(account_id, &mailboxes))
}

/// `(MESSAGES, UNSEEN)` of one mailbox; `None` when the server won't say.
pub(super) async fn counts(session: &mut ImapSession, mailbox: &str) -> Result<Option<(u32, u32)>> {
    let mut counts = HashMap::new();
    let command = format!("STATUS {} (MESSAGES UNSEEN)", quote(mailbox)?);
    let mut results = session
        .run(&[command], |response| collect_status(response, &mut counts))
        .await?;
    if let Some(Err(e)) = results.pop() {
        tracing::debug!("the server refused STATUS: {e}");
        return Ok(None);
    }
    // Servers may spell INBOX differently from the request.
    if counts.len() == 1 {
        return Ok(counts.into_values().next());
    }
    Ok(counts.remove(mailbox))
}

/// The raw name of the folder sent mail should be saved to, if any.
pub(super) async fn sent_mailbox(session: &mut ImapSession) -> Result<Option<String>> {
    let mailboxes = list_mailboxes(session, false).await?;
    let roles = assign_roles(&mailboxes);
    Ok(mailboxes
        .iter()
        .zip(roles)
        .find(|(_, role)| *role == FolderRole::Sent)
        .map(|(m, _)| m.name.clone()))
}

/// Name for a new Sent folder: servers whose folders all live below INBOX
/// (Courier, Cyrus) refuse to create top-level ones.
pub(super) async fn new_sent_mailbox_name(session: &mut ImapSession) -> Result<String> {
    let mailboxes = list_mailboxes(session, false).await?;
    let inbox = mailboxes.iter().find(|m| m.name == "INBOX");
    let delimiter = inbox.and_then(|m| m.delimiter.clone()).unwrap_or_default();
    let others: Vec<_> = mailboxes.iter().filter(|m| m.name != "INBOX").collect();
    let prefix = format!("INBOX{delimiter}");
    let below_inbox = !delimiter.is_empty()
        && !others.is_empty()
        && others.iter().all(|m| m.name.starts_with(&prefix));
    Ok(if below_inbox {
        format!("{prefix}Sent")
    } else {
        "Sent".to_owned()
    })
}

/// `LIST "" "*"`, asking for special-use attributes and (when `counts` and
/// the server supports LIST-STATUS) message counts in the same round trip.
async fn list_mailboxes(session: &mut ImapSession, counts: bool) -> Result<Vec<ListedMailbox>> {
    let mut options = Vec::new();
    if session.has_capability("SPECIAL-USE") && session.has_capability("LIST-EXTENDED") {
        options.push("SPECIAL-USE");
    }
    if counts && session.has_capability("LIST-STATUS") {
        options.push("STATUS (MESSAGES UNSEEN)");
    }
    let mut command = "LIST \"\" \"*\"".to_owned();
    if !options.is_empty() {
        command.push_str(&format!(" RETURN ({})", options.join(" ")));
    }
    let mut mailboxes = Vec::new();
    let mut status = HashMap::new();
    session
        .execute(&command, |response| match response {
            Response::MailboxData(MailboxDatum::List(data)) => {
                mailboxes.push(ListedMailbox::from_list(data));
            }
            other => collect_status(other, &mut status),
        })
        .await?;
    apply_counts(&mut mailboxes, &status);
    Ok(mailboxes)
}

fn collect_status(response: &Response<'_>, counts: &mut HashMap<String, (u32, u32)>) {
    if let Response::MailboxData(MailboxDatum::Status { mailbox, status }) = response {
        let entry = counts.entry(mailbox.to_string()).or_default();
        for attribute in status {
            match attribute {
                StatusAttribute::Messages(n) => entry.0 = *n,
                StatusAttribute::Unseen(n) => entry.1 = *n,
                _ => {}
            }
        }
    }
}

fn apply_counts(mailboxes: &mut [ListedMailbox], counts: &HashMap<String, (u32, u32)>) {
    for mailbox in mailboxes {
        if let Some(&found) = counts.get(&mailbox.name) {
            mailbox.counts = Some(found);
        }
    }
}

/// Turns `LIST` results into folders: hidden mailboxes are dropped, names
/// decoded, parents linked and roles assigned.
pub(super) fn build_folders(account_id: &str, mailboxes: &[ListedMailbox]) -> Vec<Folder> {
    let roles = assign_roles(mailboxes);
    let listed: HashSet<&str> = mailboxes
        .iter()
        .filter(|m| m.is_listed())
        .map(|m| m.name.as_str())
        .collect();
    let mut folders: Vec<Folder> = mailboxes
        .iter()
        .zip(roles)
        .filter(|(m, _)| m.is_listed())
        .map(|(m, role)| {
            let name = if m.name == "INBOX" {
                "Inbox".to_owned()
            } else {
                utf7::decode(m.segments().last().copied().unwrap_or(&m.name))
            };
            Folder {
                id: local_id(account_id, EntityKind::Folder, &m.name),
                account_id: account_id.to_owned(),
                remote_id: m.name.clone(),
                parent_id: m
                    .parent_name()
                    .filter(|parent| listed.contains(parent))
                    .map(|parent| local_id(account_id, EntityKind::Folder, parent)),
                name,
                role,
                unread_count: m.counts.map_or(0, |(_, unseen)| unseen),
                total_count: m.counts.map_or(0, |(messages, _)| messages),
            }
        })
        .collect();
    folders.sort_by(|a, b| a.remote_id.cmp(&b.remote_id));
    folders
}

/// Assigns each well-known role to at most one mailbox: special-use
/// attributes (RFC 6154) first, then well-known names as a fallback.
fn assign_roles(mailboxes: &[ListedMailbox]) -> Vec<FolderRole> {
    let mut roles: Vec<FolderRole> = mailboxes
        .iter()
        .map(|m| {
            if m.name == "INBOX" {
                FolderRole::Inbox
            } else if !m.is_listed() {
                FolderRole::Other
            } else {
                special_use_role(m).unwrap_or(FolderRole::Other)
            }
        })
        .collect();
    let mut taken: HashSet<&'static str> = HashSet::new();
    for (i, role) in roles.iter_mut().enumerate() {
        if *role != FolderRole::Other && !taken.insert(role.as_str()) {
            tracing::debug!("ignoring duplicate {role:?} folder {}", mailboxes[i].name);
            *role = FolderRole::Other;
        }
    }

    let mut candidates: Vec<(usize, FolderRole, usize)> = mailboxes
        .iter()
        .enumerate()
        .filter(|(i, m)| roles[*i] == FolderRole::Other && m.is_listed())
        .filter_map(|(i, m)| name_role(m).map(|(role, rank)| (i, role, rank)))
        .collect();
    // Prefer the most common name when several mailboxes match a role.
    candidates.sort_by_key(|&(i, _, rank)| (rank, mailboxes[i].name.len()));
    for (i, role, _) in candidates {
        if taken.insert(role.as_str()) {
            roles[i] = role;
        }
    }
    roles
}

fn special_use_role(mailbox: &ListedMailbox) -> Option<FolderRole> {
    [
        ("\\sent", FolderRole::Sent),
        ("\\drafts", FolderRole::Drafts),
        ("\\trash", FolderRole::Trash),
        ("\\junk", FolderRole::Junk),
        ("\\archive", FolderRole::Archive),
    ]
    .into_iter()
    .find(|(attribute, _)| mailbox.has(attribute))
    .map(|(_, role)| role)
}

/// Common folder names (English first, then frequent localizations), most
/// common first; the index is used as rank between competing candidates.
const ROLE_NAMES: &[(FolderRole, &[&str])] = &[
    (
        FolderRole::Sent,
        &[
            "sent",
            "sent items",
            "sent messages",
            "sent mail",
            "gesendet",
            "gesendete elemente",
            "gesendete objekte",
            "envoyés",
            "éléments envoyés",
            "enviados",
            "posta inviata",
            "verzonden",
            "verzonden items",
        ],
    ),
    (
        FolderRole::Drafts,
        &[
            "drafts",
            "draft",
            "entwürfe",
            "brouillons",
            "borradores",
            "bozze",
            "concepten",
        ],
    ),
    (
        FolderRole::Trash,
        &[
            "trash",
            "deleted items",
            "deleted messages",
            "bin",
            "papierkorb",
            "gelöschte elemente",
            "gelöschte objekte",
            "corbeille",
            "éléments supprimés",
            "papelera",
            "elementos eliminados",
            "cestino",
            "prullenbak",
            "verwijderde items",
        ],
    ),
    (
        FolderRole::Junk,
        &[
            "junk",
            "spam",
            "junk e-mail",
            "junk email",
            "junk mail",
            "bulk mail",
            "junk-e-mail",
            "courrier indésirable",
            "indésirables",
            "correo no deseado",
            "posta indesiderata",
            "ongewenste e-mail",
        ],
    ),
    (FolderRole::Archive, &["archive", "archives", "archiv"]),
];

/// Role by name, only for top-level folders and folders directly below INBOX
/// or Gmail's `[Gmail]`, so a user's "Projects/Archive" keeps no role.
fn name_role(mailbox: &ListedMailbox) -> Option<(FolderRole, usize)> {
    let segments = mailbox.segments();
    let well_placed = match segments.as_slice() {
        [_] => true,
        [parent, _] => {
            parent.eq_ignore_ascii_case("INBOX")
                || parent.eq_ignore_ascii_case("[Gmail]")
                || parent.eq_ignore_ascii_case("[Google Mail]")
        }
        _ => false,
    };
    if !well_placed {
        return None;
    }
    let name = utf7::decode(segments.last()?).to_lowercase();
    ROLE_NAMES.iter().find_map(|(role, names)| {
        names
            .iter()
            .position(|n| *n == name)
            .map(|rank| (*role, rank))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mailbox(name: &str, delimiter: &str, attributes: &[&str]) -> ListedMailbox {
        ListedMailbox {
            name: name.to_owned(),
            delimiter: Some(delimiter.to_owned()),
            attributes: attributes.iter().map(|a| a.to_lowercase()).collect(),
            counts: Some((3, 1)),
        }
    }

    fn by_name<'a>(folders: &'a [Folder], remote_id: &str) -> &'a Folder {
        folders.iter().find(|f| f.remote_id == remote_id).unwrap()
    }

    #[test]
    fn gmail_folders_use_special_use_and_hide_virtual_views() {
        let folders = build_folders(
            "acct",
            &[
                mailbox("INBOX", "/", &["\\HasNoChildren"]),
                mailbox("[Gmail]", "/", &["\\HasChildren", "\\Noselect"]),
                mailbox("[Gmail]/All Mail", "/", &["\\All"]),
                mailbox("[Gmail]/Drafts", "/", &["\\Drafts"]),
                mailbox("[Gmail]/Important", "/", &["\\Important"]),
                mailbox("[Gmail]/Sent Mail", "/", &["\\Sent"]),
                mailbox("[Gmail]/Spam", "/", &["\\Junk"]),
                mailbox("[Gmail]/Starred", "/", &["\\Flagged"]),
                mailbox("[Gmail]/Bin", "/", &["\\Trash"]),
                mailbox("Receipts", "/", &[]),
            ],
        );
        let names: Vec<_> = folders.iter().map(|f| f.remote_id.as_str()).collect();
        assert_eq!(
            names,
            [
                "INBOX",
                "Receipts",
                "[Gmail]/Bin",
                "[Gmail]/Drafts",
                "[Gmail]/Sent Mail",
                "[Gmail]/Spam"
            ]
        );
        assert_eq!(by_name(&folders, "INBOX").role, FolderRole::Inbox);
        assert_eq!(by_name(&folders, "INBOX").name, "Inbox");
        let sent = by_name(&folders, "[Gmail]/Sent Mail");
        assert_eq!(sent.role, FolderRole::Sent);
        assert_eq!(sent.name, "Sent Mail");
        // The \Noselect parent is not listed, so children are top level.
        assert_eq!(sent.parent_id, None);
        assert_eq!(by_name(&folders, "[Gmail]/Bin").role, FolderRole::Trash);
        assert_eq!(by_name(&folders, "[Gmail]/Spam").role, FolderRole::Junk);
        assert_eq!(by_name(&folders, "Receipts").role, FolderRole::Other);
        assert_eq!(sent.unread_count, 1);
        assert_eq!(sent.total_count, 3);
    }

    #[test]
    fn name_heuristics_hierarchy_and_utf7() {
        let folders = build_folders(
            "acct",
            &[
                mailbox("INBOX", ".", &[]),
                mailbox("INBOX.Sent Items", ".", &[]),
                mailbox("INBOX.Sent", ".", &[]),
                mailbox("Entw&APw-rfe", ".", &[]),
                mailbox("Junk E-mail", ".", &[]),
                mailbox("Deleted Items", ".", &[]),
                mailbox("Archive", ".", &[]),
                mailbox("Projects", ".", &[]),
                mailbox("Projects.Archive", ".", &[]),
                mailbox("Projects.Gel&APY-scht", ".", &[]),
            ],
        );
        let inbox = by_name(&folders, "INBOX");
        assert_eq!(by_name(&folders, "INBOX.Sent").role, FolderRole::Sent);
        assert_eq!(
            by_name(&folders, "INBOX.Sent").parent_id.as_ref(),
            Some(&inbox.id)
        );
        assert_eq!(
            by_name(&folders, "INBOX.Sent Items").role,
            FolderRole::Other
        );
        let drafts = by_name(&folders, "Entw&APw-rfe");
        assert_eq!(drafts.name, "Entwürfe");
        assert_eq!(drafts.role, FolderRole::Drafts);
        assert_eq!(by_name(&folders, "Junk E-mail").role, FolderRole::Junk);
        assert_eq!(by_name(&folders, "Deleted Items").role, FolderRole::Trash);
        assert_eq!(by_name(&folders, "Archive").role, FolderRole::Archive);
        let nested = by_name(&folders, "Projects.Archive");
        assert_eq!(nested.role, FolderRole::Other);
        assert_eq!(nested.name, "Archive");
        assert_eq!(
            nested.parent_id.as_ref(),
            Some(&by_name(&folders, "Projects").id)
        );
        assert_eq!(by_name(&folders, "Projects.Gel&APY-scht").name, "Gelöscht");
    }

    #[test]
    fn special_use_wins_over_names() {
        let folders = build_folders(
            "acct",
            &[
                mailbox("INBOX", "/", &[]),
                mailbox("Sent", "/", &[]),
                mailbox("Sent Messages", "/", &["\\Sent"]),
            ],
        );
        assert_eq!(by_name(&folders, "Sent Messages").role, FolderRole::Sent);
        assert_eq!(by_name(&folders, "Sent").role, FolderRole::Other);
    }
}
