//! The run log: every sign-in attempt and Microsoft call organization setup
//! makes, in memory only, for "Copy setup details".
//!
//! It holds no token, cookie or password, ever. Parameter values and raw
//! JSON are kept only for Cumulas Mail's own rules and policies; other rules
//! appear by name, order, state and mode, and whether they stop later rules
//! or send mail out (its opening sentence lists everything it names).

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};

use super::invoke::ErrorDetail;
use crate::auth::admin_setup::SETUP_APP_NAME;
use crate::error::Error;
use crate::protection::clean_value;

/// Most entries kept; the earliest go first.
const MAX_ENTRIES: usize = 500;
/// Longest message kept, in characters.
pub(crate) const MAX_MESSAGE: usize = 500;
/// Longest response body excerpt kept.
pub(crate) const MAX_EXCERPT: usize = 2000;
/// Longest raw JSON kept of one of Cumulas Mail's objects.
pub(crate) const MAX_RAW: usize = 4000;

/// What organization setup did, kept after its session ends (so "Copy
/// setup details" works in every state) until the next sign-in, and never
/// written anywhere. A handle: clones share the same log.
#[derive(Clone, Default)]
pub struct RunLog {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    header: Mutex<RunHeader>,
    entries: Mutex<Entries>,
}

#[derive(Default)]
struct Entries {
    kept: Vec<Entry>,
    /// How many of the earliest were left out.
    dropped: usize,
}

impl std::fmt::Debug for RunLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunLog")
            .field("entries", &self.inner.entries.lock().unwrap().kept.len())
            .finish_non_exhaustive()
    }
}

/// Who and what a run is about.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunHeader {
    pub tenant_id: Option<String>,
    /// The setup app's client ID.
    pub client_id: Option<String>,
    /// The administrator's sign-in name, once signed in.
    pub admin: Option<String>,
}

/// One thing the run log holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    /// Token scopes, the Security & Compliance host, licensing, sign-outs.
    Note {
        at: DateTime<Utc>,
        text: String,
    },
    /// Every sign-in attempt, failed or not.
    SignIn(SignInRecord),
    Call(CallRecord),
    /// What a read found: rule order, connectors, labels, Cumulas Mail's
    /// own objects as Microsoft 365 returned them.
    Snapshot {
        at: DateTime<Utc>,
        summary: String,
    },
}

impl Entry {
    /// When it happened (a call: when it started).
    pub fn at(&self) -> DateTime<Utc> {
        match self {
            Entry::Note { at, .. } | Entry::Snapshot { at, .. } => *at,
            Entry::SignIn(record) => record.at,
            Entry::Call(record) => record.at,
        }
    }
}

/// A sign-in attempt with the setup app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignInRecord {
    pub at: DateTime<Utc>,
    /// `login.microsoftonline.com/<tenant>`.
    pub authority: String,
    pub scope: String,
    /// The loopback listener's port.
    pub port: Option<u16>,
    /// "code redeemed", "error", "cancelled", "timed out", "failed".
    pub outcome: String,
    /// Microsoft's `error`.
    pub error: Option<String>,
    pub aadsts: Vec<u32>,
    /// The first line of Microsoft's description.
    pub description: Option<String>,
    /// The redemption's `scope`.
    pub granted: Option<String>,
    pub id_token: bool,
    pub refresh_token: bool,
}

impl SignInRecord {
    /// An attempt about to start.
    pub fn new(authority: String, scope: &str) -> Self {
        Self {
            at: Utc::now(),
            authority,
            scope: scope.to_owned(),
            port: None,
            outcome: String::new(),
            error: None,
            aadsts: Vec::new(),
            description: None,
            granted: None,
            id_token: false,
            refresh_token: false,
        }
    }

    /// Microsoft refused it: its `error`, AADSTS codes and the first line
    /// of its description.
    pub fn refused(self, error: &str, aadsts: Vec<u32>, description: &str) -> Self {
        Self {
            outcome: "error".into(),
            error: clean_value(error, 100),
            aadsts,
            description: clean_value(description, MAX_MESSAGE),
            ..self
        }
    }

    /// It ended without Microsoft's answer: canceled, timed out, or the
    /// browser or the network failed.
    pub fn failed(self, error: &Error) -> Self {
        let outcome = match (error.code, &error.details) {
            (crate::ErrorCode::SignInCancelled, Some(d)) if d["reason"] == "timeout" => "timed out",
            (crate::ErrorCode::SignInCancelled, _) => "cancelled",
            _ => "failed",
        };
        Self {
            outcome: outcome.into(),
            description: clean_value(&error.message, MAX_MESSAGE),
            ..self
        }
    }

