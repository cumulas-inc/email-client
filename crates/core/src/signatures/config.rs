//! Where the signature service is, and which app registration its API
//! belongs to: the environment at run time, else the same variables at
//! build time, else Cumulas Signatures and its published shared app. A
//! client ID of `none` turns company signatures off.

use url::Url;

/// Where Cumulas Signatures runs, unless the build or the environment says
/// otherwise (see [`signature_url`]).
pub const DEFAULT_SIGNATURE_URL: &str = "https://signature.cumulas.co";
/// The variable that points a build (at build time) or a run (at run time)
/// at another signature service.
pub const SIGNATURE_URL_VAR: &str = "CUMULAS_SIGNATURE_URL";
/// The client ID of Cumulas Signatures' shared Microsoft 365 app (its
/// Outlook add-in's), which exposes
/// `api://signature.cumulas.co/<client ID>/access_as_user` and has Cumulas
/// Mail pre-authorized. It belongs to [`DEFAULT_SIGNATURE_URL`] only: another
/// service has a shared app of its own (see [`signature_client_id`]).
pub const PUBLISHED_SIGNATURE_CLIENT_ID: &str = "da27f705-3818-4ca0-9b5d-7781405ea54e";
/// The variable with the client ID of the signature service's shared app
/// registration (its Outlook add-in's), whose API the app asks tokens for.
pub const SIGNATURE_CLIENT_ID_VAR: &str = "CUMULAS_SIGNATURE_CLIENT_ID";
/// The value of `CUMULAS_SIGNATURE_CLIENT_ID` that turns company signatures
/// off, in any case. An empty value means "not set" instead, so that a
/// packaging file can list the variable without changing anything.
pub const NO_CLIENT_ID: &str = "none";
const BUILT_IN_URL: Option<&str> = option_env!("CUMULAS_SIGNATURE_URL");
const BUILT_IN_CLIENT_ID: Option<&str> = option_env!("CUMULAS_SIGNATURE_CLIENT_ID");
/// The delegated scope the service's API exposes.
const API_SCOPE: &str = "access_as_user";

/// The signature service as this run uses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureService {
    /// Without a trailing slash, such as `https://signature.cumulas.co`.
    pub url: String,
    pub client_id: String,
    /// The API's delegated scope, made here from `url` and `client_id`.
    pub scope: String,
}

impl SignatureService {
    /// The service configured for this run; `None` when company signatures
    /// are off (see [`signature_client_id`]).
    pub fn configured() -> Option<Self> {
        let url = signature_url();
        let client_id = signature_client_id(&url)?;
        Self::new(url, client_id)
    }

    fn new(url: String, client_id: String) -> Option<Self> {
        let scope = signature_scope(&url, &client_id)?;
        Some(Self {
            url,
            client_id,
            scope,
        })
    }

    /// The host (with its port, when not the default), for the UI.
    pub fn host(&self) -> String {
        Url::parse(&self.url)
            .ok()
            .and_then(|url| host_and_port(&url))
            .unwrap_or_default()
    }
}

/// The signature service's address: the `CUMULAS_SIGNATURE_URL` environment
/// variable, else the address built in with the same variable, else
/// [`DEFAULT_SIGNATURE_URL`]. Tokens go there, so an address that isn't
/// HTTPS (other than one on this computer), or that carries a user name, a
/// query or a fragment, is ignored.
pub fn signature_url() -> String {
    let runtime = std::env::var(SIGNATURE_URL_VAR).ok();
    resolve_signature_url(runtime.as_deref(), BUILT_IN_URL)
}

