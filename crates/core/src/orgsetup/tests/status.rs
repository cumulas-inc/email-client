//! The status rows, on modeled organizations (no network).

use chrono::{Duration, Utc};
use serde_json::json;

use super::tenant::{FakeTenant, rule};
use super::{ADMIN, ADMIN_OID, CLIENT, CONFIDENTIAL, GENERAL, SECRET, ctx, snapshot_of};
use crate::auth::admin_setup::SETUP_APP_NAME;
use crate::model::{
    AccountFeature, LicenseHintState, OrgSetupAdmin, OrgSetupFix, OrgSetupRow, OrgSetupRowState,
    OrgSetupSection, OrgSetupStatus, RelayPath,
};
use crate::orgsetup::invoke::ErrorDetail;
use crate::orgsetup::status::{
    FailureKind, Part, ReadFailure, SessionInfo, SetupContext, Snapshot, status,
};
use crate::orgsetup::wanted::{DNF_RULE, ENCRYPT_RULE, RELAY_INBOUND, RELAY_RULE, STRIP_RULE};
use crate::providers::graph::LicenseHint;

fn info() -> SessionInfo {
    SessionInfo {
        admin: OrgSetupAdmin {
            username: ADMIN.into(),
            display_name: "Admin".into(),
        },
        client_id: CLIENT.into(),
        admin_object_id: ADMIN_OID.into(),
    }
}

fn of(snapshot: &Snapshot, ctx: &SetupContext) -> OrgSetupStatus {
    status(snapshot, ctx, &info())
}

fn row<'s>(status: &'s OrgSetupStatus, id: &str) -> &'s OrgSetupRow {
    status.rows.iter().find(|r| r.id == id).unwrap_or_else(|| {
        panic!(
            "no row {id}: {:?}",
            status.rows.iter().map(|r| &r.id).collect::<Vec<_>>()
        )
    })
}

fn has_row(status: &OrgSetupStatus, id: &str) -> bool {
    status.rows.iter().any(|r| r.id == id)
}

fn failure(kind: FailureKind, message: &str) -> ReadFailure {
    ReadFailure {
        kind,
        detail: ErrorDetail {
            message: message.into(),
            ..ErrorDetail::default()
        },
    }
}

