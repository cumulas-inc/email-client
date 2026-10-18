//! Searching every folder on the server (`UID SEARCH`).
//!
//! A search looks in each folder once, taking its newest matches (by UID),
//! and pages through all it found by date. Folders go to the server in
//! batches, pipelined: `EXAMINE` and `UID SEARCH` for every folder of a
//! batch in one round trip, then the dates of their matches in another,
//! where one folder at a time took three. Each batch holds the account's
//! session on its own, so syncs and opening messages go on between them.
//!
//! A page spends [`SEARCH_TIME`] at most looking: what it found by then is
//! the page, and the cursor keeps the folders it didn't get to, and the
//! older matches of folders with more than [`MAX_PER_FOLDER`], for the
//! pages after it. The cursor carries the matches found too, so later pages
//! ask the server for nothing but their summaries, and those folders.

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::time::Instant;

use super::connection::ImapSession;
use super::sync::{Target, fetch_summaries};
use super::{ImapProvider, uid_set};
use crate::error::{Error, ErrorCode, Result};
use crate::model::{Folder, MessageSummary};
use crate::providers::{SearchPage, ServerQuery};
use crate::search::imap::{ImapCriteria, Literals};

/// The newest matches taken from a folder at a time; older ones wait for a
/// later page.
const MAX_PER_FOLDER: usize = 200;
/// Matches held before folders not searched yet wait for a later page.
const MAX_HELD: usize = 2000;
/// Folders asked about in one round trip.
const FOLDERS_PER_TRIP: usize = 25;
/// How long a page may spend looking through folders, well within the 45
/// seconds a page may take, its summaries included.
const SEARCH_TIME: Duration = Duration::from_secs(20);

/// A message that matched: its folder, UIDVALIDITY, UID and the date it
/// arrived (`INTERNALDATE`, as Unix seconds) when the server said.
type Candidate = (String, u32, u32, Option<i64>);

/// A folder still to look in, and the UID its matches must be below when
/// its newest were taken already.
type Pending = (String, Option<u32>);

/// What the next page needs: the matches found and not shown yet, and the
/// folders still to look in, those never searched first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Cursor {
    v: u32,
    c: Vec<Candidate>,
    f: Vec<Pending>,
}

impl Cursor {
    fn new(folders: &[Folder]) -> Self {
        Self {
            v: 2,
            c: Vec::new(),
            f: folders
                .iter()
                .map(|f| (f.remote_id.clone(), None))
                .collect(),
        }
    }

    fn parse(text: &str) -> Result<Self> {
        serde_json::from_str::<Self>(text)
            .ok()
            .filter(|c| c.v == 2)
            .ok_or_else(|| Error::invalid_input("Not a search of this account."))
    }

    fn encode(&self) -> String {
        serde_json::to_string(self).expect("cursors always serialize")
    }
}

/// Newest first; those without a date last; then by UID, highest first.
/// Folders keep their order among equals (the sort is stable).
fn order(candidates: &mut [Candidate]) {
    candidates.sort_by(|a, b| {
        b.3.is_some()
            .cmp(&a.3.is_some())
            .then(b.3.cmp(&a.3))
            .then(b.2.cmp(&a.2))
    });
}

/// What one folder of a batch gave: its newest matches, and the UID older
/// ones are below, if there are more.
type Found = (Vec<Candidate>, Option<u32>);

/// A folder's UIDVALIDITY, its newest matching UIDs, and the UID older ones
/// are below, if there are more.
type Newest = (u32, Vec<u32>, Option<u32>);

