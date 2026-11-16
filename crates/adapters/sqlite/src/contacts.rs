//! Carnet d'adresses (ADR-012) : `ContactBook` côté écriture,
//! `ContactDirectory` côté lecture.
//!
//! Tri et filtre portent sur `sort_key` et `search_text`, calculés par le
//! domaine à l'écriture : la comparaison SQL (octets UTF-8, `instr`) est
//! celle du domaine, sans dépendre du `lower()` de SQLite (ASCII seul).
//! L'unicité d'une adresse dans le carnet est vérifiée avant l'écriture
//! (pour nommer l'adresse refusée) et garantie par le schéma.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use norelly_domain::address::EmailAddress;
use norelly_domain::contact_export::ContactSelection;
use norelly_domain::contact_import::ContactCompletion;
use norelly_domain::contacts::{
    AddressKey, Contact, ContactDraft, ContactFilter, ContactId, ContactInput, EmailInput,
    InvisibleCleanup, MAX_CONTACT_ID, PhoneInput, clean_invisible, clean_names_imitating_address,
};
use norelly_domain::ports::{
    ContactBook, ContactCursor, ContactDirectory, ContactListItem, ContactPage, ContactStoreError,
    ImportOutcome, MAX_CONTACT_LOOKUP, PageSize, StoreError,
};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::{Database, store_error};

/// Carnet d'adresses SQLite.
#[derive(Debug, Clone)]
pub struct SqliteContactBook {
    pub(crate) db: Database,
    /// Fiches invalides déjà signalées : une fois par fiche et par
    /// processus, quel que soit le nombre de relectures.
    reported: Arc<Mutex<HashSet<i64>>>,
}

impl SqliteContactBook {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self {
            db,
            reported: Arc::default(),
        }
    }

    /// Journalise les fiches écartées pas encore signalées, par leur seul
    /// identifiant : ni leur contenu ni le détail du refus.
    fn report(&self, skipped: &[i64]) {
        if skipped.is_empty() {
            return;
        }
        let mut reported = self.reported.lock().unwrap_or_else(PoisonError::into_inner);
        for &id in skipped {
            if reported.insert(id) {
                tracing::warn!(contact = id, "fiche du carnet invalide écartée");
            }
        }
    }
}

/// Colonnes d'une fiche, dans l'ordre lu par [`ContactRow::read`].
const CONTACT_COLUMNS: &str = "id, display_name, given_name, family_name, nickname, \
     organization, notes, favorite, sort_key, search_text";

/// Fiche brute, revalidée par le domaine.
struct ContactRow {
    id: i64,
    texts: [Option<String>; 6],
    favorite: bool,
    /// Clé de tri et texte cherché stockés.
    derived: (String, String),
    emails: Vec<EmailInput>,
    /// Clés stockées des adresses, dans le même ordre.
    keys: Vec<String>,
    phones: Vec<PhoneInput>,
    /// Une valeur de la fiche, de ses adresses ou de ses téléphones n'a pas
    /// le type attendu : la fiche est invalide.
    unreadable: bool,
}

impl ContactRow {
    /// Ligne de la fiche ; une valeur d'un type inattendu la rend invalide
    /// sans faire échouer la lecture des autres (l'identifiant, clé
    /// primaire entière, se lit toujours).
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        let fields = || -> rusqlite::Result<_> {
            Ok((
                [
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ],
                row.get(7)?,
                (row.get(8)?, row.get(9)?),
            ))
        };
        let (texts, favorite, derived, unreadable) = match fields() {
            Ok((texts, favorite, derived)) => (texts, favorite, derived, false),
            Err(_) => (Default::default(), false, Default::default(), true),
        };
        Ok(Self {
            id: row.get(0)?,
            texts,
            favorite,
            derived,
            emails: Vec::new(),
            keys: Vec::new(),
            phones: Vec::new(),
            unreadable,
        })
    }

    /// Fiche brute, non validée, et clés stockées de ses adresses.
    fn into_input(self) -> (ContactInput, Vec<String>) {
        let [
            display_name,
            given_name,
            family_name,
            nickname,
            organization,
            notes,
        ] = self.texts.map(Option::unwrap_or_default);
        let input = ContactInput {
            display_name,
            given_name,
            family_name,
            nickname,
            organization,
            notes,
            emails: self.emails,
            phones: self.phones,
            favorite: self.favorite,
        };
        (input, self.keys)
    }

    fn into_contact(self) -> Result<Contact, StoreError> {
        let raw = self.id;
        if self.unreadable {
            return Err(corrupted(raw, "type de valeur"));
        }
        let id = ContactId::new(raw).ok_or_else(|| corrupted(raw, "identifiant"))?;
        let derived = self.derived.clone();
        let (input, keys) = self.into_input();
        // Nom imitant une adresse, retiré à la relecture : tri et filtre
        // stockés ont pu être calculés avec lui (fiche écrite avant la
        // règle) ; ce sont alors ceux de la fiche telle qu'enregistrée.
        let as_stored = ContactDraft::keys_as_stored(&input);
        let draft = ContactDraft::from_stored(input).map_err(|e| corrupted(raw, &e.to_string()))?;
        // La contrainte d'unicité et la recherche par adresse portent sur
        // la clé stockée : elle doit être celle que calcule le domaine.
        let stored = keys.iter().map(String::as_str);
        if !draft.emails().iter().map(|e| e.key().as_str()).eq(stored) {
            return Err(corrupted(raw, "clé d'adresse"));
        }
        // L'ordre de la liste et le filtre portent sur les colonnes
        // stockées : elles doivent être celles que calcule le domaine, pour
        // la fiche relue ou, avec un nom retiré, pour la fiche telle
        // qu'enregistrée ; jamais d'autres.
        if derived != (draft.sort_key(), draft.search_text())
            && as_stored.as_ref() != Some(&derived)
        {
            return Err(corrupted(raw, "clé de tri ou texte cherché"));
        }
        Ok(draft.with_id(id))
    }
}

