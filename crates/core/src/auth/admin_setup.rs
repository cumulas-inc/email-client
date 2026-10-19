//! The organization setup app: a separate Microsoft Entra app registration
//! an administrator signs in with to read, create and repair the
//! organization's mail flow rules and auto-labeling policies
//! (`crate::orgsetup`).
//!
//! It is never the mail app's registration, and the mail app never asks
//! for what it asks for: its Exchange.Manage permission would let anyone
//! holding a token change the organization's mail flow. Nothing here builds
//! a link or a scope with the mail app's helpers (a test reads this file to
//! make sure), and the mail app's scopes, refresh scopes and approval links
//! never name a [`SETUP_ONLY_MARKERS`] string (tests in `microsoft.rs`).

use std::sync::LazyLock;

use crate::error::{Error, ErrorCode, Result};
use crate::service::{PUBLISHED_MICROSOFT_CLIENT_ID, builtin_microsoft_client_id};
use crate::signatures::config::{NO_CLIENT_ID, is_guid};

use super::microsoft::DEFAULT_AUTHORITY_HOST;

/// What people see: the registration's display name, and every mention of
/// the setup app in Cumulas Mail. `SETUP_APP_NAME` in
/// `src/modules/settings/orgsetup/copy.ts` must match.
pub const SETUP_APP_NAME: &str = "Cumulas Mail - Admin Setup";
/// "Cumulas Mail - Admin Setup": Cumulas Inc's multi-tenant public client
/// (organizations only, http://localhost, publisher-verified) that an
/// administrator signs in with to set up and repair the organization's
/// rules and auto-labeling policies. Never the mail app's registration.
/// Lowercase GUID. Released only once the registration is finished (owner
/// steps A, README "Before release").
pub const PUBLISHED_SETUP_CLIENT_ID: Option<&str> = Some("b4f17c55-b9f6-4a84-958f-afe3842da327");
/// The build variable naming another setup registration, or `none` for
/// none (then only the PowerShell script sets things up).
pub const SETUP_CLIENT_ID_VAR: &str = "CUMULAS_MS_SETUP_CLIENT_ID";
const BUILD_SETUP_CLIENT_ID: Option<&str> = option_env!("CUMULAS_MS_SETUP_CLIENT_ID");

/// Exchange Online, the resource setup's mail flow rule calls go to.
pub const EXCHANGE_RESOURCE: &str = "https://outlook.office365.com";
pub const EXCHANGE_MANAGE_SCOPE: &str = "https://outlook.office365.com/Exchange.Manage";
/// Where the regional Security & Compliance host is found.
pub const COMPLIANCE_DISCOVERY_HOST: &str = "https://ps.compliance.protection.outlook.com";
/// Tried in order for the Security & Compliance token.
pub const COMPLIANCE_SCOPES: [&str; 2] = [
    "https://ps.compliance.protection.outlook.com/.default",
    "https://ps.compliance.protection.outlook.com/Exchange.Manage",
];
/// What the setup sign-in asks consent for, in this order. String literals,
/// so check-admin-consent.sh can read them; a test asserts the last equals
/// [`EXCHANGE_MANAGE_SCOPE`].
pub const SETUP_CONSENT_SCOPES: &[&str] = &[
    "openid",
    "profile",
    "offline_access",
    "https://outlook.office365.com/Exchange.Manage",
];
/// Where the approval link and the sign-in return: the registration's only
/// redirect URI. Nothing listens there once an administrator has approved,
/// so their browser then shows a page that can't connect; the approval is
/// recorded by then.
pub const SETUP_REDIRECT_URI: &str = "http://localhost";
/// Strings that never appear in the mail app's scopes, refresh scopes,
/// approval links or the registration's documented permission list.
pub const SETUP_ONLY_MARKERS: &[&str] = &[
    "outlook.office365.com",
    "Exchange.Manage",
    "ps.compliance.protection.outlook.com",
    "00000002-0000-0ff1-ce00-000000000000",
    "00000007-0000-0ff1-ce00-000000000000",
];