#[test]
fn a_fresh_organization_reads_as_not_set_up() {
    let status = of(&snapshot_of(&FakeTenant::fresh()), &ctx());
    let summary = |id| (row(&status, id).state, row(&status, id).summary.clone());
    assert_eq!(
        summary("approval"),
        (OrgSetupRowState::Ok, "Approved for adele@contoso.com. Cumulas Mail can't see whether it's approved for everyone.".into())
    );
    assert_eq!(row(&status, "approval").fix, Some(OrgSetupFix::Approval));
    let setup_app = row(&status, "setupApp");
    assert_eq!(setup_app.title, SETUP_APP_NAME);
    assert_eq!(setup_app.state, OrgSetupRowState::Info);
    assert_eq!(
        setup_app.summary,
        format!(
            "Signed in as {ADMIN}. Cumulas Mail forgets this sign-in when you close this window. Microsoft keeps your approval of {SETUP_APP_NAME} until you remove it."
        )
    );
    let detail = setup_app.detail.as_deref().unwrap();
    assert!(
        detail.contains(&format!(
            "$sp = Get-MgServicePrincipal -Filter \"appId eq '{CLIENT}'\""
        )),
        "{detail}"
    );
    assert!(
        detail.contains(&format!("principalId eq '{ADMIN_OID}'")),
        "{detail}"
    );
    // An approval for everyone has no principal: the command counts it, and
    // the next one removes it.
    assert!(
        detail.contains(
            "$everyone = @(Get-MgOauth2PermissionGrant -Filter \"clientId eq '$($sp.Id)' and consentType eq 'AllPrincipals'\")"
        ),
        "{detail}"
    );
    assert!(
        detail.contains(
            "Get-MgOauth2PermissionGrant -Filter \"clientId eq '$($sp.Id)' and consentType eq 'AllPrincipals'\" |\n  ForEach-Object { Remove-MgOauth2PermissionGrant -OAuth2PermissionGrantId $_.Id }"
        ),
        "{detail}"
    );
    // Each command is its own paragraph, as the dialog shows them.
    let paragraphs: Vec<&str> = detail.split("\n\n").collect();
    assert_eq!(paragraphs.len(), 5, "{detail}");
    assert!(paragraphs[1].starts_with("Connect-MgGraph"), "{detail}");
    assert!(
        paragraphs[3].starts_with("Get-MgOauth2PermissionGrant"),
        "{detail}"
    );
    // Only GUIDs go into its filter.
    let odd = crate::orgsetup::status::remove_approval(CLIENT, "x' or 1 eq 1 or 'a");
    assert!(odd.contains("principalId eq '<your object ID>'"), "{odd}");
    assert!(!odd.contains("1 eq 1"), "{odd}");
    assert_eq!(
        summary("irm"),
        (
            OrgSetupRowState::Ok,
            "Microsoft Purview Message Encryption is on.".into()
        )
    );
    assert_eq!(
        summary("templates"),
        (
            OrgSetupRowState::Ok,
            "Templates: Encrypt and Do Not Forward.".into()
        )
    );
    for id in ["encryptRule", "dnfRule", "stripRule"] {
        assert_eq!(
            summary(id),
            (OrgSetupRowState::Missing, "Not set up.".into()),
            "{id}"
        );
        assert_eq!(row(&status, id).fix, Some(OrgSetupFix::Setup));
    }
    assert_eq!(row(&status, "encryptRule").title, "Encrypt rule");
    assert_eq!(
        row(&status, "encryptRule").section,
        OrgSetupSection::Encryption
    );
    assert_eq!(row(&status, "stripRule").section, OrgSetupSection::Labels);
    assert_eq!(
        summary("licensing"),
        (
            OrgSetupRowState::Ok,
            "adele@contoso.com's license includes automatic labeling.".into()
        )
    );
    for id in [CONFIDENTIAL, GENERAL, SECRET] {
        let label = row(&status, &format!("label:{id}"));
        assert_eq!(
            (label.state, label.summary.as_str()),
            (OrgSetupRowState::Missing, "Not set up.")
        );
        assert_eq!(
            label.detail.as_deref(),
            Some(format!("Matched by Guid (header value {id}).").as_str())
        );
    }
    assert_eq!(
        row(&status, &format!("label:{CONFIDENTIAL}")).title,
        "Confidential"
    );
    assert_eq!(
        summary("routing"),
        (
            OrgSetupRowState::Ok,
            "Mail doesn't go through the Cumulas Signatures relay.".into()
        )
    );
    assert_eq!(status.relay, RelayPath::None);
    assert_eq!(status.license, LicenseHintState::Included);
    assert!(
        status.exchange_readable && status.labels_readable && !status.no_role && !status.any_ours
    );
    // Labels start unchecked; Encrypt and Do Not Forward with the usual
    // templates.
    assert!(status.defaults.actions);
    assert!(status.defaults.label_ids.is_empty());
    assert_eq!(status.defaults.encrypt_template.as_deref(), Some("Encrypt"));
    assert_eq!(
        status.defaults.do_not_forward_template.as_deref(),
        Some("Do Not Forward")
    );
    assert_eq!(status.plan.changes.len(), 2);
    assert_eq!(status.labels.len(), 3);
    assert!(status.labels[0].in_catalog && status.labels[0].encrypts);
    assert_eq!(status.labels[0].for_email, Some(true));
    assert_eq!(status.templates.len(), 2);
}

