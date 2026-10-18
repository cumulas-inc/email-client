//! Mail search: the messages on this device, at once, and each account's
//! whole mailbox on its server, a page at a time.
//!
//! What a server search finds is stored, as rows outside the sync window
//! (see `Store::upsert_search_results`), and returned as local ids; the UI
//! passes them back to the local search, so every result is a stored
//! message that opens and acts like any other.
//!
//! Servers can't say every filter (Graph has no read or flagged with words,
//! nor several folders; IMAP dates are in each server's zones), so they are
//! asked for more, and what they find is checked against the query's
//! filters before it is stored.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

use chrono::Utc;
use tokio_util::sync::CancellationToken;

use super::CoreService;
use crate::error::{Error, Result};
use crate::model::{
    DataKind, Folder, SearchHit, SearchOptions, ServerSearchResult, ServerSearchStatus,
};
use crate::providers::ServerQuery;
use crate::search::{SearchQuery, dates, scope};
use crate::store::LocalSearch;

/// Most messages local search returns of its own (server hits come on top).
pub const LOCAL_SEARCH_LIMIT: u32 = 200;
/// Most server hits a local search includes.
pub const MAX_INCLUDED_IDS: usize = 1000;
/// Messages per account and page of a server search.
pub const SERVER_SEARCH_PAGE: u32 = 50;
/// How long a server may take for one call of a search (all its pages).
pub const SERVER_SEARCH_TIMEOUT: Duration = Duration::from_secs(45);
/// A call keeps fetching pages until this many messages pass the query's
/// filters (half a page)...
pub const SERVER_SEARCH_MIN_HITS: usize = 25;
/// ...the server has no more, or it fetched this many...
pub const SERVER_SEARCH_MAX_PAGES: u32 = 5;
/// ...or less time is left than another page may take: an IMAP page looks
/// through folders for up to 20 seconds, then fetches its summaries. A page
/// the timeout cut off would be lost, and its IMAP session with it.
pub const SERVER_SEARCH_PAGE_TIME: Duration = Duration::from_secs(30);
/// Search results not returned or opened for this long go at the next sync
/// of their account, whether or not it succeeds.
pub const SEARCH_RESULT_TTL_DAYS: i64 = 14;
/// Search results kept per account at most, the most recently seen.
pub const MAX_SEARCH_ROWS_PER_ACCOUNT: u32 = 2000;
/// Longest search id accepted.
const MAX_SEARCH_ID_CHARS: usize = 100;
/// Ids of searches that ended, remembered so that a call for one arriving
/// late can't replace the search that followed it.
const RETIRED_SEARCHES: usize = 32;

/// The server search in progress (one at a time, over every account).
#[derive(Default)]
pub(super) struct Registry {
    active: Option<Active>,
    retired: VecDeque<String>,
}

struct Active {
    id: String,
    cancel: CancellationToken,
    /// Per account: `Some(cursor)` while there are more pages; `None` when
    /// exhausted.
    cursors: HashMap<String, Option<String>>,
}

impl Registry {
    /// Makes `search_id` the current search, canceling the one before;
    /// `None` when it was current once and has ended.
    fn start(&mut self, search_id: &str) -> Option<&mut Active> {
        if self.retired.iter().any(|id| id == search_id) {
            return None;
        }
        if self.active.as_ref().is_some_and(|a| a.id != search_id) {
            self.retire();
        }
        Some(self.active.get_or_insert_with(|| Active {
            id: search_id.to_owned(),
            cancel: CancellationToken::new(),
            cursors: HashMap::new(),
        }))
    }

    fn retire(&mut self) {
        if let Some(active) = self.active.take() {
            active.cancel.cancel();
            self.remember(active.id);
        }
    }

    fn remember(&mut self, id: String) {
        if self.retired.len() == RETIRED_SEARCHES {
            self.retired.pop_front();
        }
        self.retired.push_back(id);
    }
}

