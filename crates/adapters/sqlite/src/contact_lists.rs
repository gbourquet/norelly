//! Listes de diffusion du carnet (ADR-012) : `ContactListBook` et
//! `ContactListDirectory`, sur la base du carnet ([`SqliteContactBook`]).
//!
//! Tri et filtre portent sur `name_key`, calculée par le domaine à
//! l'écriture (comparaison du domaine, sans le `lower()` ASCII de
//! SQLite). L'unicité du nom et l'existence des membres sont vérifiées
//! avant l'écriture (pour nommer le refus) et garanties par le schéma.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::Mutex;

use norelly_domain::contact_lists::{
    ContactList, ContactListDraft, ContactListError, ContactListFilter, ContactListId,
    ContactListInput,
};
use norelly_domain::ports::{
    ContactListBook, ContactListCursor, ContactListDirectory, ContactListPage,
    ContactListStoreError, ContactListSummary, PageSize, StoreError,
};
use rusqlite::types::Value;
use rusqlite::{Connection, Row, Transaction, params};

use crate::contacts::{SqliteContactBook, json_list};
use crate::store_error;

/// Listes corrompues déjà signalées : chacune n'est journalisée qu'une
/// fois par lancement, même relue à chaque page ou suggestion.
static REPORTED: Mutex<BTreeSet<i64>> = Mutex::new(BTreeSet::new());

/// Défaut d'une liste en base, sans rien de son contenu.
#[derive(Debug, Clone, Copy)]
enum Defect {
    Id,
    ColumnType,
    UnreadableMember,
    Refused(ContactListError),
    KeyOrMembers,
}

impl fmt::Display for Defect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Id => f.write_str("identifiant"),
            Self::ColumnType => f.write_str("type de colonne"),
            Self::UnreadableMember => f.write_str("membre illisible"),
            // Refus du domaine : une nature et un nom de champ, jamais une
            // valeur.
            Self::Refused(e) => write!(f, "refus du domaine : {e}"),
            Self::KeyOrMembers => f.write_str("clé de nom ou membres"),
        }
    }
}

/// Liste `id` écartée de la lecture. Seuls son identifiant et la nature
/// du défaut sont journalisés, jamais son contenu, une fois par liste.
fn skip_corrupted(id: i64, defect: Defect) {
    if first_report(id) {
        tracing::warn!(list_id = id, %defect, "liste corrompue écartée");
    }
}

/// Vrai la première fois que la liste `id` est signalée (registre
/// inaccessible : signalée, mieux vaut un doublon qu'un oubli).
fn first_report(id: i64) -> bool {
    REPORTED
        .lock()
        .map(|mut reported| reported.insert(id))
        .unwrap_or(true)
}

