//! Making the changes an administrator confirmed: read again, refuse a
//! stale plan, write only ours (each write's target checked against the
//! fresh read), fall back where Microsoft 365 turns a form down, and read
//! back what it now shows. Never deletes anything.

use std::collections::HashSet;

use chrono::Utc;
use serde_json::{Map, Value};

use super::Session;
use super::invoke::{Cmdlet, InvokeClient, InvokeError};
use super::objects::{AutoPolicyView, AutoRuleView, TransportRuleView};
use super::plan::{
    self, Assessment, LabelOutcome, ObjectKind, Placement, Planned, RuleOutcome, STRIP_FIRST,
    owned_target, row_text, strip_is_ok,
};
use super::status::{self, Part, SessionInfo, SetupContext, Snapshot, read_snapshot};
use super::wanted::{AUTO_ENABLE_AFTER, OwnRule, WantedRule, is_owned};
use crate::error::Result;
use crate::model::{
    OrgSetupAction, OrgSetupChange, OrgSetupChangeResult, OrgSetupChoices, OrgSetupObject,
    OrgSetupOutcome, OrgSetupResult,
};

/// Why the remaining changes were skipped.
pub const WINDOW_CLOSED: &str = "The setup window was closed.";
pub const SIGNED_OUT: &str = "Your sign-in ended. Sign in again to continue.";
const EARLIER_FAILED: &str = "The change before it didn't go through.";
const EXCHANGE_FORBIDDEN: &str = "Your account can't change mail flow rules. Sign in as an \
                                  Exchange Administrator or Global Administrator. If you just got \
                                  the role through Privileged Identity Management, Exchange can \
                                  take a while to apply it.";
const COMPLIANCE_FORBIDDEN: &str = "Your account can't change auto-labeling policies. Sign in as \
                                    a Compliance Administrator or Global Administrator (or a \
                                    member of the Information Protection Admins role group).";
const IRM_OFF: &str =
    "Exchange can't encrypt yet: Message Encryption is off. See “Message Encryption” above.";
const BUSY: &str = "Microsoft 365 is busy. Try again in a few minutes.";
const BACK_IN_SIMULATION: &str = "Microsoft 365 put it back into simulation; it turns itself on \
                                  again an hour after the simulation completes.";

/// A licensing refusal from Purview.
fn license_required(message: &str) -> String {
    format!(
        "Microsoft 365 says a license is missing: both the administrator who signs in and the \
         people who send need a plan that includes automatic labeling. Microsoft said: {}.",
        message.trim_end_matches('.')
    )
}

/// The admin's words for a failed write (3.8.1).
fn failure_message(error: &InvokeError, resource_exchange: bool) -> String {
    match error {
        InvokeError::Forbidden(_) if resource_exchange => EXCHANGE_FORBIDDEN.into(),
        InvokeError::Forbidden(_) => COMPLIANCE_FORBIDDEN.into(),
        InvokeError::LicenseRequired(d) => license_required(&d.message),
        InvokeError::Throttled(_) => BUSY.into(),
        InvokeError::Rejected(d) if resource_exchange && names_irm_licensing(&d.message) => {
            IRM_OFF.into()
        }
        InvokeError::Network(m) => {
            format!("Couldn't reach Microsoft 365: {}.", m.trim_end_matches('.'))
        }
        InvokeError::Redirect(host) => format!(
            "Microsoft 365 sent Cumulas Mail to an address it doesn't trust ({host}), so it \
             stopped."
        ),
        InvokeError::Cancelled => WINDOW_CLOSED.into(),
        InvokeError::Unauthorized(_) => SIGNED_OUT.into(),
        other => format!(
            "Microsoft 365 didn't accept this: {}.",
            other.message().trim_end_matches('.')
        ),
    }
}

fn names_irm_licensing(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("licens")
        || lower.contains("irmconfiguration")
        || lower.contains("rights management")
}

/// What applying did, and what the service records.
pub struct ApplyOutcome {
    pub result: OrgSetupResult,
    /// A mail flow rule was changed (`orgsetup.changed`'s rulesChangedAt).
    pub rules_changed: bool,
    /// An auto-labeling policy or rule was changed.
    pub policies_changed: bool,
}

