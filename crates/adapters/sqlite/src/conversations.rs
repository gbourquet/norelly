//! Conversations précalculées (ADR-003) : rattachement incrémental des
//! messages à l'insertion (règles du domaine,
//! [`norelly_domain::conversation::assign`]), et port de requête
//! [`ConversationQuery`] paginé par curseur.
//!
//! Les agrégats par dossier (`conversation_mailboxes`) sont tenus à jour
//! par les déclencheurs de la migration 3.

use std::collections::BTreeSet;

use norelly_domain::account::AccountId;
use norelly_domain::conversation::{
    Assignment, ConversationScope, SubjectCandidate, ThreadKeys, assign,
};
use norelly_domain::mailbox::{Correspondent, MAX_SUBJECT_CHARS, MailboxName, header_text};
use norelly_domain::ports::{
    ConversationCursor, ConversationId, ConversationListItem, ConversationMessage,
    ConversationPage, ConversationQuery, MAX_CONVERSATION_MESSAGES, MAX_LISTED_SENDERS,
    MAX_PAGE_SIZE, MessageId, MessageLocation, MessageLocator, StoreError, StoredBody, same_sender,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::mail::{SqliteMailStore, corrupted, flags_from, mailbox_id, uid_of};
use crate::store_error;

/// Rattache un message (déjà analysé) à sa conversation, en créant ou en
/// fusionnant des conversations au besoin ; renvoie l'identifiant à
/// enregistrer dans `messages.conversation_id`.
pub(crate) fn assign_conversation(
    tx: &Transaction<'_>,
    account: AccountId,
    keys: &ThreadKeys,
    date: i64,
) -> Result<i64, StoreError> {
    let mut linked = BTreeSet::new();
    {
        let mut find = tx
            .prepare_cached(
                "SELECT conversation_id FROM conversation_links \
                 WHERE account_id = ?1 AND message_id = ?2",
            )
            .map_err(store_error)?;
        for id in keys.all_ids() {
            let found: Option<i64> = find
                .query_row(params![account.get(), id], |r| r.get(0))
                .optional()
                .map_err(store_error)?;
            linked.extend(found);
        }
    }
    let by_subject = if linked.is_empty() && keys.is_reply() && !keys.subject.is_empty() {
        tx.prepare_cached(
            "SELECT id, latest_date FROM conversations \
             WHERE account_id = ?1 AND subject_key = ?2 \
             ORDER BY latest_date DESC, id DESC LIMIT 1",
        )
        .map_err(store_error)?
        .query_row(params![account.get(), keys.subject.as_str()], |r| {
            Ok(SubjectCandidate {
                conversation: r.get::<_, i64>(0)?,
                latest_date: r.get(1)?,
            })
        })
        .optional()
        .map_err(store_error)?
    } else {
        None
    };
    let target = match assign(keys, date, &linked, by_subject) {
        Assignment::New => {
            tx.prepare_cached(
                "INSERT INTO conversations (account_id, subject_key, latest_date) \
                 VALUES (?1, ?2, ?3)",
            )
            .map_err(store_error)?
            .execute(params![account.get(), keys.subject.as_str(), date])
            .map_err(store_error)?;
            tx.last_insert_rowid()
        }
        Assignment::Join(c) => c,
        Assignment::Merge { into, from } => {
            for old in from {
                merge(tx, into, old)?;
            }
            into
        }
    };
    {
        let mut link = tx
            .prepare_cached(
                "INSERT INTO conversation_links (account_id, message_id, conversation_id) \
                 VALUES (?1, ?2, ?3) \
                 ON CONFLICT (account_id, message_id) DO UPDATE \
                 SET conversation_id = excluded.conversation_id",
            )
            .map_err(store_error)?;
        for id in keys.all_ids() {
            link.execute(params![account.get(), id, target])
                .map_err(store_error)?;
        }
    }
    tx.prepare_cached("UPDATE conversations SET latest_date = max(latest_date, ?2) WHERE id = ?1")
        .map_err(store_error)?
        .execute(params![target, date])
        .map_err(store_error)?;
    Ok(target)
}

/// Fusionne la conversation `old` dans `into`.
fn merge(tx: &Transaction<'_>, into: i64, old: i64) -> Result<(), StoreError> {
    tx.prepare_cached("UPDATE messages SET conversation_id = ?1 WHERE conversation_id = ?2")
        .map_err(store_error)?
        .execute(params![into, old])
        .map_err(store_error)?;
    tx.prepare_cached(
        "UPDATE conversation_links SET conversation_id = ?1 WHERE conversation_id = ?2",
    )
    .map_err(store_error)?
    .execute(params![into, old])
    .map_err(store_error)?;
    tx.prepare_cached(
        "UPDATE conversations SET latest_date = \
         max(latest_date, (SELECT latest_date FROM conversations WHERE id = ?2)) WHERE id = ?1",
    )
    .map_err(store_error)?
    .execute(params![into, old])
    .map_err(store_error)?;
    tx.prepare_cached("DELETE FROM conversations WHERE id = ?1")
        .map_err(store_error)?
        .execute([old])
        .map_err(store_error)?;
    Ok(())
}

/// Nombre de messages rattachés par transaction lors du rattrapage.
const BACKFILL_BATCH: i64 = 2000;

/// Message à rattacher : identifiant, compte, `Message-ID`, références
/// (JSON), `In-Reply-To` (JSON), sujet, date.
type BackfillRow = (i64, i64, Option<String>, String, String, String, i64);

/// Rattache les messages qui n'ont pas encore de conversation (base
/// antérieure à la version 3), du plus ancien au plus récent, par lots.
pub(crate) fn backfill(conn: &mut Connection) -> Result<usize, StoreError> {
    let mut total = 0;
    loop {
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(store_error)?;
        let rows: Vec<BackfillRow> = {
            let mut stmt = tx
                .prepare_cached(
                    "SELECT m.id, b.account_id, m.message_id, m.refs, m.in_reply_to, \
                     m.subject, m.sort_date \
                     FROM messages m JOIN mailboxes b ON b.id = m.mailbox_id \
                     WHERE m.conversation_id IS NULL ORDER BY m.sort_date, m.id LIMIT ?1",
                )
                .map_err(store_error)?;
            stmt.query_map([BACKFILL_BATCH], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            })
            .map_err(store_error)?
            .collect::<Result<_, _>>()
            .map_err(store_error)?
        };
        if rows.is_empty() {
            tx.commit().map_err(store_error)?;
            return Ok(total);
        }
        for (id, account, own, refs, irt, subject, date) in rows {
            let account = AccountId::new(account).map_err(|_| corrupted("compte"))?;
            // Listes illisibles : traitées comme vides.
            let refs: Vec<String> = serde_json::from_str(&refs).unwrap_or_default();
            let irt: Vec<String> = serde_json::from_str(&irt).unwrap_or_default();
            let keys = ThreadKeys::from_parts(own.as_deref(), &refs, &irt, &subject);
            let conversation = assign_conversation(&tx, account, &keys, date)?;
            tx.prepare_cached("UPDATE messages SET conversation_id = ?2 WHERE id = ?1")
                .map_err(store_error)?
                .execute(params![id, conversation])
                .map_err(store_error)?;
            total += 1;
        }
        tx.commit().map_err(store_error)?;
    }
}

