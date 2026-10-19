//! Organization setup without a setup app (a build that names none): only
//! what is known without Microsoft and the script; every other call says it
//! isn't available.

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::*;
use crate::model::{Account, AccountKind, AccountSettings, MicrosoftSettings};
use crate::secrets::MemorySecretStore;
use crate::service::NullSink;
use crate::service::tests::{FakeConnector, FakeServer};
use crate::signatures::CONSUMER_TENANT;
use crate::store::Store;

const TENANT: &str = "85f67998-e79a-4173-95c2-90b0e6dc231d";

fn service() -> CoreService {
    let server = Arc::new(Mutex::new(FakeServer::default()));
    CoreService::new(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(MemorySecretStore::default()),
        Arc::new(FakeConnector::new(server, false)),
        Arc::new(NullSink),
    )
}

fn microsoft(service: &CoreService, tenant: &str, email: &str) -> Account {
    let account = Account {
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
    };
    service.store().insert_account(&account).unwrap();
    account
}

fn assert_code<T: std::fmt::Debug>(result: Result<T>, code: ErrorCode) {
    assert_eq!(result.unwrap_err().code, code);
}

#[tokio::test]
async fn without_a_setup_app_only_the_script_is_offered() {
    let service = service();
    let work = microsoft(&service, TENANT, "adele@contoso.com");
    assert_eq!(
        service.org_setup_info(&work.id).unwrap(),
        OrgSetupInfo {
            automatic: false,
            tenant_id: TENANT.into(),
            signed_in: None,
            approval_url: None,
            rules_changed_at: None,
            policies_changed_at: None,
        }
    );

    // When this computer last changed the organization, as applying
    // records it.
    let at = "2026-10-19T14:03:00Z".parse::<DateTime<Utc>>().unwrap();
    let key = org_setup_changed_key(TENANT);
    service
        .store()
        .set_app_value(&key, r#"{ "rulesChangedAt": "2026-10-19T14:03:00Z" }"#)
        .unwrap();
    let info = service.org_setup_info(&work.id).unwrap();
    assert_eq!(
        (info.rules_changed_at, info.policies_changed_at),
        (Some(at), None)
    );
    assert_eq!(
        service.settle(&TENANT.to_uppercase()),
        Settle {
            rules_changed_at: Some(at),
            policies_changed_at: None,
        }
    );
    // Unreadable: neither.
    service.store().set_app_value(&key, "not json").unwrap();
    assert_eq!(service.settle(TENANT), Settle::default());

    let open: &BrowserOpener = &|_| panic!("no browser without a setup app");
    let choices = OrgSetupChoices::default();
    let request = OrgSetupScriptRequest::default();
    assert_code(
        service.org_setup_sign_in(open, &work.id).await,
        ErrorCode::NotConfigured,
    );
    assert_code(
        service.read_org_setup(&work.id).await,
        ErrorCode::NotConfigured,
    );
    assert_code(
        service.plan_org_setup(&work.id, &choices).await,
        ErrorCode::NotConfigured,
    );
    assert_code(
        service.apply_org_setup(&work.id, &choices, &[]).await,
        ErrorCode::NotConfigured,
    );
    // The script works without a setup app.
    let script = service
        .org_setup_script(
            &work.id,
            &OrgSetupScriptRequest {
                actions: true,
                ..request
            },
        )
        .unwrap();
    assert!(script.script.contains("for contoso.com (tenant 85f67998-"));
    assert!(script.script.contains("New-TransportRule -Name $item.Name"));
    assert!(script.portal_steps.is_empty());
    // Nothing to end or cancel, and nothing logged yet.
    service.cancel_org_setup_sign_in();
    service.end_org_setup();
    assert_eq!(service.org_setup_details(), "");
    // People's check details need no setup app.
    assert!(
        service
            .protection_check_details(&work.id)
            .unwrap()
            .contains("setup check details for adele@contoso.com")
    );
}

#[tokio::test]
async fn only_work_accounts_have_an_organization_to_set_up() {
    let service = service();
    let personal = microsoft(&service, CONSUMER_TENANT, "adele@outlook.com");
    let imap = crate::store::test_support::imap_account("imap");
    service.store().insert_account(&imap).unwrap();
    let open: &BrowserOpener = &|_| panic!("no browser for these accounts");
    for account in [&personal, &imap] {
        let error = service.org_setup_info(&account.id).unwrap_err();
        assert_eq!(error.code, ErrorCode::Unsupported);
        assert_eq!(
            error.message,
            "Only Microsoft 365 work and school accounts have an organization to set up."
        );
        assert_code(
            service.org_setup_sign_in(open, &account.id).await,
            ErrorCode::Unsupported,
        );
        assert_code(
            service.read_org_setup(&account.id).await,
            ErrorCode::Unsupported,
        );
        assert_code(
            service.org_setup_script(&account.id, &OrgSetupScriptRequest::default()),
            ErrorCode::Unsupported,
        );
    }
    assert_code(service.org_setup_info("gone"), ErrorCode::NotFound);
    assert_code(
        service.protection_check_details("gone"),
        ErrorCode::NotFound,
    );
}

// --- With a setup app (a mock organization) ---------------------------------------

mod signed_in {
    use std::time::Duration;

    use wiremock::{MockServer, ResponseTemplate};

    use super::*;
    use crate::auth::admin_setup::{SETUP_APP_NAME, other_tenant};
    use crate::model::{OrgSetupOutcome, OrgSetupRowState};
    use crate::orgsetup::tests::tenant::FakeTenant;
    use crate::orgsetup::tests::{ADMIN, CLIENT, clock, endpoints};
    use crate::orgsetup::{IDLE_NOTE, IDLE_TICK};
    use crate::service::tests::RecordingSink;

    struct Setup {
        service: Arc<CoreService>,
        connector: Arc<FakeConnector>,
        sink: Arc<RecordingSink>,
        tenant: FakeTenant,
        _server: MockServer,
        work: Account,
    }

    /// A service whose connector names a setup app talking to a mock
    /// organization, and a work account in it.
    async fn setup() -> Setup {
        let server = MockServer::start().await;
        let tenant = FakeTenant::fresh();
        tenant.mount(&server).await;
        let connector = Arc::new(FakeConnector::new(
            Arc::new(Mutex::new(FakeServer::default())),
            false,
        ));
        *connector.admin_setup.lock().unwrap() = Some(endpoints(&server));
        let sink = Arc::new(RecordingSink::default());
        let service = Arc::new(CoreService::new(
            Arc::new(Store::open_in_memory().unwrap()),
            Arc::new(MemorySecretStore::default()),
            connector.clone(),
            sink.clone(),
        ));
        let work = microsoft(&service, TENANT, "adele@contoso.com");
        Setup {
            service,
            connector,
            sink,
            tenant,
            _server: server,
            work,
        }
    }

    fn no_browser() -> &'static BrowserOpener {
        &|_| panic!("the fake sign-in opens no browser")
    }

    fn session_slot(service: &CoreService) -> bool {
        service.org_setup.lock().unwrap().is_some()
    }

    #[tokio::test]
    async fn an_administrator_signs_in_reads_and_sets_up() {
        let Setup {
            service,
            sink,
            tenant,
            work,
            // Kept: a dropped mock server goes back to wiremock's pool,
            // and another test's organization would answer.
            _server,
            ..
        } = setup().await;
        let info = service.org_setup_info(&work.id).unwrap();
        assert!(info.automatic);
        assert_eq!(info.signed_in, None);
        assert_eq!(
            info.approval_url.as_deref(),
            Some(setup_admin_consent_url(CLIENT, Some(TENANT)).as_str())
        );

        let admin = service
            .org_setup_sign_in(no_browser(), &work.id)
            .await
            .unwrap();
        assert_eq!(admin.username, ADMIN);
        assert_eq!(
            service.org_setup_info(&work.id).unwrap().signed_in,
            Some(admin.clone())
        );

        let status = service.read_org_setup(&work.id).await.unwrap();
        assert_eq!(status.admin, admin);
        let state = |id: &str| status.rows.iter().find(|r| r.id == id).map(|r| r.state);
        assert_eq!(state("encryptRule"), Some(OrgSetupRowState::Missing));
        assert_eq!(state("setupApp"), Some(OrgSetupRowState::Info));
        assert!(status.exchange_readable && status.labels_readable && !status.any_ours);
        assert!(status.defaults.actions && status.defaults.label_ids.is_empty());

        // A plan from that read, then the changes it lists.
        let choices = status.defaults.clone();
        let plan = service.plan_org_setup(&work.id, &choices).await.unwrap();
        assert_eq!(plan, status.plan);
        let confirmed: Vec<String> = plan.changes.iter().map(|c| c.id.clone()).collect();
        assert_eq!(
            confirmed
                .iter()
                .map(|id| crate::orgsetup::tests::base_id(id))
                .collect::<Vec<_>>(),
            [
                "create:mailFlowRule:cumulas mail: encrypt",
                "create:mailFlowRule:cumulas mail: do not forward"
            ]
        );
        let before = sink.changes.lock().unwrap().len();
        let result = service
            .apply_org_setup(&work.id, &choices, &confirmed)
            .await
            .unwrap();
        assert!(!result.stale && !result.signed_out);
        assert!(
            result
                .changes
                .iter()
                .all(|c| c.outcome == OrgSetupOutcome::Done)
        );
        assert!(result.status.any_ours && result.status.plan.changes.is_empty());
        assert_eq!(tenant.rule_names().len(), 2);

        // Recorded for people's checks on this computer, and announced.
        let info = service.org_setup_info(&work.id).unwrap();
        assert!(info.rules_changed_at.is_some());
        assert_eq!(info.policies_changed_at, None);
        let announced = sink.changes.lock().unwrap()[before..]
            .iter()
            .any(|(kinds, _)| kinds.contains(&DataKind::Protection));
        assert!(announced);

        // A choice that names something that isn't a label is refused.
        let bad = OrgSetupChoices {
            label_ids: vec!["x'; Remove-TransportRule *".into()],
            ..choices.clone()
        };
        assert_code(
            service.plan_org_setup(&work.id, &bad).await,
            ErrorCode::InvalidInput,
        );

        // The script, signed in, follows the confirmed choices.
        let script = service
            .org_setup_script(
                &work.id,
                &OrgSetupScriptRequest {
                    choices: Some(choices),
                    actions: true,
                    labels: false,
                },
            )
            .unwrap();
        assert!(script.script.contains("Get-TemplateName 'Encrypt'"));

        // Ending it forgets the sign-in, and the details stay.
        service.end_org_setup();
        assert_eq!(service.org_setup_info(&work.id).unwrap().signed_in, None);
        assert_code(
            service.read_org_setup(&work.id).await,
            ErrorCode::AuthRequired,
        );
        let details = service.org_setup_details();
        assert!(details.contains(&format!("Tenant: {TENANT}")), "{details}");
        assert!(
            details.contains(&format!("Setup app: {SETUP_APP_NAME} ({CLIENT})")),
            "{details}"
        );
        assert!(
            details.contains("Signed in as: admin@contoso.com"),
            "{details}"
        );
        assert!(details.contains("code redeemed"), "{details}");
        assert!(
            details.contains("New-TransportRule (Name='Cumulas Mail: Encrypt'"),
            "{details}"
        );
        assert!(details.contains("Signed out."), "{details}");
        assert!(!details.contains("exchange-token"), "{details}");
    }

    #[tokio::test]
    async fn a_failed_sign_in_is_in_the_details_and_leaves_no_session() {
        let Setup {
            service,
            connector,
            work,
            // Kept: a dropped mock server goes back to wiremock's pool,
            // and another test's organization would answer.
            _server,
            ..
        } = setup().await;
        connector
            .admin_sign_in_failures
            .lock()
            .unwrap()
            .push_back(Error::new(
                ErrorCode::SignInCancelled,
                "Sign-in was canceled.",
            ));
        assert_code(
            service.org_setup_sign_in(no_browser(), &work.id).await,
            ErrorCode::SignInCancelled,
        );
        assert!(!session_slot(&service));
        let details = service.org_setup_details();
        assert!(details.contains("Sign-in: "), "{details}");
        assert!(details.contains("cancelled"), "{details}");
        assert!(details.contains("Signed in as: nobody yet"), "{details}");

        // Without a session, reading says the sign-in ended.
        let error = service.read_org_setup(&work.id).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::AuthRequired);
        assert_eq!(error.details.unwrap()["reason"], "setupSignedOut");
        assert_code(
            service
                .apply_org_setup(&work.id, &OrgSetupChoices::default(), &[])
                .await,
            ErrorCode::AuthRequired,
        );

        // Another organization's administrator: told with the account's
        // domain.
        connector
            .admin_sign_in_failures
            .lock()
            .unwrap()
            .push_back(other_tenant("your organization"));
        let error = service
            .org_setup_sign_in(no_browser(), &work.id)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidInput);
        assert_eq!(
            error.message,
            "You signed in with an account from another organization. Sign in with an \
             administrator account of contoso.com."
        );
        assert_eq!(error.details.unwrap()["reason"], "otherTenant");
    }

    #[tokio::test]
    async fn an_hour_unused_signs_out_even_while_the_computer_slept() {
        let Setup {
            service,
            work,
            _server,
            ..
        } = setup().await;
        let (now, wall) = clock();
        service
            .org_setup_sign_in_with(no_browser(), &work.id, wall, Duration::from_millis(20))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(session_slot(&service), "signed out too early");
        let store = service
            .org_setup
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .token_store();

        // The computer slept for an hour: Instant stood still, the wall
        // clock didn't. The watcher takes the session out by itself.
        *now.lock().unwrap() += Duration::from_secs(61 * 60);
        tokio::time::timeout(Duration::from_secs(5), async {
            while session_slot(&service) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the watcher signs the session out");
        assert!(store.upgrade().is_none(), "the tokens went with it");
        assert!(service.org_setup_details().contains(IDLE_NOTE));
        assert_eq!(service.org_setup_info(&work.id).unwrap().signed_in, None);
        assert_eq!(IDLE_TICK, Duration::from_secs(60));
    }

    #[tokio::test]
    async fn ending_setup_during_a_read_stops_it_and_drops_the_tokens() {
        let Setup {
            service,
            tenant,
            work,
            // Kept: a dropped mock server goes back to wiremock's pool,
            // and another test's organization would answer.
            _server,
            ..
        } = setup().await;
        service
            .org_setup_sign_in(no_browser(), &work.id)
            .await
            .unwrap();
        tenant.script(
            "Get-TransportRule",
            ResponseTemplate::new(200).set_delay(Duration::from_secs(10)),
        );
        let store = service
            .org_setup
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .token_store();
        let reading = tokio::spawn({
            let (service, id) = (Arc::clone(&service), work.id.clone());
            async move { service.read_org_setup(&id).await }
        });
        while !tenant
            .calls()
            .iter()
            .any(|c| c.cmdlet == "Get-TransportRule")
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // While the read holds the session's calls, these answer at once.
        let info = service.org_setup_info(&work.id).unwrap();
        assert_eq!(info.signed_in.map(|a| a.username).as_deref(), Some(ADMIN));
        assert!(
            service
                .org_setup_details()
                .contains("Read accepted domains")
        );

        let started = std::time::Instant::now();
        service.end_org_setup();
        let read = tokio::time::timeout(Duration::from_secs(5), reading)
            .await
            .expect("the read ends at once")
            .unwrap();
        assert!(started.elapsed() < Duration::from_secs(5));
        let error = read.unwrap_err();
        assert_eq!(error.code, ErrorCode::AuthRequired);
        assert_eq!(error.details.unwrap()["reason"], "setupSignedOut");
        assert!(
            store.upgrade().is_none(),
            "the tokens went with the session"
        );
        assert!(!session_slot(&service));
    }

    #[tokio::test]
    async fn a_refused_refresh_ends_the_session() {
        let Setup {
            service,
            tenant,
            work,
            // Kept: a dropped mock server goes back to wiremock's pool,
            // and another test's organization would answer.
            _server,
            ..
        } = setup().await;
        service
            .org_setup_sign_in(no_browser(), &work.id)
            .await
            .unwrap();
        tenant.script(
            "token",
            ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": "interaction_required",
                "error_description": "AADSTS50133: Session is invalid due to expiration or recent password change.",
                "error_codes": [50133],
            })),
        );
        let error = service.read_org_setup(&work.id).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::AuthRequired);
        assert_eq!(error.details.unwrap()["reason"], "setupSignedOut");
        // Signed out, so Read again doesn't try the dead sign-in again.
        assert!(!session_slot(&service));
        assert_eq!(service.org_setup_info(&work.id).unwrap().signed_in, None);
        assert!(
            service
                .org_setup_details()
                .contains("Signed out: Microsoft 365 didn't accept the sign-in any more.")
        );
    }

    #[tokio::test]
    async fn an_apply_that_ends_signed_out_ends_the_session() {
        let Setup {
            service,
            tenant,
            work,
            _server,
            ..
        } = setup().await;
        service
            .org_setup_sign_in(no_browser(), &work.id)
            .await
            .unwrap();
        let status = service.read_org_setup(&work.id).await.unwrap();
        let confirmed: Vec<String> = status.plan.changes.iter().map(|c| c.id.clone()).collect();
        // Refused again after the token's one refresh.
        for _ in 0..2 {
            tenant.script(
                "New-TransportRule",
                ResponseTemplate::new(401).set_body_json(serde_json::json!({
                    "error": { "code": "Unauthorized", "message": "Access denied." }
                })),
            );
        }
        let result = service
            .apply_org_setup(&work.id, &status.defaults, &confirmed)
            .await
            .unwrap();
        assert!(result.signed_out);
        // As the result says: sign in again (the demo does the same).
        assert!(!session_slot(&service));
        assert_code(
            service.read_org_setup(&work.id).await,
            ErrorCode::AuthRequired,
        );
    }

    #[tokio::test]
    async fn a_new_sign_in_replaces_the_session_and_starts_a_new_log() {
        let Setup {
            service,
            work,
            _server,
            ..
        } = setup().await;
        service
            .org_setup_sign_in(no_browser(), &work.id)
            .await
            .unwrap();
        service.read_org_setup(&work.id).await.unwrap();
        let first = service.org_setup.lock().unwrap().clone().unwrap();
        service
            .org_setup_sign_in(no_browser(), &work.id)
            .await
            .unwrap();
        assert!(first.is_cancelled());
        let second = service.org_setup.lock().unwrap().clone().unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        // The new run's log starts afresh.
        let details = service.org_setup_details();
        assert_eq!(details.matches("Sign-in: ").count(), 1, "{details}");
        assert!(!details.contains("Read mail flow rules"), "{details}");
        // A plan needs a read in this session first.
        assert_code(
            service
                .plan_org_setup(&work.id, &OrgSetupChoices::default())
                .await,
            ErrorCode::InvalidInput,
        );
    }
}