/// Runs the confirmed changes (3.8).
pub async fn apply(
    session: &Session,
    ctx: &SetupContext,
    info: &SessionInfo,
    choices: &OrgSetupChoices,
    confirmed: &[String],
) -> Result<ApplyOutcome> {
    // 1. A fresh read, and the plan it gives.
    let fresh = read_snapshot(session, ctx).await?;
    let choices = super::wanted::validate(choices, Some(&fresh), ctx)?;
    let assessment = plan::assess(&fresh, ctx, &choices);
    let planned_ids: HashSet<&str> = assessment
        .planned
        .iter()
        .map(|p| p.change.id.as_str())
        .collect();
    let confirmed_ids: HashSet<&str> = confirmed.iter().map(String::as_str).collect();
    if planned_ids != confirmed_ids {
        session
            .log()
            .note("The plan changed since it was shown; nothing was changed.");
        let mut status = status::status(&fresh, ctx, info);
        status.plan = assessment.plan();
        session.keep_snapshot(fresh);
        return Ok(ApplyOutcome {
            result: OrgSetupResult {
                stale: true,
                signed_out: false,
                changes: Vec::new(),
                status,
            },
            rules_changed: false,
            policies_changed: false,
        });
    }

    // 2. The changes, in order.
    let mut run = Run {
        session,
        fresh: &fresh,
        assessment: &assessment,
        results: Vec::new(),
        exchange_stop: None,
        compliance_stop: None,
        label_stop: None,
        signed_out: false,
        failed_groups: HashSet::new(),
        created: Vec::new(),
    };
    for planned in &assessment.planned {
        let result = run.change(planned).await;
        run.results.push(result);
    }
    let Run {
        mut results,
        signed_out,
        ..
    } = run;

    // 3. What Microsoft 365 shows now.
    let mut shown = fresh.clone();
    if !signed_out && !session.is_cancelled() {
        match read_snapshot(session, ctx).await {
            Ok(back) => {
                shown = back;
                let mut back = shown.clone();
                settle_compliance(session, &mut back, &results).await;
                verify(&back, ctx, &choices, &assessment.planned, &mut results);
                shown = back;
                let labels_done = results
                    .iter()
                    .any(|r| r.change.object != OrgSetupObject::MailFlowRule && is_done(r.outcome));
                let after = plan::assess(&shown, ctx, &choices);
                let strip = after.rule(OwnRule::Strip);
                if labels_done && shown.rules.read().is_some() && !strip_is_ok(strip) {
                    results.insert(0, strip_warning(strip));
                }
            }
            Err(e) => session
                .log()
                .note(format!("Couldn't read back the changes: {}", e.message)),
        }
    }
    let changed = |object: fn(OrgSetupObject) -> bool| {
        results
            .iter()
            .any(|r| object(r.change.object) && is_done(r.outcome))
    };
    let rules_changed = changed(|o| o == OrgSetupObject::MailFlowRule);
    let policies_changed = changed(|o| o != OrgSetupObject::MailFlowRule);
    let status = status::status(&shown, ctx, info);
    session.keep_snapshot(shown);
    Ok(ApplyOutcome {
        result: OrgSetupResult {
            stale: false,
            signed_out,
            changes: results,
            status,
        },
        rules_changed,
        policies_changed,
    })
}

fn is_done(outcome: OrgSetupOutcome) -> bool {
    matches!(
        outcome,
        OrgSetupOutcome::Done | OrgSetupOutcome::NotYetListed
    )
}

fn result(
    change: &OrgSetupChange,
    outcome: OrgSetupOutcome,
    message: Option<String>,
) -> OrgSetupChangeResult {
    OrgSetupChangeResult {
        change: change.clone(),
        outcome,
        message,
    }
}

/// The strip rule reads back wrong while label changes were made: said
/// first.
fn strip_warning(strip: &plan::RuleState) -> OrgSetupChangeResult {
    let difference = match &strip.outcome {
        RuleOutcome::Missing => "it isn't listed".to_owned(),
        RuleOutcome::Assessed { changes, placement } => changes
            .lines
            .first()
            .cloned()
            .or(match placement {
                Placement::Move { row, .. } => Some(row.clone()),
                _ => None,
            })
            .or(changes.enable.then(|| "it's turned off".to_owned()))
            .map(|l| row_text(&l))
            .unwrap_or_default(),
        _ => "Cumulas Mail can't check it".to_owned(),
    };
    OrgSetupChangeResult {
        change: OrgSetupChange {
            id: format!("check:mailFlowRule:{}", strip.name.to_lowercase()),
            action: OrgSetupAction::Change,
            object: OrgSetupObject::MailFlowRule,
            name: strip.name.to_owned(),
            details: Vec::new(),
            powershell: String::new(),
        },
        outcome: OrgSetupOutcome::Failed,
        message: Some(format!(
            "Label requests from outside aren't blocked ({difference}), so auto-labeling \
             policies could act on mail from outside. Repair “{}” or run Part 1 of the \
             PowerShell script.",
            strip.name
        )),
    }
}

/// One run's progress through its changes.
struct Run<'r> {
    session: &'r Session,
    fresh: &'r Snapshot,
    assessment: &'r Assessment,
    results: Vec<OrgSetupChangeResult>,
    /// Exchange refused for lack of a role: why the rest of it, and every
    /// label change, is skipped.
    exchange_stop: Option<String>,
    compliance_stop: Option<String>,
    /// Purview refused for licensing: why the other label changes are
    /// skipped.
    label_stop: Option<String>,
    signed_out: bool,
    /// Labels one of whose changes didn't go through.
    failed_groups: HashSet<String>,
    /// What this run created, which later calls may address (a new
    /// policy's simulation): ours by name, made by a `New-*` checked here.
    created: Vec<(ObjectKind, String)>,
}

/// Why a write didn't go through.
enum WriteFailure {
    Invoke(InvokeError),
    /// Microsoft 365 took neither `null` nor an empty value to clear a
    /// setting: the admin's words.
    Clearing(String),
}

/// Why a change ended without being made.
enum Ended {
    Failed(String),
    Skipped(String),
}

