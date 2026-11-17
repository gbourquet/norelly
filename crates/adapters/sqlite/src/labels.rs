//! Étiquettes : liste d'un compte (`labels`), réécritures en attente
//! (`label_rewrites`) et mots-clés permanents des dossiers
//! (`mailboxes.permanent_flags`). Une ligne illisible (mot-clé ou couleur
//! invalide) est ignorée : elle ne bloque jamais l'affichage.

use std::collections::BTreeSet;

use norelly_domain::account::AccountId;
use norelly_domain::labels::{
    KeywordPermission, LabelColor, LabelRewrite, MAX_LABELS, PendingFollow, StoredLabel,
    rewrite_flags,
};
use norelly_domain::mailbox::{Keyword, MailboxId, MailboxName};
use norelly_domain::ports::{LabelRewriteId, LabelStore, StoreError};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::actions::readable_ops;
use crate::mail::{SqliteMailStore, flags_from, keywords_text, mailbox_id};
use crate::store_error;

/// Mots-clés distincts lus au plus pour la liste d'un compte.
const MAX_SEEN_KEYWORDS: usize = 2 * MAX_LABELS;

fn stored_label(keyword: &str, color: Option<&str>, hidden: bool) -> Option<StoredLabel> {
    Some(StoredLabel {
        keyword: Keyword::parse(keyword)?,
        color: match color {
            Some(c) => Some(LabelColor::parse(c)?),
            None => None,
        },
        hidden,
    })
}

