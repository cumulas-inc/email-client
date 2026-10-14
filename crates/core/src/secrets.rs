//! Storage for passwords and OAuth refresh tokens.
//!
//! On Linux desktops secrets go to the Secret Service (GNOME Keyring, KWallet)
//! via `oo7`, which transparently switches to the XDG secret portal inside a
//! Flatpak sandbox. Nothing secret is ever written to the SQLite database.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::error::{Error, ErrorCode, Result};

/// Attribute identifying our items in the keyring.
const APP_ATTRIBUTE: &str = "co.cumulas.Mail";

#[async_trait]
pub trait SecretStore: Send + Sync {
    async fn get(&self, key: &str) -> Result<Option<String>>;
    async fn set(&self, key: &str, value: &str) -> Result<()>;
    async fn delete(&self, key: &str) -> Result<()>;
}

/// Well-known secret keys.
pub mod keys {
    pub fn imap_password(account_id: &str) -> String {
        format!("account:{account_id}:password")
    }

    pub fn refresh_token(account_id: &str) -> String {
        format!("account:{account_id}:refresh-token")
    }
}

/// Secret Service / Flatpak portal backed store.
pub struct KeyringSecretStore {
    keyring: oo7::Keyring,
}

impl KeyringSecretStore {
    pub async fn new() -> Result<Self> {
        let keyring = oo7::Keyring::new().await.map_err(keyring_error)?;
        Ok(Self { keyring })
    }

    fn attributes(key: &str) -> HashMap<&str, &str> {
        HashMap::from([("application", APP_ATTRIBUTE), ("key", key)])
    }
}

fn keyring_error(e: oo7::Error) -> Error {
    Error::new(
        ErrorCode::Storage,
        format!("could not access the system keyring: {e}"),
    )
}

#[async_trait]
impl SecretStore for KeyringSecretStore {
    async fn get(&self, key: &str) -> Result<Option<String>> {
        self.keyring.unlock().await.map_err(keyring_error)?;
        let items = self
            .keyring
            .search_items(&Self::attributes(key))
            .await
            .map_err(keyring_error)?;
        let Some(item) = items.first() else {
            return Ok(None);
        };
        let secret = item.secret().await.map_err(keyring_error)?;
        let value = String::from_utf8(secret.to_vec())
            .map_err(|_| Error::internal("stored secret is not valid UTF-8"))?;
        Ok(Some(value))
    }

    async fn set(&self, key: &str, value: &str) -> Result<()> {
        self.keyring.unlock().await.map_err(keyring_error)?;
        self.keyring
            .create_item(
                &format!("Cumulas Mail ({key})"),
                &Self::attributes(key),
                value,
                true,
            )
            .await
            .map_err(keyring_error)
    }

    async fn delete(&self, key: &str) -> Result<()> {
        self.keyring
            .delete(&Self::attributes(key))
            .await
            .map_err(keyring_error)
    }
}

/// In-memory store for tests and for environments without a keyring.
#[derive(Default)]
pub struct MemorySecretStore {
    values: Mutex<HashMap<String, String>>,
}

#[async_trait]
impl SecretStore for MemorySecretStore {
    async fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.values.lock().unwrap().get(key).cloned())
    }

    async fn set(&self, key: &str, value: &str) -> Result<()> {
        self.values
            .lock()
            .unwrap()
            .insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<()> {
        self.values.lock().unwrap().remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_store_round_trips() {
        let store = MemorySecretStore::default();
        assert_eq!(store.get("k").await.unwrap(), None);
        store.set("k", "v").await.unwrap();
        assert_eq!(store.get("k").await.unwrap().as_deref(), Some("v"));
        store.delete("k").await.unwrap();
        assert_eq!(store.get("k").await.unwrap(), None);
    }
}
