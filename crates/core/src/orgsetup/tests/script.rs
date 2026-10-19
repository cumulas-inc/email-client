//! The PowerShell script and the portal steps.

use chrono::{TimeZone, Utc};
use serde_json::json;

use super::tenant::FakeTenant;
use super::{CONFIDENTIAL, TENANT, ctx, snapshot_of};
use crate::model::OrgSetupChoices;
use crate::orgsetup::invoke::Cmdlet;
use crate::orgsetup::objects::LabelView;
use crate::orgsetup::plan;
use crate::orgsetup::script::{ScriptInput, ScriptLabel, comment, ps, render_call, script};

const ODD: &str = "6c1b0f3e-1111-4a8b-9c2d-0000000000bb";

fn view(guid: &str, name: &str) -> LabelView {
    LabelView::read(&super::tenant::label(guid, name)).unwrap()
}

fn input() -> ScriptInput {
    ScriptInput {
        version: "0.5.0".into(),
        generated: Utc.with_ymd_and_hms(2026, 10, 19, 14, 3, 0).unwrap(),
        organization: "contoso.com".into(),
        tenant_id: TENANT.into(),
        actions: true,
        encrypt_template: "Encrypt".into(),
        dnf_template: "Do Not Forward".into(),
        labels: vec![
            ScriptLabel {
                id: CONFIDENTIAL.into(),
                title: "Confidential".into(),
                view: view(CONFIDENTIAL, "Confidential"),
            },
            ScriptLabel {
                id: ODD.into(),
                title: "O'Brien’s $(whoami) `n; x".into(),
                view: view(ODD, "O'Brien’s"),
            },
        ],
    }
}

#[test]
fn the_script_matches_its_reviewed_text() {
    let script = script(&input()).script;
    let path = format!(
        "{}/src/orgsetup/tests/fixtures/script.ps1",
        env!("CARGO_MANIFEST_DIR")
    );
    if std::env::var_os("ORGSETUP_UPDATE_SCRIPT").is_some() {
        std::fs::write(&path, &script).unwrap();
    }
    let golden = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        script, golden,
        "the script changed: review it, then run with ORGSETUP_UPDATE_SCRIPT=1"
    );
}

#[test]
fn values_are_quoted_and_comments_flattened() {
    assert_eq!(ps("O'Brien’s"), "'O''Brien’’s'");
    assert_eq!(ps("‘a‚b‛"), "'‘‘a‚‚b‛‛'");
    assert_eq!(ps("$(x); `y"), "'$(x); `y'");
    assert_eq!(comment("a\r\nb # c"), "a  b   c");
    let mut weird = input();
    weird.organization = "evil.example\nRemove-TransportRule *".into();
    weird.encrypt_template = "Enc'rypt; Remove-TransportRule *".into();
    let script = script(&weird).script;
    // The organization stays in its comment; the template in one string.
    assert!(script.starts_with("# Cumulas Mail: encryption and sensitivity labels for evil.example Remove-TransportRule * (tenant"));
    assert!(script.contains("Get-TemplateName 'Enc''rypt; Remove-TransportRule *'"));
    assert!(script.contains(
        "@{ Id = '6c1b0f3e-1111-4a8b-9c2d-0000000000bb'; Title = 'O''Brien’’s $(whoami) `n; x' }"
    ));
    assert!(
        !script
            .lines()
            .any(|l| l.trim_start().starts_with("Remove-"))
    );
}

#[test]
fn only_chosen_parts_are_there() {
    let full = script(&input()).script;
    assert!(full.contains("Get-RMSTemplate") && full.contains("Connect-IPPSSession"));
    let mut actions_only = input();
    actions_only.labels.clear();
    let actions_only = script(&actions_only).script;
    assert!(actions_only.contains("'Cumulas Mail: Encrypt'"));
    assert!(!actions_only.contains("Connect-IPPSSession"));
    assert!(!actions_only.contains("-RemoveHeader 'x-cumulas-label'"));
    let mut labels_only = input();
    labels_only.actions = false;
    let labels_only = script(&labels_only).script;
    assert!(!labels_only.contains("Get-RMSTemplate"));
    assert!(labels_only.contains("-RemoveHeader 'x-cumulas-label'"));
    assert!(labels_only.contains("Connect-IPPSSession"));
}

/// The argument after `flag` on a line, if any.
fn argument<'l>(line: &'l str, flag: &str) -> Option<&'l str> {
    let at = line.find(flag)? + flag.len();
    line[at..]
        .split_whitespace()
        .next()
        .map(|a| a.trim_end_matches([';', ')']))
}

