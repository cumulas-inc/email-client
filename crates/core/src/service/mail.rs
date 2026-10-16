use std::collections::HashMap;

use chrono::Utc;
use serde::Serialize;

use super::CoreService;
use crate::error::{Error, ErrorCode, Result};
use crate::html::sanitize_email_html;
use crate::model::{
    AttachmentMeta, DataKind, EmailAddress, Folder, FolderRole, Message, MessagePage, MessageQuery,
    MessageSummary, OutgoingMessage, RemoteContentPolicy,
};
use crate::providers::{MessageContent, MessageLocator};

/// A downloaded attachment.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub meta: AttachmentMeta,
    #[serde(skip)]
    pub data: Vec<u8>,
}

/// Messages grouped by account, with their server locators.
struct Selection {
    by_account: HashMap<String, Vec<(MessageSummary, MessageLocator)>>,
}

impl CoreService {
    fn select(&self, ids: &[String]) -> Result<Selection> {
        let mut folders: HashMap<String, Folder> = HashMap::new();
        let mut by_account: HashMap<String, Vec<(MessageSummary, MessageLocator)>> = HashMap::new();
        for summary in self.store.get_message_summaries(ids)? {
            let folder = match folders.get(&summary.folder_id) {
                Some(f) => f.clone(),
                None => {
                    let f = self.store.get_folder(&summary.folder_id)?;
                    folders.insert(f.id.clone(), f.clone());
                    f
                }
            };
            let locator = MessageLocator {
                folder_remote_id: folder.remote_id,
                remote_id: summary.remote_id.clone(),
            };
            by_account
                .entry(summary.account_id.clone())
                .or_default()
                .push((summary, locator));
        }
        Ok(Selection { by_account })
    }

    pub fn list_folders(&self, account_id: Option<&str>) -> Result<Vec<Folder>> {
        self.store.list_folders(account_id)
    }

    pub fn list_messages(&self, folder_id: &str, query: &MessageQuery) -> Result<MessagePage> {
        self.store.list_messages(folder_id, query)
    }