// ---------------------------------------------------------------------------
// Requêtes
// ---------------------------------------------------------------------------

/// Colonnes d'une ligne de liste (agrégat `c` de `conversation_mailboxes`).
const ROW_COLUMNS: &str = "c.conversation_id, b.account_id, c.mailbox_id, c.latest_date, \
                           c.messages, c.unread, c.flagged, c.attachments";

type Row = (i64, i64, i64, i64, i64, i64, bool, bool);

fn read_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
    ))
}

fn query_rows(
    c: &Connection,
    scope: ConversationScope,
    cursor: Option<ConversationCursor>,
    fetch: i64,
) -> Result<Vec<Row>, StoreError> {
    // Curseur absent : au-delà de toute date.
    let (date, id) = cursor.map_or((i64::MAX, i64::MAX), |c| (c.date, c.id.get()));
    let rows = match scope {
        ConversationScope::Mailbox(mailbox) => {
            // sql-const : seule la constante ROW_COLUMNS est interpolée.
            let mut stmt = c
                .prepare_cached(&format!(
                    "SELECT {ROW_COLUMNS} FROM conversation_mailboxes c \
                     JOIN mailboxes b ON b.id = c.mailbox_id \
                     WHERE c.mailbox_id = ?1 AND (c.latest_date, c.conversation_id) < (?2, ?3) \
                     ORDER BY c.latest_date DESC, c.conversation_id DESC LIMIT ?4"
                ))
                .map_err(store_error)?;
            stmt.query_map(params![mailbox.get(), date, id, fetch], read_row)
                .map_err(store_error)?
                .collect::<Result<_, _>>()
        }
        ConversationScope::Unified(role) => {
            // sql-const : seule la constante ROW_COLUMNS est interpolée.
            let mut stmt = c
                .prepare_cached(&format!(
                    "SELECT {ROW_COLUMNS} FROM conversation_mailboxes c \
                     JOIN mailboxes b ON b.id = c.mailbox_id \
                     WHERE c.role = ?1 AND (c.latest_date, c.conversation_id) < (?2, ?3) \
                     ORDER BY c.latest_date DESC, c.conversation_id DESC LIMIT ?4"
                ))
                .map_err(store_error)?;
            stmt.query_map(params![role.as_str(), date, id, fetch], read_row)
                .map_err(store_error)?
                .collect::<Result<_, _>>()
        }
    };
    rows.map_err(store_error)
}

