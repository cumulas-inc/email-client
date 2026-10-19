# Cumulas Mail: encryption and sensitivity labels for contoso.com (tenant 85f67998-e79a-4173-95c2-90b0e6dc231d).
# Made by Cumulas Mail 0.5.0 on 2026-10-19 14:03 UTC. Safe to run again: it creates what's missing,
# repairs what differs, and never changes or deletes anything whose name doesn't start with
# "Cumulas Mail:" (it may move other rules down a place, keeping their order).
#
# Part 1, mail flow rules: PowerShell 7.4 or later on Windows, macOS or Linux, or Windows
# PowerShell 5.1, as an Exchange Administrator or Global Administrator.
# Part 2, auto-labeling policies: Windows only (Microsoft doesn't offer Security & Compliance
# PowerShell on Linux or macOS), as a Compliance Administrator or Global Administrator, in the
# same run as Part 1: it checks that label requests from outside are blocked first. Without
# Windows, use the Microsoft Purview portal steps Cumulas Mail shows next to this script.
# Anyone in your organization can apply a label set up here from any mail app that can set the
# header, whoever the label is published to.
# Then allow 30 minutes or more for the rules; auto-labeling policies turn themselves on after
# their simulation (several hours) and can take up to a day more. Then select Check again in
# Cumulas Mail.

$ErrorActionPreference = 'Stop'
$Marker = 'Managed by Cumulas Mail (cumulas-mail:v1). Check or repair it from Cumulas Mail: Settings > Accounts > Sensitivity labels and encryption > Set up or check for your organization.'
# To limit new auto-labeling policies to a group's members, put its address here (a distribution
# group or mail-enabled security group, not a Microsoft 365 group). Empty: everyone in your
# organization, and existing policies keep their group.
$Group = ''

# ExchangeOnlineManagement 3.2.0 or later (REST); 3.10 and later need PowerShell 7.6.
if ($PSVersionTable.PSEdition -eq 'Core' -and $PSVersionTable.PSVersion -lt [version]'7.4') {
  throw 'Update PowerShell to 7.4 or later, or use Windows PowerShell 5.1.'
}
$MinModule = [version]'3.2.0'
$MaxModule = $null
if ($PSVersionTable.PSEdition -eq 'Core' -and $PSVersionTable.PSVersion -lt [version]'7.6') { $MaxModule = [version]'3.9.2' }
function Get-UsableModule {
  Get-Module -ListAvailable ExchangeOnlineManagement |
    Where-Object { $_.Version -ge $MinModule -and (-not $MaxModule -or $_.Version -le $MaxModule) } |
    Sort-Object Version -Descending | Select-Object -First 1
}
$module = Get-UsableModule
if (-not $module) {
  # Only from the PowerShell Gallery; PowerShell asks before installing from it.
  $install = @{ Name = 'ExchangeOnlineManagement'; Repository = 'PSGallery'; Scope = 'CurrentUser'; MinimumVersion = $MinModule }
  if ($MaxModule) { $install.MaximumVersion = $MaxModule }
  Install-Module @install
  $module = Get-UsableModule
  if (-not $module) { throw 'ExchangeOnlineManagement 3.2.0 or later is needed and could not be installed.' }
}
Import-Module $module.Path
"Using ExchangeOnlineManagement $($module.Version) from $($module.ModuleBase)"