    pub fn suggest_addresses(&self, text: &str) -> Result<Vec<EmailAddress>> {
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }
        self.store.suggest_addresses(text.trim(), 8)
    }

    /// Cached content, or fetched from the server and cached. A message the
    /// server no longer has is removed here too: deletions outside the
    /// sync window (of messages server search found) are noticed so.
    async fn message_content(&self, summary: &MessageSummary) -> Result<MessageContent> {
        if let Some(content) = self.store.get_message_content(&summary.id)? {
            return Ok(content);
        }
        let folder = self.store.get_folder(&summary.folder_id)?;
        let providers = self.providers_for(&summary.account_id).await?;
        let fetched = providers
            .mail
            .fetch_message(&MessageLocator {
                folder_remote_id: folder.remote_id,
                remote_id: summary.remote_id.clone(),
            })
            .await;
        let content = match fetched {
            Ok(content) => content,
            Err(e) if e.code == ErrorCode::NotFound => {
                let gone = [summary.id.clone()];
                self.store.forget_gone_messages(&gone)?;
                self.sink.mail_settled(&gone);
                self.notify(&[DataKind::Mail], Some(&summary.account_id));
                return Err(Error::new(
                    ErrorCode::NotFound,
                    "This message is no longer on the server.",
                ));
            }
            Err(e) => return Err(e),
        };
        self.store.save_message_content(&summary.id, &content)?;
        Ok(content)
    }

    /// A message ready for the reading pane. Remote content is loaded when
    /// `allow_remote` is set or the settings allow it for this sender.
    pub async fn get_message(&self, id: &str, allow_remote: bool) -> Result<Message> {
        let summary = self.store.get_message_summary(id)?;
        // A search result opened is kept longer.
        self.store.touch_search_result(id, Utc::now())?;
        let content = self.message_content(&summary).await?;
        let allow_remote = allow_remote || self.remote_content_allowed(&summary)?;
        let sanitized = content
            .body_html
            .as_deref()
            .map(|html| sanitize_email_html(html, allow_remote));
        Ok(Message {
            summary,
            bcc: content.bcc,
            reply_to: content.reply_to,
            has_blocked_content: sanitized.as_ref().is_some_and(|s| s.has_blocked_content),
            body_html: sanitized.map(|s| s.html),
            body_text: content.body_text,
            attachments: content.attachments,
            internet_message_id: content.internet_message_id,
            in_reply_to: content.in_reply_to,
            references: content.references,
        })
    }

    fn remote_content_allowed(&self, summary: &MessageSummary) -> Result<bool> {
        Ok(match self.store.get_settings()?.remote_content {
            RemoteContentPolicy::Allow => true,
            RemoteContentPolicy::Block => false,
            RemoteContentPolicy::AllowContacts => match &summary.from {
                Some(from) => self
                    .store
                    .list_contacts(None, Some(&from.address))?
                    .iter()
                    .any(|c| {
                        c.emails
                            .iter()
                            .any(|e| e.address.eq_ignore_ascii_case(&from.address))
                    }),
                None => false,
            },
        })
    }

    pub async fn get_attachment(
        &self,
        message_id: &str,
        attachment_id: &str,
    ) -> Result<Attachment> {
        let summary = self.store.get_message_summary(message_id)?;
        let content = self.message_content(&summary).await?;
        let meta = content
            .attachments
            .into_iter()
            .find(|a| a.id == attachment_id)
            .ok_or_else(|| Error::not_found("attachment"))?;
        let folder = self.store.get_folder(&summary.folder_id)?;
        let providers = self.providers_for(&summary.account_id).await?;
        let data = providers
            .mail
            .fetch_attachment(
                &MessageLocator {
                    folder_remote_id: folder.remote_id,
                    remote_id: summary.remote_id,
                },
                attachment_id,
            )
            .await?;
        Ok(Attachment { meta, data })
    }

    pub async fn set_read(&self, ids: &[String], read: bool) -> Result<()> {
        for (account_id, items) in self.select(ids)?.by_account {
            let changed: Vec<_> = items.iter().filter(|(s, _)| s.is_read != read).collect();
            if changed.is_empty() {
                continue;
            }
            let locators: Vec<_> = changed.iter().map(|(_, l)| l.clone()).collect();
            self.providers_for(&account_id)
                .await?
                .mail
                .set_read(&locators, read)
                .await?;
            let ids: Vec<_> = changed.iter().map(|(s, _)| s.id.clone()).collect();
            self.store.set_messages_read(&ids, read)?;
            if read {
                self.sink.mail_settled(&ids);
            }
        }
        self.notify(&[DataKind::Mail], None);
        Ok(())
    }

    pub async fn set_flagged(&self, ids: &[String], flagged: bool) -> Result<()> {
        for (account_id, items) in self.select(ids)?.by_account {
            let locators: Vec<_> = items.iter().map(|(_, l)| l.clone()).collect();
            self.providers_for(&account_id)
                .await?
                .mail
                .set_flagged(&locators, flagged)
                .await?;
            let ids: Vec<_> = items.iter().map(|(s, _)| s.id.clone()).collect();
            self.store.set_messages_flagged(&ids, flagged)?;
        }
        self.notify(&[DataKind::Mail], None);
        Ok(())
    }

    pub async fn move_messages(&self, ids: &[String], destination_folder_id: &str) -> Result<()> {
        let destination = self.store.get_folder(destination_folder_id)?;
        for (account_id, items) in self.select(ids)?.by_account {
            if account_id != destination.account_id {
                return Err(Error::unsupported(
                    "Messages can only be moved between folders of the same account.",
                ));
            }
            let items: Vec<_> = items
                .into_iter()
                .filter(|(s, _)| s.folder_id != destination.id)
                .collect();
            if items.is_empty() {
                continue;
            }
            let locators: Vec<_> = items.iter().map(|(_, l)| l.clone()).collect();
            let new_ids = self
                .providers_for(&account_id)
                .await?
                .mail
                .move_messages(&locators, &destination.remote_id)
                .await?;
            for (i, (summary, _)) in items.iter().enumerate() {
                let new_remote = new_ids.get(i).cloned().flatten();
                self.store
                    .relocate_message(&summary.id, &destination, new_remote.as_deref())?;
            }
            // Moved messages get new ids; the old ones are settled.
            let moved: Vec<_> = items.iter().map(|(s, _)| s.id.clone()).collect();
            self.sink.mail_settled(&moved);
        }
        self.notify(&[DataKind::Mail], Some(&destination.account_id));
        Ok(())
    }

    /// Moves messages to the account's archive folder.
    pub async fn archive_messages(&self, ids: &[String]) -> Result<()> {
        for (account_id, items) in self.select(ids)?.by_account {
            let archive = self
                .store
                .find_folder_by_role(&account_id, FolderRole::Archive)?
                .ok_or_else(|| Error::unsupported("This account has no Archive folder."))?;
            let ids: Vec<_> = items.into_iter().map(|(s, _)| s.id).collect();
            self.move_messages(&ids, &archive.id).await?;
        }
        Ok(())
    }

    /// Moves messages to Deleted Items; messages already there (or in Junk,
    /// or in accounts without a trash folder) are deleted permanently.
    pub async fn delete_messages(&self, ids: &[String]) -> Result<()> {
        for (account_id, items) in self.select(ids)?.by_account {
            let trash = self
                .store
                .find_folder_by_role(&account_id, FolderRole::Trash)?;
            let mut to_trash = Vec::new();
            let mut permanent = Vec::new();
            for (summary, locator) in items {
                let role = self.store.get_folder(&summary.folder_id)?.role;
                match &trash {
                    Some(_) if !matches!(role, FolderRole::Trash | FolderRole::Junk) => {
                        to_trash.push(summary.id)
                    }
                    _ => permanent.push((summary.id, locator)),
                }
            }
            if let Some(trash) = &trash
                && !to_trash.is_empty()
            {
                self.move_messages(&to_trash, &trash.id).await?;
            }
            if !permanent.is_empty() {
                let locators: Vec<_> = permanent.iter().map(|(_, l)| l.clone()).collect();
                self.providers_for(&account_id)
                    .await?
                    .mail
                    .delete_messages(&locators)
                    .await?;
                let ids: Vec<_> = permanent.into_iter().map(|(id, _)| id).collect();
                self.store.delete_messages(&ids)?;
                self.sink.mail_settled(&ids);
            }
        }
        self.notify(&[DataKind::Mail], None);
        Ok(())
    }

    pub async fn send_message(&self, message: OutgoingMessage) -> Result<()> {
        if message.to.is_empty() && message.cc.is_empty() && message.bcc.is_empty() {
            return Err(Error::invalid_input("Add at least one recipient."));
        }
        for addr in message.to.iter().chain(&message.cc).chain(&message.bcc) {
            if !is_plausible_address(&addr.address) {
                return Err(Error::invalid_input(format!(
                    "\"{}\" is not a valid email address.",
                    addr.address
                )));
            }
        }
        self.store.get_account(&message.account_id)?;
        let original = match &message.reply_to_message_id {
            Some(id) => Some(self.get_message(id, false).await?),
            None => None,
        };
        self.providers_for(&message.account_id)
            .await?
            .mail
            .send(&message, original.as_ref())
            .await?;
        self.notify(&[DataKind::Mail], Some(&message.account_id));
        Ok(())
    }
}

fn is_plausible_address(address: &str) -> bool {
    let Some((local, domain)) = address.trim().rsplit_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !address
            .chars()
            .any(|c| c.is_whitespace() || c == '<' || c == '>' || c == ',')
}