#[test]
fn each_half_that_couldnt_be_read_gets_its_row() {
    let tenant = FakeTenant::fresh();
    let mut snapshot = snapshot_of(&tenant);
    snapshot.irm = Part::Failed(failure(FailureKind::NoRole, "denied"));
    snapshot.templates = Part::Skipped;
    snapshot.rules = Part::Skipped;
    let status = of(&snapshot, &ctx());
    let exchange = row(&status, "exchangeAccess");
    assert_eq!(exchange.state, OrgSetupRowState::Blocked);
    assert_eq!(
        exchange.summary,
        "Your account can't read mail flow rules. Sign in as an Exchange Administrator or Global Administrator."
    );
    assert_eq!(exchange.fix, Some(OrgSetupFix::SignIn));
    assert!(!has_row(&status, "encryptRule") && !has_row(&status, "routing"));
    assert!(!status.exchange_readable && !status.no_role);
    assert!(!status.defaults.actions);

    snapshot.compliance = Part::Failed(failure(FailureKind::NoRole, "denied"));
    let status = of(&snapshot, &ctx());
    assert_eq!(
        row(&status, "labelsAccess").summary,
        "Your account can't read auto-labeling policies. Sign in as a Compliance Administrator or Global Administrator."
    );
    assert!(status.no_role);
    assert!(!has_row(&status, &format!("label:{CONFIDENTIAL}")));

    snapshot.compliance = Part::Failed(failure(FailureKind::NotAccepted, "x"));
    let status = of(&snapshot, &ctx());
    assert_eq!(
        row(&status, "labelsAccess").summary,
        format!(
            "Microsoft 365 didn't accept the sign-in for {SETUP_APP_NAME}. Use the PowerShell script, and send Copy setup details to Cumulas."
        )
    );
    snapshot.compliance = Part::Failed(failure(FailureKind::Unreachable, "connection reset"));
    let status = of(&snapshot, &ctx());
    let labels = row(&status, "labelsAccess");
    assert_eq!(labels.state, OrgSetupRowState::Unknown);
    assert_eq!(
        labels.summary,
        "Couldn't reach Security & Compliance (connection reset)."
    );
    snapshot.irm = Part::Failed(failure(FailureKind::Other, "Something broke."));
    let status = of(&snapshot, &ctx());
    assert_eq!(
        row(&status, "exchangeAccess").summary,
        "Couldn't read mail flow rules: Something broke."
    );
    assert_eq!(
        row(&status, "exchangeAccess").fix,
        Some(OrgSetupFix::Manual)
    );
    // Mail flow rules read, then the connectors didn't: worded after them.
    let mut snapshot = snapshot_of(&tenant);
    snapshot.inbound = Part::Failed(failure(FailureKind::NoRole, "denied"));
    snapshot.outbound = Part::Skipped;
    let status = of(&snapshot, &ctx());
    assert_eq!(
        row(&status, "exchangeAccess").summary,
        "Your account can't read connectors, so Cumulas Mail can't tell how mail is routed or change mail flow rules. Sign in as an Exchange Administrator or Global Administrator."
    );
    assert!(has_row(&status, "encryptRule"));
    snapshot.inbound = Part::Failed(failure(FailureKind::Other, "Something broke."));
    let status = of(&snapshot, &ctx());
    assert_eq!(
        row(&status, "exchangeAccess").summary,
        "Couldn't read connectors, so mail flow rules aren't changed: Something broke."
    );
}

#[test]
fn message_encryption_off_says_what_to_run() {
    let tenant = FakeTenant::fresh();
    tenant.with(|s| {
        s.irm = json!({ "AzureRMSLicensingEnabled": false, "InternalLicensingEnabled": true })
    });
    let mut ctx = ctx();
    ctx.account_email = "o'brien@contoso.com".into();
    let status = of(&snapshot_of(&tenant), &ctx);
    let irm = row(&status, "irm");
    assert_eq!(irm.state, OrgSetupRowState::Blocked);
    assert_eq!(
        irm.summary,
        "Message Encryption is off in Exchange Online, so nothing can be encrypted."
    );
    assert_eq!(
        irm.detail.as_deref(),
        Some(
            "Cumulas Mail doesn't change this organization-wide setting. In Exchange Online PowerShell: Set-IRMConfiguration -AzureRMSLicensingEnabled $true -InternalLicensingEnabled $true, then Test-IRMConfiguration -Sender 'o''brien@contoso.com' -Recipient 'o''brien@contoso.com' must end with OVERALL RESULT: PASS."
        )
    );
    assert!(!status.defaults.actions);
    // Templates: one missing, or none.
    tenant.with(|s| s.templates.truncate(1));
    let status = of(&snapshot_of(&tenant), &ctx);
    assert_eq!(
        row(&status, "templates").summary,
        "Choose the template for Do Not Forward."
    );
    tenant.with(|s| s.templates.clear());
    let status = of(&snapshot_of(&tenant), &ctx);
    assert_eq!(
        row(&status, "templates").summary,
        "Exchange lists no rights protection templates."
    );
}

