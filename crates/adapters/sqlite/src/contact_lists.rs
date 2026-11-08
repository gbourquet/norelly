//! Listes de diffusion du carnet (ADR-012) : `ContactListBook` et
//! `ContactListDirectory`, sur la base du carnet ([`SqliteContactBook`]).
//!
//! Tri et filtre portent sur `name_key`, calculée par le domaine à
//! l'écriture (comparaison du domaine, sans le `lower()` ASCII de
//! SQLite). L'unicité du nom et l'existence des membres sont vérifiées
//! avant l'écriture (pour nommer le refus) et garanties par le schéma.

use std::collections::HashMap;

use norelly_domain::contact_lists::{
    ContactList, ContactListDraft, ContactListFilter, ContactListId, ContactListInput,
};
use norelly_domain::ports::{
    ContactListBook, ContactListCursor, ContactListDirectory, ContactListPage,
    ContactListStoreError, ContactListSummary, MAX_PAGE_SIZE, StoreError,
};
use rusqlite::{Connection, Transaction, params};

use crate::contacts::{SqliteContactBook, json_list};
use crate::store_error;

fn corrupted(id: i64, detail: &str) -> StoreError {
    StoreError(format!("liste {id} corrompue : {detail}"))
}

/// Ligne brute d'une liste, revalidée par le domaine.
struct ListRow {
    name: String,
    name_key: String,
    description: Option<String>,
    bcc: bool,
    members: Vec<i64>,
}

/// Listes `ids`, dans cet ordre (inconnues omises).
fn load(c: &Connection, ids: &[i64]) -> Result<Vec<ContactList>, StoreError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let list = json_list(ids)?;
    let mut stmt = c
        .prepare_cached(
            "SELECT id, name, name_key, description, bcc FROM contact_lists \
             WHERE id IN (SELECT value FROM json_each(?1))",
        )
        .map_err(store_error)?;
    let mut rows: HashMap<i64, ListRow> = stmt
        .query_map(params![list], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                ListRow {
                    name: r.get(1)?,
                    name_key: r.get(2)?,
                    description: r.get(3)?,
                    bcc: r.get(4)?,
                    members: Vec::new(),
                },
            ))
        })
        .map_err(store_error)?
        .collect::<Result<_, _>>()
        .map_err(store_error)?;
    let mut stmt = c
        .prepare_cached(
            "SELECT list_id, contact_id FROM contact_list_members \
             WHERE list_id IN (SELECT value FROM json_each(?1)) \
             ORDER BY list_id, position",
        )
        .map_err(store_error)?;
    let members = stmt
        .query_map(params![list], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(store_error)?;
    for member in members {
        let (id, contact) = member.map_err(store_error)?;
        if let Some(row) = rows.get_mut(&id) {
            row.members.push(contact);
        }
    }
    ids.iter()
        .filter_map(|id| rows.remove(id).map(|row| (*id, row)))
        .map(|(raw, row)| {
            let id = ContactListId::new(raw).ok_or_else(|| corrupted(raw, "identifiant"))?;
            let count = row.members.len();
            let draft = ContactListDraft::new(ContactListInput {
                name: row.name,
                description: row.description.unwrap_or_default(),
                members: row.members,
                bcc: row.bcc,
            })
            .map_err(|e| corrupted(raw, &e.to_string()))?;
            // Unicité et tri portent sur la clé stockée : elle doit être
            // celle du domaine ; un membre répété serait fusionné en
            // silence.
            if draft.name_key() != row.name_key || draft.members().len() != count {
                return Err(corrupted(raw, "clé de nom ou membres"));
            }
            Ok(draft.with_id(id))
        })
        .collect()
}

/// Refus d'une écriture : nom pris par une autre liste que `except`, ou
/// membre absent du carnet.
fn refused(
    tx: &Transaction<'_>,
    draft: &ContactListDraft,
    except: Option<ContactListId>,
) -> Result<Option<ContactListStoreError>, StoreError> {
    let taken = tx
        .prepare_cached(
            "SELECT 1 FROM contact_lists WHERE name_key = ?1 AND (?2 IS NULL OR id <> ?2)",
        )
        .and_then(|mut s| s.exists(params![draft.name_key(), except.map(ContactListId::get)]))
        .map_err(store_error)?;
    if taken {
        return Ok(Some(ContactListStoreError::DuplicateName(
            draft.name().to_owned(),
        )));
    }
    let mut stmt = tx
        .prepare_cached("SELECT 1 FROM contacts WHERE id = ?1")
        .map_err(store_error)?;
    for &member in draft.members() {
        if !stmt.exists(params![member.get()]).map_err(store_error)? {
            return Ok(Some(ContactListStoreError::UnknownContact(member)));
        }
    }
    Ok(None)
}