#[test]
fn every_write_names_only_our_own_objects() {
    let script = script(&input()).script;
    let allowed = [
        "$Strip",
        "$item.Name",
        "$rule.Identity",
        "$own.Identity",
        "$policy.Identity",
        "$name",
        "$ruleName",
    ];
    let mut writes = 0;
    for line in script.lines() {
        let line = line.trim_start();
        let write = ["New-", "Set-", "Enable-"].iter().any(|verb| {
            line.contains(&format!("{verb}TransportRule"))
                || line.contains(&format!("{verb}AutoSensitivityLabel"))
        });
        if !write || line.starts_with('#') {
            continue;
        }
        for flag in [" -Identity ", " -Name ", " -Policy "] {
            if let Some(target) = argument(line, flag) {
                writes += 1;
                assert!(
                    allowed.contains(&target) || target.starts_with("'Cumulas Mail: "),
                    "{line}"
                );
            }
        }
        // Splatted parameters: only `@fix` and `@new` built just above.
        assert!(!line.contains("@install") || line.starts_with("Install-Module"));
    }
    assert!(writes >= 10, "{writes}");
    assert!(!script.contains("Remove-"), "no Remove- at all");
    assert!(!script.contains("Disable-"));
}

#[test]
fn part_two_checks_part_one_and_the_module_comes_from_the_gallery() {
    let script = script(&input()).script;
    let check = script
        .find("throw \"Label requests from outside aren't blocked yet")
        .unwrap();
    let connect = script.find("Connect-IPPSSession").unwrap();
    assert!(check < connect);
    assert!(script.contains("Repository = 'PSGallery'"));
    assert!(script.contains("$MinModule = [version]'3.2.0'"));
    assert!(script.contains("[version]'3.9.2'"));
    assert!(!script.contains("-Force"));
    assert!(script.contains("Import-Module $module.Path"));
    assert!(script.contains("-AutoEnableAfter (New-TimeSpan -Hours 1)"));
    assert!(script.contains("$Group = ''"));
}

#[test]
fn each_planned_call_renders_as_powershell() {
    let tenant = FakeTenant::fresh();
    let choices = OrgSetupChoices {
        actions: true,
        encrypt_template: Some("Encrypt".into()),
        do_not_forward_template: Some("Do Not Forward".into()),
        label_ids: vec![CONFIDENTIAL.into()],
        labels_without_license: false,
    };
    let planned = plan::assess(&snapshot_of(&tenant), &ctx(), &choices).planned;
    let rendered: Vec<String> = planned
        .iter()
        .flat_map(|p| p.calls.iter().map(render_call))
        .collect();
    assert_eq!(
        rendered[0],
        format!(
            "New-TransportRule -Name 'Cumulas Mail: ignore label requests from outside' -Comments {} -ExceptIfFromScope 'InOrganization' -Mode 'Enforce' -Priority 0 -RemoveHeader 'x-cumulas-label'",
            ps(crate::orgsetup::wanted::MARKER)
        )
    );
    assert!(
        rendered[1].contains("-HeaderMatchesPatterns @('^encrypt$')"),
        "{}",
        rendered[1]
    );
    assert_eq!(
        rendered[4],
        format!(
            "New-AutoSensitivityLabelRule -Name 'Cumulas Mail: Confidential (header)' -HeaderMatchesPatterns @{{ 'x-cumulas-label' = '(?i)^({CONFIDENTIAL})$' }} -Policy 'Cumulas Mail: Confidential' -Workload 'Exchange'"
        )
    );
    assert_eq!(
        rendered[5],
        "Set-AutoSensitivityLabelPolicy -Identity 'Cumulas Mail: Confidential' -AutoEnableAfter '01:00:00' -StartSimulation $true"
    );
    let cleared = Cmdlet::write("Set-TransportRule", "x")
        .with("Identity", "g")
        .with("FromScope", json!(null))
        .with("HeaderContainsWords", json!([]));
    assert_eq!(
        render_call(&cleared),
        "Set-TransportRule -Identity 'g' -FromScope $null -HeaderContainsWords @()"
    );
    // The confirmation sheet's PowerShell is the same.
    assert_eq!(planned[5].change.powershell, rendered[5]);
}

#[test]
fn portal_steps_use_the_names_and_pattern_setup_uses() {
    let steps = script(&input()).portal_steps;
    assert_eq!(steps[0].label_title, "Confidential");
    assert_eq!(steps[0].policy_name, "Cumulas Mail: Confidential");
    assert_eq!(steps[0].rule_name, "Cumulas Mail: Confidential (header)");
    assert_eq!(steps[0].pattern, format!("(?i)^({CONFIDENTIAL})$"));
    // A name too long for 64 characters: the label's GUID instead.
    let mut long = input();
    long.labels[0].view = view(CONFIDENTIAL, &"A very long label name ".repeat(3));
    let steps = script(&long).portal_steps;
    assert_eq!(
        steps[0].policy_name,
        format!("Cumulas Mail: label {CONFIDENTIAL}")
    );
    assert_eq!(
        steps[0].rule_name,
        format!("Cumulas Mail: label {CONFIDENTIAL} (header)")
    );
    // A name that fits, but not with " (header)".
    long.labels[0].view = view(CONFIDENTIAL, &"x".repeat(48));
    let steps = script(&long).portal_steps;
    assert_eq!(
        steps[0].policy_name,
        format!("Cumulas Mail: {}", "x".repeat(48))
    );
    assert_eq!(
        steps[0].rule_name,
        format!("Cumulas Mail: label {CONFIDENTIAL} (header)")
    );
}