impl CoreService {
    /// Local search: messages on this device matching `text`, plus
    /// `include_ids` (server hits; the first 1000 are used), each if it
    /// passes the query's filters. Best first (see `search/rank.rs`),
    /// unless `options.sort` is newest or the query has no words to rank
    /// by; dates are days in the computer's time zone, with weeks starting
    /// on `options.week_starts_on`. `account_id` keeps to one account.
    pub fn search_messages(
        &self,
        text: &str,
        account_id: Option<&str>,
        include_ids: &[String],
        options: SearchOptions,
    ) -> Result<Vec<SearchHit>> {
        let query = SearchQuery::parse(text);
        let now = Utc::now();
        let received = dates::resolve(&query.dates, now, &chrono::Local, options.week_starts_on);
        let accounts: Option<Vec<String>> = if query.accounts.is_empty() {
            account_id.map(|id| vec![id.to_owned()])
        } else {
            Some(
                self.store
                    .list_accounts()?
                    .into_iter()
                    .filter(|a| account_id.is_none_or(|id| a.id == id))
                    .filter(|a| query.accounts.iter().any(|v| scope::account_matches(v, a)))
                    .map(|a| a.id)
                    .collect(),
            )
        };
        let folders: Option<Vec<String>> = if query.folders.is_empty() {
            None
        } else {
            Some(
                self.store
                    .list_folders(None)?
                    .into_iter()
                    .filter(|f| {
                        accounts
                            .as_ref()
                            .is_none_or(|ids| ids.contains(&f.account_id))
                    })
                    .filter(|f| query.folders.iter().any(|v| scope::folder_matches(v, f)))
                    .map(|f| f.id)
                    .collect(),
            )
        };
        self.store.search(&LocalSearch {
            received,
            accounts: accounts.as_deref(),
            folders: folders.as_deref(),
            include_ids: &include_ids[..include_ids.len().min(MAX_INCLUDED_IDS)],
            limit: LOCAL_SEARCH_LIMIT,
            sort: options.sort,
            now,
            ..LocalSearch::new(&query)
        })
    }

