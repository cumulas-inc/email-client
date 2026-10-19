//! Reading cmdlet output defensively.

use chrono::{TimeZone, Utc};
use serde_json::{Value, json};

use super::fixture;
use super::tenant::rule;
use crate::orgsetup::objects::{
    AutoPolicyView, AutoRuleView, DistributionState, LabelView, SimulationState, TransportRuleView,
    date, header_patterns,
};
use crate::orgsetup::wanted::{COMPANION_DEFAULTS, EXTRA_PARAMETERS, WANTED_RULE_PARAMETERS};

fn rules() -> Vec<TransportRuleView> {
    fixture("transport-rules.json")["value"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| TransportRuleView::read(v).unwrap())
        .collect()
}

#[test]
fn rules_read_in_every_shape_microsoft_may_send() {
    let rules = rules();
    let encrypt = &rules[0];
    assert_eq!(encrypt.name, "Cumulas Mail: Encrypt");
    assert_eq!(
        encrypt.guid.as_deref(),
        Some("3f6c2a1e-0000-4000-8000-000000000001")
    );
    assert_eq!(encrypt.priority, Some(2));
    assert_eq!(encrypt.state.as_deref(), Some("Enabled"));
    assert_eq!(encrypt.from_scope.as_deref(), Some("InOrganization"));
    assert_eq!(encrypt.except_if_from_scope, None);
    assert_eq!(encrypt.header_matches_patterns, ["^encrypt$"]);
    assert_eq!(
        encrypt.header_contains_header.as_deref(),
        Some("X-Signature-Stamped")
    );
    assert_eq!(encrypt.header_contains_words, ["1"]);
    assert_eq!(encrypt.template.as_deref(), Some("Encrypt"));
    assert_eq!(encrypt.stop_rule_processing, Some(false));
    assert_eq!(encrypt.rule_error_action.as_deref(), Some("Ignore"));
    assert_eq!(
        encrypt.when_changed,
        Some(Utc.with_ymd_and_hms(2026, 10, 18, 9, 12, 44).unwrap())
    );
    assert!(encrypt.extras.is_empty(), "{:?}", encrypt.extras);

    let dnf = &rules[1];
    // A number where a name was expected doesn't read: unknown.
    assert_eq!(dnf.state, None);
    assert_eq!(dnf.priority, Some(3));
    assert_eq!(dnf.mode.as_deref(), Some("Audit"));
    assert_eq!(dnf.header_matches_patterns, ["^do-not-forward$"]);
    assert_eq!(dnf.template.as_deref(), Some("Do Not Forward"));
    assert_eq!(dnf.stop_rule_processing, Some(false));
    assert_eq!(
        dnf.when_changed,
        Utc.timestamp_millis_opt(1_760_778_764_000).single()
    );
    // Companion settings at their defaults are no extras.
    assert!(dnf.extras.is_empty(), "{:?}", dnf.extras);
    assert!(!dnf.active(), "in test mode");
    let mut unread = fixture("transport-rules.json")["value"][1].clone();
    unread["Mode"] = json!("Enforce");
    unread["StopRuleProcessing"] = json!(true);
    let unread = TransportRuleView::read(&unread).unwrap();
    assert!(
        unread.active() && unread.stops(),
        "unreadable state counts as on"
    );

    let relay = &rules[2];
    assert!(relay.stops() && relay.routes() && relay.blocks());
    let stopper = &rules[3];
    assert!(stopper.stops() && !stopper.routes());
    assert_eq!(
        stopper.recipient_conditions(),
        [("RecipientDomainIs".to_owned(), "partner.example".to_owned())]
    );
    assert!(TransportRuleView::read(&json!({ "Guid": "x" })).is_none());
}

