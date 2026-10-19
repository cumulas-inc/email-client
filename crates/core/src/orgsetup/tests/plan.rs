//! The planner, on modeled organizations (no network).

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::{Map, Value, json};

use super::tenant::{FakeTenant, label, rule};
use super::{CONFIDENTIAL, GENERAL, SECRET, ctx, snapshot_of};
use crate::model::{OrgSetupAction, OrgSetupChoices, OrgSetupObject};
use crate::orgsetup::invoke::ErrorDetail;
use crate::orgsetup::plan::{
    self, LabelOutcome, ObjectKind, Placement, Planned, RuleOutcome, STRIP_FIRST, addressed,
    owned_target, unlicensed,
};
use crate::orgsetup::status::{FailureKind, Part, ReadFailure, Snapshot};
use crate::orgsetup::wanted::{
    DNF_RULE, ENCRYPT_RULE, MARKER, OwnRule, RELAY_INBOUND, RELAY_RULE, STRIP_RULE, is_owned,
    validate,
};
use crate::providers::graph::LicenseHint;

fn choices(actions: bool, labels: &[&str]) -> OrgSetupChoices {
    OrgSetupChoices {
        actions,
        encrypt_template: Some("Encrypt".into()),
        do_not_forward_template: Some("Do Not Forward".into()),
        label_ids: labels.iter().map(|l| (*l).to_owned()).collect(),
        labels_without_license: false,
    }
}

fn planned(tenant: &FakeTenant, choices: &OrgSetupChoices) -> Vec<Planned> {
    plan::assess(&snapshot_of(tenant), &ctx(), choices).planned
}

fn ids(planned: &[Planned]) -> Vec<String> {
    planned.iter().map(|p| p.change.id.clone()).collect()
}

