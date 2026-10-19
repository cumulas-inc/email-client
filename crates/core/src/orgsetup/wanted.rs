//! What Cumulas Mail's own rules and policies look like when they're right,
//! which objects are ours, and what an administrator's choices may name.

use super::objects::LabelView;
use super::status::{Part, SetupContext, Snapshot};
use crate::error::{Error, Result};
use crate::model::OrgSetupChoices;
use crate::signatures::config::is_guid;

/// The start of every name of ours.
pub const PREFIX: &str = "Cumulas Mail: ";
pub const ENCRYPT_RULE: &str = "Cumulas Mail: Encrypt";
pub const DNF_RULE: &str = "Cumulas Mail: Do Not Forward";
pub const STRIP_RULE: &str = "Cumulas Mail: ignore label requests from outside";
/// On every rule and policy we create or change: a pointer for admins, not
/// how we know it's ours (that's the name).
pub const MARKER: &str = "Managed by Cumulas Mail (cumulas-mail:v1). Check or repair it from \
                          Cumulas Mail: Settings > Accounts > Sensitivity labels and encryption \
                          > Set up or check for your organization.";
/// The earlier relay recipe's strip rule, for its stamped header.
pub const LEFTOVER_STRIP_RULE: &str = "Cumulas Mail: ignore stamped label requests from outside";
/// The earlier relay recipe's per-label rules end with this.
pub const LEFTOVER_SUFFIX: &str = " after the signature";
/// Cumulas Signatures' routing rule (read only).
pub const RELAY_RULE: &str = "Email signatures: add signature";
/// Cumulas Signatures' inbound connector (read only).
pub const RELAY_INBOUND: &str = "Email signatures (from signature server)";
/// Transport rule names are at most 64 characters (documented); the same
/// cap for policies.
pub const MAX_NAME: usize = 64;
/// The header people's Cumulas Mail asks for a label in.
pub const LABEL_HEADER: &str = "x-cumulas-label";
/// The earlier relay recipe's label header.
pub const STAMPED_LABEL_HEADER: &str = "x-cumulas-label-stamped";
/// The header people's Cumulas Mail asks for encryption in.
pub const PROTECT_HEADER: &str = "x-cumulas-protect";
pub const ENCRYPT_PATTERN: &str = "^encrypt$";
pub const DNF_PATTERN: &str = "^do-not-forward$";
/// The rights protection templates' usual names.
pub const ENCRYPT_TEMPLATE: &str = "Encrypt";
pub const DNF_TEMPLATE: &str = "Do Not Forward";
/// `AutoEnableAfter`: the documented minimum, an hour after the simulation.
pub const AUTO_ENABLE_AFTER: &str = "01:00:00";
/// Longest template name a script takes without a session to check it.
const MAX_TEMPLATE: usize = 100;

