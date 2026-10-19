//! A Microsoft 365 organization behind a mock server: its token endpoint,
//! the Security & Compliance discovery, and InvokeCommand for the cmdlets
//! setup uses, over an in-memory model of the rules and policies. Scripted
//! answers go first, once each, so tests can make any call fail.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{Map, Value, json};
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use super::{TENANT, token_response};

/// One InvokeCommand request.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Recorded {
    pub cmdlet: String,
    pub params: Map<String, Value>,
    pub anchor: String,
}

#[derive(Default)]
pub(crate) struct TenantState {
    pub accepted: Vec<Value>,
    pub irm: Value,
    pub templates: Vec<Value>,
    /// In priority order.
    pub rules: Vec<Value>,
    pub inbound: Vec<Value>,
    pub outbound: Vec<Value>,
    pub labels: Vec<Value>,
    pub policies: Vec<Value>,
    pub policy_rules: Vec<Value>,
    /// Answers given instead of the model's, once each, for the cmdlet
    /// named, after letting that many of its calls through to the model.
    pub scripted: Vec<(String, usize, ResponseTemplate)>,
    pub calls: Vec<Recorded>,
    /// Objects left out of this many more listings, by name.
    pub hidden: HashMap<String, usize>,
    pub token_requests: usize,
    pub discovery_requests: usize,
}

/// A handle on the organization; clones share it.
#[derive(Clone, Default)]
pub(crate) struct FakeTenant {
    pub state: Arc<Mutex<TenantState>>,
}

impl FakeTenant {
    /// An organization with Message Encryption on, the usual templates, no
    /// rules of ours, three labels and no policies.
    pub fn fresh() -> Self {
        let tenant = Self::default();
        {
            let mut s = tenant.state.lock().unwrap();
            s.accepted = vec![
                json!({ "DomainName": "contoso.com", "InitialDomain": false }),
                json!({ "DomainName": "contoso.onmicrosoft.com", "InitialDomain": true }),
            ];
            s.irm = json!({ "AzureRMSLicensingEnabled": true, "InternalLicensingEnabled": true });
            s.templates = vec![
                json!({ "Name": "Encrypt", "TemplateGuid": "11111111-0000-0000-0000-000000000001" }),
                json!({ "Name": "Do Not Forward", "TemplateGuid": "11111111-0000-0000-0000-000000000002" }),
            ];
            s.labels = vec![
                label(super::CONFIDENTIAL, "Confidential"),
                label(super::GENERAL, "General"),
                label(super::SECRET, "Highly Confidential"),
            ];
        }
        tenant
    }

    pub async fn mount(&self, server: &MockServer) {
        Mock::given(any())
            .respond_with(self.clone())
            .mount(server)
            .await;
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut TenantState) -> R) -> R {
        f(&mut self.state.lock().unwrap())
    }

    /// The next `cmdlet` call gets `response`.
    pub fn script(&self, cmdlet: &str, response: ResponseTemplate) {
        self.script_after(cmdlet, 0, response);
    }

    /// The `cmdlet` call after the next `skip` gets `response`.
    pub fn script_after(&self, cmdlet: &str, skip: usize, response: ResponseTemplate) {
        self.with(|s| s.scripted.push((cmdlet.to_owned(), skip, response)));
    }

    pub fn calls(&self) -> Vec<Recorded> {
        self.with(|s| s.calls.clone())
    }

    /// The writes sent, in order: (cmdlet, parameters).
    pub fn writes(&self) -> Vec<(String, Map<String, Value>)> {
        self.calls()
            .into_iter()
            .filter(|c| !c.cmdlet.starts_with("Get-"))
            .map(|c| (c.cmdlet, c.params))
            .collect()
    }

    pub fn rule(&self, name: &str) -> Option<Value> {
        self.with(|s| s.rules.iter().find(|r| r["Name"] == name).cloned())
    }

    pub fn rule_names(&self) -> Vec<String> {
        self.with(|s| {
            s.rules
                .iter()
                .map(|r| r["Name"].as_str().unwrap_or_default().to_owned())
                .collect()
        })
    }

    pub fn policy(&self, name: &str) -> Option<Value> {
        self.with(|s| s.policies.iter().find(|p| p["Name"] == name).cloned())
    }

    pub fn policy_rule(&self, name: &str) -> Option<Value> {
        self.with(|s| s.policy_rules.iter().find(|p| p["Name"] == name).cloned())
    }
}

