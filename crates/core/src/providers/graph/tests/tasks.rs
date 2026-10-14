//! Microsoft To Do lists and tasks.

use chrono::{NaiveDate, TimeZone, Utc};
use serde_json::{Value, json};
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, ResponseTemplate};

use super::{ACCOUNT, fixture, ok_json, setup};
use crate::model::{
    EntityKind, Importance, Task, TaskDraft, TaskList, TaskPatch, TaskStatus, local_id,
};
use crate::providers::TasksProvider;

fn list() -> TaskList {
    TaskList {
        id: local_id(ACCOUNT, EntityKind::TaskList, "AAMkList-tasks"),
        account_id: ACCOUNT.into(),
        remote_id: "AAMkList-tasks".into(),
        name: "Tasks".into(),
        is_default: true,
    }
}

#[tokio::test]
async fn task_lists_mark_the_default_and_skip_flagged_email() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me/todo/lists"))
        .respond_with(ok_json(json!({
            "value": [
                { "id": "AAMkList-tasks", "displayName": "Tasks", "isOwner": true, "isShared": false, "wellknownListName": "defaultList" },
                { "id": "AAMkList-flagged", "displayName": "Flagged Emails", "isOwner": true, "isShared": false, "wellknownListName": "flaggedEmails" },
                { "id": "AAMkList-groceries", "displayName": "Groceries", "isOwner": true, "isShared": true, "wellknownListName": "none" }
            ]
        })))
        .mount(&server)
        .await;
    let lists = provider.list_task_lists().await.unwrap();
    assert_eq!(lists.len(), 2);
    assert_eq!(lists[0], list());
    assert_eq!(lists[1].name, "Groceries");
    assert!(!lists[1].is_default);
}

#[tokio::test]
async fn tasks_are_mapped() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me/todo/lists/AAMkList-tasks/tasks"))
        .respond_with(ok_json(fixture("todo_tasks.json", &server)))
        .mount(&server)
        .await;
    let tasks = provider.list_tasks(&list()).await.unwrap();
    assert_eq!(tasks.len(), 3);

    let contract = &tasks[0];
    assert_eq!(
        contract.id,
        local_id(ACCOUNT, EntityKind::Task, "AAMkTask-contract")
    );
    assert_eq!(contract.list_id, list().id);
    assert_eq!(contract.title, "Get the signed contract back");
    assert_eq!(contract.notes.as_deref(), Some("Ask Pat & legal."));
    assert_eq!(contract.status, TaskStatus::InProgress, "waitingOnOthers");
    assert_eq!(contract.importance, Importance::High);
    assert_eq!(contract.due_date, NaiveDate::from_ymd_opt(2026, 10, 20));
    assert_eq!(contract.completed_at, None);
    assert_eq!(
        contract.updated_at.date_naive(),
        NaiveDate::from_ymd_opt(2026, 10, 2).unwrap()
    );

    let flights = &tasks[1];
    assert_eq!(flights.status, TaskStatus::Completed);
    assert_eq!(flights.notes, None);
    assert_eq!(flights.due_date, NaiveDate::from_ymd_opt(2026, 9, 25));
    assert_eq!(
        flights.completed_at,
        Some(Utc.with_ymd_and_hms(2026, 9, 21, 0, 0, 0).unwrap())
    );
    assert_eq!(
        flights.created_at,
        Utc.with_ymd_and_hms(2026, 9, 20, 8, 0, 0).unwrap()
    );

    let garage = &tasks[2];
    assert_eq!(garage.status, TaskStatus::NotStarted, "deferred");
    assert_eq!(garage.importance, Importance::Low);
    assert_eq!(garage.due_date, None);
    assert_eq!(garage.notes.as_deref(), Some("Some day."));
}

fn graph_task(id: &str, body: &Value) -> Value {
    json!({
        "id": id,
        "title": body.get("title").cloned().unwrap_or("Existing".into()),
        "status": body.get("status").cloned().unwrap_or("notStarted".into()),
        "importance": "normal",
        "createdDateTime": "2026-10-14T08:00:00Z",
        "lastModifiedDateTime": "2026-10-14T09:00:00Z",
        "body": { "content": "", "contentType": "text" }
    })
}

