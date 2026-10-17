//! Error type shared by the whole core. It serializes to
//! `{ code, message, details? }` so the UI can branch on `code`.

use serde::Serialize;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    /// Stored credentials are missing or were rejected; the user must sign in again.
    AuthRequired,
    /// The user's organization does not allow users to consent to this app.
    /// `details` carries `{ adminConsentUrl, tenant, canRequestApproval,
    /// adminPageUrl }`.
    AdminConsentRequired,
    /// The user declined the permission prompt.
    ConsentDeclined,
    /// Sign-in was canceled or timed out before the browser redirected back.
    SignInCancelled,
    /// The organization's settings block this sign-in in a way approving the app
    /// doesn't fix (except `appNotSetUp`): `details` carries
    /// `{ reason, aadsts, tenant, adminPageUrl, adminConsentUrl? }`.
    SignInBlocked,
    /// A Microsoft account's optional feature can't be used:
    /// `details` carries `{ feature, accountId, reason }`.
    PermissionRequired,
    /// A required setting (e.g. the Microsoft client id) is missing.
    NotConfigured,
    /// Could not reach the server.
    Network,
    /// The server returned an error.
    Provider,
    NotFound,
    InvalidInput,
    /// The operation is not available for this account type.
    Unsupported,
    Storage,
    Internal,
}

#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn auth_required(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::AuthRequired, message)
    }

    pub fn not_found(what: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, format!("{} not found", what.into()))
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidInput, message)
    }

    pub fn network(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Network, message)
    }

    pub fn provider(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Provider, message)
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Unsupported, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }

    /// The HTTP status the server answered with, when the error came from
    /// one that said (Microsoft Graph's errors carry it in `details`).
    pub fn http_status(&self) -> Option<u16> {
        self.details
            .as_ref()?
            .get("status")?
            .as_u64()
            .and_then(|s| u16::try_from(s).ok())
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        match e {
            rusqlite::Error::QueryReturnedNoRows => Error::new(ErrorCode::NotFound, "not found"),
            other => Error::new(ErrorCode::Storage, other.to_string()),
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::internal(format!("serialization error: {e}"))
    }
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        if e.is_connect() || e.is_timeout() || e.is_request() {
            Error::network(e.to_string())
        } else {
            Error::provider(e.to_string())
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::internal(e.to_string())
    }
}
