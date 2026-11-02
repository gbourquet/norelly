//! Boîtes et messages : [`SqliteMailStore`] implémente `MailStore`
//! (écriture, via le rédacteur) et les ports de requête d'écran
//! `FolderTreeQuery` et `MessageListQuery` (lecture, via le pool).
//!
//! Les valeurs relues sont revalidées par le domaine (UID, drapeaux, noms,
//! rôles, textes) : une base modifiée à la main ne produit pas de valeur
//! hors des invariants du domaine.

use std::collections::BTreeSet;

use norelly_domain::account::AccountId;
use norelly_domain::fetch_policy::{MimePart, PartPath};
use norelly_domain::mailbox::{
    Correspondent, Flags, Keyword, MAX_SUBJECT_CHARS, MailboxId, MailboxName, MailboxRole, ModSeq,
    RemoteMailbox, Uid, UidValidity, header_text,
};
use norelly_domain::ports::{
    BlobId, FolderNode, FolderTreeQuery, MAX_PAGE_SIZE, MailStore, MessageId, MessageListItem,
    MessageListQuery, MessagePage, PageCursor, PendingBody, StoreError, StoredBody, StoredMailbox,
    StoredMessage, SyncBatch,
};
use norelly_domain::sync::{LocalMailbox, MailboxSyncState};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::{Database, store_error};

/// Stockage des boîtes et messages.
#[derive(Debug, Clone)]
pub struct SqliteMailStore {
    db: Database,
}

