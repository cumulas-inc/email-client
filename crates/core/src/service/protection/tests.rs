//! The setup checks, the label list, and protected mail in the reading
//! pane and when sending, against a fake organization whose mail flow
//! rules and auto-labeling policies the test sets.

use std::sync::{Arc, Mutex};

use chrono::{Duration, Utc};
use serde_json::json;
use uuid::Uuid;

use super::*;
use crate::auth::microsoft::legacy_graph_scopes;
use crate::model::{
    Account, AccountKind, AccountSettings, AttachmentMeta, CatalogLabel, CheckItem, CheckNote,
    CheckState, DataKind, EmailAddress, EntityKind, LabelCatalog, LabelPolicy, MicrosoftSettings,
    OutgoingMessage, PermissionRequest, ProtectAction, ProtectionKind, ReplyKind, SyncStatus,
    local_id,
};
use crate::protection::label_copied;
use crate::providers::{MessageContent, MessageSignals};
use crate::secrets::MemorySecretStore;
use crate::service::tests::{FakeConnector, FakeLabels, FakeOrganization, FakeServer, summary};
use crate::service::{CoreService, EventSink, MailAlert};
use crate::signatures::CONSUMER_TENANT;
use crate::store::Store;

const TENANT: &str = "11111111-2222-3333-4444-555555555555";
const OTHER_TENANT: &str = "99999999-2222-3333-4444-555555555555";
const GENERAL: &str = "6c1b3b5e-0d4f-4b1e-9d3a-000000000001";
const CONFIDENTIAL: &str = "6c1b3b5e-0d4f-4b1e-9d3a-000000000002";
const SECRET: &str = "6c1b3b5e-0d4f-4b1e-9d3a-000000000003";
const EMAIL: &str = "adele@contoso.com";

const ENCRYPT: CheckItem = CheckItem::Action {
    action: ProtectAction::Encrypt,
};
const DO_NOT_FORWARD: CheckItem = CheckItem::Action {
    action: ProtectAction::DoNotForward,
};

fn label(id: &str) -> CheckItem {
    CheckItem::Label {
        label_id: id.into(),
    }
}

fn catalog_label(id: &str, name: &str, parent: Option<&str>, encrypts: bool) -> CatalogLabel {
    CatalogLabel {
        id: id.into(),
        name: name.into(),
        parent_name: parent.map(str::to_owned),
        tooltip: None,
        color: Some("#2b7aa8".into()),
        has_protection: encrypts,
        sensitivity: Some(1),
    }
}

fn catalog() -> LabelCatalog {
    LabelCatalog {
        labels: vec![
            catalog_label(GENERAL, "General", None, false),
            catalog_label(CONFIDENTIAL, "Confidential", None, false),
            catalog_label(SECRET, "Secret", Some("Highly Confidential"), true),
        ],
        policy: LabelPolicy {
            default_label_id: Some(GENERAL.into()),
            ..LabelPolicy::default()
        },
    }
}

/// Counts what the UI was told changed.
#[derive(Default)]
struct Sink {
    changes: Mutex<Vec<DataKind>>,
}

impl Sink {
    fn count(&self, kind: DataKind) -> usize {
        self.changes
            .lock()
            .unwrap()
            .iter()
            .filter(|k| **k == kind)
            .count()
    }
}

impl EventSink for Sink {
    fn data_changed(&self, kinds: &[DataKind], _: Option<&str>) {
        self.changes.lock().unwrap().extend_from_slice(kinds);
    }
    fn sync_status(&self, _: &SyncStatus) {}
    fn new_mail(&self, _: &Account, _: &MailAlert) {}
}

struct Setup {
    service: CoreService,
    connector: Arc<FakeConnector>,
    server: Arc<Mutex<FakeServer>>,
    labels: Arc<FakeLabels>,
    sink: Arc<Sink>,
    account: Account,
}

impl Setup {
    fn store(&self) -> &Arc<Store> {
        self.service.store()
    }

    fn organization(&self, organization: FakeOrganization) {
        self.server.lock().unwrap().organization = FakeOrganization {
            tenant: TENANT.into(),
            ..organization
        };
    }

    fn setup(&self) -> ProtectionSetup {
        self.service.protection_setup(&self.account.id).unwrap()
    }

    fn label_check(&self, id: &str) -> CheckResult {
        self.setup()
            .labels
            .into_iter()
            .find(|l| l.label.id == id)
            .unwrap()
            .check
    }

    fn run(&self) -> Option<CheckRun> {
        self.store()
            .protection_checks(&self.account.id)
            .unwrap()
            .run
    }

    /// Makes the waiting check `minutes` older.
    fn age_run(&self, minutes: i64) {
        self.store()
            .update_protection_checks(&self.account.id, |checks| {
                let run = checks.run.as_mut().unwrap();
                run.control_sent_at -= Duration::minutes(minutes);
                for item in &mut run.items {
                    item.sent_at -= Duration::minutes(minutes);
                }
            })
            .unwrap();
    }

    /// Records `item`'s last check as `state`, `days` ago.
    fn checked(&self, item: &CheckItem, state: CheckState, days: i64) {
        self.store()
            .update_protection_checks(&self.account.id, |checks| {
                *checks.record_mut(item) = CheckRecord {
                    state,
                    at: Some(Utc::now() - Duration::days(days)),
                    note: None,
                    test_message_id: Some("AAMk-earlier".into()),
                };
            })
            .unwrap();
    }

    /// Makes the last listing of the labels `hours` older.
    fn age_listing(&self, hours: i64) {
        let at = (Utc::now() - Duration::hours(hours)).timestamp();
        self.store().execute_for_tests(&format!(
            "UPDATE account_protection SET catalog_listed_at = {at}, \
             catalog_failed_at = CASE WHEN catalog_failed_at IS NULL THEN NULL ELSE {at} END"
        ));
    }

    async fn sync(&self) {
        self.service.sync_account(&self.account.id).await.unwrap();
    }

    async fn refresh(&self) -> ProtectionSetup {
        self.service
            .refresh_protection_check(&self.account.id)
            .await
            .unwrap()
    }
}

fn microsoft(tenant: &str, email: &str) -> Account {
    Account {
        id: Uuid::new_v4().to_string(),
        kind: AccountKind::Microsoft,
        display_name: "Adele".into(),
        email: email.into(),
        color: "#0b7a75".into(),
        settings: AccountSettings::Microsoft(MicrosoftSettings {
            tenant_id: tenant.into(),
            user_id: Uuid::new_v4().to_string(),
        }),
        created_at: Utc::now(),
        last_synced_at: None,
        sync_error: None,
        needs_permission: vec![],
        refused_features: vec![],
    }
}

/// Everything every account had, and sensitivity labels.
fn with_labels() -> Vec<String> {
    let mut scopes = legacy_graph_scopes();
    scopes.push("InformationProtectionPolicy.Read".into());
    scopes
}

