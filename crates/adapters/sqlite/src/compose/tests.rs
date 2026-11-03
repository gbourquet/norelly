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

/// Brouillon modifié à la main dans la base (sujet avec un saut de ligne,
/// injection d'en-tête) : relu comme une donnée corrompue, jamais accepté.
#[tokio::test]
async fn hand_modified_draft_is_revalidated_on_read() -> Result<(), StoreError> {
    let (store, db, a, _) = setup().await?;
    let mut draft = Draft::empty(a);
    draft.subject = "Bonjour".into();
    let stored = store.save_draft(None, &draft, 100).await?;
    assert!(store.draft(stored.id).await?.is_some());
    db.write_for_tests(
        "UPDATE drafts SET content = json_set(content, '$.subject', 'a' || char(13, 10) || 'Bcc: x@y')",
    )
    .await?;
    assert!(
        store.draft(stored.id).await.is_err(),
        "brouillon relu sans revalidation"
    );
    assert!(store.drafts(a).await.is_err());
    Ok(())
}
