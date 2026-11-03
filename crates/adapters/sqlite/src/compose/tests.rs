use norelly_domain::ports::AccountRepository;
use norelly_test_support::compose_contracts::{
    contact_query_contract, draft_store_contract, outbox_store_contract,
};
use norelly_test_support::contracts::sample_account;

use super::*;
use crate::SqliteAccountRepository;

async fn setup() -> Result<(SqliteComposeStore, Database, AccountId, AccountId), StoreError> {
    let db = Database::open_in_memory().map_err(|e| StoreError(e.to_string()))?;
    let repo = SqliteAccountRepository::new(db.clone());
    let a = repo
        .insert(&sample_account("alice@example.com", false))
        .await
        .map_err(|e| StoreError(e.to_string()))?;
    let b = repo
        .insert(&sample_account("bob@example.com", false))
        .await
        .map_err(|e| StoreError(e.to_string()))?;
    Ok((SqliteComposeStore::new(db.clone()), db, a.id(), b.id()))
}

#[tokio::test]
async fn drafts_pass_contract() -> Result<(), StoreError> {
    let (store, db, a, b) = setup().await?;
    draft_store_contract(&store, a, b).await?;
    assert_eq!(db.integrity_problems().await?, Vec::<String>::new());
    Ok(())
}

#[tokio::test]
async fn outbox_passes_contract() -> Result<(), StoreError> {
    let (store, db, a, b) = setup().await?;
    outbox_store_contract(&store, a, b).await?;
    assert_eq!(db.integrity_problems().await?, Vec::<String>::new());
    Ok(())
}

#[tokio::test]
async fn contacts_pass_contract() -> Result<(), StoreError> {
    let (_, db, a, b) = setup().await?;
    contact_query_contract(&SqliteMailStore::new(db), a, b).await
}
