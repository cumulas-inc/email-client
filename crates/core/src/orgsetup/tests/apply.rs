//! Making the changes, against a modeled organization behind a mock server.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value, json};
use url::Url;
use wiremock::{MockServer, ResponseTemplate};

use super::tenant::{FakeTenant, rule};
use super::{ADMIN, CLIENT, CONFIDENTIAL, GENERAL, TENANT, ctx, session};
use crate::model::{OrgSetupChangeResult, OrgSetupChoices, OrgSetupOutcome};
use crate::orgsetup::apply::{ApplyOutcome, SIGNED_OUT, WINDOW_CLOSED, apply};
use crate::orgsetup::plan::{self, STRIP_FIRST};
use crate::orgsetup::runlog::RunHeader;
use crate::orgsetup::status::{SessionInfo, read_snapshot};
use crate::orgsetup::wanted::{DNF_RULE, ENCRYPT_RULE, MARKER, STRIP_RULE};
use crate::orgsetup::{Session, details_text};

fn choices(actions: bool, labels: &[&str]) -> OrgSetupChoices {
    OrgSetupChoices {
        actions,
        encrypt_template: Some("Encrypt".into()),
        do_not_forward_template: Some("Do Not Forward".into()),
        label_ids: labels.iter().map(|l| (*l).to_owned()).collect(),
        labels_without_license: false,
    }
}

fn info(session: &Session) -> SessionInfo {
    SessionInfo {
        admin: session.admin.clone(),
        client_id: session.client_id.clone(),
        admin_object_id: session.admin_object_id.clone(),
    }
}

async fn start(tenant: &FakeTenant) -> (MockServer, Arc<Session>) {
    let server = MockServer::start().await;
    tenant.mount(&server).await;
    let session = Arc::new(session(&server).await);
    (server, session)
}

/// The change IDs a read shows now, as the dialog would confirm them.
async fn shown(session: &Session, choices: &OrgSetupChoices) -> Vec<String> {
    let snapshot = read_snapshot(session, &ctx()).await.unwrap();
    plan::plan(&snapshot, &ctx(), choices)
        .changes
        .into_iter()
        .map(|c| c.id)
        .collect()
}

async fn run(session: &Session, choices: &OrgSetupChoices, confirmed: &[String]) -> ApplyOutcome {
    apply(session, &ctx(), &info(session), choices, confirmed)
        .await
        .unwrap()
}

/// Reads, confirms what it shows, and applies.
async fn set_up(session: &Session, choices: &OrgSetupChoices) -> ApplyOutcome {
    let confirmed = shown(session, choices).await;
    run(session, choices, &confirmed).await
}

fn outcomes(result: &[OrgSetupChangeResult]) -> Vec<(String, OrgSetupOutcome, Option<String>)> {
    result
        .iter()
        .map(|r| (r.change.id.clone(), r.outcome, r.message.clone()))
        .collect()
}

