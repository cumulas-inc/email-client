//! The status pass: everything setup needs, read from Exchange Online and
//! Security & Compliance (each half on its own), and the rows the dialog
//! shows for it.

use std::fmt::Write as _;

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;

use super::Session;
use super::invoke::{Cmdlet, ErrorDetail, InvokeClient, InvokeError};
use super::objects::{
    AcceptedDomainView, AutoPolicyView, AutoRuleView, DistributionState, InboundConnectorView,
    IrmView, LabelView, OutboundConnectorView, RmsTemplateView, SimulationState, TransportRuleView,
};
use super::plan::{
    self, Assessment, LabelOutcome, LabelState, Placement, RuleOutcome, RuleState, leftovers,
    row_text,
};
use super::runlog::MAX_RAW;
use super::script::ps;
use super::wanted::{
    DNF_TEMPLATE, ENCRYPT_TEMPLATE, LEFTOVER_STRIP_RULE, OwnRule, RELAY_INBOUND, RELAY_RULE,
    STAMPED_LABEL_HEADER, is_owned, matched_by,
};
use crate::auth::admin_setup::SETUP_APP_NAME;
use crate::error::Result;
use crate::model::{
    AccountFeature, LabelCatalog, LicenseHintState, OrgSetupAdmin, OrgSetupChoices, OrgSetupFix,
    OrgSetupLabel, OrgSetupRow, OrgSetupRowState, OrgSetupSection, OrgSetupStatus, RelayPath,
    RmsTemplate,
};
use crate::protection::{Settle, clean_value};
use crate::providers::graph::LicenseHint;

/// One part of a read.
#[derive(Debug, Clone, PartialEq)]
pub enum Part<T> {
    Read(T),
    Failed(ReadFailure),
    /// Not read: an earlier call of its half failed.
    Skipped,
}

impl<T> Part<T> {
    pub fn read(&self) -> Option<&T> {
        match self {
            Part::Read(value) => Some(value),
            _ => None,
        }
    }

    pub fn failure(&self) -> Option<&ReadFailure> {
        match self {
            Part::Failed(failure) => Some(failure),
            _ => None,
        }
    }
}

/// Why a half couldn't be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadFailure {
    pub kind: FailureKind,
    pub detail: ErrorDetail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// The account lacks the admin role.
    NoRole,
    /// Microsoft 365 didn't accept the sign-in (R1).
    NotAccepted,
    Unreachable,
    Other,
}

impl ReadFailure {
    fn from_error(error: &InvokeError) -> Self {
        let kind = match error {
            InvokeError::Forbidden(_) => FailureKind::NoRole,
            InvokeError::Unauthorized(_) => FailureKind::NotAccepted,
            InvokeError::Network(_) | InvokeError::Redirect(_) | InvokeError::Throttled(_) => {
                FailureKind::Unreachable
            }
            _ => FailureKind::Other,
        };
        Self {
            kind,
            detail: error.detail().cloned().unwrap_or_else(|| ErrorDetail {
                message: clean_value(&error.message(), 500).unwrap_or_default(),
                ..ErrorDetail::default()
            }),
        }
    }

    /// The region couldn't be found: the half is unreachable, unless
    /// Microsoft 365 didn't accept the sign-in.
    fn from_discovery(error: &InvokeError) -> Self {
        let mut failure = Self::from_error(error);
        if failure.kind != FailureKind::NotAccepted {
            failure.kind = FailureKind::Unreachable;
        }
        failure
    }
}

/// Security & Compliance's part.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ComplianceSnapshot {
    pub labels: Vec<LabelView>,
    /// Ours with their details (call 10).
    pub policies: Vec<AutoPolicyView>,
    pub rules: Vec<AutoRuleView>,
}

/// Everything one read found.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub read_at: DateTime<Utc>,
    pub accepted_domains: Part<Vec<AcceptedDomainView>>,
    pub irm: Part<IrmView>,
    pub templates: Part<Vec<RmsTemplateView>>,
    pub rules: Part<Vec<TransportRuleView>>,
    pub inbound: Part<Vec<InboundConnectorView>>,
    pub outbound: Part<Vec<OutboundConnectorView>>,
    pub compliance: Part<ComplianceSnapshot>,
    /// Call 12: the work account's own licenses, read through its provider.
    pub licensing: Part<LicenseHint>,
}

impl Snapshot {
    /// What the license hint says.
    pub fn license_state(&self) -> LicenseHintState {
        match &self.licensing {
            Part::Read(hint) if hint.auto_labeling => LicenseHintState::Included,
            Part::Read(hint) if hint.provisioning().is_some() => LicenseHintState::Pending,
            Part::Read(_) => LicenseHintState::NotIncluded,
            _ => LicenseHintState::Unknown,
        }
    }

    /// The organization's initial domain, from call 1.
    pub fn initial_domain(&self) -> Option<&str> {
        self.accepted_domains
            .read()?
            .iter()
            .find(|d| d.initial)
            .map(|d| d.domain.as_str())
    }

    /// Exchange Online's first failure, if its half didn't read.
    pub fn exchange_failure(&self) -> Option<&ReadFailure> {
        self.accepted_domains
            .failure()
            .or(self.irm.failure())
            .or(self.templates.failure())
            .or(self.rules.failure())
            .or(self.inbound.failure())
            .or(self.outbound.failure())
    }
}

/// What the service knows without asking Microsoft.
#[derive(Debug, Clone)]
pub struct SetupContext {
    pub account_email: String,
    pub tenant_id: String,
    pub needs_permission: Vec<AccountFeature>,
    pub refused_features: Vec<AccountFeature>,
    /// The work account's label list; `None` before it was listed.
    pub catalog: Option<LabelCatalog>,
    /// Cumulas Signatures' page for administrators, when company signatures
    /// are on for this build.
    pub signatures_manage_url: Option<String>,
    /// When setup last changed the organization from this computer.
    pub settle: Settle,
    pub now: DateTime<Utc>,
    /// The work account's license hint (call 12), read by the service.
    pub licensing: Part<LicenseHint>,
}

