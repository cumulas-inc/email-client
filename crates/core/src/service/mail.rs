use std::collections::HashMap;

use chrono::Utc;
use serde::Serialize;
use serde_json::json;

use super::CoreService;
use super::protection::{label_title, listed_label, offered, work_tenant};
use crate::error::{Error, ErrorCode, Result};
use crate::html::sanitize_email_html;
use crate::model::{
    Account, AccountKind, AttachmentMeta, CheckItem, CheckState, DataKind, EmailAddress, Folder,
    FolderRole, LabelCatalog, Message, MessageLabel, MessagePage, MessageQuery, MessageSummary,
    OutgoingMessage, RemoteContentPolicy, RequestedProtection,
};
use crate::protection::{
    self, CHECK_VALID_DAYS, DiagnosticsAccount, DiagnosticsInput, PROTECTED_SENDING_UNSUPPORTED,
    container_refused, detect, has_reason, headers_not_kept, is_blank, kind_noun, own_label,
    shown_label_name,
};
use crate::providers::{HeaderRequest, MessageContent, MessageLocator};

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

    /// Cached content, or fetched from the server and cached. Content
    /// that's outdated (kept by an older version, or while its properties
    /// couldn't be read) is fetched again, and kept when the server can't
    /// be asked. A message the server no longer has is removed here too:
    /// deletions outside the sync window (of messages server search found)
    /// are noticed so.
    async fn message_content(&self, summary: &MessageSummary) -> Result<MessageContent> {
        let cached = match self.store.cached_message_content(&summary.id)? {
            Some((content, false)) => return Ok(content),
            Some((content, true)) => Some(content),
            None => None,
        };
        let fetched = async {
            let folder = self.store.get_folder(&summary.folder_id)?;
            let providers = self.providers_for(&summary.account_id).await?;
            Ok::<_, Error>(
                providers
                    .mail
                    .fetch_message(&MessageLocator {
                        folder_remote_id: folder.remote_id,
                        remote_id: summary.remote_id.clone(),
                    })
                    .await,
            )
        }
        .await;
        let fetched = match fetched {
            Ok(fetched) => fetched,
            // The server can't be asked (the keyring is locked, say).
            Err(e) => return cached_or(cached, e),
        };
        let content = match fetched {
            Ok(mut content) => {
                let signals = &mut content.signals;
                signals.properties_attempts = if signals.properties_unavailable {
                    cached
                        .as_ref()
                        .map_or(0, |c| c.signals.properties_attempts)
                        .saturating_add(1)
                } else {
                    0
                };
                content
            }
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
            Err(e) => return cached_or(cached, e),
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
        let role = self.store.get_folder(&summary.folder_id)?.role;
        let account = self.store.get_account(&summary.account_id)?;
        // From the raw HTML: a stricter sanitizer mustn't hide the portal.
        let protection = detect(
            &content.signals,
            &content.attachments,
            content.body_html.as_deref(),
            is_blank(content.body_html.as_deref(), content.body_text.as_deref()),
            matches!(role, FolderRole::Sent | FolderRole::Drafts),
        );
        let catalog = self.label_list(&account)?;
        let label = work_tenant(&account)
            .zip(content.signals.msip_labels.as_deref())
            .and_then(|(tenant, raw)| own_label(raw, tenant))
            .and_then(|label| match &catalog {
                // Only the organization's labels, by its own names: the
                // header is plain text any sender can write.
                Some(catalog) => catalog.label(&label.id).map(|listed| MessageLabel {
                    id: label.id.clone(),
                    name: Some(listed.title()),
                    color: listed.color.clone(),
                    from_catalog: true,
                }),
                None => Some(MessageLabel {
                    name: label.name.as_deref().and_then(shown_label_name),
                    id: label.id,
                    color: None,
                    from_catalog: false,
                }),
            });
        let requested = match (role, protection::requested(&content.signals)) {
            (FolderRole::Sent, (action, label)) if action.is_some() || label.is_some() => {
                Some(RequestedProtection {
                    action,
                    label: label.map(|id| catalog_label(catalog.as_ref(), id)),
                })
            }
            _ => None,
        };
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
            protection,
            label,
            requested,
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
        self.attachment_of(summary, content, attachment_id).await
    }

    /// An attachment to open on this computer: [`Self::get_attachment`],
    /// but for the files that only hold a protected message's content,
    /// which nothing here can open.
    pub async fn get_attachment_to_open(
        &self,
        message_id: &str,
        attachment_id: &str,
    ) -> Result<Attachment> {
        let summary = self.store.get_message_summary(message_id)?;
        let content = self.message_content(&summary).await?;
        // Detected again from what's cached: the same files the reading
        // pane leaves out.
        let protection = detect(
            &content.signals,
            &content.attachments,
            content.body_html.as_deref(),
            is_blank(content.body_html.as_deref(), content.body_text.as_deref()),
            false,
        );
        if protection.is_some_and(|p| p.container_ids.iter().any(|id| id == attachment_id)) {
            return Err(container_refused());
        }
        self.attachment_of(summary, content, attachment_id).await
    }

    async fn attachment_of(
        &self,
        summary: MessageSummary,
        content: MessageContent,
        attachment_id: &str,
    ) -> Result<Attachment> {
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
        let account = self.store.get_account(&message.account_id)?;
        let original = match &message.reply_to_message_id {
            Some(id) => Some(self.get_message(id, false).await?),
            None => None,
        };
        // A reply or forward would go out without the original's
        // protection.
        if message.reply_kind.is_some()
            && let Some(original) = &original
            && original
                .protection
                .as_ref()
                .is_some_and(|p| p.kind.is_encrypted())
        {
            return Err(protected_original(&account, &original.summary.id));
        }
        let catalog = self.label_list(&account)?;
        let request = self.protection_request(&account, &message, catalog.as_ref())?;
        let mail = self.providers_for(&message.account_id).await?.mail;
        match &request {
            None => mail.send(&message, original.as_ref()).await?,
            Some(request) => mail
                .send_requesting(&message, original.as_ref(), request)
                .await
                .map_err(|e| reworded(e, &message, request, catalog.as_ref()))?,
        }
        self.notify(&[DataKind::Mail], Some(&message.account_id));
        Ok(())
    }

    /// What a message must ask its organization's mail server for, checked
    /// against the account's setup; `None` for ordinary mail. Refuses
    /// mail without a label when the organization requires one and labels
    /// can be applied from here.
    fn protection_request(
        &self,
        account: &Account,
        message: &OutgoingMessage,
        catalog: Option<&LabelCatalog>,
    ) -> Result<Option<HeaderRequest>> {
        let now = Utc::now();
        let checks = self.store.protection_checks(&account.id)?;
        if message.label_id.is_none()
            && let Some(catalog) = catalog
            && catalog.policy.required
            && catalog
                .labels
                .iter()
                .any(|l| offered(&checks.record(&label_item(&l.id)), now).is_ok())
        {
            return Err(Error::invalid_input(
                "Choose a sensitivity label. Your organization requires one.",
            )
            .with_details(json!({ "reason": "labelRequired" })));
        }
        if message.protect.is_none() && message.label_id.is_none() {
            return Ok(None);
        }
        let Some(tenant) = work_tenant(account) else {
            return Err(Error::unsupported(PROTECTED_SENDING_UNSUPPORTED));
        };
        let email = &account.email;
        let not_set_up = |name: &str, state: CheckState| match state {
            CheckState::Expired => Error::invalid_input(format!(
                "{name} was last checked more than {CHECK_VALID_DAYS} days ago. Check it again in \
                 Settings → Accounts."
            )),
            _ => Error::invalid_input(format!(
                "{name} isn't set up for {email}. Check it in Settings → Accounts."
            )),
        };
        let label = match message.label_id.as_deref() {
            Some(id) => {
                let Some(label) = listed_label(catalog, id) else {
                    return Err(Error::invalid_input(format!(
                        "That sensitivity label isn't available for {email}."
                    )));
                };
                if let Err(state) = offered(&checks.record(&label_item(&label.id)), now) {
                    return Err(not_set_up(&format!("The label {}", label.title()), state));
                }
                Some(label)
            }
            None => None,
        };
        if let Some(action) = message.protect {
            if let Some(label) = label.filter(|l| l.has_protection) {
                return Err(Error::invalid_input(format!(
                    "The label {} already encrypts this message. Choose either the label or {}.",
                    label.title(),
                    action.name()
                )));
            }
            if let Err(state) = offered(&checks.record(&CheckItem::Action { action }), now) {
                return Err(not_set_up(action.name(), state));
            }
        }
        Ok(Some(HeaderRequest {
            protect: message.protect,
            label_id: label.map(|l| l.id.clone()),
            tenant_id: tenant.to_owned(),
            label_encrypts: label.is_some_and(|l| l.has_protection),
        }))
    }

    /// The message's Outlook on the web link, fetched now. Errors:
    /// `Unsupported` (IMAP), `NotFound`, `Provider` (no link).
    pub async fn message_web_link(&self, message_id: &str) -> Result<String> {
        let summary = self.store.get_message_summary(message_id)?;
        let account = self.store.get_account(&summary.account_id)?;
        if account.kind != AccountKind::Microsoft {
            return Err(Error::unsupported(
                "Only Microsoft 365 messages open in Outlook on the web.",
            ));
        }
        let folder = self.store.get_folder(&summary.folder_id)?;
        let link = self
            .providers_for(&summary.account_id)
            .await?
            .mail
            .web_link(&MessageLocator {
                folder_remote_id: folder.remote_id,
                remote_id: summary.remote_id,
            })
            .await?;
        link.ok_or_else(|| Error::provider("Microsoft 365 didn't give a link to this message."))
    }

    /// What the app knows of a message's protection and label, for a bug
    /// report (see [`protection::diagnostics`]): never its content.
    pub async fn message_diagnostics(&self, message_id: &str) -> Result<String> {
        let summary = self.store.get_message_summary(message_id)?;
        let content = self.message_content(&summary).await?;
        let role = self.store.get_folder(&summary.folder_id)?.role;
        let account = self.store.get_account(&summary.account_id)?;
        let protection = detect(
            &content.signals,
            &content.attachments,
            content.body_html.as_deref(),
            is_blank(content.body_html.as_deref(), content.body_text.as_deref()),
            matches!(role, FolderRole::Sent | FolderRole::Drafts),
        );
        let tenant = work_tenant(&account);
        let kind = match account.kind {
            AccountKind::Imap => DiagnosticsAccount::Imap,
            AccountKind::Microsoft if tenant.is_some() => DiagnosticsAccount::MicrosoftWork,
            AccountKind::Microsoft => DiagnosticsAccount::MicrosoftPersonal,
        };
        let catalog = self.label_list(&account)?;
        Ok(protection::diagnostics(&DiagnosticsInput {
            account: kind,
            folder_role: role,
            content: &content,
            protection: protection.as_ref(),
            tenant_id: tenant,
            catalog: catalog.as_ref(),
        }))
    }
}