/// A work account that turned sensitivity labels on, synced once (its
/// folders and labels stored).
async fn setup() -> Setup {
    let mut fake = FakeServer::default();
    fake.folders = vec![
        ("INBOX".into(), FolderRole::Inbox),
        ("Sent".into(), FolderRole::Sent),
        ("Trash".into(), FolderRole::Trash),
    ];
    fake.own_address = Some(EmailAddress::new(Some("Adele"), EMAIL));
    fake.organization.tenant = TENANT.into();
    let server = Arc::new(Mutex::new(fake));
    let connector = Arc::new(FakeConnector::new(Arc::clone(&server), false));
    let labels = FakeLabels::new(catalog());
    *connector.labels.lock().unwrap() = Some(labels.clone());
    let sink = Arc::new(Sink::default());
    let service = CoreService::new(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(MemorySecretStore::default()),
        connector.clone(),
        sink.clone(),
    );
    let account = microsoft(TENANT, EMAIL);
    service.store().insert_account(&account).unwrap();
    service
        .store()
        .set_sign_in_grant(&account.id, &with_labels())
        .unwrap();
    let s = Setup {
        service,
        connector,
        server,
        labels,
        sink,
        account,
    };
    s.sync().await;
    s
}

fn subject_token(subject: &str) -> &str {
    subject
        .strip_prefix("Cumulas Mail setup check [")
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or_else(|| panic!("not a check's subject: {subject}"))
}

// --- Who can --------------------------------------------------------------------

#[tokio::test]
async fn only_work_accounts_have_a_setup() {
    let s = setup().await;
    let setup = s.setup();
    assert!(setup.supported);
    assert_eq!(setup.labels.len(), 3);

    let personal = microsoft(CONSUMER_TENANT, "adele@outlook.com");
    s.store().insert_account(&personal).unwrap();
    let imap = crate::store::test_support::imap_account("imap");
    s.store().insert_account(&imap).unwrap();
    for account in [&personal, &imap] {
        let setup = s.service.protection_setup(&account.id).unwrap();
        assert!(!setup.supported);
        assert!(setup.labels.is_empty() && !setup.checking);
        let error = s
            .service
            .check_protection(&account.id, &[ENCRYPT])
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Unsupported);
        assert_eq!(
            error.message,
            "Only Microsoft 365 work and school accounts can send protected mail."
        );
        let error = s
            .service
            .refresh_sensitivity_labels(&account.id)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Unsupported);
    }
    assert_eq!(
        s.service.protection_setup("gone").unwrap_err().code,
        ErrorCode::NotFound
    );
}

// --- The label list ---------------------------------------------------------------

#[tokio::test]
async fn the_sync_lists_the_labels_every_six_hours() {
    let s = setup().await;
    assert_eq!(s.labels.calls(), 1);
    let setup = s.setup();
    assert_eq!(
        setup
            .labels
            .iter()
            .map(|l| l.label.id.as_str())
            .collect::<Vec<_>>(),
        [GENERAL, CONFIDENTIAL, SECRET]
    );
    assert!(
        setup
            .labels
            .iter()
            .all(|l| l.check.state == CheckState::NotChecked)
    );
    assert_eq!(setup.policy.default_label_id.as_deref(), Some(GENERAL));
    assert!(setup.labels_listed_at.is_some());
    assert_eq!(setup.labels_error, None);
    assert!(s.sink.count(DataKind::Protection) >= 1);

    s.sync().await;
    assert_eq!(s.labels.calls(), 1, "listed less than six hours ago");
    s.age_listing(7);
    s.sync().await;
    assert_eq!(s.labels.calls(), 2);
}

#[tokio::test]
async fn failed_listings_keep_the_last_list_and_back_off() {
    let s = setup().await;
    s.labels.answer(Err(Error::provider("Service unavailable")
        .with_details(json!({ "status": 503, "code": "ServiceUnavailable" }))));
    s.age_listing(7);
    s.sync().await;
    assert_eq!(s.labels.calls(), 2);
    let setup = s.setup();
    // The last list stays, with the error; the sync didn't fail.
    assert_eq!(setup.labels.len(), 3);
    assert_eq!(setup.labels_error.as_deref(), Some("Service unavailable"));
    assert_eq!(
        s.store().get_account(&s.account.id).unwrap().sync_error,
        None
    );

    // An hour, then two.
    s.sync().await;
    assert_eq!(s.labels.calls(), 2);
    let failed_ago = |minutes: i64| {
        let at = (Utc::now() - Duration::minutes(minutes)).timestamp();
        s.store().execute_for_tests(&format!(
            "UPDATE account_protection SET catalog_failed_at = {at}"
        ));
    };
    failed_ago(61);
    s.sync().await;
    assert_eq!(s.labels.calls(), 3);
    assert_eq!(s.store().label_catalog(&s.account.id).unwrap().failures, 2);
    failed_ago(61);
    s.sync().await;
    assert_eq!(s.labels.calls(), 3);
    failed_ago(121);
    s.sync().await;
    assert_eq!(s.labels.calls(), 4);

    // Refresh doesn't wait.
    let error = s
        .service
        .refresh_sensitivity_labels(&s.account.id)
        .await
        .unwrap_err();
    assert_eq!(error.message, "Service unavailable");
    assert_eq!(s.labels.calls(), 5);
    s.labels.answer(Ok(catalog()));
    let setup = s
        .service
        .refresh_sensitivity_labels(&s.account.id)
        .await
        .unwrap();
    assert_eq!(s.labels.calls(), 6);
    assert_eq!(setup.labels_error, None);
    assert_eq!(s.store().label_catalog(&s.account.id).unwrap().failures, 0);
}

#[tokio::test]
async fn a_refused_list_marks_the_feature_refused() {
    let s = setup().await;
    s.labels.answer(Err(Error::provider("Access is denied.")
        .with_details(json!({ "status": 403, "code": "Forbidden" }))));
    let error = s
        .service
        .refresh_sensitivity_labels(&s.account.id)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::PermissionRequired);
    assert_eq!(error.details.unwrap()["reason"], "refused");
    let account = s.store().get_account(&s.account.id).unwrap();
    assert_eq!(
        account.refused_features,
        [AccountFeature::SensitivityLabels]
    );
    // Its labels go, and aren't asked for again while refused.
    assert!(s.setup().labels.is_empty());
    assert_eq!(
        s.store().label_catalog(&s.account.id).unwrap().catalog,
        None
    );
    let calls = s.labels.calls();
    s.sync().await;
    assert_eq!(s.labels.calls(), calls);

    // Checked again: listed, and no longer refused.
    s.labels.answer(Ok(catalog()));
    s.service.retry_microsoft_features(&s.account.id).unwrap();
    s.sync().await;
    assert_eq!(s.labels.calls(), calls + 1);
    assert_eq!(s.setup().labels.len(), 3);
    assert!(
        s.store()
            .get_account(&s.account.id)
            .unwrap()
            .refused_features
            .is_empty()
    );
}

#[tokio::test]
async fn without_the_permission_the_labels_go() {
    let s = setup().await;
    s.store()
        .set_sign_in_grant(&s.account.id, &legacy_graph_scopes())
        .unwrap();
    let calls = s.labels.calls();
    s.sync().await;
    // The stub answered, not Microsoft 365.
    assert_eq!(s.labels.calls(), calls);
    assert_eq!(
        s.store().label_catalog(&s.account.id).unwrap().catalog,
        None
    );
    let setup = s.setup();
    assert!(setup.supported && setup.labels.is_empty());
    let error = s
        .service
        .refresh_sensitivity_labels(&s.account.id)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::PermissionRequired);
    assert_eq!(error.details.unwrap()["feature"], "sensitivityLabels");
    // Encrypt and Do Not Forward are still checkable.
    s.service
        .check_protection(&s.account.id, &[ENCRYPT])
        .await
        .unwrap();
}

