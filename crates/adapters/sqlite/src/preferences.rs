//! Préférences de lecture ([`ReadingPreferences`]) : expéditeurs dont les
//! images distantes sont autorisées, préférence pour le texte brut.

use norelly_domain::address::EmailAddress;
use norelly_domain::ports::{ReadingPreferences, StoreError};
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

#[cfg(test)]
mod tests {
    use norelly_test_support::reading_contracts::reading_preferences_contract;

    use super::*;

    #[tokio::test]
    async fn sqlite_preferences_pass_contract() -> Result<(), StoreError> {
        let db = Database::open_in_memory().map_err(|e| StoreError(e.to_string()))?;
        reading_preferences_contract(&SqliteReadingPreferences::new(db)).await
    }
}
