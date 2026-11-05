//! Préférences de lecture ([`ReadingPreferences`]) : expéditeurs dont les
//! images distantes sont autorisées, préférence pour le texte brut.

use norelly_domain::address::EmailAddress;
use norelly_domain::ports::{ReadingPreferences, SettingsStore, StoreError};
use norelly_domain::settings::{Language, Settings, UpdateCheck};
use rusqlite::{OptionalExtension, params};

use crate::{Database, store_error};

/// Clé du réglage « texte brut ».
const PLAIN_TEXT_KEY: &str = "reading.prefer_plain_text";

/// Préférences de lecture en base.
#[derive(Debug, Clone)]
pub struct SqliteReadingPreferences {
    db: Database,
}

impl SqliteReadingPreferences {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

impl ReadingPreferences for SqliteReadingPreferences {
    async fn remote_images_allowed(&self, sender: &EmailAddress) -> Result<bool, StoreError> {
        let address = sender.to_string();
        self.db
            .read(move |c| {
                c.query_row(
                    "SELECT 1 FROM remote_image_senders WHERE address = ?1",
                    [address],
                    |_| Ok(()),
                )
                .optional()
                .map(|found| found.is_some())
                .map_err(store_error)
            })
            .await
    }

    async fn allow_remote_images(&self, sender: &EmailAddress) -> Result<(), StoreError> {
        let address = sender.to_string();
        self.db
            .run(move |c| {
                c.execute(
                    "INSERT OR IGNORE INTO remote_image_senders (address) VALUES (?1)",
                    [address],
                )
                .map(|_| ())
                .map_err(store_error)
            })
            .await
    }

    async fn revoke_remote_images(&self, sender: &EmailAddress) -> Result<(), StoreError> {
        let address = sender.to_string();
        self.db
            .run(move |c| {
                c.execute(
                    "DELETE FROM remote_image_senders WHERE address = ?1",
                    [address],
                )
                .map(|_| ())
                .map_err(store_error)
            })
            .await
    }

    async fn prefer_plain_text(&self) -> Result<bool, StoreError> {
        self.db
            .read(|c| {
                let value: Option<String> = c
                    .query_row(
                        "SELECT value FROM settings WHERE key = ?1",
                        [PLAIN_TEXT_KEY],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(store_error)?;
                // Valeur inconnue : réglage par défaut.
                Ok(value.as_deref() == Some("1"))
            })
            .await
    }

    async fn set_prefer_plain_text(&self, prefer: bool) -> Result<(), StoreError> {
        self.db
            .run(move |c| {
                c.execute(
                    "INSERT INTO settings (key, value) VALUES (?1, ?2) \
                     ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                    params![PLAIN_TEXT_KEY, if prefer { "1" } else { "0" }],
                )
                .map(|_| ())
                .map_err(store_error)
            })
            .await
    }
}

// --- Paramètres de l'application ---

const LANGUAGE_KEY: &str = "app.language";
const UPDATE_KEY: &str = "app.update_check";
const NOTIFY_KEY: &str = "notify.enabled";
const PRIVACY_KEY: &str = "notify.privacy";
const THRESHOLD_KEY: &str = "fetch.attachment_threshold";

const fn flag(value: bool) -> &'static str {
    if value { "1" } else { "0" }
}

/// Paramètres relus : chaque valeur absente, illisible ou hors bornes
/// reprend sa valeur par défaut (base modifiée à la main).
fn settings_of(rows: &[(String, String)]) -> Settings {
    let defaults = Settings::default();
    let value = |key: &str| rows.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
    let flag_of = |key: &str, default: bool| match value(key) {
        Some("1") => true,
        Some("0") => false,
        _ => default,
    };
    let mut settings = Settings {
        language: value(LANGUAGE_KEY)
            .and_then(Language::parse)
            .unwrap_or(defaults.language),
        update_check: value(UPDATE_KEY)
            .and_then(UpdateCheck::parse)
            .unwrap_or(defaults.update_check),
        notifications: flag_of(NOTIFY_KEY, defaults.notifications),
        // Présente mais illisible : la confidentialité s'applique (échec
        // fermé sur une option de vie privée).
        notification_privacy: match value(PRIVACY_KEY) {
            None => defaults.notification_privacy,
            Some(v) => v != "0",
        },
        attachment_threshold: value(THRESHOLD_KEY)
            .and_then(|v| v.parse().ok())
            .unwrap_or(defaults.attachment_threshold),
    };
    if settings.validate().is_err() {
        settings.attachment_threshold = defaults.attachment_threshold;
    }
    settings
}

/// Paramètres de l'application en base (table `settings`, une clé par
/// réglage).
#[derive(Debug, Clone)]
pub struct SqliteSettingsStore {
    db: Database,
}

impl SqliteSettingsStore {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

impl SettingsStore for SqliteSettingsStore {
    async fn settings(&self) -> Result<Settings, StoreError> {
        self.db
            .read(|c| {
                let mut stmt = c
                    .prepare_cached(
                        "SELECT key, value FROM settings WHERE key LIKE 'app.%' \
                                     OR key LIKE 'notify.%' OR key LIKE 'fetch.%'",
                    )
                    .map_err(store_error)?;
                let rows: Vec<(String, String)> = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .map_err(store_error)?
                    .collect::<Result<_, _>>()
                    .map_err(store_error)?;
                Ok(settings_of(&rows))
            })
            .await
    }

