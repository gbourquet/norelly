//! Actions sur les messages : file durable `pending_ops` et effets locaux
//! (drapeau changé, message masqué), appliqués dans la même transaction
//! que la modification de la file (ADR-003).

use norelly_domain::account::AccountId;
use norelly_domain::actions::{
    ActionFlag, FlagChange, FolderRef, LocalEffect, LocalMessage, MAX_ACTION_TARGETS, Member,
    OpKind, PendingOp, QueueChange, local_effect, revert_effect,
};
use norelly_domain::labels::KeywordPermission;
use norelly_domain::mailbox::{Keyword, MailboxId, MailboxName, MailboxRole, ModSeq, UidValidity};
use norelly_domain::ports::{ConversationId, MessageId, PendingOpId, PendingOpStore, StoreError};
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};

use crate::mail::{SqliteMailStore, corrupted, flags_from, keywords_text, mailbox_id, uid_of};
use crate::store_error;

/// Masque d'un drapeau dans la colonne `messages.flags`.
/// Membre d'une conversation relu : identifiant, dossier, rôle,
/// drapeaux, mots-clés, date de tri, `Message-ID`.
type MemberRow = (i64, i64, String, i64, String, i64, Option<String>);

const fn flag_bit(flag: &ActionFlag) -> i64 {
    match flag {
        ActionFlag::Seen => 1,
        ActionFlag::Flagged => 4,
        ActionFlag::Keyword(_) => 0,
    }
}

const fn flag_name(flag: &ActionFlag) -> &'static str {
    match flag {
        ActionFlag::Seen => "seen",
        ActionFlag::Flagged => "flagged",
        ActionFlag::Keyword(_) => "keyword",
    }
}

fn parse_flag(raw: &str, keyword: Option<&str>) -> Result<ActionFlag, StoreError> {
    match raw {
        "seen" => Ok(ActionFlag::Seen),
        "flagged" => Ok(ActionFlag::Flagged),
        "keyword" => keyword
            .and_then(Keyword::parse)
            .map(ActionFlag::Keyword)
            .ok_or_else(|| corrupted("mot-clé d'opération")),
        _ => Err(corrupted("drapeau d'opération")),
    }
}