/// Nombre de fiches relues à la fois par [`clean_invisible_contacts`].
const CLEANUP_BATCH: i64 = 500;

/// Étape de la migration 18 : retire des fiches enregistrées les
/// caractères invisibles que la saisie refuse désormais, selon
/// [`clean_invisible`] (fiche réécrite, ou retirée sans adresse
/// restante). Une fiche illisible (type ou encodage d'une colonne) est
/// laissée telle quelle : elle ne doit pas empêcher d'ouvrir la base. De
/// même, comme à la migration 19, pour une fiche dont une adresse gardée
/// n'a pas pour clé stockée celle que calcule le domaine (clé modifiée à
/// la main) : la réécrire pourrait heurter l'unicité des clés.
pub(crate) fn clean_invisible_contacts(tx: &Transaction<'_>) -> Result<(), StoreError> {
    let (mut rewritten, mut removed, mut unreadable) = (0_usize, 0_usize, 0_usize);
    let mut invalid = 0_usize;
    let mut after = 0_i64;
    loop {
        let ids: Vec<i64> = tx
            .prepare_cached("SELECT id FROM contacts WHERE id > ?1 ORDER BY id LIMIT ?2")
            .and_then(|mut s| {
                s.query_map(params![after, CLEANUP_BATCH], |r| r.get(0))?
                    .collect()
            })
            .map_err(store_error)?;
        let Some(&last) = ids.last() else {
            break;
        };
        after = last;
        // Lot illisible : fiches relues une à une, les illisibles passées.
        let mut batch = match rows(tx, &ids) {
            Ok(batch) => batch,
            Err(_) => {
                let mut batch = HashMap::new();
                for &id in &ids {
                    match rows(tx, &[id]) {
                        Ok(one) => batch.extend(one),
                        Err(_) => unreadable += 1,
                    }
                }
                batch
            }
        };
        for id in ids {
            let Some(row) = batch.remove(&id) else {
                continue;
            };
            // Valeur d'un type inattendu : champs lus incomplets, la fiche
            // est laissée telle quelle (ni réécrite vide, ni retirée).
            if row.unreadable {
                unreadable += 1;
                continue;
            }
            let (input, keys) = row.into_input();
            match clean_invisible(input) {
                InvisibleCleanup::Keep => {}
                InvisibleCleanup::Rewrite(draft) => {
                    // Les clés stockées sont uniques dans le carnet : une clé
                    // recalculée déjà portée par cette fiche ne peut heurter
                    // aucune autre fiche, quel que soit l'ordre de passage.
                    let owned = draft
                        .emails()
                        .iter()
                        .all(|e| keys.iter().any(|k| k == e.key().as_str()));
                    if !owned {
                        invalid += 1;
                        continue;
                    }
                    write(tx, id, &draft)?;
                    rewritten += 1;
                }
                InvisibleCleanup::Remove => {
                    tx.execute("DELETE FROM contacts WHERE id = ?1", params![id])
                        .map_err(store_error)?;
                    removed += 1;
                }
            }
        }
    }
    // Des nombres seulement : aucune donnée de fiche dans le journal.
    tracing::info!(
        rewritten,
        removed,
        unreadable,
        invalid,
        "carnet : caractères invisibles retirés"
    );
    Ok(())
}