fn params(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

fn bad_request(message: &str) -> ResponseTemplate {
    ResponseTemplate::new(400)
        .set_body_json(json!({ "error": { "code": "BadRequest", "message": message } }))
}

fn assert_never_removed(tenant: &FakeTenant) {
    for (cmdlet, _) in tenant.writes() {
        assert!(
            !cmdlet.starts_with("Remove-") && !cmdlet.starts_with("Disable-"),
            "{cmdlet}"
        );
    }
}

/// Our 0.4.0 Encrypt rule, and Do Not Forward right.
fn repairable() -> FakeTenant {
    let tenant = FakeTenant::fresh();
    tenant.with(|s| {
        s.rules = vec![
            rule(ENCRYPT_RULE, json!({
                "FromScope": "InOrganization", "HeaderMatchesMessageHeader": "x-cumulas-protect",
                "HeaderMatchesPatterns": ["^encrypt$"], "HeaderContainsMessageHeader": "X-Signature-Stamped",
                "HeaderContainsWords": ["1"], "ApplyRightsProtectionTemplate": "Encrypt",
                "RemoveHeader": "x-cumulas-protect",
            })),
            rule(DNF_RULE, json!({
                "ExceptIfFromScope": "NotInOrganization", "HeaderMatchesMessageHeader": "x-cumulas-protect",
                "HeaderMatchesPatterns": ["^do-not-forward$"], "ApplyRightsProtectionTemplate": "Do Not Forward",
                "RemoveHeader": "x-cumulas-protect", "RuleErrorAction": "Defer",
            })),
        ]
    });
    tenant
}

#[tokio::test]
async fn a_fresh_organization_is_set_up_in_order_and_read_back() {
    let tenant = FakeTenant::fresh();
    let (_server, session) = start(&tenant).await;
    let chosen = choices(true, &[CONFIDENTIAL]);
    let outcome = set_up(&session, &chosen).await;
    let result = &outcome.result;
    assert!(!result.stale && !result.signed_out);
    assert!(
        result
            .changes
            .iter()
            .all(|c| c.outcome == OrgSetupOutcome::Done),
        "{:?}",
        outcomes(&result.changes)
    );
    assert_eq!(result.changes.len(), 6);
    assert!(outcome.rules_changed && outcome.policies_changed);

    let writes = tenant.writes();
    let names: Vec<&str> = writes.iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(
        names,
        [
            "New-TransportRule",
            "New-TransportRule",
            "New-TransportRule",
            "New-AutoSensitivityLabelPolicy",
            "New-AutoSensitivityLabelRule",
            "Set-AutoSensitivityLabelPolicy",
        ]
    );
    assert_eq!(
        writes[0].1,
        params(json!({
            "Name": STRIP_RULE, "ExceptIfFromScope": "InOrganization", "RemoveHeader": "x-cumulas-label",
            "Priority": 0, "Mode": "Enforce", "Comments": MARKER,
        }))
    );
    // Nothing blocks: they land last, without a Priority.
    assert_eq!(
        writes[1].1,
        params(json!({
            "Name": ENCRYPT_RULE, "ExceptIfFromScope": "NotInOrganization",
            "HeaderMatchesMessageHeader": "x-cumulas-protect", "HeaderMatchesPatterns": ["^encrypt$"],
            "ApplyRightsProtectionTemplate": "Encrypt", "RemoveHeader": "x-cumulas-protect",
            "RuleErrorAction": "Defer", "Mode": "Enforce", "Comments": MARKER,
        }))
    );
    assert_eq!(
        writes[2].1,
        params(json!({
            "Name": DNF_RULE, "ExceptIfFromScope": "NotInOrganization",
            "HeaderMatchesMessageHeader": "x-cumulas-protect", "HeaderMatchesPatterns": ["^do-not-forward$"],
            "ApplyRightsProtectionTemplate": "Do Not Forward", "RemoveHeader": "x-cumulas-protect",
            "RuleErrorAction": "Defer", "Mode": "Enforce", "Comments": MARKER,
        }))
    );
    assert_eq!(
        writes[3].1,
        params(json!({
            "Name": "Cumulas Mail: Confidential", "ApplySensitivityLabel": CONFIDENTIAL,
            "ExchangeLocation": ["All"], "OverwriteLabel": true, "Mode": "TestWithoutNotifications",
            "Comment": MARKER,
        }))
    );
    assert_eq!(
        writes[4].1,
        params(json!({
            "Name": "Cumulas Mail: Confidential (header)", "Policy": "Cumulas Mail: Confidential",
            "Workload": "Exchange",
            "HeaderMatchesPatterns": { "@odata.type": "#Exchange.GenericHashTable", "x-cumulas-label": format!("(?i)^({CONFIDENTIAL})$") },
        }))
    );
    assert_eq!(
        writes[5].1,
        params(
            json!({ "Identity": "Cumulas Mail: Confidential", "StartSimulation": true, "AutoEnableAfter": "01:00:00" })
        )
    );
    // The simulation starts after the rule exists.
    assert_eq!(tenant.rule_names(), [STRIP_RULE, ENCRYPT_RULE, DNF_RULE]);
    assert!(
        tenant
            .policy_rule("Cumulas Mail: Confidential (header)")
            .is_some()
    );
    let policy = tenant.policy("Cumulas Mail: Confidential").unwrap();
    assert_eq!(policy["Mode"], json!("TestWithoutNotifications"));
    assert_eq!(policy["AutoEnableAfter"], json!("01:00:00"));
    // The status after: ours are there, the button says Repair.
    let status = &result.status;
    assert!(status.any_ours);
    assert_eq!(status.defaults.label_ids, [CONFIDENTIAL]);
    assert!(status.plan.changes.is_empty(), "{:?}", status.plan.changes);
    // Reads before writes, every Exchange call with the admin's mailbox.
    let calls = tenant.calls();
    assert!(
        calls
            .iter()
            .filter(|c| c.cmdlet == "Get-TransportRule")
            .count()
            >= 3
    );
    assert!(calls.iter().all(|c| c.anchor == "UPN:admin@contoso.com"));
    assert_never_removed(&tenant);
}

#[tokio::test]
async fn a_stale_plan_writes_nothing() {
    let tenant = FakeTenant::fresh();
    let (_server, session) = start(&tenant).await;
    let chosen = choices(true, &[]);
    let confirmed = shown(&session, &chosen).await;
    // Someone else made Encrypt meanwhile.
    tenant.with(|s| {
        s.rules.push(rule(
            ENCRYPT_RULE,
            json!({ "RemoveHeader": "x-cumulas-protect" }),
        ))
    });
    let outcome = run(&session, &chosen, &confirmed).await;
    assert!(outcome.result.stale);
    assert!(outcome.result.changes.is_empty());
    assert!(tenant.writes().is_empty());
    assert!(!outcome.rules_changed && !outcome.policies_changed);
    // The plan to review now repairs it.
    assert_eq!(
        outcome.result.status.plan.changes[0].id,
        "change:mailFlowRule:cumulas mail: encrypt"
    );
    // A made-up confirmation likewise.
    let outcome = run(
        &session,
        &chosen,
        &["create:mailFlowRule:contoso: x".into()],
    )
    .await;
    assert!(outcome.result.stale);
    assert!(tenant.writes().is_empty());
}

#[tokio::test]
async fn one_that_already_exists_is_adopted_and_repaired() {
    let tenant = FakeTenant::fresh();
    tenant.with(|s| {
        s.rules.push(rule(ENCRYPT_RULE, json!({
            "ExceptIfFromScope": "NotInOrganization", "HeaderMatchesMessageHeader": "x-cumulas-protect",
            "HeaderMatchesPatterns": ["^encrypt$"], "ApplyRightsProtectionTemplate": "Encrypt",
            "RemoveHeader": "x-cumulas-protect", "RuleErrorAction": "Ignore",
        })));
        // Not listed by the read nor apply's fresh read.
        s.hidden.insert(ENCRYPT_RULE.into(), 2);
    });
    let (_server, session) = start(&tenant).await;
    let outcome = set_up(&session, &choices(true, &[])).await;
    let results = outcomes(&outcome.result.changes);
    assert_eq!(results[0].1, OrgSetupOutcome::Done, "{results:?}");
    assert_eq!(
        results[0].2.as_deref(),
        Some("It was already there; Cumulas Mail repaired it.")
    );
    let writes = tenant.writes();
    assert_eq!(writes[0].0, "New-TransportRule");
    assert_eq!(writes[1].0, "Set-TransportRule");
    assert_eq!(writes[1].1["RuleErrorAction"], json!("Defer"));
    assert_eq!(
        tenant.rule(ENCRYPT_RULE).unwrap()["RuleErrorAction"],
        json!("Defer")
    );
}

#[tokio::test]
async fn a_policy_that_already_exists_is_adopted_and_its_simulation_started() {
    let tenant = FakeTenant::fresh();
    tenant.with(|s| {
        s.rules.push(rule(
            STRIP_RULE,
            json!({ "ExceptIfFromScope": "InOrganization", "RemoveHeader": "x-cumulas-label" }),
        ));
        s.policies.push(json!({
            "Name": "Cumulas Mail: Confidential", "Identity": "Cumulas Mail: Confidential",
            "Mode": "TestWithoutNotifications", "Enabled": true, "ApplySensitivityLabel": CONFIDENTIAL,
            "ExchangeLocation": [], "OverwriteLabel": false, "SharePointLocation": [],
        }));
        // Not listed by the read nor apply's fresh read.
        s.hidden.insert("Cumulas Mail: Confidential".into(), 2);
    });
    let (_server, session) = start(&tenant).await;
    let outcome = set_up(&session, &choices(false, &[CONFIDENTIAL])).await;
    let results = outcomes(&outcome.result.changes);
    assert!(
        results.iter().all(|r| r.1 == OrgSetupOutcome::Done),
        "{results:?}"
    );
    assert_eq!(
        results[0].2.as_deref(),
        Some("It was already there; Cumulas Mail repaired it.")
    );
    let writes: Vec<String> = tenant.writes().into_iter().map(|(c, _)| c).collect();
    assert_eq!(
        writes,
        [
            "New-AutoSensitivityLabelPolicy",
            "Set-AutoSensitivityLabelPolicy",
            "New-AutoSensitivityLabelRule",
            "Set-AutoSensitivityLabelPolicy",
        ]
    );
    let policy = tenant.policy("Cumulas Mail: Confidential").unwrap();
    assert_eq!(policy["OverwriteLabel"], json!(true));
    assert_eq!(policy["ExchangeLocation"], json!(["All"]));
    assert_eq!(policy["AutoEnableAfter"], json!("01:00:00"));

    // Someone else's rule in it: left alone.
    let tenant = FakeTenant::fresh();
    tenant.with(|s| {
        s.rules.push(rule(
            STRIP_RULE,
            json!({ "ExceptIfFromScope": "InOrganization", "RemoveHeader": "x-cumulas-label" }),
        ));
        s.policies.push(json!({
            "Name": "Cumulas Mail: Confidential", "Mode": "Enable", "ExchangeLocation": ["All"],
        }));
        s.policy_rules.push(json!({
            "Name": "Contoso: everything", "ParentPolicyName": "Cumulas Mail: Confidential",
        }));
        s.hidden.insert("Cumulas Mail: Confidential".into(), 2);
    });
    let (_server, session) = start(&tenant).await;
    let outcome = set_up(&session, &choices(false, &[CONFIDENTIAL])).await;
    let results = outcomes(&outcome.result.changes);
    assert_eq!(results[0].1, OrgSetupOutcome::Failed, "{results:?}");
    assert!(results[1..].iter().all(|r| r.1 == OrgSetupOutcome::Skipped));
    let writes: Vec<String> = tenant.writes().into_iter().map(|(c, _)| c).collect();
    assert_eq!(writes, ["New-AutoSensitivityLabelPolicy"]);
}

#[tokio::test]
async fn a_refused_null_is_sent_again_as_empty_and_pinned() {
    let tenant = repairable();
    tenant.script(
        "Set-TransportRule",
        bad_request("Cannot bind argument to parameter 'FromScope' because it is null."),
    );
    let (_server, session) = start(&tenant).await;
    let outcome = set_up(&session, &choices(true, &[])).await;
    let results = outcomes(&outcome.result.changes);
    assert_eq!(
        results,
        [(
            "change:mailFlowRule:cumulas mail: encrypt".to_owned(),
            OrgSetupOutcome::Done,
            None
        )]
    );
    let writes = tenant.writes();
    assert_eq!(writes[0].1["FromScope"], Value::Null);
    assert_eq!(writes[1].1["FromScope"], json!(""));
    assert_eq!(writes[1].1["HeaderContainsMessageHeader"], json!(""));
    assert_eq!(writes[1].1["HeaderContainsWords"], json!([]));
    assert!(session.pins().clear_with_empty);
    assert_eq!(tenant.rule(ENCRYPT_RULE).unwrap()["FromScope"], Value::Null);
}

#[tokio::test]
async fn a_setting_that_cant_be_cleared_fails_and_nothing_is_deleted() {
    let tenant = repairable();
    tenant.script(
        "Set-TransportRule",
        bad_request("Cannot bind argument to parameter 'FromScope' because it is null."),
    );
    tenant.script(
        "Set-TransportRule",
        bad_request("Cannot bind argument to parameter 'FromScope' because it is an empty string."),
    );
    let (_server, session) = start(&tenant).await;
    let outcome = set_up(&session, &choices(true, &[])).await;
    let result = &outcome.result.changes[0];
    assert_eq!(result.outcome, OrgSetupOutcome::Failed);
    assert_eq!(
        result.message.as_deref(),
        Some(
            "Microsoft 365 didn't accept clearing FromScope, HeaderContainsMessageHeader, HeaderContainsWords through this interface. Part 1 of the PowerShell script does it."
        )
    );
    assert_never_removed(&tenant);
    // The rule stays as it was.
    assert_eq!(
        tenant.rule(ENCRYPT_RULE).unwrap()["FromScope"],
        json!("InOrganization")
    );
    assert!(!outcome.rules_changed);
}

#[tokio::test]
async fn a_refused_comment_is_left_out_from_then_on() {
    let tenant = FakeTenant::fresh();
    tenant.script(
        "New-TransportRule",
        bad_request("A parameter cannot be found that matches parameter name 'Comments'."),
    );
    let (_server, session) = start(&tenant).await;
    let outcome = set_up(&session, &choices(true, &[])).await;
    assert!(
        outcome
            .result
            .changes
            .iter()
            .all(|c| c.outcome == OrgSetupOutcome::Done)
    );
    let writes = tenant.writes();
    assert!(writes[0].1.contains_key("Comments"));
    assert!(!writes[1].1.contains_key("Comments"));
    // Do Not Forward's goes without it straight away.
    assert_eq!(writes.len(), 3);
    assert!(!writes[2].1.contains_key("Comments"));
    assert!(session.pins().without_comments);
}

#[tokio::test]
async fn a_refused_hashtable_is_sent_plain_from_then_on() {
    let tenant = FakeTenant::fresh();
    tenant.script(
        "New-AutoSensitivityLabelRule",
        bad_request(
            "Cannot convert the \"Newtonsoft.Json.Linq.JObject\" value to type \"PswsHashtable\".",
        ),
    );
    let (_server, session) = start(&tenant).await;
    let outcome = set_up(&session, &choices(false, &[CONFIDENTIAL])).await;
    assert!(
        outcome
            .result
            .changes
            .iter()
            .all(|c| c.outcome == OrgSetupOutcome::Done),
        "{:?}",
        outcomes(&outcome.result.changes)
    );
    let rules: Vec<Map<String, Value>> = tenant
        .writes()
        .into_iter()
        .filter(|(c, _)| c == "New-AutoSensitivityLabelRule")
        .map(|(_, p)| p)
        .collect();
    assert!(
        rules[0]["HeaderMatchesPatterns"]
            .get("@odata.type")
            .is_some()
    );
    assert_eq!(
        rules[1]["HeaderMatchesPatterns"],
        json!({ "x-cumulas-label": format!("(?i)^({CONFIDENTIAL})$") })
    );
    assert!(session.pins().plain_hashtable);
}

#[tokio::test]
async fn a_missing_license_fails_its_label_and_skips_the_others() {
    let tenant = FakeTenant::fresh();
    tenant.script(
        "New-AutoSensitivityLabelPolicy",
        bad_request("The tenant isn't licensed for this feature."),
    );
    let (_server, session) = start(&tenant).await;
    let outcome = set_up(&session, &choices(false, &[CONFIDENTIAL, GENERAL])).await;
    let results = outcomes(&outcome.result.changes);
    assert_eq!(results[0].1, OrgSetupOutcome::Done, "{results:?}");
    let license = "Microsoft 365 says a license is missing: both the administrator who signs in and the people who send need a plan that includes automatic labeling. Microsoft said: The tenant isn't licensed for this feature.";
    assert_eq!(results[1].1, OrgSetupOutcome::Failed);
    assert_eq!(results[1].2.as_deref(), Some(license));
    for skipped in &results[2..] {
        assert_eq!(skipped.1, OrgSetupOutcome::Skipped, "{skipped:?}");
        assert_eq!(skipped.2.as_deref(), Some(license));
    }
    // No simulation as a fallback.
    assert!(
        !tenant
            .writes()
            .iter()
            .any(|(c, _)| c.starts_with("Set-AutoSensitivity"))
    );
}

#[tokio::test]
async fn without_a_compliance_role_the_rules_are_still_made() {
    let tenant = FakeTenant::fresh();
    tenant.script(
        "New-AutoSensitivityLabelPolicy",
        ResponseTemplate::new(403)
            .set_body_json(json!({ "error": { "message": "Access denied" } })),
    );
    let (_server, session) = start(&tenant).await;
    let outcome = set_up(&session, &choices(true, &[CONFIDENTIAL])).await;
    let results = outcomes(&outcome.result.changes);
    for done in &results[..3] {
        assert_eq!(done.1, OrgSetupOutcome::Done, "{done:?}");
    }
    let forbidden = "Your account can't change auto-labeling policies. Sign in as a Compliance Administrator or Global Administrator (or a member of the Information Protection Admins role group).";
    assert_eq!(results[3].1, OrgSetupOutcome::Failed);
    assert_eq!(results[3].2.as_deref(), Some(forbidden));
    assert!(
        results[4..]
            .iter()
            .all(|r| r.1 == OrgSetupOutcome::Skipped && r.2.as_deref() == Some(forbidden))
    );
}

#[tokio::test]
async fn without_an_exchange_role_no_label_change_is_made() {
    let tenant = FakeTenant::fresh();
    tenant.script(
        "New-TransportRule",
        ResponseTemplate::new(403).set_body_json(
            json!({ "error": { "message": "User is not allowed to call New-TransportRule" } }),
        ),
    );
    let (_server, session) = start(&tenant).await;
    let outcome = set_up(&session, &choices(true, &[CONFIDENTIAL])).await;
    let results = outcomes(&outcome.result.changes);
    let forbidden = "Your account can't change mail flow rules. Sign in as an Exchange Administrator or Global Administrator. If you just got the role through Privileged Identity Management, Exchange can take a while to apply it.";
    assert_eq!(
        (results[0].1, results[0].2.as_deref()),
        (OrgSetupOutcome::Failed, Some(forbidden))
    );
    assert_eq!(
        (results[1].1, results[1].2.as_deref()),
        (OrgSetupOutcome::Skipped, Some(forbidden))
    );
    for label in &results[3..] {
        assert_eq!(
            (label.1, label.2.as_deref()),
            (OrgSetupOutcome::Skipped, Some(STRIP_FIRST))
        );
    }
    assert!(
        !tenant
            .writes()
            .iter()
            .any(|(c, _)| c.contains("AutoSensitivity"))
    );
}

#[tokio::test]
async fn a_failed_strip_rule_keeps_every_label_change_out() {
    let tenant = FakeTenant::fresh();
    tenant.script("New-TransportRule", bad_request("Something went wrong."));
    let (_server, session) = start(&tenant).await;
    let outcome = set_up(&session, &choices(true, &[CONFIDENTIAL])).await;
    let results = outcomes(&outcome.result.changes);
    assert_eq!(results[0].1, OrgSetupOutcome::Failed);
    assert_eq!(
        results[0].2.as_deref(),
        Some("Microsoft 365 didn't accept this: Something went wrong.")
    );
    // Encrypt and Do Not Forward still go ahead.
    assert_eq!(results[1].1, OrgSetupOutcome::Done);
    for label in &results[3..] {
        assert_eq!(
            (label.1, label.2.as_deref()),
            (OrgSetupOutcome::Skipped, Some(STRIP_FIRST))
        );
    }
    assert!(
        !tenant
            .writes()
            .iter()
            .any(|(c, _)| c.contains("AutoSensitivity"))
    );
}

#[tokio::test]
async fn a_sign_in_that_ends_midway_keeps_what_was_done() {
    let tenant = FakeTenant::fresh();
    // Do Not Forward's creation: refused, and again with a fresh token.
    tenant.script_after("New-TransportRule", 1, ResponseTemplate::new(401));
    tenant.script("New-TransportRule", ResponseTemplate::new(401));
    let (_server, session) = start(&tenant).await;
    let outcome = set_up(&session, &choices(true, &[])).await;
    let result = &outcome.result;
    assert!(result.signed_out);
    let results = outcomes(&result.changes);
    assert_eq!(results[0].1, OrgSetupOutcome::Done);
    assert_eq!(
        (results[1].1, results[1].2.as_deref()),
        (OrgSetupOutcome::Skipped, Some(SIGNED_OUT))
    );
    // What was made counts.
    assert!(outcome.rules_changed);
    assert!(tenant.with(|s| s.token_requests) >= 1);
}

#[tokio::test]
async fn closing_the_window_skips_the_changes_not_yet_started() {
    let tenant = FakeTenant::fresh();
    tenant.script(
        "New-TransportRule",
        ResponseTemplate::new(200).set_delay(Duration::from_millis(1500)),
    );
    let (_server, session) = start(&tenant).await;
    let chosen = choices(true, &[]);
    let confirmed = shown(&session, &chosen).await;
    let closer = Arc::clone(&session);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        closer.cancel();
    });
    let outcome = run(&session, &chosen, &confirmed).await;
    let results = outcomes(&outcome.result.changes);
    // The change in flight finishes; the next one isn't started.
    assert_eq!(results[0].1, OrgSetupOutcome::Done, "{results:?}");
    assert_eq!(
        (results[1].1, results[1].2.as_deref()),
        (OrgSetupOutcome::Skipped, Some(WINDOW_CLOSED))
    );
    assert_eq!(tenant.writes().len(), 1);
}

