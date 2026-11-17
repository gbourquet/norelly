//! Groupes de doublons écartés du carnet (ADR-012) : `DuplicateDismissals`,
//! sur la base du carnet ([`SqliteContactBook`]).
//!
//! L'oubli d'une paire quand l'une de ses fiches change ou disparaît est
//! garanti par le schéma (migration 22 : clés étrangères en cascade et
//! déclencheur sur `contacts`), quel que soit le chemin d'écriture.

use norelly_domain::contact_duplicates::{DismissedPair, MAX_DISMISSED_PAIRS};
use norelly_domain::contacts::ContactId;
use norelly_domain::ports::{DuplicateDismissals, StoreError};
use rusqlite::params;

use crate::contacts::SqliteContactBook;
use crate::store_error;

/// Retient une paire dont les deux fiches sont dans le carnet ; une paire
/// déjà retenue est réinsérée, donc la plus récente.
const DISMISS: &str = "INSERT OR REPLACE INTO contact_duplicate_dismissals (low_id, high_id) \
     SELECT ?1, ?2 WHERE EXISTS (SELECT 1 FROM contacts WHERE id = ?1) \
       AND EXISTS (SELECT 1 FROM contacts WHERE id = ?2)";
/// Oublie les plus anciennes paires au-delà des `?1` plus récentes.
const FORGET_OLDEST: &str = "DELETE FROM contact_duplicate_dismissals WHERE rowid <= \
     (SELECT rowid FROM contact_duplicate_dismissals ORDER BY rowid DESC LIMIT 1 OFFSET ?1)";
const DISMISSED: &str = "SELECT low_id, high_id FROM contact_duplicate_dismissals \
     ORDER BY rowid DESC LIMIT ?1";

/// Borne en paramètre SQL.
fn bound() -> Result<i64, StoreError> {
    i64::try_from(MAX_DISMISSED_PAIRS).map_err(|e| StoreError(e.to_string()))
}

impl DuplicateDismissals for SqliteContactBook {
    async fn dismiss(&self, pairs: &[DismissedPair]) -> Result<(), StoreError> {
        if pairs.is_empty() {
            return Ok(());
        }
        let pairs = pairs.to_vec();
        let bound = bound()?;
        self.db
            .run(move |c| {
                let tx = c.transaction().map_err(store_error)?;
                {
                    let mut insert = tx.prepare_cached(DISMISS).map_err(store_error)?;
                    for p in &pairs {
                        insert
                            .execute(params![p.low().get(), p.high().get()])
                            .map_err(store_error)?;
                    }
                }
                tx.execute(FORGET_OLDEST, params![bound])
                    .map_err(store_error)?;
                tx.commit().map_err(store_error)
            })
            .await
    }

    async fn dismissed(&self) -> Result<Vec<DismissedPair>, StoreError> {
        let bound = bound()?;
        self.db
            .read(move |c| {
                let mut select = c.prepare_cached(DISMISSED).map_err(store_error)?;
                let rows = select
                    .query_map(params![bound], |row| {
                        Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
                    })
                    .map_err(store_error)?;
                let mut pairs = Vec::new();
                for row in rows {
                    let (low, high) = row.map_err(store_error)?;
                    // Une ligne invalide (identifiant hors bornes) est
                    // ignorée : au pire, un groupe est proposé de nouveau.
                    if let (Some(low), Some(high)) = (ContactId::new(low), ContactId::new(high))
                        && let Some(pair) = DismissedPair::new(low, high)
                    {
                        pairs.push(pair);
                    }
                }
                Ok(pairs)
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use norelly_domain::ports::ContactStoreError;
    use norelly_test_support::contact_dismissals_contracts::{
        duplicate_dismissals_bound_contract, duplicate_dismissals_contract,
    };

    use super::*;
    use crate::Database;

    fn book() -> SqliteContactBook {
        SqliteContactBook::new(Database::open_in_memory().unwrap_or_else(|e| panic!("{e}")))
    }

    #[tokio::test]
    async fn dismissals_pass_contract() -> Result<(), ContactStoreError> {
        duplicate_dismissals_contract(&book()).await
    }

    #[tokio::test]
    async fn dismissals_pass_bound_contract() -> Result<(), ContactStoreError> {
        duplicate_dismissals_bound_contract(&book()).await
    }
}
