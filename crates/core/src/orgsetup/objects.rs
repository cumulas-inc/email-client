//! Read-only views of what cmdlets return, read defensively: the shapes
//! aren't documented. A field may come as a string, a number, an object
//! (its `Name`, `DisplayName` or `Value`), an array, `null`, or be missing;
//! booleans as `true` or `"True"`; dates as RFC 3339 or `"/Date(ms)/"`. A
//! field that doesn't read is `None`: for ours that counts as different
//! (we set it again), for others' as unknown.

use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;

use super::wanted::{AUTO_RULE_CONDITIONS, EXTRA_PARAMETERS, POLICY_OTHER_LOCATIONS};

/// A field of an object, its name's case ignored.
pub fn field<'v>(value: &'v Value, name: &str) -> Option<&'v Value> {
    let object = value.as_object()?;
    object.get(name).or_else(|| {
        object
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v)
    })
}

/// A value as one text: a string trimmed, a number, an object's `Name`,
/// `DisplayName` or `Value`, an array's first; `None` when empty.
pub fn text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.trim().to_owned()).filter(|s| !s.is_empty()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(if *b { "True" } else { "False" }.to_owned()),
        Value::Object(_) => ["Name", "DisplayName", "Value"]
            .iter()
            .find_map(|k| field(value, k).and_then(text)),
        Value::Array(items) => items.iter().find_map(text),
        Value::Null => None,
    }
}

/// A value as a list of texts: an array's (nested arrays flattened), or
/// one.
pub fn texts(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items.iter().flat_map(texts).collect(),
        other => text(other).into_iter().collect(),
    }
}

/// A boolean: `true`, `"True"`; `None` for anything else.
pub fn boolean(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(b) => Some(*b),
        Value::String(s) if s.trim().eq_ignore_ascii_case("true") => Some(true),
        Value::String(s) if s.trim().eq_ignore_ascii_case("false") => Some(false),
        _ => None,
    }
}