impl SetupContext {
    /// The work account's email domain.
    pub fn organization(&self) -> &str {
        self.account_email
            .rsplit_once('@')
            .map_or(self.account_email.as_str(), |(_, domain)| domain)
    }
}

/// Who is signed in, for the rows about the setup app.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub admin: OrgSetupAdmin,
    pub client_id: String,
    pub admin_object_id: String,
}

/// A routing rule that isn't the Cumulas Signatures relay's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignRouting {
    pub name: String,
    pub priority: i64,
    pub stops: bool,
    pub connector: String,
    /// Its recipient conditions.
    pub conditions: Vec<(String, String)>,
}

/// Whether mail goes through the Cumulas Signatures relay, and whether it
/// counts as internal when it comes back.
pub fn relay_path(snapshot: &Snapshot) -> RelayPath {
    let Some(rules) = snapshot.rules.read() else {
        return RelayPath::None;
    };
    if !rules.iter().any(|r| r.routes() && r.is_named(RELAY_RULE)) {
        return RelayPath::None;
    }
    let inbound = snapshot.inbound.read().and_then(|connectors| {
        connectors
            .iter()
            .find(|c| c.name.trim().eq_ignore_ascii_case(RELAY_INBOUND))
    });
    match inbound.and_then(|c| c.treat_as_internal) {
        Some(true) => RelayPath::CumulasInternal,
        Some(false) => RelayPath::CumulasNotInternal,
        None => RelayPath::CumulasUnknown,
    }
}

/// Every routing rule but the relay's.
pub fn foreign_routing(snapshot: &Snapshot) -> Vec<ForeignRouting> {
    let Some(rules) = snapshot.rules.read() else {
        return Vec::new();
    };
    rules
        .iter()
        .filter(|r| r.routes() && !r.is_named(RELAY_RULE))
        .map(|r| ForeignRouting {
            name: r.name.clone(),
            priority: r.priority.unwrap_or_default(),
            stops: r.stops(),
            connector: r.route_connector.clone().unwrap_or_default(),
            conditions: r.recipient_conditions(),
        })
        .collect()
}

/// One read's call, unless its half already failed.
async fn read_part<T>(
    client: &InvokeClient,
    cmdlet: Cmdlet,
    failed: &mut bool,
    parse: impl FnOnce(&[Value]) -> T,
) -> Result<Part<T>> {
    if *failed {
        return Ok(Part::Skipped);
    }
    match client.invoke(&cmdlet).await {
        Ok(output) => Ok(Part::Read(parse(&output.values))),
        Err(InvokeError::Cancelled) => Err(super::signed_out()),
        Err(e) => {
            *failed = true;
            Ok(Part::Failed(ReadFailure::from_error(&e)))
        }
    }
}

/// Reads everything setup needs, one call at a time; a failure fails only
/// its half (the rest of it is skipped). Errors only when the session ends.
pub async fn read_snapshot(session: &Session, ctx: &SetupContext) -> Result<Snapshot> {
    let exchange = session.exchange();
    let mut failed = false;
    let accepted_domains = read_part(
        exchange,
        Cmdlet::read("Get-AcceptedDomain", "Read accepted domains"),
        &mut failed,
        |values| {
            values
                .iter()
                .filter_map(AcceptedDomainView::read)
                .collect::<Vec<_>>()
        },
    )
    .await?;
    if let Part::Read(domains) = &accepted_domains {
        session.set_initial_domain(domains.iter().find(|d| d.initial).map(|d| d.domain.clone()));
    }
    let irm = read_part(
        exchange,
        Cmdlet::read("Get-IRMConfiguration", "Read the encryption settings"),
        &mut failed,
        |values| values.first().map(IrmView::read).unwrap_or_default(),
    )
    .await?;
    let templates = read_part(
        exchange,
        Cmdlet::read("Get-RMSTemplate", "Read rights protection templates")
            .with("ResultSize", "Unlimited"),
        &mut failed,
        |values| {
            values
                .iter()
                .filter_map(RmsTemplateView::read)
                .collect::<Vec<_>>()
        },
    )
    .await?;
    let rules = read_part(
        exchange,
        Cmdlet::read("Get-TransportRule", "Read mail flow rules").with("ResultSize", "Unlimited"),
        &mut failed,
        |values| {
            values
                .iter()
                .filter_map(TransportRuleView::read)
                .collect::<Vec<_>>()
        },
    )
    .await?;
    let inbound = read_part(
        exchange,
        Cmdlet::read("Get-InboundConnector", "Read inbound connectors"),
        &mut failed,
        |values| {
            values
                .iter()
                .filter_map(InboundConnectorView::read)
                .collect::<Vec<_>>()
        },
    )
    .await?;
    let outbound = read_part(
        exchange,
        Cmdlet::read("Get-OutboundConnector", "Read outbound connectors"),
        &mut failed,
        |values| {
            values
                .iter()
                .filter_map(OutboundConnectorView::read)
                .collect::<Vec<_>>()
        },
    )
    .await?;
    let compliance = read_compliance(session).await?;
    let snapshot = Snapshot {
        read_at: ctx.now,
        accepted_domains,
        irm,
        templates,
        rules,
        inbound,
        outbound,
        compliance,
        licensing: ctx.licensing.clone(),
    };
    session.log().snapshot(snapshot_summary(&snapshot));
    Ok(snapshot)
}