    async fn save_settings(&self, settings: &Settings) -> Result<(), StoreError> {
        settings.validate().map_err(|e| StoreError(e.to_string()))?;
        let values = [
            (LANGUAGE_KEY, settings.language.code().to_owned()),
            (UPDATE_KEY, settings.update_check.code().to_owned()),
            (NOTIFY_KEY, flag(settings.notifications).to_owned()),
            (PRIVACY_KEY, flag(settings.notification_privacy).to_owned()),
            (THRESHOLD_KEY, settings.attachment_threshold.to_string()),
        ];
        self.db
            .run(move |c| {
                let tx = c.transaction().map_err(store_error)?;
                {
                    let mut stmt = tx
                        .prepare_cached(
                            "INSERT INTO settings (key, value) VALUES (?1, ?2) \
                             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                        )
                        .map_err(store_error)?;
                    for (key, value) in &values {
                        stmt.execute(params![key, value]).map_err(store_error)?;
                    }
                }
                tx.commit().map_err(store_error)
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use norelly_test_support::reading_contracts::{
        reading_preferences_contract, settings_store_contract,
    };

    use super::*;

    #[tokio::test]
    async fn sqlite_preferences_pass_contract() -> Result<(), StoreError> {
        let db = Database::open_in_memory().map_err(|e| StoreError(e.to_string()))?;
        reading_preferences_contract(&SqliteReadingPreferences::new(db)).await
    }

    #[tokio::test]
    async fn sqlite_settings_pass_contract() -> Result<(), StoreError> {
        let db = Database::open_in_memory().map_err(|e| StoreError(e.to_string()))?;
        settings_store_contract(&SqliteSettingsStore::new(db)).await
    }

    /// Base modifiée à la main : une valeur illisible ou hors bornes
    /// reprend sa valeur par défaut, les autres sont gardées.
    #[tokio::test]
    async fn tampered_settings_fall_back_to_defaults() -> Result<(), StoreError> {
        let db = Database::open_in_memory().map_err(|e| StoreError(e.to_string()))?;
        let store = SqliteSettingsStore::new(db.clone());
        let chosen = Settings {
            notifications: false,
            ..Settings::default()
        };
        store.save_settings(&chosen).await?;
        db.write_for_tests(
            "UPDATE settings SET value = 'klingon' WHERE key = 'app.language'; \
             UPDATE settings SET value = '1' WHERE key = 'fetch.attachment_threshold';",
        )
        .await?;
        assert_eq!(store.settings().await?, chosen);
        Ok(())
    }

    /// Revue de la phase 9 (BASSE) : une option de confidentialité
    /// illisible s'applique (échec fermé), les autres réglages illisibles
    /// reprennent leur valeur par défaut.
    #[tokio::test]
    async fn unreadable_privacy_fails_closed() -> Result<(), StoreError> {
        let db = Database::open_in_memory().map_err(|e| StoreError(e.to_string()))?;
        let store = SqliteSettingsStore::new(db.clone());
        store.save_settings(&Settings::default()).await?;
        db.write_for_tests("UPDATE settings SET value = 'oui' WHERE key = 'notify.privacy'")
            .await?;
        assert!(store.settings().await?.notification_privacy);
        Ok(())
    }
}