    /// Searches one account's whole mailbox on its server (or the folders
    /// `in:` names there): the first pages, or with `more` the next ones.
    /// Stores what it finds that passes the query's filters (search-only
    /// rows) and returns their local ids. Since the server can't say every
    /// filter, a call reads pages until [`SERVER_SEARCH_MIN_HITS`] messages
    /// pass, the server has no more, or it read
    /// [`SERVER_SEARCH_MAX_PAGES`], all within [`SERVER_SEARCH_TIMEOUT`]:
    /// it starts no page with less than [`SERVER_SEARCH_PAGE_TIME`] left.
    /// Starting a search with a new `search_id` cancels the previous
    /// search. A query too broad for the servers (see
    /// `SearchQuery::is_narrow_enough_for_servers`), or for another
    /// account, or for no time or folder there, is done at once with
    /// nothing, as an empty one is. Account failures are reported in the
    /// result; it rejects only for an unknown account (NotFound) or an
    /// empty or over-long `search_id` (InvalidInput, 100 chars at most).
    pub async fn search_server(
        &self,
        search_id: &str,
        text: &str,
        account_id: &str,
        more: bool,
        options: SearchOptions,
    ) -> Result<ServerSearchResult> {
        if search_id.is_empty() || search_id.chars().count() > MAX_SEARCH_ID_CHARS {
            return Err(Error::invalid_input("Not a valid search id."));
        }
        let account = self.store.get_account(account_id)?;
        let answer = |status, error| ServerSearchResult {
            search_id: search_id.to_owned(),
            account_id: account_id.to_owned(),
            status,
            message_ids: Vec::new(),
            added: 0,
            has_more: false,
            error,
        };
        let query = SearchQuery::parse(text);
        let other_account = !query.accounts.is_empty()
            && !query
                .accounts
                .iter()
                .any(|v| scope::account_matches(v, &account));
        if !query.is_narrow_enough_for_servers() || other_account {
            return Ok(answer(ServerSearchStatus::Done, None));
        }
        let received = dates::resolve(
            &query.dates,
            Utc::now(),
            &chrono::Local,
            options.week_starts_on,
        );
        if received.is_impossible() {
            return Ok(answer(ServerSearchStatus::Done, None));
        }
        let (cancel, cursor) = {
            let mut searches = self.searches.lock().unwrap();
            let Some(active) = searches.start(search_id) else {
                return Ok(answer(ServerSearchStatus::Cancelled, None));
            };
            let cursor = if more {
                match active.cursors.get(account_id) {
                    Some(Some(cursor)) => Some(cursor.clone()),
                    _ => return Ok(answer(ServerSearchStatus::Done, None)),
                }
            } else {
                None
            };
            (active.cancel.clone(), cursor)
        };
        let failed = |e: Error| answer(ServerSearchStatus::Failed, Some(e));
        // Broken credentials are left to sync, which reports them.
        let providers = match self.providers_for(account_id).await {
            Ok(providers) => providers,
            Err(e) => return Ok(failed(e)),
        };
        let folders = match self.store.list_folders(Some(account_id)) {
            Ok(folders) => folders,
            Err(e) => return Ok(failed(e)),
        };
        let scoped: Vec<Folder> = folders
            .iter()
            .filter(|f| query.folders.iter().any(|v| scope::folder_matches(v, f)))
            .cloned()
            .collect();
        if !query.folders.is_empty() && scoped.is_empty() {
            return Ok(answer(ServerSearchStatus::Done, None));
        }
        let scope_ids: Option<HashSet<String>> =
            (!query.folders.is_empty()).then(|| scoped.iter().map(|f| f.id.clone()).collect());
        // Changes made here while the server answers win over its answer.
        let page_mark = self.store.search_page_started();
        let deadline = tokio::time::Instant::now() + SERVER_SEARCH_TIMEOUT;
        let server_query = ServerQuery {
            query: &query,
            // The dates exactly, as far as servers can say them; what they
            // find is checked against `received` itself.
            received: received.for_servers(),
            folders: &folders,
            scope: (!scoped.is_empty()).then_some(&scoped[..]),
            deadline,
        };

        let mut cursor = cursor;
        let mut admitted = Vec::new();
        let mut pages = 0;
        loop {
            let searched = tokio::select! {
                // Dropping the search leaves an IMAP session mid-command
                // unhealthy, so it is replaced before its next use.
                _ = cancel.cancelled() => return Ok(answer(ServerSearchStatus::Cancelled, None)),
                searched = tokio::time::timeout_at(
                    deadline,
                    providers.mail.search(&server_query, cursor.as_deref(), SERVER_SEARCH_PAGE),
                ) => searched,
            };
            if let Ok(Err(e)) = &searched {
                // The server's reason may repeat the query: kept out of logs.
                tracing::debug!(
                    "searching account {account_id} on the server failed ({:?})",
                    e.code
                );
            }
            let page = match searched {
                Ok(Ok(page)) => page,
                Ok(Err(e)) if pages == 0 => return Ok(failed(e)),
                // Also when it asked to wait (throttled) for longer than this.
                // The UI offers to try again.
                Err(_) if pages == 0 => {
                    return Ok(failed(Error::network(
                        "The server didn't answer in time; it may be busy.",
                    )));
                }
                // What the pages before found stands: "Show more results"
                // asks for this one again (and reports it if it fails then).
                Ok(Err(_)) | Err(_) => break,
            };
            pages += 1;
            admitted.extend(
                page.messages
                    .into_iter()
                    .filter(|m| query.admits(m, &received, scope_ids.as_ref())),
            );
            cursor = page.next;
            if admitted.len() >= SERVER_SEARCH_MIN_HITS
                || cursor.is_none()
                || pages == SERVER_SEARCH_MAX_PAGES
                // What was found comes now; "Show more results" goes on.
                || deadline.saturating_duration_since(tokio::time::Instant::now())
                    < SERVER_SEARCH_PAGE_TIME
            {
                break;
            }
        }
        if cancel.is_cancelled() {
            return Ok(answer(ServerSearchStatus::Cancelled, None));
        }
        let stored =
            match self
                .store
                .upsert_search_results(account_id, &admitted, Utc::now(), &page_mark)
            {
                Ok(stored) => stored,
                Err(e) => return Ok(failed(e)),
            };
        let has_more = cursor.is_some();
        if let Some(active) = self
            .searches
            .lock()
            .unwrap()
            .active
            .as_mut()
            .filter(|a| a.id == search_id)
        {
            active.cursors.insert(account_id.to_owned(), cursor);
        }
        if stored.added > 0 {
            self.notify(&[DataKind::Mail], Some(account_id));
        }
        Ok(ServerSearchResult {
            message_ids: stored.ids,
            added: stored.added,
            has_more,
            ..answer(ServerSearchStatus::Done, None)
        })
    }

    /// Cancels the search if it is the current one: its pending calls
    /// return `cancelled`, and so do later ones.
    pub fn cancel_server_search(&self, search_id: &str) {
        let mut searches = self.searches.lock().unwrap();
        if searches.active.as_ref().is_some_and(|a| a.id == search_id) {
            searches.retire();
        } else if !searches.retired.iter().any(|id| id == search_id) {
            // Canceled before it started.
            searches.remember(search_id.to_owned());
        }
    }

    /// Bounds the search results kept for an account (after each sync).
    pub(super) fn prune_search_results(&self, account_id: &str) {
        let older_than = Utc::now() - chrono::Duration::days(SEARCH_RESULT_TTL_DAYS);
        match self
            .store
            .prune_search_results(account_id, older_than, MAX_SEARCH_ROWS_PER_ACCOUNT)
        {
            Ok(0) => {}
            Ok(_) => self.notify(&[DataKind::Mail], Some(account_id)),
            Err(e) => tracing::warn!("pruning search results of {account_id} failed: {e}"),
        }
    }
}
