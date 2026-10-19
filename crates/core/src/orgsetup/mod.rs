//! Organization setup: an administrator signs in with the setup app
//! ([`crate::auth::admin_setup`]), and Cumulas Mail reads, creates and
//! repairs the organization's mail flow rules and auto-labeling policies
//! through Exchange Online's InvokeCommand interface, or writes a PowerShell
//! script that does the same. Only objects whose names start with
//! "Cumulas Mail: " are ever changed, and nothing is ever deleted.

pub mod invoke;
pub mod runlog;

use tokio_util::sync::CancellationToken;

pub use invoke::HostRules;
pub use runlog::{RunLog, details_text};

use crate::model::OrgSetupAdmin;

/// An administrator's setup sign-in for one organization: at most one at a
/// time, in memory only. Dropping it drops its tokens.
pub struct Session {
    pub tenant_id: String,
    /// The work account the dialog was opened from.
    pub account_id: String,
    pub admin: OrgSetupAdmin,
    /// The ID token's oid: for the command that removes this
    /// administrator's approval.
    pub admin_object_id: String,
    pub client_id: String,
    /// Cancelled by `end_org_setup`, a new sign-in, or the idle watcher.
    cancel: CancellationToken,
}

impl Session {
    /// Ends the session's work: a read in flight returns at once, an apply
    /// finishes the change it's in.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}
