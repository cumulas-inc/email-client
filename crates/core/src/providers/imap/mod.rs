//! IMAP (mail access) + SMTP (sending) provider for standards-based accounts.
//!
//! STUB: the public API below is the contract used by `service.rs`; the
//! implementation is filled in separately.

use async_trait::async_trait;

use super::{MailProvider, MessageBatch, MessageContent, MessageLocator, SyncWindow};
use crate::error::{Error, Result};
use crate::model::{Folder, ImapSettings, Message, OutgoingMessage};

/// Credentials and server settings for one IMAP account.
#[derive(Clone)]
pub struct ImapCredentials {
    pub settings: ImapSettings,
    pub password: String,
    /// Used as the `From` display name when sending.
    pub display_name: String,
    pub email: String,
}

pub struct ImapProvider {
    _account_id: String,
    _credentials: ImapCredentials,
}

impl ImapProvider {
    pub fn new(account_id: impl Into<String>, credentials: ImapCredentials) -> Self {
        Self {
            _account_id: account_id.into(),
            _credentials: credentials,
        }
    }
}

/// Logs in to the IMAP server and authenticates against the SMTP server
/// without changing anything; used to validate settings when adding an account.
pub async fn test_connection(_credentials: &ImapCredentials) -> Result<()> {
    Err(todo())
}

fn todo() -> Error {
    Error::unsupported("IMAP provider is not implemented yet")
}

#[async_trait]
impl MailProvider for ImapProvider {
    async fn list_folders(&self) -> Result<Vec<Folder>> {
        Err(todo())
    }
    async fn sync_messages(
        &self,
        _: &Folder,
        _: Option<&str>,
        _: SyncWindow,
    ) -> Result<MessageBatch> {
        Err(todo())
    }
    async fn fetch_message(&self, _: &MessageLocator) -> Result<MessageContent> {
        Err(todo())
    }
    async fn fetch_attachment(&self, _: &MessageLocator, _: &str) -> Result<Vec<u8>> {
        Err(todo())
    }
    async fn set_read(&self, _: &[MessageLocator], _: bool) -> Result<()> {
        Err(todo())
    }
    async fn set_flagged(&self, _: &[MessageLocator], _: bool) -> Result<()> {
        Err(todo())
    }
    async fn move_messages(&self, _: &[MessageLocator], _: &str) -> Result<Vec<Option<String>>> {
        Err(todo())
    }
    async fn delete_messages(&self, _: &[MessageLocator]) -> Result<()> {
        Err(todo())
    }
    async fn send(&self, _: &OutgoingMessage, _: Option<&Message>) -> Result<()> {
        Err(todo())
    }
}