impl Run<'_> {
    /// Why the change can't run now, if it can't.
    fn skip_reason(&self, planned: &Planned) -> Option<String> {
        if self.signed_out {
            return Some(SIGNED_OUT.into());
        }
        if self.session.is_cancelled() {
            return Some(WINDOW_CLOSED.into());
        }
        if !planned.needs_strip_rule {
            return self.exchange_stop.clone();
        }
        // A label change (D11).
        if self.exchange_stop.is_some() {
            return Some(STRIP_FIRST.into());
        }
        if let Some(stop) = self.compliance_stop.clone().or(self.label_stop.clone()) {
            return Some(stop);
        }
        if !self.strip_ok() {
            return Some(STRIP_FIRST.into());
        }
        if planned
            .group
            .as_ref()
            .is_some_and(|g| self.failed_groups.contains(g))
        {
            return Some(EARLIER_FAILED.into());
        }
        None
    }

    /// Whether label requests from outside are removed: the strip rule's
    /// own changes all went through in this run, or (with none planned) the
    /// fresh read shows it right.
    fn strip_ok(&self) -> bool {
        let strip = self.assessment.rule(OwnRule::Strip);
        let ours: Vec<&OrgSetupChangeResult> = self
            .results
            .iter()
            .filter(|r| {
                r.change.object == OrgSetupObject::MailFlowRule
                    && r.change.name.eq_ignore_ascii_case(strip.name)
            })
            .collect();
        let planned = self.assessment.planned.iter().any(|p| {
            p.change.object == OrgSetupObject::MailFlowRule
                && p.change.name.eq_ignore_ascii_case(strip.name)
        });
        if planned {
            !ours.is_empty() && ours.iter().all(|r| is_done(r.outcome))
        } else {
            strip_is_ok(strip)
        }
    }

    async fn change(&mut self, planned: &Planned) -> OrgSetupChangeResult {
        if let Some(why) = self.skip_reason(planned) {
            return result(&planned.change, OrgSetupOutcome::Skipped, Some(why));
        }
        match self.calls(planned).await {
            Ok(message) => result(&planned.change, OrgSetupOutcome::Done, message),
            Err(ended) => {
                if let Some(group) = &planned.group {
                    self.failed_groups.insert(group.clone());
                }
                match ended {
                    Ended::Failed(why) => {
                        result(&planned.change, OrgSetupOutcome::Failed, Some(why))
                    }
                    Ended::Skipped(why) => {
                        result(&planned.change, OrgSetupOutcome::Skipped, Some(why))
                    }
                }
            }
        }
    }

    fn client_for(&self, planned: &Planned) -> bool {
        planned.change.object == OrgSetupObject::MailFlowRule
    }

    /// Runs a change's calls; `Ok` with a note when it went through.
    async fn calls(&mut self, planned: &Planned) -> std::result::Result<Option<String>, Ended> {
        let exchange = self.client_for(planned);
        let client = if exchange {
            self.session.exchange()
        } else {
            match self.session.compliance().await {
                Ok(client) => client,
                Err(e) => return Err(self.stop(&e, false, planned)),
            }
        };
        let kind = ObjectKind::from(planned.change.object);
        let mut note = None;
        for call in &planned.calls {
            if let Err(why) = self.owned(call, kind) {
                return Err(Ended::Failed(why));
            }
            let error = match self.write(client, call).await {
                Ok(()) => {
                    self.remember(call, kind);
                    continue;
                }
                Err(WriteFailure::Clearing(why)) => return Err(Ended::Failed(why)),
                Err(WriteFailure::Invoke(error)) => error,
            };
            match error {
                InvokeError::AlreadyExists(_) if call.name.starts_with("New-") => {
                    self.adopt(client, call, kind).await?;
                    note = Some("It was already there; Cumulas Mail repaired it.".to_owned());
                }
                e if self.might_have_happened(&e) => {
                    if !self.happened(client, planned, call).await {
                        return Err(Ended::Failed(failure_message(&e, exchange)));
                    }
                    self.session.log().note(format!(
                        "{} went through after all ({}).",
                        call.step,
                        e.message()
                    ));
                }
                e => return Err(self.stop(&e, exchange, planned)),
            }
            // There now, and ours: later calls (a new policy's simulation)
            // may address it.
            self.remember(call, kind);
        }
        if let Some(policy) = &planned.restart_if_off {
            note = self.restart_if_off(client, policy).await.or(note);
        }
        Ok(note)
    }

    /// Notes what a `New-*` call made (its name passed [`Self::owned`]).
    fn remember(&mut self, call: &Cmdlet, kind: ObjectKind) {
        if let Some(name) = call.new_name().filter(|_| call.name.starts_with("New-")) {
            self.created.push((kind, name.to_owned()));
        }
    }

    /// The target must be ours in the fresh read: a new object's name ours
    /// and not listed yet, anything else's Identity one of ours there.
    fn owned(&self, call: &Cmdlet, kind: ObjectKind) -> std::result::Result<(), String> {
        if call.name.starts_with("New-") {
            let name = call.new_name().unwrap_or_default();
            if is_owned(name) && !listed(self.fresh, kind, name) {
                return Ok(());
            }
            let refused = format!(
                "Cumulas Mail refused to create “{name}”: it isn't one of its own names, or it exists."
            );
            self.session.log().note(refused.clone());
            return Err(refused);
        }
        let identity = call.identity().unwrap_or_default();
        if is_owned(identity)
            && self
                .created
                .iter()
                .any(|(k, name)| *k == kind && name == identity)
        {
            return Ok(());
        }
        owned_target(identity, kind, self.fresh)
            .map(|_| ())
            .map_err(|e| {
                self.session.log().note(e.message.clone());
                e.message
            })
    }

    /// Sends a write as this session sends it now (the pinned forms), with
    /// its fallbacks (R8): a refused `null` as `""` or `[]`, a refused
    /// comment left out, a refused hashtable without its type.
    async fn write(
        &self,
        client: &InvokeClient,
        call: &Cmdlet,
    ) -> std::result::Result<(), WriteFailure> {
        let pins = self.session.pins();
        let mut call = prepared(call, pins);
        let mut tried_empty = pins.clear_with_empty;
        let (mut tried_comment, mut tried_hashtable) = (false, false);
        loop {
            let error = match client.invoke(&call).await {
                Ok(_) => return Ok(()),
                Err(e) => e,
            };
            let InvokeError::Rejected(detail) = &error else {
                return Err(WriteFailure::Invoke(error));
            };
            let text = format!("{} {}", detail.message, detail.excerpt).to_ascii_lowercase();
            let comment = ["Comments", "Comment"]
                .into_iter()
                .find(|k| call.params.contains_key(*k));
            if let Some(key) = comment.filter(|_| !tried_comment && text.contains("comment")) {
                tried_comment = true;
                call.params.remove(key);
                self.session.pin(|p| p.without_comments = true);
                self.session
                    .log()
                    .note(format!("{}: tried again without {key} (R8)", call.step));
                continue;
            }
            let typed = call
                .params
                .get("HeaderMatchesPatterns")
                .is_some_and(|v| v.get("@odata.type").is_some());
            if typed
                && !tried_hashtable
                && ["headermatchespatterns", "hashtable", "convert"]
                    .iter()
                    .any(|w| text.contains(w))
            {
                tried_hashtable = true;
                if let Some(Value::Object(object)) = call.params.get_mut("HeaderMatchesPatterns") {
                    object.remove("@odata.type");
                }
                self.session.pin(|p| p.plain_hashtable = true);
                self.session.log().note(format!(
                    "{}: tried again with a plain hashtable (R8)",
                    call.step
                ));
                continue;
            }
            let mentions = |names: &[String]| {
                text.contains("null")
                    || names.iter().any(|n| text.contains(&n.to_ascii_lowercase()))
            };
            let null_names = nulls(&call);
            if !null_names.is_empty() && !tried_empty && mentions(&null_names) {
                tried_empty = true;
                call = empty_cleared(&call);
                self.session.pin(|p| p.clear_with_empty = true);
                self.session.log().note(format!(
                    "{}: tried again clearing with \"\" or [] (R8)",
                    call.step
                ));
                continue;
            }
            let cleared = cleared_names(&call);
            if tried_empty && !cleared.is_empty() && (mentions(&cleared) || text.contains("empty"))
            {
                return Err(WriteFailure::Clearing(format!(
                    "Microsoft 365 didn't accept clearing {} through this interface. Part 1 of \
                     the PowerShell script does it.",
                    cleared.join(", ")
                )));
            }
            return Err(WriteFailure::Invoke(error));
        }
    }

    /// A write that may have happened although it got no answer: listed
    /// again before it counts as failed (never sent again blindly).
    fn might_have_happened(&self, error: &InvokeError) -> bool {
        match error {
            InvokeError::Network(_) | InvokeError::Unexpected(_) => true,
            InvokeError::Rejected(d) => d.status.is_some_and(|s| s >= 500),
            _ => false,
        }
    }

    /// Whether `call` now shows as done.
    async fn happened(&self, client: &InvokeClient, planned: &Planned, call: &Cmdlet) -> bool {
        let Some(listing) = list_again(client, planned.change.object).await else {
            return false;
        };
        let name = call
            .new_name()
            .map(str::to_owned)
            .unwrap_or_else(|| planned.change.name.clone());
        match planned.change.object {
            OrgSetupObject::MailFlowRule => {
                let rules: Vec<TransportRuleView> =
                    listing.iter().filter_map(TransportRuleView::read).collect();
                let Some(rule) = rules.iter().find(|r| r.is_named(&planned.change.name)) else {
                    return false;
                };
                rule_matches_call(rule, call, &rules)
            }
            OrgSetupObject::AutoLabelingPolicy => listing
                .iter()
                .filter_map(AutoPolicyView::read)
                .any(|p| p.is_named(&name)),
            OrgSetupObject::AutoLabelingRule => listing
                .iter()
                .filter_map(AutoRuleView::read)
                .any(|r| r.is_named(&name)),
        }
    }

    /// A `New-*` found its object there already: if it's ours and has
    /// nothing we don't set, repair it instead.
    async fn adopt(
        &self,
        client: &InvokeClient,
        call: &Cmdlet,
        kind: ObjectKind,
    ) -> std::result::Result<(), Ended> {
        let name = call.new_name().unwrap_or_default().to_owned();
        let object = match kind {
            ObjectKind::MailFlowRule => OrgSetupObject::MailFlowRule,
            ObjectKind::AutoLabelingPolicy => OrgSetupObject::AutoLabelingPolicy,
            ObjectKind::AutoLabelingRule => OrgSetupObject::AutoLabelingRule,
        };
        let failed = |why: &str| Ended::Failed(why.to_owned());
        let listing = list_again(client, object)
            .await
            .ok_or_else(|| failed("Microsoft 365 says it exists, but it couldn't be listed."))?;
        let set = match kind {
            ObjectKind::MailFlowRule => {
                let rule = listing
                    .iter()
                    .filter_map(TransportRuleView::read)
                    .find(|r| r.is_named(&name) && is_owned(&r.name))
                    .ok_or_else(|| failed("Microsoft 365 says it exists, but doesn't list it."))?;
                let guid = rule.guid.clone().ok_or_else(|| {
                    failed("Microsoft 365 didn't return its ID, so Cumulas Mail can't change it.")
                })?;
                let wanted = wanted_rule_for(call);
                match plan::diff_rule(&rule, &wanted, None) {
                    RuleOutcome::Blocked(extras) => {
                        let names: Vec<&str> = extras.iter().map(|(p, _)| p.as_str()).collect();
                        return Err(Ended::Failed(format!(
                            "It has settings Cumulas Mail doesn't set: {}. Remove them in the \
                             Exchange admin center (or delete the rule), then select Read again.",
                            names.join(", ")
                        )));
                    }
                    RuleOutcome::Assessed { changes, .. } => {
                        let mut set = Cmdlet::write(
                            "Set-TransportRule",
                            format!("Repair “{name}” (adopted)"),
                        )
                        .with("Identity", guid.as_str());
                        for (key, value) in changes.set {
                            set = set.with(&key, value);
                        }
                        let mut calls = vec![set.with("Comments", super::wanted::MARKER)];
                        if changes.enable {
                            calls.push(
                                Cmdlet::write("Enable-TransportRule", format!("Turn on “{name}”"))
                                    .with("Identity", guid.as_str()),
                            );
                        }
                        if let Some(priority) = call.params.get("Priority") {
                            calls.push(
                                Cmdlet::write("Set-TransportRule", format!("Move “{name}”"))
                                    .with("Identity", guid.as_str())
                                    .with("Priority", priority.clone()),
                            );
                        }
                        calls
                    }
                    _ => return Err(failed("Microsoft 365 says it exists, but doesn't list it.")),
                }
            }
            ObjectKind::AutoLabelingPolicy => {
                let policy = listing
                    .iter()
                    .filter_map(AutoPolicyView::read)
                    .find(|p| p.is_named(&name) && is_owned(&p.name))
                    .ok_or_else(|| failed("Microsoft 365 says it exists, but doesn't list it."))?;
                if !policy.other_locations.is_empty() {
                    return Err(failed(
                        "Its auto-labeling policy has SharePoint or OneDrive locations that \
                         Cumulas Mail doesn't set. Remove them in the Microsoft Purview portal, \
                         then select Read again.",
                    ));
                }
                // Only rules of ours in it (D12).
                let rules = list_again(client, OrgSetupObject::AutoLabelingRule)
                    .await
                    .ok_or_else(|| {
                        failed("Microsoft 365 says it exists, but its rules couldn't be listed.")
                    })?;
                if rules
                    .iter()
                    .filter_map(AutoRuleView::read)
                    .any(|r| r.in_policy(&policy.name) && !is_owned(&r.name))
                {
                    return Err(failed(
                        "Its auto-labeling policy has another rule that Cumulas Mail doesn't set. \
                         Remove it in the Microsoft Purview portal, then select Read again.",
                    ));
                }
                let mut set = Cmdlet::write(
                    "Set-AutoSensitivityLabelPolicy",
                    format!("Repair “{name}” (adopted)"),
                )
                .with("Identity", policy.name.as_str());
                for key in ["ApplySensitivityLabel", "OverwriteLabel", "Comment"] {
                    if let Some(value) = call.params.get(key) {
                        set = set.with(key, value.clone());
                    }
                }
                vec![set.with("AddExchangeLocation", serde_json::json!(["All"]))]
            }
            ObjectKind::AutoLabelingRule => {
                let rule = listing
                    .iter()
                    .filter_map(AutoRuleView::read)
                    .find(|r| r.is_named(&name) && is_owned(&r.name))
                    .ok_or_else(|| failed("Microsoft 365 says it exists, but doesn't list it."))?;
                if !rule.other_conditions.is_empty() {
                    return Err(failed(
                        "Its auto-labeling policy has other conditions that Cumulas Mail doesn't \
                         set. Remove them in the Microsoft Purview portal, then select Read again.",
                    ));
                }
                let mut set = Cmdlet::write(
                    "Set-AutoSensitivityLabelRule",
                    format!("Repair “{name}” (adopted)"),
                )
                .with("Identity", rule.name.as_str());
                if let Some(patterns) = call.params.get("HeaderMatchesPatterns") {
                    set = set.with("HeaderMatchesPatterns", patterns.clone());
                }
                vec![set]
            }
        };
        self.session
            .log()
            .note(format!("“{name}” already existed: adopted"));
        for call in &set {
            match self.write(client, call).await {
                Ok(()) => {}
                Err(WriteFailure::Clearing(why)) => return Err(Ended::Failed(why)),
                Err(WriteFailure::Invoke(e)) => {
                    return Err(Ended::Failed(failure_message(
                        &e,
                        kind == ObjectKind::MailFlowRule,
                    )));
                }
            }
        }
        Ok(())
    }

    /// After an edit of a policy that was on: if Microsoft 365 put it back
    /// into simulation, start it again with its turn-on time.
    async fn restart_if_off(&self, client: &InvokeClient, policy: &str) -> Option<String> {
        let mut read = Cmdlet::read(
            "Get-AutoSensitivityLabelPolicy",
            format!("Read “{policy}” again"),
        )
        .with("Identity", policy);
        read.log_values = true;
        let output = client.invoke(&read).await.ok()?;
        let view = output.values.first().and_then(AutoPolicyView::read)?;
        if view.is_on() || !view.is_named(policy) || !is_owned(&view.name) {
            return None;
        }
        let call = Cmdlet::write(
            "Set-AutoSensitivityLabelPolicy",
            format!("Start the simulation of “{policy}” again"),
        )
        .with("Identity", view.name.as_str())
        .with("StartSimulation", true)
        .with("AutoEnableAfter", AUTO_ENABLE_AFTER);
        self.write(client, &call).await.ok()?;
        Some(BACK_IN_SIMULATION.to_owned())
    }

    /// A refusal that ends more than this change.
    fn stop(&mut self, error: &InvokeError, exchange: bool, planned: &Planned) -> Ended {
        let message = failure_message(error, exchange);
        match error {
            InvokeError::Unauthorized(_) => {
                self.signed_out = true;
                return Ended::Skipped(SIGNED_OUT.into());
            }
            InvokeError::Cancelled => return Ended::Skipped(WINDOW_CLOSED.into()),
            InvokeError::Forbidden(_) if exchange => self.exchange_stop = Some(message.clone()),
            InvokeError::Forbidden(_) => self.compliance_stop = Some(message.clone()),
            InvokeError::LicenseRequired(_) if planned.needs_strip_rule => {
                self.label_stop = Some(message.clone());
            }
            _ => {}
        }
        Ended::Failed(message)
    }
}

