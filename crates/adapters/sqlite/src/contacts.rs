//! Carnet d'adresses (ADR-012) : `ContactBook` côté écriture,
//! `ContactDirectory` côté lecture.
//!
//! Tri et filtre portent sur `sort_key` et `search_text`, calculés par le
//! domaine à l'écriture : la comparaison SQL (octets UTF-8, `instr`) est
//! celle du domaine, sans dépendre du `lower()` de SQLite (ASCII seul).
//! L'unicité d'une adresse dans le carnet est vérifiée avant l'écriture
//! (pour nommer l'adresse refusée) et garantie par le schéma.

use std::collections::HashMap;

use norelly_domain::address::EmailAddress;
use norelly_domain::contact_export::ContactSelection;
use norelly_domain::contacts::{
    AddressKey, Contact, ContactDraft, ContactFilter, ContactId, ContactInput, EmailInput,
    PhoneInput,
};
use norelly_domain::ports::{
    ContactBook, ContactCursor, ContactDirectory, ContactListItem, ContactPage, ContactStoreError,
    ImportOutcome, MAX_CONTACT_LOOKUP, MAX_PAGE_SIZE, StoreError,
};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::{Database, store_error};

/// Carnet d'adresses SQLite.
#[derive(Debug, Clone)]
pub struct SqliteContactBook {
    pub(crate) db: Database,
}

impl SqliteContactBook {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

/// Colonnes d'une fiche, dans l'ordre lu par [`ContactRow::read`].
const CONTACT_COLUMNS: &str = "id, display_name, given_name, family_name, nickname, \
     organization, notes, favorite";

/// Fiche brute, revalidée par le domaine.
struct ContactRow {
    id: i64,
    texts: [Option<String>; 6],
    favorite: bool,
    emails: Vec<EmailInput>,
    /// Clés stockées des adresses, dans le même ordre.
    keys: Vec<String>,
    phones: Vec<PhoneInput>,
}

impl ContactRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            texts: [
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ],
            favorite: row.get(7)?,
            emails: Vec::new(),
            keys: Vec::new(),
            phones: Vec::new(),
        })
    }

    fn into_contact(self) -> Result<Contact, StoreError> {
        let id = ContactId::new(self.id).ok_or_else(|| corrupted(self.id, "identifiant"))?;
        let [
            display_name,
            given_name,
            family_name,
            nickname,
            organization,
            notes,
        ] = self.texts.map(Option::unwrap_or_default);
        let draft = ContactDraft::new(ContactInput {
            display_name,
            given_name,
            family_name,
            nickname,
            organization,
            notes,
            emails: self.emails,
            phones: self.phones,
            favorite: self.favorite,
        })
        .map_err(|e| corrupted(self.id, &e.to_string()))?;
        // La contrainte d'unicité et la recherche par adresse portent sur
        // la clé stockée : elle doit être celle que calcule le domaine.
        let stored = self.keys.iter().map(String::as_str);
        if !draft.emails().iter().map(|e| e.key().as_str()).eq(stored) {
            return Err(corrupted(self.id, "clé d'adresse"));
        }
        Ok(draft.with_id(id))
    }
}

fn corrupted(id: i64, detail: &str) -> StoreError {
    StoreError(format!("contact {id} corrompu : {detail}"))
}

/// Tableau JSON des valeurs, lu en SQL par `json_each` (liste de taille
/// variable passée en un seul paramètre lié).
pub(crate) fn json_list<T: serde::Serialize>(values: &[T]) -> Result<String, StoreError> {
    serde_json::to_string(values).map_err(|e| StoreError(e.to_string()))
}