/// Étape de la migration 19 : retire des fiches enregistrées les noms qui
/// imitent une adresse, que la saisie refuse désormais, selon
/// [`clean_names_imitating_address`] (fiche réécrite, tri et filtre
/// recalculés). Comme à la migration 18, une fiche illisible (type ou
/// encodage d'une colonne) ou invalide pour une autre raison est laissée
/// telle quelle : elle ne doit pas empêcher d'ouvrir la base. C'est le cas
/// d'une fiche dont une clé d'adresse stockée n'est pas celle de son
/// adresse : la réécrire pourrait heurter l'unicité des clés.
pub(crate) fn drop_names_imitating_address(tx: &Transaction<'_>) -> Result<(), StoreError> {
    let (mut rewritten, mut removed, mut unreadable) = (0_usize, 0_usize, 0_usize);
    let mut invalid = 0_usize;
    let mut after = 0_i64;
    loop {
        let ids: Vec<i64> = tx
            .prepare_cached("SELECT id FROM contacts WHERE id > ?1 ORDER BY id LIMIT ?2")
            .and_then(|mut s| {
                s.query_map(params![after, CLEANUP_BATCH], |r| r.get(0))?
                    .collect()
            })
            .map_err(store_error)?;
        let Some(&last) = ids.last() else {
            break;
        };
        after = last;
        // Lot illisible : fiches relues une à une, les illisibles passées.
        let mut batch = match rows(tx, &ids) {
            Ok(batch) => batch,
            Err(_) => {
                let mut batch = HashMap::new();
                for &id in &ids {
                    match rows(tx, &[id]) {
                        Ok(one) => batch.extend(one),
                        Err(_) => unreadable += 1,
                    }
                }
                batch
            }
        };
        for id in ids {
            let Some(row) = batch.remove(&id) else {
                continue;
            };
            // Valeur d'un type inattendu : champs lus incomplets, la fiche
            // est laissée telle quelle, comme à la migration 18.
            if row.unreadable {
                unreadable += 1;
                continue;
            }
            let (input, keys) = row.into_input();
            let consistent =
                input.emails.len() == keys.len()
                    && input.emails.iter().zip(&keys).all(|(e, key)| {
                        AddressKey::parse(&e.address).is_ok_and(|k| k.as_str() == key)
                    });
            if !consistent {
                invalid += 1;
                continue;
            }
            match clean_names_imitating_address(input) {
                InvisibleCleanup::Keep => {}
                InvisibleCleanup::Rewrite(draft) => {
                    write(tx, id, &draft)?;
                    rewritten += 1;
                }
                InvisibleCleanup::Remove => {
                    tx.execute("DELETE FROM contacts WHERE id = ?1", params![id])
                        .map_err(store_error)?;
                    removed += 1;
                }
            }
        }
    }
    // Des nombres seulement : aucune donnée de fiche dans le journal.
    tracing::info!(
        rewritten,
        removed,
        unreadable,
        invalid,
        "carnet : noms imitant une adresse retirés"
    );
    Ok(())
}

fn corrupted(id: i64, detail: &str) -> StoreError {
    StoreError(format!("contact {id} corrompu : {detail}"))
}

/// Tableau JSON des valeurs, lu en SQL par `json_each` (liste de taille
/// variable passée en un seul paramètre lié).
pub(crate) fn json_list<T: serde::Serialize>(values: &[T]) -> Result<String, StoreError> {
    serde_json::to_string(values).map_err(|e| StoreError(e.to_string()))
}

/// Fiches `ids`, dans cet ordre (inconnues omises) ; une fiche invalide
/// fait échouer la lecture.
fn load(c: &Connection, ids: &[i64]) -> Result<Vec<Contact>, StoreError> {
    load_rows(c, ids)?
        .into_iter()
        .map(ContactRow::into_contact)
        .collect()
}

/// Fiches valides parmi `ids`, dans cet ordre (inconnues omises), et
/// identifiants des fiches invalides écartées.
fn load_valid(c: &Connection, ids: &[i64]) -> Result<(Vec<Contact>, Vec<i64>), StoreError> {
    let mut contacts = Vec::new();
    let mut skipped = Vec::new();
    for row in load_rows(c, ids)? {
        let id = row.id;
        match row.into_contact() {
            Ok(contact) => contacts.push(contact),
            Err(_) => skipped.push(id),
        }
    }
    Ok((contacts, skipped))
}

/// Lignes brutes des fiches `ids`, dans cet ordre (inconnues omises).
fn load_rows(c: &Connection, ids: &[i64]) -> Result<Vec<ContactRow>, StoreError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut rows = rows(c, ids)?;
    Ok(ids.iter().filter_map(|id| rows.remove(id)).collect())
}

