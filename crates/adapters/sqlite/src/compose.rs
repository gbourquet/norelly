//! Rédaction : brouillons ([`DraftStore`]) et file d'envoi
//! ([`OutboxStore`]), migration 0006.

use norelly_domain::account::AccountId;
use norelly_domain::address::EmailAddress;
use norelly_domain::compose::Threading;
use norelly_domain::compose::draft::{Draft, DraftAttachment, DraftSync, Recipient, RemoteCopy};
use norelly_domain::compose::outbox::{OutboxEntry, OutboxState, SendFailure};
use norelly_domain::mailbox::{Uid, UidValidity};
use norelly_domain::ports::{
    BlobId, DraftId, DraftStore, OutboxId, OutboxItem, OutboxStore, OutgoingMessage, StoreError,
    StoredDraft,
};
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::mail::corrupted;
use crate::{Database, store_error};

/// Brouillons et file d'envoi en base.
#[derive(Debug, Clone)]
pub struct SqliteComposeStore {
    db: Database,
}

impl SqliteComposeStore {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

// --- Contenu d'un brouillon (JSON, revalidé à la relecture) ---

#[derive(Serialize, Deserialize)]
struct RecipientDto {
    name: String,
    address: String,
}

#[derive(Serialize, Deserialize)]
struct AttachmentDto {
    blob: String,
    file_name: String,
    content_type: String,
    size: u64,
}

#[derive(Serialize, Deserialize)]
struct DraftDto {
    to: Vec<RecipientDto>,
    cc: Vec<RecipientDto>,
    bcc: Vec<RecipientDto>,
    subject: String,
    text: String,
    html: Option<String>,
    in_reply_to: Option<String>,
    references: Vec<String>,
    attachments: Vec<AttachmentDto>,
}

fn recipients_dto(list: &[Recipient]) -> Vec<RecipientDto> {
    list.iter()
        .map(|r| RecipientDto {
            name: r.name.clone(),
            address: r.address.to_string(),
        })
        .collect()
}

fn recipients_of(list: Vec<RecipientDto>) -> Result<Vec<Recipient>, StoreError> {
    list.into_iter()
        .map(|r| {
            let address = EmailAddress::parse(&r.address).map_err(|_| corrupted("adresse"))?;
            Ok(Recipient::new(&r.name, address))
        })
        .collect()
}

fn content_of(draft: &Draft) -> Result<String, StoreError> {
    let dto = DraftDto {
        to: recipients_dto(&draft.to),
        cc: recipients_dto(&draft.cc),
        bcc: recipients_dto(&draft.bcc),
        subject: draft.subject.clone(),
        text: draft.text.clone(),
        html: draft.html.clone(),
        in_reply_to: draft.threading.in_reply_to.clone(),
        references: draft.threading.references.clone(),
        attachments: draft
            .attachments
            .iter()
            .map(|a| AttachmentDto {
                blob: a.blob.as_str().to_owned(),
                file_name: a.file_name.clone(),
                content_type: a.content_type.clone(),
                size: a.size,
            })
            .collect(),
    };
    serde_json::to_string(&dto).map_err(|e| StoreError(format!("sérialisation : {e}")))
}

fn draft_of(account: AccountId, content: &str) -> Result<Draft, StoreError> {
    let dto: DraftDto = serde_json::from_str(content).map_err(|_| corrupted("brouillon"))?;
    Ok(Draft {
        account,
        to: recipients_of(dto.to)?,
        cc: recipients_of(dto.cc)?,
        bcc: recipients_of(dto.bcc)?,
        subject: dto.subject,
        text: dto.text,
        html: dto.html,
        threading: Threading {
            in_reply_to: dto.in_reply_to,
            references: dto.references,
        },
        attachments: dto
            .attachments
            .into_iter()
            .map(|a| {
                Ok(DraftAttachment {
                    blob: BlobId::parse(&a.blob).ok_or_else(|| corrupted("pièce jointe"))?,
                    file_name: a.file_name,
                    content_type: a.content_type,
                    size: a.size,
                })
            })
            .collect::<Result<_, StoreError>>()?,
    })
}

fn copy_of(validity: Option<i64>, uid: Option<i64>) -> Result<Option<RemoteCopy>, StoreError> {
    match (validity, uid) {
        (Some(v), Some(u)) => {
            let validity = u32::try_from(v)
                .ok()
                .and_then(UidValidity::new)
                .ok_or_else(|| corrupted("UIDVALIDITY"))?;
            let uid = u32::try_from(u)
                .ok()
                .and_then(Uid::new)
                .ok_or_else(|| corrupted("UID"))?;
            Ok(Some(RemoteCopy { validity, uid }))
        }
        (None, None) => Ok(None),
        _ => Err(corrupted("copie distante")),
    }
}

fn revision_of(raw: Option<i64>) -> Result<Option<u64>, StoreError> {
    raw.map(|r| u64::try_from(r).map_err(|_| corrupted("révision")))
        .transpose()
}

const DRAFT_COLUMNS: &str = "id, account_id, content, revision, remote_revision, \
     remote_validity, remote_uid, last_remote_save, updated_at";

type DraftRow = (
    i64,
    i64,
    String,
    i64,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    i64,
);

fn read_draft_row(r: &Row<'_>) -> rusqlite::Result<DraftRow> {
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
    ))
}