/// Fiches `ids`, dans cet ordre (inconnues omises).
fn load(c: &Connection, ids: &[i64]) -> Result<Vec<Contact>, StoreError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let list = json_list(ids)?;
    // sql-const : liste des colonnes.
    let sql = format!(
        "SELECT {CONTACT_COLUMNS} FROM contacts WHERE id IN (SELECT value FROM json_each(?1))"
    );
    let mut stmt = c.prepare_cached(&sql).map_err(store_error)?;
    let mut rows: HashMap<i64, ContactRow> = stmt
        .query_map(params![list], ContactRow::read)
        .map_err(store_error)?
        .map(|r| r.map(|row| (row.id, row)))
        .collect::<Result<_, _>>()
        .map_err(store_error)?;

    let mut stmt = c
        .prepare_cached(
            "SELECT contact_id, address, label, preferred, address_key FROM contact_emails \
             WHERE contact_id IN (SELECT value FROM json_each(?1)) \
             ORDER BY contact_id, position",
        )
        .map_err(store_error)?;
    let emails = stmt
        .query_map(params![list], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                EmailInput {
                    address: r.get(1)?,
                    label: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    preferred: r.get(3)?,
                },
                r.get::<_, String>(4)?,
            ))
        })
        .map_err(store_error)?;
    for email in emails {
        let (id, email, key) = email.map_err(store_error)?;
        if let Some(row) = rows.get_mut(&id) {
            row.emails.push(email);
            row.keys.push(key);
        }
    }

    let mut stmt = c
        .prepare_cached(
            "SELECT contact_id, number, label FROM contact_phones \
             WHERE contact_id IN (SELECT value FROM json_each(?1)) \
             ORDER BY contact_id, position",
        )
        .map_err(store_error)?;
    let phones = stmt
        .query_map(params![list], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                PhoneInput {
                    number: r.get(1)?,
                    label: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                },
            ))
        })
        .map_err(store_error)?;
    for phone in phones {
        let (id, phone) = phone.map_err(store_error)?;
        if let Some(row) = rows.get_mut(&id) {
            row.phones.push(phone);
        }
    }

    ids.iter()
        .filter_map(|id| rows.remove(id))
        .map(ContactRow::into_contact)
        .collect()
}

/// Première adresse de `draft` déjà portée par un contact autre que
/// `except`.
fn taken(
    tx: &Transaction<'_>,
    draft: &ContactDraft,
    except: Option<ContactId>,
) -> Result<Option<EmailAddress>, StoreError> {
    let mut stmt = tx
        .prepare_cached(
            "SELECT 1 FROM contact_emails WHERE address_key = ?1 \
             AND (?2 IS NULL OR contact_id <> ?2)",
        )
        .map_err(store_error)?;
    for email in draft.emails() {
        let found = stmt
            .exists(params![email.key().as_str(), except.map(ContactId::get)])
            .map_err(store_error)?;
        if found {
            return Ok(Some(email.address().clone()));
        }
    }
    Ok(None)
}

