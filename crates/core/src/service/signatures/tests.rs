//! Signature tests against a scripted organization.

use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::{DateTime, Duration, TimeZone, Utc};
use url::Url;
use uuid::Uuid;

use super::*;
use crate::auth::microsoft::SignInResult;
use crate::error::ErrorCode;
use crate::model::{
    Account, AccountKind, AccountSettings, MicrosoftSettings, ReplyKind, Settings, SyncStatus,
};
use crate::secrets::MemorySecretStore;
use crate::service::tests::{FakeConnector, FakeServer};
use crate::service::{EventSink, MailAlert};
use crate::signatures::{CONSUMER_TENANT, CheckOutcome, OrganizationSignatures};
use crate::store::Store;

const SCOPE: &str =
    "api://signature.example.org/99999999-8888-7777-6666-555555555555/access_as_user";
const TENANT: &str = "11111111-2222-3333-4444-555555555555";

/// An organization whose answers the test scripts.
#[derive(Default)]
struct ScriptedOrganization {
    /// What the next checks find, in order; then a failure.
    outcomes: Mutex<VecDeque<CheckOutcome>>,
    /// The ETag each check sent.
    etags: Mutex<Vec<Option<String>>>,
    /// Each check takes this long.
    delay: Mutex<Option<StdDuration>>,
}

impl ScriptedOrganization {
    fn answer(&self, outcome: CheckOutcome) {
        self.outcomes.lock().unwrap().push_back(outcome);
    }

    fn calls(&self) -> usize {
        self.etags.lock().unwrap().len()
    }

    fn take_slow(&self, delay: StdDuration) {
        *self.delay.lock().unwrap() = Some(delay);
    }
}

#[async_trait]
impl OrganizationSignatures for ScriptedOrganization {
    fn scope(&self) -> &str {
        SCOPE
    }

    async fn check(&self, etag: Option<&str>) -> CheckOutcome {
        self.etags.lock().unwrap().push(etag.map(str::to_owned));
        let delay = *self.delay.lock().unwrap();
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
        self.outcomes
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(CheckOutcome::Failed {
                retry_after: None,
                reason: "nothing scripted".into(),
            })
    }
}

/// Records which accounts' signatures the UI was told changed.
#[derive(Default)]
struct Sink {
    signatures: Mutex<Vec<Option<String>>>,
}

impl Sink {
    fn count(&self) -> usize {
        self.signatures.lock().unwrap().len()
    }
}

impl EventSink for Sink {
    fn data_changed(&self, kinds: &[DataKind], account_id: Option<&str>) {
        if kinds.contains(&DataKind::Signatures) {
            self.signatures
                .lock()
                .unwrap()
                .push(account_id.map(str::to_owned));
        }
    }
    fn sync_status(&self, _: &SyncStatus) {}
    fn new_mail(&self, _: &Account, _: &MailAlert) {}
}

struct Setup {
    service: Arc<CoreService>,
    connector: Arc<FakeConnector>,
    organization: Arc<ScriptedOrganization>,
    sink: Arc<Sink>,
}

/// 2026-10-15 09:00 UTC, when the tests start.
fn epoch() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 15, 9, 0, 0).unwrap()
}

fn setup() -> Setup {
    let server = Arc::new(Mutex::new(FakeServer::default()));
    let connector = Arc::new(FakeConnector::new(server, false));
    let organization = Arc::new(ScriptedOrganization::default());
    *connector.organization_signatures.lock().unwrap() = Some(organization.clone());
    let store = Arc::new(Store::open_in_memory().unwrap());
    // Background work, when started, stays out of the way: no push, a
    // daily sync, no reminders.
    store
        .save_settings(&Settings {
            instant_push: false,
            sync_interval_minutes: 24 * 60,
            reminders_enabled: false,
            ..Settings::default()
        })
        .unwrap();
    let sink = Arc::new(Sink::default());
    let service = Arc::new(CoreService::new(
        store,
        Arc::new(MemorySecretStore::default()),
        connector.clone(),
        sink.clone(),
    ));
    // The wall clock follows Tokio's (paused) time.
    let origin = tokio::time::Instant::now();
    service.signatures.set_clock(Arc::new(move || {
        epoch() + Duration::from_std(tokio::time::Instant::now() - origin).unwrap()
    }));
    Setup {
        service,
        connector,
        organization,
        sink,
    }
}

