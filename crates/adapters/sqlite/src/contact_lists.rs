//! Listes de diffusion du carnet (ADR-012) : `ContactListBook` et
//! `ContactListDirectory`, sur la base du carnet ([`SqliteContactBook`]).
//!
//! Tri et filtre portent sur `name_key`, calculée par le domaine à
//! l'écriture (comparaison du domaine, sans le `lower()` ASCII de
//! SQLite). L'unicité du nom et l'existence des membres sont vérifiées
//! avant l'écriture (pour nommer le refus) et garanties par le schéma ;
//! une liste écartée à la lecture (corrompue) ne garde pas son nom.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::{Mutex, PoisonError};

use norelly_domain::contact_lists::{
    ContactList, ContactListDraft, ContactListError, ContactListFilter, ContactListId,
    ContactListInput, MAX_CONTACT_LIST_MEMBERS,
};
use norelly_domain::ports::{
    ContactListBook, ContactListCursor, ContactListDirectory, ContactListPage,
    ContactListStoreError, ContactListSummary, PageSize, StoreError,
};
use rusqlite::ToSql;
use rusqlite::types::{FromSql, FromSqlResult, ToSqlOutput, Value, ValueRef};
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

/// Vrai la première fois que la liste `id` est signalée.
fn first_report(id: i64) -> bool {
    // Un verrou empoisonné garde un registre cohérent (une insertion
    // n'est jamais à moitié faite) : il est repris tel quel.
    REPORTED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(id)
}