/// The setup client ID this build uses: the build variable (a GUID, or
/// `none`), else [`PUBLISHED_SETUP_CLIENT_ID`]. A value equal to the mail
/// app's client ID (published, built in, or `mail_client_id`, the one
/// entered in Settings) is refused, with a warning: the two registrations
/// must never be one.
pub fn setup_client_id(mail_client_id: Option<&str>) -> Option<String> {
    // Worked out once, so that a bad value is warned about once.
    static BUILT: LazyLock<Option<String>> =
        LazyLock::new(|| resolve_setup_client_id(BUILD_SETUP_CLIENT_ID));
    apart_from_mail(BUILT.as_deref()?, mail_client_id)
}

/// The build variable's setup client ID, else the published one; `None`
/// when the build said `none`. A value that isn't a GUID is ignored, with a
/// warning.
fn resolve_setup_client_id(built: Option<&str>) -> Option<String> {
    let Some(value) = built.map(str::trim).filter(|v| !v.is_empty()) else {
        return PUBLISHED_SETUP_CLIENT_ID.map(str::to_owned);
    };
    if value.eq_ignore_ascii_case(NO_CLIENT_ID) {
        return None;
    }
    if is_guid(value) {
        return Some(value.to_ascii_lowercase());
    }
    tracing::warn!(
        "ignoring the setup client ID from the build ({SETUP_CLIENT_ID_VAR}): it must be a GUID \
         (or `{NO_CLIENT_ID}`)"
    );
    PUBLISHED_SETUP_CLIENT_ID.map(str::to_owned)
}

/// `setup`, unless it is one of the mail app's client IDs.
fn apart_from_mail(setup: &str, mail_client_id: Option<&str>) -> Option<String> {
    let mail = [
        Some(PUBLISHED_MICROSOFT_CLIENT_ID),
        builtin_microsoft_client_id(),
        mail_client_id,
    ];
    if mail
        .into_iter()
        .flatten()
        .any(|id| id.trim().eq_ignore_ascii_case(setup))
    {
        tracing::warn!(
            "not using the setup client ID {setup}: it is the mail app's own, and the two \
             registrations must never be one"
        );
        return None;
    }
    Some(setup.to_owned())
}

/// The setup app's approval link for `tenant` (a tenant ID; `organizations`
/// without one):
/// `https://login.microsoftonline.com/<tenant>/v2.0/adminconsent?client_id=…&scope=…&redirect_uri=http://localhost`,
/// asking for [`SETUP_CONSENT_SCOPES`], the OpenID Connect ones unqualified.
pub fn setup_admin_consent_url(client_id: &str, tenant: Option<&str>) -> String {
    let plausible = |t: &str| {
        !t.is_empty()
            && t.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    };
    let tenant = tenant
        .map(str::trim)
        .filter(|t| plausible(t))
        .unwrap_or("organizations");
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", client_id)
        .append_pair("scope", &SETUP_CONSENT_SCOPES.join(" "))
        .append_pair("redirect_uri", SETUP_REDIRECT_URI)
        .finish();
    format!("{DEFAULT_AUTHORITY_HOST}/{tenant}/v2.0/adminconsent?{query}")
}

/// Why setup can't sign in from this build (yet): the PowerShell script
/// does the same.
pub(crate) fn setup_not_available() -> Error {
    Error::new(
        ErrorCode::NotConfigured,
        "Automatic setup isn't available in this version of Cumulas Mail. Use the PowerShell \
         script instead.",
    )
}

/// An administrator's setup sign-in.
pub struct AdminSignIn {
    /// The ID token's preferred_username (else email): the UPN anchor.
    pub username: String,
    /// Its name, else the username.
    pub display_name: String,
    /// Its tid, checked against the work account's organization.
    pub tenant_id: String,
    /// Its oid: for the command that removes this administrator's approval.
    pub object_id: String,
    pub tokens: AdminTokens,
    /// The redemption's `scope`, for the run log.
    pub granted: String,
}

