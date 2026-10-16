//! Mail search: the messages on this device, at once, and each account's
//! whole mailbox on its server, a page at a time.
//!
//! What a server search finds is stored, as rows outside the sync window
//! (see `Store::upsert_search_results`), and returned as local ids; the UI
//! passes them back to the local search, so every result is a stored
//! message that opens and acts like any other.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use chrono::Utc;
use tokio_util::sync::CancellationToken;

use super::CoreService;
use crate::error::{Error, Result};
use crate::model::{DataKind, MessageSummary, ServerSearchResult, ServerSearchStatus};
use crate::search::SearchQuery;

/// Most messages local search returns of its own (server hits come on top).
pub const LOCAL_SEARCH_LIMIT: u32 = 200;
/// Most server hits a local search includes.
pub const MAX_INCLUDED_IDS: usize = 1000;
/// Messages per account and page of a server search.
pub const SERVER_SEARCH_PAGE: u32 = 50;
/// How long a server may take for one page.
pub const SERVER_SEARCH_TIMEOUT: Duration = Duration::from_secs(45);
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
    /// `include_ids` (server hits; the first 1000 are used). Newest first.
    pub fn search_messages(
        &self,
        text: &str,
        account_id: Option<&str>,
        include_ids: &[String],
    ) -> Result<Vec<MessageSummary>> {
        let query = SearchQuery::parse(text);
        let include = &include_ids[..include_ids.len().min(MAX_INCLUDED_IDS)];
        self.store
            .search_messages(&query, account_id, include, LOCAL_SEARCH_LIMIT)
    }

    /// Searches one account's whole mailbox on its server: the first page,
    /// or with `more` the next one. Stores what it finds (search-only rows)
    /// and returns their local ids. Starting a search with a new
    /// `search_id` cancels the previous search. A query too broad for the
    /// servers (see `SearchQuery::is_narrow_enough_for_servers`) is done at
    /// once with nothing, as an empty one is. Account failures are
    /// reported in the result; it rejects only for an unknown account
    /// (NotFound) or an empty or over-long `search_id` (InvalidInput, 100
    /// chars at most).
    pub async fn search_server(
        &self,
        search_id: &str,
        text: &str,
        account_id: &str,
        more: bool,
    ) -> Result<ServerSearchResult> {
        if search_id.is_empty() || search_id.chars().count() > MAX_SEARCH_ID_CHARS {
            return Err(Error::invalid_input("Not a valid search id."));
        }
        self.store.get_account(account_id)?;
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
        if !query.is_narrow_enough_for_servers() {
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
        // Changes made here while the server answers win over its answer.
        let page_mark = self.store.search_page_started();
        let searched = tokio::select! {
            // Dropping the search leaves an IMAP session mid-command
            // unhealthy, so it is replaced before its next use.
            _ = cancel.cancelled() => return Ok(answer(ServerSearchStatus::Cancelled, None)),
            searched = tokio::time::timeout(
                SERVER_SEARCH_TIMEOUT,
                providers.mail.search(&query, &folders, cursor.as_deref(), SERVER_SEARCH_PAGE),
            ) => searched,
        };
        let page = match searched {
            Ok(Ok(page)) => page,
            Ok(Err(e)) => {
                // The server's reason may repeat the query: kept out of logs.
                tracing::debug!(
                    "searching account {account_id} on the server failed ({:?})",
                    e.code
                );
                return Ok(failed(e));
            }
            // Also when it asked to wait (throttled) for longer than this.
            // The UI offers to try again.
            Err(_) => {
                return Ok(failed(Error::network(
                    "The server didn't answer in time; it may be busy.",
                )));
            }
        };
        if cancel.is_cancelled() {
            return Ok(answer(ServerSearchStatus::Cancelled, None));
        }
        let stored = match self.store.upsert_search_results(
            account_id,
            &page.messages,
            Utc::now(),
            &page_mark,
        ) {
            Ok(stored) => stored,
            Err(e) => return Ok(failed(e)),
        };
        let has_more = page.next.is_some();
        if let Some(active) = self
            .searches
            .lock()
            .unwrap()
            .active
            .as_mut()
            .filter(|a| a.id == search_id)
        {
            active.cursors.insert(account_id.to_owned(), page.next);
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