function Test-Same($Now, $Want) {
  $a = (@($Now) | Where-Object { $null -ne $_ } | ForEach-Object { "$_".Trim() }) -join '|'
  $b = (@($Want) | Where-Object { $null -ne $_ } | ForEach-Object { "$_".Trim() }) -join '|'
  return $a -eq $b
}
function Test-Active($Rule) { "$($Rule.State)" -eq 'Enabled' -and "$($Rule.Mode)" -eq 'Enforce' }
# Rules that keep later rules from acting on the mail they match: they stop processing, or send
# the mail out through another service.
function Test-Blocks($Rule) { (Test-Active $Rule) -and ($Rule.StopRuleProcessing -or $Rule.RouteMessageOutboundConnector) }
function Get-Rules { @(Get-TransportRule -ResultSize Unlimited | Sort-Object Priority) }
function Get-OwnRule([string]$Name) { Get-Rules | Where-Object { $_.Name -eq $Name } | Select-Object -First 1 }
# Settings Cumulas Mail never sets: a rule of ours with any of them is left alone.
$ExtraParameters = @('ActivationDate', 'ADComparisonAttribute', 'AddManagerAsRecipientType', 'AddToRecipients', 'AnyOfCcHeader', 'AnyOfCcHeaderMemberOf', 'AnyOfRecipientAddressContainsWords', 'AnyOfRecipientAddressMatchesPatterns', 'AnyOfToCcHeader', 'AnyOfToCcHeaderMemberOf', 'AnyOfToHeader', 'AnyOfToHeaderMemberOf', 'ApplyClassification', 'ApplyHtmlDisclaimerText', 'ApplyOME', 'ApplyRightsProtectionCustomizationTemplate', 'AttachmentContainsWords', 'AttachmentExtensionMatchesWords', 'AttachmentHasExecutableContent', 'AttachmentIsPasswordProtected', 'AttachmentIsUnsupported', 'AttachmentMatchesPatterns', 'AttachmentNameMatchesPatterns', 'AttachmentProcessingLimitExceeded', 'AttachmentPropertyContainsWords', 'AttachmentSizeOver', 'BetweenMemberOf1', 'BetweenMemberOf2', 'BlindCopyTo', 'ContentCharacterSetContainsWords', 'CopyTo', 'DeleteMessage', 'Disconnect', 'DlpPolicy', 'ExceptIfADComparisonAttribute', 'ExceptIfAnyOfCcHeader', 'ExceptIfAnyOfCcHeaderMemberOf', 'ExceptIfAnyOfRecipientAddressContainsWords', 'ExceptIfAnyOfRecipientAddressMatchesPatterns', 'ExceptIfAnyOfToCcHeader', 'ExceptIfAnyOfToCcHeaderMemberOf', 'ExceptIfAnyOfToHeader', 'ExceptIfAnyOfToHeaderMemberOf', 'ExceptIfAttachmentContainsWords', 'ExceptIfAttachmentExtensionMatchesWords', 'ExceptIfAttachmentHasExecutableContent', 'ExceptIfAttachmentIsPasswordProtected', 'ExceptIfAttachmentIsUnsupported', 'ExceptIfAttachmentMatchesPatterns', 'ExceptIfAttachmentNameMatchesPatterns', 'ExceptIfAttachmentProcessingLimitExceeded', 'ExceptIfAttachmentPropertyContainsWords', 'ExceptIfAttachmentSizeOver', 'ExceptIfBetweenMemberOf1', 'ExceptIfBetweenMemberOf2', 'ExceptIfContentCharacterSetContainsWords', 'ExceptIfFrom', 'ExceptIfFromAddressContainsWords', 'ExceptIfFromAddressMatchesPatterns', 'ExceptIfFromMemberOf', 'ExceptIfHasClassification', 'ExceptIfHasNoClassification', 'ExceptIfHasSenderOverride', 'ExceptIfHeaderContainsMessageHeader', 'ExceptIfHeaderContainsWords', 'ExceptIfHeaderMatchesMessageHeader', 'ExceptIfHeaderMatchesPatterns', 'ExceptIfManagerAddresses', 'ExceptIfManagerForEvaluatedUser', 'ExceptIfMessageContainsDataClassifications', 'ExceptIfMessageSizeOver', 'ExceptIfMessageTypeMatches', 'ExceptIfRecipientADAttributeContainsWords', 'ExceptIfRecipientADAttributeMatchesPatterns', 'ExceptIfRecipientAddressContainsWords', 'ExceptIfRecipientAddressMatchesPatterns', 'ExceptIfRecipientDomainIs', 'ExceptIfRecipientInSenderList', 'ExceptIfSCLOver', 'ExceptIfSenderADAttributeContainsWords', 'ExceptIfSenderADAttributeMatchesPatterns', 'ExceptIfSenderDomainIs', 'ExceptIfSenderInRecipientList', 'ExceptIfSenderIpRanges', 'ExceptIfSenderManagementRelationship', 'ExceptIfSentTo', 'ExceptIfSentToMemberOf', 'ExceptIfSentToScope', 'ExceptIfSubjectContainsWords', 'ExceptIfSubjectMatchesPatterns', 'ExceptIfSubjectOrBodyContainsWords', 'ExceptIfSubjectOrBodyMatchesPatterns', 'ExceptIfWithImportance', 'ExpiryDate', 'From', 'FromAddressContainsWords', 'FromAddressMatchesPatterns', 'FromMemberOf', 'GenerateIncidentReport', 'GenerateNotification', 'HasClassification', 'HasNoClassification', 'HasSenderOverride', 'IncidentReportContent', 'LogEventText', 'ManagerAddresses', 'ManagerForEvaluatedUser', 'MessageContainsDataClassifications', 'MessageSizeOver', 'MessageTypeMatches', 'ModerateMessageByManager', 'ModerateMessageByUser', 'NotifySender', 'PrependSubject', 'Quarantine', 'RecipientADAttributeContainsWords', 'RecipientADAttributeMatchesPatterns', 'RecipientAddressContainsWords', 'RecipientAddressMatchesPatterns', 'RecipientDomainIs', 'RecipientInSenderList', 'RedirectMessageTo', 'RejectMessageEnhancedStatusCode', 'RejectMessageReasonText', 'RemoveOME', 'RemoveOMEv2', 'RemoveRMSAttachmentEncryption', 'RouteMessageOutboundConnector', 'RouteMessageOutboundRequireTls', 'SCLOver', 'SenderADAttributeContainsWords', 'SenderADAttributeMatchesPatterns', 'SenderDomainIs', 'SenderInRecipientList', 'SenderIpRanges', 'SenderManagementRelationship', 'SentTo', 'SentToMemberOf', 'SentToScope', 'SetHeaderName', 'SetHeaderValue', 'SetSCL', 'SmtpRejectMessageRejectStatusCode', 'SmtpRejectMessageRejectText', 'SubjectContainsWords', 'SubjectMatchesPatterns', 'SubjectOrBodyContainsWords', 'SubjectOrBodyMatchesPatterns', 'WithImportance')
function Get-Extras($Rule) {
  foreach ($p in $ExtraParameters) {
    $v = $Rule.$p
    if ($null -ne $v -and "$v" -ne '' -and "$v" -ne 'False' -and @($v).Count) { $p }
  }
}
# The strip rule never encrypts either.
function Get-StripExtras($Rule) {
  @(Get-Extras $Rule) + @(if ("$($Rule.ApplyRightsProtectionTemplate)") { 'ApplyRightsProtectionTemplate' })
}
$Strip = 'Cumulas Mail: ignore label requests from outside'
# Only routing rules for mail from inside the organization may come before the strip rule.
function Test-StripPlaced($Rule) {
  -not (Get-Rules | Where-Object {
    $_.Priority -lt $Rule.Priority -and (Test-Active $_) -and $_.StopRuleProcessing -and
    -not ($_.RouteMessageOutboundConnector -and "$($_.FromScope)" -eq 'InOrganization')
  })
}