#[tokio::test]
async fn turning_labels_on_lists_them_at_once() {
    let s = setup().await;
    s.labels.answer(Err(Error::network("offline")));
    s.age_listing(7);
    s.sync().await;
    let calls = s.labels.calls();
    s.sync().await;
    assert_eq!(s.labels.calls(), calls, "backing off");
    *s.connector.sign_in.lock().unwrap() = Some(crate::auth::microsoft::SignInResult {
        email: EMAIL.into(),
        display_name: "Adele".into(),
        tenant_id: TENANT.into(),
        user_id: match &s.account.settings {
            AccountSettings::Microsoft(m) => m.user_id.clone(),
            AccountSettings::Imap(_) => unreachable!(),
        },
        access_token: "access".into(),
        refresh_token: "refresh".into(),
        expires_in: std::time::Duration::from_secs(3600),
        requested_scopes: Vec::new(),
        response_scopes: with_labels(),
        consent_refused: None,
    });
    s.labels.answer(Ok(catalog()));
    s.service
        .add_microsoft_account(
            &|_| Ok(()),
            Some(EMAIL),
            PermissionRequest::Feature {
                feature: AccountFeature::SensitivityLabels,
            },
        )
        .await
        .unwrap();
    s.sync().await;
    assert_eq!(s.labels.calls(), calls + 1);
    assert_eq!(s.setup().labels_error, None);
}