fn microsoft_account(s: &Setup, tenant: &str) -> Account {
    let account = Account {
        id: Uuid::new_v4().to_string(),
        kind: AccountKind::Microsoft,
        display_name: "Adele".into(),
        email: "adele@contoso.com".into(),
        color: "#0b7a75".into(),
        settings: AccountSettings::Microsoft(MicrosoftSettings {
            tenant_id: tenant.into(),
            user_id: Uuid::new_v4().to_string(),
        }),
        created_at: Utc::now(),
        last_synced_at: None,
        sync_error: None,
    };
    s.service.store().insert_account(&account).unwrap();
    account
}

fn work_account(s: &Setup) -> Account {
    microsoft_account(s, TENANT)
}

fn imap_account(s: &Setup) -> Account {
    let account = crate::store::test_support::imap_account(&Uuid::new_v4().to_string());
    s.service.store().insert_account(&account).unwrap();
    account
}

fn base() -> Url {
    Url::parse("https://signature.example.org/").unwrap()
}

fn signature(html: &str, on_replies: bool, etag: &str) -> CheckOutcome {
    CheckOutcome::Signature {
        html: html.into(),
        on_replies,
        base: Some(base()),
        etag: Some(etag.into()),
    }
}

fn failure(retry_after: Option<StdDuration>) -> CheckOutcome {
    CheckOutcome::Failed {
        retry_after,
        reason: "offline".into(),
    }
}

async fn sleep(seconds: u64) {
    tokio::time::sleep(StdDuration::from_secs(seconds)).await;
}

/// Lets spawned checks run (on paused time, without moving it much).
async fn settle() {
    tokio::time::sleep(StdDuration::from_millis(10)).await;
}

impl Setup {
    fn organization_state(&self, account: &Account) -> OrganizationSignature {
        self.service
            .get_account_signature(&account.id)
            .unwrap()
            .organization
    }

    async fn compose(&self, account: &Account, reply: Option<ReplyKind>) -> ComposeSignature {
        self.service
            .get_compose_signature(&account.id, reply)
            .await
            .unwrap()
    }