fn stored_of(row: DraftRow) -> Result<StoredDraft, StoreError> {
    let (id, account, content, revision, remote_revision, validity, uid, last, updated_at) = row;
    let account = AccountId::new(account).map_err(|_| corrupted("compte"))?;
    Ok(StoredDraft {
        id: DraftId::new(id).ok_or_else(|| corrupted("identifiant de brouillon"))?,
        draft: draft_of(account, &content)?,
        sync: DraftSync {
            revision: u64::try_from(revision).map_err(|_| corrupted("révision"))?,
            remote_revision: revision_of(remote_revision)?,
            remote: copy_of(validity, uid)?,
            last_remote_save: last,
        },
        updated_at,
    })
}

fn load_draft(c: &Connection, id: DraftId) -> Result<Option<StoredDraft>, StoreError> {
    let row = c
        .query_row(
            // sql-const : liste de colonnes constante.
            &format!("SELECT {DRAFT_COLUMNS} FROM drafts WHERE id = ?1"),
            [id.get()],
            read_draft_row,
        )
        .optional()
        .map_err(store_error)?;
    row.map(stored_of).transpose()
}

fn trash_insert(c: &Connection, account: AccountId, copy: RemoteCopy) -> Result<(), StoreError> {
    c.execute(
        "INSERT OR IGNORE INTO draft_remote_trash (account_id, validity, uid) VALUES (?1, ?2, ?3)",
        params![account.get(), copy.validity.get(), copy.uid.get()],
    )
    .map(|_| ())
    .map_err(store_error)
}

/// Supprime un brouillon ; sa copie distante rejoint la corbeille.
fn delete_draft_in(c: &Connection, id: DraftId) -> Result<Option<StoredDraft>, StoreError> {
    let Some(stored) = load_draft(c, id)? else {
        return Ok(None);
    };
    c.execute("DELETE FROM drafts WHERE id = ?1", [id.get()])
        .map_err(store_error)?;
    if let Some(copy) = stored.sync.remote {
        trash_insert(c, stored.draft.account, copy)?;
    }
    Ok(Some(stored))
}

impl DraftStore for SqliteComposeStore {
    async fn save_draft(
        &self,
        id: Option<DraftId>,
        draft: &Draft,
        now: i64,
    ) -> Result<StoredDraft, StoreError> {
        let content = content_of(draft)?;
        let account = draft.account;
        self.db
            .run(move |c| {
                let tx = c
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(store_error)?;
                let id = match id {
                    None => {
                        tx.execute(
                            "INSERT INTO drafts (account_id, content, revision, updated_at) \
                             VALUES (?1, ?2, 1, ?3)",
                            params![account.get(), content, now],
                        )
                        .map_err(store_error)?;
                        DraftId::new(tx.last_insert_rowid())
                            .ok_or_else(|| corrupted("identifiant de brouillon"))?
                    }
                    Some(id) => {
                        let n = tx
                            .execute(
                                "UPDATE drafts SET content = ?3, revision = revision + 1, \
                                 updated_at = ?4 WHERE id = ?1 AND account_id = ?2",
                                params![id.get(), account.get(), content, now],
                            )
                            .map_err(store_error)?;
                        if n == 0 {
                            return Err(StoreError("brouillon inconnu".into()));
                        }
                        id
                    }
                };
                let stored = load_draft(&tx, id)?.ok_or_else(|| corrupted("brouillon"))?;
                tx.commit().map_err(store_error)?;
                Ok(stored)
            })
            .await
    }

    async fn draft(&self, id: DraftId) -> Result<Option<StoredDraft>, StoreError> {
        self.db.read(move |c| load_draft(c, id)).await
    }

    async fn drafts(&self, account: AccountId) -> Result<Vec<StoredDraft>, StoreError> {
        self.db
            .read(move |c| {
                let mut stmt = c
                    .prepare_cached(&format!(
                        // sql-const : liste de colonnes constante.
                        "SELECT {DRAFT_COLUMNS} FROM drafts WHERE account_id = ?1 \
                         ORDER BY updated_at DESC, id DESC"
                    ))
                    .map_err(store_error)?;
                let rows: Vec<DraftRow> = stmt
                    .query_map([account.get()], read_draft_row)
                    .map_err(store_error)?
                    .collect::<Result<_, _>>()
                    .map_err(store_error)?;
                rows.into_iter().map(stored_of).collect()
            })
            .await
    }

