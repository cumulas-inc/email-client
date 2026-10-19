//! From what a read found and what the administrator chose to the changes
//! that make it right: which of ours are missing or differ, where Encrypt
//! and Do Not Forward go (design 2.5.3), and the exact calls. Pure: no
//! Microsoft call happens here.
//!
//! Only objects whose names start with "Cumulas Mail: " are ever changed,
//! never one with settings we don't set (D12), never a leftover of the
//! earlier recipe, never anything in a half that couldn't be read, and no
//! auto-labeling policy unless label requests from outside are removed first
//! (D11). Nothing is ever deleted.

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::invoke::Cmdlet;
use super::objects::{
    AutoPolicyView, AutoRuleView, LabelView, RmsTemplateView, SimulationState, TransportRuleView,
};
use super::script::render_call;
use super::status::{Part, SetupContext, Snapshot, relay_path};
use super::wanted::{
    AUTO_ENABLE_AFTER, LABEL_HEADER, MARKER, OwnRule, RELAY_RULE, WantedRule, is_leftover,
    is_owned, label_pattern, policy_name, policy_rule_name,
};
use crate::error::{Error, Result};
use crate::model::{
    LicenseHintState, OrgSetupAction, OrgSetupChange, OrgSetupChoices, OrgSetupObject,
    OrgSetupPlan, RelayPath,
};

/// Under the first label change (and above the label list).
pub const LABEL_DISCLOSURE: &str = "Anyone in your organization can apply a label you set up \
                                    here, from Cumulas Mail or any mail app that can set the \
                                    header, whoever the label is published to.";
/// Under a change to an auto-labeling policy that is on.
pub const BACK_TO_SIMULATION: &str =
    "Microsoft 365 may put it back into simulation for a few hours; it turns itself on again.";
/// Labels chosen while label requests from outside can't be removed first
/// (D11).
pub const STRIP_FIRST: &str = "Label requests from outside must be blocked first: sign in as an \
                               Exchange Administrator or Global Administrator, or run Part 1 of \
                               the PowerShell script.";

/// Labels chosen while the license hint says automatic labeling isn't
/// included.
pub fn unlicensed(email: &str) -> String {
    format!(
        "{email}'s license doesn't include automatic labeling, which sending with a label needs. \
         Select “Set up labels anyway” if your organization has it."
    )
}

/// What kind of object a call addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectKind {
    MailFlowRule,
    AutoLabelingPolicy,
    AutoLabelingRule,
}

impl From<OrgSetupObject> for ObjectKind {
    fn from(object: OrgSetupObject) -> Self {
        match object {
            OrgSetupObject::MailFlowRule => ObjectKind::MailFlowRule,
            OrgSetupObject::AutoLabelingPolicy => ObjectKind::AutoLabelingPolicy,
            OrgSetupObject::AutoLabelingRule => ObjectKind::AutoLabelingRule,
        }
    }
}

/// The object of ours that `identity`, the exact value about to be sent,
/// names in this run's fresh read: a mail flow rule's `Guid`, a Purview
/// object's `Name`. Returns its name. Anything else is refused: there is
/// no fallback to another field.
pub fn owned_target<'a>(identity: &str, kind: ObjectKind, fresh: &'a Snapshot) -> Result<&'a str> {
    let found = match kind {
        ObjectKind::MailFlowRule => match &fresh.rules {
            Part::Read(rules) => rules
                .iter()
                .find(|r| r.guid.as_deref() == Some(identity) && is_owned(&r.name))
                .map(|r| r.name.as_str()),
            _ => None,
        },
        ObjectKind::AutoLabelingPolicy => match &fresh.compliance {
            Part::Read(c) => c
                .policies
                .iter()
                .find(|p| p.name == identity && is_owned(&p.name))
                .map(|p| p.name.as_str()),
            _ => None,
        },
        ObjectKind::AutoLabelingRule => match &fresh.compliance {
            Part::Read(c) => c
                .rules
                .iter()
                .find(|r| r.name == identity && is_owned(&r.name))
                .map(|r| r.name.as_str()),
            _ => None,
        },
    };
    found.ok_or_else(|| {
        tracing::error!(target: "orgsetup", ?kind, "refused a change to an object that isn't ours");
        Error::internal(format!(
            "Cumulas Mail refused to change “{identity}”: it isn't one of its own in what \
             Microsoft 365 just listed."
        ))
    })
}

/// One change and the calls that make it.
#[derive(Debug, Clone, PartialEq)]
pub struct Planned {
    /// What the confirmation sheet shows.
    pub change: OrgSetupChange,
    /// What apply runs, in order.
    pub calls: Vec<Cmdlet>,
    /// A label change: label requests from outside must be removed first
    /// (D11).
    pub needs_strip_rule: bool,
    /// The label it's for: once one of a label's changes fails, the rest
    /// of that label's are skipped.
    pub group: Option<String>,
    /// An edit of an auto-labeling policy that is on: when Microsoft 365
    /// put it back into simulation, start it again with its turn-on time.
    pub restart_if_off: Option<String>,
}

/// Where one of ours goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    Stay,
    /// To this index (priority), for these reasons.
    Move {
        to: usize,
        lines: Vec<String>,
        row: String,
    },
    /// A foreign rule that stops later rules sits above the relay rule while
    /// its return is internal: ours stay after the relay rule (moving there
    /// if `to`), and the admin decides about the other.
    Conflict {
        to: Option<usize>,
        lines: Vec<String>,
        rule: String,
        conditions: String,
    },
}

/// What differs on one of our mail flow rules.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuleChanges {
    /// Parameters to set (`null` clears).
    pub set: Map<String, Value>,
    /// A sentence each.
    pub lines: Vec<String>,
    /// The first difference other than the mode, for its row.
    pub row: Option<String>,
    /// Its mode when that isn't Enforce (`Some("")`: unreadable).
    pub mode: Option<String>,
    /// It's turned off.
    pub enable: bool,
}

