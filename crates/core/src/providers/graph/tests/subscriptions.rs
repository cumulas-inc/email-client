use chrono::{Duration, TimeZone, Utc};
use serde_json::json;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, ResponseTemplate};

use super::*;
use crate::error::ErrorCode;
use crate::providers::{ChangeSubscriptions, NewSubscription};

fn subscription(id: &str, resource: &str, url: &str) -> serde_json::Value {
    json!({
        "id": id,
        "resource": resource,
        "applicationId": "aaaaaaaa-0000-0000-0000-000000000001",
        "changeType": "created,updated,deleted",
        "clientState": null,
        "notificationUrl": url,
        "lifecycleNotificationUrl": url,
        "expirationDateTime": "2026-10-18T09:30:00.1234567Z",
        "creatorId": "33333333-0000-0000-0000-00000000000a",
        "includeResourceData": false,
        "latestSupportedTlsVersion": "v1_2",
    })
}

#[tokio::test]
async fn subscriptions_are_created_without_resource_data() {
    let (server, provider) = setup().await;
    let expires = Utc.with_ymd_and_hms(2026, 10, 18, 9, 30, 0).unwrap();
    Mock::given(method("POST"))
        .and(path("/subscriptions"))
        .and(authorized)
        .and(body_json(json!({
            "changeType": "created,updated,deleted",
            "notificationUrl": "https://sync.example.org/graph/v1/k1route",
            "lifecycleNotificationUrl": "https://sync.example.org/graph/v1/k1route",
            "resource": "me/mailFolders('inbox')/messages",
            "expirationDateTime": "2026-10-18T09:30:00.000Z",
            "clientState": "the-client-state",
            "includeResourceData": false,
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(subscription(
            "7f105c7d-2dc5-4530-97cd-4e7ae6534c07",
            "me/mailFolders('inbox')/messages",
            "https://sync.example.org/graph/v1/k1route",
        )))
        .expect(1)
        .mount(&server)
        .await;
    let created = provider
        .create_subscription(&NewSubscription {
            resource: "me/mailFolders('inbox')/messages".into(),
            change_type: "created,updated,deleted".into(),
            notification_url: "https://sync.example.org/graph/v1/k1route".into(),
            lifecycle_notification_url: "https://sync.example.org/graph/v1/k1route".into(),
            client_state: "the-client-state".into(),
            expires_at: expires,
        })
        .await
        .unwrap();
    assert_eq!(created.id, "7f105c7d-2dc5-4530-97cd-4e7ae6534c07");
    assert_eq!(created.resource, "me/mailFolders('inbox')/messages");
    assert_eq!(
        created.notification_url.as_deref(),
        Some("https://sync.example.org/graph/v1/k1route")
    );
    assert!((created.expires_at - expires).abs() < Duration::seconds(1));
}

#[tokio::test]
async fn renewing_patches_the_expiry() {
    let (server, provider) = setup().await;
    Mock::given(method("PATCH"))
        .and(path("/subscriptions/sub-1"))
        .and(body_json(
            json!({ "expirationDateTime": "2026-10-18T09:30:00.000Z" }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(subscription(
            "sub-1",
            "me/events",
            "https://sync.example.org/graph/v1/k1route",
        )))
        .expect(1)
        .mount(&server)
        .await;
    let renewed = provider
        .renew_subscription(
            "sub-1",
            Utc.with_ymd_and_hms(2026, 10, 18, 9, 30, 0).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(renewed.resource, "me/events");
}

#[tokio::test]
async fn listing_follows_every_page() {
    let (server, provider) = setup().await;
    let base = server.uri();
    Mock::given(method("GET"))
        .and(path("/subscriptions"))
        .respond_with(ok_json(json!({
            "value": [subscription("a", "me/events", "https://sync.example.org/x")],
            "@odata.nextLink": format!("{base}/subscriptions/page2"),
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/subscriptions/page2"))
        .respond_with(ok_json(json!({
            "value": [{ "id": "b", "resource": "me/messages" }],
        })))
        .mount(&server)
        .await;
    let listed = provider.list_subscriptions().await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, "a");
    // Missing fields don't fail the list: no URL, and long expired.
    assert_eq!(listed[1].notification_url, None);
    assert_eq!(listed[1].expires_at, chrono::DateTime::UNIX_EPOCH);
}

#[tokio::test]
async fn deleting_one_already_gone_succeeds() {
    let (server, provider) = setup().await;
    Mock::given(method("DELETE"))
        .and(path("/subscriptions/here"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/subscriptions/gone"))
        .respond_with(graph_error(404, "ResourceNotFound", "Not found"))
        .expect(1)
        .mount(&server)
        .await;
    provider.delete_subscription("here").await.unwrap();
    provider.delete_subscription("gone").await.unwrap();
}

#[tokio::test]
async fn refusals_keep_their_status() {
    let (server, provider) = setup().await;
    Mock::given(method("PATCH"))
        .and(path("/subscriptions/gone"))
        .respond_with(graph_error(404, "ResourceNotFound", "Not found"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/subscriptions"))
        .respond_with(graph_error(
            409,
            "ExtensionError",
            "Subscription already exists",
        ))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/subscriptions"))
        .respond_with(graph_error(
            403,
            "ExtensionError",
            "Operation: Create; Exception: [Status Code: Forbidden]",
        ))
        .mount(&server)
        .await;
    let gone = provider
        .renew_subscription("gone", Utc::now())
        .await
        .unwrap_err();
    assert_eq!(gone.code, ErrorCode::NotFound);
    let new = NewSubscription {
        resource: "me/events".into(),
        change_type: "created,updated,deleted".into(),
        notification_url: "https://sync.example.org/x".into(),
        lifecycle_notification_url: "https://sync.example.org/x".into(),
        client_state: "s".into(),
        expires_at: Utc::now(),
    };
    let conflict = provider.create_subscription(&new).await.unwrap_err();
    assert_eq!(conflict.http_status(), Some(409));
    let forbidden = provider.create_subscription(&new).await.unwrap_err();
    assert_eq!(forbidden.http_status(), Some(403));
    // Its Debug output keeps the clientState and URLs out of logs.
    let debug = format!("{new:?}");
    assert!(
        !debug.contains("sync.example.org") && !debug.contains("\"s\""),
        "{debug}"
    );
}