#[test]
fn settings_we_never_set_are_extras_and_defaults_are_not() {
    for (parameter, value) in [
        ("BlindCopyTo", json!(["x@evil.example"])),
        ("RedirectMessageTo", json!(["x@evil.example"])),
        ("AddToRecipients", json!("x@evil.example")),
        ("CopyTo", json!(["x@evil.example"])),
        ("ModerateMessageByUser", json!(["boss@contoso.com"])),
        ("RouteMessageOutboundConnector", json!("Gateway")),
        ("ExceptIfRecipientDomainIs", json!(["partner.example"])),
        ("SentTo", json!(["someone@contoso.com"])),
        ("ActivationDate", json!("2026-11-01T00:00:00Z")),
        ("DeleteMessage", json!(true)),
        ("SetSCL", json!(5)),
    ] {
        let view =
            TransportRuleView::read(&rule("Cumulas Mail: Encrypt", json!({ parameter: value })))
                .unwrap();
        assert_eq!(
            view.extras
                .iter()
                .map(|(p, _)| p.as_str())
                .collect::<Vec<_>>(),
            [parameter],
            "{parameter}"
        );
    }
    // A fresh rule of ours, and every empty form.
    let fresh = TransportRuleView::read(&rule(
        "Cumulas Mail: Encrypt",
        json!({
            "ExceptIfFromScope": "NotInOrganization", "HeaderMatchesMessageHeader": "x-cumulas-protect",
            "HeaderMatchesPatterns": ["^encrypt$"], "ApplyRightsProtectionTemplate": "Encrypt",
            "RemoveHeader": "x-cumulas-protect", "RuleErrorAction": "Defer",
            "DeleteMessage": false, "CopyTo": null, "SentTo": "", "BlindCopyTo": [], "IncidentReportContent": {},
            "RecipientAddressType": "Resolved", "UseLegacyRegex": false,
        }),
    ))
    .unwrap();
    assert!(fresh.extras.is_empty(), "{:?}", fresh.extras);

    // Type lists, when the output has them (R18).
    let typed = TransportRuleView::read(&rule(
        "Cumulas Mail: Encrypt",
        json!({
            "Conditions": ["Microsoft.Exchange.MessagingPolicies.Rules.Tasks.HeaderMatchesPredicate"],
            "Exceptions": [{ "Type": "FromScopePredicate" }],
            "Actions": ["RightsProtectMessageAction", "RemoveHeaderAction", "BlindCopyToAction"],
        }),
    ))
    .unwrap();
    assert_eq!(
        typed.extras,
        [("Actions".to_owned(), "BlindCopyToAction".to_owned())]
    );
    assert_eq!(typed.kinds.as_ref().map(Vec::len), Some(5));
}

#[test]
fn the_extras_list_never_names_what_we_set() {
    for wanted in WANTED_RULE_PARAMETERS {
        assert!(!EXTRA_PARAMETERS.contains(wanted), "{wanted}");
    }
    for (companion, _) in COMPANION_DEFAULTS {
        assert!(!EXTRA_PARAMETERS.contains(companion), "{companion}");
    }
    for needed in [
        "BlindCopyTo",
        "RedirectMessageTo",
        "AddToRecipients",
        "CopyTo",
        "ModerateMessageByUser",
        "RouteMessageOutboundConnector",
        "ExceptIfRecipientDomainIs",
        "SentTo",
        "ActivationDate",
        "ExpiryDate",
    ] {
        assert!(EXTRA_PARAMETERS.contains(&needed), "{needed}");
    }
    let mut sorted = EXTRA_PARAMETERS.to_vec();
    sorted.dedup();
    assert_eq!(sorted.len(), EXTRA_PARAMETERS.len());
}