/// Conditions, exceptions, actions and dates of `New-TransportRule`
/// (scratchpad/docs/New-TransportRule.md) that Cumulas Mail never sets:
/// a rule of ours with any of them is left alone (D12). Without the wanted
/// parameters (3.7.2), the cmdlet's own switches, and the companion
/// settings in [`COMPANION_DEFAULTS`].
#[rustfmt::skip]
pub const EXTRA_PARAMETERS: &[&str] = &[
    "ActivationDate", "ADComparisonAttribute", "AddManagerAsRecipientType", "AddToRecipients",
    "AnyOfCcHeader", "AnyOfCcHeaderMemberOf", "AnyOfRecipientAddressContainsWords",
    "AnyOfRecipientAddressMatchesPatterns", "AnyOfToCcHeader", "AnyOfToCcHeaderMemberOf",
    "AnyOfToHeader", "AnyOfToHeaderMemberOf", "ApplyClassification", "ApplyHtmlDisclaimerText",
    "ApplyOME", "ApplyRightsProtectionCustomizationTemplate", "AttachmentContainsWords",
    "AttachmentExtensionMatchesWords", "AttachmentHasExecutableContent",
    "AttachmentIsPasswordProtected", "AttachmentIsUnsupported", "AttachmentMatchesPatterns",
    "AttachmentNameMatchesPatterns", "AttachmentProcessingLimitExceeded",
    "AttachmentPropertyContainsWords", "AttachmentSizeOver", "BetweenMemberOf1",
    "BetweenMemberOf2", "BlindCopyTo", "ContentCharacterSetContainsWords", "CopyTo",
    "DeleteMessage", "Disconnect", "DlpPolicy", "ExceptIfADComparisonAttribute",
    "ExceptIfAnyOfCcHeader", "ExceptIfAnyOfCcHeaderMemberOf",
    "ExceptIfAnyOfRecipientAddressContainsWords", "ExceptIfAnyOfRecipientAddressMatchesPatterns",
    "ExceptIfAnyOfToCcHeader", "ExceptIfAnyOfToCcHeaderMemberOf", "ExceptIfAnyOfToHeader",
    "ExceptIfAnyOfToHeaderMemberOf", "ExceptIfAttachmentContainsWords",
    "ExceptIfAttachmentExtensionMatchesWords", "ExceptIfAttachmentHasExecutableContent",
    "ExceptIfAttachmentIsPasswordProtected", "ExceptIfAttachmentIsUnsupported",
    "ExceptIfAttachmentMatchesPatterns", "ExceptIfAttachmentNameMatchesPatterns",
    "ExceptIfAttachmentProcessingLimitExceeded", "ExceptIfAttachmentPropertyContainsWords",
    "ExceptIfAttachmentSizeOver", "ExceptIfBetweenMemberOf1", "ExceptIfBetweenMemberOf2",
    "ExceptIfContentCharacterSetContainsWords", "ExceptIfFrom",
    "ExceptIfFromAddressContainsWords", "ExceptIfFromAddressMatchesPatterns",
    "ExceptIfFromMemberOf", "ExceptIfHasClassification", "ExceptIfHasNoClassification",
    "ExceptIfHasSenderOverride", "ExceptIfHeaderContainsMessageHeader",
    "ExceptIfHeaderContainsWords", "ExceptIfHeaderMatchesMessageHeader",
    "ExceptIfHeaderMatchesPatterns", "ExceptIfManagerAddresses",
    "ExceptIfManagerForEvaluatedUser", "ExceptIfMessageContainsDataClassifications",
    "ExceptIfMessageSizeOver", "ExceptIfMessageTypeMatches",
    "ExceptIfRecipientADAttributeContainsWords", "ExceptIfRecipientADAttributeMatchesPatterns",
    "ExceptIfRecipientAddressContainsWords", "ExceptIfRecipientAddressMatchesPatterns",
    "ExceptIfRecipientDomainIs", "ExceptIfRecipientInSenderList", "ExceptIfSCLOver",
    "ExceptIfSenderADAttributeContainsWords", "ExceptIfSenderADAttributeMatchesPatterns",
    "ExceptIfSenderDomainIs", "ExceptIfSenderInRecipientList", "ExceptIfSenderIpRanges",
    "ExceptIfSenderManagementRelationship", "ExceptIfSentTo", "ExceptIfSentToMemberOf",
    "ExceptIfSentToScope", "ExceptIfSubjectContainsWords", "ExceptIfSubjectMatchesPatterns",
    "ExceptIfSubjectOrBodyContainsWords", "ExceptIfSubjectOrBodyMatchesPatterns",
    "ExceptIfWithImportance", "ExpiryDate", "From", "FromAddressContainsWords",
    "FromAddressMatchesPatterns", "FromMemberOf", "GenerateIncidentReport",
    "GenerateNotification", "HasClassification", "HasNoClassification", "HasSenderOverride",
    "IncidentReportContent", "LogEventText", "ManagerAddresses", "ManagerForEvaluatedUser",
    "MessageContainsDataClassifications", "MessageSizeOver", "MessageTypeMatches",
    "ModerateMessageByManager", "ModerateMessageByUser", "NotifySender", "PrependSubject",
    "Quarantine", "RecipientADAttributeContainsWords", "RecipientADAttributeMatchesPatterns",
    "RecipientAddressContainsWords", "RecipientAddressMatchesPatterns", "RecipientDomainIs",
    "RecipientInSenderList", "RedirectMessageTo", "RejectMessageEnhancedStatusCode",
    "RejectMessageReasonText", "RemoveOME", "RemoveOMEv2", "RemoveRMSAttachmentEncryption",
    "RouteMessageOutboundConnector", "RouteMessageOutboundRequireTls", "SCLOver",
    "SenderADAttributeContainsWords", "SenderADAttributeMatchesPatterns", "SenderDomainIs",
    "SenderInRecipientList", "SenderIpRanges", "SenderManagementRelationship", "SentTo",
    "SentToMemberOf", "SentToScope", "SetHeaderName", "SetHeaderValue", "SetSCL",
    "SmtpRejectMessageRejectStatusCode", "SmtpRejectMessageRejectText", "SubjectContainsWords",
    "SubjectMatchesPatterns", "SubjectOrBodyContainsWords", "SubjectOrBodyMatchesPatterns",
    "WithImportance",
];