/// Écrit le contenu de la liste `id` (ligne existante), membres remplacés.
fn write(tx: &Transaction<'_>, id: i64, draft: &ContactListDraft) -> Result<(), StoreError> {
    tx.execute(
        "UPDATE contact_lists SET name = ?2, name_key = ?3, description = ?4, bcc = ?5 \
         WHERE id = ?1",
        params![
            id,
            draft.name(),
            draft.name_key(),
            draft.description(),
            draft.bcc()
        ],
    )
    .map_err(store_error)?;
    tx.execute(
        "DELETE FROM contact_list_members WHERE list_id = ?1",
        params![id],
    )
    .map_err(store_error)?;
    let mut stmt = tx
        .prepare_cached(
            "INSERT INTO contact_list_members (list_id, position, contact_id) \
             VALUES (?1, ?2, ?3)",
        )
        .map_err(store_error)?;
    for (position, member) in (0_i64..).zip(draft.members()) {
        stmt.execute(params![id, position, member.get()])
            .map_err(store_error)?;
    }
    Ok(())
}

impl ContactListBook for SqliteContactBook {
    async fn create_list(
        &self,
        draft: &ContactListDraft,
    ) -> Result<ContactList, ContactListStoreError> {
        let draft = draft.clone();
        self.db
            .run(move |c| {
                let tx = c.transaction().map_err(store_error)?;
                if let Some(refusal) = refused(&tx, &draft, None)? {
                    return Ok(Err(refusal));
                }
                // Clé provisoire unique (un nom ne contient pas de saut de
                // ligne) : remplacée par write.
                tx.execute(
                    "INSERT INTO contact_lists (name, name_key) VALUES ('', ?1)",
                    params![format!("\n{}", draft.name_key())],
                )
                .map_err(store_error)?;
                let raw = tx.last_insert_rowid();
                write(&tx, raw, &draft)?;
                tx.commit().map_err(store_error)?;
                Ok(ContactListId::new(raw)
                    .map(|id| draft.with_id(id))
                    .ok_or_else(|| {
                        ContactListStoreError::Store(StoreError(format!("identifiant {raw}")))
                    }))
            })
            .await?
    }

    async fn update_list(
        &self,
        id: ContactListId,
        draft: &ContactListDraft,
    ) -> Result<ContactList, ContactListStoreError> {
        let draft = draft.clone();
        self.db
            .run(move |c| {
                let tx = c.transaction().map_err(store_error)?;
                let exists = tx
                    .prepare_cached("SELECT 1 FROM contact_lists WHERE id = ?1")
                    .and_then(|mut s| s.exists(params![id.get()]))
                    .map_err(store_error)?;
                if !exists {
                    return Ok(Err(ContactListStoreError::NotFound));
                }
                if let Some(refusal) = refused(&tx, &draft, Some(id))? {
                    return Ok(Err(refusal));
                }
                write(&tx, id.get(), &draft)?;
                tx.commit().map_err(store_error)?;
                Ok(Ok(draft.with_id(id)))
            })
            .await?
    }

    async fn delete_lists(&self, ids: &[ContactListId]) -> Result<usize, ContactListStoreError> {
        if ids.is_empty() {
            return Ok(0);
        }
        let list = json_list(&ids.iter().map(|i| i.get()).collect::<Vec<_>>())?;
        Ok(self
            .db
            .run(move |c| {
                c.execute(
                    "DELETE FROM contact_lists WHERE id IN (SELECT value FROM json_each(?1))",
                    params![list],
                )
                .map_err(store_error)
            })
            .await?)
    }
}

/// Identifiants de la page : filtre sur `name_key`, après le curseur, dans
/// l'ordre de la liste.
const PAGE_FIRST: &str = "SELECT id FROM contact_lists \
     WHERE (?1 = '' OR instr(name_key, ?1) > 0) \
     ORDER BY name_key, id LIMIT ?2";
const PAGE_AFTER: &str = "SELECT id FROM contact_lists \
     WHERE (?1 = '' OR instr(name_key, ?1) > 0) AND (name_key, id) > (?3, ?4) \
     ORDER BY name_key, id LIMIT ?2";