impl SqliteMailStore {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

fn corrupted(what: &str) -> StoreError {
    StoreError(format!("donnée corrompue : {what}"))
}

fn json<T: serde::Serialize>(value: &T) -> Result<String, StoreError> {
    serde_json::to_string(value).map_err(|e| StoreError(format!("sérialisation : {e}")))
}

fn keywords_text(flags: &Flags) -> String {
    flags
        .keywords()
        .iter()
        .map(Keyword::as_str)
        .collect::<Vec<_>>()
        .join(" ")
}

fn flags_from(bits: i64, keywords: &str) -> Flags {
    Flags::from_parts(u8::try_from(bits & 0x1f).unwrap_or(0), keywords.split(' '))
}

fn i64_of(v: u32) -> i64 {
    i64::from(v)
}

fn modseq_sql(m: Option<ModSeq>) -> Option<i64> {
    m.and_then(|m| i64::try_from(m.get()).ok())
}

fn uid_of(raw: i64) -> Result<Uid, StoreError> {
    u32::try_from(raw)
        .ok()
        .and_then(Uid::new)
        .ok_or_else(|| corrupted("UID"))
}

fn state_of(validity: Option<i64>, modseq: Option<i64>, next: Option<i64>) -> MailboxSyncState {
    MailboxSyncState {
        uid_validity: validity
            .and_then(|v| u32::try_from(v).ok())
            .and_then(UidValidity::new),
        highest_modseq: modseq
            .and_then(|v| u64::try_from(v).ok())
            .and_then(ModSeq::new),
        uid_next: next.and_then(|v| u32::try_from(v).ok()).and_then(Uid::new),
    }
}

fn mailbox_id(raw: i64) -> Result<MailboxId, StoreError> {
    MailboxId::new(raw).ok_or_else(|| corrupted("identifiant de boîte"))
}

fn delimiter_of(raw: Option<String>) -> Option<char> {
    let raw = raw?;
    let mut chars = raw.chars();
    let c = chars.next()?;
    (chars.next().is_none() && !c.is_control()).then_some(c)
}

/// Vérifie que la boîte existe (sinon erreur, plutôt qu'une écriture
/// silencieusement perdue).
fn ensure_mailbox(tx: &Transaction<'_>, mailbox: MailboxId) -> Result<(), StoreError> {
    let found: Option<i64> = tx
        .query_row(
            "SELECT id FROM mailboxes WHERE id = ?1",
            [mailbox.get()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    found
        .map(|_| ())
        .ok_or_else(|| StoreError(format!("boîte {mailbox} inconnue")))
}

fn write_body(tx: &Transaction<'_>, message: i64, body: &StoredBody) -> Result<(), StoreError> {
    tx.execute("DELETE FROM message_parts WHERE message_id = ?1", [message])
        .map_err(store_error)?;
    let (state, blob) = match body {
        StoredBody::Missing => ("missing", None),
        StoredBody::Full(id) => ("full", Some(id.as_str())),
        StoredBody::Partial { header, .. } => ("partial", Some(header.as_str())),
    };
    tx.execute(
        "UPDATE messages SET body_state = ?2, body_blob = ?3 WHERE id = ?1",
        params![message, state, blob],
    )
    .map_err(store_error)?;
    if let StoredBody::Partial {
        parts, deferred, ..
    } = body
    {
        let mut stmt = tx
            .prepare_cached(
                "INSERT OR REPLACE INTO message_parts (message_id, section, blob) \
                 VALUES (?1, ?2, ?3)",
            )
            .map_err(store_error)?;
        for (path, blob) in parts {
            stmt.execute(params![message, path.to_string(), blob.as_str()])
                .map_err(store_error)?;
        }
        for path in deferred {
            stmt.execute(params![message, path.to_string(), Option::<&str>::None])
                .map_err(store_error)?;
        }
    }
    Ok(())
}

fn insert_message(
    tx: &Transaction<'_>,
    mailbox: MailboxId,
    message: &StoredMessage,
) -> Result<(), StoreError> {
    let meta = &message.meta;
    let h = &meta.headers;
    tx.execute(
        "DELETE FROM messages WHERE mailbox_id = ?1 AND uid = ?2",
        params![mailbox.get(), i64_of(meta.uid.get())],
    )
    .map_err(store_error)?;
    let from = h.from.first().cloned().unwrap_or_default();
    let structure = meta.structure.as_ref().map(json).transpose()?;
    let mut stmt = tx
        .prepare_cached(
            "INSERT INTO messages (mailbox_id, uid, modseq, flags, keywords, internal_date, \
             sort_date, size, subject, from_name, from_address, to_json, cc_json, message_id, \
             in_reply_to, refs, header_date, has_attachments, structure) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, \
             ?17, ?18, ?19)",
        )
        .map_err(store_error)?;
    stmt.execute(params![
        mailbox.get(),
        i64_of(meta.uid.get()),
        modseq_sql(meta.modseq),
        i64::from(meta.flags.system_bits()),
        keywords_text(&meta.flags),
        meta.internal_date,
        meta.sort_date(),
        i64_of(meta.size),
        h.subject,
        from.name,
        from.address,
        json(&h.to)?,
        json(&h.cc)?,
        h.message_id,
        json(&h.in_reply_to)?,
        json(&h.references)?,
        h.date,
        meta.has_attachments(),
        structure,
    ])
    .map_err(store_error)?;
    let id = tx.last_insert_rowid();
    if message.body != StoredBody::Missing {
        write_body(tx, id, &message.body)?;
    }
    Ok(())
}

fn apply_batch(
    conn: &mut Connection,
    mailbox: MailboxId,
    batch: &SyncBatch,
) -> Result<(), StoreError> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(store_error)?;
    ensure_mailbox(&tx, mailbox)?;
    if batch.reset {
        tx.execute(
            "DELETE FROM messages WHERE mailbox_id = ?1",
            [mailbox.get()],
        )
        .map_err(store_error)?;
    }
    {
        let mut remove = tx
            .prepare_cached("DELETE FROM messages WHERE mailbox_id = ?1 AND uid = ?2")
            .map_err(store_error)?;
        for uid in &batch.removed {
            remove
                .execute(params![mailbox.get(), i64_of(uid.get())])
                .map_err(store_error)?;
        }
        let mut update = tx
            .prepare_cached(
                "UPDATE messages SET flags = ?3, keywords = ?4, modseq = ?5 \
                 WHERE mailbox_id = ?1 AND uid = ?2",
            )
            .map_err(store_error)?;
        for change in &batch.flag_changes {
            update
                .execute(params![
                    mailbox.get(),
                    i64_of(change.uid.get()),
                    i64::from(change.flags.system_bits()),
                    keywords_text(&change.flags),
                    modseq_sql(change.modseq),
                ])
                .map_err(store_error)?;
        }
    }
    for message in &batch.new {
        insert_message(&tx, mailbox, message)?;
    }
    let s = batch.state;
    tx.execute(
        "UPDATE mailboxes SET uid_validity = ?2, highest_modseq = ?3, uid_next = ?4 WHERE id = ?1",
        params![
            mailbox.get(),
            s.uid_validity.map(|v| i64_of(v.get())),
            modseq_sql(s.highest_modseq),
            s.uid_next.map(|u| i64_of(u.get())),
        ],
    )
    .map_err(store_error)?;
    tx.commit().map_err(store_error)
}

fn replace(
    conn: &mut Connection,
    account: AccountId,
    mailboxes: &[RemoteMailbox],
) -> Result<Vec<StoredMailbox>, StoreError> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(store_error)?;
    let wanted: BTreeSet<&str> = mailboxes.iter().map(|m| m.name.as_str()).collect();
    let existing: Vec<(i64, String)> = {
        let mut stmt = tx
            .prepare("SELECT id, name FROM mailboxes WHERE account_id = ?1")
            .map_err(store_error)?;
        let rows = stmt
            .query_map([account.get()], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(store_error)?;
        rows.collect::<Result<_, _>>().map_err(store_error)?
    };
    for (id, name) in &existing {
        if !wanted.contains(name.as_str()) {
            tx.execute("DELETE FROM mailboxes WHERE id = ?1", [id])
                .map_err(store_error)?;
        }
    }
    let mut out = Vec::with_capacity(mailboxes.len());
    let mut seen = BTreeSet::new();
    {
        let mut upsert = tx
            .prepare_cached(
                "INSERT INTO mailboxes (account_id, name, delimiter, role, selectable, position) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT (account_id, name) DO UPDATE SET delimiter = excluded.delimiter, \
                 role = excluded.role, selectable = excluded.selectable, \
                 position = excluded.position \
                 RETURNING id",
            )
            .map_err(store_error)?;
        for (position, mailbox) in mailboxes.iter().enumerate() {
            if !seen.insert(mailbox.name.as_str()) {
                continue;
            }
            let id: i64 = upsert
                .query_row(
                    params![
                        account.get(),
                        mailbox.name.as_str(),
                        mailbox.delimiter.map(String::from),
                        mailbox.role.as_str(),
                        mailbox.selectable,
                        i64::try_from(position).unwrap_or(i64::MAX),
                    ],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            out.push(StoredMailbox {
                id: mailbox_id(id)?,
                mailbox: mailbox.clone(),
            });
        }
    }
    tx.commit().map_err(store_error)?;
    Ok(out)
}

fn local(conn: &Connection, mailbox: MailboxId) -> Result<LocalMailbox, StoreError> {
    let state = conn
        .query_row(
            "SELECT uid_validity, highest_modseq, uid_next FROM mailboxes WHERE id = ?1",
            [mailbox.get()],
            |r| Ok(state_of(r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(store_error)?
        .unwrap_or_default();
    let mut stmt = conn
        .prepare_cached("SELECT uid, flags, keywords FROM messages WHERE mailbox_id = ?1")
        .map_err(store_error)?;
    let rows = stmt
        .query_map([mailbox.get()], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(store_error)?;
    let mut messages = std::collections::BTreeMap::new();
    for row in rows {
        let (uid, bits, keywords) = row.map_err(store_error)?;
        messages.insert(uid_of(uid)?, flags_from(bits, &keywords));
    }
    Ok(LocalMailbox { state, messages })
}

impl MailStore for SqliteMailStore {
    async fn replace_mailboxes(
        &self,
        account: AccountId,
        mailboxes: &[RemoteMailbox],
    ) -> Result<Vec<StoredMailbox>, StoreError> {
        let mailboxes = mailboxes.to_vec();
        self.db.run(move |c| replace(c, account, &mailboxes)).await
    }

    async fn local_mailbox(&self, mailbox: MailboxId) -> Result<LocalMailbox, StoreError> {
        // Lecture par le rédacteur : l'état doit refléter la dernière
        // écriture de la synchronisation en cours.
        self.db.run(move |c| local(c, mailbox)).await
    }

    async fn apply_sync(&self, mailbox: MailboxId, batch: &SyncBatch) -> Result<(), StoreError> {
        let batch = batch.clone();
        self.db.run(move |c| apply_batch(c, mailbox, &batch)).await
    }

    async fn missing_bodies(
        &self,
        mailbox: MailboxId,
        limit: usize,
    ) -> Result<Vec<PendingBody>, StoreError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.db
            .run(move |c| {
                let mut stmt = c
                    .prepare_cached(
                        "SELECT uid, size, structure FROM messages \
                         WHERE mailbox_id = ?1 AND body_state = 'missing' \
                         ORDER BY uid DESC LIMIT ?2",
                    )
                    .map_err(store_error)?;
                let rows = stmt
                    .query_map(params![mailbox.get(), limit], |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, i64>(1)?,
                            r.get::<_, Option<String>>(2)?,
                        ))
                    })
                    .map_err(store_error)?;
                let mut out = Vec::new();
                for row in rows {
                    let (uid, size, structure) = row.map_err(store_error)?;
                    out.push(PendingBody {
                        uid: uid_of(uid)?,
                        size: u32::try_from(size).unwrap_or(u32::MAX),
                        // Structure illisible : traitée comme inconnue.
                        structure: structure
                            .and_then(|s| serde_json::from_str::<MimePart>(&s).ok()),
                    });
                }
                Ok(out)
            })
            .await
    }

    async fn set_body(
        &self,
        mailbox: MailboxId,
        uid: Uid,
        body: &StoredBody,
    ) -> Result<(), StoreError> {
        let body = body.clone();
        self.db
            .run(move |c| {
                let tx = c
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(store_error)?;
                let id: Option<i64> = tx
                    .query_row(
                        "SELECT id FROM messages WHERE mailbox_id = ?1 AND uid = ?2",
                        params![mailbox.get(), i64_of(uid.get())],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(store_error)?;
                if let Some(id) = id {
                    write_body(&tx, id, &body)?;
                }
                tx.commit().map_err(store_error)
            })
            .await
    }
}

impl SqliteMailStore {
    /// Corps enregistré d'un message (tests et lecture en phase 5).
    pub async fn body_of(
        &self,
        mailbox: MailboxId,
        uid: Uid,
    ) -> Result<Option<StoredBody>, StoreError> {
        self.db
            .read(move |c| {
                let row: Option<(i64, String, Option<String>)> = c
                    .query_row(
                        "SELECT id, body_state, body_blob FROM messages \
                         WHERE mailbox_id = ?1 AND uid = ?2",
                        params![mailbox.get(), i64_of(uid.get())],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()
                    .map_err(store_error)?;
                let Some((id, state, blob)) = row else {
                    return Ok(None);
                };
                let blob = blob.as_deref().and_then(BlobId::parse);
                Ok(Some(match (state.as_str(), blob) {
                    ("full", Some(b)) => StoredBody::Full(b),
                    ("partial", Some(header)) => {
                        let mut stmt = c
                            .prepare_cached(
                                "SELECT section, blob FROM message_parts \
                                 WHERE message_id = ?1 ORDER BY section",
                            )
                            .map_err(store_error)?;
                        let rows = stmt
                            .query_map([id], |r| {
                                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
                            })
                            .map_err(store_error)?;
                        let (mut parts, mut deferred) = (Vec::new(), Vec::new());
                        for row in rows {
                            let (section, blob) = row.map_err(store_error)?;
                            let Some(path) = PartPath::parse(&section) else {
                                continue;
                            };
                            match blob.as_deref().and_then(BlobId::parse) {
                                Some(b) => parts.push((path, b)),
                                None => deferred.push(path),
                            }
                        }
                        StoredBody::Partial {
                            header,
                            parts,
                            deferred,
                        }
                    }
                    _ => StoredBody::Missing,
                }))
            })
            .await
    }
}

impl FolderTreeQuery for SqliteMailStore {
    async fn tree(&self, account: AccountId) -> Result<Vec<FolderNode>, StoreError> {
        self.db
            .read(move |c| {
                let mut stmt = c
                    .prepare_cached(
                        "SELECT m.id, m.name, m.delimiter, m.role, m.selectable, \
                         (SELECT COUNT(*) FROM messages WHERE mailbox_id = m.id), \
                         (SELECT COUNT(*) FROM messages WHERE mailbox_id = m.id \
                          AND (flags & 1) = 0) \
                         FROM mailboxes m WHERE m.account_id = ?1",
                    )
                    .map_err(store_error)?;
                let rows = stmt
                    .query_map([account.get()], |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, Option<String>>(2)?,
                            r.get::<_, String>(3)?,
                            r.get::<_, bool>(4)?,
                            r.get::<_, i64>(5)?,
                            r.get::<_, i64>(6)?,
                        ))
                    })
                    .map_err(store_error)?;
                let mut out = Vec::new();
                for row in rows {
                    let (id, name, delimiter, role, selectable, total, unread) =
                        row.map_err(store_error)?;
                    out.push(FolderNode {
                        id: mailbox_id(id)?,
                        name: MailboxName::parse(&name).map_err(|_| corrupted("nom de boîte"))?,
                        delimiter: delimiter_of(delimiter),
                        role: MailboxRole::parse(&role).ok_or_else(|| corrupted("rôle"))?,
                        selectable,
                        total: u32::try_from(total).unwrap_or(u32::MAX),
                        unread: u32::try_from(unread).unwrap_or(u32::MAX),
                    });
                }
                out.sort_by(|a, b| {
                    (a.role.rank(), a.name.as_str()).cmp(&(b.role.rank(), b.name.as_str()))
                });
                Ok(out)
            })
            .await
    }
}

const LIST_COLUMNS: &str = "id, uid, sort_date, from_name, from_address, subject, flags, \
                            keywords, has_attachments, size";

type ListRow = (
    i64,
    i64,
    i64,
    String,
    String,
    String,
    i64,
    String,
    bool,
    i64,
);

fn list_item(row: ListRow) -> Result<MessageListItem, StoreError> {
    let (id, uid, date, from_name, from_address, subject, bits, keywords, attachments, size) = row;
    let from = (!from_name.is_empty() || !from_address.is_empty())
        .then(|| Correspondent::new(&from_name, &from_address));
    Ok(MessageListItem {
        id: MessageId::new(id).ok_or_else(|| corrupted("identifiant de message"))?,
        uid: uid_of(uid)?,
        date,
        from,
        subject: header_text(&subject, MAX_SUBJECT_CHARS),
        flags: flags_from(bits, &keywords),
        has_attachments: attachments,
        size: u32::try_from(size).unwrap_or(u32::MAX),
    })
}

/// Page de messages (requête seule, sur une connexion donnée).
pub(crate) fn page_query(
    c: &Connection,
    mailbox: MailboxId,
    cursor: Option<PageCursor>,
    limit: u32,
) -> Result<MessagePage, StoreError> {
    let limit = limit.min(MAX_PAGE_SIZE);
    if limit == 0 {
        return Ok(MessagePage::default());
    }
    // Une ligne de plus pour savoir s'il reste une page.
    let fetch = i64::from(limit) + 1;
    let read = |r: &rusqlite::Row<'_>| -> rusqlite::Result<ListRow> {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
            r.get(7)?,
            r.get(8)?,
            r.get(9)?,
        ))
    };
    let rows: Vec<ListRow> = match cursor {
        None => {
            // sql-const : seule la constante LIST_COLUMNS est interpolée.
            let mut stmt = c
                .prepare_cached(&format!(
                    "SELECT {LIST_COLUMNS} FROM messages WHERE mailbox_id = ?1 \
                     ORDER BY sort_date DESC, id DESC LIMIT ?2"
                ))
                .map_err(store_error)?;
            stmt.query_map(params![mailbox.get(), fetch], read)
                .map_err(store_error)?
                .collect::<Result<_, _>>()
                .map_err(store_error)?
        }
        Some(cursor) => {
            // sql-const : seule la constante LIST_COLUMNS est interpolée.
            let mut stmt = c
                .prepare_cached(&format!(
                    "SELECT {LIST_COLUMNS} FROM messages WHERE mailbox_id = ?1 \
                     AND (sort_date, id) < (?2, ?3) \
                     ORDER BY sort_date DESC, id DESC LIMIT ?4"
                ))
                .map_err(store_error)?;
            stmt.query_map(
                params![mailbox.get(), cursor.date, cursor.id.get(), fetch],
                read,
            )
            .map_err(store_error)?
            .collect::<Result<_, _>>()
            .map_err(store_error)?
        }
    };
    let more = rows.len() > limit as usize;
    let mut items = rows
        .into_iter()
        .take(limit as usize)
        .map(list_item)
        .collect::<Result<Vec<_>, _>>()?;
    items.shrink_to_fit();
    let next = if more {
        items.last().map(|i| PageCursor {
            date: i.date,
            id: i.id,
        })
    } else {
        None
    };
    Ok(MessagePage { items, next })
}

impl MessageListQuery for SqliteMailStore {
    async fn page(
        &self,
        mailbox: MailboxId,
        cursor: Option<PageCursor>,
        limit: u32,
    ) -> Result<MessagePage, StoreError> {
        self.db
            .read(move |c| page_query(c, mailbox, cursor, limit))
            .await
    }
}

#[cfg(test)]
mod tests;