    async fn set_remote(
        &self,
        account: AccountId,
        id: DraftId,
        sync: DraftSync,
        replaced: Option<RemoteCopy>,
    ) -> Result<bool, StoreError> {
        self.db
            .run(move |c| {
                let tx = c
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(store_error)?;
                let remote_revision = sync
                    .remote_revision
                    .map(|r| i64::try_from(r).map_err(|_| corrupted("révision")))
                    .transpose()?;
                let n = tx
                    .execute(
                        "UPDATE drafts SET remote_revision = ?3, remote_validity = ?4, \
                         remote_uid = ?5, last_remote_save = ?6 \
                         WHERE id = ?1 AND account_id = ?2",
                        params![
                            id.get(),
                            account.get(),
                            remote_revision,
                            sync.remote.map(|r| r.validity.get()),
                            sync.remote.map(|r| r.uid.get()),
                            sync.last_remote_save,
                        ],
                    )
                    .map_err(store_error)?;
                let found = n > 0;
                if !found && let Some(copy) = sync.remote {
                    // Brouillon disparu pendant la copie : à supprimer.
                    trash_insert(&tx, account, copy)?;
                }
                if let Some(old) = replaced {
                    trash_insert(&tx, account, old)?;
                }
                tx.commit().map_err(store_error)?;
                Ok(found)
            })
            .await
    }

    async fn delete_draft(&self, id: DraftId) -> Result<Option<StoredDraft>, StoreError> {
        self.db
            .run(move |c| {
                let tx = c
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(store_error)?;
                let removed = delete_draft_in(&tx, id)?;
                tx.commit().map_err(store_error)?;
                Ok(removed)
            })
            .await
    }

    async fn remote_trash(&self, account: AccountId) -> Result<Vec<RemoteCopy>, StoreError> {
        self.db
            .read(move |c| {
                let mut stmt = c
                    .prepare_cached(
                        "SELECT validity, uid FROM draft_remote_trash WHERE account_id = ?1 \
                         ORDER BY validity, uid",
                    )
                    .map_err(store_error)?;
                let rows: Vec<(i64, i64)> = stmt
                    .query_map([account.get()], |r| Ok((r.get(0)?, r.get(1)?)))
                    .map_err(store_error)?
                    .collect::<Result<_, _>>()
                    .map_err(store_error)?;
                rows.into_iter()
                    .map(|(v, u)| copy_of(Some(v), Some(u))?.ok_or_else(|| corrupted("copie")))
                    .collect()
            })
            .await
    }

    async fn forget_remote(&self, account: AccountId, copy: RemoteCopy) -> Result<(), StoreError> {
        self.db
            .run(move |c| {
                c.execute(
                    "DELETE FROM draft_remote_trash WHERE account_id = ?1 AND validity = ?2 \
                     AND uid = ?3",
                    params![account.get(), copy.validity.get(), copy.uid.get()],
                )
                .map(|_| ())
                .map_err(store_error)
            })
            .await
    }
}

// --- File d'envoi ---

const fn failure_name(f: SendFailure) -> &'static str {
    match f {
        SendFailure::Unreachable => "unreachable",
        SendFailure::Temporary => "temporary",
        SendFailure::Rejected => "rejected",
        SendFailure::Authentication => "authentication",
        SendFailure::Security => "security",
        SendFailure::Uncertain => "uncertain",
        SendFailure::TooManyAttempts => "too_many_attempts",
    }
}

fn failure_of(raw: &str) -> Result<SendFailure, StoreError> {
    Ok(match raw {
        "unreachable" => SendFailure::Unreachable,
        "temporary" => SendFailure::Temporary,
        "rejected" => SendFailure::Rejected,
        "authentication" => SendFailure::Authentication,
        "security" => SendFailure::Security,
        "uncertain" => SendFailure::Uncertain,
        "too_many_attempts" => SendFailure::TooManyAttempts,
        _ => return Err(corrupted("cause d'échec")),
    })
}

/// Colonnes (state, not_before, failure, attempts) d'un état.
fn entry_columns(e: OutboxEntry) -> (&'static str, Option<i64>, Option<&'static str>, i64) {
    let (state, not_before, failure) = match e.state {
        OutboxState::Queued { not_before } => ("queued", Some(not_before), None),
        OutboxState::Sending => ("sending", None, None),
        OutboxState::Sent => ("sent", None, None),
        OutboxState::Failed(f) => ("failed", None, Some(failure_name(f))),
    };
    (state, not_before, failure, i64::from(e.attempts))
}