/// How one of our mail flow rules stands.
#[derive(Debug, Clone, PartialEq)]
pub enum RuleOutcome {
    /// Mail flow rules weren't read.
    Unread,
    Missing,
    /// It has settings we don't set (D12).
    Blocked(Vec<(String, String)>),
    /// Microsoft 365 didn't return its Guid, so it can't be addressed.
    NoId,
    Assessed {
        changes: RuleChanges,
        placement: Placement,
    },
}

/// One of our mail flow rules.
#[derive(Debug, Clone, PartialEq)]
pub struct RuleState {
    pub which: OwnRule,
    pub name: &'static str,
    pub rule: Option<TransportRuleView>,
    pub outcome: RuleOutcome,
}

/// What differs on a label's auto-labeling policy and its rule.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LabelChanges {
    /// Policy parameters to set, other than `Enabled`.
    pub policy_set: Map<String, Value>,
    pub policy_lines: Vec<String>,
    /// The policy is turned off (`Enabled: false`).
    pub enable: bool,
    /// Our rule is missing from the policy.
    pub create_rule: bool,
    pub rule_set: Map<String, Value>,
    pub rule_lines: Vec<String>,
}

impl LabelChanges {
    /// Whether anything but the policy's on/off differs.
    pub fn differs(&self) -> bool {
        !self.policy_set.is_empty() || self.create_rule || !self.rule_set.is_empty()
    }

    pub fn first_line(&self) -> Option<&str> {
        self.policy_lines
            .first()
            .or(self.rule_lines.first())
            .map(String::as_str)
    }
}

/// How a label's auto-labeling policy stands.
#[derive(Debug, Clone, PartialEq)]
pub enum LabelOutcome {
    /// Security & Compliance wasn't read.
    Unread,
    /// Purview doesn't list the label.
    NotListed,
    NotForEmail,
    /// No policy of ours for it.
    Missing,
    /// Another rule, other conditions or other locations (D12).
    Blocked(&'static str),
    PendingDeletion,
    Assessed(LabelChanges),
}

/// A label setup can make an auto-labeling policy for.
#[derive(Debug, Clone, PartialEq)]
pub struct LabelState {
    /// The catalog ID, else Purview's Guid (lowercase).
    pub id: String,
    pub title: String,
    pub in_catalog: bool,
    pub encrypts: bool,
    pub view: Option<LabelView>,
    pub policy_name: Option<String>,
    pub policy: Option<AutoPolicyView>,
    pub rule: Option<AutoRuleView>,
    pub outcome: LabelOutcome,
}

/// Everything the planner worked out.
#[derive(Debug, Clone, PartialEq)]
pub struct Assessment {
    pub relay: RelayPath,
    /// The strip rule, Encrypt, Do Not Forward.
    pub rules: Vec<RuleState>,
    pub labels: Vec<LabelState>,
    pub planned: Vec<Planned>,
    pub problems: Vec<String>,
    pub blocked: u32,
}

impl Assessment {
    pub fn rule(&self, which: OwnRule) -> &RuleState {
        self.rules
            .iter()
            .find(|r| r.which == which)
            .expect("every rule is assessed")
    }

    pub fn plan(&self) -> OrgSetupPlan {
        OrgSetupPlan {
            changes: self.planned.iter().map(|p| p.change.clone()).collect(),
            problems: self.problems.clone(),
            blocked: self.blocked,
        }
    }
}

/// The changes `choices` make on `snapshot`.
pub fn plan(snapshot: &Snapshot, ctx: &SetupContext, choices: &OrgSetupChoices) -> OrgSetupPlan {
    assess(snapshot, ctx, choices).plan()
}

/// A rule's place in the order the planner keeps as it plans.
#[derive(Debug, Clone)]
struct Slot {
    name: String,
    /// Which of ours it is, if it is.
    own: Option<OwnRule>,
    stops: bool,
    routes: bool,
    /// A routing rule for mail from inside the organization only.
    internal_route: bool,
    relay: bool,
    conditions: Vec<(String, String)>,
}

impl Slot {
    fn of(rule: &TransportRuleView) -> Self {
        let own = [
            (OwnRule::Strip, super::wanted::STRIP_RULE),
            (OwnRule::Encrypt, super::wanted::ENCRYPT_RULE),
            (OwnRule::DoNotForward, super::wanted::DNF_RULE),
        ]
        .into_iter()
        .find(|(_, name)| rule.is_named(name))
        .map(|(which, _)| which);
        Self {
            name: rule.name.clone(),
            own,
            stops: rule.stops(),
            routes: rule.routes(),
            internal_route: rule.routes()
                && rule
                    .from_scope
                    .as_deref()
                    .is_some_and(|s| s.eq_ignore_ascii_case("InOrganization")),
            relay: rule.is_named(RELAY_RULE),
            conditions: rule.extras.clone(),
        }
    }

    fn blocks(&self) -> bool {
        self.stops || self.routes
    }