/// What a setup token is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Resource {
    /// Exchange Online: mail flow rules.
    Exchange,
    /// Security & Compliance: labels and auto-labeling policies.
    Compliance,
}

/// The setup sign-in's tokens, in memory only: never written to the
/// keyring, the database or the log. Not built yet: every token request
/// fails with `NotConfigured`.
pub struct AdminTokens {
    _private: (),
}

impl AdminTokens {
    /// An access token for `resource`.
    pub async fn token(&self, _resource: Resource) -> Result<String> {
        Err(setup_not_available())
    }

    /// Forgets the cached access token after a server rejected it.
    pub async fn invalidate(&self, _resource: Resource) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The approval link the IT page publishes for the setup app.
    const PUBLISHED_SETUP_LINK: &str = "https://login.microsoftonline.com/organizations/v2.0/adminconsent?client_id=b4f17c55-b9f6-4a84-958f-afe3842da327&scope=openid+profile+offline_access+https%3A%2F%2Foutlook.office365.com%2FExchange.Manage&redirect_uri=http%3A%2F%2Flocalhost";
    const ID: &str = "99999999-8888-7777-6666-555555555555";

    #[test]
    fn the_setup_app_is_named_and_registered_as_the_owner_made_it() {
        assert_eq!(SETUP_APP_NAME, "Cumulas Mail - Admin Setup");
        let id = PUBLISHED_SETUP_CLIENT_ID.unwrap();
        assert_eq!(id, "b4f17c55-b9f6-4a84-958f-afe3842da327");
        assert!(is_guid(id) && id == id.to_ascii_lowercase());
        assert_ne!(id, PUBLISHED_MICROSOFT_CLIENT_ID);
        assert_eq!(
            SETUP_CONSENT_SCOPES.last().copied(),
            Some(EXCHANGE_MANAGE_SCOPE)
        );
        assert_eq!(SETUP_CONSENT_SCOPES[3], EXCHANGE_MANAGE_SCOPE);
        assert_eq!(
            EXCHANGE_MANAGE_SCOPE,
            format!("{EXCHANGE_RESOURCE}/Exchange.Manage")
        );
        assert!(
            COMPLIANCE_SCOPES
                .iter()
                .all(|s| s.starts_with(COMPLIANCE_DISCOVERY_HOST))
        );
    }

    #[test]
    fn setup_client_id_is_never_the_mail_client_id() {
        let setup = PUBLISHED_SETUP_CLIENT_ID.unwrap();
        // Not set, or set empty: the published one.
        for unset in [None, Some(""), Some("  ")] {
            assert_eq!(
                resolve_setup_client_id(unset).as_deref(),
                Some(setup),
                "{unset:?}"
            );
        }
        assert_eq!(
            resolve_setup_client_id(Some(&format!(" {} ", ID.to_uppercase()))).as_deref(),
            Some(ID)
        );
        for none in ["none", " NONE "] {
            assert_eq!(resolve_setup_client_id(Some(none)), None, "{none}");
        }
        assert_eq!(
            resolve_setup_client_id(Some("not-a-guid")).as_deref(),
            Some(setup)
        );

        // The published setup app, next to the published mail app or an
        // organization's own: kept.
        assert_eq!(
            apart_from_mail(setup, Some(PUBLISHED_MICROSOFT_CLIENT_ID)).as_deref(),
            Some(setup)
        );
        assert_eq!(apart_from_mail(setup, Some(ID)).as_deref(), Some(setup));
        // The setup ID entered in Settings as the mail app's: refused.
        assert_eq!(apart_from_mail(setup, Some(setup)), None);
        assert_eq!(
            apart_from_mail(setup, Some(&format!(" {} ", setup.to_uppercase()))),
            None
        );
        // A build naming the published mail app as the setup app: refused,
        // whatever Settings says.
        let built = resolve_setup_client_id(Some(PUBLISHED_MICROSOFT_CLIENT_ID)).unwrap();
        assert_eq!(apart_from_mail(&built, None), None);
        assert_eq!(apart_from_mail(&built, Some(ID)), None);
        // A build value equal to the mail app's own client ID: refused.
        let built = resolve_setup_client_id(Some(ID)).unwrap();
        assert_eq!(apart_from_mail(&built, Some(ID)), None);

        // What this build does.
        if BUILD_SETUP_CLIENT_ID.is_none_or(|v| v.trim().is_empty()) {
            assert_eq!(
                setup_client_id(Some(PUBLISHED_MICROSOFT_CLIENT_ID)).as_deref(),
                Some(setup)
            );
            assert_eq!(setup_client_id(Some(setup)), None);
        }
    }