#[tokio::test]
async fn personal_accounts_cant_turn_labels_on() {
    let s = setup().await;
    let personal = microsoft(CONSUMER_TENANT, "adele@outlook.com");
    s.store().insert_account(&personal).unwrap();
    let error = s
        .service
        .add_microsoft_account(
            &|_| Ok(()),
            Some("adele@outlook.com"),
            PermissionRequest::Feature {
                feature: AccountFeature::SensitivityLabels,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert_eq!(
        error.message,
        "Sensitivity labels are only for work and school accounts."
    );
    assert!(s.connector.sign_in_scopes.lock().unwrap().is_empty());
}

// --- Checks -----------------------------------------------------------------------

#[tokio::test]
async fn a_check_sends_a_control_and_test_messages_that_look_alike() {
    let s = setup().await;
    let setup = s
        .service
        .check_protection(&s.account.id, &[ENCRYPT, DO_NOT_FORWARD, label(GENERAL)])
        .await
        .unwrap();
    assert!(setup.checking);
    assert_eq!(setup.encrypt.state, CheckState::Waiting);
    assert_eq!(setup.do_not_forward.state, CheckState::Waiting);
    assert_eq!(s.label_check(GENERAL).state, CheckState::Waiting);
    assert_eq!(s.label_check(CONFIDENTIAL).state, CheckState::NotChecked);

    let server = s.server.lock().unwrap();
    // The control, without any request, sent like any mail.
    assert_eq!(server.sent.len(), 1);
    let control = &server.sent[0];
    assert_eq!(control.to, [EmailAddress::new(Some("Adele"), EMAIL)]);
    assert_eq!((control.protect, control.label_id.as_deref()), (None, None));
    // One test message per option, asking for it.
    let asked: Vec<_> = server
        .requested
        .iter()
        .map(|(_, r)| {
            (
                r.protect,
                r.label_id.clone(),
                r.tenant_id.clone(),
                r.label_encrypts,
            )
        })
        .collect();
    assert_eq!(
        asked,
        [
            (Some(ProtectAction::Encrypt), None, TENANT.into(), false),
            (
                Some(ProtectAction::DoNotForward),
                None,
                TENANT.into(),
                false
            ),
            (None, Some(GENERAL.into()), TENANT.into(), false),
        ]
    );
    // All alike but for the request: the same body, a subject that differs
    // only by its random token, naming no option or label.
    let mut tokens = vec![subject_token(&control.subject).to_owned()];
    for (message, _) in &server.requested {
        assert_eq!(message.body_html, control.body_html);
        assert_eq!(message.to, control.to);
        assert!(message.attachments.is_empty() && message.reply_kind.is_none());
        tokens.push(subject_token(&message.subject).to_owned());
    }
    for token in &tokens {
        assert_eq!(token.len(), 8);
        assert!(
            token
                .bytes()
                .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
        );
    }
    tokens.sort();
    tokens.dedup();
    assert_eq!(tokens.len(), 4);
    for message in std::iter::once(control).chain(server.requested.iter().map(|(m, _)| m)) {
        let text = format!("{} {}", message.subject, message.body_html).to_lowercase();
        for name in ["encrypt", "forward", "general", "label", GENERAL] {
            assert!(!text.contains(name), "{name} in {text}");
        }
    }
}

#[tokio::test]
async fn options_that_arrive_protected_against_a_plain_control_are_applied() {
    let s = setup().await;
    s.organization(FakeOrganization {
        encrypts: vec![ProtectAction::Encrypt],
        labels: vec![GENERAL.into()],
        ..FakeOrganization::default()
    });
    s.service
        .check_protection(
            &s.account.id,
            &[ENCRYPT, DO_NOT_FORWARD, label(GENERAL), label(CONFIDENTIAL)],
        )
        .await
        .unwrap();
    let setup = s.refresh().await;
    assert!(!setup.checking);
    assert_eq!(setup.encrypt.state, CheckState::Applied);
    assert_eq!(setup.encrypt.note, None);
    assert!(setup.encrypt.test_message);
    assert!(setup.encrypt.at.is_some());
    // No rule for these.
    assert_eq!(setup.do_not_forward.state, CheckState::NotApplied);
    assert_eq!(s.label_check(GENERAL).state, CheckState::Applied);
    assert_eq!(s.label_check(CONFIDENTIAL).state, CheckState::NotApplied);
    assert_eq!(s.run(), None);

    // Every message read went to Deleted Items, the control too.
    {
        let server = s.server.lock().unwrap();
        assert_eq!(server.delivered.len(), 5);
        assert!(server.delivered.iter().all(|d| d.folder == "Trash"));
    }

    // Its test message opens in Outlook on the web.
    let link = s
        .service
        .check_message_web_link(&s.account.id, &ENCRYPT)
        .await
        .unwrap();
    assert!(link.starts_with("https://outlook.office365.com/owa/?ItemID=AAMk-arrived-"));
    let link = s
        .service
        .check_message_web_link(&s.account.id, &label(&GENERAL.to_uppercase()))
        .await
        .unwrap();
    assert!(link.starts_with("https://"));
    let error = s
        .service
        .check_message_web_link(&s.account.id, &label(SECRET))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);
    assert_eq!(
        error.message,
        "There's no test message to open. Check again."
    );
}

#[tokio::test]
async fn a_control_protected_too_means_another_rule() {
    let s = setup().await;
    // Everything encrypted, and Cumulas Mail's own rule too.
    s.organization(FakeOrganization {
        encrypts: vec![ProtectAction::Encrypt],
        encrypts_everything: true,
        ..FakeOrganization::default()
    });
    s.service
        .check_protection(&s.account.id, &[ENCRYPT, DO_NOT_FORWARD])
        .await
        .unwrap();
    let setup = s.refresh().await;
    for check in [setup.encrypt, setup.do_not_forward] {
        assert_eq!(check.state, CheckState::Unknown);
        assert_eq!(check.note, Some(CheckNote::OtherRule));
    }
}

#[tokio::test]
async fn a_default_label_blocks_only_its_own_check() {
    let s = setup().await;
    s.organization(FakeOrganization {
        labels: vec![GENERAL.into(), CONFIDENTIAL.into()],
        default_label: Some(GENERAL.into()),
        ..FakeOrganization::default()
    });
    s.service
        .check_protection(&s.account.id, &[label(GENERAL), label(CONFIDENTIAL)])
        .await
        .unwrap();
    s.refresh().await;
    let general = s.label_check(GENERAL);
    assert_eq!(
        (general.state, general.note),
        (CheckState::Unknown, Some(CheckNote::OtherRule))
    );
    assert_eq!(s.label_check(CONFIDENTIAL).state, CheckState::Applied);
}

#[tokio::test]
async fn encryption_that_keeps_the_header_may_be_another_rules() {
    let s = setup().await;
    s.organization(FakeOrganization {
        encrypts: vec![ProtectAction::Encrypt],
        keeps_header: true,
        ..FakeOrganization::default()
    });
    s.service
        .check_protection(&s.account.id, &[ENCRYPT])
        .await
        .unwrap();
    let setup = s.refresh().await;
    assert_eq!(
        (setup.encrypt.state, setup.encrypt.note),
        (CheckState::Unknown, Some(CheckNote::HeaderKept))
    );
}

#[tokio::test]
async fn messages_that_never_arrive_end_the_check_after_15_minutes() {
    let s = setup().await;
    s.organization(FakeOrganization {
        encrypts: vec![ProtectAction::Encrypt],
        ..FakeOrganization::default()
    });
    s.server.lock().unwrap().hold = vec!["Cumulas Mail setup check".into()];
    s.service
        .check_protection(&s.account.id, &[ENCRYPT, label(GENERAL)])
        .await
        .unwrap();
    let setup = s.refresh().await;
    assert!(setup.checking);
    assert_eq!(setup.encrypt.state, CheckState::Waiting);
    s.age_run(14);
    assert!(s.refresh().await.checking);
    s.age_run(2);
    let setup = s.refresh().await;
    assert!(!setup.checking);
    for check in [setup.encrypt, s.label_check(GENERAL)] {
        assert_eq!(
            (check.state, check.note),
            (CheckState::Unknown, Some(CheckNote::NotArrived))
        );
        assert!(!check.test_message);
    }
    // A lookup failure meanwhile keeps a check waiting.
    s.server.lock().unwrap().release();
    s.server.lock().unwrap().lookup_failure = Some(Error::network("offline"));
    s.service
        .check_protection(&s.account.id, &[ENCRYPT])
        .await
        .unwrap();
    assert!(s.refresh().await.checking);
}

#[tokio::test]
async fn an_item_is_judged_only_once_the_control_arrived() {
    let s = setup().await;
    s.organization(FakeOrganization {
        encrypts: vec![ProtectAction::Encrypt],
        ..FakeOrganization::default()
    });
    s.server.lock().unwrap().hold = vec!["Cumulas Mail setup check".into()];
    s.service
        .check_protection(&s.account.id, &[ENCRYPT])
        .await
        .unwrap();
    // The item arrives; the control doesn't yet.
    let control = s.run().unwrap().control_token;
    {
        let mut server = s.server.lock().unwrap();
        let (held, arrived): (Vec<_>, Vec<_>) = std::mem::take(&mut server.held)
            .into_iter()
            .partition(|d| d.subject.contains(&control));
        server.held = held;
        server.delivered.extend(arrived);
    }
    let setup = s.refresh().await;
    assert_eq!(setup.encrypt.state, CheckState::Waiting);
    assert_eq!(s.server.lock().unwrap().delivered[0].folder, "INBOX");

    // Arrived without its properties: not judged.
    s.server.lock().unwrap().release();
    s.server.lock().unwrap().properties_missing = 1;
    let setup = s.refresh().await;
    assert_eq!(setup.encrypt.state, CheckState::Waiting);
    assert!(s.run().unwrap().control.is_none());
    let setup = s.refresh().await;
    assert_eq!(setup.encrypt.state, CheckState::Applied);
}

#[tokio::test]
async fn a_failed_send_keeps_what_wasnt_learned() {
    let s = setup().await;
    s.checked(&DO_NOT_FORWARD, CheckState::Applied, 3);
    s.checked(&label(GENERAL), CheckState::NotApplied, 5);
    s.server.lock().unwrap().requesting_failures = [
        None,
        Some(Error::network("Could not reach Microsoft Graph")),
    ]
    .into();
    let error = s
        .service
        .check_protection(&s.account.id, &[ENCRYPT, DO_NOT_FORWARD, label(GENERAL)])
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Network);
    let setup = s.setup();
    // Sent: waiting for it.
    assert!(setup.checking);
    assert_eq!(setup.encrypt.state, CheckState::Waiting);
    // Not sent: as before.
    assert_eq!(setup.do_not_forward.state, CheckState::Applied);
    assert!(setup.do_not_forward.test_message);
    assert_eq!(s.label_check(GENERAL).state, CheckState::NotApplied);
    assert_eq!(s.server.lock().unwrap().requested.len(), 2);
    assert_eq!(s.run().unwrap().items.len(), 1);
}

#[tokio::test]
async fn a_draft_that_loses_the_property_ends_that_item_only() {
    let s = setup().await;
    s.organization(FakeOrganization {
        encrypts: vec![ProtectAction::Encrypt],
        drops_labels: vec![GENERAL.into()],
        ..FakeOrganization::default()
    });
    let setup = s
        .service
        .check_protection(&s.account.id, &[label(GENERAL), ENCRYPT])
        .await
        .unwrap();
    let general = s.label_check(GENERAL);
    assert_eq!(
        (general.state, general.note),
        (CheckState::NotApplied, Some(CheckNote::HeaderNotKept))
    );
    assert_eq!(setup.encrypt.state, CheckState::Waiting);
    assert_eq!(s.refresh().await.encrypt.state, CheckState::Applied);
    assert_eq!(s.label_check(GENERAL).note, Some(CheckNote::HeaderNotKept));

    // Every item refused: nothing to wait for.
    let setup = s
        .service
        .check_protection(&s.account.id, &[label(GENERAL)])
        .await
        .unwrap();
    assert!(!setup.checking);
}

