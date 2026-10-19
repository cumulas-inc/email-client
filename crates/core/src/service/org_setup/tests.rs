//! Organization setup as this build has it: no setup app yet, so only what
//! is known without Microsoft, and every other call says it isn't available.

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
    assert_code(
        service.org_setup_script(&work.id, &request),
        ErrorCode::NotConfigured,
    );
    // Nothing to end or cancel, and nothing logged yet.
    service.cancel_org_setup_sign_in();
    service.end_org_setup();
    assert_eq!(service.org_setup_details(), "");
    assert_eq!(service.protection_check_details(&work.id).unwrap(), "");
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