/// Looks in each folder of `batch`: which messages match, the newest
/// [`MAX_PER_FOLDER`] of each, and when they arrived. Per folder, what it
/// found or why it couldn't; the outer error means the connection failed.
async fn search_batch(
    session: &mut ImapSession,
    criteria: &ImapCriteria,
    batch: &[Pending],
) -> Result<Vec<Result<Found>>> {
    let literals = if criteria.is_quoted()
        || session.has_capability("LITERAL+")
        || session.has_capability("LITERAL-")
    {
        Literals::NonSync
    } else {
        Literals::Sync
    };
    let matched: Vec<Result<(u32, Vec<u32>)>> = match literals {
        Literals::NonSync => {
            let jobs: Vec<(String, String)> = batch
                .iter()
                .map(|(mailbox, below)| {
                    let criteria = criteria.render_below(literals, *below).concat();
                    (mailbox.clone(), format!("UID SEARCH {criteria}"))
                })
                .collect();
            session
                .examine_each(&jobs)
                .await?
                .into_iter()
                .map(|run| run.map(|run| (run.info.uid_validity, run.uids)))
                .collect()
        }
        // A literal waits for the server's go-ahead: a folder at a time.
        Literals::Sync => {
            let mut matched = Vec::with_capacity(batch.len());
            for (mailbox, below) in batch {
                let searched: Result<(u32, Vec<u32>)> = async {
                    // EXAMINE, for the current UIDVALIDITY even when the
                    // folder is selected already.
                    let info = session.select(mailbox, false).await?;
                    let segments = criteria.render_below(literals, *below);
                    let uids = session.uid_search_segments(&segments).await?;
                    Ok((info.uid_validity, uids))
                }
                .await;
                match searched {
                    Err(e) if !matches!(e.code, ErrorCode::Provider | ErrorCode::Unsupported) => {
                        return Err(e);
                    }
                    searched => matched.push(searched),
                }
            }
            matched
        }
    };

    // The newest of each folder, and when they arrived: one more trip.
    let mut kept: Vec<Result<Newest>> = Vec::with_capacity(batch.len());
    for ((_, below), matched) in batch.iter().zip(matched) {
        kept.push(matched.map(|(uid_validity, mut uids)| {
            uids.sort_unstable_by(|a, b| b.cmp(a));
            uids.dedup();
            // `1:n` matches the highest UID too when that is lower than 1.
            if let Some(below) = below {
                uids.retain(|uid| uid < below);
            }
            let more_below = uids
                .get(MAX_PER_FOLDER)
                .and(uids.get(MAX_PER_FOLDER - 1))
                .copied()
                .filter(|&lowest| lowest > 1);
            uids.truncate(MAX_PER_FOLDER);
            (uid_validity, uids, more_below)
        }));
    }
    let jobs: Vec<(String, String)> = batch
        .iter()
        .zip(&kept)
        .filter_map(|((mailbox, _), kept)| match kept {
            Ok((_, uids, _)) if !uids.is_empty() => Some((
                mailbox.clone(),
                format!("UID FETCH {} (UID INTERNALDATE)", uid_set(uids)),
            )),
            _ => None,
        })
        .collect();
    let mut dated = session.examine_each(&jobs).await?.into_iter();
    Ok(batch
        .iter()
        .zip(kept)
        .map(|((mailbox, _), kept)| {
            let (uid_validity, uids, more_below) = kept?;
            if uids.is_empty() {
                return Ok((Vec::new(), None));
            }
            let dates: HashMap<u32, i64> = match dated.next() {
                Some(Ok(run)) if run.info.uid_validity == uid_validity => run
                    .fetched
                    .into_iter()
                    .filter_map(|m| Some((m.uid, m.internal_date?.timestamp())))
                    .collect(),
                // Renumbered meanwhile: these UIDs name other messages now.
                Some(Ok(_)) => return Ok((Vec::new(), None)),
                // Without dates, they come last.
                Some(Err(_)) | None => HashMap::new(),
            };
            let candidates = uids
                .into_iter()
                .map(|uid| {
                    let date = dates.get(&uid).copied();
                    (mailbox.clone(), uid_validity, uid, date)
                })
                .collect();
            Ok((candidates, more_below))
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
    /// See [`crate::providers::MailProvider::search`]. A search scoped to
    /// some folders looks in those only.
    pub(super) async fn search_mailbox(
        &self,
        search: &ServerQuery<'_>,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<SearchPage> {
        let Some(criteria) = ImapCriteria::new(search.query, search.received) else {
            return Ok(SearchPage::default());
        };
        let folders = search.folders;
        let mut cursor = match cursor {
            Some(text) => Cursor::parse(text)?,
            None => Cursor::new(search.scope.unwrap_or(folders)),
        };
        let limit = limit.max(1) as usize;
        self.look(&criteria, folders, &mut cursor, limit, SEARCH_TIME)
            .await?;
        order(&mut cursor.c);
        let page: Vec<Candidate> = cursor.c.drain(..limit.min(cursor.c.len())).collect();
        // The page's candidates by folder, in the order they come.
        let mut groups: Vec<(&str, u32, Vec<u32>)> = Vec::new();
        for (mailbox, uid_validity, uid, _) in &page {
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
        let next = (!cursor.c.is_empty() || !cursor.f.is_empty()).then(|| cursor.encode());
        Ok(SearchPage { messages, next })
    }

    /// Looks in the folders the cursor has left, in turn, adding what they
    /// match to its candidates: every folder never searched, unless
    /// [`MAX_HELD`] are held or `time` is up, and older matches of a folder
    /// only while the page needs them. A folder the server refuses to
    /// search is skipped; the first refusal is the error when every folder
    /// refused. A failure of the connection once some folders were
    /// searched ends the looking, keeping the rest for the next page.
    async fn look(
        &self,
        criteria: &ImapCriteria,
        folders: &[Folder],
        cursor: &mut Cursor,
        limit: usize,
        time: Duration,
    ) -> Result<()> {
        let started = Instant::now();
        let mut searched = false;
        let mut refused: Option<Error> = None;
        while started.elapsed() < time {
            let mut batch: Vec<Pending> = Vec::new();
            while batch.len() < FOLDERS_PER_TRIP {
                let Some((_, below)) = cursor.f.first() else {
                    break;
                };
                let wanted = match below {
                    None => cursor.c.len() < MAX_HELD,
                    Some(_) => cursor.c.len() < limit,
                };
                if !wanted {
                    break;
                }
                let pending = cursor.f.remove(0);
                // A folder removed since the first page is gone.
                if folders.iter().any(|f| f.remote_id == pending.0) {
                    batch.push(pending);
                }
            }
            if batch.is_empty() {
                break;
            }
            let results =
                match with_session!(self, |session| search_batch(session, criteria, &batch)) {
                    Ok(results) => results,
                    Err(e) if searched => {
                        tracing::warn!("searching the folders stopped: {e}");
                        cursor.f.splice(0..0, batch);
                        break;
                    }
                    Err(e) => return Err(e),
                };
            for (pending, result) in batch.into_iter().zip(results) {
                match result {
                    Ok((candidates, more_below)) => {
                        searched = true;
                        cursor.c.extend(candidates);
                        if let Some(below) = more_below {
                            cursor.f.push((pending.0, Some(below)));
                        }
                    }
                    Err(e) if e.code == ErrorCode::Provider => {
                        tracing::warn!("searching folder {} failed: {e}", pending.0);
                        refused.get_or_insert(e);
                    }
                    // The charset: the same for every folder.
                    Err(e) => return Err(e),
                }
            }
        }
        if let Some(e) = refused.filter(|_| !searched) {
            return Err(e);
        }
        Ok(())
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
    use crate::search::SearchQuery;
    use crate::search::dates::Received;

    /// A mailbox on a loopback server, with `LITERAL+`: folders (name,
    /// UIDVALIDITY, messages by UID with their arrival in Unix seconds),
    /// where every message matches any search but for the UIDs asked for.
    /// A folder's name, UIDVALIDITY and messages (UID, arrival).
    type FakeFolder = (String, u32, Vec<(u32, i64)>);

    #[derive(Default)]
    struct Mailbox {
        folders: Vec<FakeFolder>,
        /// Folders EXAMINE refuses.
        refused: Vec<String>,
        /// Connections drop at the EXAMINE after this many, from then on.
        cut_after: Option<usize>,
        /// How long the server takes to answer.
        delay: Duration,
    }

    /// What the server saw.
    #[derive(Default)]
    struct Seen {
        /// Times it answered what had come (the client's round trips).
        turns: usize,
        examines: usize,
        /// The folders examined, in turn.
        examined: Vec<String>,
        searches: Vec<String>,
    }

    /// The server's answer to one command line.
    fn answer(mailbox: &Mailbox, selected: &mut Option<usize>, line: &str) -> String {
        let (tag, command) = line.split_once(' ').unwrap();
        let name = command.split(' ').next().unwrap().to_ascii_uppercase();
        let folder = selected.map(|i| &mailbox.folders[i]);
        match name.as_str() {
            "CAPABILITY" => format!("* CAPABILITY IMAP4rev1 LITERAL+\r\n{tag} OK done"),
            "EXAMINE" => {
                let wanted = command[8..].trim_matches('"');
                *selected = mailbox.folders.iter().position(|f| f.0 == wanted);
                match *selected {
                    Some(i) if !mailbox.refused.iter().any(|r| r == wanted) => {
                        let (_, validity, messages) = &mailbox.folders[i];
                        format!(
                            "* {} EXISTS\r\n* OK [UIDVALIDITY {validity}] ok\r\n\
                             * OK [UIDNEXT 9999] ok\r\n{tag} OK [READ-ONLY] done",
                            messages.len()
                        )
                    }
                    _ => {
                        *selected = None;
                        format!("{tag} NO no such folder")
                    }
                }
            }
            "UID" if command.starts_with("UID SEARCH") => match folder {
                Some((_, _, messages)) => {
                    let below = command
                        .split_once("UID 1:")
                        .map(|(_, rest)| rest.split(' ').next().unwrap().parse::<u32>().unwrap());
                    let uids: Vec<String> = messages
                        .iter()
                        .map(|(uid, _)| *uid)
                        .filter(|uid| below.is_none_or(|n| *uid <= n))
                        .map(|uid| uid.to_string())
                        .collect();
                    format!("* SEARCH {}\r\n{tag} OK done", uids.join(" "))
                }
                None => format!("{tag} BAD no mailbox selected"),
            },
            "UID" => match folder {
                Some((_, _, messages)) => {
                    let set = command.split(' ').nth(2).unwrap();
                    let wanted = |uid: u32| {
                        set.split(',').any(|range| match range.split_once(':') {
                            Some((a, b)) => {
                                (a.parse().unwrap()..=b.parse().unwrap()).contains(&uid)
                            }
                            None => range.parse::<u32>().unwrap() == uid,
                        })
                    };
                    let mut out = String::new();
                    for (seq, (uid, at)) in messages.iter().enumerate() {
                        if wanted(*uid) {
                            let date = chrono::DateTime::from_timestamp(*at, 0).unwrap();
                            out.push_str(&format!(
                                "* {} FETCH (UID {uid} INTERNALDATE \"{}\")\r\n",
                                seq + 1,
                                date.format("%d-%b-%Y %H:%M:%S +0000")
                            ));
                        }
                    }
                    format!("{out}{tag} OK done")
                }
                None => format!("{tag} BAD no mailbox selected"),
            },
            "LOGOUT" => format!("* BYE\r\n{tag} OK bye"),
            _ => format!("{tag} OK done"),
        }
    }

    /// Serves `mailbox` on a loopback port. What arrives within 20 ms of
    /// what came before is one turn, answered at once.
    async fn serve(mailbox: Mailbox) -> (ImapProvider, Arc<Mutex<Seen>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Seen::default()));
        let mailbox = Arc::new(mailbox);
        let log = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let (mailbox, seen) = (mailbox.clone(), log.clone());
                tokio::spawn(async move {
                    let (read, mut write) = stream.into_split();
                    let mut read = BufReader::new(read);
                    write.write_all(b"* OK ready\r\n").await.unwrap();
                    let mut selected = None;
                    let mut partial = String::new();
                    loop {
                        let mut lines = Vec::new();
                        if read.read_line(&mut partial).await.unwrap_or(0) == 0 {
                            return;
                        }
                        lines.push(std::mem::take(&mut partial));
                        while let Ok(Ok(n)) = tokio::time::timeout(
                            Duration::from_millis(20),
                            read.read_line(&mut partial),
                        )
                        .await
                        {
                            if n == 0 {
                                break;
                            }
                            lines.push(std::mem::take(&mut partial));
                        }
                        tokio::time::sleep(mailbox.delay).await;
                        let mut reply = String::new();
                        {
                            let mut seen = seen.lock().unwrap();
                            seen.turns += 1;
                            for line in &lines {
                                let line = line.trim_end();
                                if let Some((_, name)) = line.split_once(" EXAMINE ") {
                                    seen.examined.push(name.trim_matches('"').to_owned());
                                    seen.examines += 1;
                                    if mailbox.cut_after.is_some_and(|n| seen.examines > n) {
                                        return;
                                    }
                                }
                                if line.contains(" UID SEARCH ") {
                                    seen.searches.push(line.to_owned());
                                }
                                reply.push_str(&answer(&mailbox, &mut selected, line));
                                reply.push_str("\r\n");
                            }
                        }
                        if write.write_all(reply.as_bytes()).await.is_err() {
                            return;
                        }
                    }
                });
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
        (ImapProvider::new("acct", credentials), seen)
    }

    /// `n` folders `F0`… of `messages` messages each, the newest the highest
    /// UID; UIDVALIDITY 100 + the folder's number.
    fn folders(n: u32, messages: u32) -> Vec<FakeFolder> {
        (0..n)
            .map(|i| {
                let messages = (1..=messages)
                    .map(|uid| (uid, 1_700_000_000 + i64::from(uid) * 60 + i64::from(i)))
                    .collect();
                (format!("F{i}"), 100 + i, messages)
            })
            .collect()
    }

    fn stored(mailbox: &Mailbox) -> Vec<Folder> {
        mailbox
            .folders
            .iter()
            .map(|(name, _, _)| Folder {
                id: format!("folder-{name}"),
                account_id: "acct".into(),
                remote_id: name.clone(),
                parent_id: None,
                name: name.clone(),
                role: crate::model::FolderRole::Other,
                unread_count: 0,
                total_count: 0,
            })
            .collect()
    }

    fn criteria() -> ImapCriteria {
        ImapCriteria::new(&SearchQuery::parse("zebracorn"), Received::default()).unwrap()
    }

    #[tokio::test]
    async fn folders_are_searched_a_batch_per_round_trip() {
        let mailbox = Mailbox {
            folders: folders(30, 2),
            ..Default::default()
        };
        let folders = stored(&mailbox);
        let (provider, seen) = serve(mailbox).await;
        let mut cursor = Cursor::new(&folders);
        provider
            .look(&criteria(), &folders, &mut cursor, 50, SEARCH_TIME)
            .await
            .unwrap();
        assert!(cursor.f.is_empty(), "{:?}", cursor.f);
        assert_eq!(cursor.c.len(), 60);
        // Each match with its folder's UIDVALIDITY and its arrival.
        for (mailbox, validity, uid, date) in &cursor.c {
            let i: u32 = mailbox[1..].parse().unwrap();
            assert_eq!(*validity, 100 + i);
            assert_eq!(
                *date,
                Some(1_700_000_000 + i64::from(*uid) * 60 + i64::from(i))
            );
        }
        // Logging in, then two trips for each batch of 25 folders: one
        // folder at a time took three trips each, 90 in all.
        let seen = seen.lock().unwrap();
        assert!(seen.turns <= 2 + 2 * 2, "{} turns", seen.turns);
        assert_eq!(seen.searches.len(), 30);
    }

    #[tokio::test]
    async fn a_page_out_of_time_leaves_the_rest_for_the_next() {
        let mailbox = Mailbox {
            folders: folders(60, 1),
            delay: Duration::from_millis(100),
            ..Default::default()
        };
        let folders = stored(&mailbox);
        let (provider, _) = serve(mailbox).await;
        let time = Duration::from_millis(150);
        let mut cursor = Cursor::new(&folders);
        provider
            .look(&criteria(), &folders, &mut cursor, 50, time)
            .await
            .unwrap();
        // The first batch, then time was up.
        assert_eq!(cursor.c.len(), FOLDERS_PER_TRIP);
        assert_eq!(cursor.f.len(), 60 - FOLDERS_PER_TRIP);
        assert!(cursor.f.iter().all(|(_, below)| below.is_none()));
        assert_eq!(cursor.f[0].0, format!("F{FOLDERS_PER_TRIP}"));
        // The next page goes on where it stopped.
        provider
            .look(&criteria(), &folders, &mut cursor, 50, time)
            .await
            .unwrap();
        assert_eq!(cursor.c.len(), 2 * FOLDERS_PER_TRIP);
        assert_eq!(cursor.f.len(), 60 - 2 * FOLDERS_PER_TRIP);
    }

    #[tokio::test]
    async fn older_matches_of_a_busy_folder_come_on_later_pages() {
        let mailbox = Mailbox {
            folders: folders(1, 450),
            ..Default::default()
        };
        let folders = stored(&mailbox);
        let (provider, seen) = serve(mailbox).await;
        let mut cursor = Cursor::new(&folders);
        let look = async |cursor: &mut Cursor| {
            provider
                .look(&criteria(), &folders, cursor, 50, SEARCH_TIME)
                .await
                .unwrap()
        };
        look(&mut cursor).await;
        let uids = |cursor: &Cursor| -> (u32, u32) {
            let uids: Vec<u32> = cursor.c.iter().map(|c| c.2).collect();
            (*uids.iter().max().unwrap(), *uids.iter().min().unwrap())
        };
        assert_eq!(cursor.c.len(), MAX_PER_FOLDER);
        assert_eq!(uids(&cursor), (450, 251));
        assert_eq!(cursor.f, [("F0".to_owned(), Some(251))]);
        // While the page has enough, the older ones wait.
        look(&mut cursor).await;
        assert_eq!(cursor.c.len(), MAX_PER_FOLDER);
        // Shown: the next 200, then the last 50.
        cursor.c.clear();
        look(&mut cursor).await;
        assert_eq!(uids(&cursor), (250, 51));
        assert_eq!(cursor.f, [("F0".to_owned(), Some(51))]);
        cursor.c.clear();
        look(&mut cursor).await;
        assert_eq!((cursor.c.len(), uids(&cursor)), (50, (50, 1)));
        assert!(cursor.f.is_empty());
        let searches = seen.lock().unwrap().searches.clone();
        assert!(
            searches[1].contains(" UID SEARCH UID 1:250 "),
            "{searches:?}"
        );
        assert!(
            searches[2].contains(" UID SEARCH UID 1:50 "),
            "{searches:?}"
        );
    }

    #[tokio::test]
    async fn a_refused_folder_is_skipped_and_a_cut_keeps_what_was_found() {
        let mailbox = Mailbox {
            folders: folders(60, 2),
            refused: vec!["F1".into()],
            // The first batch's two trips, then the connection drops, and
            // drops again when it is made anew.
            cut_after: Some(2 * FOLDERS_PER_TRIP - 1),
            ..Default::default()
        };
        let folders = stored(&mailbox);
        let (provider, _) = serve(mailbox).await;
        let mut cursor = Cursor::new(&folders);
        provider
            .look(&criteria(), &folders, &mut cursor, 50, SEARCH_TIME)
            .await
            .unwrap();
        assert_eq!(cursor.c.len(), 2 * (FOLDERS_PER_TRIP - 1));
        assert!(cursor.c.iter().all(|c| c.0 != "F1"));
        assert_eq!(cursor.f.len(), 60 - FOLDERS_PER_TRIP);
        assert_eq!(cursor.f[0].0, format!("F{FOLDERS_PER_TRIP}"));

        // Before anything was found, the failure is the answer.
        let mut cursor = Cursor::new(&folders);
        let error = provider
            .look(&criteria(), &folders, &mut cursor, 50, SEARCH_TIME)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Network);
    }

    #[tokio::test]
    async fn a_scoped_search_looks_in_its_folders_only() {
        let mailbox = Mailbox {
            folders: folders(4, 2),
            ..Default::default()
        };
        let folders = stored(&mailbox);
        let (provider, seen) = serve(mailbox).await;
        let query = SearchQuery::parse("zebracorn is:unread is:flagged received:2026-10-01");
        let received = Received {
            since: Some(chrono::DateTime::from_timestamp(1_790_827_200, 0).unwrap()),
            before: Some(chrono::DateTime::from_timestamp(1_790_913_600, 0).unwrap()),
        };
        let search = ServerQuery {
            query: &query,
            received,
            folders: &folders,
            scope: Some(&folders[1..3]),
        };
        let page = provider.search_mailbox(&search, None, 50).await.unwrap();
        // This server tells nothing but UIDs and dates.
        assert!(page.messages.is_empty());
        let seen = seen.lock().unwrap();
        let mut examined = seen.examined.clone();
        examined.sort();
        examined.dedup();
        assert_eq!(examined, ["F1", "F2"]);
        assert_eq!(seen.searches.len(), 2);
        // Flags and dates (1 October 2026 in UTC−4, widened) on the wire.
        for line in &seen.searches {
            assert!(
                line.ends_with(" FLAGGED UNSEEN SINCE 30-Sep-2026 BEFORE 3-Oct-2026 UNDELETED"),
                "{line}"
            );
        }
    }

    #[test]
    fn cursors_round_trip_and_reject_others() {
        let cursor = Cursor {
            v: 2,
            c: vec![
                ("INBOX".into(), 7, 42, Some(1_700_000_000)),
                ("Archive/\"2023\"".into(), 9, 3, None),
            ],
            f: vec![("Sent".into(), None), ("INBOX".into(), Some(12))],
        };
        let text = cursor.encode();
        assert_eq!(
            text,
            r#"{"v":2,"c":[["INBOX",7,42,1700000000],["Archive/\"2023\"",9,3,null]],"f":[["Sent",null],["INBOX",12]]}"#
        );
        assert_eq!(Cursor::parse(&text).unwrap(), cursor);
        for other in [
            "",
            "https://graph.microsoft.com/v1.0/me/messages?$skip=50",
            r#"{"v":1,"c":[],"p":0}"#,
            r#"{"v":2,"c":[],"f":[["INBOX"]]}"#,
            r#"{"v":2,"c":[["INBOX",1,2]],"f":[]}"#,
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
        let criteria =
            ImapCriteria::new(&SearchQuery::parse("subject:café"), Received::default()).unwrap();
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
        let criteria = ImapCriteria::new(&SearchQuery::parse("café"), Received::default()).unwrap();
        let error = session
            .uid_search_segments(&criteria.render(Literals::Sync))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Unsupported);
        assert!(session.is_healthy());
    }
}
