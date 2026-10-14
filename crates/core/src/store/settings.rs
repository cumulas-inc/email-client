use rusqlite::{OptionalExtension, params};

use super::Store;
use crate::error::Result;
use crate::model::Settings;

const SETTINGS_KEY: &str = "settings";

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