# ---------------------------------------------------------------------------------------------
# Part 1: mail flow rules (Exchange Online)
# ---------------------------------------------------------------------------------------------
Connect-ExchangeOnline -ShowBanner:$false

# Label requests in mail that isn't authenticated mail from inside your organization are removed.
$rule = Get-OwnRule $Strip
$extra = if ($rule) { @(Get-StripExtras $rule) } else { @() }
if (-not $rule) {
  New-TransportRule -Name $Strip -ExceptIfFromScope InOrganization -RemoveHeader 'x-cumulas-label' `
    -Priority 0 -Mode Enforce -Comments $Marker | Out-Null
  "Created: $Strip"
} elseif ($extra.Count) {
  Write-Warning "$Strip has settings Cumulas Mail doesn't set ($($extra -join ', ')), so it was left alone. Remove them, then run this again."
} else {
  $fix = @{}
  if ("$($rule.FromScope)") { $fix.FromScope = $null }
  if (-not (Test-Same $rule.ExceptIfFromScope 'InOrganization')) { $fix.ExceptIfFromScope = 'InOrganization' }
  if (-not (Test-Same $rule.RemoveHeader 'x-cumulas-label')) { $fix.RemoveHeader = 'x-cumulas-label' }
  if ("$($rule.HeaderMatchesMessageHeader)") { $fix.HeaderMatchesMessageHeader = $null; $fix.HeaderMatchesPatterns = $null }
  if ("$($rule.HeaderContainsMessageHeader)") { $fix.HeaderContainsMessageHeader = $null; $fix.HeaderContainsWords = $null }
  if ($rule.StopRuleProcessing) { $fix.StopRuleProcessing = $false }
  if ("$($rule.Mode)" -ne 'Enforce') { $fix.Mode = 'Enforce' }
  if ("$($rule.SetAuditSeverity)" -eq 'DoNotAudit') { $fix.SetAuditSeverity = 'Low' }
  if (-not (Test-StripPlaced $rule)) { $fix.Priority = 0 }
  if ($fix.Count) { $fix.Comments = $Marker; Set-TransportRule -Identity $rule.Identity @fix; "Repaired: $Strip" }
  else { "Already set up: $Strip" }
  if ("$($rule.State)" -ne 'Enabled') { Enable-TransportRule -Identity $rule.Identity; "Turned on: $Strip" }
}

$templates = @(Get-RMSTemplate -ResultSize Unlimited)
function Get-TemplateName([string]$Want) {
  $t = $templates | Where-Object { $_.Name -eq $Want } | Select-Object -First 1
  if (-not $t) { throw "Exchange has no rights protection template named '$Want'. Get-RMSTemplate lists them; if it lists none, check Get-IRMConfiguration (AzureRMSLicensingEnabled and InternalLicensingEnabled must be True)." }
  return $t.Name
}
# Where Encrypt and Do Not Forward go: never below a rule that stops later rules or sends mail out
# through another service, except the Cumulas Signatures relay's rule when mail coming back from
# it counts as internal (then mail is signed before it's encrypted).
$RelayRule = 'Email signatures: add signature'
$Inbound = Get-InboundConnector | Where-Object { $_.Name -eq 'Email signatures (from signature server)' } | Select-Object -First 1
$RelayInternal = [bool]($Inbound -and $Inbound.TreatMessagesAsInternal)
function Move-ToPlace([string]$Name) {
  $rules = Get-Rules
  $own = $rules | Where-Object { $_.Name -eq $Name } | Select-Object -First 1
  $relay = $rules | Where-Object { $_.Name -eq $RelayRule -and (Test-Blocks $_) } | Select-Object -First 1
  $useRelay = [bool]($relay -and $RelayInternal)
  $above = @($rules | Where-Object { $_.Priority -lt $own.Priority -and (Test-Blocks $_) -and -not ($useRelay -and $_.Name -eq $RelayRule) })
  $relayBelow = $useRelay -and $relay.Priority -gt $own.Priority
  if (-not $above.Count -and -not $relayBelow) { return }
  if ($useRelay) {
    # Right after the relay's rule; Do Not Forward below Encrypt when that's there.
    $anchor = $relay
    $encrypt = $rules | Where-Object { $_.Name -eq 'Cumulas Mail: Encrypt' } | Select-Object -First 1
    if ($Name -ne 'Cumulas Mail: Encrypt' -and $encrypt -and $encrypt.Priority -eq $relay.Priority + 1) { $anchor = $encrypt }
    $to = if ($anchor.Priority -gt $own.Priority) { $anchor.Priority } else { $anchor.Priority + 1 }
    foreach ($r in @($above | Where-Object { $_.Priority -lt $relay.Priority })) {
      Write-Warning "'$($r.Name)' comes before the relay's rule and stops later rules for the mail it matches, so $Name doesn't act on that mail. Decide whether it should."
    }
    if ($relayBelow) {
      foreach ($r in @($rules | Where-Object { $_.Priority -gt $own.Priority -and $_.Priority -lt $relay.Priority -and (Test-Blocks $_) })) {
        Write-Warning "$Name moves below '$($r.Name)', which stops later rules or sends mail out for the mail it matches, so $Name won't act on that mail any more. Decide whether it should."
      }
    }
  } else {
    $to = $above[0].Priority
  }
  if ($to -ne $own.Priority) { Set-TransportRule -Identity $own.Identity -Priority $to; "Moved: $Name" }
}
foreach ($item in @(
  @{ Name = 'Cumulas Mail: Encrypt'; Pattern = '^encrypt$'; Template = (Get-TemplateName 'Encrypt') },
  @{ Name = 'Cumulas Mail: Do Not Forward'; Pattern = '^do-not-forward$'; Template = (Get-TemplateName 'Do Not Forward') }
)) {
  $rule = Get-OwnRule $item.Name
  if (-not $rule) {
    New-TransportRule -Name $item.Name -ExceptIfFromScope NotInOrganization `
      -HeaderMatchesMessageHeader 'x-cumulas-protect' -HeaderMatchesPatterns $item.Pattern `
      -ApplyRightsProtectionTemplate $item.Template -RemoveHeader 'x-cumulas-protect' `
      -RuleErrorAction Defer -Mode Enforce -Comments $Marker | Out-Null
    "Created: $($item.Name)"
  } else {
    $extra = @(Get-Extras $rule)
    if ($extra.Count) {
      Write-Warning "$($item.Name) has settings Cumulas Mail doesn't set ($($extra -join ', ')), so it was left alone. Remove them, then run this again."
      continue
    }
    $fix = @{}
    if ("$($rule.FromScope)") { $fix.FromScope = $null }
    if (-not (Test-Same $rule.ExceptIfFromScope 'NotInOrganization')) { $fix.ExceptIfFromScope = 'NotInOrganization' }
    if (-not (Test-Same $rule.HeaderMatchesMessageHeader 'x-cumulas-protect')) { $fix.HeaderMatchesMessageHeader = 'x-cumulas-protect' }
    if (-not (Test-Same $rule.HeaderMatchesPatterns $item.Pattern)) { $fix.HeaderMatchesPatterns = $item.Pattern }
    if ("$($rule.HeaderContainsMessageHeader)") { $fix.HeaderContainsMessageHeader = $null; $fix.HeaderContainsWords = $null }
    if (-not (Test-Same $rule.ApplyRightsProtectionTemplate $item.Template)) { $fix.ApplyRightsProtectionTemplate = $item.Template }
    if (-not (Test-Same $rule.RemoveHeader 'x-cumulas-protect')) { $fix.RemoveHeader = 'x-cumulas-protect' }
    if (-not (Test-Same $rule.RuleErrorAction 'Defer')) { $fix.RuleErrorAction = 'Defer' }
    if ($rule.StopRuleProcessing) { $fix.StopRuleProcessing = $false }
    if ("$($rule.Mode)" -ne 'Enforce') { $fix.Mode = 'Enforce' }
    if ("$($rule.SetAuditSeverity)" -eq 'DoNotAudit') { $fix.SetAuditSeverity = 'Low' }
    if ($fix.Count) { $fix.Comments = $Marker; Set-TransportRule -Identity $rule.Identity @fix; "Repaired: $($item.Name)" }
    else { "Already set up: $($item.Name)" }
    if ("$($rule.State)" -ne 'Enabled') { Enable-TransportRule -Identity $rule.Identity; "Turned on: $($item.Name)" }
  }
  Move-ToPlace $item.Name
}