/// Valeur d'une colonne, `None` si elle n'a pas le type attendu (ligne
/// corrompue) ; toute autre erreur de lecture est remontée.
fn tolerant<T>(read: rusqlite::Result<T>) -> rusqlite::Result<Option<T>> {
    match read {
        Ok(value) => Ok(Some(value)),
        Err(
            rusqlite::Error::InvalidColumnType(..)
            | rusqlite::Error::FromSqlConversionFailure(..)
            | rusqlite::Error::IntegralValueOutOfRange(..)
            | rusqlite::Error::Utf8Error(..),
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
            "SELECT list_id, contact_id FROM ( \
                 SELECT list_id, contact_id, position, \
                        row_number() OVER (PARTITION BY list_id ORDER BY position) AS rank \
                 FROM contact_list_members \
                 WHERE list_id IN (SELECT value FROM json_each(?1))) \
             WHERE rank <= ?2 \
             ORDER BY list_id, position",
        )
        .map_err(store_error)?;
    // Un membre de plus que la borne suffit au domaine pour refuser une
    // liste trop longue : une liste corrompue n'est pas lue en entier.
    let cap = i64::try_from(MAX_CONTACT_LIST_MEMBERS + 1).map_err(|e| StoreError(e.to_string()))?;
    let members = stmt
        .query_map(params![list, cap], |r| {
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

/// Libère la clé de nom de la liste écartée `id` : seule colonne
/// réécrite, le reste de la ligne (nom, description, Cci, membres) est
/// intact, et la liste reste écartée (sa clé n'est plus celle de son nom,
/// qui permet de la recalculer). La clé libre choisie commence par un
/// caractère de contrôle, refusé dans un nom : jamais celle d'une liste
/// lisible ; une ligne abîmée qui la porterait déjà est contournée.
fn release_name(tx: &Transaction<'_>, id: i64) -> Result<(), StoreError> {
    let mut key = format!("\u{1}{id}");
    let mut taken = tx
        .prepare_cached("SELECT 1 FROM contact_lists WHERE name_key = ?1")
        .map_err(store_error)?;
    // Chaque essai est une clé nouvelle : la boucle s'arrête au plus
    // après autant d'essais que de lignes.
    while taken.exists(params![key]).map_err(store_error)? {
        key.push('\u{1}');
    }
    tx.execute(
        "UPDATE contact_lists SET name_key = ?2 WHERE id = ?1",
        params![id, key],
    )
    .map_err(store_error)?;
    Ok(())
}

/// Admission d'une écriture : refusée si le nom est pris par une autre
/// liste lisible que `except`, ou si un membre est absent du carnet. Une
/// liste écartée à la lecture ne compte pas dans l'unicité du nom : une
/// fois tous les refus écartés, sa clé est libérée ([`release_name`]),
/// dans la transaction de l'écriture.
fn admit(
    tx: &Transaction<'_>,
    draft: &ContactListDraft,
    except: Option<ContactListId>,
) -> Result<Result<(), ContactListStoreError>, StoreError> {
    let holder: Option<i64> = tx
        .prepare_cached(
            "SELECT id FROM contact_lists WHERE name_key = ?1 AND (?2 IS NULL OR id <> ?2)",
        )
        .and_then(|mut s| {
            s.query_map(
                params![draft.name_key(), except.map(ContactListId::get)],
                |r| r.get(0),
            )?
            .next()
            .transpose()
        })
        .map_err(store_error)?;
    if let Some(holder) = holder
        && !load(tx, &[holder])?.is_empty()
    {
        return Ok(Err(ContactListStoreError::DuplicateName(
            draft.name().to_owned(),
        )));
    }
    let mut stmt = tx
        .prepare_cached("SELECT 1 FROM contacts WHERE id = ?1")
        .map_err(store_error)?;
    for &member in draft.members() {
        if !stmt.exists(params![member.get()]).map_err(store_error)? {
            return Ok(Err(ContactListStoreError::UnknownContact(member)));
        }
    }
    if let Some(holder) = holder {
        release_name(tx, holder)?;
    }
    Ok(Ok(()))
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
                if let Err(refusal) = admit(&tx, &draft, None)? {
                    return Ok(Err(refusal));
                }
                // Clé du nom, libre après l'admission ; le reste est écrit
                // par write.
                tx.execute(
                    "INSERT INTO contact_lists (name, name_key) VALUES ('', ?1)",
                    params![draft.name_key()],
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
                if let Err(refusal) = admit(&tx, &draft, Some(id))? {
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

/// Clé de tri telle que stockée, relue et reliée octet pour octet : une
/// clé corrompue (texte qui n'est pas de l'UTF-8, autre type) ne bloque
/// pas la lecture de la suite, et un texte reste comparé comme un texte.
enum RawKey {
    Text(Vec<u8>),
    Other(Value),
}

impl FromSql for RawKey {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        Ok(match value {
            ValueRef::Text(bytes) => Self::Text(bytes.to_vec()),
            ValueRef::Null => Self::Other(Value::Null),
            ValueRef::Integer(i) => Self::Other(Value::Integer(i)),
            ValueRef::Real(f) => Self::Other(Value::Real(f)),
            ValueRef::Blob(bytes) => Self::Other(Value::Blob(bytes.to_vec())),
        })
    }
}

impl ToSql for RawKey {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Borrowed(match self {
            Self::Text(bytes) => ValueRef::Text(bytes),
            Self::Other(value) => value.into(),
        }))
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
                // liste corrompue est écartée sans arrêter la lecture.
                let mut after: Option<(RawKey, i64)> =
                    cursor.map(|from| (RawKey::Text(from.sort_key.into_bytes()), from.id.get()));
                loop {
                    let fetch = i64::from(batch);
                    let mut rows: Vec<(i64, RawKey)> = match &after {
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
    use norelly_domain::ports::ContactBook;
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
    /// pas du texte, clé de nom qui n'est pas du texte, texte qui n'est pas
    /// de l'UTF-8 (nom, description, clé de nom), description de plusieurs
    /// lignes.
    const CORRUPTIONS: [&str; 8] = [
        "UPDATE contact_lists SET name = '' WHERE id = ?1",
        "UPDATE contact_lists SET name_key = 'zz' || name_key WHERE id = ?1",
        "UPDATE contact_lists SET name = x'00ff' WHERE id = ?1",
        "UPDATE contact_lists SET name_key = CAST(name_key AS BLOB) WHERE id = ?1",
        "UPDATE contact_lists SET name = CAST(x'ff' AS TEXT) WHERE id = ?1",
        "UPDATE contact_lists SET description = CAST(x'ff' AS TEXT) WHERE id = ?1",
        "UPDATE contact_lists SET name_key = name_key || CAST(x'ff' AS TEXT) WHERE id = ?1",
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

    #[tokio::test]
    async fn a_list_over_the_member_cap_is_skipped() -> Result<(), ContactListStoreError> {
        let book = book();
        let list = with_list(&book).await?;
        // Membres 2 à 101 ajoutés au premier : un de trop.
        book.db
            .write_for_tests(
                "WITH RECURSIVE n(i) AS (SELECT 2 UNION ALL SELECT i + 1 FROM n WHERE i < 101) \
                 INSERT INTO contacts (id, sort_key, search_text) SELECT i, 'c', 'c' FROM n; \
                 WITH RECURSIVE n(i) AS (SELECT 2 UNION ALL SELECT i + 1 FROM n WHERE i < 101) \
                 INSERT INTO contact_list_members (list_id, position, contact_id) \
                 SELECT 1, i, i FROM n;",
            )
            .await?;
        assert_eq!(MAX_CONTACT_LIST_MEMBERS, 100, "jeu de test à ajuster");
        assert_eq!(book.get_list(list.id()).await?, None);
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

    /// Nom, description, Cci et membres de la liste 1, tels que stockés.
    async fn raw_content(db: &Database) -> Result<(Vec<u8>, Vec<u8>, i64, Vec<i64>), StoreError> {
        db.read_for_tests(|c| {
            let row = c
                .query_row(
                    "SELECT CAST(name AS BLOB), CAST(description AS BLOB), bcc \
                     FROM contact_lists WHERE id = 1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .map_err(store_error)?;
            let members = c
                .prepare("SELECT contact_id FROM contact_list_members WHERE list_id = 1 ORDER BY position")
                .and_then(|mut s| s.query_map([], |r| r.get(0))?.collect())
                .map_err(store_error)?;
            Ok((row.0, row.1, row.2, members))
        })
        .await
    }

    #[tokio::test]
    async fn reusing_the_name_of_a_corrupted_list_keeps_its_content()
    -> Result<(), ContactListStoreError> {
        let book = book();
        let list = with_list(&book).await?;
        book.db
            .write_for_tests(
                "UPDATE contact_lists SET bcc = 1, description = CAST(x'ff' AS TEXT) WHERE id = 1;",
            )
            .await?;
        assert_eq!(book.get_list(list.id()).await?, None);
        let before = raw_content(&book.db).await?;
        let key = |db: Database| async move {
            db.read_for_tests(|c| {
                c.query_row("SELECT name_key FROM contact_lists WHERE id = 1", [], |r| {
                    r.get::<_, String>(0)
                })
                .map_err(store_error)
            })
            .await
        };
        // Écriture refusée pour une autre raison : la clé n'est pas libérée.
        let ghost = norelly_domain::contacts::ContactId::new(99).unwrap_or_else(|| panic!("id"));
        assert_eq!(
            book.create_list(&sample_list("l", &[ghost], false)).await,
            Err(ContactListStoreError::UnknownContact(ghost))
        );
        assert_eq!(key(book.db.clone()).await?, "l");
        let other = book.create_list(&sample_list("l", &[], false)).await;
        assert!(
            other.is_ok(),
            "nom d'une liste écartée encore pris : {other:?}"
        );
        let other = other?;
        assert_eq!(raw_content(&book.db).await?, before, "contenu touché");
        assert_eq!(book.get_list(list.id()).await?, None);
        // Libérée, la clé ne heurte ni un autre nom ni un renommage.
        book.update_list(other.id(), &sample_list("M", &[], false))
            .await?;
        book.create_list(&sample_list("L", &[], false)).await?;
        assert_eq!(raw_content(&book.db).await?, before, "contenu touché");
        // Réparée sous un nom libre, elle redevient lisible.
        let repaired = book
            .update_list(list.id(), &sample_list("N", list.draft().members(), true))
            .await?;
        assert_eq!(book.get_list(list.id()).await?, Some(repaired));
        assert!(book.db.integrity_problems().await?.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn damaged_rows_holding_reserved_keys_do_not_block_a_name()
    -> Result<(), ContactListStoreError> {
        let book = book();
        let list = with_list(&book).await?;
        // La liste « L » abîmée, puis des lignes abîmées qui portent la clé
        // que sa libération viserait, et une clé en saut de ligne.
        book.db
            .write_for_tests(
                "UPDATE contact_lists SET name = '' WHERE id = 1; \
                 INSERT INTO contact_lists (name, name_key) VALUES ('', char(1) || '1'); \
                 INSERT INTO contact_lists (name, name_key) VALUES ('', char(1) || '1' || char(1)); \
                 INSERT INTO contact_lists (name, name_key) VALUES ('', char(10) || 'l');",
            )
            .await?;
        let created = book.create_list(&sample_list("L", &[], false)).await;
        assert!(
            created.is_ok(),
            "nom bloqué par une ligne abîmée : {created:?}"
        );
        let created = created?;
        assert_eq!(book.get_list(created.id()).await?, Some(created));
        assert_eq!(book.get_list(list.id()).await?, None);
        Ok(())
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