#[tokio::test]
async fn checks_refuse_what_cant_be_checked() {
    let s = setup().await;
    let check = |items: Vec<CheckItem>| {
        let service = &s.service;
        let id = s.account.id.clone();
        async move { service.check_protection(&id, &items).await.unwrap_err() }
    };
    let error = check(vec![]).await;
    assert_eq!(
        (error.code, error.message.as_str()),
        (ErrorCode::InvalidInput, "Choose what to check.")
    );
    let error = check(vec![label("6c1b3b5e-0d4f-4b1e-9d3a-00000000000f")]).await;
    assert_eq!(
        error.message,
        "That sensitivity label isn't available for adele@contoso.com."
    );
    // At most 19 at a time.
    let many = LabelCatalog {
        labels: (0..25)
            .map(|i| {
                catalog_label(
                    &format!("6c1b3b5e-0d4f-4b1e-9d3a-0000000001{i:02}"),
                    "L",
                    None,
                    false,
                )
            })
            .collect(),
        ..LabelCatalog::default()
    };
    s.labels.answer(Ok(many.clone()));
    s.service
        .refresh_sensitivity_labels(&s.account.id)
        .await
        .unwrap();
    let twenty: Vec<_> = many.labels[..20].iter().map(|l| label(&l.id)).collect();
    let error = check(twenty).await;
    assert_eq!(error.message, "Cumulas Mail checks at most 19 at a time.");
    assert!(s.server.lock().unwrap().sent.is_empty());

    // One at a time.
    s.service
        .check_protection(&s.account.id, &[ENCRYPT])
        .await
        .unwrap();
    let error = check(vec![DO_NOT_FORWARD]).await;
    assert_eq!(
        error.message,
        "Wait for the check that's running to finish."
    );
    assert_eq!(s.server.lock().unwrap().sent.len(), 1);
}

#[tokio::test]
async fn a_sync_moves_a_waiting_check_on() {
    let s = setup().await;
    s.organization(FakeOrganization {
        encrypts: vec![ProtectAction::DoNotForward],
        ..FakeOrganization::default()
    });
    s.service
        .check_protection(&s.account.id, &[DO_NOT_FORWARD])
        .await
        .unwrap();
    s.sync().await;
    assert_eq!(s.setup().do_not_forward.state, CheckState::Applied);
}

#[tokio::test]
async fn checking_again_keeps_the_last_pass_until_its_decided() {
    let s = setup().await;
    s.labels.answer(Ok(LabelCatalog {
        policy: LabelPolicy {
            required: true,
            ..LabelPolicy::default()
        },
        ..catalog()
    }));
    s.service
        .refresh_sensitivity_labels(&s.account.id)
        .await
        .unwrap();
    s.checked(&ENCRYPT, CheckState::Applied, 3);
    s.checked(&label(GENERAL), CheckState::Applied, 3);
    let passed = s.setup().encrypt.at;
    assert_eq!(s.setup().encrypt.passed_at, passed);
    s.server.lock().unwrap().hold = vec!["Cumulas Mail setup check".into()];
    s.service
        .check_protection(
            &s.account.id,
            &[ENCRYPT, label(GENERAL), label(CONFIDENTIAL)],
        )
        .await
        .unwrap();

    // Shown waiting, while the passes before still count.
    let setup = s.setup();
    assert_eq!(setup.encrypt.state, CheckState::Waiting);
    assert_eq!(setup.encrypt.passed_at, passed);
    assert!(!setup.encrypt.test_message);
    let confidential = s.label_check(CONFIDENTIAL);
    assert_eq!(confidential.state, CheckState::Waiting);
    assert_eq!(confidential.passed_at, None);
    // A label is still required, and the passed options still go out.
    let error = s.service.send_message(outgoing(&s)).await.unwrap_err();
    assert_eq!(error.details.unwrap(), json!({ "reason": "labelRequired" }));
    s.service
        .send_message(OutgoingMessage {
            protect: Some(ProtectAction::Encrypt),
            label_id: Some(GENERAL.into()),
            ..outgoing(&s)
        })
        .await
        .unwrap();
    let error = s
        .service
        .send_message(OutgoingMessage {
            label_id: Some(CONFIDENTIAL.into()),
            ..outgoing(&s)
        })
        .await
        .unwrap_err();
    assert_eq!(
        error.message,
        "The label Confidential is being checked for adele@contoso.com. Wait for the check in \
         Settings → Accounts to finish."
    );

    // The test messages never come: nothing was learned, so the passes
    // stay, saying so; what hadn't passed couldn't be checked.
    s.age_run(16);
    let setup = s.refresh().await;
    assert!(!setup.checking);
    assert_eq!(
        (setup.encrypt.state, setup.encrypt.note),
        (CheckState::Applied, Some(CheckNote::NotArrived))
    );
    assert_eq!(setup.encrypt.at, passed);
    assert_eq!(setup.encrypt.passed_at, passed);
    assert!(setup.encrypt.test_message);
    assert_eq!(s.label_check(GENERAL).state, CheckState::Applied);
    let confidential = s.label_check(CONFIDENTIAL);
    assert_eq!(
        (confidential.state, confidential.note),
        (CheckState::Unknown, Some(CheckNote::NotArrived))
    );

    // A check that's decided replaces the pass.
    s.server.lock().unwrap().release();
    s.server.lock().unwrap().hold.clear();
    s.service
        .check_protection(&s.account.id, &[ENCRYPT])
        .await
        .unwrap();
    let setup = s.refresh().await;
    assert_eq!(
        (setup.encrypt.state, setup.encrypt.note),
        (CheckState::NotApplied, None)
    );
    assert_eq!(setup.encrypt.passed_at, None);
}

#[tokio::test]
async fn passed_checks_expire_after_90_days() {
    let s = setup().await;
    s.checked(&ENCRYPT, CheckState::Applied, 89);
    s.checked(&label(GENERAL), CheckState::Applied, 91);
    let setup = s.setup();
    assert_eq!(setup.encrypt.state, CheckState::Applied);
    let general = s.label_check(GENERAL);
    assert_eq!(general.state, CheckState::Expired);
    // When it was last checked shows.
    assert!(general.at.unwrap() < Utc::now() - Duration::days(90));
    // Only passes expire.
    s.checked(&DO_NOT_FORWARD, CheckState::NotApplied, 91);
    assert_eq!(s.setup().do_not_forward.state, CheckState::NotApplied);
}

// --- Sending ----------------------------------------------------------------------

fn outgoing(s: &Setup) -> OutgoingMessage {
    OutgoingMessage {
        account_id: s.account.id.clone(),
        to: vec![EmailAddress::new(None, "pat@fabrikam.com")],
        cc: vec![],
        bcc: vec![],
        subject: "Atlas pricing".into(),
        body_html: "<p>Attached.</p>".into(),
        attachments: vec![],
        importance: None,
        reply_to_message_id: None,
        reply_kind: None,
        protect: None,
        label_id: None,
    }
}

/// A message in the account's `folder` with this content, stored.
async fn stored_message(s: &Setup, folder: &str, remote: &str, content: MessageContent) -> String {
    let m = summary(&s.account.id, folder, remote, "Q4 forecast");
    let id = m.id.clone();
    {
        let mut server = s.server.lock().unwrap();
        server.messages.insert(remote.into(), (folder.into(), m));
        server.contents.insert(remote.into(), content);
    }
    s.sync().await;
    id
}

fn rpmsg() -> AttachmentMeta {
    AttachmentMeta {
        id: "att-rpmsg".into(),
        name: "message.rpmsg".into(),
        content_type: "application/x-microsoft-rpmsg-message".into(),
        size: 48_213,
        is_inline: false,
        content_id: None,
    }
}