#[test]
fn rule_rows_say_what_needs_repair() {
    let tenant = FakeTenant::fresh();
    let mut dnf = rule(
        DNF_RULE,
        json!({
            "ExceptIfFromScope": "NotInOrganization", "HeaderMatchesMessageHeader": "x-cumulas-protect",
            "HeaderMatchesPatterns": ["^do-not-forward$"], "ApplyRightsProtectionTemplate": "Do Not Forward",
            "RemoveHeader": "x-cumulas-protect", "RuleErrorAction": "Defer", "Mode": "AuditAndNotify",
        }),
    );
    dnf["WhenChanged"] = json!(Utc::now().to_rfc3339());
    let strip = rule(
        STRIP_RULE,
        json!({
            "ExceptIfFromScope": "InOrganization", "RemoveHeader": "x-cumulas-label",
            "WhenChanged": (Utc::now() - Duration::minutes(10)).to_rfc3339(),
        }),
    );
    tenant.with(|s| {
        s.rules = vec![
            rule(
                "Contoso: stop",
                json!({ "StopRuleProcessing": true, "BlindCopyTo": ["audit@contoso.com"] }),
            ),
            rule(
                ENCRYPT_RULE,
                json!({ "FromScope": "InOrganization", "BlindCopyTo": ["x@evil.example"] }),
            ),
            dnf,
            strip,
        ]
    });
    let status = of(&snapshot_of(&tenant), &ctx());
    let encrypt = row(&status, "encryptRule");
    assert_eq!(encrypt.state, OrgSetupRowState::Blocked);
    assert_eq!(
        encrypt.summary,
        "It has settings Cumulas Mail doesn't set: BlindCopyTo. Remove them in the Exchange admin center (or delete the rule), then select Read again."
    );
    assert_eq!(
        encrypt.detail.as_deref(),
        Some("BlindCopyTo: x@evil.example")
    );
    assert_eq!(encrypt.fix, Some(OrgSetupFix::Manual));
    // Misplaced comes before the test mode: a placement difference.
    let dnf = row(&status, "dnfRule");
    assert_eq!(dnf.state, OrgSetupRowState::Differs);
    assert_eq!(
        dnf.summary,
        "Needs repair: moves above “Contoso: stop”, which stops later rules for mail it matches, so it acts on that mail first."
    );
    let strip = row(&status, "stripRule");
    assert_eq!(strip.summary, "Needs repair: moves to the top.");

    // In place, in test mode: off.
    tenant.with(|s| {
        s.rules.remove(0);
    });
    let status = of(&snapshot_of(&tenant), &ctx());
    let dnf = row(&status, "dnfRule");
    assert_eq!(dnf.state, OrgSetupRowState::Off);
    assert_eq!(
        dnf.summary,
        "In test mode (AuditAndNotify): it doesn't encrypt."
    );
    // Recently set up and right: waiting, then OK.
    let strip = row(&status, "stripRule");
    assert_eq!(strip.state, OrgSetupRowState::Waiting);
    assert_eq!(
        strip.summary,
        "Set up 10 minutes ago. New rules can take 30 minutes or more to start working."
    );
    let set_strip = |key: &str, value: serde_json::Value| {
        tenant.with(|s| {
            let strip = s
                .rules
                .iter_mut()
                .find(|r| r["Name"] == STRIP_RULE)
                .unwrap();
            strip[key] = value;
        })
    };
    set_strip("WhenChanged", json!("2026-01-01T00:00:00Z"));
    let status = of(&snapshot_of(&tenant), &ctx());
    let strip = row(&status, "stripRule");
    assert_eq!(
        (strip.state, strip.summary.as_str()),
        (OrgSetupRowState::Ok, "Set up.")
    );
    assert_eq!(
        strip.detail.as_deref(),
        Some(
            "Removes x-cumulas-label from mail that isn't authenticated mail from inside your organization, so nobody outside can ask for a label."
        )
    );
    // In test mode, it says what it then doesn't do.
    set_strip("Mode", json!("Audit"));
    let status = of(&snapshot_of(&tenant), &ctx());
    let strip = row(&status, "stripRule");
    assert_eq!(
        (strip.state, strip.summary.as_str()),
        (
            OrgSetupRowState::Off,
            "In test mode (Audit): it doesn't remove label requests."
        )
    );
    set_strip("Mode", json!("Enforce"));
    // Turned off.
    set_strip("State", json!("Disabled"));
    let status = of(&snapshot_of(&tenant), &ctx());
    assert_eq!(row(&status, "stripRule").summary, "Turned off.");
    assert!(status.any_ours);
}