/// A label as `Get-Label` lists it.
pub(crate) fn label(guid: &str, name: &str) -> Value {
    json!({
        "Guid": guid, "ImmutableId": guid, "Name": name, "DisplayName": name,
        "ContentType": "File, Email", "ParentId": null,
    })
}

/// A rule as `Get-TransportRule` lists it, with everything empty but
/// `fields`.
pub(crate) fn rule(name: &str, fields: Value) -> Value {
    let mut rule = json!({
        "Name": name, "Guid": uuid::Uuid::new_v4().to_string(), "Identity": name,
        "State": "Enabled", "Mode": "Enforce",
        "FromScope": null, "ExceptIfFromScope": null,
        "HeaderMatchesMessageHeader": null, "HeaderMatchesPatterns": [],
        "HeaderContainsMessageHeader": null, "HeaderContainsWords": [],
        "ApplyRightsProtectionTemplate": null, "RemoveHeader": null,
        "StopRuleProcessing": false, "RouteMessageOutboundConnector": null,
        "BlindCopyTo": [], "RedirectMessageTo": null, "ActivationDate": null,
        "SentToScope": null, "RecipientDomainIs": [],
        "RuleErrorAction": "Ignore", "SetAuditSeverity": "", "Comments": "",
        "WhenChanged": "2026-10-01T09:00:00Z",
    });
    if let Value::Object(fields) = fields {
        for (key, value) in fields {
            rule[key] = value;
        }
    }
    rule
}

fn envelope(values: Vec<Value>) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "@odata.context": format!("https://outlook.office365.com/adminapi/beta/{TENANT}/$metadata#Collection(Exchange.GenericHashTable)"),
        "value": values,
    }))
}

fn not_found(identity: &str) -> ResponseTemplate {
    ResponseTemplate::new(404).set_body_json(json!({
        "error": { "code": "NotFound", "message": format!("The operation couldn't be performed because object '{identity}' couldn't be found.") }
    }))
}

fn now() -> Value {
    json!(chrono::Utc::now().to_rfc3339())
}

/// A value the model keeps for one a cmdlet sent: an emptied value as
/// what Exchange returns for it.
fn stored(value: &Value) -> Value {
    match value {
        Value::String(s) if s.is_empty() => Value::Null,
        Value::Object(object) => {
            let mut object = object.clone();
            object.remove("@odata.type");
            Value::Object(object)
        }
        other => other.clone(),
    }
}

impl TenantState {
    /// The scripted answer for this call of `cmdlet`, if any.
    fn scripted_for(&mut self, cmdlet: &str) -> Option<ResponseTemplate> {
        let at = self.scripted.iter().position(|(c, _, _)| c == cmdlet)?;
        if self.scripted[at].1 > 0 {
            self.scripted[at].1 -= 1;
            return None;
        }
        Some(self.scripted.remove(at).2)
    }

    fn listed(&mut self, which: &str) -> Vec<Value> {
        let list = match which {
            "policies" => self.policies.clone(),
            "transport" => self.rules.clone(),
            _ => self.policy_rules.clone(),
        };
        list.into_iter()
            .filter(|item| {
                let name = item["Name"].as_str().unwrap_or_default().to_owned();
                match self.hidden.get_mut(&name) {
                    Some(left) if *left > 0 => {
                        *left -= 1;
                        false
                    }
                    _ => true,
                }
            })
            .collect()
    }

