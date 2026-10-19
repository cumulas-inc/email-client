//! Finding the organization's Security & Compliance host: the global host
//! redirects each organization to its region, the way the
//! ExchangeOnlineManagement module and CIPP find it.

use std::time::Instant;

use reqwest::header::LOCATION;
use url::Url;

use super::invoke::{Anchor, ErrorDetail, InvokeError, classify, error_detail, host_of};
use super::runlog::{CallRecord, MAX_MESSAGE, RunLog};
use crate::auth::admin_setup::{AdminTokens, Resource};
use crate::error::ErrorCode;
use crate::protection::clean_value;

/// The module version the discovery request names, as CIPP's does.
const MODULE_VERSION: &str = "3.9.2";
/// Where regional hosts are.
const COMPLIANCE_DOMAIN: &str = "ps.compliance.protection.outlook.com";

/// `GET <discovery>/adminapi/beta/{tid}/EXOBanner('AutogenSession')?Version=3.9.2`
/// with the Security & Compliance token, redirects off, trying `anchors` in
/// order (the administrator's mailbox, then the system mailbox at the
/// initial domain when known). A redirect's Location names the region: its
/// host's first label `<x>` gives `https://<x>.ps.compliance.protection.outlook.com`
/// (an `.admin.protection.outlook.com` Location is read the same way). A
/// success means the global host serves the organization: `discovery`'s
/// origin. A 401, 403 or 404 with the administrator's mailbox tries the
/// next anchor. The Location's host (never its path or query) and the
/// anchor that produced it go to the run log.
pub async fn compliance_host(
    http: &reqwest::Client,
    tokens: &AdminTokens,
    tenant_id: &str,
    anchors: &[Anchor],
    discovery: &Url,
    log: &RunLog,
) -> Result<Url, InvokeError> {
    let started = Instant::now();
    let mut record = CallRecord::new(
        "Find the Security & Compliance host",
        &host_of(discovery),
        "EXOBanner",
    );
    let result = discover(
        http,
        tokens,
        tenant_id,
        anchors,
        discovery,
        log,
        &mut record,
    )
    .await;
    record.ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    if let Err(e) = &result {
        record.error = Some(e.detail().cloned().unwrap_or_else(|| ErrorDetail {
            message: clean_value(&e.message(), MAX_MESSAGE).unwrap_or_default(),
            ..ErrorDetail::default()
        }));
    }
    tracing::info!(
        target: "orgsetup",
        host = %record.host,
        status = ?record.status,
        ms = record.ms,
        anchor = record.anchor.unwrap_or("none"),
        "EXOBanner"
    );
    log.call(record);
    result
}

async fn discover(
    http: &reqwest::Client,
    tokens: &AdminTokens,
    tenant_id: &str,
    anchors: &[Anchor],
    discovery: &Url,
    log: &RunLog,
    record: &mut CallRecord,
) -> Result<Url, InvokeError> {
    let mut url = discovery.clone();
    url.set_query(None);
    if let Ok(mut segments) = url.path_segments_mut() {
        segments
            .clear()
            .push("adminapi")
            .push("beta")
            .push(tenant_id)
            .push("EXOBanner('AutogenSession')");
    }
    url.query_pairs_mut().append_pair("Version", MODULE_VERSION);

    let mut first: Option<InvokeError> = None;
    for (i, anchor) in anchors.iter().enumerate() {
        let mut refreshed = false;
        let answer = loop {
            let token = tokens
                .token(Resource::Compliance)
                .await
                .map_err(|e| match e.code {
                    ErrorCode::Network => InvokeError::Network(e.message),
                    _ => InvokeError::Unauthorized(ErrorDetail {
                        message: clean_value(&e.message, MAX_MESSAGE).unwrap_or_default(),
                        ..ErrorDetail::default()
                    }),
                })?;
            let response = http
                .get(url.clone())
                .bearer_auth(token)
                .header("X-AnchorMailbox", anchor.value())
                .header("client-request-id", uuid::Uuid::new_v4().to_string())
                .send()
                .await;
            let response = match response {
                Ok(response) => response,
                Err(e) => {
                    record.attempts.push((Some(anchor.kind()), None));
                    return Err(InvokeError::Network(e.without_url().to_string()));
                }
            };
            let status = response.status().as_u16();
            record.attempts.push((Some(anchor.kind()), Some(status)));
            record.status = Some(status);
            record.anchor = Some(anchor.kind());
            if status == 401 && !refreshed {
                refreshed = true;
                tokens.invalidate(Resource::Compliance).await;
                continue;
            }
            break response;
        };
        let status = answer.status();
        if status.is_redirection() {
            let location = answer
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|l| url.join(l).ok());
            let Some(location) = location else {
                return Err(InvokeError::Redirect(
                    "a redirect without a location".into(),
                ));
            };
            let host = regional_host(&location)?;
            log_host(log, &host, "redirect", anchor);
            return Ok(host);
        }
        if status.is_success() {
            let mut global = discovery.clone();
            global.set_path("/");
            global.set_query(None);
            log_host(log, &global, "global", anchor);
            return Ok(global);
        }
        let detail = error_detail(
            status.as_u16(),
            &answer.bytes().await.unwrap_or_default(),
            None,
        );
        let error = classify(status.as_u16(), detail, Resource::Compliance);
        let next = i + 1 < anchors.len()
            && matches!(anchor, Anchor::Upn(_))
            && matches!(status.as_u16(), 401 | 403 | 404);
        if !next {
            return Err(first.unwrap_or(error));
        }
        first.get_or_insert(error);
    }
    Err(first.unwrap_or_else(|| InvokeError::Unexpected("no anchor to discover with".into())))
}

/// The regional REST host a discovery redirect names.
fn regional_host(location: &Url) -> Result<Url, InvokeError> {
    let host = location
        .host_str()
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let label = host
        .strip_suffix(".protection.outlook.com")
        .and_then(|rest| rest.split('.').next())
        .filter(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    let Some(label) = label else {
        return Err(InvokeError::Redirect(host_of(location)));
    };
    let host = if label == "ps" {
        COMPLIANCE_DOMAIN.to_owned()
    } else {
        format!("{label}.{COMPLIANCE_DOMAIN}")
    };
    Url::parse(&format!("https://{host}/")).map_err(|_| InvokeError::Redirect(host))
}

fn log_host(log: &RunLog, host: &Url, how: &str, anchor: &Anchor) {
    log.note(format!(
        "Security & Compliance host: {} ({how}, anchor {})",
        host_of(host),
        anchor.kind()
    ));
}
