//! État de la synchronisation CardDAV (ADR-014) : [`CardDavStore`], sur la
//! base du carnet (migration 24).
//!
//! Les dates de modification des contacts et de suppression d'un contact
//! relié sont posées par des déclencheurs : elles valent quel que soit le
//! chemin d'écriture du carnet.

use norelly_domain::carddav::{
    AddressBook, CardDavAccount, DavUrl, DavUsername, ETag, Href, LinkState, SyncFailure, SyncLink,
    SyncReport,
};
use norelly_domain::contacts::ContactId;
use norelly_domain::ports::{CardDavStore, CardDavSyncRecord, StoreError};
use rusqlite::{OptionalExtension, params};

use crate::{Database, store_error};

const ACCOUNT: &str =
    "SELECT server, username, book_url, book_name FROM carddav_account WHERE id = 1";
const CURRENT_BOOK: &str = "SELECT server, book_url FROM carddav_account WHERE id = 1";
const SAVE_ACCOUNT: &str = "INSERT OR REPLACE INTO carddav_account \
     (id, server, username, book_url, book_name) VALUES (1, ?1, ?2, ?3, ?4)";
const LINKS: &str =
    "SELECT href, etag, contact_id, deleted_at, synced, remote FROM carddav_links ORDER BY href";
const SAVE_LINK: &str = "INSERT INTO carddav_links \
     (href, etag, contact_id, deleted_at, synced, remote) VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
     ON CONFLICT (href) DO UPDATE SET etag = excluded.etag, contact_id = excluded.contact_id, \
     deleted_at = excluded.deleted_at, synced = excluded.synced, remote = excluded.remote";
const FORGET_LINK: &str = "DELETE FROM carddav_links WHERE href = ?1";
const MODIFICATION_TIMES: &str = "SELECT id, modified_at FROM contacts";
const SAVE_SYNC: &str = "INSERT OR REPLACE INTO carddav_sync \
     (id, at, pulled, pushed, skipped, failure) VALUES (1, ?1, ?2, ?3, ?4, ?5)";
const LAST_SYNC: &str =
    "SELECT at, pulled, pushed, skipped, failure FROM carddav_sync WHERE id = 1";

/// État de la synchronisation CardDAV.
#[derive(Debug, Clone)]
pub struct SqliteCardDavStore {
    db: Database,
}

impl SqliteCardDavStore {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

/// Compte relu, revalidé par le domaine.
fn account_of(
    server: &str,
    username: &str,
    book_url: &str,
    book_name: &str,
) -> Result<CardDavAccount, StoreError> {
    let invalid = |e: &dyn std::fmt::Display| StoreError(format!("compte CardDAV invalide : {e}"));
    let server = DavUrl::parse(server).map_err(|e| invalid(&e))?;
    let username = DavUsername::parse(username).map_err(|e| invalid(&e))?;
    let book = DavUrl::parse(book_url).map_err(|e| invalid(&e))?;
    CardDavAccount::new(server, username, AddressBook::new(book, book_name))
        .map_err(|e| invalid(&e))
}

fn count(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

fn uncount(n: i64) -> usize {
    usize::try_from(n).unwrap_or(0)
}

type LinkRow = (String, String, Option<i64>, Option<i64>, String, String);

/// Ressource relue ; `None` si elle est invalide (elle sera revue comme
/// nouvelle).
fn link_of((href, etag, contact, deleted_at, synced, remote): LinkRow) -> Option<SyncLink> {
    let href = Href::parse(&href).ok()?;
    let etag = ETag::parse(&etag)?;
    let state = match (contact, deleted_at) {
        (Some(id), _) => LinkState::Linked(ContactId::new(id)?),
        (None, Some(at)) => LinkState::Deleted { at },
        (None, None) => LinkState::Ignored,
    };
    Some(SyncLink {
        href,
        etag,
        state,
        synced,
        remote,
    })
}

impl CardDavStore for SqliteCardDavStore {
    async fn account(&self) -> Result<Option<CardDavAccount>, StoreError> {
        let row = self
            .db
            .read(|c| {
                c.query_row(ACCOUNT, [], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .optional()
                .map_err(store_error)
            })
            .await?;
        row.map(|(server, username, book, name)| account_of(&server, &username, &book, &name))
            .transpose()
    }

    async fn save_account(&self, account: &CardDavAccount) -> Result<(), StoreError> {
        let server = account.server().to_string();
        let username = account.username().as_str().to_owned();
        let book = account.book().url().to_string();
        let name = account.book().name().to_owned();
        self.db
            .run(move |c| {
                let tx = c.transaction().map_err(store_error)?;
                let current: Option<(String, String)> = tx
                    .query_row(CURRENT_BOOK, [], |row| Ok((row.get(0)?, row.get(1)?)))
                    .optional()
                    .map_err(store_error)?;
                if current.as_ref() != Some(&(server.clone(), book.clone())) {
                    tx.execute("DELETE FROM carddav_links", [])
                        .map_err(store_error)?;
                    tx.execute("DELETE FROM carddav_sync", [])
                        .map_err(store_error)?;
                }
                tx.execute(SAVE_ACCOUNT, params![server, username, book, name])
                    .map_err(store_error)?;
                tx.commit().map_err(store_error)
            })
            .await
    }

    async fn remove_account(&self) -> Result<(), StoreError> {
        self.db
            .run(|c| {
                let tx = c.transaction().map_err(store_error)?;
                tx.execute("DELETE FROM carddav_links", [])
                    .map_err(store_error)?;
                tx.execute("DELETE FROM carddav_sync", [])
                    .map_err(store_error)?;
                tx.execute("DELETE FROM carddav_account", [])
                    .map_err(store_error)?;
                tx.commit().map_err(store_error)
            })
            .await
    }

    async fn links(&self) -> Result<Vec<SyncLink>, StoreError> {
        self.db
            .read(|c| {
                let mut select = c.prepare_cached(LINKS).map_err(store_error)?;
                let rows = select
                    .query_map([], |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                        ))
                    })
                    .map_err(store_error)?;
                let mut links = Vec::new();
                for row in rows {
                    if let Some(link) = link_of(row.map_err(store_error)?) {
                        links.push(link);
                    }
                }
                Ok(links)
            })
            .await
    }