/// The client ID of the app registration of the signature service at
/// `url` (see [`signature_url`]): the `CUMULAS_SIGNATURE_CLIENT_ID`
/// environment variable, else the one built in with the same variable, else
/// [`PUBLISHED_SIGNATURE_CLIENT_ID`] when `url` is Cumulas Signatures'.
/// `None`, and company signatures off, when the first of the two that is
/// set says `none`, or when neither names a client ID and `url` is another
/// service's: the published ID would make a scope that service's shared app
/// doesn't expose. A value that isn't a GUID is ignored, with a warning.
pub fn signature_client_id(url: &str) -> Option<String> {
    let runtime = std::env::var(SIGNATURE_CLIENT_ID_VAR).ok();
    resolve_client_id(runtime.as_deref(), BUILT_IN_CLIENT_ID, url)
}

/// The API's delegated scope:
/// `api://<host, with a port that isn't the default>/<client id>/access_as_user`.
/// Made here, never taken from a server's answer, so that no address can
/// make the app ask for a token to anything else (Microsoft Graph, say).
pub fn signature_scope(url: &str, client_id: &str) -> Option<String> {
    let host = host_and_port(&Url::parse(url).ok()?)?;
    is_guid(client_id).then(|| format!("api://{host}/{client_id}/{API_SCOPE}"))
}

/// The signature service's host, for the UI (`AppInfo`); `None` when
/// company signatures are off.
pub fn signature_host() -> Option<String> {
    SignatureService::configured().map(|service| service.host())
}

/// Whether `text` is a GUID in its usual form
/// (`xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`).
pub fn is_guid(text: &str) -> bool {
    text.len() == 36
        && text.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

fn host_and_port(url: &Url) -> Option<String> {
    let host = url.host_str()?;
    // `port` is `None` for the scheme's default.
    Some(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    })
}

fn resolve_signature_url(runtime: Option<&str>, built_in: Option<&str>) -> String {
    for (candidate, origin) in [(runtime, "the environment"), (built_in, "the build")] {
        let Some(candidate) = candidate.map(str::trim).filter(|c| !c.is_empty()) else {
            continue;
        };
        match usable_signature_url(candidate) {
            Some(url) => return url,
            None => tracing::warn!(
                "ignoring the signature service address from {origin}: it must start with \
                 https:// (or http:// for this computer), without a user name, query or fragment"
            ),
        }
    }
    DEFAULT_SIGNATURE_URL.to_owned()
}

fn usable_signature_url(text: &str) -> Option<String> {
    let url = Url::parse(text).ok()?;
    let host = url.host_str()?;
    let loopback = matches!(host, "localhost" | "127.0.0.1" | "[::1]");
    let secure = url.scheme() == "https" || (url.scheme() == "http" && loopback);
    let plain = url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none();
    (secure && plain).then(|| text.trim_end_matches('/').to_owned())
}

fn resolve_client_id(runtime: Option<&str>, built_in: Option<&str>, url: &str) -> Option<String> {
    for (candidate, origin) in [(runtime, "the environment"), (built_in, "the build")] {
        let Some(candidate) = candidate.map(str::trim).filter(|c| !c.is_empty()) else {
            continue;
        };
        if candidate.eq_ignore_ascii_case(NO_CLIENT_ID) {
            return None;
        }
        if is_guid(candidate) {
            return Some(candidate.to_ascii_lowercase());
        }
        tracing::warn!(
            "ignoring the signature service's client ID from {origin}: it must be a GUID \
             (or `{NO_CLIENT_ID}`)"
        );
    }
    if is_published_service(url) {
        return Some(PUBLISHED_SIGNATURE_CLIENT_ID.to_owned());
    }
    tracing::warn!(
        "company signatures are off: the signature service at {url} needs its shared app's \
         client ID in {SIGNATURE_CLIENT_ID_VAR}"
    );
    None
}