#[test]
fn labels_policies_and_rules_read_defensively() {
    let fixture = fixture("auto-labeling.json");
    let labels: Vec<LabelView> = fixture["labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(LabelView::read)
        .collect();
    assert_eq!(labels[0].guid, "6c1b0f3e-1111-4a8b-9c2d-000000000001");
    assert_eq!(
        labels[0].immutable_id.as_deref(),
        Some("9d0e0c1b-2222-4c3d-8e4f-000000000001")
    );
    assert_eq!(labels[0].for_email(), Some(true));
    assert!(labels[0].encrypts());
    assert!(labels[0].is("9D0E0C1B-2222-4C3D-8E4F-000000000001"));
    assert_eq!(labels[1].for_email(), Some(false));
    assert_eq!(labels[2].for_email(), None);

    let policies: Vec<AutoPolicyView> = fixture["policies"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(AutoPolicyView::read)
        .collect();
    let confidential = &policies[0];
    assert_eq!(confidential.enabled, Some(true));
    assert_eq!(confidential.exchange_location, ["All"]);
    assert!(confidential.other_locations.is_empty());
    assert_eq!(
        confidential.sender_member_of,
        ["cumulas-mail-users@contoso.com"]
    );
    assert_eq!(confidential.auto_enable_after.as_deref(), Some("01:00:00"));
    assert_eq!(
        confidential.simulation,
        SimulationState::Complete {
            at: Some(Utc.with_ymd_and_hms(2026, 10, 18, 12, 0, 0).unwrap())
        }
    );
    assert_eq!(confidential.distribution, DistributionState::Success);
    assert_eq!(
        confidential.when_changed,
        Utc.timestamp_millis_opt(1_760_778_764_000).single()
    );
    let shared = &policies[1];
    assert!(shared.is_on());
    assert_eq!(
        shared.other_locations,
        [("SharePointLocation".to_owned(), "All".to_owned())]
    );
    assert_eq!(
        shared.distribution,
        DistributionState::Error("Exchange: policy sync failed".into())
    );
    assert_eq!(shared.simulation, SimulationState::Unknown);

    let rules: Vec<AutoRuleView> = fixture["rules"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(AutoRuleView::read)
        .collect();
    assert_eq!(
        rules[0].policy.as_deref(),
        Some("Cumulas Mail: Confidential")
    );
    assert_eq!(
        rules[0].header_patterns,
        [(
            "x-cumulas-label".to_owned(),
            "^6c1b0f3e-1111-4a8b-9c2d-000000000001$".to_owned()
        )]
    );
    assert_eq!(rules[0].disabled, Some(false));
    assert!(rules[0].other_conditions.is_empty());
    assert_eq!(rules[1].policy.as_deref(), Some("Cumulas Mail: Shared"));
    assert_eq!(
        rules[1].header_patterns,
        [
            (
                "x-cumulas-label-stamped".to_owned(),
                "(?i)^(a|b)$".to_owned()
            ),
            ("x-cumulas-label-stamped".to_owned(), "^c$".to_owned()),
        ]
    );
    assert_eq!(rules[1].other_conditions, ["SentTo"]);
    assert_eq!(
        rules[2].header_patterns,
        [("x-cumulas-label".to_owned(), "^x$".to_owned())]
    );
}

#[test]
fn simulation_and_distribution_under_other_names() {
    let read = |fields: Value| {
        let mut policy = json!({ "Name": "Cumulas Mail: X" });
        for (k, v) in fields.as_object().unwrap() {
            policy[k] = v.clone();
        }
        AutoPolicyView::read(&policy).unwrap()
    };
    assert_eq!(
        read(json!({ "SimulationStatus": "InProgress" })).simulation,
        SimulationState::Running
    );
    assert_eq!(
        read(json!({ "TestModeResults": "NotStarted" })).simulation,
        SimulationState::NotRun
    );
    assert_eq!(
        read(json!({ "SimulationStatus": "Completed", "SimulationCompletedTime": "2026-10-18T12:00:00Z" })).simulation,
        SimulationState::Complete { at: Some(Utc.with_ymd_and_hms(2026, 10, 18, 12, 0, 0).unwrap()) }
    );
    assert_eq!(
        read(json!({ "Whatever": "Completed" })).simulation,
        SimulationState::Unknown
    );
    assert_eq!(
        read(json!({ "DistributionStatus": "Pending" })).distribution,
        DistributionState::Pending
    );
    assert_eq!(read(json!({})).distribution, DistributionState::Unknown);
    assert_eq!(read(json!({ "Enabled": 1 })).enabled, None);
}

#[test]
fn dates_and_header_patterns_in_their_forms() {
    assert_eq!(
        date(&json!("/Date(0)/")),
        Utc.timestamp_millis_opt(0).single()
    );
    assert_eq!(
        date(&json!("/Date(1760778764000-0700)/")),
        Utc.timestamp_millis_opt(1_760_778_764_000).single()
    );
    assert_eq!(date(&json!("yesterday")), None);
    assert_eq!(date(&json!(12)), None);
    assert_eq!(
        header_patterns(&json!("x-cumulas-label=^y$")),
        [("x-cumulas-label".to_owned(), "^y$".to_owned())]
    );
    assert!(header_patterns(&json!(null)).is_empty());
}