/// The cached copy of a message that couldn't be fetched again, else why.
fn cached_or(cached: Option<MessageContent>, error: Error) -> Result<MessageContent> {
    match cached {
        Some(content) => {
            tracing::info!(
                "using the cached copy of a message that couldn't be fetched again: {error}"
            );
            Ok(content)
        }
        None => Err(error),
    }
}

fn label_item(id: &str) -> CheckItem {
    CheckItem::Label {
        label_id: id.to_owned(),
    }
}

/// A label a message asked for, named from the account's labels when it's
/// one of them.
fn catalog_label(catalog: Option<&LabelCatalog>, id: String) -> MessageLabel {
    match catalog.and_then(|c| c.label(&id)) {
        Some(listed) => MessageLabel {
            id,
            name: Some(listed.title()),
            color: listed.color.clone(),
            from_catalog: true,
        },
        None => MessageLabel {
            id,
            name: None,
            color: None,
            from_catalog: false,
        },
    }
}

/// The refusal to reply to or forward an encrypted message here.
fn protected_original(account: &Account, message_id: &str) -> Error {
    let message = match account.kind {
        AccountKind::Microsoft => {
            "This message is protected. Reply to it or forward it in Outlook on the web."
        }
        AccountKind::Imap => {
            "This message is encrypted. Cumulas Mail can't reply to it or forward it without \
             removing the encryption."
        }
    };
    Error::unsupported(message).with_details(json!({
        "reason": "protectedOriginal",
        "messageId": message_id,
    }))
}