fn msip(id: &str, tenant: &str, name: &str) -> String {
    format!(
        "MSIP_Label_{id}_Enabled=true; MSIP_Label_{id}_SiteId={tenant}; MSIP_Label_{id}_Name={name}"
    )
}

#[tokio::test]
async fn only_checked_options_can_be_asked_for() {
    let s = setup().await;
    let send = |message: OutgoingMessage| s.service.send_message(message);
    let error = send(OutgoingMessage {
        protect: Some(ProtectAction::Encrypt),
        ..outgoing(&s)
    })
    .await
    .unwrap_err();
    assert_eq!(
        (error.code, error.message.as_str()),
        (
            ErrorCode::InvalidInput,
            "Encrypt isn't set up for adele@contoso.com. Check it in Settings → Accounts."
        )
    );
    s.checked(&DO_NOT_FORWARD, CheckState::Applied, 91);
    let error = send(OutgoingMessage {
        protect: Some(ProtectAction::DoNotForward),
        ..outgoing(&s)
    })
    .await
    .unwrap_err();
    assert_eq!(
        error.message,
        "Do Not Forward was last checked more than 90 days ago. Check it again in Settings → \
         Accounts."
    );
    for unknown in ["Confidential", "6c1b3b5e-0d4f-4b1e-9d3a-00000000000f"] {
        let error = send(OutgoingMessage {
            label_id: Some(unknown.into()),
            ..outgoing(&s)
        })
        .await
        .unwrap_err();
        assert_eq!(
            error.message,
            "That sensitivity label isn't available for adele@contoso.com."
        );
    }
    let error = send(OutgoingMessage {
        label_id: Some(GENERAL.into()),
        ..outgoing(&s)
    })
    .await
    .unwrap_err();
    assert_eq!(
        error.message,
        "The label General isn't set up for adele@contoso.com. Check it in Settings → Accounts."
    );
    s.checked(&label(GENERAL), CheckState::Applied, 91);
    let error = send(OutgoingMessage {
        label_id: Some(GENERAL.into()),
        ..outgoing(&s)
    })
    .await
    .unwrap_err();
    assert_eq!(
        error.message,
        "The label General was last checked more than 90 days ago. Check it again in Settings → \
         Accounts."
    );
    s.checked(&label(SECRET), CheckState::Applied, 1);
    s.checked(&ENCRYPT, CheckState::Applied, 1);
    let error = send(OutgoingMessage {
        label_id: Some(SECRET.into()),
        protect: Some(ProtectAction::Encrypt),
        ..outgoing(&s)
    })
    .await
    .unwrap_err();
    assert_eq!(
        error.message,
        "The label Highly Confidential › Secret already encrypts this message. Choose either the \
         label or Encrypt."
    );
    let server = s.server.lock().unwrap();
    assert!(server.sent.is_empty() && server.requested.is_empty());
}

#[tokio::test]
async fn checked_options_go_out_through_a_draft_that_asks_for_them() {
    let s = setup().await;
    s.checked(&ENCRYPT, CheckState::Applied, 1);
    s.checked(&label(CONFIDENTIAL), CheckState::Applied, 1);
    s.checked(&label(SECRET), CheckState::Applied, 1);
    s.service
        .send_message(OutgoingMessage {
            protect: Some(ProtectAction::Encrypt),
            label_id: Some(CONFIDENTIAL.to_uppercase()),
            ..outgoing(&s)
        })
        .await
        .unwrap();
    s.service
        .send_message(OutgoingMessage {
            label_id: Some(SECRET.into()),
            ..outgoing(&s)
        })
        .await
        .unwrap();
    s.service.send_message(outgoing(&s)).await.unwrap();
    let server = s.server.lock().unwrap();
    let asked: Vec<_> = server.requested.iter().map(|(_, r)| r.clone()).collect();
    assert_eq!(
        asked,
        [
            HeaderRequest {
                protect: Some(ProtectAction::Encrypt),
                label_id: Some(CONFIDENTIAL.into()),
                tenant_id: TENANT.into(),
                label_encrypts: false,
            },
            HeaderRequest {
                protect: None,
                label_id: Some(SECRET.into()),
                tenant_id: TENANT.into(),
                label_encrypts: true,
            },
        ]
    );
    // Ordinary mail as before.
    assert_eq!(server.sent.len(), 1);
}