/// Valeur d'une colonne, `None` si elle n'a pas le type attendu (ligne
/// corrompue) ; toute autre erreur de lecture est remontée.
fn tolerant<T>(read: rusqlite::Result<T>) -> rusqlite::Result<Option<T>> {
    match read {
        Ok(value) => Ok(Some(value)),
        Err(
            rusqlite::Error::InvalidColumnType(..)
            | rusqlite::Error::FromSqlConversionFailure(..)
            | rusqlite::Error::IntegralValueOutOfRange(..),
        ) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Champs d'une liste tels que stockés, revalidés par le domaine.
struct ListFields {
    name: String,
    name_key: String,
    description: Option<String>,
    bcc: bool,
}

impl ListFields {
    /// Colonnes 1 à 4 de `r` ; `None` si l'une n'a pas le type attendu.
    fn read(r: &Row<'_>) -> rusqlite::Result<Option<Self>> {
        let (Some(name), Some(name_key), Some(description), Some(bcc)) = (
            tolerant(r.get(1))?,
            tolerant(r.get(2))?,
            tolerant(r.get(3))?,
            tolerant(r.get(4))?,
        ) else {
            return Ok(None);
        };
        Ok(Some(Self {
            name,
            name_key,
            description,
            bcc,
        }))
    }
}

/// Ligne brute d'une liste : `fields` vaut `None` si une colonne n'a pas
/// le type attendu ; un membre illisible rend la liste entière corrompue.
struct ListRow {
    fields: Option<ListFields>,
    members: Vec<i64>,
    unreadable_member: bool,
}

/// Liste revalidée par le domaine, ou la nature de son défaut.
fn validate(raw: i64, row: ListRow) -> Result<ContactList, Defect> {
    let id = ContactListId::new(raw).ok_or(Defect::Id)?;
    let fields = row.fields.ok_or(Defect::ColumnType)?;
    if row.unreadable_member {
        return Err(Defect::UnreadableMember);
    }
    let count = row.members.len();
    let draft = ContactListDraft::new(ContactListInput {
        name: fields.name,
        description: fields.description.unwrap_or_default(),
        members: row.members,
        bcc: fields.bcc,
    })
    .map_err(Defect::Refused)?;
    // Unicité et tri portent sur la clé stockée : elle doit être celle du
    // domaine ; un membre répété serait fusionné en silence.
    if draft.name_key() != fields.name_key || draft.members().len() != count {
        return Err(Defect::KeyOrMembers);
    }
    Ok(draft.with_id(id))
}

/// Listes `ids`, dans cet ordre (inconnues omises) ; une liste corrompue
/// est écartée et journalisée, les autres restent lisibles.
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
                    fields: ListFields::read(r)?,
                    members: Vec::new(),
                    unreadable_member: false,
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
            Ok((r.get::<_, i64>(0)?, tolerant(r.get::<_, i64>(1))?))
        })
        .map_err(store_error)?;
    for member in members {
        let (id, contact) = member.map_err(store_error)?;
        if let Some(row) = rows.get_mut(&id) {
            match contact {
                Some(contact) => row.members.push(contact),
                None => row.unreadable_member = true,
            }
        }
    }
    Ok(ids
        .iter()
        .filter_map(|id| rows.remove(id).map(|row| (*id, row)))
        .filter_map(|(raw, row)| {
            validate(raw, row)
                .inspect_err(|defect| skip_corrupted(raw, *defect))
                .ok()
        })
        .collect())
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

/// Identifiants et clés de tri de la page : filtre sur `name_key`, après
/// le curseur, dans l'ordre de la liste.
const PAGE_FIRST: &str = "SELECT id, name_key FROM contact_lists \
     WHERE (?1 = '' OR instr(name_key, ?1) > 0) \
     ORDER BY name_key, id LIMIT ?2";
const PAGE_AFTER: &str = "SELECT id, name_key FROM contact_lists \
     WHERE (?1 = '' OR instr(name_key, ?1) > 0) AND (name_key, id) > (?3, ?4) \
     ORDER BY name_key, id LIMIT ?2";