/// A whole number, from a number or a numeric string.
pub fn integer(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// A date: RFC 3339, or OData's `/Date(ms)/` (with an optional offset).
pub fn date(value: &Value) -> Option<DateTime<Utc>> {
    let text = value.as_str()?.trim();
    if let Some(ms) = text
        .strip_prefix("/Date(")
        .and_then(|rest| rest.strip_suffix(")/"))
    {
        let digits: String = ms
            .chars()
            .enumerate()
            .take_while(|(i, c)| c.is_ascii_digit() || (*i == 0 && *c == '-'))
            .map(|(_, c)| c)
            .collect();
        return Utc.timestamp_millis_opt(digits.parse().ok()?).single();
    }
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

fn get_text(value: &Value, name: &str) -> Option<String> {
    field(value, name).and_then(text)
}

fn get_texts(value: &Value, name: &str) -> Vec<String> {
    field(value, name).map(texts).unwrap_or_default()
}

fn get_bool(value: &Value, name: &str) -> Option<bool> {
    field(value, name).and_then(boolean)
}

/// Whether a value is empty: null, "", [], {}, false.
fn is_empty(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Array(items) => items.iter().all(is_empty),
        Value::Object(object) => object
            .iter()
            .filter(|(k, _)| !k.starts_with('@'))
            .all(|(_, v)| is_empty(v)),
        Value::Bool(true) | Value::Number(_) => false,
    }
}

/// A value as the admin sees it in a row: its texts, comma-separated, at
/// most 100 characters.
fn shown(value: &Value) -> String {
    let shown = match value {
        Value::Object(_) => text(value).unwrap_or_else(|| value.to_string()),
        other => texts(other).join(", "),
    };
    crate::protection::clean_value(&shown, 100).unwrap_or_default()
}

/// The non-empty fields among `names`, with their values.
fn non_empty(value: &Value, names: &[&str]) -> Vec<(String, String)> {
    names
        .iter()
        .filter_map(|name| {
            field(value, name)
                .filter(|v| !is_empty(v))
                .map(|v| ((*name).to_owned(), shown(v)))
        })
        .collect()
}

/// A mail flow rule.
#[derive(Debug, Clone, PartialEq)]
pub struct TransportRuleView {
    pub guid: Option<String>,
    pub name: String,
    pub identity: String,
    pub priority: Option<i64>,
    pub state: Option<String>,
    pub mode: Option<String>,
    pub from_scope: Option<String>,
    pub except_if_from_scope: Option<String>,
    pub header_matches_header: Option<String>,
    pub header_matches_patterns: Vec<String>,
    pub header_contains_header: Option<String>,
    pub header_contains_words: Vec<String>,
    pub template: Option<String>,
    pub remove_header: Option<String>,
    pub stop_rule_processing: Option<bool>,
    pub route_connector: Option<String>,
    pub rule_error_action: Option<String>,
    pub audit_severity: Option<String>,
    pub comments: Option<String>,
    pub when_changed: Option<DateTime<Utc>>,
    /// Every non-empty condition, exception, action or date that isn't one
    /// of the wanted parameters: `(parameter, value)`. For ours it blocks
    /// repair (D12); for others it describes the rule in rows (values shown
    /// to the admin, never written to the run log).
    pub extras: Vec<(String, String)>,
    /// `Conditions`, `Exceptions`, `Actions` type names when the output has
    /// them (R18).
    pub kinds: Option<Vec<String>>,
    pub raw: Value,
}

/// Condition, exception and action types a rule of ours may have.
const OWN_KINDS: &[&str] = &[
    "headermatches",
    "headercontains",
    "fromscope",
    "rightsprotect",
    "removeheader",
    "auditseverity",
    "stoprule",
];

impl TransportRuleView {
    /// A rule from `Get-TransportRule`; `None` without a name.
    pub fn read(value: &Value) -> Option<Self> {
        let name = get_text(value, "Name")?;
        let mut extras = non_empty(value, EXTRA_PARAMETERS);
        let mut kinds = None;
        for list in ["Conditions", "Exceptions", "Actions"] {
            let Some(items) = field(value, list).and_then(Value::as_array) else {
                continue;
            };
            let found: &mut Vec<String> = kinds.get_or_insert_with(Vec::new);
            for item in items {
                let kind = match item {
                    Value::Object(_) => ["@odata.type", "Type", "Name"]
                        .iter()
                        .find_map(|k| field(item, k).and_then(text)),
                    other => text(other),
                };
                let Some(kind) = kind else {
                    continue;
                };
                let lower = kind.to_ascii_lowercase();
                if !OWN_KINDS.iter().any(|own| lower.contains(own)) {
                    extras.push((list.to_owned(), kind.clone()));
                }
                found.push(kind);
            }
        }
        Some(Self {
            guid: get_text(value, "Guid").map(|g| g.to_ascii_lowercase()),
            identity: get_text(value, "Identity").unwrap_or_else(|| name.clone()),
            priority: field(value, "Priority").and_then(integer),
            state: field(value, "State")
                .and_then(Value::as_str)
                .map(|s| s.trim().to_owned()),
            mode: field(value, "Mode")
                .and_then(Value::as_str)
                .map(|s| s.trim().to_owned()),
            from_scope: get_text(value, "FromScope"),
            except_if_from_scope: get_text(value, "ExceptIfFromScope"),
            header_matches_header: get_text(value, "HeaderMatchesMessageHeader"),
            header_matches_patterns: get_texts(value, "HeaderMatchesPatterns"),
            header_contains_header: get_text(value, "HeaderContainsMessageHeader"),
            header_contains_words: get_texts(value, "HeaderContainsWords"),
            template: get_text(value, "ApplyRightsProtectionTemplate"),
            remove_header: get_text(value, "RemoveHeader"),
            stop_rule_processing: get_bool(value, "StopRuleProcessing"),
            route_connector: get_text(value, "RouteMessageOutboundConnector"),
            rule_error_action: get_text(value, "RuleErrorAction"),
            audit_severity: get_text(value, "SetAuditSeverity"),
            comments: get_text(value, "Comments"),
            when_changed: field(value, "WhenChanged").and_then(date),
            extras,
            kinds,
            raw: value.clone(),
            name,
        })
    }

    /// Whether it's on and enforced; unreadable counts as on (it may act).
    pub fn active(&self) -> bool {
        self.state
            .as_deref()
            .is_none_or(|s| s.eq_ignore_ascii_case("Enabled"))
            && self
                .mode
                .as_deref()
                .is_none_or(|m| m.eq_ignore_ascii_case("Enforce"))
    }

    /// An active rule that stops the rules after it.
    pub fn stops(&self) -> bool {
        self.active() && self.stop_rule_processing == Some(true)
    }

    /// An active rule that sends mail out through a connector.
    pub fn routes(&self) -> bool {
        self.active() && self.route_connector.is_some()
    }

    /// Whether later rules may not act on the mail it matches: it stops
    /// them, or sends the mail out through another service.
    pub fn blocks(&self) -> bool {
        self.stops() || self.routes()
    }

    pub fn is_named(&self, name: &str) -> bool {
        self.name.trim().eq_ignore_ascii_case(name)
    }

    /// Its recipient conditions, for a routing rule's row.
    pub fn recipient_conditions(&self) -> Vec<(String, String)> {
        self.extras
            .iter()
            .filter(|(p, _)| {
                let p = p.strip_prefix("ExceptIf").unwrap_or(p);
                p.starts_with("SentTo")
                    || p == "RecipientDomainIs"
                    || p.starts_with("AnyOfRecipientAddress")
            })
            .cloned()
            .collect()
    }
}

/// An accepted domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedDomainView {
    pub domain: String,
    /// The organization's `*.onmicrosoft.com` domain.
    pub initial: bool,
}

