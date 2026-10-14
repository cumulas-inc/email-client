//! Microsoft Graph provider (mail, calendar, contacts, Microsoft To Do).
//!
//! All requests go through [`client::GraphClient`], which adds the bearer
//! token, retries throttled requests, follows paging links and maps Graph
//! errors. Outlook items (messages, events, contacts) are always requested
//! with `Prefer: IdType="ImmutableId"` so their ids survive moves between
//! folders; local ids are derived from those.

mod calendar;
mod client;
mod contacts;
mod mail;
mod tasks;
mod types;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use serde::Deserialize;
use tokio::sync::OnceCell;

use crate::auth::TokenSource;
use client::GraphClient;

/// Microsoft Graph v1.0 endpoint of the public cloud.
pub const GRAPH_BASE_URL: &str = "https://graph.microsoft.com/v1.0";

/// Time zone used when the mailbox's own cannot be read.
const FALLBACK_TIME_ZONE: &str = "UTC";

/// One Microsoft account on Microsoft Graph; implements all four provider
/// traits.
pub struct GraphProvider {
    account_id: String,
    client: GraphClient,
    /// The mailbox's time zone (Windows name such as `W. Europe Standard
    /// Time`), read once. Dates without a time (all-day events, task due
    /// dates) are anchored to it so Outlook shows them on the intended day.
    time_zone: OnceCell<String>,
}

impl GraphProvider {
    /// Provider for `account_id` against the public Graph endpoint.
    pub fn new(account_id: impl Into<String>, tokens: Arc<dyn TokenSource>) -> Self {
        Self::with_base_url(account_id, tokens, GRAPH_BASE_URL)
    }

    /// For tests against a mock server.
    pub fn with_base_url(
        account_id: impl Into<String>,
        tokens: Arc<dyn TokenSource>,
        base_url: impl Into<String>,
    ) -> Self {
        Self {
            account_id: account_id.into(),
            client: GraphClient::new(base_url.into(), tokens),
            time_zone: OnceCell::new(),
        }
    }

    /// The mailbox time zone, or UTC when it cannot be read (the failure is
    /// not cached, so the next call tries again).
    async fn mailbox_time_zone(&self) -> String {
        #[derive(Deserialize)]
        struct Value {
            value: Option<String>,
        }
        let read = self
            .time_zone
            .get_or_try_init(|| async {
                let url = self.client.url("/me/mailboxSettings/timeZone");
                let tz: Value = self.client.get_json(&url, &[]).await?;
                Ok::<_, crate::error::Error>(
                    tz.value
                        .filter(|tz| !tz.trim().is_empty())
                        .unwrap_or_else(|| FALLBACK_TIME_ZONE.into()),
                )
            })
            .await;
        match read {
            Ok(tz) => tz.clone(),
            Err(e) => {
                tracing::warn!("could not read the mailbox time zone, using UTC: {e}");
                FALLBACK_TIME_ZONE.into()
            }
        }
    }
}
