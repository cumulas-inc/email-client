//! Change subscriptions (Graph webhooks), which push through the Cumulas push
//! service keeps: listed, created without resource data, renewed (which
//! also reauthorizes them) and deleted.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::json;

use super::GraphProvider;
use super::client::{Failure, segment};
use crate::error::Result;
use crate::providers::{ChangeSubscriptions, NewSubscription, Subscription};

/// Graph pauses a subscription once the token it was last authorized with
/// expires, so creating and renewing use one that outlives the next renewal
/// (every 30 minutes) by a margin.
const TOKEN_LIFE: Duration = Duration::from_secs(45 * 60);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphSubscription {
    id: String,
    #[serde(default)]
    resource: String,
    notification_url: Option<String>,
    /// Unreadable or missing counts as long expired, so it's renewed.
    #[serde(default)]
    expiration_date_time: Option<DateTime<Utc>>,
}

impl From<GraphSubscription> for Subscription {
    fn from(s: GraphSubscription) -> Self {
        Self {
            id: s.id,
            resource: s.resource,
            notification_url: s.notification_url,
            expires_at: s.expiration_date_time.unwrap_or(DateTime::UNIX_EPOCH),
        }
    }
}

fn timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[async_trait]
impl ChangeSubscriptions for GraphProvider {
    async fn list_subscriptions(&self) -> Result<Vec<Subscription>> {
        let url = self.client.url("/subscriptions");
        let listed: Vec<GraphSubscription> = self.client.get_all(&url, &[]).await?;
        Ok(listed.into_iter().map(Subscription::from).collect())
    }

    async fn create_subscription(&self, new: &NewSubscription) -> Result<Subscription> {
        self.client.token_lasting(TOKEN_LIFE).await?;
        let body = json!({
            "changeType": new.change_type,
            "notificationUrl": new.notification_url,
            "lifecycleNotificationUrl": new.lifecycle_notification_url,
            "resource": new.resource,
            "expirationDateTime": timestamp(new.expires_at),
            "clientState": new.client_state,
            "includeResourceData": false,
        });
        let url = self.client.url("/subscriptions");
        let created: GraphSubscription = self
            .client
            .send_json(Method::POST, &url, &body, &[])
            .await?;
        Ok(created.into())
    }

    async fn renew_subscription(
        &self,
        id: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<Subscription> {
        self.client.token_lasting(TOKEN_LIFE).await?;
        let url = self.client.url(&format!("/subscriptions/{}", segment(id)));
        let body = json!({ "expirationDateTime": timestamp(expires_at) });
        let renewed: GraphSubscription = self
            .client
            .send_json(Method::PATCH, &url, &body, &[])
            .await?;
        Ok(renewed.into())
    }

    async fn delete_subscription(&self, id: &str) -> Result<()> {
        let url = self.client.url(&format!("/subscriptions/{}", segment(id)));
        match self
            .client
            .send_no_content(Method::DELETE, &url, None, &[])
            .await
        {
            Ok(()) => Ok(()),
            Err(Failure::Graph {
                status: StatusCode::NOT_FOUND,
                ..
            }) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}
