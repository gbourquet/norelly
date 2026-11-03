use norelly_domain::account::AccountId;
use norelly_domain::mailbox::MailboxRole;
use norelly_domain::ports::{
    AccountRepository, MailStore, SearchIndex, SearchRequest, SearchScope, StoreError, SyncBatch,
};
use norelly_domain::search::parse_query;
use norelly_test_support::contracts::sample_account;
use norelly_test_support::mail_contracts::{mailbox, sample_message};
use norelly_test_support::search_contracts::search_index_contract;

use crate::mail::SqliteMailStore;
use crate::{Database, SqliteAccountRepository};

async fn two_accounts(db: &Database) -> Result<(AccountId, AccountId), StoreError> {
    let repo = SqliteAccountRepository::new(db.clone());
    let mut ids = Vec::new();
    for address in ["alice@example.com", "bob@example.com"] {
        let account = repo
            .insert(&sample_account(address, false))
            .await
            .map_err(|e| StoreError(e.to_string()))?;
        ids.push(account.id());
    }
    match ids[..] {
        [a, b] => Ok((a, b)),
        _ => Err(StoreError("comptes".into())),
    }
}

#[tokio::test]
async fn search_passes_contract() -> Result<(), StoreError> {
    let db = Database::open_in_memory().map_err(|e| StoreError(e.to_string()))?;
    let (a, b) = two_accounts(&db).await?;
    search_index_contract(&SqliteMailStore::new(db), a, b).await
}

#[tokio::test]
async fn file_search_with_reader_pool_passes_contract() -> Result<(), StoreError> {
    let dir = std::env::temp_dir().join(format!("norelly-sqlite-search-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| StoreError(e.to_string()))?;
    let db = Database::open(&dir.join("test.sqlite3")).map_err(|e| StoreError(e.to_string()))?;
    let (a, b) = two_accounts(&db).await?;
    search_index_contract(&SqliteMailStore::new(db.clone()), a, b).await?;
    assert_eq!(db.integrity_problems().await?, Vec::<String>::new());
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

fn request(text: &str) -> SearchRequest {
    SearchRequest {
        query: parse_query(text).unwrap_or_default(),
        scope: SearchScope::All,
        utc_offset_seconds: 0,
        offset: 0,
        limit: 10,
    }
}

/// Base avec un compte, une boîte et le message `sample_message(1)`
/// (« Sujet 1 »).
async fn one_message() -> Result<(Database, SqliteMailStore), StoreError> {
    let db = Database::open_in_memory().map_err(|e| StoreError(e.to_string()))?;
    let (a, _) = two_accounts(&db).await?;
    let store = SqliteMailStore::new(db.clone());
    let boxes = store
        .replace_mailboxes(a, &[mailbox("INBOX", MailboxRole::Inbox, true)])
        .await?;
    let inbox = boxes
        .first()
        .map(|m| m.id)
        .ok_or_else(|| StoreError("boîte".into()))?;
    store
        .apply_sync(
            inbox,
            &SyncBatch {
                new: vec![sample_message(1, 100, false)],
                ..SyncBatch::default()
            },
        )
        .await?;
    Ok((db, store))
}

#[tokio::test]
async fn messages_hidden_by_a_pending_action_are_not_found() -> Result<(), StoreError> {
    let (db, store) = one_message().await?;
    assert_eq!(store.search(request("sujet")).await?.items.len(), 1);
    db.write_for_tests("UPDATE messages SET hidden = 1").await?;
    assert!(store.search(request("sujet")).await?.items.is_empty());
    Ok(())
}

#[tokio::test]
async fn a_desynchronized_index_is_reported() -> Result<(), StoreError> {
    let (db, _store) = one_message().await?;
    assert_eq!(db.integrity_problems().await?, Vec::<String>::new());
    // Entrée retirée de l'index sans passer par la table de contenu.
    db.write_for_tests(
        "INSERT INTO message_search (message_search, rowid, subject, sender, recipients, body) \
         SELECT 'delete', message_id, subject, sender, recipients, body FROM message_texts",
    )
    .await?;
    let problems = db.integrity_problems().await?;
    assert!(
        problems.iter().any(|p| p.contains("recherche")),
        "{problems:?}"
    );
    Ok(())
}