    /// The code was redeemed.
    pub fn redeemed(self, granted: Option<&str>, id_token: bool, refresh_token: bool) -> Self {
        Self {
            outcome: "code redeemed".into(),
            granted: granted.and_then(|g| clean_value(g, MAX_MESSAGE)),
            id_token,
            refresh_token,
            ..self
        }
    }
}

/// One InvokeCommand call (or the discovery and licensing requests), with
/// every attempt it took.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallRecord {
    pub at: DateTime<Utc>,
    /// "Create “Cumulas Mail: Encrypt”".
    pub step: String,
    /// The host only.
    pub host: String,
    /// The cmdlet, or "EXOBanner".
    pub cmdlet: String,
    /// Names always; values only for calls on Cumulas Mail's own objects.
    pub parameters: Vec<(String, Option<String>)>,
    /// The anchor of the attempt that answered: "UPN" or "SystemMailbox".
    pub anchor: Option<&'static str>,
    /// Each attempt's anchor and status (`None`: no answer).
    pub attempts: Vec<(Option<&'static str>, Option<u16>)>,
    /// Hosts redirected to.
    pub redirects: Vec<String>,
    pub status: Option<u16>,
    pub ms: u64,
    pub results: Option<usize>,
    pub warnings: Vec<String>,
    pub error: Option<ErrorDetail>,
    /// "adopted", "cleared with \"\"", "without Comments", ...
    pub outcome: Option<String>,
}

impl CallRecord {
    pub fn new(step: &str, host: &str, cmdlet: &str) -> Self {
        Self {
            at: Utc::now(),
            step: step.to_owned(),
            host: host.to_owned(),
            cmdlet: cmdlet.to_owned(),
            parameters: Vec::new(),
            anchor: None,
            attempts: Vec::new(),
            redirects: Vec::new(),
            status: None,
            ms: 0,
            results: None,
            warnings: Vec::new(),
            error: None,
            outcome: None,
        }
    }
}

impl RunLog {
    /// Starts a new run (a sign-in attempt): the header is replaced and the
    /// entries cleared.
    pub fn start(&self, header: RunHeader) {
        *self.inner.header.lock().unwrap() = header;
        *self.inner.entries.lock().unwrap() = Entries::default();
    }

    /// The administrator who signed in.
    pub fn set_admin(&self, username: &str) {
        self.inner.header.lock().unwrap().admin = clean_value(username, 320);
    }

    pub fn header(&self) -> RunHeader {
        self.inner.header.lock().unwrap().clone()
    }

    pub fn note(&self, text: impl Into<String>) {
        let text = text.into();
        if let Some(text) = clean_value(&text, MAX_MESSAGE) {
            self.push(Entry::Note {
                at: Utc::now(),
                text,
            });
        }
    }

    pub fn sign_in(&self, record: SignInRecord) {
        self.push(Entry::SignIn(record));
    }

    pub fn call(&self, record: CallRecord) {
        self.push(Entry::Call(record));
    }

    /// What a read found, as [`super::status::snapshot_summary`] puts it.
    pub fn snapshot(&self, summary: String) {
        self.push(Entry::Snapshot {
            at: Utc::now(),
            summary,
        });
    }

    /// The entries kept, oldest first.
    pub fn entries(&self) -> Vec<Entry> {
        self.inner.entries.lock().unwrap().kept.clone()
    }

    fn push(&self, entry: Entry) {
        let mut entries = self.inner.entries.lock().unwrap();
        if entries.kept.len() >= MAX_ENTRIES {
            entries.kept.remove(0);
            entries.dropped += 1;
        }
        entries.kept.push(entry);
    }
}

/// The "Copy setup details" text; empty while nothing was logged.
pub fn details_text(log: &RunLog) -> String {
    let header = log.header();
    let (mut entries, dropped) = {
        let entries = log.inner.entries.lock().unwrap();
        (entries.kept.clone(), entries.dropped)
    };
    // A call is logged once it ends, with the time it started: what was
    // noted while it ran comes after it.
    entries.sort_by_key(Entry::at);
    if header == RunHeader::default() && entries.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Cumulas Mail {}: organization setup details",
        env!("CARGO_PKG_VERSION")
    );
    out.push_str(
        "These details include no passwords or tokens. They name your organization's tenant and \
         initial domain, the administrator who signed in, the work account's licenses, your \
         Message Encryption settings and template names, every sensitivity label's name and \
         IDs, Cumulas Signatures' connector, the settings of Cumulas Mail's own rules and \
         policies, and Microsoft 365's error messages. Other mail flow rules appear by name, \
         order, state and mode, and whether they stop later rules or send mail out; their \
         conditions and actions aren't included.\n\n",
    );
    let unknown = || "unknown".to_owned();
    let _ = writeln!(out, "Tenant: {}", header.tenant_id.unwrap_or_else(unknown));
    let _ = writeln!(
        out,
        "Setup app: {SETUP_APP_NAME} ({})",
        header.client_id.unwrap_or_else(unknown)
    );
    let _ = writeln!(
        out,
        "Signed in as: {}",
        header.admin.unwrap_or_else(|| "nobody yet".into())
    );
    if dropped > 0 {
        let _ = writeln!(out, "\n({dropped} earlier entries were left out.)");
    }
    for entry in &entries {
        out.push('\n');
        write_entry(&mut out, entry);
    }
    out
}