    fn invoke(&mut self, cmdlet: &str, params: &Map<String, Value>) -> ResponseTemplate {
        let identity = params
            .get("Identity")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        match cmdlet {
            "Get-AcceptedDomain" => envelope(self.accepted.clone()),
            "Get-IRMConfiguration" => envelope(vec![self.irm.clone()]),
            "Get-RMSTemplate" => envelope(self.templates.clone()),
            "Get-TransportRule" => {
                let order: Vec<String> = self
                    .rules
                    .iter()
                    .map(|r| r["Name"].as_str().unwrap_or_default().to_owned())
                    .collect();
                let rules = self
                    .listed("transport")
                    .into_iter()
                    .map(|mut r| {
                        let name = r["Name"].as_str().unwrap_or_default().to_owned();
                        r["Priority"] = json!(order.iter().position(|n| *n == name));
                        r
                    })
                    .collect();
                envelope(rules)
            }
            "Get-InboundConnector" => envelope(self.inbound.clone()),
            "Get-OutboundConnector" => envelope(self.outbound.clone()),
            "New-TransportRule" => {
                let name = params["Name"].as_str().unwrap_or_default();
                if self.rules.iter().any(|r| r["Name"] == name) {
                    return ResponseTemplate::new(400).set_body_json(json!({
                        "error": { "code": "BadRequest", "message": format!("A rule named '{name}' already exists.") }
                    }));
                }
                let mut new = rule(name, json!({}));
                for (key, value) in params {
                    if key != "Priority" {
                        new[key] = stored(value);
                    }
                }
                new["WhenChanged"] = now();
                let at = params
                    .get("Priority")
                    .and_then(Value::as_u64)
                    .map_or(self.rules.len(), |p| (p as usize).min(self.rules.len()));
                self.rules.insert(at, new.clone());
                envelope(vec![new])
            }
            "Set-TransportRule" | "Enable-TransportRule" => {
                let Some(at) = self
                    .rules
                    .iter()
                    .position(|r| r["Guid"] == identity.as_str() || r["Name"] == identity.as_str())
                else {
                    return not_found(&identity);
                };
                let mut rule = self.rules.remove(at);
                let mut to = at;
                for (key, value) in params {
                    match key.as_str() {
                        "Identity" => {}
                        "Priority" => to = value.as_u64().unwrap_or(0) as usize,
                        "Enabled" => {
                            return ResponseTemplate::new(400).set_body_json(json!({
                                "error": { "message": "A parameter cannot be found that matches parameter name 'Enabled'." }
                            }));
                        }
                        _ => rule[key] = stored(value),
                    }
                }
                if cmdlet == "Enable-TransportRule" {
                    rule["State"] = json!("Enabled");
                }
                rule["WhenChanged"] = now();
                self.rules.insert(to.min(self.rules.len()), rule);
                ResponseTemplate::new(200)
            }
            "Get-Label" => envelope(self.labels.clone()),
            "Get-AutoSensitivityLabelPolicy" => {
                if identity.is_empty() {
                    let listed = self.listed("policies");
                    envelope(listed)
                } else {
                    match self
                        .policies
                        .iter()
                        .find(|p| p["Name"] == identity.as_str())
                    {
                        Some(p) => envelope(vec![p.clone()]),
                        None => not_found(&identity),
                    }
                }
            }
            "New-AutoSensitivityLabelPolicy" => {
                let name = params["Name"].as_str().unwrap_or_default();
                if self.policies.iter().any(|p| p["Name"] == name) {
                    return ResponseTemplate::new(400).set_body_json(json!({
                        "error": { "code": "BadRequest", "message": format!("The policy '{name}' already exists.") }
                    }));
                }
                let mut policy = json!({
                    "Enabled": true, "AutoEnableAfter": null, "DistributionStatus": "Pending",
                    "SharePointLocation": [], "OneDriveLocation": [], "ExchangeSenderMemberOf": [],
                });
                for (key, value) in params {
                    policy[key] = stored(value);
                }
                policy["Identity"] = params["Name"].clone();
                policy["WhenChanged"] = now();
                self.policies.push(policy.clone());
                envelope(vec![policy])
            }
            "Set-AutoSensitivityLabelPolicy" => {
                let Some(policy) = self
                    .policies
                    .iter_mut()
                    .find(|p| p["Name"] == identity.as_str())
                else {
                    return not_found(&identity);
                };
                for (key, value) in params {
                    match key.as_str() {
                        "Identity" => {}
                        "StartSimulation" => policy["SimulationStatus"] = json!("InProgress"),
                        "AddExchangeLocation" => {
                            let mut locations = policy["ExchangeLocation"]
                                .as_array()
                                .cloned()
                                .unwrap_or_default();
                            locations.extend(value.as_array().cloned().unwrap_or_default());
                            policy["ExchangeLocation"] = json!(locations);
                        }
                        _ => policy[key] = stored(value),
                    }
                }
                policy["WhenChanged"] = now();
                ResponseTemplate::new(200)
            }
            "Get-AutoSensitivityLabelRule" => {
                let listed = self.listed("rules");
                envelope(listed)
            }
            "New-AutoSensitivityLabelRule" => {
                let name = params["Name"].as_str().unwrap_or_default();
                if self.policy_rules.iter().any(|r| r["Name"] == name) {
                    return ResponseTemplate::new(400).set_body_json(json!({
                        "error": { "code": "BadRequest", "message": format!("The rule '{name}' already exists.") }
                    }));
                }
                let mut rule = json!({ "Disabled": false });
                for (key, value) in params {
                    rule[key] = stored(value);
                }
                rule["Identity"] = params["Name"].clone();
                rule["ParentPolicyName"] = params["Policy"].clone();
                rule["WhenChanged"] = now();
                self.policy_rules.push(rule.clone());
                envelope(vec![rule])
            }
            "Set-AutoSensitivityLabelRule" => {
                let Some(rule) = self
                    .policy_rules
                    .iter_mut()
                    .find(|r| r["Name"] == identity.as_str())
                else {
                    return not_found(&identity);
                };
                for (key, value) in params {
                    if key != "Identity" {
                        rule[key] = stored(value);
                    }
                }
                rule["WhenChanged"] = now();
                ResponseTemplate::new(200)
            }
            other => ResponseTemplate::new(400).set_body_json(json!({
                "error": { "message": format!("The term '{other}' is not known to this fake.") }
            })),
        }
    }
}