#[test]
fn routing_rows_follow_the_relay_and_other_services() {
    let relay = rule(
        RELAY_RULE,
        json!({
            "FromScope": "InOrganization", "StopRuleProcessing": true,
            "RouteMessageOutboundConnector": "Email signatures (to signature server)",
        }),
    );
    let tenant = FakeTenant::fresh();
    for (inbound, path, state) in [
        (Some(true), RelayPath::CumulasInternal, OrgSetupRowState::Ok),
        (
            Some(false),
            RelayPath::CumulasNotInternal,
            OrgSetupRowState::Differs,
        ),
        (None, RelayPath::CumulasUnknown, OrgSetupRowState::Unknown),
    ] {
        tenant.with(|s| {
            s.rules = vec![relay.clone()];
            s.inbound = inbound
                .map(|internal| json!({ "Name": RELAY_INBOUND, "TreatMessagesAsInternal": internal }))
                .into_iter()
                .collect();
        });
        let status = of(&snapshot_of(&tenant), &ctx());
        assert_eq!(status.relay, path);
        assert_eq!(row(&status, "routing").state, state);
    }
    let tenant = FakeTenant::fresh();
    tenant.with(|s| {
        s.rules = vec![relay.clone()];
        s.inbound = vec![json!({ "Name": RELAY_INBOUND, "TreatMessagesAsInternal": false })];
    });
    let status = of(&snapshot_of(&tenant), &ctx());
    let routing = row(&status, "routing");
    assert_eq!(routing.fix, Some(OrgSetupFix::Signatures));
    assert_eq!(
        routing.link.as_deref(),
        Some("https://signature.cumulas.co/manage")
    );
    assert!(routing.summary.starts_with("Mail goes through the Cumulas Signatures relay, and Microsoft 365 treats it as outside mail when it comes back."));

    // Another routing service: a row of its own.
    tenant.with(|s| {
        s.rules = vec![rule(
            "Contoso: gateway",
            json!({
                "RouteMessageOutboundConnector": "Mimecast", "SentToScope": "NotInOrganization",
            }),
        )];
    });
    let status = of(&snapshot_of(&tenant), &ctx());
    let other = row(&status, "otherRouting:Contoso: gateway");
    assert_eq!(other.state, OrgSetupRowState::Unknown);
    assert_eq!(other.section, OrgSetupSection::Routing);
    assert!(other.summary.starts_with("Mail this rule matches goes out through “Mimecast”, for mail to people outside your organization. Cumulas Mail's encryption rules come before it"), "{}", other.summary);
}