/// Applique un effet local ; sans effet si le message a disparu.
fn apply_effect(c: &Connection, effect: LocalEffect) -> Result<(), StoreError> {
    match effect {
        LocalEffect::SetFlag {
            message,
            flag: flag @ ActionFlag::Keyword(_),
            value,
        } => {
            let row: Option<(i64, String)> = c
                .query_row(
                    "SELECT flags, keywords FROM messages WHERE id = ?1",
                    [message.get()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(store_error)?;
            if let Some((bits, keywords)) = row {
                let mut flags = flags_from(bits, &keywords);
                flag.set(&mut flags, value);
                c.prepare_cached("UPDATE messages SET keywords = ?2 WHERE id = ?1")
                    .and_then(|mut stmt| {
                        stmt.execute(params![message.get(), keywords_text(&flags)])
                    })
                    .map_err(store_error)?;
            }
        }
        LocalEffect::SetFlag {
            message,
            flag,
            value,
        } => {
            let bit = flag_bit(&flag);
            c.prepare_cached("UPDATE messages SET flags = (flags & ~?2) | ?3 WHERE id = ?1")
                .and_then(|mut stmt| {
                    stmt.execute(params![message.get(), bit, if value { bit } else { 0 }])
                })
                .map_err(store_error)?;
        }
        LocalEffect::Hide { message } | LocalEffect::Show { message } => {
            let hidden = matches!(effect, LocalEffect::Hide { .. });
            c.prepare_cached("UPDATE messages SET hidden = ?2 WHERE id = ?1")
                .and_then(|mut stmt| stmt.execute(params![message.get(), hidden]))
                .map_err(store_error)?;
        }
    }
    Ok(())
}

/// Retire l'opération `id` et inverse son effet local, seulement si le
/// rejeu ne l'a pas réservée et, si `account` est donné, si elle est de ce
/// compte. `false` si elle est réservée, absente ou d'un autre compte.
fn cancel_unclaimed(
    c: &Connection,
    account: Option<AccountId>,
    id: PendingOpId,
) -> Result<bool, StoreError> {
    let Some(op) = read_op(c, id)? else {
        return Ok(false);
    };
    if account.is_some_and(|a| a != op.account) {
        return Ok(false);
    }
    let n = c
        .execute(
            "DELETE FROM pending_ops WHERE id = ?1 AND claimed = 0",
            [id.get()],
        )
        .map_err(store_error)?;
    if n == 0 {
        return Ok(false);
    }
    apply_effect(c, revert_effect(&op))?;
    remove_placeholders(c, id.get())?;
    Ok(true)
}

/// UID synthétique d'une copie provisoire : tout en haut de l'espace des
/// UID (les serveurs attribuent les UID en croissant depuis 1), distinct
/// pour chaque opération. La synchronisation ignore ces lignes
/// (`pending_op IS NOT NULL`).
fn placeholder_uid(op: i64) -> i64 {
    4_294_967_295 - op.rem_euclid(100_000_000)
}

/// Copie provisoire du message déplacé dans le dossier de destination
/// (l'utilisateur voit l'état final sans attendre le serveur). Sans effet
/// si la destination est inconnue localement.
fn create_placeholder(
    c: &Connection,
    op_id: i64,
    op: &PendingOp,
    to: &MailboxName,
) -> Result<(), StoreError> {
    let target: Option<i64> = c
        .query_row(
            "SELECT id FROM mailboxes WHERE account_id = ?1 AND name = ?2",
            params![op.account.get(), to.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    let Some(target) = target else {
        return Ok(());
    };
    let uid = placeholder_uid(op_id);
    c.execute(
        "DELETE FROM messages WHERE mailbox_id = ?1 AND uid = ?2",
        params![target, uid],
    )
    .map_err(store_error)?;
    let copied = c
        .execute(
            "INSERT INTO messages (mailbox_id, uid, flags, keywords, internal_date, sort_date, \
             size, subject, from_name, from_address, to_json, cc_json, message_id, in_reply_to, \
             refs, header_date, has_attachments, structure, body_state, body_blob, \
             conversation_id, pending_op, sender_auth) \
             SELECT ?1, ?2, flags & ~8, keywords, internal_date, sort_date, size, subject, \
             from_name, from_address, to_json, cc_json, message_id, in_reply_to, refs, \
             header_date, has_attachments, structure, body_state, body_blob, conversation_id, ?3, \
             sender_auth \
             FROM messages WHERE id = ?4",
            params![target, uid, op_id, op.message.get()],
        )
        .map_err(store_error)?;
    if copied > 0 {
        let copy = c.last_insert_rowid();
        c.execute(
            "INSERT INTO message_parts (message_id, section, blob) \
             SELECT ?1, section, blob FROM message_parts WHERE message_id = ?2",
            params![copy, op.message.get()],
        )
        .map_err(store_error)?;
    }
    Ok(())
}

/// Retire les copies provisoires d'une opération.
fn remove_placeholders(c: &Connection, op_id: i64) -> Result<(), StoreError> {
    c.execute("DELETE FROM messages WHERE pending_op = ?1", [op_id])
        .map_err(store_error)?;
    Ok(())
}

fn insert_op(c: &Connection, op: &PendingOp) -> Result<i64, StoreError> {
    let (kind, flag, value, previous, since, target) = match &op.kind {
        OpKind::Flag {
            change,
            previous,
            unchanged_since,
        } => (
            "flag",
            Some(flag_name(&change.flag)),
            Some(change.value),
            Some(*previous),
            unchanged_since.map(|m| i64::try_from(m.get()).unwrap_or(i64::MAX)),
            None,
        ),
        OpKind::Move { to } => ("move", None, None, None, None, Some(to.as_str())),
        OpKind::Expunge => ("expunge", None, None, None, None, None),
    };
    let keyword = match &op.kind {
        OpKind::Flag {
            change:
                FlagChange {
                    flag: ActionFlag::Keyword(k),
                    ..
                },
            ..
        } => Some(k.as_str()),
        _ => None,
    };
    c.execute(
        "INSERT INTO pending_ops (account_id, message_id, mailbox, uid_validity, uid, kind, \
         flag, value, previous, unchanged_since, target, not_before, keyword) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            op.account.get(),
            op.message.get(),
            op.mailbox.as_str(),
            op.uid_validity.get(),
            op.uid.get(),
            kind,
            flag,
            value,
            previous,
            since,
            target,
            op.not_before,
            keyword,
        ],
    )
    .map_err(store_error)?;
    Ok(c.last_insert_rowid())
}

const OP_COLUMNS: &str = "id, account_id, message_id, mailbox, uid_validity, uid, kind, flag, \
                          value, previous, unchanged_since, target, not_before, keyword";

type OpRow = (
    i64,
    i64,
    i64,
    String,
    i64,
    i64,
    String,
    Option<String>,
    Option<bool>,
    Option<bool>,
    Option<i64>,
    Option<String>,
    i64,
    Option<String>,
);

fn read_op_row(r: &Row<'_>) -> rusqlite::Result<OpRow> {
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
}

/// Revalide une ligne par les constructeurs du domaine.
fn op_of(row: OpRow) -> Result<(PendingOpId, PendingOp), StoreError> {
    let (
        id,
        account,
        message,
        mailbox,
        validity,
        uid,
        kind,
        flag,
        value,
        previous,
        since,
        target,
        not_before,
        keyword,
    ) = row;
    let kind = match kind.as_str() {
        "flag" => OpKind::Flag {
            change: FlagChange {
                flag: parse_flag(flag.as_deref().unwrap_or_default(), keyword.as_deref())?,
                value: value.ok_or_else(|| corrupted("valeur d'opération"))?,
            },
            previous: previous.ok_or_else(|| corrupted("valeur précédente"))?,
            unchanged_since: since
                .and_then(|m| u64::try_from(m).ok())
                .and_then(ModSeq::new),
        },
        "move" => OpKind::Move {
            to: MailboxName::parse(target.as_deref().unwrap_or_default())
                .map_err(|_| corrupted("boîte de destination"))?,
        },
        "expunge" => OpKind::Expunge,
        _ => return Err(corrupted("type d'opération")),
    };
    Ok((
        PendingOpId::new(id).ok_or_else(|| corrupted("identifiant d'opération"))?,
        PendingOp {
            account: AccountId::new(account).map_err(|_| corrupted("compte"))?,
            message: MessageId::new(message).ok_or_else(|| corrupted("message"))?,
            mailbox: MailboxName::parse(&mailbox).map_err(|_| corrupted("boîte"))?,
            uid_validity: u32::try_from(validity)
                .ok()
                .and_then(UidValidity::new)
                .ok_or_else(|| corrupted("UIDVALIDITY"))?,
            uid: uid_of(uid)?,
            kind,
            not_before,
        },
    ))
}

fn read_op(c: &Connection, id: PendingOpId) -> Result<Option<PendingOp>, StoreError> {
    // sql-const : seule la constante OP_COLUMNS est interpolée.
    let row = c
        .query_row(
            &format!("SELECT {OP_COLUMNS} FROM pending_ops WHERE id = ?1"),
            [id.get()],
            read_op_row,
        )
        .optional()
        .map_err(store_error)?;
    row.map(op_of).transpose().map(|o| o.map(|(_, op)| op))
}

fn pending_ops(
    c: &Connection,
    account: AccountId,
) -> Result<Vec<(PendingOpId, PendingOp)>, StoreError> {
    // sql-const : seule la constante OP_COLUMNS est interpolée.
    let mut stmt = c
        .prepare_cached(&format!(
            "SELECT {OP_COLUMNS} FROM pending_ops WHERE account_id = ?1 ORDER BY id"
        ))
        .map_err(store_error)?;
    let rows: Vec<OpRow> = stmt
        .query_map([account.get()], read_op_row)
        .map_err(store_error)?
        .collect::<Result<_, _>>()
        .map_err(store_error)?;
    rows.into_iter().map(op_of).collect()
}

/// Opérations en attente du compte, revalidées une à une : une ligne
/// illisible est ignorée (elle ne bloque pas un renommage local).
fn readable_ops(
    c: &Connection,
    account: AccountId,
) -> Result<Vec<(PendingOpId, PendingOp)>, StoreError> {
    // sql-const : seule la constante OP_COLUMNS est interpolée.
    let mut stmt = c
        .prepare_cached(&format!(
            "SELECT {OP_COLUMNS} FROM pending_ops WHERE account_id = ?1 ORDER BY id"
        ))
        .map_err(store_error)?;
    let rows: Vec<OpRow> = stmt
        .query_map([account.get()], read_op_row)
        .map_err(store_error)?
        .collect::<Result<_, _>>()
        .map_err(store_error)?;
    Ok(rows.into_iter().filter_map(|row| op_of(row).ok()).collect())
}

/// Opérations en attente du compte qui visent le dossier `from` renommé
/// en `to` (ou un de ses sous-dossiers), en source ou en destination :
/// réécrites d'après le domaine ([`PendingOp::follow_folder`]), dans la
/// transaction de l'appelant.
pub(crate) fn follow_renamed(
    c: &Connection,
    account: AccountId,
    from: &MailboxName,
    to: &MailboxName,
    delimiter: Option<char>,
) -> Result<(), StoreError> {
    for (id, op) in readable_ops(c, account)? {
        let Some(moved) = op.follow_folder(from, to, delimiter) else {
            continue;
        };
        // Même genre d'opération : `target` reste non nul pour un
        // déplacement seulement.
        let target = match &moved.kind {
            OpKind::Move { to } => Some(to.as_str()),
            _ => None,
        };
        c.prepare_cached(
            "UPDATE pending_ops SET mailbox = ?3, target = ?4 \
             WHERE id = ?1 AND account_id = ?2",
        )
        .and_then(|mut stmt| {
            stmt.execute(params![
                id.get(),
                account.get(),
                moved.mailbox.as_str(),
                target
            ])
        })
        .map_err(store_error)?;
    }
    Ok(())
}

/// Abandonne, comme `revert`, les opérations en attente du compte dont la
/// source ou la destination est une des boîtes `names`, dans la
/// transaction de l'appelant.
pub(crate) fn revert_ops_on(
    c: &Connection,
    account: AccountId,
    names: &[&MailboxName],
) -> Result<(), StoreError> {
    for (id, op) in readable_ops(c, account)? {
        let target = match &op.kind {
            OpKind::Move { to } => Some(to),
            _ => None,
        };
        if !names
            .iter()
            .any(|n| **n == op.mailbox || target == Some(*n))
        {
            continue;
        }
        c.execute(
            "DELETE FROM pending_ops WHERE id = ?1 AND account_id = ?2",
            [id.get(), account.get()],
        )
        .map_err(store_error)?;
        apply_effect(c, revert_effect(&op))?;
        remove_placeholders(c, id.get())?;
    }
    Ok(())
}

/// Opérations en attente du compte sur des messages de `mailbox`.
fn pending_ops_in(
    c: &Connection,
    account: AccountId,
    mailbox: MailboxId,
) -> Result<Vec<(PendingOpId, PendingOp)>, StoreError> {
    // sql-const : seule la constante OP_COLUMNS est interpolée.
    let mut stmt = c
        .prepare_cached(&format!(
            "SELECT {OP_COLUMNS} FROM pending_ops WHERE account_id = ?1 \
             AND message_id IN (SELECT id FROM messages WHERE mailbox_id = ?2) ORDER BY id"
        ))
        .map_err(store_error)?;
    let rows: Vec<OpRow> = stmt
        .query_map([account.get(), mailbox.get()], read_op_row)
        .map_err(store_error)?
        .collect::<Result<_, _>>()
        .map_err(store_error)?;
    rows.into_iter().map(op_of).collect()
}

fn folder_of(
    id: i64,
    name: &str,
    role: &str,
    selectable: bool,
    permanent_flags: Option<&str>,
) -> Result<FolderRef, StoreError> {
    Ok(FolderRef {
        id: mailbox_id(id)?,
        name: MailboxName::parse(name).map_err(|_| corrupted("nom de boîte"))?,
        role: MailboxRole::parse(role).ok_or_else(|| corrupted("rôle"))?,
        selectable,
        keywords: KeywordPermission::from_storage(permanent_flags),
    })
}

impl PendingOpStore for SqliteMailStore {
    async fn action_target(&self, message: MessageId) -> Result<Option<LocalMessage>, StoreError> {
        self.db()
            .run(move |c| {
                let row = c
                    .query_row(
                        "SELECT b.account_id, b.id, b.name, b.role, b.selectable, m.uid, m.flags, \
                         m.keywords, b.uid_validity, b.highest_modseq, b.permanent_flags \
                         FROM messages m JOIN mailboxes b ON b.id = m.mailbox_id \
                         WHERE m.id = ?1 AND m.hidden = 0 AND m.pending_op IS NULL",
                        [message.get()],
                        |r| {
                            Ok((
                                r.get::<_, i64>(0)?,
                                r.get::<_, i64>(1)?,
                                r.get::<_, String>(2)?,
                                r.get::<_, String>(3)?,
                                r.get::<_, bool>(4)?,
                                r.get::<_, i64>(5)?,
                                r.get::<_, i64>(6)?,
                                r.get::<_, String>(7)?,
                                r.get::<_, Option<i64>>(8)?,
                                r.get::<_, Option<i64>>(9)?,
                                r.get::<_, Option<String>>(10)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(store_error)?;
                let Some((
                    account,
                    mbox,
                    name,
                    role,
                    selectable,
                    uid,
                    bits,
                    keywords,
                    validity,
                    modseq,
                    permanent_flags,
                )) = row
                else {
                    return Ok(None);
                };
                Ok(Some(LocalMessage {
                    id: message,
                    account: AccountId::new(account).map_err(|_| corrupted("compte"))?,
                    mailbox: folder_of(mbox, &name, &role, selectable, permanent_flags.as_deref())?,
                    uid: uid_of(uid)?,
                    flags: flags_from(bits, &keywords),
                    uid_validity: validity
                        .and_then(|v| u32::try_from(v).ok())
                        .and_then(UidValidity::new),
                    highest_modseq: modseq
                        .and_then(|m| u64::try_from(m).ok())
                        .and_then(ModSeq::new),
                }))
            })
            .await
    }

    async fn conversation_members(
        &self,
        conversation: ConversationId,
    ) -> Result<Vec<Member>, StoreError> {
        self.db()
            .read(move |c| {
                let mut stmt = c
                    .prepare_cached(
                        "SELECT m.id, m.mailbox_id, b.role, m.flags, m.keywords, m.sort_date, \
                         m.message_id \
                         FROM messages m JOIN mailboxes b ON b.id = m.mailbox_id \
                         WHERE m.conversation_id = ?1 AND m.hidden = 0 AND (m.flags & 8) = 0 \
                         AND m.pending_op IS NULL \
                         ORDER BY m.sort_date, m.id LIMIT ?2",
                    )
                    .map_err(store_error)?;
                let limit = i64::try_from(MAX_ACTION_TARGETS).unwrap_or(i64::MAX);
                let rows: Vec<MemberRow> = stmt
                    .query_map(params![conversation.get(), limit], |r| {
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
                    .map_err(store_error)?;
                rows.into_iter()
                    .map(|(id, mailbox, role, bits, keywords, date, message_id)| {
                        Ok(Member {
                            id: MessageId::new(id).ok_or_else(|| corrupted("message"))?,
                            mailbox: mailbox_id(mailbox)?,
                            role: MailboxRole::parse(&role).ok_or_else(|| corrupted("rôle"))?,
                            flags: flags_from(bits, &keywords),
                            date,
                            message_id,
                        })
                    })
                    .collect()
            })
            .await
    }

    async fn action_folders(&self, account: AccountId) -> Result<Vec<FolderRef>, StoreError> {
        self.db()
            .run(move |c| {
                let mut stmt = c
                    .prepare_cached(
                        "SELECT id, name, role, selectable, permanent_flags FROM mailboxes \
                         WHERE account_id = ?1 ORDER BY position, id",
                    )
                    .map_err(store_error)?;
                let rows: Vec<(i64, String, String, bool, Option<String>)> = stmt
                    .query_map([account.get()], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                    })
                    .map_err(store_error)?
                    .collect::<Result<_, _>>()
                    .map_err(store_error)?;
                rows.iter()
                    .map(|(id, name, role, selectable, permanent)| {
                        folder_of(*id, name, role, *selectable, permanent.as_deref())
                    })
                    .collect()
            })
            .await
    }

    async fn record_all(&self, changes: &[QueueChange]) -> Result<Vec<PendingOpId>, StoreError> {
        let changes = changes.to_vec();
        self.db()
            .run(move |c| {
                let tx = c
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(store_error)?;
                let mut recorded = Vec::new();
                for change in &changes {
                    let op = match change {
                        QueueChange::Push(op) => op,
                        QueueChange::CancelOrPush { existing, op } => {
                            if cancel_unclaimed(&tx, Some(op.account), *existing)? {
                                continue;
                            }
                            op
                        }
                    };
                    let id = insert_op(&tx, op)?;
                    apply_effect(&tx, local_effect(op))?;
                    if let OpKind::Move { to } = &op.kind {
                        create_placeholder(&tx, id, op, to)?;
                    }
                    recorded.push(
                        PendingOpId::new(id).ok_or_else(|| corrupted("identifiant d'opération"))?,
                    );
                }
                tx.commit().map_err(store_error)?;
                Ok(recorded)
            })
            .await
    }

    async fn pending(
        &self,
        account: AccountId,
    ) -> Result<Vec<(PendingOpId, PendingOp)>, StoreError> {
        self.db().run(move |c| pending_ops(c, account)).await
    }

    async fn complete(&self, account: AccountId, id: PendingOpId) -> Result<bool, StoreError> {
        self.db()
            .run(move |c| {
                let n = c
                    .execute(
                        "DELETE FROM pending_ops WHERE id = ?1 AND account_id = ?2",
                        [id.get(), account.get()],
                    )
                    .map_err(store_error)?;
                Ok(n > 0)
            })
            .await
    }

    async fn revert(&self, account: AccountId, id: PendingOpId) -> Result<bool, StoreError> {
        self.db()
            .run(move |c| {
                let tx = c
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(store_error)?;
                let Some(op) = read_op(&tx, id)?.filter(|op| op.account == account) else {
                    return Ok(false);
                };
                tx.execute(
                    "DELETE FROM pending_ops WHERE id = ?1 AND account_id = ?2",
                    [id.get(), account.get()],
                )
                .map_err(store_error)?;
                apply_effect(&tx, revert_effect(&op))?;
                remove_placeholders(&tx, id.get())?;
                tx.commit().map_err(store_error)?;
                Ok(true)
            })
            .await
    }

    async fn cancel(&self, id: PendingOpId) -> Result<bool, StoreError> {
        self.db()
            .run(move |c| {
                let tx = c
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(store_error)?;
                let cancelled = cancel_unclaimed(&tx, None, id)?;
                tx.commit().map_err(store_error)?;
                Ok(cancelled)
            })
            .await
    }

    async fn claim(&self, account: AccountId, id: PendingOpId) -> Result<Option<u32>, StoreError> {
        self.db()
            .run(move |c| {
                let attempts: Option<i64> = c
                    .query_row(
                        "UPDATE pending_ops SET claimed = 1, attempts = attempts + 1 \
                         WHERE id = ?1 AND account_id = ?2 RETURNING attempts",
                        [id.get(), account.get()],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(store_error)?;
                Ok(attempts.map(|a| u32::try_from(a).unwrap_or(u32::MAX)))
            })
            .await
    }

    async fn purge_placeholders(
        &self,
        account: AccountId,
        mailbox: Option<MailboxId>,
    ) -> Result<usize, StoreError> {
        self.db()
            .run(move |c| {
                let n = c
                    .execute(
                        "DELETE FROM messages WHERE pending_op IS NOT NULL \
                         AND pending_op NOT IN (SELECT id FROM pending_ops) \
                         AND mailbox_id IN (SELECT id FROM mailboxes WHERE account_id = ?1) \
                         AND (?2 IS NULL OR mailbox_id = ?2)",
                        params![account.get(), mailbox.map(MailboxId::get)],
                    )
                    .map_err(store_error)?;
                Ok(n)
            })
            .await
    }

    async fn reapply(&self, account: AccountId) -> Result<(), StoreError> {
        self.db()
            .run(move |c| {
                let tx = c
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(store_error)?;
                for (_, op) in pending_ops(&tx, account)? {
                    apply_effect(&tx, local_effect(&op))?;
                }
                tx.commit().map_err(store_error)
            })
            .await
    }

    async fn reapply_mailbox(
        &self,
        account: AccountId,
        mailbox: MailboxId,
    ) -> Result<(), StoreError> {
        self.db()
            .run(move |c| {
                let tx = c
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(store_error)?;
                for (_, op) in pending_ops_in(&tx, account, mailbox)? {
                    apply_effect(&tx, local_effect(&op))?;
                }
                tx.commit().map_err(store_error)
            })
            .await
    }

    async fn archive_fallback(&self, account: AccountId) -> Result<Option<MailboxId>, StoreError> {
        self.db()
            .read(move |c| {
                let raw: Option<Option<i64>> = c
                    .query_row(
                        "SELECT archive_mailbox FROM action_settings WHERE account_id = ?1",
                        [account.get()],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(store_error)?;
                raw.flatten().map(mailbox_id).transpose()
            })
            .await
    }

    async fn set_archive_fallback(
        &self,
        account: AccountId,
        mailbox: Option<MailboxId>,
    ) -> Result<(), StoreError> {
        self.db()
            .run(move |c| {
                // Le dossier doit appartenir au compte.
                if let Some(m) = mailbox {
                    let owned: bool = c
                        .query_row(
                            "SELECT EXISTS (SELECT 1 FROM mailboxes WHERE id = ?1 AND account_id = ?2)",
                            params![m.get(), account.get()],
                            |r| r.get(0),
                        )
                        .map_err(store_error)?;
                    if !owned {
                        return Err(StoreError("dossier d'un autre compte".to_owned()));
                    }
                }
                c.execute(
                    "INSERT INTO action_settings (account_id, archive_mailbox) VALUES (?1, ?2) \
                     ON CONFLICT (account_id) DO UPDATE SET archive_mailbox = excluded.archive_mailbox",
                    params![account.get(), mailbox.map(MailboxId::get)],
                )
                .map_err(store_error)?;
                Ok(())
            })
            .await
    }
}