impl Respond for FakeTenant {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let mut state = self.state.lock().unwrap();
        let path = request.url.path();
        if path.ends_with("/oauth2/v2.0/token") {
            state.token_requests += 1;
            let form: HashMap<String, String> = url::form_urlencoded::parse(&request.body)
                .into_owned()
                .collect();
            let scope = form.get("scope").cloned().unwrap_or_default();
            let access = if scope.contains("ps.compliance") {
                "compliance-token"
            } else {
                "exchange-token"
            };
            return token_response(access);
        }
        if path.contains("EXOBanner") {
            state.discovery_requests += 1;
            if let Some(response) = state.scripted_for("EXOBanner") {
                return response;
            }
            return ResponseTemplate::new(200).set_body_json(json!({}));
        }
        if !path.ends_with("/InvokeCommand") {
            return ResponseTemplate::new(404);
        }
        let body: Value = serde_json::from_slice(&request.body).unwrap_or_default();
        let cmdlet = body["CmdletInput"]["CmdletName"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let params = body["CmdletInput"]["Parameters"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        let anchor = request
            .headers
            .get("X-AnchorMailbox")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        state.calls.push(Recorded {
            cmdlet: cmdlet.clone(),
            params: params.clone(),
            anchor,
        });
        if let Some(response) = state.scripted_for(&cmdlet) {
            return response;
        }
        state.invoke(&cmdlet, &params)
    }
}