/// Whether an object named `name` of `kind` is in `snapshot`.
fn listed(snapshot: &Snapshot, kind: ObjectKind, name: &str) -> bool {
    match kind {
        ObjectKind::MailFlowRule => snapshot
            .rules
            .read()
            .is_some_and(|rules| rules.iter().any(|r| r.is_named(name))),
        ObjectKind::AutoLabelingPolicy => snapshot
            .compliance
            .read()
            .is_some_and(|c| c.policies.iter().any(|p| p.is_named(name))),
        ObjectKind::AutoLabelingRule => snapshot
            .compliance
            .read()
            .is_some_and(|c| c.rules.iter().any(|r| r.is_named(name))),
    }
}

/// The listing of `object`'s kind, again.
async fn list_again(client: &InvokeClient, object: OrgSetupObject) -> Option<Vec<Value>> {
    let cmdlet = match object {
        OrgSetupObject::MailFlowRule => {
            Cmdlet::read("Get-TransportRule", "List mail flow rules again")
                .with("ResultSize", "Unlimited")
        }
        OrgSetupObject::AutoLabelingPolicy => Cmdlet::read(
            "Get-AutoSensitivityLabelPolicy",
            "List auto-labeling policies again",
        ),
        OrgSetupObject::AutoLabelingRule => Cmdlet::read(
            "Get-AutoSensitivityLabelRule",
            "List auto-labeling rules again",
        ),
    };
    client.invoke(&cmdlet).await.ok().map(|o| o.values)
}

