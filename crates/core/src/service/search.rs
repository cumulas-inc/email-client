//! Mail search: the messages on this device, at once.

use super::CoreService;
use crate::error::Result;
use crate::model::MessageSummary;
use crate::search::SearchQuery;

/// Most messages local search returns of its own (server hits come on top).
pub const LOCAL_SEARCH_LIMIT: u32 = 200;
/// Most server hits a local search includes.
pub const MAX_INCLUDED_IDS: usize = 1000;

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
}