#[tokio::test]
async fn what_compliance_doesnt_list_yet_is_said_so() {
    let tenant = FakeTenant::fresh();
    tenant.with(|s| {
        s.hidden.insert("Cumulas Mail: Confidential".into(), 100);
        s.hidden
            .insert("Cumulas Mail: Confidential (header)".into(), 100);
    });
    let (_server, session) = start(&tenant).await;
    let outcome = set_up(&session, &choices(false, &[CONFIDENTIAL])).await;
    let results = outcomes(&outcome.result.changes);
    assert_eq!(results[0].1, OrgSetupOutcome::Done, "{results:?}");
    for later in &results[1..] {
        assert_eq!(later.1, OrgSetupOutcome::NotYetListed, "{later:?}");
    }
    // Listed again three times, then left.
    let lists = tenant
        .calls()
        .iter()
        .filter(|c| c.cmdlet == "Get-AutoSensitivityLabelPolicy" && c.params.is_empty())
        .count();
    assert_eq!(lists, 3 + 3);
    assert!(outcome.policies_changed);
}

#[tokio::test]
async fn a_change_microsoft_accepts_but_doesnt_keep_fails_with_what_it_shows() {
    let tenant = repairable();
    // Accepted, but nothing changes.
    tenant.script("Set-TransportRule", ResponseTemplate::new(200));
    let (_server, session) = start(&tenant).await;
    let outcome = set_up(&session, &choices(true, &[])).await;
    let result = &outcome.result.changes[0];
    assert_eq!(result.outcome, OrgSetupOutcome::Failed);
    assert_eq!(
        result.message.as_deref(),
        Some(
            "Microsoft 365 accepted the change but shows something else: applies to mail from your organization's addresses (it applied only to mail Microsoft 365 had authenticated)"
        )
    );
}

