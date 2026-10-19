//! "Copy setup details".

use serde_json::json;

use super::tenant::{FakeTenant, rule};
use super::{CLIENT, TENANT, snapshot_of};
use crate::auth::admin_setup::SETUP_APP_NAME;
use crate::orgsetup::invoke::ErrorDetail;
use crate::orgsetup::runlog::{CallRecord, Entry, RunHeader, RunLog, SignInRecord, details_text};
use crate::orgsetup::status::snapshot_summary;
use crate::orgsetup::wanted::ENCRYPT_RULE;

fn header() -> RunHeader {
    RunHeader {
        tenant_id: Some(TENANT.into()),
        client_id: Some(CLIENT.into()),
        admin: None,
    }
}

#[test]
fn nothing_logged_gives_no_text() {
    assert_eq!(details_text(&RunLog::default()), "");
}

#[test]
fn the_text_names_the_run_and_every_attempt() {
    let log = RunLog::default();
    log.start(header());
    let scope = "openid profile offline_access https://outlook.office365.com/Exchange.Manage";
    log.sign_in(
        SignInRecord::new(format!("login.microsoftonline.com/{TENANT}"), scope).refused(
            "consent_required",
            vec![65001],
            "AADSTS65001: The user or administrator has not consented to use the application.",
        ),
    );
    log.sign_in(
        SignInRecord::new(format!("login.microsoftonline.com/{TENANT}"), scope).redeemed(
            Some("https://outlook.office365.com/Exchange.Manage"),
            true,
            true,
        ),
    );
    log.set_admin("admin@contoso.com");
    log.note(
        "Security & Compliance token: https://ps.compliance.protection.outlook.com/.default issued",
    );
    let mut call = CallRecord::new(
        "Create “Cumulas Mail: Encrypt”",
        "outlook.office365.com",
        "New-TransportRule",
    );
    call.parameters = vec![
        ("Name".into(), Some("'Cumulas Mail: Encrypt'".into())),
        ("ResultSize".into(), None),
    ];
    call.anchor = Some("UPN");
    call.attempts = vec![(Some("UPN"), Some(404)), (Some("SystemMailbox"), Some(400))];
    call.status = Some(400);
    call.ms = 220;
    call.error = Some(ErrorDetail {
        status: Some(400),
        code: Some("BadRequest".into()),
        exception_type: Some("Microsoft.Exchange.Exception".into()),
        message: "Something".into(),
        excerpt: "{\"error\":{}}".into(),
    });
    log.call(call);
    let text = details_text(&log);
    let version = env!("CARGO_PKG_VERSION");
    assert!(text.starts_with(&format!("Cumulas Mail {version}: organization setup details\nThese details include no passwords or tokens.")), "{text}");
    assert!(text.contains(&format!("Tenant: {TENANT}\nSetup app: {SETUP_APP_NAME} ({CLIENT})\nSigned in as: admin@contoso.com\n")), "{text}");
    assert!(text.contains(&format!(
        "Sign-in: login.microsoftonline.com/{TENANT}, scope \"{scope}\""
    )));
    assert!(text.contains("  error · consent_required · AADSTS65001 · \"AADSTS65001: The user or administrator has not consented to use the application.\""), "{text}");
    assert!(text.contains("  code redeemed · granted \"https://outlook.office365.com/Exchange.Manage\" · ID token yes · refresh token yes"), "{text}");
    assert!(text.contains("UTC  Security & Compliance token: https://ps.compliance.protection.outlook.com/.default issued"));
    assert!(text.contains("UTC  Create “Cumulas Mail: Encrypt”\n  New-TransportRule (Name='Cumulas Mail: Encrypt', ResultSize) on outlook.office365.com · anchor UPN · 400 · 220 ms\n  Attempts: UPN 404, SystemMailbox 400\n  Error: BadRequest · Microsoft.Exchange.Exception · \"Something\"\n  Body: {\"error\":{}}\n"), "{text}");

    // Kept after the session ends; replaced only at the next sign-in.
    assert_eq!(details_text(&log), text);
    log.start(header());
    let fresh = details_text(&log);
    assert!(fresh.contains("Signed in as: nobody yet"));
    assert!(!fresh.contains("Sign-in:"));
}

#[test]
fn control_characters_never_get_in() {
    let log = RunLog::default();
    log.start(header());
    log.note("bad\u{202e}text\u{0007}");
    log.set_admin("ad\u{200f}min@contoso.com\n");
    let text = details_text(&log);
    assert!(text.contains("UTC  badtext"), "{text}");
    assert!(text.contains("Signed in as: admin@contoso.com\n"), "{text}");
    assert!(!text.contains('\u{202e}') && !text.contains('\u{7}'));
}

#[test]
fn other_rules_appear_by_name_and_order_only_and_ours_in_full_but_capped() {
    let tenant = FakeTenant::fresh();
    let long = "y".repeat(5000);
    tenant.with(|s| {
        s.rules = vec![
            rule("Contoso: stop for partner.com", json!({ "StopRuleProcessing": true, "RecipientDomainIs": ["partner.example"], "Comments": "private note" })),
            rule(ENCRYPT_RULE, json!({ "Comments": long, "RemoveHeader": "x-cumulas-protect" })),
        ]
    });
    let summary = snapshot_summary(&snapshot_of(&tenant));
    assert!(
        summary
            .contains("  0  Contoso: stop for partner.com · Enabled · Enforce · stops later rules"),
        "{summary}"
    );
    assert!(
        !summary.contains("partner.example") && !summary.contains("private note"),
        "{summary}"
    );
    assert!(summary.contains(
        "Cumulas Mail's objects as Microsoft 365 returned them:\n  Cumulas Mail: Encrypt\n"
    ));
    let mut lines = summary
        .lines()
        .skip_while(|l| *l != "  Cumulas Mail: Encrypt");
    lines.next();
    let raw = lines.next().unwrap();
    assert!(raw.trim().starts_with('{'), "{raw}");
    assert_eq!(raw.trim().chars().count(), 4000);
    assert!(summary.contains("Labels: Confidential (Guid"), "{summary}");

    // In the run log, it's one entry.
    let log = RunLog::default();
    log.snapshot(summary);
    assert!(matches!(log.entries()[0], Entry::Snapshot { .. }));
    let text = details_text(&log);
    assert!(
        text.contains("What Microsoft 365 returned:\n  Initial domain: contoso.onmicrosoft.com\n  Message Encryption:"),
        "{text}"
    );
}

#[test]
fn the_log_keeps_the_latest_five_hundred() {
    let log = RunLog::default();
    log.start(header());
    for i in 0..510 {
        log.note(format!("note {i}"));
    }
    assert_eq!(log.entries().len(), 500);
    let text = details_text(&log);
    assert!(text.contains("(10 earlier entries were left out.)"));
    assert!(!text.contains("note 9\n") && text.contains("note 509"));
}