# ---------------------------------------------------------------------------------------------
# Part 2: auto-labeling policies (Security & Compliance; Windows only)
# ---------------------------------------------------------------------------------------------
# Auto-labeling acts on mail by sender address, so label requests from outside must be removed
# first. Part 2 checks Part 1's rule itself and stops without it.
$rule = Get-OwnRule $Strip
if (-not $rule -or -not (Test-Active $rule) -or @(Get-StripExtras $rule).Count -or -not (Test-StripPlaced $rule)) {
  throw "Label requests from outside aren't blocked yet ($Strip). Run Part 1 as an Exchange Administrator or Global Administrator, then run this again."
}
Disconnect-ExchangeOnline -Confirm:$false
if ($PSVersionTable.PSEdition -eq 'Core' -and -not $IsWindows) {
  throw 'Part 1 is done. Part 2 needs Windows: Microsoft doesn''t offer Security & Compliance PowerShell on Linux or macOS. Run this on Windows, or follow the Microsoft Purview portal steps Cumulas Mail shows.'
}
Connect-IPPSSession

$labels = @(Get-Label)
$policies = @(Get-AutoSensitivityLabelPolicy)
$policyRules = @(Get-AutoSensitivityLabelRule)
# Conditions and locations Cumulas Mail never sets: a policy of ours with any of them is left alone.
$AutoRuleConditions = @('AccessScope', 'ActivationDate', 'AnyOfRecipientAddressContainsWords', 'AnyOfRecipientAddressMatchesPatterns', 'ContentContainsSensitiveInformation', 'ContentExtensionMatchesWords', 'ContentPropertyContainsWords', 'DocumentCreatedBy', 'DocumentIsPasswordProtected', 'DocumentIsUnsupported', 'DocumentNameMatchesWords', 'DocumentSizeOver', 'ExceptIfAccessScope', 'ExceptIfAnyOfRecipientAddressContainsWords', 'ExceptIfAnyOfRecipientAddressMatchesPatterns', 'ExceptIfContentContainsSensitiveInformation', 'ExceptIfContentExtensionMatchesWords', 'ExceptIfContentPropertyContainsWords', 'ExceptIfDocumentCreatedBy', 'ExceptIfDocumentIsPasswordProtected', 'ExceptIfDocumentIsUnsupported', 'ExceptIfDocumentNameMatchesWords', 'ExceptIfDocumentSizeOver', 'ExceptIfFrom', 'ExceptIfFromAddressContainsWords', 'ExceptIfFromAddressMatchesPatterns', 'ExceptIfFromMemberOf', 'ExceptIfHeaderMatchesPatterns', 'ExceptIfProcessingLimitExceeded', 'ExceptIfRecipientDomainIs', 'ExceptIfSenderDomainIs', 'ExceptIfSenderIPRanges', 'ExceptIfSentTo', 'ExceptIfSentToMemberOf', 'ExceptIfSubjectMatchesPatterns', 'ExpiryDate', 'From', 'FromAddressContainsWords', 'FromAddressMatchesPatterns', 'FromMemberOf', 'ProcessingLimitExceeded', 'RecipientDomainIs', 'SenderDomainIs', 'SenderIPRanges', 'SentTo', 'SentToMemberOf', 'SubjectMatchesPatterns')
# Conditions that always carry a value, and the one that means they aren't used.
$AutoRuleDefaults = @{ AccessScope = 'None'; ExceptIfAccessScope = 'None' }
$OtherLocations = @('SharePointLocation', 'OneDriveLocation', 'TeamsLocation', 'SharePointAdaptiveScopes', 'OneDriveAdaptiveScopes')
function Get-AutoExtras($Rule) {
  foreach ($p in $AutoRuleConditions) {
    $v = $Rule.$p
    if ($AutoRuleDefaults.ContainsKey($p) -and "$v".Trim() -in $AutoRuleDefaults[$p], '0') { continue }
    if ($null -ne $v -and "$v" -ne '' -and "$v" -ne 'False' -and @($v).Count) { $p }
  }
  if ("$($Rule.Workload)" -and "$($Rule.Workload)" -ne 'Exchange') { 'Workload' }
  if (@($Rule.HeaderMatchesPatterns.Keys).Count -gt 1) { 'HeaderMatchesPatterns' }
}

