//! Searching every folder on the server (`UID SEARCH`).
//!
//! The first page searches each folder in turn and keeps the newest matches
//! of all (the candidates, by date); each page then fetches the summaries
//! of the next candidates. The cursor carries the candidates, so later
//! pages ask the server for nothing but those summaries. Each folder, and
//! each page's fetch from a folder, holds the account's session on its own,
//! so syncs and opening messages go on between them.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::connection::ImapSession;
use super::sync::{Target, fetch_summaries};
use super::{ImapProvider, uid_set};
use crate::error::{Error, ErrorCode, Result};
use crate::model::{Folder, MessageSummary};
use crate::providers::SearchPage;
use crate::search::SearchQuery;
use crate::search::imap::{ImapCriteria, Literals};

/// The newest matches kept of each folder.
const MAX_CANDIDATES_PER_FOLDER: usize = 200;
/// The newest matches kept of all.
const MAX_CANDIDATES: usize = 2000;

/// A message that matched: its folder, UIDVALIDITY, UID and the date it
/// arrived (`INTERNALDATE`, as Unix seconds) when the server said.
type Candidate = (String, u32, u32, Option<i64>);

/// What the next page needs: every candidate, and where the page starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Cursor {
    v: u32,
    c: Vec<Candidate>,
    p: usize,
}

impl Cursor {
    fn parse(text: &str) -> Result<Self> {
        serde_json::from_str::<Self>(text)
            .ok()
            .filter(|c| c.v == 1 && c.p <= c.c.len())
            .ok_or_else(|| Error::invalid_input("Not a search of this account."))
    }

    fn encode(&self) -> String {
        serde_json::to_string(self).expect("cursors always serialize")
    }
}

/// Newest first; those without a date last; then by UID, highest first.
/// Folders keep their order among equals (the sort is stable). At most
/// [`MAX_CANDIDATES`].
fn order(candidates: &mut Vec<Candidate>) {
    candidates.sort_by(|a, b| {
        b.3.is_some()
            .cmp(&a.3.is_some())
            .then(b.3.cmp(&a.3))
            .then(b.2.cmp(&a.2))
    });
    candidates.truncate(MAX_CANDIDATES);
}

/// The matches in one folder, the newest [`MAX_CANDIDATES_PER_FOLDER`].
async fn search_folder(
    session: &mut ImapSession,
    folder: &Folder,
    criteria: &ImapCriteria,
) -> Result<Vec<Candidate>> {
    // EXAMINE, for a current message count and UIDVALIDITY even when the
    // folder is selected already.
    let info = session.select(&folder.remote_id, false).await?;
    if info.exists == 0 {
        return Ok(Vec::new());
    }
    let literals = if session.has_capability("LITERAL+") || session.has_capability("LITERAL-") {
        Literals::NonSync
    } else {
        Literals::Sync
    };
    let mut uids = session
        .uid_search_segments(&criteria.render(literals))
        .await?;
    uids.sort_unstable_by(|a, b| b.cmp(a));
    uids.dedup();
    uids.truncate(MAX_CANDIDATES_PER_FOLDER);
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    let dates: HashMap<u32, i64> = session
        .uid_fetch(&uid_set(&uids), "(UID INTERNALDATE)")
        .await?
        .into_iter()
        .filter_map(|m| Some((m.uid, m.internal_date?.timestamp())))
        .collect();
    Ok(uids
        .into_iter()
        .map(|uid| {
            (
                folder.remote_id.clone(),
                info.uid_validity,
                uid,
                dates.get(&uid).copied(),
            )
        })
        .collect())
}

/// Summaries of `uids` in `folder`, unless its UIDs changed meaning.
async fn fetch_group(
    session: &mut ImapSession,
    folder: &Folder,
    uid_validity: u32,
    uids: &[u32],
) -> Result<Vec<MessageSummary>> {
    let info = session.open(&folder.remote_id, false).await?;
    if info.uid_validity != uid_validity {
        return Ok(Vec::new());
    }
    let target = Target {
        folder,
        uid_validity,
    };
    fetch_summaries(session, &target, uids).await
}