#[test]
fn leftovers_say_whether_they_can_go() {
    let tenant = FakeTenant::fresh();
    tenant.with(|s| {
        s.rules = vec![
            rule(
                "Cumulas Mail: Confidential after the signature",
                json!({ "SetHeaderName": "x-cumulas-label-stamped" }),
            ),
            rule(
                "Cumulas Mail: ignore stamped label requests from outside",
                json!({ "RemoveHeader": "x-cumulas-label-stamped" }),
            ),
        ];
    });
    let status = of(&snapshot_of(&tenant), &ctx());
    let unused = "Left from an earlier setup and not used any more. You can delete it.";
    assert_eq!(
        row(
            &status,
            "leftover:Cumulas Mail: Confidential after the signature"
        )
        .summary,
        unused
    );
    let stamped = row(
        &status,
        "leftover:Cumulas Mail: ignore stamped label requests from outside",
    );
    assert_eq!(
        (stamped.state, stamped.summary.as_str()),
        (OrgSetupRowState::Info, unused)
    );
    // Without the auto-labeling policies, whether the stamped strip rule is
    // still needed isn't known: it's never called deletable.
    let mut unread = snapshot_of(&tenant);
    for compliance in [
        Part::Failed(failure(FailureKind::NoRole, "denied")),
        Part::Failed(failure(FailureKind::Unreachable, "connection reset")),
        Part::Skipped,
    ] {
        unread.compliance = compliance;
        let status = of(&unread, &ctx());
        let stamped = row(
            &status,
            "leftover:Cumulas Mail: ignore stamped label requests from outside",
        );
        assert_eq!(stamped.state, OrgSetupRowState::Unknown);
        assert_eq!(
            stamped.summary,
            "Auto-labeling policies couldn't be read, so Cumulas Mail can't tell whether this is still needed: policies from an earlier setup may use x-cumulas-label-stamped. Sign in as a Compliance Administrator or Global Administrator to check before deleting it."
        );
        // The other leftover doesn't depend on them.
        assert_eq!(
            row(
                &status,
                "leftover:Cumulas Mail: Confidential after the signature"
            )
            .summary,
            unused
        );
    }
    // Still used by a policy of ours.
    tenant.with(|s| {
        s.policies = vec![json!({ "Name": "Cumulas Mail: Confidential", "Mode": "Enable", "ApplySensitivityLabel": CONFIDENTIAL, "ExchangeLocation": ["All"], "OverwriteLabel": true })];
        s.policy_rules = vec![json!({
            "Name": "Cumulas Mail: Confidential (header)", "ParentPolicyName": "Cumulas Mail: Confidential",
            "HeaderMatchesPatterns": { "x-cumulas-label-stamped": format!("^{CONFIDENTIAL}$") },
        })];
    });
    let status = of(&snapshot_of(&tenant), &ctx());
    assert_eq!(
        row(
            &status,
            "leftover:Cumulas Mail: ignore stamped label requests from outside"
        )
        .summary,
        "Still needed: auto-labeling policies from an earlier setup (Cumulas Mail: Confidential) use x-cumulas-label-stamped. Select those labels and Repair them first."
    );
    // The policy needs repair (its rule looks at the stamped header), and
    // it's checked by default so Repair covers it.
    let label = row(&status, &format!("label:{CONFIDENTIAL}"));
    assert_eq!(label.state, OrgSetupRowState::Differs);
    assert!(
        label.summary.starts_with("Needs repair: header pattern"),
        "{}",
        label.summary
    );
    assert_eq!(status.defaults.label_ids, [CONFIDENTIAL]);
}