/// Whether `url` is Cumulas Signatures' own address (its scheme, host and
/// port; the path doesn't change the scope), the one service whose shared
/// app is [`PUBLISHED_SIGNATURE_CLIENT_ID`].
fn is_published_service(url: &str) -> bool {
    let origin = |url: &str| Url::parse(url).ok().map(|url| url.origin());
    origin(url).is_some() && origin(url) == origin(DEFAULT_SIGNATURE_URL)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "99999999-8888-7777-6666-555555555555";

    #[test]
    fn the_address_comes_from_the_environment_then_the_build() {
        assert_eq!(resolve_signature_url(None, None), DEFAULT_SIGNATURE_URL);
        assert_eq!(
            resolve_signature_url(None, Some("https://signature.example.org/")),
            "https://signature.example.org"
        );
        assert_eq!(
            resolve_signature_url(
                Some(" https://staging.example.org "),
                Some("https://signature.example.org")
            ),
            "https://staging.example.org"
        );
        assert_eq!(
            resolve_signature_url(Some("http://localhost:8787"), None),
            "http://localhost:8787"
        );
        assert_eq!(
            resolve_signature_url(Some("http://[::1]:8787/"), None),
            "http://[::1]:8787"
        );
        // Tokens would cross the network in the clear: ignored.
        assert_eq!(
            resolve_signature_url(Some("http://signature.example.org"), None),
            DEFAULT_SIGNATURE_URL
        );
        // Junk falls back to the next candidate.
        for junk in [
            "",
            "not a url",
            "ftp://signature.example.org",
            "https://user:secret@signature.example.org",
            "https://signature.example.org/?tenant=x",
            "https://signature.example.org/#x",
        ] {
            assert_eq!(
                resolve_signature_url(Some(junk), Some("https://built.example.org")),
                "https://built.example.org",
                "{junk}"
            );
        }
    }

    #[test]
    fn the_client_id_comes_from_the_environment_then_the_build_then_the_published_app() {
        let published = Some(PUBLISHED_SIGNATURE_CLIENT_ID);
        // Not set, or set empty (as the Flatpak manifest does): the
        // published shared app's.
        for (runtime, built_in) in [
            (None, None),
            (Some(""), None),
            (None, Some("")),
            (Some(" "), Some("")),
        ] {
            assert_eq!(
                resolve_client_id(runtime, built_in, DEFAULT_SIGNATURE_URL).as_deref(),
                published,
                "{runtime:?} {built_in:?}"
            );
        }
        assert_eq!(
            resolve_client_id(None, Some(ID), DEFAULT_SIGNATURE_URL).as_deref(),
            Some(ID)
        );
        assert_eq!(
            resolve_client_id(Some(&ID.to_uppercase()), Some("x"), DEFAULT_SIGNATURE_URL)
                .as_deref(),
            Some(ID)
        );
        assert_eq!(
            resolve_client_id(Some(""), Some(ID), DEFAULT_SIGNATURE_URL).as_deref(),
            Some(ID)
        );
    }

    #[test]
    fn none_turns_company_signatures_off() {
        for none in ["none", "NONE", " None "] {
            assert_eq!(
                resolve_client_id(None, Some(none), DEFAULT_SIGNATURE_URL),
                None
            );
            assert_eq!(
                resolve_client_id(Some(none), Some(ID), DEFAULT_SIGNATURE_URL),
                None
            );
        }
        // The environment comes first, so it can turn them on in a build
        // that has them off.
        assert_eq!(
            resolve_client_id(Some(ID), Some("none"), DEFAULT_SIGNATURE_URL).as_deref(),
            Some(ID)
        );
        assert_eq!(
            resolve_client_id(Some(""), Some("none"), DEFAULT_SIGNATURE_URL),
            None
        );
    }

    #[test]
    fn a_client_id_that_isnt_a_guid_is_ignored() {
        assert_eq!(
            resolve_client_id(Some("not-a-guid"), Some(ID), DEFAULT_SIGNATURE_URL).as_deref(),
            Some(ID)
        );
        assert_eq!(
            resolve_client_id(Some("{x}"), None, DEFAULT_SIGNATURE_URL).as_deref(),
            Some(PUBLISHED_SIGNATURE_CLIENT_ID)
        );
        assert_eq!(
            resolve_client_id(None, Some("off"), DEFAULT_SIGNATURE_URL).as_deref(),
            Some(PUBLISHED_SIGNATURE_CLIENT_ID)
        );
        // Ignored, so the build's `none` still holds.
        assert_eq!(
            resolve_client_id(Some("x"), Some("none"), DEFAULT_SIGNATURE_URL),
            None
        );
        assert_eq!(
            resolve_client_id(Some("x"), None, "https://signature.example.org"),
            None
        );
        assert!(is_guid(PUBLISHED_SIGNATURE_CLIENT_ID));
        assert!(!is_guid("99999999888877776666555555555555"));
        assert!(!is_guid("99999999-8888-7777-6666-55555555555g"));
    }

    #[test]
    fn the_published_client_id_goes_with_the_published_service_only() {
        for url in [
            DEFAULT_SIGNATURE_URL,
            "https://Signature.Cumulas.co",
            "https://signature.cumulas.co:443",
            "https://signature.cumulas.co/base",
        ] {
            assert_eq!(
                resolve_client_id(None, None, url).as_deref(),
                Some(PUBLISHED_SIGNATURE_CLIENT_ID),
                "{url}"
            );
        }
        // Another service has a shared app of its own: without its client
        // ID, company signatures are off rather than asking for a scope it
        // doesn't expose.
        for url in [
            "https://signature.example.org",
            "https://staging.signature.cumulas.co",
            "https://signature.cumulas.co:8443",
            "http://localhost:8787",
            "not a url",
        ] {
            assert_eq!(resolve_client_id(None, None, url), None, "{url}");
            assert_eq!(resolve_client_id(Some(""), Some(""), url), None, "{url}");
            assert_eq!(
                resolve_client_id(None, Some(ID), url).as_deref(),
                Some(ID),
                "{url}"
            );
            assert_eq!(
                resolve_client_id(Some(ID), None, url).as_deref(),
                Some(ID),
                "{url}"
            );
        }
    }

    #[test]
    fn with_nothing_set_signatures_come_from_cumulas_signatures() {
        let url = resolve_signature_url(None, Some(""));
        let client_id = resolve_client_id(None, Some(""), &url).unwrap();
        let service = SignatureService::new(url, client_id).unwrap();
        assert_eq!(service.host(), "signature.cumulas.co");
        assert_eq!(
            service.scope,
            "api://signature.cumulas.co/da27f705-3818-4ca0-9b5d-7781405ea54e/access_as_user"
        );
    }

    #[test]
    fn the_scope_names_the_host_and_a_port_that_isnt_the_default() {
        assert_eq!(
            signature_scope("https://signature.cumulas.co", ID).as_deref(),
            Some(format!("api://signature.cumulas.co/{ID}/access_as_user").as_str())
        );
        assert_eq!(
            signature_scope("https://Signature.Example.org:8443/base", ID).as_deref(),
            Some(format!("api://signature.example.org:8443/{ID}/access_as_user").as_str())
        );
        assert_eq!(
            signature_scope("https://signature.example.org:443", ID).as_deref(),
            Some(format!("api://signature.example.org/{ID}/access_as_user").as_str())
        );
        assert_eq!(
            signature_scope("http://localhost:8787", ID).as_deref(),
            Some(format!("api://localhost:8787/{ID}/access_as_user").as_str())
        );
        assert_eq!(signature_scope("https://signature.cumulas.co", "x"), None);
        assert_eq!(signature_scope("not a url", ID), None);
    }

    #[test]
    fn the_service_shows_its_host() {
        let service =
            SignatureService::new("https://signature.example.org:8443".into(), ID.into()).unwrap();
        assert_eq!(service.host(), "signature.example.org:8443");
        assert_eq!(
            service.scope,
            format!("api://signature.example.org:8443/{ID}/access_as_user")
        );
    }
}