impl AcceptedDomainView {
    pub fn read(value: &Value) -> Option<Self> {
        let domain = get_text(value, "DomainName").or_else(|| get_text(value, "Name"))?;
        Some(Self {
            initial: get_bool(value, "InitialDomain").unwrap_or(false),
            domain: domain.to_ascii_lowercase(),
        })
    }
}

/// An inbound connector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundConnectorView {
    pub name: String,
    pub identity: String,
    pub connector_type: Option<String>,
    pub treat_as_internal: Option<bool>,
    pub cloud_services: Option<bool>,
    pub tls_name: Option<String>,
}

impl InboundConnectorView {
    pub fn read(value: &Value) -> Option<Self> {
        let name = get_text(value, "Name")?;
        Some(Self {
            identity: get_text(value, "Identity").unwrap_or_else(|| name.clone()),
            connector_type: get_text(value, "ConnectorType"),
            treat_as_internal: get_bool(value, "TreatMessagesAsInternal"),
            cloud_services: get_bool(value, "CloudServicesMailEnabled"),
            tls_name: get_text(value, "TlsSenderCertificateName"),
            name,
        })
    }
}

/// An outbound connector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundConnectorView {
    pub name: String,
    pub identity: String,
    pub connector_type: Option<String>,
    pub rule_scoped: Option<bool>,
}

impl OutboundConnectorView {
    pub fn read(value: &Value) -> Option<Self> {
        let name = get_text(value, "Name")?;
        Some(Self {
            identity: get_text(value, "Identity").unwrap_or_else(|| name.clone()),
            connector_type: get_text(value, "ConnectorType"),
            rule_scoped: get_bool(value, "IsTransportRuleScoped"),
            name,
        })
    }
}

/// Whether Message Encryption is on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IrmView {
    pub azure_rms: Option<bool>,
    pub internal: Option<bool>,
}

impl IrmView {
    pub fn read(value: &Value) -> Self {
        Self {
            azure_rms: get_bool(value, "AzureRMSLicensingEnabled"),
            internal: get_bool(value, "InternalLicensingEnabled"),
        }
    }
}

/// A rights protection template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RmsTemplateView {
    pub name: String,
    pub id: Option<String>,
}

impl RmsTemplateView {
    pub fn read(value: &Value) -> Option<Self> {
        Some(Self {
            name: get_text(value, "Name")?,
            id: get_text(value, "TemplateGuid")
                .or_else(|| get_text(value, "Guid"))
                .or_else(|| get_text(value, "Id")),
        })
    }
}

/// A sensitivity label as Purview lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct LabelView {
    /// Lowercase.
    pub guid: String,
    /// Lowercase.
    pub immutable_id: Option<String>,
    pub name: String,
    pub display_name: Option<String>,
    pub parent_id: Option<String>,
    /// "File, Email, ..." when Purview says; `None` when the property
    /// doesn't read.
    pub content_type: Option<String>,
    pub raw: Value,
}

impl LabelView {
    pub fn read(value: &Value) -> Option<Self> {
        let guid = get_text(value, "Guid")?.to_ascii_lowercase();
        let name = get_text(value, "Name").unwrap_or_else(|| guid.clone());
        Some(Self {
            immutable_id: get_text(value, "ImmutableId").map(|i| i.to_ascii_lowercase()),
            display_name: get_text(value, "DisplayName"),
            parent_id: get_text(value, "ParentId").map(|p| p.to_ascii_lowercase()),
            content_type: field(value, "ContentType")
                .map(texts)
                .filter(|t| !t.is_empty())
                .map(|t| t.join(", ")),
            raw: value.clone(),
            guid,
            name,
        })
    }

    /// Purview says whether it's for email: `None` when it doesn't.
    pub fn for_email(&self) -> Option<bool> {
        self.content_type.as_deref().map(|types| {
            types
                .split(',')
                .any(|t| t.trim().eq_ignore_ascii_case("email"))
        })
    }

