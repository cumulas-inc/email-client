//! Where to search: `in:` names folders, `account:` accounts.
//!
//! Values are kept as scope keys, and names are keyed the same way before
//! they are compared, so the value the search filters write for a folder
//! (`in:"Project  X"`, `in:Q1\Q2`) always names that folder again. The UI's
//! port in `src/lib/search/query.ts` keys and matches alike.

use icu_normalizer::ComposingNormalizerBorrowed;

use crate::model::{Account, Folder, FolderRole};

/// NFC; quotes, backslashes and control characters removed; whitespace runs as
/// one space; trimmed; lowercased. What `in:` and `account:` values are kept as,
/// and what names are compared by. Curly apostrophes become `'`, as the
/// parser reads them, so that "Bob’s" names its folder.
pub fn scope_key(s: &str) -> String {
    ComposingNormalizerBorrowed::new_nfc()
        .normalize(s)
        .chars()
        .map(|c| match c {
            '\u{2019}' | '\u{2018}' => '\'',
            c => c,
        })
        .filter(|&c| {
            !matches!(
                c,
                '"' | '\\' | '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{201f}' | '\u{2033}'
            ) && (c.is_whitespace() || !c.is_control())
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The role an `in:` value names, as the well-known folders' names (and
/// their other names) say.
fn role(value: &str) -> Option<FolderRole> {
    Some(match value {
        "inbox" => FolderRole::Inbox,
        "drafts" | "draft" => FolderRole::Drafts,
        "sent" => FolderRole::Sent,
        "archive" => FolderRole::Archive,
        "junk" | "spam" => FolderRole::Junk,
        "trash" | "deleted" => FolderRole::Trash,
        "outbox" => FolderRole::Outbox,
        _ => return None,
    })
}

/// Whether `in:value` names `folder`: by its role (`in:inbox`, `in:spam`)
/// or by its name. Not its subfolders.
pub fn folder_matches(value: &str, folder: &Folder) -> bool {
    let value = scope_key(value);
    role(&value).is_some_and(|role| folder.role == role) || scope_key(&folder.name) == value
}

/// Whether `account:value` names `account`: by its address or its name.
pub fn account_matches(value: &str, account: &Account) -> bool {
    let value = scope_key(value);
    scope_key(&account.email) == value || scope_key(&account.display_name) == value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::SearchQuery;
    use crate::store::test_support::imap_account;

    fn folder(name: &str, role: FolderRole) -> Folder {
        Folder {
            id: format!("id-{name}"),
            account_id: "a".into(),
            remote_id: name.into(),
            parent_id: None,
            name: name.into(),
            role,
            unread_count: 0,
            total_count: 0,
        }
    }

    /// The shared scope-key vectors (`src/lib/search/query.test.ts` too).
    #[test]
    fn keys() {
        for (name, key) in [
            ("Project  X", "project x"),
            (" Clients", "clients"),
            ("Q1\\Q2 plans", "q1q2 plans"),
            ("Say \"Hi\"", "say hi"),
            ("Say \u{201c}Hi\u{201d}", "say hi"),
            ("ÜBERWEISUNGEN", "überweisungen"),
            ("U\u{308}", "ü"),
            ("a\tb\u{7}c\u{0}", "a bc"),
            ("„x‟ ″y″", "x y"),
            ("Bob\u{2019}s", "bob's"),
        ] {
            assert_eq!(scope_key(name), key, "{name:?}");
            assert_eq!(scope_key(key), key, "idempotent: {key:?}");
        }
    }

    #[test]
    fn folders_by_role_or_name() {
        let inbox = folder("Posteingang", FolderRole::Inbox);
        let junk = folder("Bulk Mail", FolderRole::Junk);
        let trash = folder("Deleted Items", FolderRole::Trash);
        let drafts = folder("Entwürfe", FolderRole::Drafts);
        let transfers = folder("Überweisungen", FolderRole::Other);
        assert!(folder_matches("inbox", &inbox));
        assert!(folder_matches("posteingang", &inbox));
        assert!(folder_matches("spam", &junk) && folder_matches("junk", &junk));
        assert!(folder_matches("bulk mail", &junk));
        assert!(folder_matches("trash", &trash) && folder_matches("deleted", &trash));
        assert!(folder_matches("deleted items", &trash));
        assert!(folder_matches("draft", &drafts) && folder_matches("drafts", &drafts));
        assert!(folder_matches("entwürfe", &drafts));
        assert!(folder_matches("überweisungen", &transfers));
        assert!(folder_matches("ÜBERWEISUNGEN", &transfers));
        assert!(!folder_matches("inbox", &junk));
        assert!(!folder_matches("transfers", &transfers));
        // A folder named like a role it doesn't have is found by its name.
        let named = folder("Archive", FolderRole::Other);
        assert!(folder_matches("archive", &named));
        assert!(!folder_matches("sent", &named));
        // Not the folders inside.
        let child = Folder {
            parent_id: Some(inbox.id.clone()),
            ..folder("Receipts", FolderRole::Other)
        };
        assert!(!folder_matches("inbox", &child));
        assert!(!folder_matches("posteingang", &child));
    }

    #[test]
    fn accounts_by_address_or_name() {
        let account = crate::model::Account {
            display_name: "Work  Mail".into(),
            email: "Megan@Contoso.com".into(),
            ..imap_account("a")
        };
        assert!(account_matches("megan@contoso.com", &account));
        assert!(account_matches("work mail", &account));
        assert!(account_matches("\"Work Mail\"", &account));
        assert!(!account_matches("work", &account));
        assert!(!account_matches("contoso.com", &account));
    }

    /// The value the search filters write for a folder (`formatValue` in
    /// `src/lib/search/edit.ts`): bare when it has no whitespace or quotes,
    /// else quoted, without quotes and backslashes.
    fn format_value(value: &str) -> String {
        let bare = !value.is_empty()
            && !value.chars().any(|c| {
                c.is_whitespace()
                    || matches!(
                        c,
                        '"' | '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{201f}' | '\u{2033}'
                    )
            });
        if bare {
            value.to_owned()
        } else {
            let inner: String = value
                .chars()
                .filter(|c| {
                    !matches!(
                        c,
                        '"' | '\\' | '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{201f}' | '\u{2033}'
                    )
                })
                .collect();
            format!("\"{inner}\"")
        }
    }

    #[test]
    fn what_the_filters_write_finds_the_folder_again() {
        for name in [
            "Project  X",
            " Clients",
            "Q1\\Q2 plans",
            "Say \"Hi\"",
            "Überweisungen",
            "Q1\\Q2",
            "Say \u{201c}Hi\u{201d}",
            "Bob\u{2019}s plans",
        ] {
            let folder = folder(name, FolderRole::Other);
            let query = SearchQuery::parse(&format!("in:{}", format_value(name)));
            assert_eq!(query.folders.len(), 1, "{name:?}: {query:?}");
            assert!(
                folder_matches(&query.folders[0], &folder),
                "{name:?}: {:?}",
                query.folders
            );
            assert!(query.terms.is_empty(), "{name:?}: {query:?}");
        }
    }
}