/// Security & Compliance's half: the region, the labels, our policies with
/// their details, and the policies' rules.
async fn read_compliance(session: &Session) -> Result<Part<ComplianceSnapshot>> {
    let client = match session.compliance().await {
        Ok(client) => client,
        Err(InvokeError::Cancelled) => return Err(super::signed_out()),
        Err(e) => return Ok(Part::Failed(ReadFailure::from_discovery(&e))),
    };
    let mut failed = false;
    let labels = read_part(
        client,
        Cmdlet::read("Get-Label", "Read sensitivity labels"),
        &mut failed,
        |values| {
            values
                .iter()
                .filter_map(LabelView::read)
                .collect::<Vec<_>>()
        },
    )
    .await?;
    let policies = read_part(
        client,
        Cmdlet::read(
            "Get-AutoSensitivityLabelPolicy",
            "Read auto-labeling policies",
        ),
        &mut failed,
        |values| {
            values
                .iter()
                .filter_map(AutoPolicyView::read)
                .collect::<Vec<_>>()
        },
    )
    .await?;
    let mut policies = match policies {
        Part::Read(policies) => policies,
        Part::Failed(failure) => return Ok(Part::Failed(failure)),
        Part::Skipped => return Ok(labels_failure(labels)),
    };
    for policy in policies.iter_mut().filter(|p| is_owned(&p.name)) {
        let mut cmdlet = Cmdlet::read(
            "Get-AutoSensitivityLabelPolicy",
            format!("Read “{}” in detail", policy.name),
        )
        .with("Identity", policy.name.as_str())
        .with("IncludeTestModeResults", true)
        .with("DistributionDetail", true);
        cmdlet.log_values = true;
        match client.invoke(&cmdlet).await {
            Ok(output) => {
                if let Some(detailed) = output.values.first().and_then(AutoPolicyView::read) {
                    *policy = detailed;
                }
            }
            Err(InvokeError::Cancelled) => return Err(super::signed_out()),
            // The details' parameters aren't documented for this interface
            // (R9): the listing stands in for them.
            Err(InvokeError::Rejected(_)) => session.log().note(format!(
                "Couldn't read “{}” in detail; its simulation and distribution are unknown.",
                policy.name
            )),
            Err(e) => return Ok(Part::Failed(ReadFailure::from_error(&e))),
        }
    }
    let rules = read_part(
        client,
        Cmdlet::read("Get-AutoSensitivityLabelRule", "Read auto-labeling rules"),
        &mut failed,
        |values| {
            values
                .iter()
                .filter_map(AutoRuleView::read)
                .collect::<Vec<_>>()
        },
    )
    .await?;
    Ok(match (labels, rules) {
        (Part::Read(labels), Part::Read(rules)) => Part::Read(ComplianceSnapshot {
            labels,
            policies,
            rules,
        }),
        (Part::Failed(failure), _) | (_, Part::Failed(failure)) => Part::Failed(failure),
        _ => Part::Skipped,
    })
}

fn labels_failure<T>(labels: Part<Vec<LabelView>>) -> Part<T> {
    match labels {
        Part::Failed(failure) => Part::Failed(failure),
        _ => Part::Skipped,
    }
}

/// What a read found, for the run log: the rule order (others by name and
/// order only), the relay's connector, labels, and ours as Microsoft 365
/// returned them.
pub fn snapshot_summary(snapshot: &Snapshot) -> String {
    let mut out = String::new();
    if let Some(failure) = snapshot.exchange_failure() {
        let _ = writeln!(
            out,
            "Exchange Online: couldn't read ({})",
            failure.detail.message
        );
    }
    if let Some(domain) = snapshot.initial_domain() {
        let _ = writeln!(out, "Initial domain: {domain}");
    }
    if let Some(irm) = snapshot.irm.read() {
        let _ = writeln!(
            out,
            "Message Encryption: AzureRMSLicensingEnabled {} · InternalLicensingEnabled {}",
            shown_bool(irm.azure_rms),
            shown_bool(irm.internal)
        );
    }
    if let Some(templates) = snapshot.templates.read() {
        let names: Vec<&str> = templates.iter().map(|t| t.name.as_str()).collect();
        let _ = writeln!(
            out,
            "Templates: {}",
            if names.is_empty() {
                "none".to_owned()
            } else {
                names.join(", ")
            }
        );
    }
    let mut ours = Vec::new();
    if let Some(rules) = snapshot.rules.read() {
        let mut sorted: Vec<&TransportRuleView> = rules.iter().collect();
        sorted.sort_by_key(|r| r.priority.unwrap_or(i64::MAX));
        let _ = writeln!(out, "Mail flow rules, by priority:");
        for rule in sorted {
            let mut line = format!(
                "  {}  {} · {} · {}",
                rule.priority
                    .map_or_else(|| "?".to_owned(), |p| p.to_string()),
                rule.name,
                rule.state.as_deref().unwrap_or("state unknown"),
                rule.mode.as_deref().unwrap_or("mode unknown")
            );
            if rule.stop_rule_processing == Some(true) {
                line.push_str(" · stops later rules");
            }
            if rule.route_connector.is_some() {
                line.push_str(" · routes mail out");
            }
            let _ = writeln!(out, "{line}");
            if is_owned(&rule.name) {
                ours.push((rule.name.clone(), &rule.raw));
            }
        }
    }
    if let Some(connectors) = snapshot.inbound.read()
        && let Some(relay) = connectors
            .iter()
            .find(|c| c.name.trim().eq_ignore_ascii_case(RELAY_INBOUND))
    {
        let _ = writeln!(
            out,
            "Connector \"{}\": {} · TreatMessagesAsInternal {} · CloudServicesMailEnabled {}",
            relay.name,
            relay.connector_type.as_deref().unwrap_or("type unknown"),
            shown_bool(relay.treat_as_internal),
            shown_bool(relay.cloud_services)
        );
    }
    match &snapshot.compliance {
        Part::Read(compliance) => {
            let labels: Vec<String> = compliance
                .labels
                .iter()
                .map(|l| {
                    format!(
                        "{} (Guid {}, ImmutableId {}, email {})",
                        l.title(),
                        l.guid,
                        l.immutable_id.as_deref().unwrap_or("none"),
                        match l.for_email() {
                            Some(true) => "yes",
                            Some(false) => "no",
                            None => "unknown",
                        }
                    )
                })
                .collect();
            let _ = writeln!(
                out,
                "Labels: {}",
                if labels.is_empty() {
                    "none".to_owned()
                } else {
                    labels.join("; ")
                }
            );
            for policy in compliance.policies.iter().filter(|p| is_owned(&p.name)) {
                ours.push((policy.name.clone(), &policy.raw));
            }
            for rule in compliance.rules.iter().filter(|r| is_owned(&r.name)) {
                ours.push((rule.name.clone(), &rule.raw));
            }
        }
        Part::Failed(failure) => {
            let _ = writeln!(
                out,
                "Security & Compliance: couldn't read ({})",
                failure.detail.message
            );
        }
        Part::Skipped => {}
    }
    if !ours.is_empty() {
        let _ = writeln!(
            out,
            "Cumulas Mail's objects as Microsoft 365 returned them:"
        );
        for (name, raw) in ours {
            let _ = writeln!(out, "  {name}");
            let json = serde_json::to_string(raw).unwrap_or_default();
            let _ = writeln!(out, "  {}", clean_value(&json, MAX_RAW).unwrap_or_default());
        }
    }
    out
}