    /// It encrypts, when Purview says.
    pub fn encrypts(&self) -> bool {
        get_bool(&self.raw, "EncryptionEnabled").unwrap_or(false)
    }

    pub fn title(&self) -> String {
        self.display_name
            .clone()
            .unwrap_or_else(|| self.name.clone())
    }

    /// Whether `id` (a catalog ID) names it.
    pub fn is(&self, id: &str) -> bool {
        self.guid.eq_ignore_ascii_case(id)
            || self
                .immutable_id
                .as_deref()
                .is_some_and(|i| i.eq_ignore_ascii_case(id))
    }
}

/// Where an auto-labeling policy's simulation stands (call 10, R9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimulationState {
    NotRun,
    Running,
    Complete { at: Option<DateTime<Utc>> },
    Unknown,
}

/// Where an auto-labeling policy's distribution stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DistributionState {
    Pending,
    Success,
    Error(String),
    Unknown,
}

/// An auto-labeling policy.
#[derive(Debug, Clone, PartialEq)]
pub struct AutoPolicyView {
    pub name: String,
    pub identity: String,
    pub mode: Option<String>,
    /// The separate Enabled flag; `None` when unreadable.
    pub enabled: Option<bool>,
    pub label: Option<String>,
    pub exchange_location: Vec<String>,
    /// Non-empty locations other than Exchange email: `(parameter, value)`.
    pub other_locations: Vec<(String, String)>,
    pub sender_member_of: Vec<String>,
    pub overwrite: Option<bool>,
    pub comment: Option<String>,
    pub when_changed: Option<DateTime<Utc>>,
    pub auto_enable_after: Option<String>,
    pub simulation: SimulationState,
    pub distribution: DistributionState,
    pub raw: Value,
}

impl AutoPolicyView {
    pub fn read(value: &Value) -> Option<Self> {
        let name = get_text(value, "Name")?;
        Some(Self {
            identity: get_text(value, "Identity").unwrap_or_else(|| name.clone()),
            mode: field(value, "Mode")
                .and_then(Value::as_str)
                .map(|s| s.trim().to_owned()),
            enabled: get_bool(value, "Enabled"),
            label: get_text(value, "ApplySensitivityLabel"),
            exchange_location: get_texts(value, "ExchangeLocation"),
            other_locations: non_empty(value, POLICY_OTHER_LOCATIONS),
            sender_member_of: get_texts(value, "ExchangeSenderMemberOf"),
            overwrite: get_bool(value, "OverwriteLabel"),
            comment: get_text(value, "Comment"),
            when_changed: field(value, "WhenChanged").and_then(date),
            auto_enable_after: get_text(value, "AutoEnableAfter")
                .filter(|a| !matches!(a.as_str(), "00:00:00" | "0")),
            simulation: simulation(value),
            distribution: distribution(value),
            raw: value.clone(),
            name,
        })
    }

    pub fn is_named(&self, name: &str) -> bool {
        self.name.trim().eq_ignore_ascii_case(name)
    }

    /// In `Enable` mode: labels mail (once distributed).
    pub fn is_on(&self) -> bool {
        self.mode
            .as_deref()
            .is_some_and(|m| m.eq_ignore_ascii_case("Enable"))
    }

    pub fn pending_deletion(&self) -> bool {
        self.mode
            .as_deref()
            .is_some_and(|m| m.eq_ignore_ascii_case("PendingDeletion"))
    }
}

/// The fields whose names mention `words`, case ignored (the simulation's
/// and distribution's property names aren't documented, R9).
fn fields_naming<'v>(value: &'v Value, words: &[&str]) -> Vec<(&'v str, &'v Value)> {
    value
        .as_object()
        .map(|object| {
            object
                .iter()
                .filter(|(k, _)| {
                    let k = k.to_ascii_lowercase();
                    words.iter().any(|w| k.contains(w))
                })
                .map(|(k, v)| (k.as_str(), v))
                .collect()
        })
        .unwrap_or_default()
}

/// What a status text says about a simulation.
fn simulation_word(status: &str) -> Option<SimulationState> {
    let status = status.to_ascii_lowercase();
    if status.contains("complet") || status.contains("finished") || status.contains("succeed") {
        Some(SimulationState::Complete { at: None })
    } else if status.contains("notstarted") || status.contains("not started") || status == "none" {
        Some(SimulationState::NotRun)
    } else if status.contains("progress")
        || status.contains("running")
        || status.contains("started")
        || status.contains("pending")
        || status.contains("queued")
    {
        Some(SimulationState::Running)
    } else {
        None
    }
}

