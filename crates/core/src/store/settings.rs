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
            .and_then(|j| serde_json::from_str(&j).ok())
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
    use crate::model::Theme;

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