    /// Keeps an organization answer as if a check had just given it.
    fn known(&self, account: &Account, state: OrganizationSignatureState, on_replies: bool) {
        let now = self.service.signatures.now();
        let stored = StoredOrganizationSignature {
            scope: Some(SCOPE.into()),
            state,
            html: (state == OrganizationSignatureState::Ready).then(|| "<p>Org</p>".into()),
            on_replies,
            etag: Some(format!("{}\"k\"", etag_prefix())),
            checked_at: Some(now),
            attempted_at: Some(now),
            stale: false,
            retry_at: None,
        };
        self.service
            .store()
            .save_organization_signature(&account.id, &stored)
            .unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn accounts_outside_organizations_never_reach_the_network() {
    let s = setup();
    let personal = microsoft_account(&s, CONSUMER_TENANT);
    let consumers = microsoft_account(&s, "consumers");
    let imap = imap_account(&s);
    for account in [&personal, &consumers, &imap] {
        s.service
            .update_own_signature(&account.id, "<p>Own</p>", true)
            .unwrap();
        assert_eq!(
            s.organization_state(account),
            OrganizationSignature {
                state: OrganizationSignatureState::Unsupported,
                html: None,
                on_replies: false,
                checked_at: None,
                stale: false,
            }
        );
        let checked = s
            .service
            .check_organization_signature(&account.id)
            .await
            .unwrap();
        assert_eq!(
            checked.organization.state,
            OrganizationSignatureState::Unsupported
        );
        assert_eq!(
            s.compose(account, None).await,
            ComposeSignature {
                kind: ComposeSignatureKind::Own,
                html: Some("<p>Own</p>".into()),
            }
        );
    }
    s.service.start_background();
    sleep(3600).await;
    assert_eq!(s.organization.calls(), 0);
    assert_eq!(s.connector.organization_requests.load(Ordering::SeqCst), 0);

    // A work account, in a build without the service's client ID: asked
    // once of the connector, never of the network.
    *s.connector.organization_signatures.lock().unwrap() = None;
    let work = work_account(&s);
    assert_eq!(
        s.organization_state(&work).state,
        OrganizationSignatureState::Unsupported
    );
    assert_eq!(s.compose(&work, None).await, ComposeSignature::none());
    sleep(3600).await;
    assert_eq!(s.connector.organization_requests.load(Ordering::SeqCst), 1);
    assert_eq!(s.organization.calls(), 0);
    s.service.shutdown().await;
}

#[tokio::test]
async fn own_signatures_are_sanitized_and_kept() {
    let s = setup();
    let account = imap_account(&s);
    let saved = s
        .service
        .update_own_signature(
            &account.id,
            r#"<p onclick="x()">Ada<script>x()</script></p><img src="/logo.png" alt="Logo">"#,
            false,
        )
        .unwrap();
    assert_eq!(saved.own_html, r#"<p>Ada</p><img alt="Logo">"#);
    assert!(!saved.own_on_replies);
    assert_eq!(s.sink.count(), 1);
    assert_eq!(s.service.get_account_signature(&account.id).unwrap(), saved);
    // The same again changes nothing.
    s.service
        .update_own_signature(&account.id, &saved.own_html, false)
        .unwrap();
    assert_eq!(s.sink.count(), 1);
    assert_eq!(
        *s.sink.signatures.lock().unwrap(),
        [Some(account.id.clone())]
    );

    // Too large: refused, and nothing changes.
    let big = format!("<p>{}</p>", "a".repeat(600 * 1024));
    let error = s
        .service
        .update_own_signature(&account.id, &big, true)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert_eq!(s.service.get_account_signature(&account.id).unwrap(), saved);

    // An editor's empty paragraph is no signature.
    let cleared = s
        .service
        .update_own_signature(&account.id, "<p><br></p>", true)
        .unwrap();
    assert_eq!(cleared.own_html, "");
    assert!(cleared.own_on_replies);
    assert_eq!(s.sink.count(), 2);

    let error = s
        .service
        .update_own_signature("missing", "<p>x</p>", true)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);
}

#[tokio::test(start_paused = true)]
async fn only_answers_change_what_is_known_of_the_organization() {
    let s = setup();
    let account = work_account(&s);
    assert_eq!(
        s.organization_state(&account).state,
        OrganizationSignatureState::Unchecked
    );

    s.organization.answer(signature(
        r#"<p>Ada<script>x()</script></p><a href="/about">About</a>"#,
        true,
        "\"1\"",
    ));
    let first = s
        .service
        .check_organization_signature(&account.id)
        .await
        .unwrap()
        .organization;
    let ready_html = r#"<p>Ada</p><a href="https://signature.example.org/about" rel="noopener noreferrer">About</a>"#;
    assert_eq!(
        first,
        OrganizationSignature {
            state: OrganizationSignatureState::Ready,
            html: Some(ready_html.into()),
            on_replies: true,
            checked_at: Some(epoch()),
            stale: false,
        }
    );
    assert_eq!(s.sink.count(), 1);

    // Asked again within the minute: not again.
    let again = s
        .service
        .check_organization_signature(&account.id)
        .await
        .unwrap();
    assert_eq!(again.organization, first);
    assert_eq!(s.organization.calls(), 1);

    // A failure keeps the signature, marked stale; it still goes on mail.
    sleep(61).await;
    s.organization.answer(failure(None));
    let failed = s
        .service
        .check_organization_signature(&account.id)
        .await
        .unwrap()
        .organization;
    assert_eq!(
        failed,
        OrganizationSignature {
            stale: true,
            ..first.clone()
        }
    );
    assert_eq!(s.sink.count(), 2);
    assert_eq!(
        s.compose(&account, None).await,
        ComposeSignature {
            kind: ComposeSignatureKind::Organization,
            html: Some(ready_html.into()),
        }
    );

    // "Not modified": the same signature, fresh again.
    sleep(61).await;
    s.organization.answer(CheckOutcome::Unchanged);
    let unchanged = s
        .service
        .check_organization_signature(&account.id)
        .await
        .unwrap()
        .organization;
    assert!(!unchanged.stale);
    assert_eq!(unchanged.html.as_deref(), Some(ready_html));
    assert!(unchanged.checked_at > first.checked_at);
    assert_eq!(s.sink.count(), 3);
    // Only when it was checked changed: the UI isn't told.
    sleep(61).await;
    s.organization.answer(CheckOutcome::Unchanged);
    s.service
        .check_organization_signature(&account.id)
        .await
        .unwrap();
    assert_eq!(s.sink.count(), 3);

    let steps = [
        (
            CheckOutcome::AddedOnSend {
                on_replies: true,
                etag: Some("\"2\"".into()),
            },
            OrganizationSignatureState::AddedOnSend,
            true,
        ),
        (
            CheckOutcome::NotUsed { etag: None },
            OrganizationSignatureState::NotUsed,
            false,
        ),
        (
            CheckOutcome::NotAllowed,
            OrganizationSignatureState::NotAllowed,
            false,
        ),
    ];
    for (outcome, state, on_replies) in steps {
        sleep(61).await;
        s.organization.answer(outcome);
        let now = s
            .service
            .check_organization_signature(&account.id)
            .await
            .unwrap()
            .organization;
        assert_eq!(now.state, state);
        assert_eq!(now.html, None);
        assert_eq!(now.on_replies, on_replies);
        assert!(!now.stale);
    }
    assert_eq!(s.sink.count(), 6);
    // Each check sent the last answer's ETag.
    assert_eq!(
        *s.organization.etags.lock().unwrap(),
        [
            None,
            Some("\"1\"".into()),
            Some("\"1\"".into()),
            Some("\"1\"".into()),
            Some("\"1\"".into()),
            Some("\"2\"".into()),
            None,
        ]
    );

    // The service asks to wait: it isn't asked before then.
    sleep(61).await;
    s.organization
        .answer(failure(Some(StdDuration::from_secs(10 * 60))));
    let busy = s
        .service
        .check_organization_signature(&account.id)
        .await
        .unwrap()
        .organization;
    assert_eq!(busy.state, OrganizationSignatureState::NotAllowed);
    assert!(busy.stale);
    sleep(5 * 60).await;
    s.service
        .check_organization_signature(&account.id)
        .await
        .unwrap();
    assert_eq!(s.organization.calls(), 8);
    sleep(5 * 60).await;
    s.organization.answer(CheckOutcome::NotAllowed);
    let after = s
        .service
        .check_organization_signature(&account.id)
        .await
        .unwrap()
        .organization;
    assert_eq!(s.organization.calls(), 9);
    assert!(!after.stale);
}

#[tokio::test]
async fn a_message_starts_with_the_signature_that_applies() {
    use ComposeSignatureKind as Kind;
    use OrganizationSignatureState as State;
    let s = setup();
    let organization = |kind| ComposeSignature {
        kind,
        html: Some("<p>Org</p>".into()),
    };
    let own = ComposeSignature {
        kind: Kind::Own,
        html: Some("<p>Own</p>".into()),
    };
    let added = ComposeSignature {
        kind: Kind::AddedOnSend,
        html: None,
    };
    let none = ComposeSignature::none();
    /// The organization's state and whether it signs replies, the own
    /// signature and whether it goes on replies, then what a new message
    /// and a reply start with.
    type Case = (
        Option<(State, bool)>,
        &'static str,
        bool,
        ComposeSignature,
        ComposeSignature,
    );
    let cases: Vec<Case> = vec![
        (
            Some((State::Ready, true)),
            "<p>Own</p>",
            true,
            organization(Kind::Organization),
            organization(Kind::Organization),
        ),
        (
            Some((State::Ready, false)),
            "<p>Own</p>",
            true,
            organization(Kind::Organization),
            none.clone(),
        ),
        (
            Some((State::AddedOnSend, true)),
            "<p>Own</p>",
            true,
            added.clone(),
            added.clone(),
        ),
        // Its mail server leaves replies and forwards unsigned: nothing
        // says it signs them, and the own signature isn't used either.
        (
            Some((State::AddedOnSend, false)),
            "<p>Own</p>",
            true,
            added.clone(),
            none.clone(),
        ),
        (
            Some((State::NotUsed, false)),
            "<p>Own</p>",
            true,
            own.clone(),
            own.clone(),
        ),
        (
            Some((State::NotUsed, false)),
            "<p>Own</p>",
            false,
            own.clone(),
            none.clone(),
        ),
        (
            Some((State::NotAllowed, false)),
            "<p>Own</p>",
            true,
            own.clone(),
            own.clone(),
        ),
        (
            Some((State::NotUsed, false)),
            "",
            true,
            none.clone(),
            none.clone(),
        ),
        // IMAP
        (None, "<p>Own</p>", false, own.clone(), none.clone()),
        (None, "", true, none.clone(), none.clone()),
    ];
    for (organization, own_html, own_on_replies, new_message, reply) in cases {
        let account = match organization {
            Some((state, on_replies)) => {
                let account = work_account(&s);
                s.known(&account, state, on_replies);
                account
            }
            None => imap_account(&s),
        };
        s.service
            .update_own_signature(&account.id, own_html, own_on_replies)
            .unwrap();
        let case = format!("{organization:?} {own_html:?} {own_on_replies}");
        assert_eq!(s.compose(&account, None).await, new_message, "{case}");
        for kind in [ReplyKind::Reply, ReplyKind::ReplyAll, ReplyKind::Forward] {
            assert_eq!(
                s.compose(&account, Some(kind)).await,
                reply,
                "{case} {kind:?}"
            );
        }
    }
    // Every answer was fresh: nothing was asked.
    assert_eq!(s.organization.calls(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_message_waits_four_seconds_at_most_for_the_first_answer() {
    let s = setup();
    let account = work_account(&s);
    s.service
        .update_own_signature(&account.id, "<p>Own</p>", true)
        .unwrap();
    s.organization.take_slow(StdDuration::from_secs(10));
    s.organization
        .answer(signature("<p>Org</p>", false, "\"1\""));
    let started = tokio::time::Instant::now();
    assert_eq!(
        s.compose(&account, None).await,
        ComposeSignature {
            kind: ComposeSignatureKind::Own,
            html: Some("<p>Own</p>".into()),
        }
    );
    assert_eq!(started.elapsed(), StdDuration::from_secs(4));
    // The check goes on, for the next message.
    sleep(7).await;
    assert_eq!(
        s.organization_state(&account).state,
        OrganizationSignatureState::Ready
    );
    let started = tokio::time::Instant::now();
    assert_eq!(
        s.compose(&account, None).await,
        ComposeSignature {
            kind: ComposeSignatureKind::Organization,
            html: Some("<p>Org</p>".into()),
        }
    );
    assert!(started.elapsed().is_zero());
    assert_eq!(s.organization.calls(), 1);

    // A quick answer is waited for, and a check in progress joined.
    let other = work_account(&s);
    s.organization.take_slow(StdDuration::from_secs(1));
    s.organization.answer(CheckOutcome::AddedOnSend {
        on_replies: true,
        etag: None,
    });
    let started = tokio::time::Instant::now();
    let (checked, composed) = tokio::join!(
        s.service.check_organization_signature(&other.id),
        s.compose(&other, Some(ReplyKind::Reply)),
    );
    assert_eq!(
        checked.unwrap().organization.state,
        OrganizationSignatureState::AddedOnSend
    );
    assert_eq!(composed.kind, ComposeSignatureKind::AddedOnSend);
    assert_eq!(started.elapsed(), StdDuration::from_secs(1));
    assert_eq!(s.organization.calls(), 2);
}

#[tokio::test(start_paused = true)]
async fn an_hour_old_answer_is_checked_again_in_the_background() {
    let s = setup();
    let account = work_account(&s);
    s.known(&account, OrganizationSignatureState::Ready, true);
    sleep(30 * 60).await;
    s.compose(&account, None).await;
    settle().await;
    assert_eq!(s.organization.calls(), 0);

    sleep(31 * 60).await;
    s.organization.take_slow(StdDuration::from_secs(10));
    s.organization.answer(CheckOutcome::Unchanged);
    let started = tokio::time::Instant::now();
    assert_eq!(
        s.compose(&account, Some(ReplyKind::Forward)).await.kind,
        ComposeSignatureKind::Organization
    );
    // Not waited for.
    assert!(started.elapsed().is_zero());
    settle().await;
    assert_eq!(s.organization.calls(), 1);
    sleep(11).await;
    let known = s.organization_state(&account);
    assert_eq!(
        known.checked_at,
        Some(epoch() + Duration::minutes(61) + Duration::seconds(10))
    );
    // Only when it was checked changed.
    assert_eq!(s.sink.count(), 0);
    s.compose(&account, None).await;
    settle().await;
    assert_eq!(s.organization.calls(), 1);
}

#[tokio::test(start_paused = true)]
async fn kept_signatures_follow_the_current_sanitizer() {
    let s = setup();
    let account = work_account(&s);
    // Kept by an earlier version, whose sanitizer let image-set through
    // and whose ETags had no prefix.
    let earlier =
        r#"<p style="background: image-set('http://t.example/p.png' 1x); color: red">Org</p>"#;
    let cleaned = r#"<p style="color: red">Org</p>"#;
    let stored = StoredOrganizationSignature {
        scope: Some(SCOPE.into()),
        state: OrganizationSignatureState::Ready,
        html: Some(earlier.into()),
        on_replies: true,
        etag: Some("\"v1\"".into()),
        checked_at: Some(epoch()),
        attempted_at: Some(epoch()),
        stale: false,
        retry_at: None,
    };
    let store = s.service.store();
    store
        .save_organization_signature(&account.id, &stored)
        .unwrap();
    store
        .save_own_signature(&account.id, earlier, true)
        .unwrap();

    // Read, both go through today's sanitizer.
    let read = s.service.get_account_signature(&account.id).unwrap();
    assert_eq!(read.organization.html.as_deref(), Some(cleaned));
    assert_eq!(read.own_html, cleaned);
    assert_eq!(
        s.compose(&account, Some(ReplyKind::Reply)).await,
        ComposeSignature {
            kind: ComposeSignatureKind::Organization,
            html: Some(cleaned.into()),
        }
    );
    let imap = imap_account(&s);
    store.save_own_signature(&imap.id, earlier, true).unwrap();
    assert_eq!(s.compose(&imap, None).await.html.as_deref(), Some(cleaned));

    // An ETag from another sanitizer isn't sent, so the answer comes in
    // full; a "not modified" without one is a failure, which keeps the
    // signature.
    s.organization.answer(CheckOutcome::Unchanged);
    let failed = s
        .service
        .check_organization_signature(&account.id)
        .await
        .unwrap()
        .organization;
    assert!(failed.stale);
    assert_eq!(failed.html.as_deref(), Some(cleaned));
    sleep(61).await;
    s.organization
        .answer(signature("<p>Org</p>", true, "\"v2\""));
    s.service
        .check_organization_signature(&account.id)
        .await
        .unwrap();
    assert_eq!(
        store
            .account_signatures(&account.id)
            .unwrap()
            .organization
            .etag,
        Some(format!("s{SIGNATURE_SANITIZER_VERSION}:\"v2\""))
    );
    // Kept by this one, it's sent back as the service gave it.
    sleep(61).await;
    s.organization.answer(CheckOutcome::Unchanged);
    let unchanged = s
        .service
        .check_organization_signature(&account.id)
        .await
        .unwrap()
        .organization;
    assert!(!unchanged.stale);
    assert_eq!(unchanged.html.as_deref(), Some("<p>Org</p>"));
    assert_eq!(
        *s.organization.etags.lock().unwrap(),
        [None, None, Some("\"v2\"".into())]
    );
}

#[tokio::test(start_paused = true)]
async fn one_check_at_a_time_and_one_a_minute() {
    let s = setup();
    let account = work_account(&s);
    s.organization.take_slow(StdDuration::from_secs(2));
    s.organization
        .answer(signature("<p>Org</p>", true, "\"1\""));
    let (a, b) = tokio::join!(
        s.service.check_organization_signature(&account.id),
        s.service.check_organization_signature(&account.id),
    );
    assert_eq!(a.unwrap(), b.unwrap());
    assert_eq!(s.organization.calls(), 1);
    s.service
        .check_organization_signature(&account.id)
        .await
        .unwrap();
    assert_eq!(s.organization.calls(), 1);
    sleep(60).await;
    s.service
        .check_organization_signature(&account.id)
        .await
        .unwrap();
    assert_eq!(s.organization.calls(), 2);
}

#[tokio::test]
async fn an_answer_for_another_service_is_not_used() {
    let s = setup();
    let account = work_account(&s);
    let stored = StoredOrganizationSignature {
        scope: Some("api://old.example.org/x/access_as_user".into()),
        state: OrganizationSignatureState::Ready,
        html: Some("<p>Old</p>".into()),
        on_replies: true,
        etag: Some("\"old\"".into()),
        checked_at: Some(epoch()),
        attempted_at: Some(epoch()),
        stale: false,
        retry_at: None,
    };
    s.service
        .store()
        .save_organization_signature(&account.id, &stored)
        .unwrap();
    assert_eq!(
        s.organization_state(&account).state,
        OrganizationSignatureState::Unchecked
    );
    s.organization.answer(CheckOutcome::NotUsed { etag: None });
    s.service
        .check_organization_signature(&account.id)
        .await
        .unwrap();
    // Its ETag wasn't sent.
    assert_eq!(*s.organization.etags.lock().unwrap(), [None]);
}

#[tokio::test(start_paused = true)]
async fn the_organization_is_asked_again_on_its_own_schedule() {
    let s = setup();
    let account = work_account(&s);
    let imap = imap_account(&s);
    s.organization
        .answer(signature("<p>Org</p>", true, "\"1\""));
    s.organization.answer(CheckOutcome::Unchanged);
    s.organization.answer(failure(None));
    s.organization.answer(CheckOutcome::Unchanged);
    s.organization.answer(CheckOutcome::NotUsed { etag: None });
    s.organization.answer(CheckOutcome::NotUsed { etag: None });
    let start = tokio::time::Instant::now();
    let at = |seconds: u64| tokio::time::sleep_until(start + StdDuration::from_secs(seconds));
    s.service.start_background();

    // About 15 seconds after the start...
    at(14).await;
    assert_eq!(s.organization.calls(), 0);
    at(16).await;
    assert_eq!(s.organization.calls(), 1);
    assert_eq!(
        s.organization_state(&account).state,
        OrganizationSignatureState::Ready
    );
    // ...then every 6 hours while the organization signs the mail...
    let second = 15 + 6 * 3600;
    at(second - 30).await;
    assert_eq!(s.organization.calls(), 1);
    at(second + 30).await;
    assert_eq!(s.organization.calls(), 2);
    let third = second + 6 * 3600;
    at(third + 30).await;
    assert_eq!(s.organization.calls(), 3);
    assert!(s.organization_state(&account).stale);
    // ...30 minutes after a failure...
    let fourth = third + 30 * 60;
    at(fourth - 30).await;
    assert_eq!(s.organization.calls(), 3);
    at(fourth + 30).await;
    assert_eq!(s.organization.calls(), 4);
    assert!(!s.organization_state(&account).stale);
    // ...and every 24 hours while it doesn't sign it.
    let fifth = fourth + 6 * 3600;
    at(fifth + 30).await;
    assert_eq!(s.organization.calls(), 5);
    assert_eq!(
        s.organization_state(&account).state,
        OrganizationSignatureState::NotUsed
    );
    let sixth = fifth + 24 * 3600;
    at(sixth - 30).await;
    assert_eq!(s.organization.calls(), 5);
    at(sixth + 30).await;
    assert_eq!(s.organization.calls(), 6);
    assert_eq!(
        s.organization_state(&account).checked_at,
        Some(epoch() + Duration::seconds(sixth as i64))
    );

    assert_eq!(
        s.service
            .get_account_signature(&imap.id)
            .unwrap()
            .organization
            .state,
        OrganizationSignatureState::Unsupported
    );
    s.service.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn failed_checks_go_again_when_the_network_is_back_or_the_service_says() {
    let s = setup();
    let account = work_account(&s);
    s.organization.answer(failure(None));
    s.service.start_background();
    sleep(16).await;
    assert_eq!(s.organization.calls(), 1);
    assert!(s.organization_state(&account).stale);

    // Offline, nothing is asked; back online, the failed check goes again.
    s.service.network_changed(false);
    sleep(40 * 60).await;
    assert_eq!(s.organization.calls(), 1);
    s.organization
        .answer(failure(Some(StdDuration::from_secs(2 * 3600))));
    s.service.network_changed(true);
    settle().await;
    assert_eq!(s.organization.calls(), 2);

    // The service asked to wait two hours: not 30 minutes.
    sleep(3600).await;
    assert_eq!(s.organization.calls(), 2);
    // Nor when the network comes back meanwhile.
    s.service.network_changed(false);
    s.service.network_changed(true);
    settle().await;
    assert_eq!(s.organization.calls(), 2);
    s.organization.answer(CheckOutcome::NotAllowed);
    sleep(3600 + 60).await;
    assert_eq!(s.organization.calls(), 3);
    assert_eq!(
        s.organization_state(&account),
        OrganizationSignature {
            state: OrganizationSignatureState::NotAllowed,
            html: None,
            on_replies: false,
            checked_at: Some(epoch() + Duration::seconds(16 + 40 * 60 + 2 * 3600)),
            stale: false,
        }
    );
    s.service.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn a_check_that_failed_just_before_the_network_came_back_goes_again() {
    let s = setup();
    let account = work_account(&s);
    s.organization.answer(failure(None));
    s.organization.answer(CheckOutcome::NotUsed { etag: None });
    s.service.start_background();
    sleep(16).await;
    assert_eq!(s.organization.calls(), 1);
    s.service.network_changed(false);
    sleep(10).await;
    s.service.network_changed(true);
    settle().await;
    // Not within the minute of the last...
    assert_eq!(s.organization.calls(), 1);
    // ...but once it's over, rather than 30 minutes after the failure.
    sleep(60).await;
    assert_eq!(s.organization.calls(), 2);
    assert_eq!(
        s.organization_state(&account).state,
        OrganizationSignatureState::NotUsed
    );
    s.service.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn a_computer_that_slept_checks_when_it_wakes() {
    let s = setup();
    let account = work_account(&s);
    // A clock that jumps when the test says the computer slept.
    let origin = tokio::time::Instant::now();
    let slept = Arc::new(Mutex::new(Duration::zero()));
    let jump = Arc::clone(&slept);
    s.service.signatures.set_clock(Arc::new(move || {
        epoch()
            + Duration::from_std(tokio::time::Instant::now() - origin).unwrap()
            + *jump.lock().unwrap()
    }));
    s.organization
        .answer(signature("<p>Org</p>", true, "\"1\""));
    s.organization.answer(CheckOutcome::Unchanged);
    s.service.start_background();
    sleep(16).await;
    assert_eq!(s.organization.calls(), 1);
    // Seven hours asleep, which Tokio's timers don't count.
    *slept.lock().unwrap() = Duration::hours(7);
    sleep(61).await;
    assert_eq!(s.organization.calls(), 2);
    assert!(s.organization_state(&account).checked_at > Some(epoch() + Duration::hours(7)));
    s.service.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn signing_in_asks_at_once() {
    let s = setup();
    s.organization
        .answer(signature("<p>Org</p>", false, "\"1\""));
    *s.connector.sign_in.lock().unwrap() = Some(SignInResult {
        email: "adele@contoso.com".into(),
        display_name: "Adele".into(),
        tenant_id: TENANT.into(),
        user_id: "33333333-0000-0000-0000-00000000000a".into(),
        access_token: "access".into(),
        refresh_token: "refresh".into(),
        expires_in: StdDuration::from_secs(3600),
        consent_refused: None,
    });
    let account = s
        .service
        .add_microsoft_account(&|_| Ok(()), None)
        .await
        .unwrap();
    settle().await;
    assert_eq!(s.organization.calls(), 1);
    assert_eq!(
        s.organization_state(&account).state,
        OrganizationSignatureState::Ready
    );
}

#[tokio::test(start_paused = true)]
async fn removing_an_account_stops_its_check_and_forgets_it() {
    let s = setup();
    let account = work_account(&s);
    s.service
        .update_own_signature(&account.id, "<p>Own</p>", true)
        .unwrap();
    s.organization.take_slow(StdDuration::from_secs(3600));
    s.organization
        .answer(signature("<p>Org</p>", true, "\"1\""));
    s.service.check_signature_soon(&account.id);
    settle().await;
    assert_eq!(s.organization.calls(), 1);
    assert!(s.service.signatures.in_flight(&account.id).is_some());

    s.service.remove_account(&account.id).await.unwrap();
    assert!(s.service.signatures.in_flight(&account.id).is_none());
    assert!(
        !s.service
            .signatures
            .services
            .lock()
            .unwrap()
            .contains_key(&account.id)
    );
    assert_eq!(
        s.service.store().account_signatures(&account.id).unwrap(),
        crate::store::StoredSignatures::default()
    );
    // The UI hears that signatures changed with the account.
    assert_eq!(s.sink.signatures.lock().unwrap().last(), Some(&None));
    sleep(2 * 3600).await;
    assert_eq!(
        s.service.store().account_signatures(&account.id).unwrap(),
        crate::store::StoredSignatures::default()
    );
    assert_eq!(s.organization.calls(), 1);
}

#[test]
fn the_next_check_follows_what_was_said() {
    use OrganizationSignatureState as State;
    let now = epoch();
    let answered = |state, hours_ago| StoredOrganizationSignature {
        scope: Some(SCOPE.into()),
        state,
        checked_at: Some(now - Duration::hours(hours_ago)),
        attempted_at: Some(now - Duration::hours(hours_ago)),
        ..Default::default()
    };
    // Not asked yet this run: now, whatever was said.
    assert_eq!(
        next_check(&answered(State::Ready, 1), false, false, now),
        now
    );
    assert_eq!(
        next_check(&answered(State::Ready, 1), true, false, now),
        now + Duration::hours(5)
    );
    assert_eq!(
        next_check(&answered(State::AddedOnSend, 2), true, false, now),
        now + Duration::hours(4)
    );
    assert_eq!(
        next_check(&answered(State::NotUsed, 1), true, false, now),
        now + Duration::hours(23)
    );
    assert_eq!(
        next_check(&answered(State::NotAllowed, 25), true, false, now),
        now - Duration::hours(1)
    );
    assert_eq!(
        next_check(&StoredOrganizationSignature::default(), true, false, now),
        now
    );
    let failed = StoredOrganizationSignature {
        stale: true,
        attempted_at: Some(now - Duration::minutes(10)),
        ..answered(State::Ready, 3)
    };
    assert_eq!(
        next_check(&failed, true, false, now),
        now + Duration::minutes(20)
    );
    // The network came back.
    assert_eq!(next_check(&failed, true, true, now), now);
    // The service said when: never before.
    let busy = StoredOrganizationSignature {
        retry_at: Some(now + Duration::hours(2)),
        ..failed
    };
    for (asked, retry) in [(false, false), (true, false), (true, true)] {
        assert_eq!(
            next_check(&busy, asked, retry, now),
            now + Duration::hours(2)
        );
    }
}