impl ImapProvider {
    /// See [`crate::providers::MailProvider::search`].
    pub(super) async fn search_mailbox(
        &self,
        query: &SearchQuery,
        folders: &[Folder],
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<SearchPage> {
        let Some(criteria) = ImapCriteria::new(query) else {
            return Ok(SearchPage::default());
        };
        let cursor = match cursor {
            Some(text) => Cursor::parse(text)?,
            None => Cursor {
                v: 1,
                c: self.candidates(&criteria, folders).await?,
                p: 0,
            },
        };
        let end = (cursor.p + limit.max(1) as usize).min(cursor.c.len());
        // The page's candidates by folder, in the order they come.
        let mut groups: Vec<(&str, u32, Vec<u32>)> = Vec::new();
        for (mailbox, uid_validity, uid, _) in &cursor.c[cursor.p..end] {
            match groups
                .iter_mut()
                .find(|(m, v, _)| m == mailbox && v == uid_validity)
            {
                Some((_, _, uids)) => uids.push(*uid),
                None => groups.push((mailbox, *uid_validity, vec![*uid])),
            }
        }
        let mut messages = Vec::new();
        for (mailbox, uid_validity, uids) in groups {
            // A folder removed since the first page.
            let Some(folder) = folders.iter().find(|f| f.remote_id == mailbox) else {
                continue;
            };
            let found = with_session!(self, |session| fetch_group(
                session,
                folder,
                uid_validity,
                &uids
            ))?;
            messages.extend(found);
        }
        if criteria.has_attachment() {
            messages.retain(|m| m.has_attachments);
        }
        messages.sort_by_key(|m| std::cmp::Reverse(m.received_at));
        let next = (end < cursor.c.len()).then(|| Cursor { p: end, ..cursor }.encode());
        Ok(SearchPage { messages, next })
    }

    /// Searches each folder, in the order given, and keeps the newest
    /// matches. A folder the server refuses to search is skipped; the first
    /// such refusal is the error when every folder refused.
    async fn candidates(
        &self,
        criteria: &ImapCriteria,
        folders: &[Folder],
    ) -> Result<Vec<Candidate>> {
        let mut candidates = Vec::new();
        let mut searched = false;
        let mut refused: Option<Error> = None;
        for folder in folders {
            match with_session!(self, |session| search_folder(session, folder, criteria)) {
                Ok(found) => {
                    searched = true;
                    candidates.extend(found);
                }
                Err(e) if e.code == ErrorCode::Provider => {
                    tracing::warn!("searching folder {} failed: {e}", folder.remote_id);
                    refused.get_or_insert(e);
                }
                // The connection, the sign-in or the charset: the same for
                // every folder.
                Err(e) => return Err(e),
            }
        }
        if let Some(e) = refused.filter(|_| !searched) {
            return Err(e);
        }
        order(&mut candidates);
        Ok(candidates)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    use super::*;
    use crate::model::{ImapSettings, TlsMode};
    use crate::providers::imap::ImapCredentials;

    #[test]
    fn cursors_round_trip_and_reject_others() {
        let cursor = Cursor {
            v: 1,
            c: vec![
                ("INBOX".into(), 7, 42, Some(1_700_000_000)),
                ("Archive/\"2023\"".into(), 9, 3, None),
            ],
            p: 1,
        };
        let text = cursor.encode();
        assert_eq!(
            text,
            r#"{"v":1,"c":[["INBOX",7,42,1700000000],["Archive/\"2023\"",9,3,null]],"p":1}"#
        );
        assert_eq!(Cursor::parse(&text).unwrap(), cursor);
        for other in [
            "",
            "https://graph.microsoft.com/v1.0/me/messages?$skip=50",
            r#"{"v":2,"c":[],"p":0}"#,
            r#"{"v":1,"c":[],"p":1}"#,
            r#"{"v":1,"c":[["INBOX",1,2]],"p":0}"#,
        ] {
            assert_eq!(
                Cursor::parse(other).unwrap_err().code,
                ErrorCode::InvalidInput,
                "{other}"
            );
        }
    }

    #[test]
    fn candidates_are_ordered_newest_first() {
        let mut candidates: Vec<Candidate> = vec![
            ("INBOX".into(), 1, 5, Some(100)),
            ("INBOX".into(), 1, 9, None),
            ("Archive".into(), 2, 50, Some(300)),
            ("INBOX".into(), 1, 7, Some(300)),
            ("Archive".into(), 2, 4, None),
            ("Sent".into(), 3, 7, Some(300)),
        ];
        order(&mut candidates);
        let ordered: Vec<(&str, u32)> = candidates
            .iter()
            .map(|(m, _, uid, _)| (m.as_str(), *uid))
            .collect();
        assert_eq!(
            ordered,
            [
                ("Archive", 50),
                ("INBOX", 7),
                ("Sent", 7),
                ("INBOX", 5),
                ("INBOX", 9),
                ("Archive", 4),
            ]
        );
        let mut many: Vec<Candidate> = (0..2500)
            .map(|uid| ("INBOX".into(), 1, uid, Some(i64::from(uid))))
            .collect();
        order(&mut many);
        assert_eq!(many.len(), MAX_CANDIDATES);
        assert_eq!(many[0].2, 2499);
    }

    /// A loopback server without `LITERAL+` that answers one search for
    /// "café": what it read, line by line.
    async fn literal_server(refuse_charset: bool) -> (ImapCredentials, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let lines = Arc::new(Mutex::new(Vec::new()));
        let log = lines.clone();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            stream.set_nodelay(true).unwrap();
            let (read, mut write) = stream.into_split();
            let mut read = BufReader::new(read);
            write.write_all(b"* OK ready\r\n").await.unwrap();
            let mut literal_for: Option<String> = None;
            loop {
                let mut line = String::new();
                if read.read_line(&mut line).await.unwrap_or(0) == 0 {
                    break;
                }
                let line = line.trim_end().to_owned();
                log.lock().unwrap().push(line.clone());
                let reply = if let Some(tag) = literal_for.take() {
                    format!("* SEARCH 4 9\r\n{tag} OK done")
                } else {
                    let (tag, command) = line.split_once(' ').unwrap();
                    let name = command.split(' ').next().unwrap().to_ascii_uppercase();
                    match name.as_str() {
                        "LOGIN" => format!("{tag} OK logged in"),
                        "CAPABILITY" => format!("* CAPABILITY IMAP4rev1\r\n{tag} OK done"),
                        "EXAMINE" => format!(
                            "* 3 EXISTS\r\n* OK [UIDVALIDITY 7] ok\r\n* OK [UIDNEXT 10] ok\r\n\
                             {tag} OK [READ-ONLY] done"
                        ),
                        "UID" if refuse_charset => {
                            format!("{tag} NO [BADCHARSET (US-ASCII)] unknown charset")
                        }
                        "UID" => {
                            literal_for = Some(tag.to_owned());
                            "+ go on".to_owned()
                        }
                        _ => format!("* BYE\r\n{tag} OK bye"),
                    }
                };
                write
                    .write_all(format!("{reply}\r\n").as_bytes())
                    .await
                    .unwrap();
            }
        });
        let credentials = ImapCredentials {
            settings: ImapSettings {
                imap_host: "127.0.0.1".into(),
                imap_port: port,
                imap_tls: TlsMode::None,
                smtp_host: "127.0.0.1".into(),
                smtp_port: 1,
                smtp_tls: TlsMode::None,
                username: "me".into(),
            },
            password: "secret".into(),
            display_name: "Me".into(),
            email: "me@example.com".into(),
        };
        (credentials, lines)
    }