fn simulation(value: &Value) -> SimulationState {
    let mut state = SimulationState::Unknown;
    let mut at = None;
    for (key, field_value) in fields_naming(value, &["simulation", "testmode"]) {
        let candidates: Vec<(&str, &Value)> = match field_value {
            Value::Object(object) => object.iter().map(|(k, v)| (k.as_str(), v)).collect(),
            other => vec![(key, other)],
        };
        for (name, v) in candidates {
            let name = name.to_ascii_lowercase();
            if (name.contains("time") || name.contains("date") || name.contains("completed"))
                && let Some(d) = date(v)
            {
                at = at.max(Some(d));
                continue;
            }
            if let Some(found) = v.as_str().and_then(simulation_word)
                && state == SimulationState::Unknown
            {
                state = found;
            }
        }
    }
    match state {
        SimulationState::Complete { .. } => SimulationState::Complete { at },
        other => other,
    }
}

fn distribution(value: &Value) -> DistributionState {
    let status = field(value, "DistributionStatus").and_then(text);
    let results = field(value, "DistributionResults")
        .map(texts)
        .unwrap_or_default()
        .join("; ");
    match status.as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("success") => DistributionState::Success,
        Some("pending") => DistributionState::Pending,
        Some("error" | "failed" | "failure") => DistributionState::Error(
            crate::protection::clean_value(&results, 300).unwrap_or_else(|| "Error".into()),
        ),
        _ => DistributionState::Unknown,
    }
}

/// An auto-labeling policy's rule.
#[derive(Debug, Clone, PartialEq)]
pub struct AutoRuleView {
    pub name: String,
    pub identity: String,
    /// `ParentPolicyName`, else `Policy`.
    pub policy: Option<String>,
    pub workload: Option<String>,
    /// `(header, pattern)` pairs, lists flattened.
    pub header_patterns: Vec<(String, String)>,
    pub disabled: Option<bool>,
    /// Any other non-empty condition or exception.
    pub other_conditions: Vec<String>,
    pub when_changed: Option<DateTime<Utc>>,
    pub raw: Value,
}

impl AutoRuleView {
    pub fn read(value: &Value) -> Option<Self> {
        let name = get_text(value, "Name")?;
        Some(Self {
            identity: get_text(value, "Identity").unwrap_or_else(|| name.clone()),
            policy: get_text(value, "ParentPolicyName").or_else(|| get_text(value, "Policy")),
            workload: get_text(value, "Workload"),
            header_patterns: field(value, "HeaderMatchesPatterns")
                .map(header_patterns)
                .unwrap_or_default(),
            disabled: get_bool(value, "Disabled"),
            other_conditions: non_empty(value, AUTO_RULE_CONDITIONS)
                .into_iter()
                .map(|(p, _)| p)
                .collect(),
            when_changed: field(value, "WhenChanged").and_then(date),
            raw: value.clone(),
            name,
        })
    }

    pub fn is_named(&self, name: &str) -> bool {
        self.name.trim().eq_ignore_ascii_case(name)
    }

    /// Whether it belongs to the policy `name`.
    pub fn in_policy(&self, name: &str) -> bool {
        self.policy
            .as_deref()
            .is_some_and(|p| p.trim().eq_ignore_ascii_case(name))
    }
}

/// `HeaderMatchesPatterns` in any of its shapes: an object
/// (`{"x-cumulas-label": "…"}`, `@odata.type` ignored), an array of
/// `{Key, Value}`, or text `key: value` / `key=value`; values may be arrays.
pub fn header_patterns(value: &Value) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut add = |key: &str, value: &Value| {
        for pattern in texts(value) {
            pairs.push((key.trim().to_owned(), pattern));
        }
    };
    match value {
        Value::Object(object) => {
            for (key, v) in object.iter().filter(|(k, _)| !k.starts_with('@')) {
                add(key, v);
            }
        }
        Value::Array(items) => {
            for item in items {
                match (field(item, "Key").and_then(text), field(item, "Value")) {
                    (Some(key), Some(v)) => add(&key, v),
                    _ => {
                        if let Some(line) = item.as_str() {
                            add_line(line, &mut add);
                        }
                    }
                }
            }
        }
        Value::String(line) => add_line(line, &mut add),
        _ => {}
    }
    pairs
}

fn add_line(line: &str, add: &mut impl FnMut(&str, &Value)) {
    let split = line.split_once(':').or_else(|| line.split_once('='));
    if let Some((key, pattern)) = split {
        add(key, &Value::String(pattern.trim().to_owned()));
    }
}