/// A protected send's failure in the person's words: what the original's
/// copied label is called, and what kind of message wasn't sent.
fn reworded(
    error: Error,
    message: &OutgoingMessage,
    request: &HeaderRequest,
    catalog: Option<&LabelCatalog>,
) -> Error {
    let kind = kind_noun(message.reply_kind);
    if has_reason(&error, "headersNotKept") {
        return headers_not_kept(message.reply_kind);
    }
    if !has_reason(&error, "labelCopied") {
        return error;
    }
    let copied_id = error
        .details
        .as_ref()
        .and_then(|d| d.get("labelId"))
        .and_then(|id| id.as_str())
        .unwrap_or_default()
        .to_owned();
    let copied_title = label_title(catalog, &copied_id);
    let copied = match &copied_title {
        Some(title) => format!("the label {title}"),
        None => "a sensitivity label".to_owned(),
    };
    let text = if request.label_encrypts {
        format!(
            "This {kind} already carries {copied} from the original, so Cumulas Mail can't make \
             sure your organization's mail server encrypts it. Send it from Outlook on the web \
             instead."
        )
    } else {
        let title = request
            .label_id
            .as_deref()
            .and_then(|id| label_title(catalog, id))
            .unwrap_or_else(|| "the label you chose".to_owned());
        let next = match &copied_title {
            Some(copied_title) => {
                format!(" Choose {copied_title}, or send it from Outlook on the web.")
            }
            None => " Send it from Outlook on the web instead.".to_owned(),
        };
        format!(
            "This {kind} already carries {copied} from the original, and Cumulas Mail can't make \
             sure your organization's mail server applies {title} instead.{next}"
        )
    };
    Error::invalid_input(text)
        .with_details(json!({ "reason": "labelCopied", "labelId": copied_id }))
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