    async fn save_link(&self, link: &SyncLink) -> Result<(), StoreError> {
        let (contact, deleted_at) = match link.state {
            LinkState::Linked(id) => (Some(id.get()), None),
            LinkState::Deleted { at } => (None, Some(at)),
            LinkState::Ignored => (None, None),
        };
        let link = link.clone();
        self.db
            .run(move |c| {
                c.execute(
                    SAVE_LINK,
                    params![
                        link.href.as_str(),
                        link.etag.as_str(),
                        contact,
                        deleted_at,
                        link.synced,
                        link.remote
                    ],
                )
                .map_err(store_error)?;
                Ok(())
            })
            .await
    }

    async fn forget_link(&self, href: &Href) -> Result<(), StoreError> {
        let href = href.as_str().to_owned();
        self.db
            .run(move |c| {
                c.execute(FORGET_LINK, params![href]).map_err(store_error)?;
                Ok(())
            })
            .await
    }

    async fn modification_times(&self) -> Result<Vec<(ContactId, i64)>, StoreError> {
        self.db
            .read(|c| {
                let mut select = c.prepare_cached(MODIFICATION_TIMES).map_err(store_error)?;
                let rows = select
                    .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
                    .map_err(store_error)?;
                let mut times = Vec::new();
                for row in rows {
                    let (id, at) = row.map_err(store_error)?;
                    if let Some(id) = ContactId::new(id) {
                        times.push((id, at));
                    }
                }
                Ok(times)
            })
            .await
    }

    async fn record_sync(&self, record: &CardDavSyncRecord) -> Result<(), StoreError> {
        let (report, failure) = match record.outcome {
            Ok(report) => (report, None),
            Err(failure) => (SyncReport::default(), Some(failure.as_str())),
        };
        let at = record.at;
        self.db
            .run(move |c| {
                c.execute(
                    SAVE_SYNC,
                    params![
                        at,
                        count(report.pulled),
                        count(report.pushed),
                        count(report.skipped),
                        failure
                    ],
                )
                .map_err(store_error)?;
                Ok(())
            })
            .await
    }

    async fn last_sync(&self) -> Result<Option<CardDavSyncRecord>, StoreError> {
        let row = self
            .db
            .read(|c| {
                c.query_row(LAST_SYNC, [], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                })
                .optional()
                .map_err(store_error)
            })
            .await?;
        Ok(
            row.map(|(at, pulled, pushed, skipped, failure)| CardDavSyncRecord {
                at,
                outcome: match failure {
                    // Code inconnu (base d'une version plus récente) :
                    // réponse invalide.
                    Some(code) => Err(SyncFailure::parse(&code).unwrap_or(SyncFailure::Protocol)),
                    None => Ok(SyncReport {
                        pulled: uncount(pulled),
                        pushed: uncount(pushed),
                        skipped: uncount(skipped),
                    }),
                },
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use norelly_domain::contacts::ContactDraft;
    use norelly_domain::ports::{ContactBook, ContactStoreError};
    use norelly_test_support::carddav_contracts::carddav_store_contract;
    use norelly_test_support::contacts_contracts::sample_contact;

    use super::*;
    use crate::contacts::SqliteContactBook;

    #[tokio::test]
    async fn the_store_passes_the_contract() -> Result<(), ContactStoreError> {
        let db = Database::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let book = SqliteContactBook::new(db.clone());
        carddav_store_contract(&SqliteCardDavStore::new(db), &book).await
    }

    /// Les déclencheurs datent les écritures quel que soit leur chemin.
    #[tokio::test]
    async fn modification_times_come_from_the_schema() -> Result<(), ContactStoreError> {
        let db = Database::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let book = SqliteContactBook::new(db.clone());
        let store = SqliteCardDavStore::new(db.clone());
        let alice = book
            .create(&sample_contact("Alice", &["alice@example.com"]))
            .await?;
        db.write_for_tests("UPDATE contacts SET modified_at = 5")
            .await?;
        assert_eq!(store.modification_times().await?, [(alice.id(), 5)]);
        // Une modification, même du seul favori, redate la fiche.
        let mut input = alice.draft().to_input();
        input.favorite = true;
        let draft = ContactDraft::new(input).unwrap_or_else(|e| panic!("{e}"));
        book.update(alice.id(), &draft).await?;
        let times = store.modification_times().await?;
        assert!(times[0].1 > 5, "{times:?}");
        Ok(())
    }
}