#[test]
fn label_rows_follow_their_policies() {
    let tenant = FakeTenant::fresh();
    let pattern = format!("(?i)^({CONFIDENTIAL})$");
    let policy = |fields: serde_json::Value| {
        let mut p = json!({
            "Name": "Cumulas Mail: Confidential", "Mode": "Enable", "Enabled": true,
            "ApplySensitivityLabel": CONFIDENTIAL, "ExchangeLocation": ["All"], "OverwriteLabel": true,
            "WhenChanged": "2026-01-01T00:00:00Z", "ExchangeSenderMemberOf": ["pilot@contoso.com"],
        });
        for (k, v) in fields.as_object().unwrap() {
            p[k] = v.clone();
        }
        p
    };
    let check = |fields: serde_json::Value| {
        tenant.with(|s| {
            s.rules = vec![rule(STRIP_RULE, json!({ "ExceptIfFromScope": "InOrganization", "RemoveHeader": "x-cumulas-label" }))];
            s.policies = vec![policy(fields)];
            s.policy_rules = vec![json!({
                "Name": "Cumulas Mail: Confidential (header)", "ParentPolicyName": "Cumulas Mail: Confidential",
                "Workload": "Exchange", "HeaderMatchesPatterns": { "x-cumulas-label": pattern }, "WhenChanged": "2026-01-01T00:00:00Z",
            })];
        });
        let status = of(&snapshot_of(&tenant), &ctx());
        let row = row(&status, &format!("label:{CONFIDENTIAL}")).clone();
        (row.state, row.summary, row.detail)
    };
    let (state, summary, detail) = check(json!({}));
    assert_eq!((state, summary.as_str()), (OrgSetupRowState::Ok, "On."));
    assert_eq!(
        detail.as_deref(),
        Some(format!("Matched by Guid (header value {CONFIDENTIAL}). Applies only to mail from members of pilot@contoso.com.").as_str())
    );
    assert_eq!(check(json!({ "Enabled": false })).1, "Turned off.");
    assert_eq!(
        check(json!({ "Mode": "PendingDeletion" })).1,
        "Microsoft 365 is still deleting its auto-labeling policy. Try again later."
    );
    assert_eq!(
        check(json!({ "Mode": "TestWithoutNotifications" })).1,
        "In simulation: it doesn't label anything."
    );
    assert_eq!(
        check(json!({ "Mode": "TestWithoutNotifications", "AutoEnableAfter": "01:00:00" })).1,
        "In simulation; it turns on by itself an hour after the simulation completes. Simulations can take several hours."
    );
    assert_eq!(
        check(json!({ "Mode": "TestWithoutNotifications", "AutoEnableAfter": "01:00:00", "SimulationStatus": "Completed", "SimulationCompletedTime": "2026-10-19T12:00:00Z" })).1,
        "Simulation complete; it turns on by itself around Oct 19, 13:00 UTC."
    );
    assert_eq!(
        check(json!({ "DistributionStatus": "Pending" })).1,
        "Microsoft 365 is still applying it."
    );
    assert_eq!(
        check(json!({ "DistributionStatus": "Error", "DistributionResults": ["Sync failed."] })).1,
        "Microsoft 365 couldn't apply its auto-labeling policy: Sync failed."
    );
    let (state, summary, _) =
        check(json!({ "WhenChanged": (Utc::now() - Duration::hours(3)).to_rfc3339() }));
    assert_eq!(state, OrgSetupRowState::Waiting);
    assert_eq!(
        summary,
        "Turned on 3 hours ago. Auto-labeling policies can take up to a day to start working."
    );
    let (_, summary, _) =
        check(json!({ "WhenChanged": (Utc::now() - Duration::minutes(70)).to_rfc3339() }));
    assert_eq!(
        summary,
        "Turned on 1 hour ago. Auto-labeling policies can take up to a day to start working."
    );
    assert_eq!(
        check(json!({ "SharePointLocation": ["All"] })).1,
        "Its auto-labeling policy has SharePoint or OneDrive locations that Cumulas Mail doesn't set. Remove them in the Microsoft Purview portal, then select Read again."
    );

    // A label of the account's list that Purview doesn't list; one that
    // isn't for email; one Purview lists that the account's list doesn't.
    let tenant = FakeTenant::fresh();
    tenant.with(|s| {
        s.labels.retain(|l| l["Guid"] != SECRET);
        s.labels[1]["ContentType"] = json!("File");
        s.labels.push(super::tenant::label(
            "6c1b0f3e-1111-4a8b-9c2d-0000000000aa",
            "Board only",
        ));
    });
    let status = of(&snapshot_of(&tenant), &ctx());
    assert_eq!(
        row(&status, &format!("label:{SECRET}")).summary,
        "Purview doesn't list this label."
    );
    assert_eq!(
        row(&status, &format!("label:{GENERAL}")).summary,
        "This label isn't for email."
    );
    let extra = status
        .labels
        .iter()
        .find(|l| l.title == "Board only")
        .unwrap();
    assert!(!extra.in_catalog);
    assert_eq!(extra.id, "6c1b0f3e-1111-4a8b-9c2d-0000000000aa");
}