impl ContactListDirectory for SqliteContactBook {
    async fn list_page(
        &self,
        filter: &ContactListFilter,
        cursor: Option<ContactListCursor>,
        limit: PageSize,
    ) -> Result<ContactListPage, StoreError> {
        let limit = limit.get();
        if limit == 0 {
            return Ok(ContactListPage::default());
        }
        let filter = filter.clone();
        self.db
            .read(move |c| {
                // Une liste valide de plus : savoir s'il existe une suite.
                let wanted = limit as usize + 1;
                // Lignes lues par requête : la page d'abord, puis par
                // grands lots une fois des listes corrompues écartées.
                let mut batch = limit.saturating_add(1);
                let text = filter.text();
                let mut lists = Vec::new();
                // Position de la dernière ligne lue, valide ou non : une
                // liste corrompue est écartée sans arrêter la lecture. La
                // clé est relue telle quelle : une clé qui n'est pas du
                // texte ne bloque pas la suite.
                let mut after: Option<(Value, i64)> =
                    cursor.map(|from| (Value::Text(from.sort_key), from.id.get()));
                loop {
                    let fetch = i64::from(batch);
                    let mut rows: Vec<(i64, Value)> = match &after {
                        None => c.prepare_cached(PAGE_FIRST).and_then(|mut s| {
                            s.query_map(params![text, fetch], |r| Ok((r.get(0)?, r.get(1)?)))?
                                .collect()
                        }),
                        Some((key, id)) => c.prepare_cached(PAGE_AFTER).and_then(|mut s| {
                            s.query_map(params![text, fetch, key, id], |r| {
                                Ok((r.get(0)?, r.get(1)?))
                            })?
                            .collect()
                        }),
                    }
                    .map_err(store_error)?;
                    let ids: Vec<i64> = rows.iter().map(|(id, _)| *id).collect();
                    lists.extend(load(c, &ids)?);
                    let exhausted = rows.len() < batch as usize;
                    after = rows.pop().map(|(id, key)| (key, id));
                    if exhausted || lists.len() >= wanted {
                        break;
                    }
                    batch = PageSize::MAX.get().saturating_add(1);
                }
                let more = lists.len() > limit as usize;
                lists.truncate(limit as usize);
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
    use norelly_domain::ports::{ContactBook, PageSize};
    use norelly_test_support::contact_lists_contracts::{
        contact_list_contract, contact_list_page_size_contract, contact_merge_contract,
        corrupted_list_contract, sample_list,
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

    #[tokio::test]
    async fn contact_lists_pass_page_size_contract() -> Result<(), ContactListStoreError> {
        contact_list_page_size_contract(&book()).await
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

    /// Corruptions injectées tour à tour : nom vide (refusé par le
    /// domaine), clé de nom qui n'est pas celle du domaine, nom qui n'est
    /// pas du texte, clé de nom qui n'est pas du texte, description de
    /// plusieurs lignes.
    const CORRUPTIONS: [&str; 5] = [
        "UPDATE contact_lists SET name = '' WHERE id = ?1",
        "UPDATE contact_lists SET name_key = 'zz' || name_key WHERE id = ?1",
        "UPDATE contact_lists SET name = x'00ff' WHERE id = ?1",
        "UPDATE contact_lists SET name_key = CAST(name_key AS BLOB) WHERE id = ?1",
        "UPDATE contact_lists SET description = 'a' || char(10) || 'b' WHERE id = ?1",
    ];

    #[tokio::test]
    async fn corrupted_list_passes_contract() -> Result<(), ContactListStoreError> {
        for first in 0..CORRUPTIONS.len() {
            let book = book();
            let turn = std::sync::atomic::AtomicUsize::new(first);
            corrupted_list_contract(&book, |id| {
                let sql = CORRUPTIONS
                    [turn.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % CORRUPTIONS.len()];
                let db = book.db.clone();
                async move {
                    db.run(move |c| c.execute(sql, params![id.get()]).map_err(store_error))
                        .await?;
                    Ok(())
                }
            })
            .await?;
        }
        Ok(())
    }

    #[test]
    fn a_corrupted_list_is_reported_once() {
        // Identifiant propre à ce test : le registre vit pour le processus.
        let id = -4242;
        assert!(first_report(id));
        assert!(!first_report(id), "liste signalée deux fois");
        assert!(first_report(id - 1));
    }

    #[tokio::test]
    async fn a_corrupted_list_can_still_be_replaced_or_deleted() -> Result<(), ContactListStoreError>
    {
        let book = book();
        let list = with_list(&book).await?;
        book.db
            .write_for_tests("UPDATE contact_lists SET name = '';")
            .await?;
        assert_eq!(book.get_list(list.id()).await?, None);
        let repaired = book
            .update_list(list.id(), &sample_list("L", list.draft().members(), false))
            .await?;
        assert_eq!(book.get_list(list.id()).await?, Some(repaired));
        book.db
            .write_for_tests("UPDATE contact_lists SET name = '';")
            .await?;
        assert_eq!(book.delete_lists(&[list.id()]).await?, 1);
        Ok(())
    }
}