    #[test]
    fn setup_admin_consent_url_for_the_published_registration() {
        let setup = PUBLISHED_SETUP_CLIENT_ID.unwrap();
        assert_eq!(setup_admin_consent_url(setup, None), PUBLISHED_SETUP_LINK);
        // Not a tenant: organizations.
        for junk in [Some(""), Some(" "), Some("contoso.com/../x"), Some("a b")] {
            assert_eq!(
                setup_admin_consent_url(setup, junk),
                PUBLISHED_SETUP_LINK,
                "{junk:?}"
            );
        }
    }

    #[test]
    fn setup_admin_consent_url_names_the_tenant_and_the_scopes_in_order() {
        let tenant = "85f67998-e79a-4173-95c2-90b0e6dc231d";
        let url = setup_admin_consent_url(ID, Some(tenant));
        assert!(
            url.starts_with(&format!(
                "https://login.microsoftonline.com/{tenant}/v2.0/adminconsent?client_id={ID}&"
            )),
            "{url}"
        );
        assert!(
            url.ends_with("&redirect_uri=http%3A%2F%2Flocalhost"),
            "{url}"
        );
        let parsed = url::Url::parse(&url).unwrap();
        let scope = parsed
            .query_pairs()
            .find(|(k, _)| k == "scope")
            .map(|(_, v)| v.into_owned())
            .unwrap();
        assert_eq!(
            scope,
            "openid profile offline_access https://outlook.office365.com/Exchange.Manage"
        );
        assert_eq!(scope.split(' ').collect::<Vec<_>>(), SETUP_CONSENT_SCOPES);
        // Never the mail app's: no Graph scope, no push service.
        assert!(!url.contains("graph.microsoft.com"));
        assert!(!url.contains("Sync.Receive"));
        assert!(!url.contains("email"));
    }

    /// Whether `source` calls the function `name` (a whole word, not part
    /// of a longer name such as `setup_admin_consent_url`).
    fn calls(source: &str, name: &str) -> bool {
        source.match_indices(&format!("{name}(")).any(|(at, _)| {
            source[..at]
                .chars()
                .next_back()
                .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'))
        })
    }

    #[test]
    fn never_builds_a_link_with_the_mail_apps_helpers() {
        let source = include_str!("admin_setup.rs");
        let (code, _tests) = source.split_once("#[cfg(test)]").unwrap();
        for helper in ["admin_consent_link", "admin_consent_url", "sign_in_error"] {
            assert!(!calls(code, helper), "admin_setup.rs calls {helper}");
        }
        // The check itself finds calls, and only whole names.
        assert!(calls(
            "x = admin_consent_url(&c, None);",
            "admin_consent_url"
        ));
        assert!(calls("microsoft::sign_in_error(c, e)", "sign_in_error"));
        assert!(!calls(
            "setup_admin_consent_url(id, None)",
            "admin_consent_url"
        ));
        assert!(!calls("admin_sign_in_error(c, e)", "sign_in_error"));
    }
}
