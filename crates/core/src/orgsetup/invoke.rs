//! The InvokeCommand client's rules for where its requests may go.

use url::{Origin, Url};

/// Microsoft's hosts InvokeCommand requests go to, and their subdomains
/// (regional Security & Compliance hosts, redirects).
const MICROSOFT_HOSTS: &[&str] = &[
    "outlook.office365.com",
    "ps.compliance.protection.outlook.com",
];

/// Where InvokeCommand requests, their redirects and their next pages may
/// go, which receive the administrator's token. Decided on the origin
/// (scheme, host, port), like `GraphClient::is_own_link`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRules {
    /// Another origin allowed besides Microsoft's (a mock server's, in
    /// tests).
    also: Option<Origin>,
}

impl HostRules {
    /// Microsoft's only: https on port 443, to Exchange Online and Security
    /// & Compliance hosts.
    pub fn production() -> Self {
        Self { also: None }
    }

    /// Microsoft's, and `url`'s origin (a mock server's, in tests).
    pub fn also(url: &Url) -> Self {
        Self {
            also: Some(url.origin()),
        }
    }

    /// Whether a request may go to `url`.
    pub fn allows(&self, url: &Url) -> bool {
        if !url.username().is_empty() || url.password().is_some() {
            return false;
        }
        if self.also.as_ref().is_some_and(|o| *o == url.origin()) {
            return true;
        }
        if url.scheme() != "https" || url.port_or_known_default() != Some(443) {
            return false;
        }
        let Some(url::Host::Domain(host)) = url.host() else {
            return false;
        };
        let host = host.to_ascii_lowercase();
        MICROSOFT_HOSTS.iter().any(|base| {
            host == *base
                || host
                    .strip_suffix(base)
                    .is_some_and(|sub| sub.len() > 1 && sub.ends_with('.'))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn only_microsofts_https_hosts_get_requests() {
        let rules = HostRules::production();
        for allowed in [
            "https://outlook.office365.com/adminapi/beta/t/InvokeCommand",
            "https://OUTLOOK.office365.com:443/adminapi/beta/t/InvokeCommand",
            "https://eur03b.outlook.office365.com/adminapi/beta/t/InvokeCommand",
            "https://ps.compliance.protection.outlook.com/adminapi/beta/t/InvokeCommand",
            "https://eur03b.ps.compliance.protection.outlook.com/adminapi/beta/t/InvokeCommand",
        ] {
            assert!(rules.allows(&url(allowed)), "{allowed}");
        }
        for refused in [
            "http://outlook.office365.com/adminapi/beta/t/InvokeCommand",
            "https://outlook.office365.com:446/adminapi/beta/t/InvokeCommand",
            "https://evil.example/adminapi/beta/t/InvokeCommand",
            "https://outlook.office365.com.evil.example/x",
            "https://evilоutlook.office365.com/x",
            "https://notoutlook.office365.com/x",
            "https://.outlook.office365.com/x",
            "https://eur03b.admin.protection.outlook.com/x",
            "https://user:pass@outlook.office365.com/x",
            "https://52.96.0.1/x",
        ] {
            assert!(!rules.allows(&url(refused)), "{refused}");
        }
    }

    #[test]
    fn tests_may_also_allow_a_mock_server() {
        let mock = url("http://127.0.0.1:41234/adminapi/beta/t/InvokeCommand");
        let rules = HostRules::also(&mock);
        assert!(rules.allows(&url("http://127.0.0.1:41234/next?page=2")));
        assert!(!rules.allows(&url("http://127.0.0.1:41235/next")));
        assert!(rules.allows(&url("https://outlook.office365.com/x")));
        // Production keeps refusing an http Location.
        assert!(!HostRules::production().allows(&mock));
    }
}