impl ContactListDirectory for SqliteContactBook {
    async fn list_page(
        &self,
        filter: &ContactListFilter,
        cursor: Option<ContactListCursor>,
        limit: u32,
    ) -> Result<ContactListPage, StoreError> {
        let limit = limit.min(MAX_PAGE_SIZE);
        if limit == 0 {
            return Ok(ContactListPage::default());
        }
        let filter = filter.clone();
        self.db
            .read(move |c| {
                // Une ligne de plus : savoir s'il existe une suite.
                let fetch = i64::from(limit) + 1;
                let text = filter.text();
                let ids: Vec<i64> = match &cursor {
                    None => c.prepare_cached(PAGE_FIRST).and_then(|mut s| {
                        s.query_map(params![text, fetch], |r| r.get(0))?.collect()
                    }),
                    Some(after) => c.prepare_cached(PAGE_AFTER).and_then(|mut s| {
                        s.query_map(params![text, fetch, after.sort_key, after.id.get()], |r| {
                            r.get(0)
                        })?
                        .collect()
                    }),
                }
                .map_err(store_error)?;
                let more = ids.len() > limit as usize;
                let lists = load(c, &ids[..ids.len().min(limit as usize)])?;
                let next = if more {
                    lists.last().map(ContactListCursor::of)
                } else {
                    None
                };
                Ok(ContactListPage {
                    items: lists.iter().map(ContactListSummary::from).collect(),
                    next,
                })
            })
            .await
    }

    async fn get_list(&self, id: ContactListId) -> Result<Option<ContactList>, StoreError> {
        self.db.read(move |c| Ok(load(c, &[id.get()])?.pop())).await
    }
}

#[cfg(test)]
mod tests {
    use norelly_domain::ports::ContactBook;
    use norelly_test_support::contact_lists_contracts::{
        contact_list_contract, contact_merge_contract, sample_list,
    };
    use norelly_test_support::contacts_contracts::sample_contact;

    use super::*;
    use crate::Database;

    fn book() -> SqliteContactBook {
        SqliteContactBook::new(Database::open_in_memory().unwrap_or_else(|e| panic!("{e}")))
    }

    #[tokio::test]
    async fn contact_lists_pass_contract() -> Result<(), ContactListStoreError> {
        contact_list_contract(&book()).await
    }

    #[tokio::test]
    async fn contact_merge_passes_contract() -> Result<(), ContactListStoreError> {
        contact_merge_contract(&book()).await
    }

    async fn with_list(book: &SqliteContactBook) -> Result<ContactList, ContactListStoreError> {
        let a = book
            .create(&sample_contact("A", &["a@example.com"]))
            .await
            .map_err(|e| StoreError(e.to_string()))?;
        book.create_list(&sample_list("L", &[a.id()], false)).await
    }

    #[tokio::test]
    async fn schema_refuses_a_repeated_member_or_an_unknown_contact()
    -> Result<(), ContactListStoreError> {
        let book = book();
        with_list(&book).await?;
        // Témoin : un nouveau membre connu est accepté.
        book.db
            .write_for_tests(
                "INSERT INTO contacts (sort_key, search_text) VALUES ('b', 'b'); \
                 INSERT INTO contact_list_members (list_id, position, contact_id) \
                 VALUES (1, 1, 2);",
            )
            .await?;
        for refused in [
            "INSERT INTO contact_list_members (list_id, position, contact_id) VALUES (1, 5, 1);",
            "INSERT INTO contact_list_members (list_id, position, contact_id) VALUES (1, 6, 99);",
            "INSERT INTO contact_lists (name, name_key) VALUES ('l', 'l');",
        ] {
            assert!(book.db.write_for_tests(refused).await.is_err(), "{refused}");
        }
        Ok(())
    }

    #[tokio::test]
    async fn deleting_a_list_or_a_contact_removes_member_rows() -> Result<(), ContactListStoreError>
    {
        let book = book();
        let list = with_list(&book).await?;
        let members = |db: Database| async move {
            db.read_for_tests(|c| {
                c.query_row("SELECT COUNT(*) FROM contact_list_members", [], |r| {
                    r.get::<_, i64>(0)
                })
                .map_err(store_error)
            })
            .await
        };
        assert_eq!(members(book.db.clone()).await?, 1);
        book.delete_lists(&[list.id()]).await?;
        assert_eq!(members(book.db.clone()).await?, 0);
        let list = book
            .create_list(&sample_list("M", list.draft().members(), false))
            .await?;
        book.delete_many(list.draft().members())
            .await
            .map_err(|e| StoreError(e.to_string()))?;
        assert_eq!(members(book.db.clone()).await?, 0);
        assert!(book.db.integrity_problems().await?.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn corrupted_lists_are_reported() -> Result<(), ContactListStoreError> {
        let book = book();
        let list = with_list(&book).await?;
        book.db
            .write_for_tests("UPDATE contact_lists SET name_key = 'autre';")
            .await?;
        assert!(
            book.get_list(list.id()).await.is_err(),
            "clé incohérente acceptée"
        );
        book.db
            .write_for_tests("UPDATE contact_lists SET name = '', name_key = '';")
            .await?;
        assert!(book.get_list(list.id()).await.is_err(), "nom vide accepté");
        assert!(
            book.list_page(&ContactListFilter::default(), None, 10)
                .await
                .is_err()
        );
        Ok(())
    }
}
