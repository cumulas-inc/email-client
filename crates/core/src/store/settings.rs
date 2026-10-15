use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use rusqlite::{OptionalExtension, params};

use super::Store;
use crate::error::Result;
use crate::model::Settings;

const SETTINGS_KEY: &str = "settings";
/// Identifies this installation to the push service. Random and not a secret.
const INSTALLATION_ID_KEY: &str = "installation_id";

impl Store {
    pub fn get_settings(&self) -> Result<Settings> {
        let json: Option<String> = self
            .conn()
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                [SETTINGS_KEY],
                |r| r.get(0),
            )
            .optional()?;
        Ok(json
            .map(|j| Settings::from_stored_json(&j))
            .unwrap_or_default())
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        self.conn().execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![SETTINGS_KEY, super::to_json(settings)],
        )?;
        Ok(())
    }

    /// A small value the app keeps between runs that isn't a setting, such
    /// as whether a one-time notice was shown. Callers prefix their keys
    /// (`shell.` for the desktop shell) so they never meet the core's own.
    pub fn app_value(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                [key],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn set_app_value(&self, key: &str, value: &str) -> Result<()> {
        if matches!(key, SETTINGS_KEY | INSTALLATION_ID_KEY) {
            return Err(crate::error::Error::invalid_input(format!(
                "{key} is not an app value"
            )));
        }
        self.conn().execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// The id of this installation: 22 URL-safe characters (128 random
    /// bits), made the first time it is asked for and kept from then on.
    pub fn installation_id(&self) -> Result<String> {
        let mut bytes = [0u8; 16];
        rand::rng().fill_bytes(&mut bytes);
        let conn = self.conn();
        // Keeps the stored id when there is one.
        conn.execute(
            "INSERT OR IGNORE INTO app_settings (key, value) VALUES (?1, ?2)",
            params![INSTALLATION_ID_KEY, URL_SAFE_NO_PAD.encode(bytes)],
        )?;
        Ok(conn.query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [INSTALLATION_ID_KEY],
            |r| r.get(0),
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{InterfaceFont, ReadingFont, TextSize, Theme};

    #[test]
    fn settings_saved_by_an_older_version_get_the_new_defaults() {
        let store = Store::open_in_memory().unwrap();
        store
            .conn()
            .execute(
                "INSERT INTO app_settings (key, value) VALUES (?1, ?2)",
                params![
                    SETTINGS_KEY,
                    r#"{"theme":"dark","syncIntervalMinutes":30,"notificationsEnabled":false}"#
                ],
            )
            .unwrap();
        let settings = store.get_settings().unwrap();
        assert_eq!(settings.theme, Theme::Dark);
        assert_eq!(settings.sync_interval_minutes, 30);
        assert!(!settings.notifications_enabled);
        assert_eq!(settings.text_size, TextSize::Default);
        assert_eq!(settings.interface_font, InterfaceFont::Default);
    }

    #[test]
    fn appearance_settings_persist() {
        let store = Store::open_in_memory().unwrap();
        let settings = Settings {
            interface_font: InterfaceFont::System,
            reading_font: ReadingFont::Family {
                family: "Literata".into(),
            },
            text_size: TextSize::Largest,
            ..Settings::default()
        };
        store.save_settings(&settings).unwrap();
        assert_eq!(store.get_settings().unwrap(), settings);
    }

    #[test]
    fn installation_id_is_made_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail.db");
        let id = Store::open(&path).unwrap().installation_id().unwrap();
        assert_eq!(id.len(), 22);
        assert!(
            id.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "{id}"
        );
        let reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.installation_id().unwrap(), id);
        assert_eq!(reopened.installation_id().unwrap(), id);
        // Saving settings leaves it alone.
        reopened.save_settings(&Settings::default()).unwrap();
        assert_eq!(reopened.installation_id().unwrap(), id);
        assert_ne!(
            Store::open_in_memory().unwrap().installation_id().unwrap(),
            id
        );
    }

    #[test]
    fn app_values_persist_and_leave_settings_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail.db");
        let store = Store::open(&path).unwrap();
        assert_eq!(store.app_value("shell.notice").unwrap(), None);
        store.set_app_value("shell.notice", "shown").unwrap();
        store.set_app_value("shell.notice", "shown again").unwrap();
        let reopened = Store::open(&path).unwrap();
        assert_eq!(
            reopened.app_value("shell.notice").unwrap().as_deref(),
            Some("shown again")
        );
        assert!(reopened.set_app_value(SETTINGS_KEY, "{}").is_err());
        assert!(reopened.set_app_value(INSTALLATION_ID_KEY, "x").is_err());
        assert_eq!(reopened.get_settings().unwrap(), Settings::default());
    }

    #[test]
    fn settings_default_then_persist() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.get_settings().unwrap(), Settings::default());
        let settings = Settings {
            theme: Theme::Dark,
            ..Settings::default()
        };
        store.save_settings(&settings).unwrap();
        assert_eq!(store.get_settings().unwrap().theme, Theme::Dark);
    }
}