fn shown_bool(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "True",
        Some(false) => "False",
        None => "unknown",
    }
}

/// The status of everything setup reads, with the default choices and
/// their plan.
pub fn status(snapshot: &Snapshot, ctx: &SetupContext, info: &SessionInfo) -> OrgSetupStatus {
    let defaults = defaults(snapshot, ctx);
    let assessment = plan::assess(snapshot, ctx, &defaults);
    let rows = rows(snapshot, ctx, info, &assessment);
    let labels = assessment
        .labels
        .iter()
        .map(|label| OrgSetupLabel {
            id: label.id.clone(),
            title: label.title.clone(),
            in_catalog: label.in_catalog,
            encrypts: label.encrypts,
            for_email: label.view.as_ref().and_then(LabelView::for_email),
            state: label_row(label, ctx).state,
        })
        .collect();
    let exchange_role = snapshot
        .exchange_failure()
        .is_some_and(|f| f.kind == FailureKind::NoRole);
    let compliance_role = snapshot
        .compliance
        .failure()
        .is_some_and(|f| f.kind == FailureKind::NoRole);
    let any_ours = snapshot
        .rules
        .read()
        .is_some_and(|r| r.iter().any(|r| is_owned(&r.name)))
        || snapshot.compliance.read().is_some_and(|c| {
            c.policies.iter().any(|p| is_owned(&p.name))
                || c.rules.iter().any(|r| is_owned(&r.name))
        });
    OrgSetupStatus {
        read_at: snapshot.read_at,
        admin: info.admin.clone(),
        rows,
        labels,
        templates: snapshot
            .templates
            .read()
            .map(|ts| {
                ts.iter()
                    .map(|t| RmsTemplate {
                        name: t.name.clone(),
                        id: t.id.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        relay: assessment.relay,
        license: snapshot.license_state(),
        exchange_readable: snapshot.rules.read().is_some(),
        labels_readable: snapshot.compliance.read().is_some(),
        no_role: exchange_role && compliance_role,
        any_ours,
        plan: assessment.plan(),
        defaults,
    }
}

/// Whether Message Encryption is off, as read.
fn irm_off(snapshot: &Snapshot) -> bool {
    snapshot
        .irm
        .read()
        .is_some_and(|irm| irm.azure_rms == Some(false) || irm.internal == Some(false))
}

/// The choices the dialog starts with: Encrypt and Do Not Forward unless
/// Message Encryption is off or mail flow rules couldn't be read; the
/// templates named as usual, else the ones ours use; only labels that
/// already have a policy of ours (so Repair covers them).
pub fn defaults(snapshot: &Snapshot, ctx: &SetupContext) -> OrgSetupChoices {
    let templates = snapshot.templates.read();
    let rules = snapshot.rules.read();
    let template = |usual: &str, ours: &str| {
        templates
            .and_then(|ts| ts.iter().find(|t| t.name.trim() == usual))
            .map(|t| t.name.trim().to_owned())
            .or_else(|| {
                rules
                    .and_then(|rs| rs.iter().find(|r| r.is_named(ours)))
                    .and_then(|r| r.template.clone())
            })
    };
    let labels = plan::assess(snapshot, ctx, &OrgSetupChoices::default()).labels;
    OrgSetupChoices {
        actions: rules.is_some() && !irm_off(snapshot),
        encrypt_template: template(ENCRYPT_TEMPLATE, super::wanted::ENCRYPT_RULE),
        do_not_forward_template: template(DNF_TEMPLATE, super::wanted::DNF_RULE),
        label_ids: labels
            .iter()
            .filter(|l| l.policy.is_some())
            .map(|l| l.id.clone())
            .collect(),
        labels_without_license: false,
    }
}

fn row(
    id: impl Into<String>,
    section: OrgSetupSection,
    title: impl Into<String>,
    state: OrgSetupRowState,
    summary: impl Into<String>,
) -> OrgSetupRow {
    OrgSetupRow {
        id: id.into(),
        section,
        title: title.into(),
        state,
        summary: summary.into(),
        detail: None,
        object_name: None,
        fix: None,
        link: None,
    }
}

fn with_fix(mut row: OrgSetupRow, fix: OrgSetupFix) -> OrgSetupRow {
    row.fix = Some(fix);
    row
}

fn with_detail(mut row: OrgSetupRow, detail: impl Into<String>) -> OrgSetupRow {
    row.detail = Some(detail.into());
    row
}

/// How to remove this administrator's approval of the setup app.
pub fn remove_approval(client_id: &str, admin_object_id: &str) -> String {
    format!(
        "Microsoft keeps your approval of {SETUP_APP_NAME} until you remove it. If this was a \
         one-off setup, remove it:\n\n\
         Connect-MgGraph -Scopes 'DelegatedPermissionGrant.ReadWrite.All','Application.Read.All'\n\
         $sp = Get-MgServicePrincipal -Filter \"appId eq '{client_id}'\"\n\
         Get-MgOauth2PermissionGrant -Filter \"clientId eq '$($sp.Id)' and principalId eq \
         '{admin_object_id}'\" |\n  \
         ForEach-Object {{ Remove-MgOauth2PermissionGrant -OAuth2PermissionGrantId $_.Id }}\n\n\
         Or delete it for everyone: Entra admin center → Enterprise apps → {SETUP_APP_NAME} → \
         Properties → Delete. To keep it for later repairs: set Assignment required to Yes, \
         assign only your IT admins under Users and groups, and add a Conditional Access policy \
         for it that requires phishing-resistant MFA or a compliant device."
    )
}

/// Every row (3.6.4).
fn rows(
    snapshot: &Snapshot,
    ctx: &SetupContext,
    info: &SessionInfo,
    assessment: &Assessment,
) -> Vec<OrgSetupRow> {
    use OrgSetupRowState as S;
    use OrgSetupSection as Section;
    let email = ctx.account_email.as_str();
    let mut rows = Vec::new();

    // --- Cumulas Mail's permissions ------------------------------------------
    let labels = AccountFeature::SensitivityLabels;
    let approval = if ctx.needs_permission.contains(&labels) {
        row(
            "approval",
            Section::Approval,
            "Cumulas Mail's permissions",
            S::Missing,
            format!("Reading sensitivity labels isn't approved for {email}."),
        )
    } else if ctx.refused_features.contains(&labels) {
        row(
            "approval",
            Section::Approval,
            "Cumulas Mail's permissions",
            S::Blocked,
            format!(
                "Microsoft 365 refused the label list for {email}, often because of licensing."
            ),
        )
    } else {
        row(
            "approval",
            Section::Approval,
            "Cumulas Mail's permissions",
            S::Ok,
            format!(
                "Approved for {email}. Cumulas Mail can't see whether it's approved for everyone."
            ),
        )
    };
    rows.push(with_fix(approval, OrgSetupFix::Approval));
    let username = &info.admin.username;
    rows.push(with_detail(
        row(
            "setupApp",
            Section::Approval,
            SETUP_APP_NAME,
            S::Info,
            format!(
                "Signed in as {username}. Cumulas Mail forgets this sign-in when you close this \
             window. Microsoft keeps your approval of {SETUP_APP_NAME} until you remove it."
            ),
        ),
        remove_approval(&info.client_id, &info.admin_object_id),
    ));

    // --- Encryption ----------------------------------------------------------
    if let Some(failure) = snapshot.exchange_failure() {
        rows.push(access_row(failure, true));
    }
    if let Some(irm) = snapshot.irm.read() {
        let r = match (irm.azure_rms, irm.internal) {
            (Some(true), Some(true)) => row(
                "irm",
                Section::Encryption,
                "Message Encryption",
                S::Ok,
                "Microsoft Purview Message Encryption is on.",
            ),
            (Some(false), _) | (_, Some(false)) => with_fix(
                with_detail(
                    row(
                        "irm",
                        Section::Encryption,
                        "Message Encryption",
                        S::Blocked,
                        "Message Encryption is off in Exchange Online, so nothing can be encrypted.",
                    ),
                    format!(
                        "Cumulas Mail doesn't change this organization-wide setting. In Exchange \
                     Online PowerShell: Set-IRMConfiguration -AzureRMSLicensingEnabled $true \
                     -InternalLicensingEnabled $true, then Test-IRMConfiguration -Sender {} \
                     -Recipient {} must end with OVERALL RESULT: PASS.",
                        ps(email),
                        ps(email)
                    ),
                ),
                OrgSetupFix::Manual,
            ),
            _ => row(
                "irm",
                Section::Encryption,
                "Message Encryption",
                S::Unknown,
                "Couldn't read the encryption settings (Exchange didn't say whether it's on).",
            ),
        };
        rows.push(r);
    }
    if let Some(templates) = snapshot.templates.read() {
        let has = |name: &str| templates.iter().any(|t| t.name.trim() == name);
        let missing: Vec<&str> = [ENCRYPT_TEMPLATE, DNF_TEMPLATE]
            .into_iter()
            .filter(|n| !has(n))
            .collect();
        let r = if templates.is_empty() {
            row(
                "templates",
                Section::Encryption,
                "Encryption templates",
                S::Blocked,
                "Exchange lists no rights protection templates.",
            )
        } else if missing.is_empty() {
            row(
                "templates",
                Section::Encryption,
                "Encryption templates",
                S::Ok,
                "Templates: Encrypt and Do Not Forward.",
            )
        } else {
            row(
                "templates",
                Section::Encryption,
                "Encryption templates",
                S::Differs,
                format!("Choose the template for {}.", missing.join(" and ")),
            )
        };
        rows.push(r);
    }
    for (which, id, title) in [
        (OwnRule::Encrypt, "encryptRule", "Encrypt rule"),
        (OwnRule::DoNotForward, "dnfRule", "Do Not Forward rule"),
    ] {
        if let Some(r) = rule_row(assessment.rule(which), id, title, Section::Encryption, ctx) {
            rows.push(r);
        }
    }

    // --- Sensitivity labels --------------------------------------------------
    if let Some(failure) = snapshot.compliance.failure() {
        rows.push(access_row(failure, false));
    }
    if let Some(mut r) = rule_row(
        assessment.rule(OwnRule::Strip),
        "stripRule",
        "Label requests from outside",
        Section::Labels,
        ctx,
    ) {
        if r.state == S::Ok {
            r.detail = Some(
                "Removes x-cumulas-label from mail that isn't authenticated mail from inside \
                 your organization, so nobody outside can ask for a label."
                    .into(),
            );
        }
        rows.push(r);
    }
    rows.push(licensing_row(snapshot, email));
    if snapshot.compliance.read().is_some() {
        rows.extend(assessment.labels.iter().map(|l| label_row(l, ctx)));
    }

    // --- Mail routing ----------------------------------------------------------
    if snapshot.rules.read().is_some() {
        let r = match assessment.relay {
            RelayPath::None => row(
                "routing",
                Section::Routing,
                "Mail routing",
                S::Ok,
                "Mail doesn't go through the Cumulas Signatures relay.",
            ),
            RelayPath::CumulasInternal => row(
                "routing",
                Section::Routing,
                "Mail routing",
                S::Ok,
                "Mail goes through the Cumulas Signatures relay and counts as internal when it \
                 comes back, so mail is signed before it's encrypted.",
            ),
            RelayPath::CumulasNotInternal => {
                let mut r = row(
                    "routing",
                    Section::Routing,
                    "Mail routing",
                    S::Differs,
                    "Mail goes through the Cumulas Signatures relay, and Microsoft 365 treats it \
                     as outside mail when it comes back. Encrypt and Do Not Forward act before \
                     the relay's rule, so encrypted mail goes out without a signature. \
                     Sensitivity labels can't work until mail coming back counts as internal.",
                );
                r.fix = Some(OrgSetupFix::Signatures);
                r.link = ctx.signatures_manage_url.clone();
                r
            }
            RelayPath::CumulasUnknown => row(
                "routing",
                Section::Routing,
                "Mail routing",
                S::Unknown,
                "Mail goes through the Cumulas Signatures relay, but its connector couldn't be \
                 read. Encrypt and Do Not Forward act before the relay's rule to be safe.",
            ),
        };
        rows.push(r);
        for routing in foreign_routing(snapshot) {
            let conditions = recipients(&routing.conditions);
            let to = conditions
                .map(|c| format!(", for mail to {c}"))
                .unwrap_or_default();
            let mut r = row(
                format!("otherRouting:{}", routing.name),
                Section::Routing,
                routing.name.clone(),
                S::Unknown,
                format!(
                    "Mail this rule matches goes out through “{}”{to}. Cumulas Mail's \
                     encryption rules come before it, so that service gets such mail already \
                     encrypted and can't add a signature to it. People's checks send mail to \
                     themselves, so they don't show whether encryption and labels work for mail \
                     this rule routes: send a test to an address it covers and look at what \
                     arrives.",
                    routing.connector
                ),
            );
            r.object_name = Some(routing.name);
            rows.push(r);
        }
    }
    for leftover in leftovers(snapshot) {
        let stamped = leftover.is_named(LEFTOVER_STRIP_RULE);
        let users: Vec<&str> = if stamped {
            snapshot
                .compliance
                .read()
                .map(|c| {
                    c.rules
                        .iter()
                        .filter(|r| {
                            is_owned(&r.name)
                                && r.header_patterns
                                    .iter()
                                    .any(|(k, _)| k.eq_ignore_ascii_case(STAMPED_LABEL_HEADER))
                        })
                        .map(|r| r.policy.as_deref().unwrap_or(r.name.as_str()))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let summary = if users.is_empty() {
            "Left from an earlier setup and not used any more. You can delete it.".to_owned()
        } else {
            format!(
                "Still needed: auto-labeling policies from an earlier setup ({}) use \
                 x-cumulas-label-stamped. Select those labels and Repair them first.",
                users.join(", ")
            )
        };
        let section = if stamped {
            Section::Labels
        } else {
            Section::Routing
        };
        let mut r = row(
            format!("leftover:{}", leftover.name),
            section,
            leftover.name.clone(),
            S::Info,
            summary,
        );
        r.object_name = Some(leftover.name.clone());
        rows.push(r);
    }
    rows
}

/// A routing rule's recipient conditions, as words.
fn recipients(conditions: &[(String, String)]) -> Option<String> {
    let words: Vec<String> = conditions
        .iter()
        .map(|(parameter, value)| {
            let (except, parameter) = match parameter.strip_prefix("ExceptIf") {
                Some(rest) => (true, rest),
                None => (false, parameter.as_str()),
            };
            let value = match (parameter, value.as_str()) {
                ("SentToScope", "NotInOrganization") => {
                    "people outside your organization".to_owned()
                }
                ("SentToScope", "InOrganization") => "people inside your organization".to_owned(),
                _ => value.clone(),
            };
            if except {
                format!("anyone but {value}")
            } else {
                value
            }
        })
        .collect();
    (!words.is_empty()).then(|| words.join("; "))
}

/// A half's row when it couldn't be read (3.6.1).
fn access_row(failure: &ReadFailure, exchange: bool) -> OrgSetupRow {
    use OrgSetupRowState as S;
    let (id, title, section, what, place) = if exchange {
        (
            "exchangeAccess",
            "Exchange Online",
            OrgSetupSection::Encryption,
            "mail flow rules",
            "Exchange Online",
        )
    } else {
        (
            "labelsAccess",
            "Security & Compliance",
            OrgSetupSection::Labels,
            "auto-labeling policies",
            "Security & Compliance",
        )
    };
    let message = failure.detail.message.trim_end_matches('.');
    let (state, summary, fix) = match failure.kind {
        FailureKind::NoRole => (
            S::Blocked,
            if exchange {
                "Your account can't read mail flow rules. Sign in as an Exchange Administrator or \
                 Global Administrator."
                    .to_owned()
            } else {
                "Your account can't read auto-labeling policies. Sign in as a Compliance \
                 Administrator or Global Administrator."
                    .to_owned()
            },
            OrgSetupFix::SignIn,
        ),
        FailureKind::NotAccepted => (
            S::Blocked,
            format!(
                "Microsoft 365 didn't accept the sign-in for {SETUP_APP_NAME}. Use the PowerShell \
                 script, and send Copy setup details to Cumulas."
            ),
            OrgSetupFix::Manual,
        ),
        FailureKind::Unreachable => (
            S::Unknown,
            format!("Couldn't reach {place} ({message})."),
            OrgSetupFix::Manual,
        ),
        FailureKind::Other => (
            S::Unknown,
            format!("Couldn't read {what}: {message}."),
            OrgSetupFix::Manual,
        ),
    };
    with_fix(row(id, section, title, state, summary), fix)
}

fn minutes_since(at: DateTime<Utc>, now: DateTime<Utc>) -> i64 {
    (now - at).num_minutes().max(1)
}

/// A rule of ours's row; none when mail flow rules weren't read.
fn rule_row(
    state: &RuleState,
    id: &str,
    title: &str,
    section: OrgSetupSection,
    ctx: &SetupContext,
) -> Option<OrgSetupRow> {
    use OrgSetupRowState as S;
    let make = |state, summary: String| {
        let mut r = row(id, section, title, state, summary);
        r.object_name = Some(state_name(id).to_owned());
        r
    };
    let r = match &state.outcome {
        RuleOutcome::Unread => return None,
        RuleOutcome::Missing => {
            with_fix(make(S::Missing, "Not set up.".into()), OrgSetupFix::Setup)
        }
        RuleOutcome::Blocked(extras) => {
            let names: Vec<&str> = extras.iter().map(|(p, _)| p.as_str()).collect();
            let values: Vec<String> = extras.iter().map(|(p, v)| format!("{p}: {v}")).collect();
            with_fix(
                with_detail(
                    make(
                        S::Blocked,
                        format!(
                            "It has settings Cumulas Mail doesn't set: {}. Remove them in the \
                         Exchange admin center (or delete the rule), then select Read again.",
                            names.join(", ")
                        ),
                    ),
                    values.join("\n"),
                ),
                OrgSetupFix::Manual,
            )
        }
        RuleOutcome::NoId => with_fix(
            make(
                S::Blocked,
                "Microsoft 365 didn't return its ID, so Cumulas Mail can't change \
                              it. Use the PowerShell script."
                    .into(),
            ),
            OrgSetupFix::Manual,
        ),
        RuleOutcome::Assessed { changes, placement } => {
            let moved = match placement {
                Placement::Move { row, .. } => Some(row.clone()),
                Placement::Conflict {
                    to: Some(_), lines, ..
                } => lines.first().map(|l| row_text(l)),
                _ => None,
            };
            let test_mode = changes.mode.as_deref().filter(|m| {
                m.eq_ignore_ascii_case("Audit") || m.eq_ignore_ascii_case("AuditAndNotify")
            });
            if let Some(difference) = changes.row.clone().or(moved) {
                with_fix(
                    make(S::Differs, format!("Needs repair: {difference}.")),
                    OrgSetupFix::Setup,
                )
            } else if changes.enable {
                with_fix(make(S::Off, "Turned off.".into()), OrgSetupFix::Setup)
            } else if let Some(mode) = test_mode {
                with_fix(
                    make(
                        S::Off,
                        format!("In test mode ({mode}): it doesn't encrypt."),
                    ),
                    OrgSetupFix::Setup,
                )
            } else if changes.mode.is_some() {
                let line = changes
                    .lines
                    .last()
                    .map(|l| row_text(l))
                    .unwrap_or_default();
                with_fix(
                    make(S::Differs, format!("Needs repair: {line}.")),
                    OrgSetupFix::Setup,
                )
            } else if let Placement::Conflict {
                rule, conditions, ..
            } = placement
            {
                with_fix(
                    make(
                        S::Differs,
                        format!(
                            "“{rule}” comes first and stops later rules for mail it matches \
                         ({conditions}). Mail it matches isn't encrypted."
                        ),
                    ),
                    OrgSetupFix::Manual,
                )
            } else {
                let changed = state
                    .rule
                    .as_ref()
                    .and_then(|r| r.when_changed)
                    .or(ctx.settle.rules_changed_at);
                match changed.filter(|at| ctx.now - *at < Duration::minutes(30)) {
                    Some(at) => make(
                        S::Waiting,
                        format!(
                            "Set up {} minutes ago. New rules can take 30 minutes or more to start \
                         working.",
                            minutes_since(at, ctx.now)
                        ),
                    ),
                    None => make(S::Ok, "Set up.".into()),
                }
            }
        }
    };
    Some(r)
}

fn state_name(id: &str) -> &'static str {
    match id {
        "encryptRule" => super::wanted::ENCRYPT_RULE,
        "dnfRule" => super::wanted::DNF_RULE,
        _ => super::wanted::STRIP_RULE,
    }
}

/// The licensing row (section 5).
fn licensing_row(snapshot: &Snapshot, email: &str) -> OrgSetupRow {
    use OrgSetupRowState as S;
    let title = "Automatic labeling license";
    let section = OrgSetupSection::Labels;
    let unknown_detail = "Sending with a label needs automatic labeling: Microsoft 365 E5, or an \
                          add-on such as Microsoft Purview Suite.";
    let included = |from: &[String]| {
        format!(
            "From: {}. Everyone who sends labeled mail, and the administrator who manages \
             auto-labeling policies, needs a license like it.",
            from.join(", ")
        )
    };
    match &snapshot.licensing {
        Part::Read(hint) if hint.auto_labeling => with_detail(
            row(
                "licensing",
                section,
                title,
                S::Ok,
                format!("{email}'s license includes automatic labeling."),
            ),
            included(&hint.from),
        ),
        Part::Read(hint) if hint.provisioning().is_some() => {
            let status = hint
                .provisioning()
                .map(|(_, state)| state.clone())
                .unwrap_or_default();
            let skus: Vec<String> = hint
                .pending
                .iter()
                .filter(|(_, state)| *state == status)
                .map(|(sku, _)| sku.clone())
                .collect();
            with_detail(
                row(
                    "licensing",
                    section,
                    title,
                    S::Waiting,
                    format!(
                        "{email}'s license includes automatic labeling, but Microsoft 365 hasn't \
                     finished turning it on ({status})."
                    ),
                ),
                included(&skus),
            )
        }
        Part::Read(_) => with_detail(
            row(
                "licensing",
                section,
                title,
                S::Blocked,
                format!(
                    "{email}'s license doesn't include automatic labeling, which sending with a label \
                 needs."
                ),
            ),
            "Microsoft 365 E3 and Business Premium don't include it. It comes with Microsoft 365 \
             E5, or as an add-on such as Microsoft Purview Suite (formerly Microsoft 365 E5 \
             Compliance). Everyone who sends labeled mail, and the administrator who manages \
             auto-labeling policies, needs it. Encrypt and Do Not Forward work with E3 and \
             Business Premium.",
        ),
        Part::Failed(failure) => with_detail(
            row(
                "licensing",
                section,
                title,
                S::Unknown,
                format!(
                    "Couldn't check {email}'s license ({}).",
                    failure.detail.message.trim_end_matches('.')
                ),
            ),
            unknown_detail,
        ),
        Part::Skipped => with_detail(
            row(
                "licensing",
                section,
                title,
                S::Unknown,
                format!("Couldn't check {email}'s license (Microsoft 365 didn't say)."),
            ),
            unknown_detail,
        ),
    }
}

/// When an auto-labeling policy that completed its simulation turns
/// itself on.
fn turns_on_at(at: DateTime<Utc>) -> String {
    (at + Duration::hours(1))
        .format("%b %-d, %H:%M UTC")
        .to_string()
}

/// A label's row (3.6.4).
pub fn label_row(label: &LabelState, ctx: &SetupContext) -> OrgSetupRow {
    use OrgSetupRowState as S;
    let mut r = row(
        format!("label:{}", label.id),
        OrgSetupSection::Labels,
        label.title.clone(),
        S::Unknown,
        "",
    );
    r.object_name = label.policy_name.clone();
    let (state, summary, fix) = match &label.outcome {
        LabelOutcome::Unread => (
            S::Unknown,
            "Couldn't check: Security & Compliance couldn't be read.".to_owned(),
            None,
        ),
        LabelOutcome::NotListed => (
            S::Blocked,
            "Purview doesn't list this label.".to_owned(),
            Some(OrgSetupFix::Manual),
        ),
        LabelOutcome::NotForEmail => (
            S::Blocked,
            "This label isn't for email.".to_owned(),
            Some(OrgSetupFix::Manual),
        ),
        LabelOutcome::Missing => (
            S::Missing,
            "Not set up.".to_owned(),
            Some(OrgSetupFix::Setup),
        ),
        LabelOutcome::Blocked(what) => (
            S::Blocked,
            format!(
                "Its auto-labeling policy has {what} that Cumulas Mail doesn't set. Remove them in \
                 the Microsoft Purview portal, then select Read again."
            ),
            Some(OrgSetupFix::Manual),
        ),
        LabelOutcome::PendingDeletion => (
            S::Blocked,
            "Microsoft 365 is still deleting its auto-labeling policy. Try again later.".to_owned(),
            Some(OrgSetupFix::Manual),
        ),
        LabelOutcome::Assessed(changes) => {
            let policy = label.policy.as_ref();
            if let Some(line) = changes.first_line().filter(|_| changes.differs()) {
                (
                    S::Differs,
                    format!("Needs repair: {}.", row_text(line)),
                    Some(OrgSetupFix::Setup),
                )
            } else if changes.enable {
                (S::Off, "Turned off.".to_owned(), Some(OrgSetupFix::Setup))
            } else if let Some(DistributionState::Error(message)) = policy.map(|p| &p.distribution)
            {
                (
                    S::Blocked,
                    format!(
                        "Microsoft 365 couldn't apply its auto-labeling policy: {}.",
                        message.trim_end_matches('.')
                    ),
                    Some(OrgSetupFix::Manual),
                )
            } else {
                policy_state(label, ctx)
            }
        }
    };
    r.state = state;
    r.summary = summary;
    r.fix = fix;
    if let Some(view) = &label.view {
        let mut detail = format!(
            "Matched by {} (header value {}).",
            matched_by(view, &label.id),
            label.id
        );
        if let Some(policy) = &label.policy
            && !policy.sender_member_of.is_empty()
        {
            detail.push_str(&format!(
                " Applies only to mail from members of {}.",
                policy.sender_member_of.join(", ")
            ));
        }
        r.detail = Some(detail);
    }
    r
}

/// An assessed policy's mode, distribution and age.
fn policy_state(
    label: &LabelState,
    ctx: &SetupContext,
) -> (OrgSetupRowState, String, Option<OrgSetupFix>) {
    use OrgSetupRowState as S;
    let Some(policy) = &label.policy else {
        return (S::Missing, "Not set up.".into(), Some(OrgSetupFix::Setup));
    };
    let mode = policy
        .mode
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if mode.starts_with("testwith") {
        return match (&policy.auto_enable_after, policy.simulation) {
            (Some(_), SimulationState::Complete { at: Some(at) }) => (
                S::Waiting,
                format!(
                    "Simulation complete; it turns on by itself around {}.",
                    turns_on_at(at)
                ),
                None,
            ),
            (Some(_), _) => (
                S::Waiting,
                "In simulation; it turns on by itself an hour after the simulation completes. \
                 Simulations can take several hours."
                    .into(),
                None,
            ),
            (None, _) => (
                S::Off,
                "In simulation: it doesn't label anything.".into(),
                Some(OrgSetupFix::Setup),
            ),
        };
    }
    if !policy.is_on() {
        return (S::Off, "Turned off.".into(), Some(OrgSetupFix::Setup));
    }
    if policy.distribution == DistributionState::Pending {
        return (
            S::Waiting,
            "Microsoft 365 is still applying it.".into(),
            None,
        );
    }
    let changed = [
        policy.when_changed,
        label.rule.as_ref().and_then(|r| r.when_changed),
        ctx.settle.policies_changed_at,
    ]
    .into_iter()
    .flatten()
    .max();
    match changed.filter(|at| ctx.now - *at < Duration::hours(24)) {
        Some(at) => (
            S::Waiting,
            format!(
                "Turned on {} hours ago. Auto-labeling policies can take up to a day to start \
                 working.",
                (ctx.now - at).num_hours().max(1)
            ),
            None,
        ),
        None => (S::Ok, "On.".into(), None),
    }
}