fn color_of(
    c: &Connection,
    account: AccountId,
    keyword: &Keyword,
) -> Result<Option<LabelColor>, StoreError> {
    let color: Option<Option<String>> = c
        .query_row(
            "SELECT color FROM labels WHERE account_id = ?1 AND keyword = ?2",
            params![account.get(), keyword.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    Ok(color.flatten().as_deref().and_then(LabelColor::parse))
}

fn upsert(c: &Connection, account: AccountId, label: &StoredLabel) -> Result<(), StoreError> {
    c.execute(
        "INSERT INTO labels (account_id, keyword, color, hidden) VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT (account_id, keyword) DO UPDATE SET color = excluded.color, \
         hidden = excluded.hidden",
        params![
            account.get(),
            label.keyword.as_str(),
            label.color.map(LabelColor::to_hex),
            label.hidden
        ],
    )
    .map_err(store_error)?;
    Ok(())
}

/// Réécrit en local les mots-clés des messages du compte qui portent
/// `rewrite.from`.
fn rewrite_messages(
    c: &Connection,
    account: AccountId,
    rewrite: &LabelRewrite,
) -> Result<(), StoreError> {
    let sql = "SELECT m.id, m.flags, m.keywords FROM messages m \
               JOIN mailboxes b ON b.id = m.mailbox_id \
               WHERE b.account_id = ?1 AND instr(' ' || m.keywords || ' ', ' ' || ?2 || ' ') > 0";
    let rows: Vec<(i64, i64, String)> = c
        .prepare(sql)
        .and_then(|mut st| {
            st.query_map(params![account.get(), rewrite.from.as_str()], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .collect()
        })
        .map_err(store_error)?;
    let mut update = c
        .prepare_cached("UPDATE messages SET keywords = ?2 WHERE id = ?1")
        .map_err(store_error)?;
    for (id, bits, keywords) in rows {
        let mut flags = flags_from(bits, &keywords);
        rewrite_flags(&mut flags, &rewrite.from, rewrite.to.as_ref());
        update
            .execute(params![id, keywords_text(&flags)])
            .map_err(store_error)?;
    }
    Ok(())
}

/// Opérations en attente sur l'ancien mot-clé : elles suivent la
/// réécriture (domaine : [`LabelRewrite::follow`]).
fn follow_pending_ops(
    c: &Connection,
    account: AccountId,
    rewrite: &LabelRewrite,
) -> Result<(), StoreError> {
    for (id, op) in readable_ops(c, account)? {
        match rewrite.follow(&op) {
            PendingFollow::Unchanged => {}
            PendingFollow::Rewritten(_) => {
                c.execute(
                    "UPDATE pending_ops SET keyword = ?3 WHERE id = ?1 AND account_id = ?2",
                    params![
                        id.get(),
                        account.get(),
                        rewrite.to.as_ref().map(Keyword::as_str)
                    ],
                )
                .map_err(store_error)?;
            }
            PendingFollow::Dropped => {
                c.execute(
                    "DELETE FROM pending_ops WHERE id = ?1 AND account_id = ?2",
                    params![id.get(), account.get()],
                )
                .map_err(store_error)?;
            }
        }
    }
    Ok(())
}

impl LabelStore for SqliteMailStore {
    async fn stored_labels(&self, account: AccountId) -> Result<Vec<StoredLabel>, StoreError> {
        self.db()
            .read(move |c| {
                let rows: Vec<(String, Option<String>, bool)> = c
                    .prepare_cached(
                        "SELECT keyword, color, hidden FROM labels WHERE account_id = ?1 \
                         ORDER BY keyword",
                    )
                    .and_then(|mut st| {
                        st.query_map([account.get()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                            .collect()
                    })
                    .map_err(store_error)?;
                Ok(rows
                    .iter()
                    .filter_map(|(k, color, hidden)| stored_label(k, color.as_deref(), *hidden))
                    .collect())
            })
            .await
    }

    async fn seen_keywords(&self, account: AccountId) -> Result<BTreeSet<Keyword>, StoreError> {
        self.db()
            .read(move |c| {
                let mut stmt = c
                    .prepare_cached(
                        "SELECT DISTINCT m.keywords FROM messages m \
                         JOIN mailboxes b ON b.id = m.mailbox_id \
                         WHERE b.account_id = ?1 AND m.keywords <> '' AND m.hidden = 0 \
                         AND m.pending_op IS NULL",
                    )
                    .map_err(store_error)?;
                let mut rows = stmt.query([account.get()]).map_err(store_error)?;
                let mut out = BTreeSet::new();
                while let Some(row) = rows.next().map_err(store_error)? {
                    let text: String = row.get(0).map_err(store_error)?;
                    for word in text.split(' ') {
                        if out.len() >= MAX_SEEN_KEYWORDS {
                            return Ok(out);
                        }
                        if let Some(k) = Keyword::parse(word) {
                            out.insert(k);
                        }
                    }
                }
                Ok(out)
            })
            .await
    }

    async fn save_label(&self, account: AccountId, label: &StoredLabel) -> Result<(), StoreError> {
        let label = label.clone();
        self.db().run(move |c| upsert(c, account, &label)).await
    }

    async fn record_rewrite(
        &self,
        account: AccountId,
        rewrite: &LabelRewrite,
    ) -> Result<LabelRewriteId, StoreError> {
        let rewrite = rewrite.clone();
        self.db()
            .run(move |c| {
                let tx = c
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(store_error)?;
                tx.execute(
                    "INSERT INTO label_rewrites (account_id, from_keyword, to_keyword) \
                     VALUES (?1, ?2, ?3)",
                    params![
                        account.get(),
                        rewrite.from.as_str(),
                        rewrite.to.as_ref().map(Keyword::as_str)
                    ],
                )
                .map_err(store_error)?;
                let id = LabelRewriteId::new(tx.last_insert_rowid())
                    .ok_or_else(|| StoreError("identifiant de réécriture".into()))?;
                rewrite_messages(&tx, account, &rewrite)?;
                follow_pending_ops(&tx, account, &rewrite)?;
                let color = color_of(&tx, account, &rewrite.from)?;
                tx.execute(
                    "DELETE FROM labels WHERE account_id = ?1 AND keyword = ?2",
                    params![account.get(), rewrite.from.as_str()],
                )
                .map_err(store_error)?;
                if let Some(to) = &rewrite.to {
                    let color = color.or(color_of(&tx, account, to)?);
                    upsert(
                        &tx,
                        account,
                        &StoredLabel {
                            keyword: to.clone(),
                            color,
                            hidden: false,
                        },
                    )?;
                }
                tx.commit().map_err(store_error)?;
                Ok(id)
            })
            .await
    }

    async fn label_rewrites(
        &self,
        account: AccountId,
    ) -> Result<Vec<(LabelRewriteId, LabelRewrite)>, StoreError> {
        self.db()
            .read(move |c| {
                let rows: Vec<(i64, String, Option<String>)> = c
                    .prepare_cached(
                        "SELECT id, from_keyword, to_keyword FROM label_rewrites \
                         WHERE account_id = ?1 ORDER BY id",
                    )
                    .and_then(|mut st| {
                        st.query_map([account.get()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                            .collect()
                    })
                    .map_err(store_error)?;
                Ok(rows
                    .iter()
                    .filter_map(|(id, from, to)| {
                        let to = match to {
                            Some(t) => Some(Keyword::parse(t)?),
                            None => None,
                        };
                        Some((
                            LabelRewriteId::new(*id)?,
                            LabelRewrite {
                                from: Keyword::parse(from)?,
                                to,
                            },
                        ))
                    })
                    .collect())
            })
            .await
    }

    async fn claim_rewrite(
        &self,
        account: AccountId,
        id: LabelRewriteId,
    ) -> Result<Option<u32>, StoreError> {
        self.db()
            .run(move |c| {
                let attempts: Option<i64> = c
                    .query_row(
                        "UPDATE label_rewrites SET attempts = attempts + 1 \
                         WHERE id = ?1 AND account_id = ?2 RETURNING attempts",
                        params![id.get(), account.get()],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(store_error)?;
                Ok(attempts.map(|n| u32::try_from(n).unwrap_or(u32::MAX)))
            })
            .await
    }

    async fn complete_rewrite(
        &self,
        account: AccountId,
        id: LabelRewriteId,
    ) -> Result<(), StoreError> {
        self.db()
            .run(move |c| {
                c.execute(
                    "DELETE FROM label_rewrites WHERE id = ?1 AND account_id = ?2",
                    params![id.get(), account.get()],
                )
                .map_err(store_error)?;
                Ok(())
            })
            .await
    }

    async fn keyword_permissions(
        &self,
        account: AccountId,
    ) -> Result<Vec<(MailboxId, KeywordPermission)>, StoreError> {
        self.db()
            .read(move |c| {
                let rows: Vec<(i64, String)> = c
                    .prepare_cached(
                        "SELECT id, permanent_flags FROM mailboxes \
                         WHERE account_id = ?1 AND permanent_flags IS NOT NULL ORDER BY id",
                    )
                    .and_then(|mut st| {
                        st.query_map([account.get()], |r| Ok((r.get(0)?, r.get(1)?)))?
                            .collect()
                    })
                    .map_err(store_error)?;
                rows.iter()
                    .map(|(id, raw)| {
                        Ok((mailbox_id(*id)?, KeywordPermission::from_storage(Some(raw))))
                    })
                    .collect()
            })
            .await
    }

    async fn forget_flag_state(
        &self,
        account: AccountId,
        mailbox: &MailboxName,
    ) -> Result<(), StoreError> {
        let mailbox = mailbox.clone();
        self.db()
            .run(move |c| {
                c.execute(
                    "UPDATE mailboxes SET highest_modseq = NULL \
                     WHERE account_id = ?1 AND name = ?2",
                    params![account.get(), mailbox.as_str()],
                )
                .map_err(store_error)?;
                Ok(())
            })
            .await
    }

    async fn set_keyword_permission(
        &self,
        account: AccountId,
        mailbox: &MailboxName,
        permission: &KeywordPermission,
    ) -> Result<(), StoreError> {
        let (mailbox, stored) = (mailbox.clone(), permission.to_storage());
        self.db()
            .run(move |c| {
                c.execute(
                    "UPDATE mailboxes SET permanent_flags = ?3 WHERE account_id = ?1 AND name = ?2",
                    params![account.get(), mailbox.as_str(), stored],
                )
                .map_err(store_error)?;
                Ok(())
            })
            .await
    }
}