async fn mount_time_zone(server: &wiremock::MockServer) {
    Mock::given(method("GET"))
        .and(path("/me/mailboxSettings/timeZone"))
        .respond_with(ok_json(json!({ "value": "Pacific Standard Time" })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn tasks_are_created_with_due_dates_in_the_mailbox_zone() {
    let (server, provider) = setup().await;
    mount_time_zone(&server).await;
    let expected = json!({
        "title": "Renew passport",
        "body": { "content": "Photos first", "contentType": "text" },
        "importance": "high",
        "dueDateTime": { "dateTime": "2026-11-02T00:00:00", "timeZone": "Pacific Standard Time" }
    });
    Mock::given(method("POST"))
        .and(path("/me/todo/lists/AAMkList-tasks/tasks"))
        .and(body_json(expected.clone()))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(graph_task("AAMkTask-new", &expected)),
        )
        .expect(1)
        .mount(&server)
        .await;
    let task = provider
        .create_task(
            &list(),
            &TaskDraft {
                list_id: list().id,
                title: "Renew passport".into(),
                notes: Some("Photos first".into()),
                importance: Some(Importance::High),
                due_date: NaiveDate::from_ymd_opt(2026, 11, 2),
            },
        )
        .await
        .unwrap();
    assert_eq!(task.remote_id, "AAMkTask-new");
    assert_eq!(task.title, "Renew passport");
}

fn current() -> Task {
    Task {
        id: local_id(ACCOUNT, EntityKind::Task, "AAMkTask-1"),
        account_id: ACCOUNT.into(),
        list_id: list().id,
        remote_id: "AAMkTask-1".into(),
        title: "Existing".into(),
        notes: Some("old".into()),
        status: TaskStatus::NotStarted,
        importance: Importance::Normal,
        due_date: NaiveDate::from_ymd_opt(2026, 10, 30),
        completed_at: None,
        created_at: Utc.with_ymd_and_hms(2026, 10, 14, 8, 0, 0).unwrap(),
        updated_at: Utc.with_ymd_and_hms(2026, 10, 14, 8, 0, 0).unwrap(),
    }
}

#[tokio::test]
async fn task_patches_send_only_the_changed_fields() {
    let (server, provider) = setup().await;
    mount_time_zone(&server).await;
    for expected in [
        json!({ "status": "completed" }),
        json!({ "dueDateTime": null, "body": { "content": "", "contentType": "text" } }),
        json!({
            "title": "Renamed",
            "importance": "low",
            "body": { "content": "new notes", "contentType": "text" },
            "dueDateTime": { "dateTime": "2026-12-01T00:00:00", "timeZone": "Pacific Standard Time" }
        }),
    ] {
        Mock::given(method("PATCH"))
            .and(path("/me/todo/lists/AAMkList-tasks/tasks/AAMkTask-1"))
            .and(body_json(expected.clone()))
            .respond_with(ok_json(graph_task("AAMkTask-1", &expected)))
            .expect(1)
            .mount(&server)
            .await;
    }

    let done = provider
        .update_task(
            &list(),
            &current(),
            &TaskPatch {
                status: Some(TaskStatus::Completed),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(done.status, TaskStatus::Completed);
    provider
        .update_task(
            &list(),
            &current(),
            &TaskPatch {
                clear_due_date: true,
                clear_notes: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    provider
        .update_task(
            &list(),
            &current(),
            &TaskPatch {
                title: Some("Renamed".into()),
                notes: Some("new notes".into()),
                importance: Some(Importance::Low),
                due_date: NaiveDate::from_ymd_opt(2026, 12, 1),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // An empty patch changes nothing and sends nothing.
    let unchanged = provider
        .update_task(&list(), &current(), &TaskPatch::default())
        .await
        .unwrap();
    assert_eq!(unchanged, current());
}

#[tokio::test]
async fn tasks_are_deleted() {
    let (server, provider) = setup().await;
    Mock::given(method("DELETE"))
        .and(path("/me/todo/lists/AAMkList-tasks/tasks/AAMkTask-1"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    provider.delete_task(&list(), "AAMkTask-1").await.unwrap();
}