/// The wanted rule a `New-TransportRule` call creates.
fn wanted_rule_for(call: &Cmdlet) -> WantedRule {
    let template = call
        .params
        .get("ApplyRightsProtectionTemplate")
        .and_then(Value::as_str)
        .unwrap_or("Encrypt");
    match call.new_name().unwrap_or_default() {
        name if name.eq_ignore_ascii_case(super::wanted::ENCRYPT_RULE) => {
            WantedRule::encrypt(template)
        }
        name if name.eq_ignore_ascii_case(super::wanted::DNF_RULE) => {
            WantedRule::do_not_forward(template)
        }
        _ => WantedRule::strip(),
    }
}

/// Whether a rule shows what `call` set.
fn rule_matches_call(rule: &TransportRuleView, call: &Cmdlet, rules: &[TransportRuleView]) -> bool {
    if call.name == "Enable-TransportRule" {
        return rule
            .state
            .as_deref()
            .is_some_and(|s| s.eq_ignore_ascii_case("Enabled"));
    }
    if let Some(priority) = call.params.get("Priority").and_then(Value::as_i64) {
        let mut sorted: Vec<&TransportRuleView> = rules.iter().collect();
        sorted.sort_by_key(|r| r.priority.unwrap_or(i64::MAX));
        let index = sorted.iter().position(|r| r.is_named(&rule.name));
        if index.and_then(|i| i64::try_from(i).ok()) != Some(priority) {
            return false;
        }
        if call.params.len() <= 2 {
            return true;
        }
    }
    let wanted = wanted_rule_for(&Cmdlet {
        params: {
            let mut p = call.params.clone();
            p.insert("Name".into(), Value::String(rule.name.clone()));
            p
        },
        ..call.clone()
    });
    match plan::diff_rule(rule, &wanted, None) {
        RuleOutcome::Assessed { changes, .. } => {
            call.params.keys().all(|k| !changes.set.contains_key(k))
        }
        _ => false,
    }
}