    #[tokio::test]
    async fn synchronizing_literals_wait_for_the_server() {
        let (credentials, lines) = literal_server(false).await;
        let mut session = ImapSession::connect(&credentials).await.unwrap();
        assert!(!session.has_capability("LITERAL+"));
        session.select("INBOX", false).await.unwrap();
        let criteria = ImapCriteria::new(&SearchQuery::parse("subject:café")).unwrap();
        let uids = session
            .uid_search_segments(&criteria.render(Literals::Sync))
            .await
            .unwrap();
        assert_eq!(uids, [4, 9]);
        assert!(session.is_healthy());
        // The command up to the announced literal, then (after the
        // server's "+") the literal's 5 bytes and the rest.
        let lines = lines.lock().unwrap().clone();
        let [.., command, literal] = &lines[..] else {
            panic!("{lines:?}");
        };
        assert!(
            command.ends_with(" UID SEARCH CHARSET UTF-8 SUBJECT {5}"),
            "{command}"
        );
        assert_eq!(literal, "café UNDELETED");
    }

    #[tokio::test]
    async fn an_unknown_charset_is_unsupported_and_the_session_survives() {
        let (credentials, _) = literal_server(true).await;
        let mut session = ImapSession::connect(&credentials).await.unwrap();
        session.select("INBOX", false).await.unwrap();
        let criteria = ImapCriteria::new(&SearchQuery::parse("café")).unwrap();
        let error = session
            .uid_search_segments(&criteria.render(Literals::Sync))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Unsupported);
        assert!(session.is_healthy());
    }
}