    fn new_own(which: OwnRule, name: &str) -> Self {
        Self {
            name: name.to_owned(),
            own: Some(which),
            stops: false,
            routes: false,
            internal_route: false,
            relay: false,
            conditions: Vec::new(),
        }
    }
}

/// "“A”", "“A” and “B”", "“A”, “B” and “C”".
fn quoted(names: &[&str]) -> String {
    let quoted: Vec<String> = names.iter().map(|n| format!("“{n}”")).collect();
    match quoted.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// "Moves above “X”, which stops later rules for mail it matches, so it
/// acts on that mail first".
fn above_line(passed: &[&Slot]) -> String {
    let names: Vec<&str> = passed.iter().map(|s| s.name.as_str()).collect();
    let stops = passed.iter().all(|s| s.stops);
    let routes = passed.iter().all(|s| s.routes && !s.stops);
    let (one, many) = if stops {
        ("stops later rules", "stop later rules")
    } else if routes {
        (
            "sends mail out through another service",
            "send mail out through another service",
        )
    } else {
        (
            "stops later rules or sends mail out through another service",
            "stop later rules or send mail out through another service",
        )
    };
    if passed.len() == 1 {
        format!(
            "Moves above {}, which {one} for mail it matches, so it acts on that mail first",
            quoted(&names)
        )
    } else {
        format!(
            "Moves above {}, which {many} for mail they match, so it acts on that mail first",
            quoted(&names)
        )
    }
}

const ABOVE_RELAY: &str = "Moves above “Email signatures: add signature”, because mail coming \
                           back from the relay doesn't count as internal: encrypted mail then \
                           goes out without a signature";
const AFTER_RELAY: &str = "Moves after “Email signatures: add signature”, so mail is signed \
                           before it's encrypted (mail coming back from the relay now counts as \
                           internal)";
const AFTER_RELAY_ROW: &str =
    "moves after “Email signatures: add signature”, so mail is signed before it's encrypted";
const TO_THE_TOP: &str = "Moves to the top, so no other rule can stop it first";
const TO_THE_TOP_ROW: &str = "moves to the top";

/// A line as the middle of a row's sentence: its first letter lowercase,
/// unless it starts with a parameter's name.
pub fn row_text(line: &str) -> String {
    let first_word = line.split([' ', ':']).next().unwrap_or_default();
    if first_word.chars().skip(1).any(|c| c.is_ascii_uppercase()) {
        return line.to_owned();
    }
    let mut chars = line.chars();
    match chars.next() {
        Some(c) => c.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Where the strip rule goes: no rule that stops later rules above it, but
/// routing rules for mail from inside the organization (they never see
/// mail from outside).
fn place_strip(slots: &[Slot], own: usize) -> Placement {
    let stopped = slots[..own].iter().any(|s| s.stops && !s.internal_route);
    if stopped && own != 0 {
        Placement::Move {
            to: 0,
            lines: vec![TO_THE_TOP.to_owned()],
            row: TO_THE_TOP_ROW.to_owned(),
        }
    } else {
        Placement::Stay
    }
}

/// Where Encrypt or Do Not Forward goes (D3): above every rule that stops
/// later rules or routes mail out, except the Cumulas Signatures relay's
/// rule when mail coming back from it counts as internal, which ours then
/// come right after (Do Not Forward after Encrypt when that's there). `own`
/// is its index (`slots.len()` for one to create).
fn place_protect(slots: &[Slot], own: usize, relay_internal: bool) -> Placement {
    let relay = slots.iter().position(|s| s.relay && s.blocks());
    let use_relay = relay.is_some() && relay_internal;
    // What ours go right after: the relay's rule, and Encrypt when it
    // already follows it (Do Not Forward goes below Encrypt).
    let anchor = relay.map(|r| {
        let dnf =
            slots.get(own).and_then(|s| s.own) == Some(OwnRule::DoNotForward) || own == slots.len();
        let next = r + 1;
        if dnf && next != own && slots.get(next).and_then(|s| s.own) == Some(OwnRule::Encrypt) {
            next
        } else {
            r
        }
    });
    let above: Vec<usize> = (0..own.min(slots.len()))
        .filter(|&j| slots[j].blocks() && !(use_relay && Some(j) == relay))
        .collect();
    let relay_below = use_relay && relay.is_some_and(|r| r > own && r < slots.len());
    if above.is_empty() && !relay_below {
        return Placement::Stay;
    }
    if let (true, Some(relay), Some(anchor)) = (use_relay, relay, anchor) {
        let to = if anchor > own { anchor } else { anchor + 1 };
        let between: Vec<&Slot> = above
            .iter()
            .filter(|&&j| j > relay)
            .map(|&j| &slots[j])
            .collect();
        let mut lines = Vec::new();
        let mut row = None;
        if relay_below {
            lines.push(AFTER_RELAY.to_owned());
            row = Some(AFTER_RELAY_ROW.to_owned());
        }
        if !between.is_empty() {
            let line = above_line(&between);
            row.get_or_insert_with(|| row_text(&line));
            lines.push(line);
        }
        if let Some(&first) = above.iter().find(|&&j| j < relay) {
            let conflict = &slots[first];
            let conditions = if conflict.conditions.is_empty() {
                "every message".to_owned()
            } else {
                conflict
                    .conditions
                    .iter()
                    .map(|(p, v)| format!("{p}: {v}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            };
            return Placement::Conflict {
                to: (to != own).then_some(to),
                lines,
                rule: conflict.name.clone(),
                conditions,
            };
        }
        return Placement::Move {
            to,
            lines,
            row: row.unwrap_or_default(),
        };
    }
    let to = above[0];
    let mut lines = Vec::new();
    let others: Vec<&Slot> = above
        .iter()
        .map(|&j| &slots[j])
        .filter(|s| !s.relay)
        .collect();
    if above.iter().any(|&j| slots[j].relay) {
        lines.push(ABOVE_RELAY.to_owned());
    }
    if !others.is_empty() {
        lines.push(above_line(&others));
    }
    let row = row_text(&lines[0]);
    Placement::Move { to, lines, row }
}

/// Moves `slots[from]` to `to`, as Exchange does when a rule's priority is
/// set: the others keep their order.
fn shift(slots: &mut Vec<Slot>, from: usize, to: usize) {
    let slot = slots.remove(from);
    slots.insert(to.min(slots.len()), slot);
}

fn same(a: Option<&str>, b: &str) -> bool {
    a.is_some_and(|a| a.trim().eq_ignore_ascii_case(b.trim()))
}

fn was(value: Option<&str>) -> String {
    value.map_or_else(|| "none".to_owned(), str::to_owned)
}

/// What differs on one of our rules (D12: one with extras is blocked, and
/// so is a strip rule that encrypts).
pub fn diff_rule(
    rule: &TransportRuleView,
    wanted: &WantedRule,
    template_id: Option<&str>,
) -> RuleOutcome {
    let mut extras = rule.extras.clone();
    if let (None, Some(template)) = (&wanted.template, &rule.template) {
        extras.push(("ApplyRightsProtectionTemplate".into(), template.clone()));
    }
    if !extras.is_empty() {
        return RuleOutcome::Blocked(extras);
    }
    let mut changes = RuleChanges::default();
    let set = |changes: &mut RuleChanges, key: &str, value: Value, line: String| {
        changes.set.insert(key.to_owned(), value);
        changes.row.get_or_insert_with(|| row_text(&line));
        changes.lines.push(line);
    };
    let protect = wanted.which != OwnRule::Strip;
    if let Some(scope) = &rule.from_scope {
        let line = if protect {
            "Applies to mail from your organization's addresses (it applied only to mail \
             Microsoft 365 had authenticated)"
                .to_owned()
        } else {
            format!("FromScope: none (was {scope})")
        };
        set(&mut changes, "FromScope", Value::Null, line);
    }
    if !same(
        rule.except_if_from_scope.as_deref(),
        wanted.except_if_from_scope,
    ) {
        let line = format!(
            "ExceptIfFromScope: {} (was {})",
            wanted.except_if_from_scope,
            was(rule.except_if_from_scope.as_deref())
        );
        set(
            &mut changes,
            "ExceptIfFromScope",
            json!(wanted.except_if_from_scope),
            line,
        );
    }
    match wanted.header {
        Some((header, pattern)) => {
            if !same(rule.header_matches_header.as_deref(), header) {
                let line = format!(
                    "HeaderMatchesMessageHeader: {header} (was {})",
                    was(rule.header_matches_header.as_deref())
                );
                set(
                    &mut changes,
                    "HeaderMatchesMessageHeader",
                    json!(header),
                    line,
                );
            }
            let patterns: Vec<&str> = rule
                .header_matches_patterns
                .iter()
                .map(|p| p.trim())
                .collect();
            if patterns != [pattern] {
                let old = (!patterns.is_empty()).then(|| patterns.join(", "));
                let line = format!(
                    "HeaderMatchesPatterns: {pattern} (was {})",
                    was(old.as_deref())
                );
                set(
                    &mut changes,
                    "HeaderMatchesPatterns",
                    json!([pattern]),
                    line,
                );
            }
        }
        None => {
            if let Some(header) = &rule.header_matches_header {
                let line = format!("HeaderMatchesMessageHeader: none (was {header})");
                set(
                    &mut changes,
                    "HeaderMatchesMessageHeader",
                    Value::Null,
                    line,
                );
            }
            if !rule.header_matches_patterns.is_empty() {
                let line = format!(
                    "HeaderMatchesPatterns: none (was {})",
                    rule.header_matches_patterns.join(", ")
                );
                set(&mut changes, "HeaderMatchesPatterns", Value::Null, line);
            }
        }
    }
    if rule.header_contains_header.is_some() || !rule.header_contains_words.is_empty() {
        let line = "No longer waits for X-Signature-Stamped (from the earlier Cumulas Signatures \
                    recipe)"
            .to_owned();
        changes
            .set
            .insert("HeaderContainsMessageHeader".into(), Value::Null);
        set(&mut changes, "HeaderContainsWords", Value::Null, line);
    }
    if let Some(template) = &wanted.template {
        let matches = same(rule.template.as_deref(), template)
            || template_id.is_some_and(|id| same(rule.template.as_deref(), id));
        if !matches {
            let line = format!(
                "Template: {template} (was {})",
                was(rule.template.as_deref())
            );
            set(
                &mut changes,
                "ApplyRightsProtectionTemplate",
                json!(template),
                line,
            );
        }
    }
    if !same(rule.remove_header.as_deref(), wanted.remove_header) {
        let line = format!(
            "RemoveHeader: {} (was {})",
            wanted.remove_header,
            was(rule.remove_header.as_deref())
        );
        set(
            &mut changes,
            "RemoveHeader",
            json!(wanted.remove_header),
            line,
        );
    }
    if let Some(action) = wanted.rule_error_action
        && !same(rule.rule_error_action.as_deref(), action)
    {
        let line = "Holds the message and tries again if encryption fails, instead of sending it \
                    unencrypted"
            .to_owned();
        set(&mut changes, "RuleErrorAction", json!(action), line);
    }
    if same(rule.audit_severity.as_deref(), "DoNotAudit") {
        let line = "Records its matches again, so message traces show them".to_owned();
        set(&mut changes, "SetAuditSeverity", json!("Low"), line);
    }
    if rule.stop_rule_processing == Some(true) {
        let line = "No longer stops the rules after it".to_owned();
        set(&mut changes, "StopRuleProcessing", json!(false), line);
    }
    if !same(rule.mode.as_deref(), "Enforce") {
        let line = match &rule.mode {
            Some(mode) => format!("Turns from test mode ({mode}) to Enforce"),
            None => "Mode: Enforce (was unknown)".to_owned(),
        };
        changes.set.insert("Mode".into(), json!("Enforce"));
        changes.lines.push(line);
        changes.mode = Some(rule.mode.clone().unwrap_or_default());
    }
    changes.enable = !same(rule.state.as_deref(), "Enabled");
    RuleOutcome::Assessed {
        changes,
        placement: Placement::Stay,
    }
}

/// The template a rule of ours uses: the choice, else the default.
fn template_of<'t>(
    chosen: Option<&str>,
    templates: Option<&'t [RmsTemplateView]>,
) -> (Option<String>, Option<&'t str>) {
    let Some(name) = chosen.map(str::trim).filter(|t| !t.is_empty()) else {
        return (None, None);
    };
    let id = templates.and_then(|ts| {
        ts.iter()
            .find(|t| t.name.trim().eq_ignore_ascii_case(name))
            .and_then(|t| t.id.as_deref())
    });
    (Some(name.to_owned()), id)
}

fn change_id(action: OrgSetupAction, object: OrgSetupObject, name: &str) -> String {
    let action = match action {
        OrgSetupAction::Create => "create",
        OrgSetupAction::Change => "change",
        OrgSetupAction::TurnOn => "turnOn",
        OrgSetupAction::Move => "move",
        OrgSetupAction::StartSimulation => "startSimulation",
    };
    let object = match object {
        OrgSetupObject::MailFlowRule => "mailFlowRule",
        OrgSetupObject::AutoLabelingPolicy => "autoLabelingPolicy",
        OrgSetupObject::AutoLabelingRule => "autoLabelingRule",
    };
    format!("{action}:{object}:{}", name.trim().to_lowercase())
}

fn planned(
    action: OrgSetupAction,
    object: OrgSetupObject,
    name: &str,
    details: Vec<String>,
    calls: Vec<Cmdlet>,
) -> Planned {
    let powershell = calls.iter().map(render_call).collect::<Vec<_>>().join("\n");
    Planned {
        change: OrgSetupChange {
            id: change_id(action, object, name),
            action,
            object,
            name: name.to_owned(),
            details,
            powershell,
        },
        calls,
        needs_strip_rule: object != OrgSetupObject::MailFlowRule,
        group: None,
        restart_if_off: None,
    }
}

fn step(verb: &str, name: &str) -> String {
    format!("{verb} “{name}”")
}

/// Works out every rule and label, and the changes `choices` make.
pub fn assess(snapshot: &Snapshot, ctx: &SetupContext, choices: &OrgSetupChoices) -> Assessment {
    let relay = relay_path(snapshot);
    let mut problems = Vec::new();
    let mut planned_all = Vec::new();
    let mut blocked = 0;

    // --- labels chosen, and whether they may be planned -------------------
    let label_ids: Vec<String> = choices
        .label_ids
        .iter()
        .map(|id| id.trim().to_ascii_lowercase())
        .collect();
    let mut labels_allowed = !label_ids.is_empty();
    if labels_allowed
        && snapshot.license_state() == LicenseHintState::NotIncluded
        && !choices.labels_without_license
    {
        problems.push(unlicensed(&ctx.account_email));
        labels_allowed = false;
    }

    // --- mail flow rules ----------------------------------------------------
    let rules = match &snapshot.rules {
        Part::Read(rules) => Some(rules.as_slice()),
        _ => None,
    };
    let templates = match &snapshot.templates {
        Part::Read(templates) => Some(templates.as_slice()),
        _ => None,
    };
    let mut ordered: Vec<&TransportRuleView> =
        rules.map(|r| r.iter().collect()).unwrap_or_default();
    ordered.sort_by_key(|r| r.priority.unwrap_or(i64::MAX));
    let mut slots: Vec<Slot> = ordered.iter().map(|r| Slot::of(r)).collect();
    let relay_internal = relay == RelayPath::CumulasInternal;

    let (encrypt_template, encrypt_id) =
        template_of(choices.encrypt_template.as_deref(), templates);
    let (dnf_template, dnf_id) = template_of(choices.do_not_forward_template.as_deref(), templates);
    if choices.actions && rules.is_some() {
        for (template, name) in [
            (&encrypt_template, "Encrypt"),
            (&dnf_template, "Do Not Forward"),
        ] {
            if template.is_none() {
                problems.push(format!("Choose the template for {name}."));
            }
        }
    }
    let wanted = [
        (WantedRule::strip(), None),
        (
            WantedRule::encrypt(encrypt_template.as_deref().unwrap_or("Encrypt")),
            encrypt_id,
        ),
        (
            WantedRule::do_not_forward(dnf_template.as_deref().unwrap_or("Do Not Forward")),
            dnf_id,
        ),
    ];
    let mut rule_states = Vec::new();
    for (wanted, template_id) in wanted {
        let found = ordered.iter().position(|r| r.is_named(wanted.name));
        let rule = found.map(|i| ordered[i].clone());
        let covered = match wanted.which {
            OwnRule::Strip => labels_allowed,
            _ => choices.actions && encrypt_template.is_some() && dnf_template.is_some(),
        };
        let outcome = match (&rule, rules) {
            (_, None) => RuleOutcome::Unread,
            (None, Some(_)) => RuleOutcome::Missing,
            (Some(rule), Some(_)) => diff_rule(rule, &wanted, template_id),
        };
        let mut outcome = match outcome {
            RuleOutcome::Assessed { .. } if rule.as_ref().is_some_and(|r| r.guid.is_none()) => {
                RuleOutcome::NoId
            }
            other => other,
        };
        if matches!(outcome, RuleOutcome::Blocked(_) | RuleOutcome::NoId) && covered {
            blocked += 1;
        }
        // Where it goes, in the order as planned so far.
        let index = rule.as_ref().and_then(|r| {
            slots
                .iter()
                .position(|s| s.name.eq_ignore_ascii_case(&r.name))
        });
        if let RuleOutcome::Assessed { placement, .. } = &mut outcome {
            let own = index.unwrap_or(slots.len());
            *placement = match wanted.which {
                OwnRule::Strip => place_strip(&slots, own),
                _ => place_protect(&slots, own, relay_internal),
            };
        }
        if covered {
            let planned = plan_rule(
                &wanted,
                rule.as_ref(),
                &outcome,
                &mut slots,
                index,
                relay_internal,
            );
            planned_all.extend(planned);
        }
        rule_states.push(RuleState {
            which: wanted.which,
            name: wanted.name,
            rule,
            outcome,
        });
    }

    // --- labels -------------------------------------------------------------
    let strip = rule_states
        .iter()
        .find(|r| r.which == OwnRule::Strip)
        .expect("assessed");
    let strip_planned = planned_all
        .iter()
        .any(|p: &Planned| p.change.name.eq_ignore_ascii_case(strip.name));
    let strip_ok = strip_is_ok(strip);
    if labels_allowed
        && (rules.is_none()
            || matches!(
                strip.outcome,
                RuleOutcome::Blocked(_) | RuleOutcome::NoId | RuleOutcome::Unread
            ))
    {
        problems.push(STRIP_FIRST.to_owned());
        labels_allowed = false;
    }
    let labels_plannable = labels_allowed && (strip_ok || strip_planned);
    let label_states = assess_labels(snapshot, ctx);
    let mut first_label_change = true;
    for id in &label_ids {
        let Some(state) = label_states.iter().find(|l| &l.id == id) else {
            continue;
        };
        match &state.outcome {
            LabelOutcome::Blocked(_) | LabelOutcome::PendingDeletion => {
                blocked += 1;
                continue;
            }
            LabelOutcome::Unread | LabelOutcome::NotListed | LabelOutcome::NotForEmail => continue,
            _ => {}
        }
        if !labels_plannable {
            continue;
        }
        let mut planned = plan_label(state);
        for p in &mut planned {
            p.group = Some(state.id.clone());
        }
        if first_label_change && let Some(first) = planned.first_mut() {
            first.change.details.insert(0, LABEL_DISCLOSURE.to_owned());
            first_label_change = false;
        }
        planned_all.extend(planned);
    }

    Assessment {
        relay,
        rules: rule_states,
        labels: label_states,
        planned: planned_all,
        problems,
        blocked,
    }
}

/// Whether the strip rule works as read: present, on, enforced, nothing of
/// ours to change, in its place.
pub fn strip_is_ok(strip: &RuleState) -> bool {
    match &strip.outcome {
        RuleOutcome::Assessed { changes, placement } => {
            changes.set.is_empty() && !changes.enable && *placement == Placement::Stay
        }
        _ => false,
    }
}

/// The calls for one of our mail flow rules, updating `slots` as they'd
/// move it.
fn plan_rule(
    wanted: &WantedRule,
    rule: Option<&TransportRuleView>,
    outcome: &RuleOutcome,
    slots: &mut Vec<Slot>,
    index: Option<usize>,
    relay_internal: bool,
) -> Vec<Planned> {
    let name = wanted.name;
    let object = OrgSetupObject::MailFlowRule;
    match outcome {
        RuleOutcome::Missing => {
            let (priority, lines) = match wanted.which {
                OwnRule::Strip => (Some(0), vec!["Goes to the top of the list".to_owned()]),
                _ => match place_protect(slots, slots.len(), relay_internal) {
                    Placement::Stay => (None, Vec::new()),
                    Placement::Move { to, lines, .. } => (Some(to), lines),
                    Placement::Conflict { to, lines, .. } => (to, lines),
                },
            };
            let mut call = Cmdlet::write("New-TransportRule", step("Create", name))
                .with("Name", name)
                .with("ExceptIfFromScope", wanted.except_if_from_scope);
            if let Some((header, pattern)) = wanted.header {
                call = call
                    .with("HeaderMatchesMessageHeader", header)
                    .with("HeaderMatchesPatterns", json!([pattern]));
            }
            if let Some(template) = &wanted.template {
                call = call.with("ApplyRightsProtectionTemplate", template.as_str());
            }
            call = call.with("RemoveHeader", wanted.remove_header);
            if let Some(action) = wanted.rule_error_action {
                call = call.with("RuleErrorAction", action);
            }
            if let Some(priority) = priority {
                call = call.with("Priority", priority);
            }
            call = call.with("Mode", "Enforce").with("Comments", MARKER);
            let mut details = vec![match wanted.which {
                OwnRule::Strip => {
                    "Removes x-cumulas-label from mail that isn't authenticated mail \
                                   from inside your organization"
                        .to_owned()
                }
                OwnRule::Encrypt => format!(
                    "Encrypts mail that asks for it with the template “{}”",
                    wanted.template.as_deref().unwrap_or_default()
                ),
                OwnRule::DoNotForward => format!(
                    "Applies the template “{}” to mail that asks for Do Not Forward",
                    wanted.template.as_deref().unwrap_or_default()
                ),
            }];
            if wanted.rule_error_action.is_some() {
                details.push(
                    "Holds the message and tries again if encryption fails, instead of sending \
                     it unencrypted"
                        .to_owned(),
                );
            }
            details.extend(lines.into_iter().map(|l| l.replacen("Moves", "Goes", 1)));
            match priority {
                Some(p) => slots.insert(p.min(slots.len()), Slot::new_own(wanted.which, name)),
                None => slots.push(Slot::new_own(wanted.which, name)),
            }
            vec![planned(
                OrgSetupAction::Create,
                object,
                name,
                details,
                vec![call],
            )]
        }
        RuleOutcome::Assessed { changes, placement } => {
            let (Some(rule), Some(guid)) = (rule, rule.and_then(|r| r.guid.clone())) else {
                return Vec::new();
            };
            let mut out = Vec::new();
            if !changes.set.is_empty() {
                let mut call = Cmdlet::write("Set-TransportRule", step("Change", name))
                    .with("Identity", guid.as_str());
                for (key, value) in &changes.set {
                    call = call.with(key, value.clone());
                }
                call = call.with("Comments", MARKER);
                out.push(planned(
                    OrgSetupAction::Change,
                    object,
                    name,
                    changes.lines.clone(),
                    vec![call],
                ));
            }
            if changes.enable {
                let call = Cmdlet::write("Enable-TransportRule", step("Turn on", name))
                    .with("Identity", guid.as_str());
                out.push(planned(
                    OrgSetupAction::TurnOn,
                    object,
                    name,
                    vec!["Turns it on".to_owned()],
                    vec![call],
                ));
            }
            let target = match placement {
                Placement::Move { to, lines, .. } => Some((*to, lines.clone())),
                Placement::Conflict {
                    to: Some(to),
                    lines,
                    ..
                } => Some((*to, lines.clone())),
                _ => None,
            };
            if let (Some((to, lines)), Some(from)) = (target, index) {
                let call = Cmdlet::write("Set-TransportRule", step("Move", &rule.name))
                    .with("Identity", guid.as_str())
                    .with("Priority", to);
                out.push(planned(
                    OrgSetupAction::Move,
                    object,
                    name,
                    lines,
                    vec![call],
                ));
                shift(slots, from, to);
            }
            out
        }
        _ => Vec::new(),
    }
}

/// Every label a row is shown for: the account's list, then labels Purview
/// lists that aren't in it (not those only grouping others).
fn assess_labels(snapshot: &Snapshot, ctx: &SetupContext) -> Vec<LabelState> {
    let compliance = match &snapshot.compliance {
        Part::Read(compliance) => Some(compliance),
        _ => None,
    };
    let mut states = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    if let Some(catalog) = &ctx.catalog {
        for label in &catalog.labels {
            let id = label.id.to_ascii_lowercase();
            let view = compliance.and_then(|c| c.labels.iter().find(|l| l.is(&id)).cloned());
            if let Some(view) = &view {
                seen.insert(view.guid.clone());
            }
            seen.insert(id.clone());
            states.push(LabelState {
                title: label.title(),
                in_catalog: true,
                encrypts: label.has_protection,
                ..label_state(id, view, compliance)
            });
        }
    }
    if let Some(compliance) = compliance {
        let parents: HashSet<&str> = compliance
            .labels
            .iter()
            .filter_map(|l| l.parent_id.as_deref())
            .collect();
        for view in &compliance.labels {
            if seen.contains(&view.guid) || parents.contains(view.guid.as_str()) {
                continue;
            }
            seen.insert(view.guid.clone());
            states.push(LabelState {
                title: view.title(),
                in_catalog: false,
                encrypts: view.encrypts(),
                ..label_state(view.guid.clone(), Some(view.clone()), Some(compliance))
            });
        }
    }
    states
}

fn label_state(
    id: String,
    view: Option<LabelView>,
    compliance: Option<&super::status::ComplianceSnapshot>,
) -> LabelState {
    let mut state = LabelState {
        id: id.clone(),
        title: String::new(),
        in_catalog: false,
        encrypts: false,
        view: view.clone(),
        policy_name: None,
        policy: None,
        rule: None,
        outcome: LabelOutcome::Unread,
    };
    let Some(compliance) = compliance else {
        return state;
    };
    let Some(view) = view else {
        state.outcome = LabelOutcome::NotListed;
        return state;
    };
    let name = policy_name(&view);
    let rule_name = policy_rule_name(&name, &view);
    state.policy_name = Some(name.clone());
    state.policy = compliance
        .policies
        .iter()
        .find(|p| p.is_named(&name))
        .cloned();
    state.rule = compliance
        .rules
        .iter()
        .find(|r| r.is_named(&rule_name))
        .cloned();
    if view.for_email() == Some(false) {
        state.outcome = LabelOutcome::NotForEmail;
        return state;
    }
    let Some(policy) = &state.policy else {
        state.outcome = LabelOutcome::Missing;
        return state;
    };
    let others = compliance
        .rules
        .iter()
        .any(|r| r.in_policy(&policy.name) && !r.is_named(&rule_name));
    let other_conditions = state.rule.as_ref().is_some_and(|r| {
        !r.other_conditions.is_empty()
            || r.workload
                .as_deref()
                .is_some_and(|w| !w.eq_ignore_ascii_case("Exchange"))
    });
    state.outcome = if others {
        LabelOutcome::Blocked("another rule")
    } else if other_conditions {
        LabelOutcome::Blocked("other conditions")
    } else if !policy.other_locations.is_empty() {
        LabelOutcome::Blocked("SharePoint or OneDrive locations")
    } else if policy.pending_deletion() {
        LabelOutcome::PendingDeletion
    } else {
        LabelOutcome::Assessed(diff_label(&view, &id, policy, state.rule.as_ref()))
    };
    state
}

/// The header pattern an auto-labeling rule of ours sends.
pub fn header_pattern_value(pattern: &str, plain: bool) -> Value {
    if plain {
        json!({ LABEL_HEADER: pattern })
    } else {
        json!({ "@odata.type": "#Exchange.GenericHashTable", LABEL_HEADER: pattern })
    }
}

/// What differs on a label's policy and rule (3.7.4).
fn diff_label(
    view: &LabelView,
    id: &str,
    policy: &AutoPolicyView,
    rule: Option<&AutoRuleView>,
) -> LabelChanges {
    let mut changes = LabelChanges::default();
    let applied = policy.label.as_deref();
    if !(same(applied, &view.guid) || same(applied, &view.name)) {
        changes
            .policy_set
            .insert("ApplySensitivityLabel".into(), json!(view.guid));
        changes.policy_lines.push(format!(
            "Applies the label {} (was {})",
            view.title(),
            was(applied)
        ));
    }
    if !policy
        .exchange_location
        .iter()
        .any(|l| l.trim().eq_ignore_ascii_case("All"))
    {
        changes
            .policy_set
            .insert("AddExchangeLocation".into(), json!(["All"]));
        changes
            .policy_lines
            .push("Covers email from everyone in your organization".to_owned());
    }
    if policy.overwrite != Some(true) {
        changes
            .policy_set
            .insert("OverwriteLabel".into(), json!(true));
        changes.policy_lines.push(
            "Replaces labels that have the same or lower priority, such as one a reply carried \
             over"
                .to_owned(),
        );
    }
    changes.enable = policy.enabled == Some(false);
    let pattern = label_pattern(view, id);
    match rule {
        None => {
            changes.create_rule = true;
            changes
                .rule_lines
                .push(format!("Adds its rule for the header {LABEL_HEADER}"));
        }
        Some(rule) => {
            let right = matches!(rule.header_patterns.as_slice(),
                [(key, value)] if key.eq_ignore_ascii_case(LABEL_HEADER) && value.trim() == pattern);
            if !right {
                changes.rule_set.insert(
                    "HeaderMatchesPatterns".into(),
                    header_pattern_value(&pattern, false),
                );
                let old = rule
                    .header_patterns
                    .iter()
                    .map(|(k, v)| format!("{k}: {v}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                changes.rule_lines.push(format!(
                    "Header pattern: {LABEL_HEADER}: {pattern} (was {})",
                    if old.is_empty() {
                        "none".to_owned()
                    } else {
                        old
                    }
                ));
            }
            if rule.disabled == Some(true) {
                changes.rule_set.insert("Disabled".into(), json!(false));
                changes.rule_lines.push("Turns its rule on".to_owned());
            }
        }
    }
    changes
}

/// The changes for one chosen label (3.7.4).
fn plan_label(state: &LabelState) -> Vec<Planned> {
    let (Some(view), Some(name)) = (&state.view, &state.policy_name) else {
        return Vec::new();
    };
    let rule_name = policy_rule_name(name, view);
    let pattern = label_pattern(view, &state.id);
    let policy_object = OrgSetupObject::AutoLabelingPolicy;
    let rule_object = OrgSetupObject::AutoLabelingRule;
    let new_rule = || {
        Cmdlet::write("New-AutoSensitivityLabelRule", step("Create", &rule_name))
            .with("Name", rule_name.as_str())
            .with("Policy", name.as_str())
            .with("Workload", "Exchange")
            .with(
                "HeaderMatchesPatterns",
                header_pattern_value(&pattern, false),
            )
    };
    let start_simulation = |policy: &str| {
        Cmdlet::write(
            "Set-AutoSensitivityLabelPolicy",
            step("Start the simulation of", policy),
        )
        .with("Identity", policy)
        .with("StartSimulation", true)
        .with("AutoEnableAfter", AUTO_ENABLE_AFTER)
    };
    let simulation_line =
        "Starts its simulation; it turns itself on an hour after the simulation completes";
    match &state.outcome {
        LabelOutcome::Missing => {
            let policy = Cmdlet::write("New-AutoSensitivityLabelPolicy", step("Create", name))
                .with("Name", name.as_str())
                .with("ApplySensitivityLabel", view.guid.as_str())
                .with("ExchangeLocation", json!(["All"]))
                .with("OverwriteLabel", true)
                .with("Mode", "TestWithoutNotifications")
                .with("Comment", MARKER);
            vec![
                planned(
                    OrgSetupAction::Create,
                    policy_object,
                    name,
                    vec![
                        format!("Applies the label {} to mail that asks for it", state.title),
                        "Covers email from everyone in your organization".to_owned(),
                        "Starts in simulation".to_owned(),
                    ],
                    vec![policy],
                ),
                planned(
                    OrgSetupAction::Create,
                    rule_object,
                    &rule_name,
                    vec![format!("Matches the header {LABEL_HEADER} with {pattern}")],
                    vec![new_rule()],
                ),
                planned(
                    OrgSetupAction::StartSimulation,
                    policy_object,
                    name,
                    vec![simulation_line.to_owned()],
                    vec![start_simulation(name)],
                ),
            ]
        }
        LabelOutcome::Assessed(changes) => {
            let Some(policy) = &state.policy else {
                return Vec::new();
            };
            let on = policy.is_on();
            let mut out = Vec::new();
            let mut policy_set = changes.policy_set.clone();
            let mut policy_lines = changes.policy_lines.clone();
            if changes.enable {
                policy_set.insert("Enabled".into(), json!(true));
                policy_lines.push("Turns it on".to_owned());
            }
            if !policy_set.is_empty() {
                let mut call =
                    Cmdlet::write("Set-AutoSensitivityLabelPolicy", step("Change", name))
                        .with("Identity", policy.name.as_str());
                for (key, value) in policy_set {
                    call = call.with(&key, value);
                }
                call = call.with("Comment", MARKER);
                let action = if changes.policy_set.is_empty() {
                    OrgSetupAction::TurnOn
                } else {
                    OrgSetupAction::Change
                };
                if on {
                    policy_lines.push(BACK_TO_SIMULATION.to_owned());
                }
                out.push(planned(
                    action,
                    policy_object,
                    name,
                    policy_lines,
                    vec![call],
                ));
            }
            if changes.create_rule {
                out.push(planned(
                    OrgSetupAction::Create,
                    rule_object,
                    &rule_name,
                    vec![format!("Matches the header {LABEL_HEADER} with {pattern}")],
                    vec![new_rule()],
                ));
            } else if let (false, Some(rule)) = (changes.rule_set.is_empty(), &state.rule) {
                let mut call =
                    Cmdlet::write("Set-AutoSensitivityLabelRule", step("Change", &rule.name))
                        .with("Identity", rule.name.as_str());
                for (key, value) in &changes.rule_set {
                    call = call.with(key, value.clone());
                }
                let mut lines = changes.rule_lines.clone();
                if on {
                    lines.push(BACK_TO_SIMULATION.to_owned());
                }
                out.push(planned(
                    OrgSetupAction::Change,
                    rule_object,
                    &rule.name,
                    lines,
                    vec![call],
                ));
            }
            let edited = changes.differs();
            if on {
                // An edit may put it back into simulation: apply looks again.
                if let Some(last) = out.last_mut() {
                    last.restart_if_off = Some(policy.name.clone());
                }
                return out;
            }
            let simulating = policy
                .mode
                .as_deref()
                .is_some_and(|m| m.to_ascii_lowercase().starts_with("testwith"));
            if !edited && simulating && policy.auto_enable_after.is_some() {
                // It turns itself on after its simulation.
                return out;
            }
            let completed = match policy.simulation {
                SimulationState::Complete { at: Some(at) } => Some(at),
                _ => None,
            };
            let untouched = |changed: Option<chrono::DateTime<chrono::Utc>>, at| {
                changed.is_some_and(|c| c < at)
            };
            let turn_on_now = !edited
                && completed.is_some_and(|at| {
                    untouched(policy.when_changed, at)
                        && state
                            .rule
                            .as_ref()
                            .is_some_and(|r| untouched(r.when_changed, at))
                });
            if turn_on_now {
                let call = Cmdlet::write("Set-AutoSensitivityLabelPolicy", step("Turn on", name))
                    .with("Identity", policy.name.as_str())
                    .with("Mode", "Enable");
                out.push(planned(
                    OrgSetupAction::TurnOn,
                    policy_object,
                    name,
                    vec!["Turns it on: its simulation is complete".to_owned()],
                    vec![call],
                ));
            } else {
                out.push(planned(
                    OrgSetupAction::StartSimulation,
                    policy_object,
                    name,
                    vec![simulation_line.to_owned()],
                    vec![start_simulation(&policy.name)],
                ));
            }
            out
        }
        _ => Vec::new(),
    }
}

/// The calls of a plan, for the property test: every call that addresses
/// something, and what it addresses.
#[cfg(test)]
pub(crate) fn addressed(planned: &[Planned]) -> Vec<(&Cmdlet, ObjectKind)> {
    planned
        .iter()
        .flat_map(|p| {
            p.calls
                .iter()
                .map(move |c| (c, ObjectKind::from(p.change.object)))
        })
        .collect()
}

/// Rules of ours that are leftovers of the earlier recipe.
pub fn leftovers(snapshot: &Snapshot) -> Vec<&TransportRuleView> {
    match &snapshot.rules {
        Part::Read(rules) => rules.iter().filter(|r| is_leftover(&r.name)).collect(),
        _ => Vec::new(),
    }
}