/// The call as this session sends it now: pinned forms applied.
fn prepared(call: &Cmdlet, pins: super::Pins) -> Cmdlet {
    let mut call = call.clone();
    if pins.without_comments {
        call.params.remove("Comments");
        call.params.remove("Comment");
    }
    if pins.plain_hashtable
        && let Some(Value::Object(object)) = call.params.get_mut("HeaderMatchesPatterns")
    {
        object.remove("@odata.type");
    }
    if pins.clear_with_empty {
        call = empty_cleared(&call);
    }
    call
}

/// Parameters a call clears with `null`.
fn nulls(call: &Cmdlet) -> Vec<String> {
    call.params
        .iter()
        .filter(|(_, v)| v.is_null())
        .map(|(k, _)| k.clone())
        .collect()
}

/// Parameters a call clears with `""` or `[]`.
fn cleared_names(call: &Cmdlet) -> Vec<String> {
    call.params
        .iter()
        .filter(|(k, v)| {
            CLEARABLE.iter().any(|(name, _)| name == k)
                && (v.as_str() == Some("") || v.as_array().is_some_and(Vec::is_empty))
        })
        .map(|(k, _)| k.clone())
        .collect()
}

/// The settings ours may clear, and whether each takes a list.
const CLEARABLE: &[(&str, bool)] = &[
    ("FromScope", false),
    ("HeaderMatchesMessageHeader", false),
    ("HeaderMatchesPatterns", true),
    ("HeaderContainsMessageHeader", false),
    ("HeaderContainsWords", true),
];

