//! Storage for passwords and OAuth refresh tokens.
//!
//! On Linux desktops secrets go to the Secret Service (GNOME Keyring, KWallet)
//! via `oo7`, which transparently switches to the XDG secret portal inside a
//! Flatpak sandbox. Nothing secret is ever written to the SQLite database.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, Weak};

use async_trait::async_trait;
use tokio::sync::OwnedMutexGuard;

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

/// Each account's sign-in lock, while anyone holds or waits for it.
static ACCOUNT_LOCKS: LazyLock<Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(Mutex::default);

/// Locks the account's stored sign-in, for everything in the app that
/// replaces or deletes it: a refresh token Microsoft rotated, compared with
/// the one redeemed and saved; a new sign-in's, saved; and the account's
/// secrets, deleted when it's removed. Each reads what it needs and writes
/// under the lock, so none undoes another: no refresh token is ever left
/// for an account that was removed. A refresh also reads the refresh token
/// and the grant under it, so it never pairs a sign-in's token with the
/// grant before. Held only around those secret store calls, never across a
/// request to Microsoft. One per account id in the process, as the keyring
/// is.
pub(crate) async fn lock_account(account_id: &str) -> OwnedMutexGuard<()> {
    let lock = {
        let mut locks = ACCOUNT_LOCKS.lock().unwrap();
        locks.retain(|_, lock| lock.strong_count() > 0);
        match locks.get(account_id).and_then(Weak::upgrade) {
            Some(lock) => lock,
            None => {
                let lock = Arc::new(tokio::sync::Mutex::new(()));
                locks.insert(account_id.to_owned(), Arc::downgrade(&lock));
                lock
            }
        }
    };
    lock.lock_owned().await
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

impl MemorySecretStore {
    /// Stores `value` under `key` at once: no keyring to wait for.
    pub fn put(&self, key: &str, value: &str) {
        self.values
            .lock()
            .unwrap()
            .insert(key.to_owned(), value.to_owned());
    }
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

    #[tokio::test(start_paused = true)]
    async fn an_account_lock_is_one_per_account() {
        let held = lock_account("lock-a").await;
        // Another account's isn't held back...
        drop(lock_account("lock-b").await);
        // ...the same account's is, until it's released.
        let waiting = tokio::spawn(async { drop(lock_account("lock-a").await) });
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        assert!(!waiting.is_finished());
        drop(held);
        waiting.await.unwrap();
        // Then neither is kept.
        let locks = ACCOUNT_LOCKS.lock().unwrap();
        for id in ["lock-a", "lock-b"] {
            assert!(locks.get(id).is_none_or(|lock| lock.strong_count() == 0));
        }
    }

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