/// Écrit le contenu de la fiche `id` (ligne existante), adresses et
/// téléphones remplacés.
fn write(tx: &Transaction<'_>, id: i64, draft: &ContactDraft) -> Result<(), StoreError> {
    tx.execute(
        "UPDATE contacts SET display_name = ?2, given_name = ?3, family_name = ?4, \
         nickname = ?5, organization = ?6, notes = ?7, favorite = ?8, sort_key = ?9, \
         search_text = ?10 WHERE id = ?1",
        params![
            id,
            draft.display_name_field(),
            draft.given_name(),
            draft.family_name(),
            draft.nickname(),
            draft.organization(),
            draft.notes(),
            draft.favorite(),
            draft.sort_key(),
            draft.search_text(),
        ],
    )
    .map_err(store_error)?;
    tx.execute(
        "DELETE FROM contact_emails WHERE contact_id = ?1",
        params![id],
    )
    .map_err(store_error)?;
    tx.execute(
        "DELETE FROM contact_phones WHERE contact_id = ?1",
        params![id],
    )
    .map_err(store_error)?;
    let mut stmt = tx
        .prepare_cached(
            "INSERT INTO contact_emails (contact_id, position, address, address_key, label, \
             preferred) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .map_err(store_error)?;
    for (position, email) in (0_i64..).zip(draft.emails()) {
        stmt.execute(params![
            id,
            position,
            email.address().to_string(),
            email.key().as_str(),
            email.label(),
            email.preferred(),
        ])
        .map_err(store_error)?;
    }
    let mut stmt = tx
        .prepare_cached(
            "INSERT INTO contact_phones (contact_id, position, number, label) \
             VALUES (?1, ?2, ?3, ?4)",
        )
        .map_err(store_error)?;
    for (position, phone) in (0_i64..).zip(draft.phones()) {
        stmt.execute(params![id, position, phone.number(), phone.label()])
            .map_err(store_error)?;
    }
    Ok(())
}

impl ContactBook for SqliteContactBook {
    async fn create(&self, draft: &ContactDraft) -> Result<Contact, ContactStoreError> {
        let draft = draft.clone();
        self.db
            .run(move |c| {
                let tx = c.transaction().map_err(store_error)?;
                if let Some(address) = taken(&tx, &draft, None)? {
                    return Ok(Err(ContactStoreError::DuplicateAddress(address)));
                }
                tx.execute(
                    "INSERT INTO contacts (sort_key, search_text) VALUES ('', '')",
                    [],
                )
                .map_err(store_error)?;
                let raw = tx.last_insert_rowid();
                write(&tx, raw, &draft)?;
                tx.commit().map_err(store_error)?;
                Ok(ContactId::new(raw)
                    .map(|id| draft.with_id(id))
                    .ok_or_else(|| ContactStoreError::Corrupted(format!("identifiant {raw}"))))
            })
            .await?
    }

    async fn update(
        &self,
        id: ContactId,
        draft: &ContactDraft,
    ) -> Result<Contact, ContactStoreError> {
        let draft = draft.clone();
        self.db
            .run(move |c| {
                let tx = c.transaction().map_err(store_error)?;
                let exists = tx
                    .prepare_cached("SELECT 1 FROM contacts WHERE id = ?1")
                    .and_then(|mut s| s.exists(params![id.get()]))
                    .map_err(store_error)?;
                if !exists {
                    return Ok(Err(ContactStoreError::NotFound));
                }
                if let Some(address) = taken(&tx, &draft, Some(id))? {
                    return Ok(Err(ContactStoreError::DuplicateAddress(address)));
                }
                write(&tx, id.get(), &draft)?;
                tx.commit().map_err(store_error)?;
                Ok(Ok(draft.with_id(id)))
            })
            .await?
    }

    async fn delete(&self, id: ContactId) -> Result<bool, ContactStoreError> {
        Ok(self.delete_many(&[id]).await? == 1)
    }

    async fn import(
        &self,
        create: &[ContactDraft],
        complete: &[Contact],
    ) -> Result<ImportOutcome, ContactStoreError> {
        let create = create.to_vec();
        let complete = complete.to_vec();
        self.db
            .run(move |c| {
                let tx = c.transaction().map_err(store_error)?;
                // Sans commit, la transaction est annulée à sa fermeture.
                for contact in &complete {
                    let exists = tx
                        .prepare_cached("SELECT 1 FROM contacts WHERE id = ?1")
                        .and_then(|mut s| s.exists(params![contact.id().get()]))
                        .map_err(store_error)?;
                    if !exists {
                        return Ok(Err(ContactStoreError::NotFound));
                    }
                    if let Some(address) = taken(&tx, contact.draft(), Some(contact.id()))? {
                        return Ok(Err(ContactStoreError::DuplicateAddress(address)));
                    }
                    write(&tx, contact.id().get(), contact.draft())?;
                }
                for draft in &create {
                    if let Some(address) = taken(&tx, draft, None)? {
                        return Ok(Err(ContactStoreError::DuplicateAddress(address)));
                    }
                    tx.execute(
                        "INSERT INTO contacts (sort_key, search_text) VALUES ('', '')",
                        [],
                    )
                    .map_err(store_error)?;
                    let raw = tx.last_insert_rowid();
                    write(&tx, raw, draft)?;
                }
                tx.commit().map_err(store_error)?;
                Ok(Ok(ImportOutcome {
                    created: create.len(),
                    completed: complete.len(),
                }))
            })
            .await?
    }

    async fn merge(
        &self,
        keep: &Contact,
        draft: &ContactDraft,
        absorbed: &[Contact],
    ) -> Result<Contact, ContactStoreError> {
        let read: Vec<Contact> = std::iter::once(keep).chain(absorbed).cloned().collect();
        let keep = keep.id();
        let absorbed: Vec<ContactId> = absorbed.iter().map(Contact::id).collect();
        let draft = draft.clone();
        self.db
            .run(move |c| {
                let tx = c.transaction().map_err(store_error)?;
                // Sans commit, la transaction est annulée à sa fermeture.
                let mut exists = tx
                    .prepare_cached("SELECT 1 FROM contacts WHERE id = ?1")
                    .map_err(store_error)?;
                for id in std::iter::once(&keep).chain(&absorbed) {
                    if !exists.exists(params![id.get()]).map_err(store_error)? {
                        return Ok(Err(ContactStoreError::NotFound));
                    }
                }
                drop(exists);
                // Chaque fiche telle qu'elle a été lue pour calculer la
                // fusion : une modification entre-temps serait perdue.
                let ids: Vec<i64> = read.iter().map(|c| c.id().get()).collect();
                let current = load(&tx, &ids)?;
                if read
                    .iter()
                    .any(|r| current.iter().find(|c| c.id() == r.id()) != Some(r))
                {
                    return Ok(Err(ContactStoreError::Conflict));
                }
                let mut owner = tx
                    .prepare_cached("SELECT contact_id FROM contact_emails WHERE address_key = ?1")
                    .map_err(store_error)?;
                for email in draft.emails() {
                    let found: Option<i64> = owner
                        .query_row(params![email.key().as_str()], |row| row.get(0))
                        .optional()
                        .map_err(store_error)?;
                    let foreign = found.is_some_and(|id| {
                        id != keep.get() && !absorbed.iter().any(|a| a.get() == id)
                    });
                    if foreign {
                        return Ok(Err(ContactStoreError::DuplicateAddress(
                            email.address().clone(),
                        )));
                    }
                }
                drop(owner);
                for gone in &absorbed {
                    // La fiche gardée prend la place de la fiche absorbée
                    // dans les listes où elle ne figure pas encore ; les
                    // autres lignes partent avec la fiche absorbée.
                    tx.execute(
                        "UPDATE contact_list_members SET contact_id = ?1 \
                         WHERE contact_id = ?2 AND list_id NOT IN \
                         (SELECT list_id FROM contact_list_members WHERE contact_id = ?1)",
                        params![keep.get(), gone.get()],
                    )
                    .map_err(store_error)?;
                    tx.execute("DELETE FROM contacts WHERE id = ?1", params![gone.get()])
                        .map_err(store_error)?;
                }
                write(&tx, keep.get(), &draft)?;
                tx.commit().map_err(store_error)?;
                Ok(Ok(draft.with_id(keep)))
            })
            .await?
    }

    async fn delete_many(&self, ids: &[ContactId]) -> Result<usize, ContactStoreError> {
        if ids.is_empty() {
            return Ok(0);
        }
        let list = json_list(&ids.iter().map(|i| i.get()).collect::<Vec<_>>())?;
        Ok(self
            .db
            .run(move |c| {
                c.execute(
                    "DELETE FROM contacts WHERE id IN (SELECT value FROM json_each(?1))",
                    params![list],
                )
                .map_err(store_error)
            })
            .await?)
    }
}

/// Identifiants de la page : filtre (favoris, texte cherché dans
/// `search_text`), après le curseur, dans l'ordre de la liste.
const PAGE_FIRST: &str = "SELECT id FROM contacts \
     WHERE (?1 = 0 OR favorite = 1) AND (?2 = '' OR instr(search_text, ?2) > 0) \
     ORDER BY sort_key, id LIMIT ?3";
const PAGE_AFTER: &str = "SELECT id FROM contacts \
     WHERE (?1 = 0 OR favorite = 1) AND (?2 = '' OR instr(search_text, ?2) > 0) \
       AND (sort_key, id) > (?4, ?5) \
     ORDER BY sort_key, id LIMIT ?3";

impl ContactDirectory for SqliteContactBook {
    async fn page(
        &self,
        filter: &ContactFilter,
        cursor: Option<ContactCursor>,
        limit: u32,
    ) -> Result<ContactPage, StoreError> {
        let limit = limit.min(MAX_PAGE_SIZE);
        if limit == 0 {
            return Ok(ContactPage::default());
        }
        let filter = filter.clone();
        self.db
            .read(move |c| {
                // Une ligne de plus : savoir s'il existe une suite.
                let fetch = i64::from(limit) + 1;
                let favorites = filter.favorites_only();
                let text = filter.text();
                let ids: Vec<i64> = match &cursor {
                    None => c.prepare_cached(PAGE_FIRST).and_then(|mut s| {
                        s.query_map(params![favorites, text, fetch], |r| r.get(0))?
                            .collect()
                    }),
                    Some(after) => c.prepare_cached(PAGE_AFTER).and_then(|mut s| {
                        s.query_map(
                            params![favorites, text, fetch, after.sort_key, after.id.get()],
                            |r| r.get(0),
                        )?
                        .collect()
                    }),
                }
                .map_err(store_error)?;
                let more = ids.len() > limit as usize;
                let contacts = load(c, &ids[..ids.len().min(limit as usize)])?;
                let next = if more {
                    contacts.last().map(ContactCursor::of)
                } else {
                    None
                };
                Ok(ContactPage {
                    items: contacts.iter().map(ContactListItem::from).collect(),
                    next,
                })
            })
            .await
    }

    async fn get(&self, id: ContactId) -> Result<Option<Contact>, StoreError> {
        self.db.read(move |c| Ok(load(c, &[id.get()])?.pop())).await
    }

    async fn by_addresses(&self, keys: &[AddressKey]) -> Result<Vec<Contact>, StoreError> {
        let keys: Vec<&str> = keys
            .iter()
            .take(MAX_CONTACT_LOOKUP)
            .map(AddressKey::as_str)
            .collect();
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let list = json_list(&keys)?;
        self.db
            .read(move |c| {
                let ids: Vec<i64> = c
                    .prepare_cached(
                        "SELECT id FROM contacts WHERE id IN ( \
                           SELECT contact_id FROM contact_emails \
                            WHERE address_key IN (SELECT value FROM json_each(?1))) \
                         ORDER BY sort_key, id",
                    )
                    .and_then(|mut s| s.query_map(params![list], |r| r.get(0))?.collect())
                    .map_err(store_error)?;
                load(c, &ids)
            })
            .await
    }

    async fn contacts(
        &self,
        selection: &ContactSelection,
        limit: usize,
    ) -> Result<Vec<Contact>, StoreError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let chosen = match selection {
            ContactSelection::All => None,
            ContactSelection::Contacts(ids) => Some(json_list(
                &ids.iter().map(|id| id.get()).collect::<Vec<_>>(),
            )?),
        };
        self.db
            .read(move |c| {
                let ids: Vec<i64> = match &chosen {
                    None => c
                        .prepare_cached("SELECT id FROM contacts ORDER BY sort_key, id LIMIT ?1")
                        .and_then(|mut s| s.query_map(params![limit], |r| r.get(0))?.collect()),
                    Some(list) => c
                        .prepare_cached(
                            "SELECT id FROM contacts \
                              WHERE id IN (SELECT value FROM json_each(?1)) \
                              ORDER BY sort_key, id LIMIT ?2",
                        )
                        .and_then(|mut s| {
                            s.query_map(params![list, limit], |r| r.get(0))?.collect()
                        }),
                }
                .map_err(store_error)?;
                load(c, &ids)
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use norelly_test_support::contacts_contracts::{
        contact_book_contract, contact_import_contract, contact_selection_contract, full_contact,
        sample_contact,
    };

    use super::*;

    fn book() -> SqliteContactBook {
        SqliteContactBook::new(Database::open_in_memory().unwrap_or_else(|e| panic!("{e}")))
    }

    #[tokio::test]
    async fn contact_book_passes_contract() -> Result<(), ContactStoreError> {
        contact_book_contract(&book()).await
    }

    #[tokio::test]
    async fn contact_book_passes_import_contract() -> Result<(), ContactStoreError> {
        contact_import_contract(&book()).await
    }

    #[tokio::test]
    async fn contact_book_passes_selection_contract() -> Result<(), ContactStoreError> {
        contact_selection_contract(&book()).await
    }

    fn count(c: &rusqlite::Connection, table: &'static str) -> Result<i64, StoreError> {
        // sql-const : nom de table du test.
        c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .map_err(crate::store_error)
    }

    #[tokio::test]
    async fn deleting_a_contact_deletes_its_addresses_and_phones() -> Result<(), ContactStoreError>
    {
        let book = book();
        let alice = book.create(&full_contact()).await?;
        let counts = |db: Database| async move {
            db.read_for_tests(|c| Ok((count(c, "contact_emails")?, count(c, "contact_phones")?)))
                .await
        };
        assert_eq!(counts(book.db.clone()).await?, (2, 2));
        // Une modification remplace adresses et téléphones.
        book.update(alice.id(), &sample_contact("A", &["a@example.com"]))
            .await?;
        assert_eq!(counts(book.db.clone()).await?, (1, 0));
        assert!(book.delete(alice.id()).await?);
        assert_eq!(counts(book.db.clone()).await?, (0, 0));
        assert!(book.db.integrity_problems().await?.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn schema_refuses_a_shared_address_or_two_preferred() -> Result<(), ContactStoreError> {
        let book = book();
        book.create(&sample_contact("A", &["a@example.com"]))
            .await?;
        book.create(&sample_contact("B", &["b@example.com"]))
            .await?;
        // Témoin : une adresse libre, non préférée, est acceptée.
        book.db
            .write_for_tests(
                "INSERT INTO contact_emails (contact_id, position, address, address_key, \
                 preferred) VALUES (2, 2, 'd@example.com', 'd@example.com', 0);",
            )
            .await?;
        assert!(
            book.db
                .write_for_tests(
                    "INSERT INTO contact_emails (contact_id, position, address, address_key, \
                     preferred) VALUES (2, 1, 'A@example.com', 'a@example.com', 0);"
                )
                .await
                .is_err()
        );
        assert!(
            book.db
                .write_for_tests(
                    "INSERT INTO contact_emails (contact_id, position, address, address_key, \
                     preferred) VALUES (2, 1, 'c@example.com', 'c@example.com', 1);"
                )
                .await
                .is_err()
        );
        Ok(())
    }

    #[tokio::test]
    async fn stored_address_key_must_match_the_address() -> Result<(), ContactStoreError> {
        let book = book();
        let a = book
            .create(&sample_contact("A", &["a@example.com"]))
            .await?;
        book.db
            .write_for_tests("UPDATE contact_emails SET address_key = 'patron@corp.com';")
            .await?;
        assert!(
            book.get(a.id()).await.is_err(),
            "clé stockée incohérente acceptée"
        );
        Ok(())
    }

    #[tokio::test]
    async fn corrupted_rows_are_reported() -> Result<(), ContactStoreError> {
        let book = book();
        let a = book
            .create(&sample_contact("A", &["a@example.com"]))
            .await?;
        book.db
            .write_for_tests("UPDATE contact_emails SET address = 'pas une adresse';")
            .await?;
        assert!(book.get(a.id()).await.is_err());
        assert!(
            book.page(&ContactFilter::default(), None, 10)
                .await
                .is_err()
        );
        // Une fiche sans adresse est invalide.
        book.db
            .write_for_tests("DELETE FROM contact_emails;")
            .await?;
        assert!(book.get(a.id()).await.is_err());
        Ok(())
    }
}