/// The call clearing with `""` (one value) or `[]` (a list) instead of
/// `null`.
fn empty_cleared(call: &Cmdlet) -> Cmdlet {
    let mut call = call.clone();
    let params: Map<String, Value> = call
        .params
        .into_iter()
        .map(|(key, value)| {
            if value.is_null() {
                let list = CLEARABLE.iter().any(|(name, list)| *name == key && *list);
                let empty = if list {
                    Value::Array(Vec::new())
                } else {
                    Value::String(String::new())
                };
                (key, empty)
            } else {
                (key, value)
            }
        })
        .collect();
    call.params = params;
    call
}

/// Security & Compliance may not list what was just made: list its
/// policies and rules again, up to three times, while one is missing.
async fn settle_compliance(
    session: &Session,
    back: &mut Snapshot,
    results: &[OrgSetupChangeResult],
) {
    let wanted: Vec<(OrgSetupObject, String)> = results
        .iter()
        .filter(|r| {
            r.change.object != OrgSetupObject::MailFlowRule && r.outcome == OrgSetupOutcome::Done
        })
        .map(|r| (r.change.object, r.change.name.clone()))
        .collect();
    let missing = |snapshot: &Snapshot| {
        wanted
            .iter()
            .any(|(object, name)| !listed(snapshot, ObjectKind::from(*object), name))
    };
    if wanted.is_empty() || back.compliance.read().is_none() {
        return;
    }
    let Ok(client) = session.compliance().await else {
        return;
    };
    for _ in 0..3 {
        if !missing(back) {
            return;
        }
        tokio::time::sleep(session.relist_delay).await;
        let policies = list_again(client, OrgSetupObject::AutoLabelingPolicy).await;
        let rules = list_again(client, OrgSetupObject::AutoLabelingRule).await;
        if let (Part::Read(compliance), Some(policies), Some(rules)) =
            (&mut back.compliance, policies, rules)
        {
            let detailed = std::mem::take(&mut compliance.policies);
            compliance.policies = policies
                .iter()
                .filter_map(AutoPolicyView::read)
                .map(|p| {
                    detailed
                        .iter()
                        .find(|d| d.name == p.name)
                        .cloned()
                        .unwrap_or(p)
                })
                .collect();
            compliance.rules = rules.iter().filter_map(AutoRuleView::read).collect();
        }
    }
}