/// Companion settings of `New-TransportRule` that always carry a value (the
/// documented default when not set), so they're never counted as extras.
pub const COMPANION_DEFAULTS: &[(&str, &str)] = &[
    ("ADComparisonOperator", "Equal"),
    ("ApplyHtmlDisclaimerFallbackAction", "Wrap"),
    ("ApplyHtmlDisclaimerLocation", "Append"),
    ("ExceptIfADComparisonOperator", "Equal"),
    ("IncidentReportOriginalMail", "DoNotIncludeOriginalMail"),
    ("RecipientAddressType", "Resolved"),
    ("RuleSubType", "None"),
    ("SenderAddressLocation", "Header"),
    ("UseLegacyRegex", "False"),
];

/// The parameters Cumulas Mail's mail flow rules set or clear (3.7.2).
pub const WANTED_RULE_PARAMETERS: &[&str] = &[
    "Name",
    "Priority",
    "Mode",
    "Comments",
    "Enabled",
    "FromScope",
    "ExceptIfFromScope",
    "HeaderMatchesMessageHeader",
    "HeaderMatchesPatterns",
    "HeaderContainsMessageHeader",
    "HeaderContainsWords",
    "ApplyRightsProtectionTemplate",
    "RemoveHeader",
    "RuleErrorAction",
    "SetAuditSeverity",
    "StopRuleProcessing",
];

/// Conditions and exceptions of `New-AutoSensitivityLabelRule` other than
/// the header pattern: our rule has none (D12). `AdvancedRule` isn't here:
/// it restates the rule's conditions when set (unverified, R18), and our
/// repair only ever sets the header pattern.
#[rustfmt::skip]
pub const AUTO_RULE_CONDITIONS: &[&str] = &[
    "AccessScope", "ActivationDate", "AnyOfRecipientAddressContainsWords",
    "AnyOfRecipientAddressMatchesPatterns", "ContentContainsSensitiveInformation",
    "ContentExtensionMatchesWords", "ContentPropertyContainsWords", "DocumentCreatedBy",
    "DocumentIsPasswordProtected", "DocumentIsUnsupported", "DocumentNameMatchesWords",
    "DocumentSizeOver", "ExceptIfAccessScope", "ExceptIfAnyOfRecipientAddressContainsWords",
    "ExceptIfAnyOfRecipientAddressMatchesPatterns", "ExceptIfContentContainsSensitiveInformation",
    "ExceptIfContentExtensionMatchesWords", "ExceptIfContentPropertyContainsWords",
    "ExceptIfDocumentCreatedBy", "ExceptIfDocumentIsPasswordProtected",
    "ExceptIfDocumentIsUnsupported", "ExceptIfDocumentNameMatchesWords",
    "ExceptIfDocumentSizeOver", "ExceptIfFrom", "ExceptIfFromAddressContainsWords",
    "ExceptIfFromAddressMatchesPatterns", "ExceptIfFromMemberOf", "ExceptIfHeaderMatchesPatterns",
    "ExceptIfProcessingLimitExceeded", "ExceptIfRecipientDomainIs", "ExceptIfSenderDomainIs",
    "ExceptIfSenderIPRanges", "ExceptIfSentTo", "ExceptIfSentToMemberOf",
    "ExceptIfSubjectMatchesPatterns", "ExpiryDate", "From", "FromAddressContainsWords",
    "FromAddressMatchesPatterns", "FromMemberOf", "ProcessingLimitExceeded", "RecipientDomainIs",
    "SenderDomainIs", "SenderIPRanges", "SentTo", "SentToMemberOf", "SubjectMatchesPatterns",
];