foreach ($want in @(
  @{ Id = '6c1b0f3e-1111-4a8b-9c2d-000000000001'; Title = 'Confidential' },
  @{ Id = '6c1b0f3e-1111-4a8b-9c2d-0000000000bb'; Title = 'O''Brien’’s $(whoami) `n; x' }
)) {
  $label = $labels | Where-Object { "$($_.Guid)" -eq $want.Id -or "$($_.ImmutableId)" -eq $want.Id } | Select-Object -First 1
  if (-not $label) { Write-Warning "Purview lists no label with the ID $($want.Id) ($($want.Title)); skipped."; continue }
  $guid = "$($label.Guid)".ToLowerInvariant()
  $ids = @("$($label.Guid)", "$($label.ImmutableId)", $want.Id) | Where-Object { $_ } |
    ForEach-Object { $_.ToLowerInvariant() } | Where-Object { $_ -match '^[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}$' } |
    Select-Object -Unique
  $pattern = '(?i)^(' + ($ids -join '|') + ')$'
  $name = 'Cumulas Mail: ' + "$($label.Name)".Trim()
  if ($name.Length -gt 64 -or -not "$($label.Name)".Trim()) { $name = 'Cumulas Mail: label ' + $guid }
  $ruleName = "$name (header)"
  if ($ruleName.Length -gt 64) { $ruleName = 'Cumulas Mail: ' + $guid + ' (header)' }

  $policy = $policies | Where-Object { $_.Name -eq $name } | Select-Object -First 1
  if ($policy -and "$($policy.Mode)" -eq 'PendingDeletion') { Write-Warning "Microsoft 365 is still deleting $name. Run this again later."; continue }
  if (-not $policy) {
    $new = @{ Name = $name; ApplySensitivityLabel = $guid; ExchangeLocation = 'All'; OverwriteLabel = $true; Mode = 'TestWithoutNotifications'; Comment = $Marker }
    if ($Group) { $new.ExchangeSenderMemberOf = $Group }
    New-AutoSensitivityLabelPolicy @new | Out-Null
    New-AutoSensitivityLabelRule -Policy $name -Name $ruleName -Workload Exchange `
      -HeaderMatchesPatterns @{ 'x-cumulas-label' = $pattern } | Out-Null
    Set-AutoSensitivityLabelPolicy -Identity $name -StartSimulation $true -AutoEnableAfter (New-TimeSpan -Hours 1)
    "Created in simulation; it turns on by itself an hour after its simulation completes: $name"
    continue
  }

  # Only Cumulas Mail's own rule, with only its header pattern, and only email: anything else is
  # left alone.
  $own = @($policyRules | Where-Object { "$($_.ParentPolicyName)" -eq $name -or "$($_.Policy)" -eq $name })
  $elsewhere = @(foreach ($p in $OtherLocations) { @($policy.$p) | Where-Object { "$_" } })
  $rule = $own | Where-Object { $_.Name -eq $ruleName } | Select-Object -First 1
  $extra = @(if ($rule) { Get-AutoExtras $rule })
  if (@($own | Where-Object { $_.Name -ne $ruleName }).Count -or $elsewhere.Count -or $extra.Count) {
    Write-Warning "$name has rules, conditions or locations Cumulas Mail doesn't set$(if ($extra.Count) { " ($($extra -join ', '))" }), so it was left alone. Remove them in the Microsoft Purview portal, then run this again."
    continue
  }
  $changed = $false
  $fix = @{}
  $applied = $policy.ApplySensitivityLabel
  if (-not ((Test-Same $applied $guid) -or (Test-Same $applied $label.Name) -or ("$($label.ImmutableId)" -and (Test-Same $applied "$($label.ImmutableId)")))) { $fix.ApplySensitivityLabel = $guid }
  if (-not (@($policy.ExchangeLocation) | Where-Object { "$_" -eq 'All' })) { $fix.AddExchangeLocation = 'All' }
  if ($Group -and -not (Test-Same (@($policy.ExchangeSenderMemberOf) | ForEach-Object { "$_".ToLowerInvariant() }) $Group.ToLowerInvariant())) { $fix.ExchangeSenderMemberOf = $Group }
  if (-not $policy.OverwriteLabel) { $fix.OverwriteLabel = $true }
  if ($fix.Count) { $changed = $true }
  if ($policy.Enabled -eq $false) { $fix.Enabled = $true }
  if ($fix.Count) { $fix.Comment = $Marker; Set-AutoSensitivityLabelPolicy -Identity $policy.Identity @fix; "Repaired: $name" }

  if (-not $rule) {
    New-AutoSensitivityLabelRule -Policy $name -Name $ruleName -Workload Exchange `
      -HeaderMatchesPatterns @{ 'x-cumulas-label' = $pattern } | Out-Null
    $changed = $true
    "Added its rule: $ruleName"
  } else {
    # Right: the pattern Cumulas Mail sets, or one for exactly the ID people's Cumulas Mail sends.
    $text = ($rule.HeaderMatchesPatterns | Out-String -Width 4096).ToLowerInvariant()
    $right = $text.Contains('x-cumulas-label') -and -not $text.Contains('x-cumulas-label-stamped') -and
      ($text.Contains($pattern) -or $text.Contains('^' + $want.Id + '$'))
    $fixRule = @{}
    if (-not $right) { $fixRule.HeaderMatchesPatterns = @{ 'x-cumulas-label' = $pattern } }
    if ($rule.Disabled) { $fixRule.Disabled = $false }
    if ($fixRule.Count) {
      Set-AutoSensitivityLabelRule -Identity $rule.Identity @fixRule
      $changed = $true
      "Repaired its rule: $ruleName"
    }
  }

  # A simulation that runs and turns the policy on by itself is left to finish: starting it again
  # would start it over.
  $policy = Get-AutoSensitivityLabelPolicy | Where-Object { $_.Name -eq $name } | Select-Object -First 1
  $after = "$($policy.AutoEnableAfter)"
  if ("$($policy.Mode)" -eq 'Enable') { "On: $name" }
  elseif (-not $changed -and "$($policy.Mode)" -like 'TestWith*' -and $after -and $after -notin '00:00:00', '0') {
    "In simulation; it turns on by itself an hour after the simulation completes: $name"
  } else {
    # Turned off in the portal: back into simulation first.
    if ("$($policy.Mode)" -eq 'Disable') { Set-AutoSensitivityLabelPolicy -Identity $policy.Identity -Mode TestWithoutNotifications }
    Set-AutoSensitivityLabelPolicy -Identity $policy.Identity -StartSimulation $true -AutoEnableAfter (New-TimeSpan -Hours 1)
    "In simulation; it turns on by itself an hour after the simulation completes: $name"
  }
}
Disconnect-ExchangeOnline -Confirm:$false