/// Each change's outcome from what Microsoft 365 now shows: done when its
/// object matches (placement included), not yet listed when Security &
/// Compliance doesn't show it yet, else failed with the first difference.
fn verify(
    back: &Snapshot,
    ctx: &SetupContext,
    choices: &OrgSetupChoices,
    planned: &[Planned],
    results: &mut [OrgSetupChangeResult],
) {
    let after = plan::assess(back, ctx, choices);
    for (planned, result) in planned.iter().zip(results.iter_mut()) {
        if result.outcome != OrgSetupOutcome::Done {
            continue;
        }
        let shown = match planned.change.object {
            OrgSetupObject::MailFlowRule => {
                if back.rules.read().is_none() {
                    continue;
                }
                verify_rule(&after, planned)
            }
            _ => {
                if back.compliance.read().is_none() {
                    continue;
                }
                verify_label(&after, planned)
            }
        };
        match shown {
            Shown::Matches => {}
            Shown::NotListed if planned.change.object != OrgSetupObject::MailFlowRule => {
                result.outcome = OrgSetupOutcome::NotYetListed;
            }
            Shown::NotListed => {
                result.outcome = OrgSetupOutcome::Failed;
                result.message = Some(accepted_but("it isn't listed"));
            }
            Shown::Differs(difference) => {
                result.outcome = OrgSetupOutcome::Failed;
                result.message = Some(accepted_but(&difference));
            }
        }
    }
}

fn accepted_but(difference: &str) -> String {
    format!("Microsoft 365 accepted the change but shows something else: {difference}")
}

enum Shown {
    Matches,
    NotListed,
    Differs(String),
}

fn verify_rule(after: &Assessment, planned: &Planned) -> Shown {
    let Some(state) = after
        .rules
        .iter()
        .find(|r| r.name.eq_ignore_ascii_case(&planned.change.name))
    else {
        return Shown::Matches;
    };
    let (changes, placement) = match &state.outcome {
        RuleOutcome::Assessed { changes, placement } => (changes, placement),
        RuleOutcome::Missing => return Shown::NotListed,
        RuleOutcome::Blocked(extras) => {
            let names: Vec<&str> = extras.iter().map(|(p, _)| p.as_str()).collect();
            return Shown::Differs(format!(
                "settings Cumulas Mail doesn't set: {}",
                names.join(", ")
            ));
        }
        _ => return Shown::Matches,
    };
    let misplaced = match placement {
        Placement::Move { row, .. } => Some(row.clone()),
        Placement::Conflict {
            to: Some(_), lines, ..
        } => lines.first().map(|l| row_text(l)),
        _ => None,
    };
    let keys: Vec<&String> = planned
        .calls
        .iter()
        .flat_map(|c| c.params.keys())
        .filter(|k| !matches!(k.as_str(), "Identity" | "Name" | "Comments" | "Priority"))
        .collect();
    let set_differs = changes
        .set
        .keys()
        .find(|k| keys.contains(k))
        .and_then(|_| changes.lines.first().map(|l| row_text(l)));
    let check_place = matches!(
        planned.change.action,
        OrgSetupAction::Create | OrgSetupAction::Move
    );
    let check_on = matches!(
        planned.change.action,
        OrgSetupAction::Create | OrgSetupAction::TurnOn
    );
    if let Some(d) = set_differs {
        return Shown::Differs(d);
    }
    if check_on && changes.enable {
        return Shown::Differs("it's turned off".into());
    }
    if check_place && let Some(d) = misplaced {
        return Shown::Differs(d);
    }
    Shown::Matches
}

fn verify_label(after: &Assessment, planned: &Planned) -> Shown {
    let name = &planned.change.name;
    let policy = planned.change.object == OrgSetupObject::AutoLabelingPolicy;
    let state = after.labels.iter().find(|l| {
        if policy {
            l.policy_name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        } else {
            l.rule.as_ref().is_some_and(|r| r.is_named(name))
                || l.policy_name.as_deref().is_some_and(|n| {
                    name.to_ascii_lowercase()
                        .starts_with(&n.to_ascii_lowercase())
                })
        }
    });
    let Some(state) = state else {
        return Shown::NotListed;
    };
    match &state.outcome {
        LabelOutcome::Missing => Shown::NotListed,
        LabelOutcome::Blocked(what) => Shown::Differs(format!("its policy has {what}")),
        LabelOutcome::Assessed(changes) => {
            let keys: Vec<&String> = planned.calls.iter().flat_map(|c| c.params.keys()).collect();
            if policy {
                if let Some(line) = changes
                    .policy_set
                    .keys()
                    .find(|k| {
                        keys.contains(k)
                            || (k.as_str() == "AddExchangeLocation"
                                && keys.iter().any(|k| k.as_str() == "ExchangeLocation"))
                    })
                    .and(changes.policy_lines.first())
                {
                    return Shown::Differs(row_text(line));
                }
                if keys.iter().any(|k| k.as_str() == "Enabled") && changes.enable {
                    return Shown::Differs("it's turned off".into());
                }
                Shown::Matches
            } else if changes.create_rule || state.rule.is_none() {
                Shown::NotListed
            } else if let Some(line) = changes
                .rule_set
                .keys()
                .find(|k| keys.contains(k))
                .and(changes.rule_lines.first())
            {
                Shown::Differs(row_text(line))
            } else {
                Shown::Matches
            }
        }
        _ => Shown::Matches,
    }
}

/// When the setup last changed things, merged into what was recorded.
pub fn settled(
    previous: crate::protection::Settle,
    rules: bool,
    policies: bool,
) -> crate::protection::Settle {
    let now = Utc::now();
    crate::protection::Settle {
        rules_changed_at: if rules {
            Some(now)
        } else {
            previous.rules_changed_at
        },
        policies_changed_at: if policies {
            Some(now)
        } else {
            previous.policies_changed_at
        },
    }
}