#[test]
fn licensing_rows_are_a_hint_about_one_person() {
    let tenant = FakeTenant::fresh();
    let mut snapshot = snapshot_of(&tenant);
    snapshot.licensing = Part::Read(LicenseHint {
        all: vec!["SPE_E3".into()],
        ..LicenseHint::default()
    });
    let status = of(&snapshot, &ctx());
    let licensing = row(&status, "licensing");
    assert_eq!(licensing.state, OrgSetupRowState::Blocked);
    assert_eq!(
        licensing.summary,
        "adele@contoso.com's license doesn't include automatic labeling, which sending with a label needs."
    );
    assert!(
        licensing
            .detail
            .as_deref()
            .unwrap()
            .starts_with("Microsoft 365 E3 and Business Premium don't include it.")
    );
    assert_eq!(status.license, LicenseHintState::NotIncluded);
    snapshot.licensing = Part::Read(LicenseHint {
        pending: vec![("SPE_E5".into(), "PendingActivation".into())],
        all: vec!["SPE_E5".into()],
        ..LicenseHint::default()
    });
    let status = of(&snapshot, &ctx());
    let licensing = row(&status, "licensing");
    assert_eq!(licensing.state, OrgSetupRowState::Waiting);
    assert_eq!(
        licensing.summary,
        "adele@contoso.com's license includes automatic labeling, but Microsoft 365 hasn't finished turning it on (PendingActivation)."
    );
    snapshot.licensing = Part::Failed(failure(
        FailureKind::Other,
        "Microsoft Graph returned HTTP 500.",
    ));
    let status = of(&snapshot, &ctx());
    assert_eq!(
        row(&status, "licensing").summary,
        "Couldn't check adele@contoso.com's license (Microsoft Graph returned HTTP 500)."
    );
    assert_eq!(status.license, LicenseHintState::Unknown);
}

#[test]
fn the_approval_row_follows_the_work_account() {
    let snapshot = snapshot_of(&FakeTenant::fresh());
    let mut ctx = ctx();
    ctx.needs_permission = vec![AccountFeature::SensitivityLabels];
    let status = of(&snapshot, &ctx);
    assert_eq!(row(&status, "approval").state, OrgSetupRowState::Missing);
    assert_eq!(
        row(&status, "approval").summary,
        "Reading sensitivity labels isn't approved for adele@contoso.com."
    );
    ctx.needs_permission.clear();
    ctx.refused_features = vec![AccountFeature::SensitivityLabels];
    let status = of(&snapshot, &ctx);
    assert_eq!(
        row(&status, "approval").summary,
        "Microsoft 365 refused the label list for adele@contoso.com, often because of licensing."
    );
}

#[test]
fn a_repair_that_moves_ours_below_a_stopper_names_it() {
    // Placed above everything while the relay's return wasn't internal;
    // now it is, and a stopper sits between ours and the relay's rule.
    let tenant = FakeTenant::fresh();
    tenant.with(|s| {
        let protect = |name: &str, pattern: &str, template: &str| {
            rule(
                name,
                json!({
                    "ExceptIfFromScope": "NotInOrganization", "HeaderMatchesMessageHeader": "x-cumulas-protect",
                    "HeaderMatchesPatterns": [pattern], "ApplyRightsProtectionTemplate": template,
                    "RemoveHeader": "x-cumulas-protect", "RuleErrorAction": "Defer",
                }),
            )
        };
        s.rules = vec![
            protect(ENCRYPT_RULE, "^encrypt$", "Encrypt"),
            protect(DNF_RULE, "^do-not-forward$", "Do Not Forward"),
            rule(
                "Contoso: stop for partner.com",
                json!({ "StopRuleProcessing": true }),
            ),
            rule(
                RELAY_RULE,
                json!({ "FromScope": "InOrganization", "StopRuleProcessing": true, "RouteMessageOutboundConnector": "Email signatures (to signature server)" }),
            ),
        ];
        s.inbound = vec![
            json!({ "Name": RELAY_INBOUND, "ConnectorType": "OnPremises", "TreatMessagesAsInternal": true }),
        ];
    });
    let status = of(&snapshot_of(&tenant), &ctx());
    let encrypt = row(&status, "encryptRule");
    assert_eq!(encrypt.state, OrgSetupRowState::Differs);
    assert!(
        encrypt
            .summary
            .contains("moves below “Contoso: stop for partner.com”"),
        "{}",
        encrypt.summary
    );
}