/// Conditions among [`AUTO_RULE_CONDITIONS`] that always carry a value,
/// and the one that means the condition isn't used (`None`; also read as
/// the number 0): never counted as another condition.
pub const AUTO_RULE_DEFAULTS: &[(&str, &str)] =
    &[("AccessScope", "None"), ("ExceptIfAccessScope", "None")];

/// Locations of an auto-labeling policy other than Exchange email: ours
/// have none (D12).
pub const POLICY_OTHER_LOCATIONS: &[&str] = &[
    "SharePointLocation",
    "OneDriveLocation",
    "TeamsLocation",
    "SharePointAdaptiveScopes",
    "OneDriveAdaptiveScopes",
];

/// Whether an object is ours: its name starts with "Cumulas Mail: " (ASCII
/// case ignored).
pub fn is_owned(name: &str) -> bool {
    name.trim_start()
        .get(..PREFIX.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(PREFIX))
}

/// Whether a rule of ours is left from the earlier relay recipe: never
/// changed, listed as not used any more.
pub fn is_leftover(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    is_owned(name)
        && (lower == LEFTOVER_STRIP_RULE.to_ascii_lowercase() || lower.ends_with(LEFTOVER_SUFFIX))
}

/// How long PowerShell takes a name to be: UTF-16 units, as `.Length`.
fn name_length(name: &str) -> usize {
    name.encode_utf16().count()
}

/// "Cumulas Mail: <label Name>", or "Cumulas Mail: label <guid>" when that
/// would pass [`MAX_NAME`].
pub fn policy_name(label: &LabelView) -> String {
    let name = format!("{PREFIX}{}", label.name.trim());
    if name_length(&name) > MAX_NAME || label.name.trim().is_empty() {
        format!("{PREFIX}label {}", label.guid)
    } else {
        name
    }
}

/// "<policy name> (header)", or "Cumulas Mail: <guid> (header)" when that
/// would pass [`MAX_NAME`] (with "label " it would too, by one).
pub fn policy_rule_name(policy_name: &str, label: &LabelView) -> String {
    let name = format!("{policy_name} (header)");
    if name_length(&name) > MAX_NAME {
        format!("{PREFIX}{} (header)", label.guid)
    } else {
        name
    }
}

/// "(?i)^(<guid>|<immutableId>|<catalog id>)$": the distinct lowercase
/// values, in that order; only GUIDs go into the pattern.
pub fn label_pattern(label: &LabelView, catalog_id: &str) -> String {
    let mut ids: Vec<String> = Vec::new();
    let candidates = [
        Some(label.guid.as_str()),
        label.immutable_id.as_deref(),
        Some(catalog_id),
    ];
    for id in candidates.into_iter().flatten() {
        let id = id.trim().to_ascii_lowercase();
        if is_guid(&id) && !ids.contains(&id) {
            ids.push(id);
        }
    }
    format!("(?i)^({})$", ids.join("|"))
}

/// What a label's header value matched in Purview, for its row.
pub fn matched_by(label: &LabelView, catalog_id: &str) -> &'static str {
    if label.guid.eq_ignore_ascii_case(catalog_id) {
        "Guid"
    } else if label
        .immutable_id
        .as_deref()
        .is_some_and(|i| i.eq_ignore_ascii_case(catalog_id))
    {
        "ImmutableId"
    } else {
        "the ID Cumulas Mail sends"
    }
}

/// Whether `address` can stand in a command: one `@`, a local part of
/// visible characters, a domain of letters, digits, hyphens and dots.
pub fn is_address(address: &str) -> bool {
    let Some((local, domain)) = address.rsplit_once('@') else {
        return false;
    };
    let local_ok = !local.is_empty()
        && local.len() <= 64
        && !local.contains('@')
        && local.chars().all(|c| {
            c.is_ascii_graphic()
                && !matches!(
                    c,
                    '"' | '(' | ')' | ',' | ':' | ';' | '<' | '>' | '[' | '\\' | ']'
                )
        });
    let domain_ok = domain.contains('.')
        && domain.len() <= 253
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    local_ok && domain_ok
}