/// Sujet du message le plus récent et expéditeurs distincts d'une
/// conversation dans un dossier.
fn summary(
    c: &Connection,
    conversation: i64,
    mailbox: i64,
) -> Result<(String, Vec<Correspondent>), StoreError> {
    let mut stmt = c
        .prepare_cached(
            "SELECT subject, from_name, from_address FROM messages \
             WHERE conversation_id = ?1 AND mailbox_id = ?2 \
             ORDER BY sort_date DESC, id DESC LIMIT 50",
        )
        .map_err(store_error)?;
    let rows = stmt
        .query_map(params![conversation, mailbox], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(store_error)?;
    let mut subject = None;
    let mut senders: Vec<Correspondent> = Vec::new();
    for row in rows {
        let (s, name, address) = row.map_err(store_error)?;
        subject.get_or_insert_with(|| header_text(&s, MAX_SUBJECT_CHARS));
        if senders.len() < MAX_LISTED_SENDERS && (!name.is_empty() || !address.is_empty()) {
            let person = Correspondent::new(&name, &address);
            if !senders.iter().any(|p| same_sender(p, &person)) {
                senders.push(person);
            }
        }
    }
    Ok((subject.unwrap_or_default(), senders))
}

pub(crate) fn conversations_query(
    c: &Connection,
    scope: ConversationScope,
    cursor: Option<ConversationCursor>,
    limit: u32,
) -> Result<ConversationPage, StoreError> {
    let limit = limit.min(MAX_PAGE_SIZE);
    if limit == 0 {
        return Ok(ConversationPage::default());
    }
    let rows = query_rows(c, scope, cursor, i64::from(limit) + 1)?;
    let more = rows.len() > limit as usize;
    let mut items = Vec::with_capacity(rows.len().min(limit as usize));
    for (id, account, mailbox, date, messages, unread, flagged, attachments) in
        rows.into_iter().take(limit as usize)
    {
        let (subject, senders) = summary(c, id, mailbox)?;
        items.push(ConversationListItem {
            id: ConversationId::new(id).ok_or_else(|| corrupted("conversation"))?,
            account: AccountId::new(account).map_err(|_| corrupted("compte"))?,
            mailbox: mailbox_id(mailbox)?,
            date,
            subject,
            senders,
            messages: u32::try_from(messages).unwrap_or(u32::MAX),
            unread: u32::try_from(unread).unwrap_or(u32::MAX),
            flagged,
            has_attachments: attachments,
        });
    }
    let next = if more {
        items.last().map(|i| ConversationCursor {
            date: i.date,
            id: i.id,
        })
    } else {
        None
    };
    Ok(ConversationPage { items, next })
}

fn people(raw: &str) -> Vec<Correspondent> {
    serde_json::from_str::<Vec<Correspondent>>(raw)
        .unwrap_or_default()
        .iter()
        .map(|p| Correspondent::new(&p.name, &p.address))
        .collect()
}

type MessageRow = (
    i64,
    i64,
    i64,
    i64,
    i64,
    String,
    String,
    String,
    String,
    String,
    i64,
    String,
    bool,
    Option<String>,
);

fn messages_query(
    c: &Connection,
    conversation: ConversationId,
) -> Result<Vec<ConversationMessage>, StoreError> {
    let mut stmt = c
        .prepare_cached(
            "SELECT m.id, b.account_id, m.mailbox_id, m.uid, m.sort_date, m.from_name, \
             m.from_address, m.to_json, m.cc_json, m.subject, m.flags, m.keywords, \
             m.has_attachments, m.message_id \
             FROM messages m JOIN mailboxes b ON b.id = m.mailbox_id \
             WHERE m.conversation_id = ?1 \
             ORDER BY m.sort_date DESC, m.id DESC LIMIT ?2",
        )
        .map_err(store_error)?;
    // Marge pour les doublons (même message dans plusieurs dossiers).
    let fetch = i64::try_from(MAX_CONVERSATION_MESSAGES * 4).unwrap_or(i64::MAX);
    let rows: Vec<MessageRow> = stmt
        .query_map(params![conversation.get(), fetch], |r| {
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
                r.get(10)?,
                r.get(11)?,
                r.get(12)?,
                r.get(13)?,
            ))
        })
        .map_err(store_error)?
        .collect::<Result<_, _>>()
        .map_err(store_error)?;
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for row in rows {
        let (
            id,
            account,
            mailbox,
            uid,
            date,
            from_name,
            from_address,
            to,
            cc,
            subject,
            bits,
            keywords,
            attachments,
            message_id,
        ) = row;
        if let Some(mid) = message_id
            && !seen.insert(mid)
        {
            continue;
        }
        out.push(ConversationMessage {
            id: MessageId::new(id).ok_or_else(|| corrupted("identifiant de message"))?,
            account: AccountId::new(account).map_err(|_| corrupted("compte"))?,
            mailbox: mailbox_id(mailbox)?,
            uid: uid_of(uid)?,
            date,
            from: (!from_name.is_empty() || !from_address.is_empty())
                .then(|| Correspondent::new(&from_name, &from_address)),
            to: people(&to),
            cc: people(&cc),
            subject: header_text(&subject, MAX_SUBJECT_CHARS),
            flags: flags_from(bits, &keywords),
            has_attachments: attachments,
        });
        if out.len() >= MAX_CONVERSATION_MESSAGES {
            break;
        }
    }
    out.reverse();
    Ok(out)
}