fn entry_of(
    state: &str,
    not_before: Option<i64>,
    failure: Option<&str>,
    attempts: i64,
) -> Result<OutboxEntry, StoreError> {
    let state = match (state, not_before, failure) {
        ("queued", Some(not_before), None) => OutboxState::Queued { not_before },
        ("sending", None, None) => OutboxState::Sending,
        ("sent", None, None) => OutboxState::Sent,
        ("failed", None, Some(f)) => OutboxState::Failed(failure_of(f)?),
        _ => return Err(corrupted("état d'envoi")),
    };
    Ok(OutboxEntry {
        state,
        attempts: u32::try_from(attempts).map_err(|_| corrupted("essais"))?,
    })
}

impl OutboxStore for SqliteComposeStore {
    async fn enqueue(
        &self,
        account: AccountId,
        message: &OutgoingMessage,
        sent_copy: &[u8],
        subject: &str,
        draft: Option<DraftId>,
        now: i64,
    ) -> Result<OutboxId, StoreError> {
        let recipients = serde_json::to_string(&message.recipients)
            .map_err(|e| StoreError(format!("sérialisation : {e}")))?;
        let from = message.from.clone();
        let raw = message.raw.clone();
        let sent_copy = sent_copy.to_vec();
        let subject = subject.to_owned();
        let (state, not_before, failure, attempts) = entry_columns(OutboxEntry::queued(now));
        self.db
            .run(move |c| {
                let tx = c
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(store_error)?;
                if let Some(draft) = draft {
                    delete_draft_in(&tx, draft)?;
                }
                tx.execute(
                    "INSERT INTO outbox (account_id, envelope_from, recipients, raw, sent_copy, \
                     subject, state, not_before, failure, attempts, queued_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        account.get(),
                        from,
                        recipients,
                        raw,
                        sent_copy,
                        subject,
                        state,
                        not_before,
                        failure,
                        attempts,
                        now
                    ],
                )
                .map_err(store_error)?;
                let id = OutboxId::new(tx.last_insert_rowid())
                    .ok_or_else(|| corrupted("identifiant d'envoi"))?;
                tx.commit().map_err(store_error)?;
                Ok(id)
            })
            .await
    }

    async fn outbox(&self, account: AccountId) -> Result<Vec<OutboxItem>, StoreError> {
        type Row = (
            i64,
            String,
            String,
            Vec<u8>,
            Vec<u8>,
            String,
            String,
            Option<i64>,
            Option<String>,
            i64,
            i64,
        );
        self.db
            .read(move |c| {
                let mut stmt = c
                    .prepare_cached(
                        "SELECT id, envelope_from, recipients, raw, sent_copy, subject, state, \
                         not_before, failure, attempts, queued_at FROM outbox \
                         WHERE account_id = ?1 ORDER BY id",
                    )
                    .map_err(store_error)?;
                let rows: Vec<Row> = stmt
                    .query_map([account.get()], |r| {
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
                        ))
                    })
                    .map_err(store_error)?
                    .collect::<Result<_, _>>()
                    .map_err(store_error)?;
                rows.into_iter()
                    .map(
                        |(id, from, recipients, raw, sent_copy, subject, state, nb, f, att, at)| {
                            Ok(OutboxItem {
                                id: OutboxId::new(id)
                                    .ok_or_else(|| corrupted("identifiant d'envoi"))?,
                                account,
                                message: OutgoingMessage {
                                    from,
                                    recipients: serde_json::from_str(&recipients)
                                        .map_err(|_| corrupted("destinataires"))?,
                                    raw,
                                },
                                sent_copy,
                                subject,
                                entry: entry_of(&state, nb, f.as_deref(), att)?,
                                queued_at: at,
                            })
                        },
                    )
                    .collect()
            })
            .await
    }

    async fn transition(
        &self,
        id: OutboxId,
        from: OutboxEntry,
        to: OutboxEntry,
    ) -> Result<bool, StoreError> {
        let (fs, fnb, ff, fa) = entry_columns(from);
        let (ts, tnb, tf, ta) = entry_columns(to);
        self.db
            .run(move |c| {
                let n = c
                    .execute(
                        "UPDATE outbox SET state = ?6, not_before = ?7, failure = ?8, \
                         attempts = ?9 WHERE id = ?1 AND state = ?2 AND not_before IS ?3 \
                         AND failure IS ?4 AND attempts = ?5",
                        params![id.get(), fs, fnb, ff, fa, ts, tnb, tf, ta],
                    )
                    .map_err(store_error)?;
                Ok(n > 0)
            })
            .await
    }

    async fn remove(&self, id: OutboxId) -> Result<bool, StoreError> {
        self.db
            .run(move |c| {
                let n = c
                    .execute("DELETE FROM outbox WHERE id = ?1", [id.get()])
                    .map_err(store_error)?;
                Ok(n > 0)
            })
            .await
    }
}

#[cfg(test)]
mod tests;