fn time(at: &DateTime<Utc>) -> String {
    at.format("%H:%M:%S UTC").to_string()
}

fn write_entry(out: &mut String, entry: &Entry) {
    match entry {
        Entry::Note { at, text } => {
            let _ = writeln!(out, "{}  {text}", time(at));
        }
        Entry::SignIn(record) => write_sign_in(out, record),
        Entry::Call(record) => write_call(out, record),
        Entry::Snapshot { at, summary } => {
            let _ = writeln!(out, "{}  What Microsoft 365 returned:", time(at));
            for line in summary.lines() {
                let _ = writeln!(out, "  {line}");
            }
        }
    }
}

fn write_sign_in(out: &mut String, record: &SignInRecord) {
    let _ = write!(
        out,
        "{}  Sign-in: {}, scope \"{}\"",
        time(&record.at),
        record.authority,
        record.scope
    );
    if let Some(port) = record.port {
        let _ = write!(out, ", port {port}");
    }
    out.push('\n');
    let mut parts = vec![record.outcome.clone()];
    if let Some(error) = &record.error {
        parts.push(error.clone());
    }
    parts.extend(record.aadsts.iter().map(|c| format!("AADSTS{c}")));
    if let Some(description) = &record.description {
        parts.push(format!("\"{description}\""));
    }
    if record.outcome == "code redeemed" {
        if let Some(granted) = &record.granted {
            parts.push(format!("granted \"{granted}\""));
        }
        parts.push(format!("ID token {}", yes_no(record.id_token)));
        parts.push(format!("refresh token {}", yes_no(record.refresh_token)));
    }
    let _ = writeln!(out, "  {}", parts.join(" · "));
}

fn write_call(out: &mut String, record: &CallRecord) {
    let _ = writeln!(out, "{}  {}", time(&record.at), record.step);
    let mut line = record.cmdlet.clone();
    if !record.parameters.is_empty() {
        let parameters: Vec<String> = record
            .parameters
            .iter()
            .map(|(name, value)| match value {
                Some(value) => format!("{name}={value}"),
                None => name.clone(),
            })
            .collect();
        let _ = write!(line, " ({})", parameters.join(", "));
    }
    let _ = write!(line, " on {}", record.host);
    let mut parts = vec![line];
    if let Some(anchor) = record.anchor {
        parts.push(format!("anchor {anchor}"));
    }
    parts.push(match record.status {
        Some(status) => status.to_string(),
        None => "no answer".into(),
    });
    parts.push(format!("{} ms", record.ms));
    if let Some(results) = record.results {
        parts.push(match results {
            1 => "1 result".into(),
            n => format!("{n} results"),
        });
    }
    let _ = writeln!(out, "  {}", parts.join(" · "));
    if record.attempts.len() > 1 {
        let attempts: Vec<String> = record
            .attempts
            .iter()
            .map(|(anchor, status)| {
                let status = status.map_or_else(|| "no answer".into(), |s| s.to_string());
                match anchor {
                    Some(anchor) => format!("{anchor} {status}"),
                    None => status,
                }
            })
            .collect();
        let _ = writeln!(out, "  Attempts: {}", attempts.join(", "));
    }
    if !record.redirects.is_empty() {
        let _ = writeln!(out, "  Redirected to: {}", record.redirects.join(", "));
    }
    for warning in &record.warnings {
        let _ = writeln!(out, "  Warning: {warning}");
    }
    if let Some(error) = &record.error {
        let mut parts = Vec::new();
        if let Some(code) = &error.code {
            parts.push(code.clone());
        }
        if let Some(kind) = &error.exception_type {
            parts.push(kind.clone());
        }
        parts.push(format!("\"{}\"", error.message));
        let _ = writeln!(out, "  Error: {}", parts.join(" · "));
        if !error.excerpt.is_empty() {
            let _ = writeln!(out, "  Body: {}", error.excerpt);
        }
    }
    if let Some(outcome) = &record.outcome {
        let _ = writeln!(out, "  Outcome: {outcome}");
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