/// Fiches brutes `ids`, par identifiant (inconnues omises). Une valeur
/// d'un type inattendu marque sa fiche illisible (`unreadable`) sans
/// faire échouer la lecture des autres.
fn rows(c: &Connection, ids: &[i64]) -> Result<HashMap<i64, ContactRow>, StoreError> {
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
            let email = || -> rusqlite::Result<_> {
                Ok((
                    EmailInput {
                        address: r.get(1)?,
                        label: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                        preferred: r.get(3)?,
                    },
                    r.get::<_, String>(4)?,
                ))
            };
            Ok((r.get::<_, i64>(0)?, email().ok()))
        })
        .map_err(store_error)?;
    for email in emails {
        let (id, email) = email.map_err(store_error)?;
        if let Some(row) = rows.get_mut(&id) {
            match email {
                Some((email, key)) => {
                    row.emails.push(email);
                    row.keys.push(key);
                }
                None => row.unreadable = true,
            }
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
            let phone = || -> rusqlite::Result<_> {
                Ok(PhoneInput {
                    number: r.get(1)?,
                    label: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                })
            };
            Ok((r.get::<_, i64>(0)?, phone().ok()))
        })
        .map_err(store_error)?;
    for phone in phones {
        let (id, phone) = phone.map_err(store_error)?;
        if let Some(row) = rows.get_mut(&id) {
            match phone {
                Some(phone) => row.phones.push(phone),
                None => row.unreadable = true,
            }
        }
    }
    Ok(rows)
}

/// Identifiant de la fiche tout juste insérée, au plus [`MAX_CONTACT_ID`].
/// Au-delà (séquence épuisée), la création est refusée : la transaction,
/// non validée, est annulée. Un identifiant n'est jamais réattribué, une
/// fiche restée ouverte pourrait encore le désigner.
fn new_id(tx: &Transaction<'_>) -> Result<i64, StoreError> {
    let raw = tx.last_insert_rowid();
    if raw > MAX_CONTACT_ID {
        return Err(StoreError("carnet plein : identifiants épuisés".into()));
    }
    Ok(raw)
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
                let raw = new_id(&tx)?;
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
        complete: &[ContactCompletion],
    ) -> Result<ImportOutcome, ContactStoreError> {
        let create = create.to_vec();
        let complete = complete.to_vec();
        self.db
            .run(move |c| {
                let tx = c.transaction().map_err(store_error)?;
                // Sans commit, la transaction est annulée à sa fermeture.
                // Chaque contact à compléter tel qu'il a été lu pour
                // calculer le plan : une modification entre-temps serait
                // écrasée.
                let ids: Vec<i64> = complete.iter().map(|c| c.id().get()).collect();
                let current: HashMap<i64, Contact> = load(&tx, &ids)?
                    .into_iter()
                    .map(|c| (c.id().get(), c))
                    .collect();
                for contact in &complete {
                    match current.get(&contact.id().get()) {
                        None => return Ok(Err(ContactStoreError::NotFound)),
                        Some(c) if c != contact.read() => {
                            return Ok(Err(ContactStoreError::Conflict));
                        }
                        Some(_) => {}
                    }
                }
                for contact in &complete {
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
                    let raw = new_id(&tx)?;
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
/// Suite d'une lecture : après la ligne `?4`, dont SQL relit la clé de
/// tri (jamais relue par Rust : elle peut être illisible).
const PAGE_AFTER_ROW: &str = "SELECT id FROM contacts \
     WHERE (?1 = 0 OR favorite = 1) AND (?2 = '' OR instr(search_text, ?2) > 0) \
       AND (sort_key, id) > ((SELECT sort_key FROM contacts WHERE id = ?4), ?4) \
     ORDER BY sort_key, id LIMIT ?3";
/// Lignes lues au plus pour une page, en lots de la taille de la page
/// plus une : une longue suite de fiches invalides arrête la liste au lieu
/// d'être parcourue à chaque lecture.
const MAX_PAGE_BATCHES: usize = 10;

impl ContactDirectory for SqliteContactBook {
    async fn page(
        &self,
        filter: &ContactFilter,
        cursor: Option<ContactCursor>,
        limit: PageSize,
    ) -> Result<ContactPage, StoreError> {
        let limit = limit.get();
        if limit == 0 {
            return Ok(ContactPage::default());
        }
        let filter = filter.clone();
        let (contacts, skipped, next) = self
            .db
            .read(move |c| {
                // Une fiche valide de plus : savoir s'il existe une suite.
                let wanted = limit as usize + 1;
                let fetch = i64::from(limit) + 1;
                let favorites = filter.favorites_only();
                let text = filter.text();
                // Une seule transaction : la ligne de reprise d'un lot ne
                // disparaît pas avant le lot suivant.
                let tx = c.transaction().map_err(store_error)?;
                let mut contacts = Vec::new();
                let mut skipped = Vec::new();
                let mut last_row: Option<i64> = None;
                let mut more = false;
                for _ in 0..MAX_PAGE_BATCHES {
                    let ids: Vec<i64> = match (last_row, &cursor) {
                        (Some(row), _) => tx.prepare_cached(PAGE_AFTER_ROW).and_then(|mut s| {
                            s.query_map(params![favorites, text, fetch, row], |r| r.get(0))?
                                .collect()
                        }),
                        (None, None) => tx.prepare_cached(PAGE_FIRST).and_then(|mut s| {
                            s.query_map(params![favorites, text, fetch], |r| r.get(0))?
                                .collect()
                        }),
                        (None, Some(after)) => tx.prepare_cached(PAGE_AFTER).and_then(|mut s| {
                            s.query_map(
                                params![favorites, text, fetch, after.sort_key, after.id.get()],
                                |r| r.get(0),
                            )?
                            .collect()
                        }),
                    }
                    .map_err(store_error)?;
                    let (valid, invalid) = load_valid(&tx, &ids)?;
                    contacts.extend(valid);
                    skipped.extend(invalid);
                    if contacts.len() > limit as usize {
                        more = true;
                        break;
                    }
                    // Moins de lignes que demandé : fin de la liste.
                    if ids.len() < wanted {
                        break;
                    }
                    last_row = ids.last().copied();
                }
                contacts.truncate(limit as usize);
                // Suite : après la clé de tri stockée de la dernière fiche,
                // celle selon laquelle SQL ordonne la liste (celle de la
                // fiche relue peut différer : nom retiré à la relecture).
                let next = match contacts.last().filter(|_| more) {
                    Some(last) => {
                        let sort_key: String = tx
                            .prepare_cached("SELECT sort_key FROM contacts WHERE id = ?1")
                            .and_then(|mut s| s.query_row(params![last.id().get()], |r| r.get(0)))
                            .map_err(store_error)?;
                        Some(ContactCursor {
                            sort_key,
                            id: last.id(),
                        })
                    }
                    None => None,
                };
                Ok((contacts, skipped, next))
            })
            .await?;
        self.report(&skipped);
        Ok(ContactPage {
            items: contacts.iter().map(ContactListItem::from).collect(),
            next,
        })
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
        let (contacts, skipped) = self
            .db
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
                load_valid(c, &ids)
            })
            .await?;
        self.report(&skipped);
        Ok(contacts)
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
        // Export et recherche des doublons : une fiche invalide, invisible
        // partout ailleurs, est écartée (signalée) sans bloquer les autres.
        let (contacts, skipped) = self
            .db
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
                load_valid(c, &ids)
            })
            .await?;
        self.report(&skipped);
        Ok(contacts)
    }
}

