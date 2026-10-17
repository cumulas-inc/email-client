//! Microsoft Graph provider (mail, calendar, contacts, Microsoft To Do, and
//! the change subscriptions push relies on).
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
mod subscriptions;
mod tasks;
mod types;
mod windows_zones;

#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use serde::Deserialize;
use tokio::sync::OnceCell;

use crate::auth::TokenSource;
use crate::error::Error;
use client::GraphClient;

/// Microsoft Graph v1.0 endpoint of the public cloud.
pub const GRAPH_BASE_URL: &str = "https://graph.microsoft.com/v1.0";

/// The computer's time zone as a Windows name (`W. Europe Standard Time`),
/// for when the mailbox's can't be read; `UTC` when it isn't known. `TZ`
/// wins when it names a zone (`Europe/Berlin`, `:Europe/Berlin`).
fn computer_time_zone() -> String {
    match tz_zone(std::env::var("TZ").ok().as_deref()) {
        Some(zone) => zone.to_owned(),
        None => windows_time_zone(iana_time_zone::get_timezone().ok().as_deref()),
    }
}

/// The Windows time zone `TZ` names, if it names one (rather than a POSIX
/// rule such as `CET-1CEST`).
fn tz_zone(tz: Option<&str>) -> Option<&'static str> {
    let tz = tz?;
    windows_zones::windows_zone(tz.strip_prefix(':').unwrap_or(tz).trim())
}

/// The Windows name of an IANA time zone, which Graph accepts everywhere;
/// `UTC` when it isn't known.
fn windows_time_zone(iana: Option<&str>) -> String {
    iana.and_then(windows_zones::windows_zone)
        .unwrap_or("UTC")
        .to_owned()
}

/// One Microsoft account on Microsoft Graph; implements all four provider
/// traits.
pub struct GraphProvider {
    account_id: String,
    client: GraphClient,
    /// The mailbox's time zone (Windows name such as `W. Europe Standard
    /// Time`), read once. Dates without a time (all-day events, task due
    /// dates) are anchored to it so Outlook shows them on the intended day.
    time_zone: OnceCell<String>,
    /// The ids of the mail folders last listed, which the store has: a
    /// moved message's `parentFolderId` that isn't one of them is looked up
    /// in case it names one of them another way.
    folder_ids: Mutex<HashSet<String>>,
    /// The computer's time zone (tests set another).
    computer_time_zone: fn() -> String,
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
            folder_ids: Mutex::default(),
            computer_time_zone,
        }
    }

    /// The mailbox time zone, or the computer's when it cannot be read.
    /// Without the permission to read it (HTTP 403) that holds for the
    /// provider's life, which ends once the permission is granted; other
    /// failures are not cached, so the next call tries again.
    async fn mailbox_time_zone(&self) -> String {
        #[derive(Deserialize)]
        struct Value {
            value: Option<String>,
        }
        let read = self
            .time_zone
            .get_or_try_init(|| async {
                let url = self.client.url("/me/mailboxSettings/timeZone");
                match self
                    .client
                    .get_json::<Value>(&url, &[])
                    .await
                    .map_err(Error::from)
                {
                    Ok(tz) => Ok(tz
                        .value
                        .filter(|tz| !tz.trim().is_empty())
                        .unwrap_or_else(self.computer_time_zone)),
                    Err(e) if e.http_status() == Some(403) => {
                        tracing::info!(
                            "no permission to read the mailbox time zone; using the computer's"
                        );
                        Ok((self.computer_time_zone)())
                    }
                    Err(e) => Err(e),
                }
            })
            .await;
        match read {
            Ok(tz) => tz.clone(),
            Err(e) => {
                tracing::warn!("could not read the mailbox time zone, using the computer's: {e}");
                (self.computer_time_zone)()
            }
        }
    }
}