fn params(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

/// Our 0.4.0 Encrypt rule, as the guide made it.
fn old_encrypt() -> Value {
    rule(
        ENCRYPT_RULE,
        json!({
            "FromScope": "InOrganization", "HeaderMatchesMessageHeader": "x-cumulas-protect",
            "HeaderMatchesPatterns": ["^encrypt$"], "HeaderContainsMessageHeader": "X-Signature-Stamped",
            "HeaderContainsWords": ["1"], "ApplyRightsProtectionTemplate": "Encrypt",
            "RemoveHeader": "x-cumulas-protect",
        }),
    )
}

/// A rule of ours as setup makes it.
fn good(name: &str) -> Value {
    match name {
        STRIP_RULE => rule(
            STRIP_RULE,
            json!({ "ExceptIfFromScope": "InOrganization", "RemoveHeader": "x-cumulas-label" }),
        ),
        _ => {
            let (pattern, template) = if name == ENCRYPT_RULE {
                ("^encrypt$", "Encrypt")
            } else {
                ("^do-not-forward$", "Do Not Forward")
            };
            rule(
                name,
                json!({
                    "ExceptIfFromScope": "NotInOrganization", "HeaderMatchesMessageHeader": "x-cumulas-protect",
                    "HeaderMatchesPatterns": [pattern], "ApplyRightsProtectionTemplate": template,
                    "RemoveHeader": "x-cumulas-protect", "RuleErrorAction": "Defer",
                }),
            )
        }
    }
}

fn relay_rule() -> Value {
    rule(
        RELAY_RULE,
        json!({
            "FromScope": "InOrganization", "StopRuleProcessing": true,
            "RouteMessageOutboundConnector": "Email signatures (to signature server)",
        }),
    )
}

fn relay_inbound(internal: bool) -> Value {
    json!({ "Name": RELAY_INBOUND, "ConnectorType": "OnPremises", "TreatMessagesAsInternal": internal })
}

#[test]
fn a_fresh_organization_gets_everything_chosen() {
    let tenant = FakeTenant::fresh();
    let planned = planned(&tenant, &choices(true, &[CONFIDENTIAL]));
    assert_eq!(
        ids(&planned),
        [
            "create:mailFlowRule:cumulas mail: ignore label requests from outside",
            "create:mailFlowRule:cumulas mail: encrypt",
            "create:mailFlowRule:cumulas mail: do not forward",
            "create:autoLabelingPolicy:cumulas mail: confidential",
            "create:autoLabelingRule:cumulas mail: confidential (header)",
            "startSimulation:autoLabelingPolicy:cumulas mail: confidential",
        ]
    );
    let call = |i: usize| (planned[i].calls[0].name, planned[i].calls[0].params.clone());
    assert_eq!(
        call(0),
        (
            "New-TransportRule",
            params(json!({
                "Name": STRIP_RULE, "ExceptIfFromScope": "InOrganization", "RemoveHeader": "x-cumulas-label",
                "Priority": 0, "Mode": "Enforce", "Comments": MARKER,
            }))
        )
    );
    // Nothing blocks: Encrypt lands last, no Priority.
    assert_eq!(
        call(1),
        (
            "New-TransportRule",
            params(json!({
                "Name": ENCRYPT_RULE, "ExceptIfFromScope": "NotInOrganization",
                "HeaderMatchesMessageHeader": "x-cumulas-protect", "HeaderMatchesPatterns": ["^encrypt$"],
                "ApplyRightsProtectionTemplate": "Encrypt", "RemoveHeader": "x-cumulas-protect",
                "RuleErrorAction": "Defer", "Mode": "Enforce", "Comments": MARKER,
            }))
        )
    );
    assert_eq!(
        call(2).1["HeaderMatchesPatterns"],
        json!(["^do-not-forward$"])
    );
    assert_eq!(
        call(2).1["ApplyRightsProtectionTemplate"],
        json!("Do Not Forward")
    );
    assert_eq!(
        call(3),
        (
            "New-AutoSensitivityLabelPolicy",
            params(json!({
                "Name": "Cumulas Mail: Confidential", "ApplySensitivityLabel": CONFIDENTIAL,
                "ExchangeLocation": ["All"], "OverwriteLabel": true, "Mode": "TestWithoutNotifications",
                "Comment": MARKER,
            }))
        )
    );
    assert_eq!(
        call(4),
        (
            "New-AutoSensitivityLabelRule",
            params(json!({
                "Name": "Cumulas Mail: Confidential (header)", "Policy": "Cumulas Mail: Confidential",
                "Workload": "Exchange",
                "HeaderMatchesPatterns": { "@odata.type": "#Exchange.GenericHashTable", "x-cumulas-label": format!("(?i)^({CONFIDENTIAL})$") },
            }))
        )
    );
    assert_eq!(
        call(5),
        (
            "Set-AutoSensitivityLabelPolicy",
            params(json!({
                "Identity": "Cumulas Mail: Confidential", "StartSimulation": true, "AutoEnableAfter": "01:00:00",
            }))
        )
    );
    // The disclosure comes with the first label change; each change carries
    // its PowerShell.
    assert_eq!(planned[3].change.details[0], plan::LABEL_DISCLOSURE);
    assert!(planned[3].needs_strip_rule && !planned[0].needs_strip_rule);
    assert!(
        planned[1]
            .change
            .powershell
            .starts_with("New-TransportRule -Name 'Cumulas Mail: Encrypt'")
    );
    // The same snapshot and choices give the same changes.
    assert_eq!(
        ids(&planned),
        ids(&self::planned(&tenant, &choices(true, &[CONFIDENTIAL])))
    );
    // Without labels chosen, the strip rule isn't needed.
    assert_eq!(planned_count(&tenant, &choices(true, &[])), 2);
}

fn planned_count(tenant: &FakeTenant, choices: &OrgSetupChoices) -> usize {
    planned(tenant, choices).len()
}

#[test]
fn the_old_encrypt_rule_is_repaired_in_one_change() {
    let tenant = FakeTenant::fresh();
    tenant.with(|s| s.rules = vec![old_encrypt(), good(DNF_RULE)]);
    let guid = tenant.rule(ENCRYPT_RULE).unwrap()["Guid"].clone();
    let planned = planned(&tenant, &choices(true, &[]));
    assert_eq!(ids(&planned), ["change:mailFlowRule:cumulas mail: encrypt"]);
    assert_eq!(
        planned[0].calls[0].params,
        params(json!({
            "Identity": guid, "FromScope": null, "ExceptIfFromScope": "NotInOrganization",
            "HeaderContainsMessageHeader": null, "HeaderContainsWords": null,
            "RuleErrorAction": "Defer", "Comments": MARKER,
        }))
    );
    assert_eq!(planned[0].calls[0].name, "Set-TransportRule");
    let details = &planned[0].change.details;
    assert!(details.contains(&"Applies to mail from your organization's addresses (it applied only to mail Microsoft 365 had authenticated)".to_owned()), "{details:?}");
    assert!(
        details.contains(
            &"No longer waits for X-Signature-Stamped (from the earlier Cumulas Signatures recipe)"
                .to_owned()
        )
    );
    assert!(details.contains(&"Holds the message and tries again if encryption fails, instead of sending it unencrypted".to_owned()));
}

#[test]
fn test_mode_and_turned_off_rules_are_turned_on() {
    let tenant = FakeTenant::fresh();
    let mut dnf = good(DNF_RULE);
    dnf["Mode"] = json!("Audit");
    let mut encrypt = good(ENCRYPT_RULE);
    encrypt["State"] = json!("Disabled");
    tenant.with(|s| s.rules = vec![encrypt, dnf]);
    let planned = planned(&tenant, &choices(true, &[]));
    assert_eq!(
        ids(&planned),
        [
            "turnOn:mailFlowRule:cumulas mail: encrypt",
            "change:mailFlowRule:cumulas mail: do not forward",
        ]
    );
    assert_eq!(planned[0].calls[0].name, "Enable-TransportRule");
    assert_eq!(planned[1].calls[0].params["Mode"], json!("Enforce"));
    assert_eq!(
        planned[1].change.details,
        ["Turns from test mode (Audit) to Enforce"]
    );
}

/// Where Encrypt goes (or moves to) among `rules`, with the relay's return
/// internal or not.
fn placed(rules: Vec<Value>, internal: Option<bool>) -> (Vec<Planned>, Placement) {
    let tenant = FakeTenant::fresh();
    tenant.with(|s| {
        s.rules = rules;
        s.inbound = internal.map(relay_inbound).into_iter().collect();
    });
    let snapshot = snapshot_of(&tenant);
    let assessment = plan::assess(&snapshot, &ctx(), &choices(true, &[]));
    let placement = match &assessment.rule(OwnRule::Encrypt).outcome {
        RuleOutcome::Assessed { placement, .. } => placement.clone(),
        _ => Placement::Stay,
    };
    (assessment.planned, placement)
}

fn moves(planned: &[Planned], name: &str) -> Option<(i64, Vec<String>)> {
    planned
        .iter()
        .find(|p| p.change.action == OrgSetupAction::Move && p.change.name == name)
        .map(|p| {
            (
                p.calls[0].params["Priority"].as_i64().unwrap(),
                p.change.details.clone(),
            )
        })
}

#[test]
fn encrypt_and_do_not_forward_go_above_rules_that_stop_or_route() {
    let stopper = rule(
        "Contoso: stop for partner.com",
        json!({ "StopRuleProcessing": true, "RecipientDomainIs": ["partner.example"] }),
    );
    let gateway = rule(
        "Contoso: gateway",
        json!({ "RouteMessageOutboundConnector": "Mimecast", "SentToScope": "NotInOrganization" }),
    );
    // No relay, nothing above: they stay.
    let (planned, placement) = placed(
        vec![good(ENCRYPT_RULE), good(DNF_RULE), stopper.clone()],
        None,
    );
    assert!(planned.is_empty(), "{:?}", ids(&planned));
    assert_eq!(placement, Placement::Stay);

    // A foreign stopper above: both move above it, which is named.
    let (planned, _) = placed(
        vec![stopper.clone(), good(ENCRYPT_RULE), good(DNF_RULE)],
        None,
    );
    let (to, details) = moves(&planned, ENCRYPT_RULE).unwrap();
    assert_eq!(to, 0);
    assert_eq!(
        details,
        [
            "Moves above “Contoso: stop for partner.com”, which stops later rules for mail it matches, so it acts on that mail first"
        ]
    );
    assert_eq!(moves(&planned, DNF_RULE).unwrap().0, 1);

    // A gateway that routes outside mail: above it too.
    let (planned, _) = placed(vec![gateway, good(ENCRYPT_RULE), good(DNF_RULE)], None);
    let (to, details) = moves(&planned, ENCRYPT_RULE).unwrap();
    assert_eq!(to, 0);
    assert!(
        details[0].contains("which sends mail out through another service"),
        "{details:?}"
    );

    // The relay, its return not internal: above the relay.
    let (planned, _) = placed(
        vec![relay_rule(), good(ENCRYPT_RULE), good(DNF_RULE)],
        Some(false),
    );
    let (to, details) = moves(&planned, ENCRYPT_RULE).unwrap();
    assert_eq!(to, 0);
    assert_eq!(
        details,
        [
            "Moves above “Email signatures: add signature”, because mail coming back from the relay doesn't count as internal: encrypted mail then goes out without a signature"
        ]
    );

    // Its return internal and ours above it: right after it.
    let (planned, _) = placed(
        vec![good(ENCRYPT_RULE), good(DNF_RULE), relay_rule()],
        Some(true),
    );
    let (to, details) = moves(&planned, ENCRYPT_RULE).unwrap();
    assert_eq!(to, 2);
    assert!(details[0].starts_with("Moves after “Email signatures: add signature”"));
    assert_eq!(moves(&planned, DNF_RULE).unwrap().0, 2);

    // Its return internal, ours right after it already: they stay.
    let (planned, _) = placed(
        vec![relay_rule(), good(ENCRYPT_RULE), good(DNF_RULE)],
        Some(true),
    );
    assert!(planned.is_empty(), "{:?}", ids(&planned));

    // A foreign stopper above the internal-return relay: a conflict.
    let (planned, placement) = placed(
        vec![stopper, relay_rule(), good(ENCRYPT_RULE), good(DNF_RULE)],
        Some(true),
    );
    assert!(planned.is_empty(), "{:?}", ids(&planned));
    match placement {
        Placement::Conflict {
            to,
            rule,
            conditions,
            ..
        } => {
            assert_eq!(to, None);
            assert_eq!(rule, "Contoso: stop for partner.com");
            assert_eq!(conditions, "RecipientDomainIs: partner.example");
        }
        other => panic!("{other:?}"),
    }

    // New rules go straight to their place.
    let (planned, _) = placed(vec![relay_rule()], Some(false));
    assert_eq!(planned[0].calls[0].params["Priority"], json!(0));
    assert_eq!(planned[1].calls[0].params["Priority"], json!(1));
    // After an internal-return relay with nothing else in the way, they
    // land last, which is after it.
    let (planned, _) = placed(vec![relay_rule(), rule("Other", json!({}))], Some(true));
    assert!(!planned[0].calls[0].params.contains_key("Priority"));
    assert!(!planned[1].calls[0].params.contains_key("Priority"));
    // With a stopper below it: right after it, Do Not Forward below Encrypt.
    let (planned, _) = placed(
        vec![
            relay_rule(),
            rule("Contoso: stop", json!({ "StopRuleProcessing": true })),
        ],
        Some(true),
    );
    assert_eq!(planned[0].calls[0].params["Priority"], json!(1));
    assert_eq!(planned[1].calls[0].params["Priority"], json!(2));
}

#[test]
fn the_strip_rule_moves_first_when_a_stopper_comes_before_it() {
    let tenant = FakeTenant::fresh();
    tenant.with(|s| {
        s.rules = vec![
            relay_rule(),
            rule("Contoso: stop", json!({ "StopRuleProcessing": true })),
            rule("Contoso: other", json!({})),
            good(STRIP_RULE),
        ]
    });
    let planned = planned(&tenant, &choices(false, &[CONFIDENTIAL]));
    assert_eq!(
        planned[0].change.id,
        "move:mailFlowRule:cumulas mail: ignore label requests from outside"
    );
    assert_eq!(planned[0].calls[0].params["Priority"], json!(0));
    assert_eq!(
        planned[0].change.details,
        ["Moves to the top, so no other rule can stop it first"]
    );
    // The relay's rule (internal mail only) may come before it.
    tenant.with(|s| s.rules = vec![relay_rule(), good(STRIP_RULE)]);
    let planned = self::planned(&tenant, &choices(false, &[CONFIDENTIAL]));
    assert!(
        !planned
            .iter()
            .any(|p| p.change.object == OrgSetupObject::MailFlowRule)
    );
}

#[test]
fn ours_with_settings_we_dont_set_are_left_alone_and_counted() {
    let tenant = FakeTenant::fresh();
    let mut encrypt = old_encrypt();
    encrypt["BlindCopyTo"] = json!(["x@evil.example"]);
    tenant.with(|s| s.rules = vec![encrypt, good(DNF_RULE)]);
    let assessment = plan::assess(&snapshot_of(&tenant), &ctx(), &choices(true, &[]));
    assert!(assessment.planned.is_empty());
    assert_eq!(assessment.blocked, 1);
    assert!(
        matches!(&assessment.rule(OwnRule::Encrypt).outcome, RuleOutcome::Blocked(extras) if extras[0].0 == "BlindCopyTo")
    );

    // A policy with a second rule, or with a SharePoint location.
    for (extra_rule, sharepoint, what) in [
        (true, false, "another rule"),
        (false, true, "SharePoint or OneDrive locations"),
    ] {
        let tenant = FakeTenant::fresh();
        tenant.with(|s| {
            s.rules = vec![good(STRIP_RULE)];
            s.policies = vec![json!({
                "Name": "Cumulas Mail: Confidential", "Mode": "Enable", "ApplySensitivityLabel": CONFIDENTIAL,
                "ExchangeLocation": ["All"], "OverwriteLabel": true,
                "SharePointLocation": if sharepoint { json!(["All"]) } else { json!([]) },
            })];
            s.policy_rules = vec![json!({
                "Name": "Cumulas Mail: Confidential (header)", "ParentPolicyName": "Cumulas Mail: Confidential",
                "Workload": "Exchange", "HeaderMatchesPatterns": { "x-cumulas-label": format!("(?i)^({CONFIDENTIAL})$") },
            })];
            if extra_rule {
                s.policy_rules.push(json!({ "Name": "Someone else's", "ParentPolicyName": "Cumulas Mail: Confidential" }));
            }
        });
        let assessment = plan::assess(
            &snapshot_of(&tenant),
            &ctx(),
            &choices(false, &[CONFIDENTIAL]),
        );
        assert!(
            assessment.planned.is_empty(),
            "{what}: {:?}",
            ids(&assessment.planned)
        );
        assert_eq!(assessment.blocked, 1);
        let label = assessment
            .labels
            .iter()
            .find(|l| l.id == CONFIDENTIAL)
            .unwrap();
        assert_eq!(label.outcome, LabelOutcome::Blocked(what));
    }
}

#[test]
fn labels_wait_for_label_requests_from_outside_to_be_removed() {
    // An S&C-only admin: Exchange didn't read.
    let tenant = FakeTenant::fresh();
    let mut snapshot = snapshot_of(&tenant);
    snapshot.rules = Part::Failed(ReadFailure {
        kind: FailureKind::NoRole,
        detail: ErrorDetail::default(),
    });
    let assessment = plan::assess(&snapshot, &ctx(), &choices(false, &[CONFIDENTIAL]));
    assert!(assessment.planned.is_empty());
    assert_eq!(assessment.problems, [STRIP_FIRST]);

    // The strip rule is ours but blocked.
    let mut strip = good(STRIP_RULE);
    strip["CopyTo"] = json!(["x@evil.example"]);
    tenant.with(|s| s.rules = vec![strip]);
    let assessment = plan::assess(
        &snapshot_of(&tenant),
        &ctx(),
        &choices(false, &[CONFIDENTIAL]),
    );
    assert!(assessment.planned.is_empty());
    assert!(assessment.problems.contains(&STRIP_FIRST.to_owned()));

    // Unlicensed: no label changes unless chosen anyway.
    let tenant = FakeTenant::fresh();
    let mut snapshot = snapshot_of(&tenant);
    snapshot.licensing = Part::Read(LicenseHint {
        all: vec!["SPE_E3".into()],
        ..LicenseHint::default()
    });
    let assessment = plan::assess(&snapshot, &ctx(), &choices(true, &[CONFIDENTIAL]));
    assert_eq!(assessment.problems, [unlicensed("adele@contoso.com")]);
    assert!(
        assessment
            .planned
            .iter()
            .all(|p| p.change.object == OrgSetupObject::MailFlowRule)
    );
    assert_eq!(assessment.planned.len(), 2);
    let anyway = OrgSetupChoices {
        labels_without_license: true,
        ..choices(true, &[CONFIDENTIAL])
    };
    let assessment = plan::assess(&snapshot, &ctx(), &anyway);
    assert!(assessment.problems.is_empty());
    assert_eq!(assessment.planned.len(), 6);
}

#[test]
fn nothing_but_ours_is_ever_planned() {
    let tenant = FakeTenant::fresh();
    tenant.with(|s| {
        s.rules = vec![
            rule(
                "Contoso: Encrypt",
                json!({ "HeaderMatchesMessageHeader": "x-cumulas-protect" }),
            ),
            rule(
                "Cumulas Mail: Confidential after the signature",
                json!({ "SetHeaderName": "x-cumulas-label-stamped" }),
            ),
            rule(
                "Cumulas Mail: ignore stamped label requests from outside",
                json!({ "RemoveHeader": "x-cumulas-label-stamped" }),
            ),
        ]
    });
    let planned = planned(&tenant, &choices(true, &[CONFIDENTIAL, GENERAL]));
    for p in &planned {
        assert!(is_owned(&p.change.name), "{}", p.change.name);
        assert!(!p.change.name.contains("after the signature"));
        assert!(!p.change.name.contains("stamped"));
        assert!(!p.change.name.starts_with("Contoso"));
    }
}

/// An organization with our three rules right and a policy for
/// Confidential in `mode`.
fn with_policy(policy: Value, rule_pattern: &str) -> FakeTenant {
    let tenant = FakeTenant::fresh();
    tenant.with(|s| {
        s.rules = vec![good(STRIP_RULE), good(ENCRYPT_RULE), good(DNF_RULE)];
        let mut base = json!({
            "Name": "Cumulas Mail: Confidential", "Mode": "Enable", "Enabled": true,
            "ApplySensitivityLabel": CONFIDENTIAL, "ExchangeLocation": ["All"], "OverwriteLabel": true,
            "WhenChanged": "2026-10-01T09:00:00Z",
        });
        for (k, v) in policy.as_object().unwrap() {
            base[k] = v.clone();
        }
        s.policies = vec![base];
        s.policy_rules = vec![json!({
            "Name": "Cumulas Mail: Confidential (header)", "ParentPolicyName": "Cumulas Mail: Confidential",
            "Workload": "Exchange", "HeaderMatchesPatterns": { "x-cumulas-label": rule_pattern },
            "Disabled": false, "WhenChanged": "2026-10-01T09:00:00Z",
        })];
    });
    tenant
}

#[test]
fn auto_labeling_policies_are_repaired_and_turned_on_as_documented() {
    let right = format!("(?i)^({CONFIDENTIAL})$");
    // All right and on: nothing.
    let tenant = with_policy(json!({}), &right);
    assert!(planned(&tenant, &choices(true, &[CONFIDENTIAL])).is_empty());

    // The pattern differs: the rule is changed, and the policy is on, so
    // apply looks whether it went back into simulation.
    let tenant = with_policy(json!({}), &format!("^{CONFIDENTIAL}$"));
    let planned = planned(&tenant, &choices(true, &[CONFIDENTIAL]));
    assert_eq!(
        ids(&planned),
        ["change:autoLabelingRule:cumulas mail: confidential (header)"]
    );
    assert_eq!(
        planned[0].calls[0].params["HeaderMatchesPatterns"],
        json!({ "@odata.type": "#Exchange.GenericHashTable", "x-cumulas-label": right })
    );
    assert_eq!(
        planned[0].restart_if_off.as_deref(),
        Some("Cumulas Mail: Confidential")
    );
    assert!(
        planned[0]
            .change
            .details
            .contains(&plan::BACK_TO_SIMULATION.to_owned())
    );

    // Left in simulation by the 0.4.0 guide, simulation complete, nothing
    // changed since: turned on.
    let tenant = with_policy(
        json!({ "Mode": "TestWithoutNotifications", "SimulationStatus": "Completed", "SimulationCompletedTime": "2026-10-02T09:00:00Z" }),
        &right,
    );
    let planned = self::planned(&tenant, &choices(true, &[CONFIDENTIAL]));
    assert_eq!(
        ids(&planned),
        ["turnOn:autoLabelingPolicy:cumulas mail: confidential"]
    );
    assert_eq!(
        planned[0].calls[0].params,
        params(json!({ "Identity": "Cumulas Mail: Confidential", "Mode": "Enable" }))
    );
    // Not complete (or no time): the simulation starts again, with its
    // turn-on time.
    let tenant = with_policy(json!({ "Mode": "TestWithoutNotifications" }), &right);
    let planned = self::planned(&tenant, &choices(true, &[CONFIDENTIAL]));
    assert_eq!(
        ids(&planned),
        ["startSimulation:autoLabelingPolicy:cumulas mail: confidential"]
    );
    assert_eq!(
        planned[0].calls[0].params["AutoEnableAfter"],
        json!("01:00:00")
    );
    // In simulation with its turn-on time already: left to turn itself on.
    let tenant = with_policy(
        json!({ "Mode": "TestWithoutNotifications", "AutoEnableAfter": "01:00:00" }),
        &right,
    );
    assert!(self::planned(&tenant, &choices(true, &[CONFIDENTIAL])).is_empty());

    // Turned off: Enabled true.
    let tenant = with_policy(json!({ "Enabled": false }), &right);
    let planned = self::planned(&tenant, &choices(true, &[CONFIDENTIAL]));
    assert_eq!(
        ids(&planned),
        ["turnOn:autoLabelingPolicy:cumulas mail: confidential"]
    );
    assert_eq!(planned[0].calls[0].params["Enabled"], json!(true));

    // Other differences: only what differs, with the marker.
    let tenant = with_policy(
        json!({ "OverwriteLabel": false, "ExchangeLocation": [], "ExchangeSenderMemberOf": ["pilot@contoso.com"] }),
        &right,
    );
    let planned = self::planned(&tenant, &choices(true, &[CONFIDENTIAL]));
    assert_eq!(
        planned[0].calls[0].params,
        params(json!({
            "Identity": "Cumulas Mail: Confidential", "AddExchangeLocation": ["All"],
            "OverwriteLabel": true, "Comment": MARKER,
        }))
    );
    // Labels not chosen are never touched.
    assert!(
        self::planned(&tenant, &choices(true, &[GENERAL]))
            .iter()
            .all(|p| !p.change.name.contains("Confidential"))
    );
}

#[test]
fn choices_naming_anything_but_a_label_are_refused() {
    let tenant = FakeTenant::fresh();
    let snapshot = snapshot_of(&tenant);
    for bad in [
        "not-a-guid",
        "x'; Remove-TransportRule *",
        "6c1b0f3e-1111-4a8b-9c2d-00000000ffff",
    ] {
        let error = validate(&choices(true, &[bad]), Some(&snapshot), &ctx()).unwrap_err();
        assert_eq!(error.code, crate::ErrorCode::InvalidInput, "{bad}");
        assert_eq!(error.message, "That label isn't one Purview lists.");
    }
    let ok = validate(
        &choices(true, &[&SECRET.to_uppercase()]),
        Some(&snapshot),
        &ctx(),
    )
    .unwrap();
    assert_eq!(ok.label_ids, [SECRET]);
    let error = validate(
        &OrgSetupChoices {
            encrypt_template: Some("Made up".into()),
            ..choices(true, &[])
        },
        Some(&snapshot),
        &ctx(),
    )
    .unwrap_err();
    assert_eq!(error.message, "That template isn't one Exchange lists.");
    let mut weird = ctx();
    weird.account_email = "not an address".into();
    assert!(validate(&choices(true, &[]), None, &weird).is_err());
    weird.account_email = "a'|iex#@contoso.com".into();
    assert!(validate(&choices(true, &[]), None, &weird).is_ok());
}

/// A random rule: ours (one of three, maybe with an extra), a leftover,
/// the relay's, or someone else's that may stop or route.
fn random_rule(rng: &mut StdRng, i: usize) -> Value {
    let name = match rng.random_range(0..8) {
        0 => STRIP_RULE.to_owned(),
        1 => ENCRYPT_RULE.to_owned(),
        2 => DNF_RULE.to_owned(),
        3 => RELAY_RULE.to_owned(),
        4 => "Cumulas Mail: X after the signature".to_owned(),
        5 => "Cumulas Mail Encrypt".to_owned(),
        _ => format!("Contoso: rule {i}"),
    };
    let mut fields = json!({});
    for (key, value) in [
        ("StopRuleProcessing", json!(true)),
        ("RouteMessageOutboundConnector", json!("Gateway")),
        ("FromScope", json!("InOrganization")),
        ("Mode", json!("Audit")),
        ("State", json!("Disabled")),
        ("BlindCopyTo", json!(["x@evil.example"])),
        ("HeaderContainsMessageHeader", json!("X-Signature-Stamped")),
        ("ApplyRightsProtectionTemplate", json!("Encrypt")),
    ] {
        if rng.random_bool(0.2) {
            fields[key] = value;
        }
    }
    if rng.random_bool(0.1) {
        fields["Guid"] = Value::Null;
    }
    rule(&name, fields)
}

fn random_tenant(rng: &mut StdRng) -> FakeTenant {
    let tenant = FakeTenant::fresh();
    tenant.with(|s| {
        let mut rules = Vec::new();
        for i in 0..rng.random_range(0..9) {
            let candidate = random_rule(rng, i);
            if !rules.iter().any(|r: &Value| r["Name"] == candidate["Name"]) {
                rules.push(candidate);
            }
        }
        s.rules = rules;
        if rng.random_bool(0.5) {
            s.inbound = vec![relay_inbound(rng.random_bool(0.5))];
        }
        for (guid, name) in [(CONFIDENTIAL, "Confidential"), (GENERAL, "General")] {
            if rng.random_bool(0.5) {
                let policy = if rng.random_bool(0.8) {
                    format!("Cumulas Mail: {name}")
                } else {
                    format!("Contoso: {name}")
                };
                let mode = ["Enable", "TestWithoutNotifications", "PendingDeletion"][rng.random_range(0..3)];
                s.policies.push(json!({
                    "Name": policy, "Mode": mode,
                    "Enabled": rng.random_bool(0.8), "ApplySensitivityLabel": guid,
                    "ExchangeLocation": if rng.random_bool(0.8) { json!(["All"]) } else { json!([]) },
                    "SharePointLocation": if rng.random_bool(0.1) { json!(["All"]) } else { json!([]) },
                    "OverwriteLabel": rng.random_bool(0.8),
                }));
                if rng.random_bool(0.7) {
                    s.policy_rules.push(json!({
                        "Name": format!("{policy} (header)"), "ParentPolicyName": policy,
                        "Workload": "Exchange", "Disabled": rng.random_bool(0.2),
                        "HeaderMatchesPatterns": { "x-cumulas-label": if rng.random_bool(0.5) { format!("(?i)^({guid})$") } else { "^x$".to_owned() } },
                    }));
                }
            }
        }
        if rng.random_bool(0.2) {
            s.labels.push(label("6c1b0f3e-1111-4a8b-9c2d-0000000000aa", &"Very long label name ".repeat(4)));
        }
    });
    tenant
}

/// For every call the planner can make: its target is ours in the snapshot
/// it was planned from (or, for a new object, an unlisted name of ours),
/// every move is on ours, and nothing is ever removed.
fn check_ownership(snapshot: &Snapshot, planned: &[Planned], seed: u64) {
    let mut created: Vec<(ObjectKind, &str)> = Vec::new();
    for (call, kind) in addressed(planned) {
        assert!(
            !call.name.starts_with("Remove-"),
            "seed {seed}: {}",
            call.name
        );
        assert!(
            !call.name.starts_with("Disable-"),
            "seed {seed}: {}",
            call.name
        );
        if call.name.starts_with("New-") {
            let name = call.new_name().unwrap();
            assert!(is_owned(name), "seed {seed}: creates {name}");
            created.push((kind, name));
            continue;
        }
        let identity = call
            .identity()
            .unwrap_or_else(|| panic!("seed {seed}: {call:?}"));
        if created.contains(&(kind, identity)) {
            continue;
        }
        let target = owned_target(identity, kind, snapshot)
            .unwrap_or_else(|e| panic!("seed {seed}: {} ({identity}): {}", call.name, e.message));
        assert!(is_owned(target), "seed {seed}: {target}");
        if kind == ObjectKind::MailFlowRule {
            assert!(!target.contains("after the signature"), "seed {seed}");
        }
    }
}

#[test]
fn every_planned_call_addresses_only_our_own_objects() {
    let start: u64 = std::env::var("ORGSETUP_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x5eed);
    for seed in start..start + 400 {
        let mut rng = StdRng::seed_from_u64(seed);
        let tenant = random_tenant(&mut rng);
        let snapshot = snapshot_of(&tenant);
        let labels: Vec<&str> = [CONFIDENTIAL, GENERAL, SECRET]
            .into_iter()
            .filter(|_| rng.random_bool(0.6))
            .collect();
        let choices = OrgSetupChoices {
            labels_without_license: rng.random_bool(0.5),
            ..choices(rng.random_bool(0.7), &labels)
        };
        let assessment = plan::assess(&snapshot, &ctx(), &choices);
        check_ownership(&snapshot, &assessment.planned, seed);
    }
}