/// A whole run as "Copy setup details" gives it: run with `--nocapture` to
/// read it (WP1's manual check against the modeled organization).
#[tokio::test]
async fn a_whole_run_reads_as_setup_details() {
    let tenant = FakeTenant::fresh();
    let (server, session) = start(&tenant).await;
    session.log().start(RunHeader {
        tenant_id: Some(TENANT.into()),
        client_id: Some(CLIENT.into()),
        admin: None,
    });
    session.log().set_admin(ADMIN);
    let outcome = set_up(&session, &choices(true, &[CONFIDENTIAL])).await;
    assert!(
        outcome
            .result
            .changes
            .iter()
            .all(|c| c.outcome == OrgSetupOutcome::Done)
    );
    let text = details_text(session.log());
    println!("{text}");

    let host = Url::parse(&server.uri())
        .map(|u| format!("{}:{}", u.host_str().unwrap(), u.port().unwrap()))
        .unwrap();
    for expected in [
        format!(
            "Cumulas Mail {}: organization setup details\n",
            env!("CARGO_PKG_VERSION")
        ),
        format!("Tenant: {TENANT}\nSetup app: Cumulas Mail - Admin Setup ({CLIENT})\nSigned in as: {ADMIN}\n"),
        format!("  Read mail flow rules\n  Get-TransportRule (ResultSize) on {host} · anchor UPN · 200 · "),
        "Security & Compliance token: https://ps.compliance.protection.outlook.com/.default issued"
            .to_owned(),
        format!("Security & Compliance host: {host} (global, anchor UPN)"),
        "  Create “Cumulas Mail: Encrypt”\n  New-TransportRule (Name='Cumulas Mail: Encrypt', \
         ApplyRightsProtectionTemplate='Encrypt', Comments='Managed by Cumulas Mail"
            .to_owned(),
        "ExceptIfFromScope='NotInOrganization', HeaderMatchesMessageHeader='x-cumulas-protect', \
         HeaderMatchesPatterns=@('^encrypt$'), Mode='Enforce', RemoveHeader='x-cumulas-protect', \
         RuleErrorAction='Defer') on "
            .to_owned(),
        "  What Microsoft 365 returned:\n  Initial domain: contoso.onmicrosoft.com\n".to_owned(),
        "  Mail flow rules, by priority:\n    0  Cumulas Mail: ignore label requests from outside · Enabled · Enforce\n"
            .to_owned(),
        "Labels: Confidential (Guid 6c1b0f3e-1111-4a8b-9c2d-000000000001".to_owned(),
        "  Cumulas Mail's objects as Microsoft 365 returned them:\n".to_owned(),
    ] {
        assert!(text.contains(&expected), "missing {expected:?} in\n{text}");
    }
    // In the order things happened: the region is found, then its token
    // and host are noted.
    let found = text
        .find("  Find the Security & Compliance host\n")
        .unwrap();
    let token = text.find("Security & Compliance token:").unwrap();
    assert!(found < token, "{text}");
    // Reads that name nothing of ours carry no values; no token is there.
    assert!(text.contains("Get-AcceptedDomain on "), "{text}");
    for secret in ["exchange-token", "refresh-0", "refresh-rotated", "Bearer"] {
        assert!(!text.contains(secret), "{secret} in\n{text}");
    }
}