/// Checks an administrator's choices (or a script request's) and returns
/// them with label IDs lowercase. Label IDs must be GUIDs in the account's
/// label list or Purview's; templates, with a read to check them against,
/// ones Exchange lists, else plain names of at most 100 characters; the
/// account's address must be one.
pub fn validate(
    choices: &OrgSetupChoices,
    snapshot: Option<&Snapshot>,
    ctx: &SetupContext,
) -> Result<OrgSetupChoices> {
    if !is_address(&ctx.account_email) {
        return Err(Error::invalid_input(
            "The account's address can't be used in a command.",
        ));
    }
    let purview = snapshot.and_then(|s| match &s.compliance {
        Part::Read(compliance) => Some(&compliance.labels),
        _ => None,
    });
    let mut label_ids = Vec::new();
    for id in &choices.label_ids {
        let id = id.trim().to_ascii_lowercase();
        let in_catalog = ctx.catalog.as_ref().is_some_and(|c| c.label(&id).is_some());
        let in_purview = purview.is_some_and(|labels| {
            labels.iter().any(|l| {
                l.guid.eq_ignore_ascii_case(&id)
                    || l.immutable_id
                        .as_deref()
                        .is_some_and(|i| i.eq_ignore_ascii_case(&id))
            })
        });
        if !is_guid(&id) || !(in_catalog || in_purview) {
            return Err(Error::invalid_input("That label isn't one Purview lists."));
        }
        if !label_ids.contains(&id) {
            label_ids.push(id);
        }
    }
    let listed = snapshot.and_then(|s| match &s.templates {
        Part::Read(templates) => Some(templates),
        _ => None,
    });
    for template in [&choices.encrypt_template, &choices.do_not_forward_template]
        .into_iter()
        .flatten()
    {
        let fine = match listed {
            Some(listed) => listed
                .iter()
                .any(|t| t.name.trim().eq_ignore_ascii_case(template.trim())),
            None => {
                !template.trim().is_empty()
                    && template.chars().count() <= MAX_TEMPLATE
                    && !template.chars().any(char::is_control)
            }
        };
        if !fine {
            return Err(Error::invalid_input(
                "That template isn't one Exchange lists.",
            ));
        }
    }
    Ok(OrgSetupChoices {
        label_ids,
        encrypt_template: choices
            .encrypt_template
            .as_ref()
            .map(|t| t.trim().to_owned()),
        do_not_forward_template: choices
            .do_not_forward_template
            .as_ref()
            .map(|t| t.trim().to_owned()),
        ..choices.clone()
    })
}

/// Which of our mail flow rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OwnRule {
    /// Removes label requests from mail that isn't authenticated mail from
    /// inside the organization.
    Strip,
    Encrypt,
    DoNotForward,
}

/// A mail flow rule of ours as it should be (3.7.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WantedRule {
    pub which: OwnRule,
    pub name: &'static str,
    pub except_if_from_scope: &'static str,
    /// `HeaderMatchesMessageHeader` and its one pattern; none for the strip
    /// rule.
    pub header: Option<(&'static str, &'static str)>,
    /// The rights protection template's name; none for the strip rule.
    pub template: Option<String>,
    pub remove_header: &'static str,
    /// `RuleErrorAction`, when ours sets one.
    pub rule_error_action: Option<&'static str>,
}

impl WantedRule {
    pub fn strip() -> Self {
        Self {
            which: OwnRule::Strip,
            name: STRIP_RULE,
            except_if_from_scope: "InOrganization",
            header: None,
            template: None,
            remove_header: LABEL_HEADER,
            rule_error_action: None,
        }
    }

    pub fn encrypt(template: &str) -> Self {
        Self {
            which: OwnRule::Encrypt,
            name: ENCRYPT_RULE,
            except_if_from_scope: "NotInOrganization",
            header: Some((PROTECT_HEADER, ENCRYPT_PATTERN)),
            template: Some(template.to_owned()),
            remove_header: PROTECT_HEADER,
            rule_error_action: Some("Defer"),
        }
    }

    pub fn do_not_forward(template: &str) -> Self {
        Self {
            which: OwnRule::DoNotForward,
            name: DNF_RULE,
            header: Some((PROTECT_HEADER, DNF_PATTERN)),
            ..Self::encrypt(template)
        }
    }
}