#[cfg(test)]
mod tests {
    use norelly_test_support::contacts_contracts::{
        contact_book_contract, contact_corruption_contract, contact_import_contract,
        contact_page_size_contract, contact_selection_contract, full_contact, sample_contact,
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

    #[tokio::test]
    async fn contact_book_passes_page_size_contract() -> Result<(), ContactStoreError> {
        contact_page_size_contract(&book()).await
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
    async fn a_stored_name_imitating_an_address_is_dropped_on_read() -> Result<(), ContactStoreError>
    {
        let book = book();
        let a = book
            .create(&sample_contact("Alice", &["a@example.com"]))
            .await?;
        book.db
            .write_for_tests(
                "UPDATE contacts SET display_name = 'ceo@bank.example', \
                 sort_key = 'ceo@bank.example', \
                 search_text = 'ceo@bank.example' || char(10) || 'a@example.com';",
            )
            .await?;
        let read = book
            .get(a.id())
            .await?
            .unwrap_or_else(|| panic!("contact absent"));
        assert_eq!(read.draft().display_name_field(), None);
        assert_eq!(read.draft().display_name(), "a@example.com");
        Ok(())
    }

    /// Fiche enregistrée avec un nom qui imite une adresse (règle élargie
    /// après son écriture) : relue sans ce nom, elle reste dans la liste
    /// (ses tri et filtre stockés, calculés avec ce nom, ne la rendent pas
    /// invalide) ; une fiche invalide pour une autre raison est écartée
    /// seule.
    #[tokio::test]
    async fn a_stored_name_imitating_an_address_keeps_its_card_listed()
    -> Result<(), ContactStoreError> {
        let book = book();
        let alice = book
            .create(&sample_contact("Alice", &["alice@example.com"]))
            .await?;
        let bob = book
            .create(&sample_contact("Bob", &["bob@example.com"]))
            .await?;
        let carole = book
            .create(&sample_contact("Carole", &["carole@example.com"]))
            .await?;
        let (alice_id, bob_id) = (alice.id().get(), bob.id().get());
        book.db
            .run(move |c| {
                c.execute(
                    "UPDATE contacts SET display_name = 'ceo@bank.example', \
                     sort_key = 'ceo@bank.example', \
                     search_text = 'ceo@bank.example' || char(10) || 'alice@example.com' \
                     WHERE id = ?1",
                    params![alice_id],
                )
                .map_err(store_error)?;
                c.execute(
                    "UPDATE contact_emails SET address = 'pas une adresse' \
                     WHERE contact_id = ?1",
                    params![bob_id],
                )
                .map_err(store_error)
            })
            .await?;
        let page = book
            .page(&ContactFilter::default(), None, PageSize::new(10))
            .await?;
        let ids: Vec<ContactId> = page.items.iter().map(|i| i.id).collect();
        // Rangée sous son nom enregistré (« ceo@bank.example »).
        assert_eq!(ids, vec![carole.id(), alice.id()]);
        let read = book
            .get(alice.id())
            .await?
            .unwrap_or_else(|| panic!("fiche absente"));
        assert_eq!(read.draft().display_name_field(), None);
        assert!(book.get(bob.id()).await.is_err(), "fiche invalide relue");
        Ok(())
    }

    /// Un nom imitant une adresse ne dispense pas du contrôle des colonnes
    /// de tri et de recherche : forgées, elles rendent la fiche invalide
    /// (écartée seule), même si la fiche porte un tel nom.
    #[tokio::test]
    async fn a_name_imitating_an_address_does_not_excuse_forged_sort_or_search_columns()
    -> Result<(), ContactStoreError> {
        let book = book();
        let alice = book
            .create(&sample_contact("Alice", &["alice@example.com"]))
            .await?;
        let zed = book
            .create(&sample_contact("Zed", &["zed@example.com"]))
            .await?;
        let zed_id = zed.id().get();
        book.db
            .run(move |c| {
                c.execute(
                    "UPDATE contacts SET display_name = 'x@y', sort_key = '', \
                     search_text = 'patron' WHERE id = ?1",
                    params![zed_id],
                )
                .map_err(store_error)
            })
            .await?;
        let page = book
            .page(&ContactFilter::default(), None, PageSize::new(10))
            .await?;
        let ids: Vec<ContactId> = page.items.iter().map(|i| i.id).collect();
        assert_eq!(ids, vec![alice.id()]);
        assert!(book.get(zed.id()).await.is_err(), "fiche forgée relue");
        let found = book
            .page(
                &ContactFilter::new("patron", false),
                None,
                PageSize::new(10),
            )
            .await?;
        assert!(found.items.is_empty(), "{:?}", found.items);
        Ok(())
    }

    /// Fiche relue sans son nom imitant une adresse : sa clé de tri stockée
    /// (calculée avec ce nom) n'est pas celle de la fiche relue. La liste,
    /// lue page à page, n'en saute ni n'en répète aucune.
    #[tokio::test]
    async fn pages_neither_skip_nor_repeat_around_a_card_read_without_its_name()
    -> Result<(), ContactStoreError> {
        let book = book();
        let mut ids = Vec::new();
        for name in ["Alice", "Bob", "Carole", "Zed"] {
            let address = format!("{}@example.com", name.to_lowercase());
            ids.push(book.create(&sample_contact(name, &[&address])).await?.id());
        }
        // Alice, rangée sous « ceo@bank.example » (entre Carole et Zed) ;
        // relue, sa clé serait « alice@example.com ».
        let alice_id = ids[0].get();
        book.db
            .run(move |c| {
                c.execute(
                    "UPDATE contacts SET display_name = 'ceo@bank.example', \
                     sort_key = 'ceo@bank.example', \
                     search_text = 'ceo@bank.example' || char(10) || 'alice@example.com' \
                     WHERE id = ?1",
                    params![alice_id],
                )
                .map_err(store_error)
            })
            .await?;
        let mut seen = Vec::new();
        let mut cursor = None;
        for _ in 0..10 {
            let page = book
                .page(&ContactFilter::default(), cursor, PageSize::new(1))
                .await?;
            seen.extend(page.items.iter().map(|i| i.id));
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(seen, vec![ids[1], ids[2], ids[0], ids[3]]);
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
        // Écartée de la liste, sans la faire échouer.
        assert_eq!(
            book.page(&ContactFilter::default(), None, PageSize::new(10))
                .await?,
            ContactPage::default()
        );
        // Une fiche sans adresse est invalide.
        book.db
            .write_for_tests("DELETE FROM contact_emails;")
            .await?;
        assert!(book.get(a.id()).await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn contact_book_passes_corruption_contract() -> Result<(), ContactStoreError> {
        let book = book();
        contact_corruption_contract(&book, |id| {
            let db = book.db.clone();
            async move {
                db.run(move |c| {
                    c.execute(
                        "UPDATE contact_emails SET address = 'pas une adresse' \
                         WHERE contact_id = ?1",
                        params![id.get()],
                    )
                    .map(drop)
                    .map_err(store_error)
                })
                .await
            }
        })
        .await
    }

    /// Fiche d'identifiant `id` écrite directement en base, valide par
    /// ailleurs.
    async fn insert_raw(book: &SqliteContactBook, id: i64, name: &'static str) {
        let key = format!("{}@example.com", name.to_lowercase());
        let draft = sample_contact(name, &[&key]);
        book.db
            .run(move |c| {
                c.execute(
                    "INSERT INTO contacts (id, display_name, sort_key, search_text) \
                     VALUES (?1, ?2, ?3, ?4)",
                    params![id, name, draft.sort_key(), draft.search_text()],
                )
                .map_err(store_error)?;
                c.execute(
                    "INSERT INTO contact_emails (contact_id, position, address, address_key, \
                     preferred) VALUES (?1, 0, ?2, ?2, 0)",
                    params![id, key],
                )
                .map(drop)
                .map_err(store_error)
            })
            .await
            .unwrap_or_else(|e| panic!("{e}"));
    }

    fn address_key(raw: &str) -> AddressKey {
        AddressKey::parse(raw).unwrap_or_else(|e| panic!("{raw} : {e}"))
    }

    /// Les identifiants de fiche tiennent sur 32 bits (ceux de l'interface) :
    /// une fiche au-delà est écartée comme une fiche invalide.
    #[tokio::test]
    async fn contact_ids_beyond_32_bits_are_skipped() -> Result<(), ContactStoreError> {
        let book = book();
        let alice = book
            .create(&sample_contact("Alice", &["alice@example.com"]))
            .await?;
        insert_raw(&book, i64::from(u32::MAX), "Bob").await;
        insert_raw(&book, i64::from(u32::MAX) + 1, "Carole").await;
        let page = book
            .page(&ContactFilter::default(), None, PageSize::new(10))
            .await
            .unwrap_or_else(|e| panic!("page en échec : {e}"));
        let names: Vec<_> = page.items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["Alice", "Bob"]);
        let keys = [
            address_key("alice@example.com"),
            address_key("carole@example.com"),
        ];
        let found = book
            .by_addresses(&keys)
            .await
            .unwrap_or_else(|e| panic!("recherche en échec : {e}"));
        assert_eq!(found, vec![alice]);
        Ok(())
    }

    /// Le carnet n'attribue jamais d'identifiant au-delà de 32 bits : la
    /// séquence épuisée, la création est refusée sans rien écrire (jamais
    /// la réutilisation d'un identifiant, qu'une fiche restée ouverte
    /// pourrait encore désigner).
    #[tokio::test]
    async fn ids_beyond_32_bits_are_never_assigned() -> Result<(), ContactStoreError> {
        let book = book();
        insert_raw(&book, i64::from(u32::MAX) - 1, "Bob").await;
        let alice = book
            .create(&sample_contact("Alice", &["alice@example.com"]))
            .await?;
        assert_eq!(alice.id().get(), i64::from(u32::MAX));
        let before = book.db.read_for_tests(|c| count(c, "contacts")).await?;
        let created = book
            .create(&sample_contact("Carole", &["carole@example.com"]))
            .await;
        assert!(
            created.is_err(),
            "identifiant hors limites attribué : {created:?}"
        );
        let imported = book
            .import(&[sample_contact("David", &["david@example.com"])], &[])
            .await;
        assert!(
            imported.is_err(),
            "identifiant hors limites attribué : {imported:?}"
        );
        let after = book.db.read_for_tests(|c| count(c, "contacts")).await?;
        assert_eq!(after, before);
        assert!(book.db.integrity_problems().await?.is_empty());
        Ok(())
    }

    /// Contrat joué en corrompant par des requêtes SQL, une liste de
    /// requêtes par appel de `corrupt` (la dernière se répète).
    async fn corrupt_with(
        book: &SqliteContactBook,
        rounds: &[&[&'static str]],
    ) -> Result<(), ContactStoreError> {
        let round = std::sync::atomic::AtomicUsize::new(0);
        contact_corruption_contract(book, |id| {
            let db = book.db.clone();
            let n = round.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let sql: Vec<&'static str> = rounds[n.min(rounds.len() - 1)].to_vec();
            async move {
                db.run(move |c| {
                    for statement in sql {
                        c.execute(statement, params![id.get()])
                            .map_err(store_error)?;
                    }
                    Ok(())
                })
                .await
            }
        })
        .await
    }

    /// Une valeur d'un type inattendu n'écarte, elle aussi, que sa fiche ;
    /// une clé de tri illisible n'arrête pas la pagination.
    #[tokio::test]
    async fn contact_book_passes_corruption_contract_with_wrong_types()
    -> Result<(), ContactStoreError> {
        corrupt_with(
            &book(),
            &[
                &["UPDATE contacts SET display_name = x'00ff' WHERE id = ?1"],
                &[
                    "UPDATE contacts SET sort_key = CAST(x'ff' AS TEXT) WHERE id = ?1",
                    "UPDATE contact_emails SET address = x'00' WHERE contact_id = ?1",
                ],
            ],
        )
        .await
    }

    /// Clé de tri ou texte cherché stockés qui ne sont pas ceux du domaine :
    /// la fiche est invalide (elle serait mal placée, ou proposée à tort).
    #[tokio::test]
    async fn contact_book_passes_corruption_contract_with_stale_derived_columns()
    -> Result<(), ContactStoreError> {
        corrupt_with(
            &book(),
            &[
                &["UPDATE contacts SET search_text = search_text || ' patron' WHERE id = ?1"],
                &["UPDATE contacts SET sort_key = x'00' WHERE id = ?1"],
            ],
        )
        .await
    }

    /// Une longue suite de fiches invalides ne rend pas la lecture d'une
    /// page sans borne : au-delà de dix lots, la liste s'arrête.
    #[tokio::test]
    async fn a_long_run_of_invalid_contacts_bounds_the_page_read() -> Result<(), ContactStoreError>
    {
        let book = book();
        book.create(&sample_contact("Aaron", &["aaron@example.com"]))
            .await?;
        for n in 0..30 {
            let name = format!("B{n:02}");
            let address = format!("b{n}@example.com");
            book.create(&sample_contact(&name, &[&address])).await?;
        }
        book.create(&sample_contact("Zoe", &["zoe@example.com"]))
            .await?;
        book.db
            .write_for_tests(
                "UPDATE contact_emails SET address = 'invalide' \
                 WHERE contact_id IN (SELECT id FROM contacts WHERE display_name LIKE 'B%');",
            )
            .await?;
        // Une fiche par page : lots de 2 lignes, 20 lignes lues au plus.
        let first = book
            .page(&ContactFilter::default(), None, PageSize::new(1))
            .await?;
        let names: Vec<_> = first.items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["Aaron"]);
        assert_eq!(first.next, None, "lecture poursuivie au-delà de la borne");
        // Avec des pages plus grandes, la fiche suivante est atteinte.
        let all = book
            .page(&ContactFilter::default(), None, PageSize::new(10))
            .await?;
        let names: Vec<_> = all.items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["Aaron", "Zoe"]);
        Ok(())
    }

    /// Journal partagé entre le test et l'abonné `tracing`.
    #[derive(Clone, Default)]
    struct CapturedLogs(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for CapturedLogs {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Une fiche invalide est signalée une seule fois, par son seul
    /// identifiant : ni son contenu ni le détail de l'erreur.
    #[tokio::test]
    async fn an_invalid_contact_is_logged_once_without_its_content() -> Result<(), ContactStoreError>
    {
        let logs = CapturedLogs::default();
        let writer = logs.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        let book = book();
        let bob = book
            .create(&sample_contact("Bob Secret", &["bob.secret@example.com"]))
            .await?;
        book.db
            .write_for_tests("UPDATE contact_emails SET address = 'adresse-secrete';")
            .await?;
        // Exécuteur à un seul fil (`tokio::test`) : l'abonné du fil du
        // test reçoit les événements des lectures du carnet (installé après
        // l'ouverture de la base, qui journalise ses migrations).
        let _guard = tracing::subscriber::set_default(subscriber);
        let key = address_key("bob.secret@example.com");
        for _ in 0..3 {
            for filter in [ContactFilter::default(), ContactFilter::new("bob", false)] {
                book.page(&filter, None, PageSize::new(10))
                    .await
                    .unwrap_or_else(|e| panic!("page en échec : {e}"));
            }
            book.by_addresses(std::slice::from_ref(&key))
                .await
                .unwrap_or_else(|e| panic!("recherche en échec : {e}"));
        }
        let text = String::from_utf8_lossy(
            &logs
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
        .into_owned();
        assert_eq!(text.lines().count(), 1, "{text}");
        assert!(text.contains(&format!("contact={}", bob.id())), "{text}");
        for secret in ["Secret", "secret", "adresse"] {
            assert!(!text.contains(secret), "contenu journalisé : {text}");
        }
        Ok(())
    }
}