impl ConversationQuery for SqliteMailStore {
    async fn conversations(
        &self,
        scope: ConversationScope,
        cursor: Option<ConversationCursor>,
        limit: u32,
    ) -> Result<ConversationPage, StoreError> {
        self.db()
            .read(move |c| conversations_query(c, scope, cursor, limit))
            .await
    }

    async fn conversation_messages(
        &self,
        conversation: ConversationId,
    ) -> Result<Vec<ConversationMessage>, StoreError> {
        self.db()
            .read(move |c| messages_query(c, conversation))
            .await
    }
}

/// Compte, boîte, nom de boîte, UID, expéditeur, structure (JSON).
type LocateRow = (i64, i64, String, i64, String, String, Option<String>);

fn locate(c: &Connection, message: MessageId) -> Result<Option<MessageLocation>, StoreError> {
    let row: Option<LocateRow> = c
        .query_row(
            "SELECT b.account_id, m.mailbox_id, b.name, m.uid, m.from_name, m.from_address, \
             m.structure FROM messages m JOIN mailboxes b ON b.id = m.mailbox_id WHERE m.id = ?1",
            [message.get()],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .optional()
        .map_err(store_error)?;
    let Some((account, mailbox, name, uid, from_name, from_address, structure)) = row else {
        return Ok(None);
    };
    let body = crate::mail::read_body(c, message.get())?.unwrap_or(StoredBody::Missing);
    Ok(Some(MessageLocation {
        id: message,
        account: AccountId::new(account).map_err(|_| corrupted("compte"))?,
        mailbox: mailbox_id(mailbox)?,
        mailbox_name: MailboxName::parse(&name).map_err(|_| corrupted("nom de boîte"))?,
        uid: uid_of(uid)?,
        from: (!from_name.is_empty() || !from_address.is_empty())
            .then(|| Correspondent::new(&from_name, &from_address)),
        // Structure illisible : traitée comme inconnue.
        structure: structure.and_then(|s: String| serde_json::from_str(&s).ok()),
        body,
    }))
}

impl MessageLocator for SqliteMailStore {
    async fn locate(&self, message: MessageId) -> Result<Option<MessageLocation>, StoreError> {
        self.db().read(move |c| locate(c, message)).await
    }
}