#[tokio::test]
async fn a_required_label_is_required_once_one_can_be_applied() {
    let s = setup().await;
    s.labels.answer(Ok(LabelCatalog {
        policy: LabelPolicy {
            required: true,
            ..LabelPolicy::default()
        },
        ..catalog()
    }));
    s.service
        .refresh_sensitivity_labels(&s.account.id)
        .await
        .unwrap();
    // None applies yet: nothing blocks.
    s.service.send_message(outgoing(&s)).await.unwrap();
    s.checked(&label(GENERAL), CheckState::Applied, 1);
    s.checked(&ENCRYPT, CheckState::Applied, 1);
    for protect in [None, Some(ProtectAction::Encrypt)] {
        let error = s
            .service
            .send_message(OutgoingMessage {
                protect,
                ..outgoing(&s)
            })
            .await
            .unwrap_err();
        assert_eq!(
            error.message,
            "Choose a sensitivity label. Your organization requires one."
        );
        assert_eq!(error.details.unwrap(), json!({ "reason": "labelRequired" }));
    }
    s.service
        .send_message(OutgoingMessage {
            label_id: Some(GENERAL.into()),
            ..outgoing(&s)
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn protected_originals_are_answered_in_outlook_on_the_web() {
    let s = setup().await;
    let original = stored_message(
        &s,
        "INBOX",
        "AAMk-protected",
        MessageContent {
            body_html: Some("<p>You've received an encrypted message.</p>".into()),
            attachments: vec![rpmsg()],
            ..MessageContent::default()
        },
    )
    .await;
    for kind in [ReplyKind::Reply, ReplyKind::ReplyAll, ReplyKind::Forward] {
        let error = s
            .service
            .send_message(OutgoingMessage {
                reply_to_message_id: Some(original.clone()),
                reply_kind: Some(kind),
                ..outgoing(&s)
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Unsupported);
        assert_eq!(
            error.message,
            "This message is protected. Reply to it or forward it in Outlook on the web."
        );
        assert_eq!(
            error.details.unwrap(),
            json!({
                "reason": "protectedOriginal",
                "messageId": original,
                "protection": "rightsManaged",
            })
        );
    }

    // IMAP: no Outlook on the web.
    let imap = crate::store::test_support::imap_account("imap");
    s.store().insert_account(&imap).unwrap();
    s.service.sync_account(&imap.id).await.unwrap();
    let m = summary(&imap.id, "INBOX", "INBOX:9", "Tax documents");
    let encrypted = m.id.clone();
    {
        let mut server = s.server.lock().unwrap();
        server
            .messages
            .insert("INBOX:9".into(), ("INBOX".into(), m));
        server.contents.insert(
            "INBOX:9".into(),
            MessageContent {
                attachments: vec![AttachmentMeta {
                    name: "smime.p7m".into(),
                    content_type: "application/pkcs7-mime".into(),
                    ..rpmsg()
                }],
                signals: MessageSignals {
                    mime_type: Some("application/pkcs7-mime; smime-type=enveloped-data".into()),
                    ..MessageSignals::default()
                },
                ..MessageContent::default()
            },
        );
    }
    s.service.sync_account(&imap.id).await.unwrap();
    let error = s
        .service
        .send_message(OutgoingMessage {
            account_id: imap.id.clone(),
            reply_to_message_id: Some(encrypted),
            reply_kind: Some(ReplyKind::Reply),
            ..outgoing(&s)
        })
        .await
        .unwrap_err();
    assert_eq!(
        error.message,
        "This message is encrypted. Cumulas Mail can't reply to it or forward it without removing \
         the encryption."
    );
    let details = error.details.unwrap();
    assert_eq!(details["reason"], "protectedOriginal");
    assert_eq!(details["protection"], "smimeEncrypted");
    // Nor protected mail.
    let error = s
        .service
        .send_message(OutgoingMessage {
            account_id: imap.id.clone(),
            protect: Some(ProtectAction::Encrypt),
            ..outgoing(&s)
        })
        .await
        .unwrap_err();
    assert_eq!(
        (error.code, error.message.as_str()),
        (
            ErrorCode::Unsupported,
            "Only Microsoft 365 work and school accounts can send protected mail."
        )
    );
}

#[tokio::test]
async fn a_copied_label_is_named_from_the_catalog() {
    let s = setup().await;
    for id in [GENERAL, CONFIDENTIAL, SECRET] {
        s.checked(&label(id), CheckState::Applied, 1);
    }
    let original = stored_message(&s, "INBOX", "AAMk-labeled", MessageContent::default()).await;
    let reply = |label_id: &str| OutgoingMessage {
        reply_to_message_id: Some(original.clone()),
        reply_kind: Some(ReplyKind::Reply),
        label_id: Some(label_id.into()),
        ..outgoing(&s)
    };
    let copied = |id: &str| {
        s.server.lock().unwrap().requesting_failures = [Some(label_copied(id))].into();
    };

    copied(GENERAL);
    let error = s
        .service
        .send_message(reply(CONFIDENTIAL))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert_eq!(
        error.message,
        "This reply already carries the label General from the original, and Cumulas Mail \
         can't make sure your organization's mail server applies Confidential instead. Choose \
         General, or send it from Outlook on the web."
    );
    assert_eq!(
        error.details.unwrap(),
        json!({ "reason": "labelCopied", "labelId": GENERAL })
    );

    // Listed, but not offered: choosing it isn't possible here.
    for (state, days) in [
        (CheckState::NotChecked, 0),
        (CheckState::NotApplied, 1),
        (CheckState::Applied, 91),
    ] {
        s.checked(&label(GENERAL), state, days);
        copied(GENERAL);
        let error = s
            .service
            .send_message(reply(CONFIDENTIAL))
            .await
            .unwrap_err();
        assert_eq!(
            error.message,
            "This reply already carries the label General from the original, and Cumulas Mail \
             can't make sure your organization's mail server applies Confidential instead. Send \
             it from Outlook on the web instead.",
            "{state:?}"
        );
    }
    s.checked(&label(GENERAL), CheckState::Applied, 1);

    let unknown = "6c1b3b5e-0d4f-4b1e-9d3a-00000000000f";
    copied(unknown);
    let error = s
        .service
        .send_message(reply(CONFIDENTIAL))
        .await
        .unwrap_err();
    assert_eq!(
        error.message,
        "This reply already carries a sensitivity label from the original, and Cumulas Mail \
         can't make sure your organization's mail server applies Confidential instead. Send it \
         from Outlook on the web instead."
    );

    copied(GENERAL);
    let forward = OutgoingMessage {
        reply_kind: Some(ReplyKind::Forward),
        ..reply(SECRET)
    };
    let error = s.service.send_message(forward).await.unwrap_err();
    assert_eq!(
        error.message,
        "This forward already carries the label General from the original, so Cumulas Mail \
         can't make sure your organization's mail server encrypts it. Send it from Outlook on \
         the web instead."
    );

    // A draft that lost the property: nothing sent, in its words.
    s.server.lock().unwrap().requesting_failures =
        [Some(crate::protection::headers_not_kept(None))].into();
    let error = s.service.send_message(reply(GENERAL)).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::Provider);
    assert_eq!(
        error.message,
        "Cumulas Mail couldn't add the label or encryption to this reply, so it wasn't sent. \
         Send it from Outlook on the web instead."
    );
    assert_eq!(error.details.unwrap()["reason"], "headersNotKept");
}

// --- Reading ---------------------------------------------------------------------

#[tokio::test]
async fn messages_show_their_protection_and_their_organizations_label() {
    let s = setup().await;
    let html = r#"<a href="https://outlook.office365.com/Encryption/retrieve.ashx?id=1">Read the message</a>"#;
    let protected = stored_message(
        &s,
        "INBOX",
        "AAMk-1",
        MessageContent {
            body_html: Some(html.into()),
            attachments: vec![rpmsg()],
            signals: MessageSignals {
                msip_labels: Some(msip(GENERAL, TENANT, "General")),
                msip_labels_source: Some("header".into()),
                ..MessageSignals::default()
            },
            ..MessageContent::default()
        },
    )
    .await;
    let message = s.service.get_message(&protected, false).await.unwrap();
    let protection = message.protection.unwrap();
    assert_eq!(protection.kind, ProtectionKind::RightsManaged);
    assert_eq!(protection.container_ids, ["att-rpmsg"]);
    assert_eq!(
        protection.portal_url.as_deref(),
        Some("https://outlook.office365.com/Encryption/retrieve.ashx?id=1")
    );
    let label = message.label.unwrap();
    assert_eq!(label.id, GENERAL);
    assert_eq!(label.name.as_deref(), Some("General"));
    assert_eq!(label.color.as_deref(), Some("#2b7aa8"));
    assert!(label.from_catalog);
    assert_eq!(message.requested, None);
    // The attachments are listed as they are; the UI leaves the container
    // out.
    assert_eq!(message.attachments.len(), 1);

    // Another organization's label, and a forged id: no label.
    let labeled = |id: &str, tenant: &str| MessageContent {
        signals: MessageSignals {
            msip_labels: Some(msip(id, tenant, "Highly Confidential")),
            ..MessageSignals::default()
        },
        ..MessageContent::default()
    };
    let foreign = stored_message(&s, "INBOX", "AAMk-2", labeled(GENERAL, OTHER_TENANT)).await;
    assert_eq!(
        s.service.get_message(&foreign, false).await.unwrap().label,
        None
    );
    let forged_id = "6c1b3b5e-0d4f-4b1e-9d3a-00000000000f";
    let forged = stored_message(&s, "INBOX", "AAMk-3", labeled(forged_id, TENANT)).await;
    assert_eq!(
        s.service.get_message(&forged, false).await.unwrap().label,
        None
    );

    // Without a list, that a label is named, never the name the message
    // gives it: anyone can write it, with the organization's public tenant.
    s.store()
        .set_sign_in_grant(&s.account.id, &legacy_graph_scopes())
        .unwrap();
    for id in [&forged, &protected] {
        let label = s
            .service
            .get_message(id, false)
            .await
            .unwrap()
            .label
            .unwrap();
        assert_eq!(label.name, None);
        assert!(!label.from_catalog);
        assert_eq!(label.color, None);
    }
    let label = s.service.get_message(&forged, false).await.unwrap().label;
    assert_eq!(label.unwrap().id, forged_id);
}

#[tokio::test]
async fn sent_copies_show_what_was_requested() {
    let s = setup().await;
    let requesting = MessageContent {
        body_html: Some(
            r#"<blockquote><a href="https://outlook.office365.com/Encryption/x">Read</a></blockquote>"#
                .into(),
        ),
        signals: MessageSignals {
            requested_action: Some("encrypt".into()),
            requested_label: Some(CONFIDENTIAL.to_uppercase()),
            ..MessageSignals::default()
        },
        ..MessageContent::default()
    };
    let sent = stored_message(&s, "Sent", "AAMk-sent", requesting.clone()).await;
    let message = s.service.get_message(&sent, false).await.unwrap();
    let requested = message.requested.unwrap();
    assert_eq!(requested.action, Some(ProtectAction::Encrypt));
    let label = requested.label.unwrap();
    assert_eq!(
        (label.id.as_str(), label.name.as_deref(), label.from_catalog),
        (CONFIDENTIAL, Some("Confidential"), true)
    );
    // A quoted portal notice in Sent isn't a portal link.
    assert_eq!(message.protection, None);

    // Only in Sent.
    let inbox = stored_message(&s, "INBOX", "AAMk-in", requesting).await;
    let message = s.service.get_message(&inbox, false).await.unwrap();
    assert_eq!(message.requested, None);
    assert_eq!(message.protection.unwrap().kind, ProtectionKind::PortalLink);
}

#[tokio::test]
async fn outdated_content_is_fetched_again_and_kept_offline() {
    let s = setup().await;
    let id = stored_message(&s, "INBOX", "AAMk-1", MessageContent::default()).await;
    let fetches = || s.server.lock().unwrap().fetches;
    s.service.get_message(&id, false).await.unwrap();
    assert_eq!(fetches(), 1);
    s.service.get_message(&id, false).await.unwrap();
    assert_eq!(fetches(), 1, "current: from the cache");

    // Kept by an older version.
    s.store().execute_for_tests(&format!(
        r#"UPDATE messages SET content_json = '{{"attachments":[]}}' WHERE id = '{id}'"#
    ));
    s.service.get_message(&id, false).await.unwrap();
    assert_eq!(fetches(), 2);
    s.service.get_message(&id, false).await.unwrap();
    assert_eq!(fetches(), 2);

    // Properties that couldn't be read: tried again, three times in all.
    s.store().execute_for_tests(&format!(
        r#"UPDATE messages SET content_json = '{{}}' WHERE id = '{id}'"#
    ));
    s.server.lock().unwrap().properties_missing = 10;
    for expected in [3, 4, 5, 5, 5] {
        s.service.get_message(&id, false).await.unwrap();
        assert_eq!(fetches(), expected);
    }

    // Offline: the old copy.
    s.store().execute_for_tests(&format!(
        r#"UPDATE messages SET content_json = '{{}}', body_html = '<p>Kept</p>' WHERE id = '{id}'"#
    ));
    s.server.lock().unwrap().fetch_failure = Some(Error::network("offline"));
    let message = s.service.get_message(&id, false).await.unwrap();
    assert!(message.body_html.unwrap().contains("Kept"));
    // Gone from the server: gone here too.
    s.server.lock().unwrap().fetch_failure = Some(Error::not_found("message"));
    let error = s.service.get_message(&id, false).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);
}

#[tokio::test]
async fn container_files_are_never_opened() {
    let s = setup().await;
    let invoice = AttachmentMeta {
        id: "att-invoice".into(),
        name: "fattura.xml.p7m".into(),
        content_type: "application/pkcs7-mime".into(),
        ..rpmsg()
    };
    let protected = stored_message(
        &s,
        "INBOX",
        "AAMk-1",
        MessageContent {
            attachments: vec![rpmsg(), invoice.clone()],
            ..MessageContent::default()
        },
    )
    .await;
    let error = s
        .service
        .get_attachment_to_open(&protected, "att-rpmsg")
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Unsupported);
    assert_eq!(
        error.message,
        "This file holds a protected message's content, which Cumulas Mail can't open. Open the \
         message in Outlook on the web."
    );
    let opened = s
        .service
        .get_attachment_to_open(&protected, "att-invoice")
        .await
        .unwrap();
    assert_eq!(opened.meta.name, "fattura.xml.p7m");
    // Saving isn't refused.
    s.service
        .get_attachment(&protected, "att-rpmsg")
        .await
        .unwrap();

    // An e-invoice in ordinary mail is an ordinary file.
    let ordinary = stored_message(
        &s,
        "INBOX",
        "AAMk-2",
        MessageContent {
            body_html: Some("<p>In allegato la fattura.</p>".into()),
            attachments: vec![invoice],
            ..MessageContent::default()
        },
    )
    .await;
    assert_eq!(
        s.service
            .get_message(&ordinary, false)
            .await
            .unwrap()
            .protection,
        None
    );
    s.service
        .get_attachment_to_open(&ordinary, "att-invoice")
        .await
        .unwrap();
}

#[tokio::test]
async fn messages_open_in_outlook_on_the_web_and_describe_themselves() {
    let s = setup().await;
    let id = stored_message(
        &s,
        "INBOX",
        "AAMk-1",
        MessageContent {
            attachments: vec![rpmsg()],
            ..MessageContent::default()
        },
    )
    .await;
    assert_eq!(
        s.service.message_web_link(&id).await.unwrap(),
        "https://outlook.office365.com/owa/?ItemID=AAMk-1"
    );
    let text = s.service.message_diagnostics(&id).await.unwrap();
    assert!(text.contains("Account: Microsoft 365 (work or school)"));
    assert!(text.contains("Detected: Microsoft Purview encryption"));
    assert!(text.contains("  - message.rpmsg · application/x-microsoft-rpmsg-message"));
    assert!(!text.contains("Q4 forecast"));
    assert_eq!(
        s.service.message_web_link("gone").await.unwrap_err().code,
        ErrorCode::NotFound
    );

    let imap = crate::store::test_support::imap_account("imap");
    s.store().insert_account(&imap).unwrap();
    s.service.sync_account(&imap.id).await.unwrap();
    let m = summary(&imap.id, "INBOX", "INBOX:9", "Hi");
    let imap_message = local_id(&imap.id, EntityKind::Message, "INBOX:9");
    s.server
        .lock()
        .unwrap()
        .messages
        .insert("INBOX:9".into(), ("INBOX".into(), m));
    s.service.sync_account(&imap.id).await.unwrap();
    let error = s.service.message_web_link(&imap_message).await.unwrap_err();
    assert_eq!(
        (error.code, error.message.as_str()),
        (
            ErrorCode::Unsupported,
            "Only Microsoft 365 messages open in Outlook on the web."
        )
    );
    let text = s.service.message_diagnostics(&imap_message).await.unwrap();
    assert!(text.contains("Account: IMAP"));
    assert!(text.contains("Detected: nothing"));
}
