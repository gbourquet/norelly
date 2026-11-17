//! Suite de contrat de [`LabelStore`] (étiquettes enregistrées, mots-clés
//! vus, réécritures en attente, mots-clés permanents) et des opérations de
//! mot-clé de [`PendingOpStore`], exécutée contre le faux en mémoire et
//! contre SQLite.

use std::collections::BTreeSet;

use norelly_domain::account::AccountId;
use norelly_domain::actions::{ActionFlag, Effect, FlagChange, PendingOp, QueueChange};
use norelly_domain::labels::{KeywordPermission, LabelColor, LabelRewrite, StoredLabel};
use norelly_domain::mailbox::{Flags, Keyword, MailboxRole};
use norelly_domain::ports::{
    LabelStore, MailStore, MessageId, MessageListQuery, PendingOpStore, StoreError, SyncBatch,
};

use crate::mail_contracts::{mailbox, sample_message, state};

fn fail(what: &str) -> StoreError {
    StoreError(format!("contrat des étiquettes : {what}"))
}

fn keyword(raw: &str) -> Result<Keyword, StoreError> {
    Keyword::parse(raw).ok_or_else(|| fail("mot-clé"))
}

async fn flags_of<S: MessageListQuery>(
    store: &S,
    mailbox: norelly_domain::mailbox::MailboxId,
    id: MessageId,
) -> Result<Flags, StoreError> {
    store
        .page(mailbox, None, 100)
        .await?
        .items
        .into_iter()
        .find(|i| i.id == id)
        .map(|i| i.flags)
        .ok_or_else(|| fail("message absent de la liste"))
}

/// Contrat de [`LabelStore`] pour `account` (qui doit exister et n'avoir
/// aucune boîte) ; `other` : second compte (isolation).
pub async fn label_store_contract<S>(
    store: &S,
    account: AccountId,
    other: AccountId,
) -> Result<(), StoreError>
where
    S: LabelStore + PendingOpStore + MailStore + MessageListQuery,
{
    let boxes = store
        .replace_mailboxes(
            account,
            &[
                mailbox("INBOX", MailboxRole::Inbox, true),
                mailbox("Projets", MailboxRole::Normal, true),
            ],
        )
        .await?;
    let inbox = boxes
        .iter()
        .find(|m| m.mailbox.name.as_str() == "INBOX")
        .map(|m| m.id)
        .ok_or_else(|| fail("INBOX"))?;
    let mut tagged = sample_message(1, 1_000, false);
    tagged.meta.flags = Flags::from_tokens(["Vieux", "$Junk", "Autre"]);
    store
        .apply_sync(
            inbox,
            &SyncBatch {
                new: vec![tagged, sample_message(2, 2_000, false)],
                state: state(7, 50, 3),
                ..SyncBatch::default()
            },
        )
        .await?;
    let ids: Vec<MessageId> = store
        .page(inbox, None, 100)
        .await?
        .items
        .into_iter()
        .map(|i| i.id)
        .collect();
    let (m2, m1) = match ids.as_slice() {
        [a, b] => (*a, *b),
        _ => return Err(fail("deux messages attendus")),
    };

    // --- Mots-clés vus ---
    assert_eq!(
        store.seen_keywords(account).await?,
        BTreeSet::from([keyword("Vieux")?, keyword("$Junk")?, keyword("Autre")?])
    );
    assert!(store.seen_keywords(other).await?.is_empty());

    // --- Étiquettes enregistrées ---
    assert!(store.stored_labels(account).await?.is_empty());
    let blue = LabelColor::parse("#1565c0").ok_or_else(|| fail("couleur"))?;
    let projet = StoredLabel {
        keyword: keyword("Projet")?,
        color: Some(blue),
        hidden: false,
    };
    store.save_label(account, &projet).await?;
    let vieux = StoredLabel {
        keyword: keyword("Vieux")?,
        color: None,
        hidden: true,
    };
    store.save_label(account, &vieux).await?;
    // Remplacement : la couleur et le masquage suivent.
    let vieux = StoredLabel {
        color: Some(blue),
        hidden: false,
        ..vieux
    };
    store.save_label(account, &vieux).await?;
    let mut labels = store.stored_labels(account).await?;
    labels.sort_by(|a, b| a.keyword.cmp(&b.keyword));
    assert_eq!(labels, [projet.clone(), vieux.clone()]);
    assert!(store.stored_labels(other).await?.is_empty());

    // --- Mots-clés permanents ---
    let target = store
        .action_target(m1)
        .await?
        .ok_or_else(|| fail("message visé"))?;
    assert_eq!(target.mailbox.keywords, KeywordPermission::Unknown);
    let listed = KeywordPermission::from_tokens(["Projet"]);
    store
        .set_keyword_permission(account, &target.mailbox.name, &listed)
        .await?;
    // Boîte d'un autre compte ou inconnue : sans effet.
    store
        .set_keyword_permission(other, &target.mailbox.name, &KeywordPermission::Any)
        .await?;
    let absent = norelly_domain::mailbox::MailboxName::parse("Absente").map_err(|_| fail("nom"))?;
    store
        .set_keyword_permission(account, &absent, &KeywordPermission::Any)
        .await?;
    assert_eq!(
        store.keyword_permissions(account).await?,
        [(inbox, listed.clone())]
    );
    let target = store
        .action_target(m1)
        .await?
        .ok_or_else(|| fail("message visé"))?;
    assert_eq!(target.mailbox.keywords, listed);
    let folders = store.action_folders(account).await?;
    assert!(
        folders
            .iter()
            .any(|f| f.id == inbox && f.keywords == listed),
        "{folders:?}"
    );
    store
        .set_keyword_permission(account, &target.mailbox.name, &KeywordPermission::Any)
        .await?;
    assert_eq!(
        store.keyword_permissions(account).await?,
        [(inbox, KeywordPermission::Any)]
    );

    // --- Opération de mot-clé : application optimiste et annulation ---
    let t2 = store
        .action_target(m2)
        .await?
        .ok_or_else(|| fail("message visé"))?;
    let tag = Effect::SetFlag(FlagChange {
        flag: ActionFlag::Keyword(keyword("Projet")?),
        value: true,
    });
    let op = PendingOp::new(&tag, &t2, 100).ok_or_else(|| fail("opération"))?;
    let recorded = store.record_all(&[QueueChange::Push(op.clone())]).await?;
    let [id] = recorded[..] else {
        return Err(fail("une opération attendue"));
    };
    assert!(
        flags_of(store, inbox, m2)
            .await?
            .keywords()
            .contains(&keyword("Projet")?),
        "étiquette posée tout de suite"
    );
    assert_eq!(store.pending(account).await?, [(id, op)]);
    assert!(store.cancel(id).await?);
    assert!(flags_of(store, inbox, m2).await?.keywords().is_empty());

    // --- Opération en attente sur l'ancien mot-clé : suit la réécriture ---
    let t2 = store
        .action_target(m2)
        .await?
        .ok_or_else(|| fail("message visé"))?;
    let tag_old = Effect::SetFlag(FlagChange {
        flag: ActionFlag::Keyword(keyword("Vieux")?),
        value: true,
    });
    let pending_old = PendingOp::new(&tag_old, &t2, 100).ok_or_else(|| fail("opération"))?;
    store
        .record_all(&[QueueChange::Push(pending_old.clone())])
        .await?;

    // --- Réécriture : renommage ---
    let rename = LabelRewrite {
        from: keyword("Vieux")?,
        to: Some(keyword("Neuf")?),
    };
    let rewrite = store.record_rewrite(account, &rename).await?;
    assert_eq!(
        flags_of(store, inbox, m1).await?,
        Flags::from_tokens(["Neuf", "$Junk", "Autre"]),
        "réécrit tout de suite en local"
    );
    let mut labels = store.stored_labels(account).await?;
    labels.sort_by(|a, b| a.keyword.cmp(&b.keyword));
    assert_eq!(
        labels,
        [
            StoredLabel {
                keyword: keyword("Neuf")?,
                color: Some(blue),
                hidden: false,
            },
            projet
        ],
        "la couleur suit le nouveau nom"
    );
    assert_eq!(
        store.label_rewrites(account).await?,
        [(rewrite, rename.clone())]
    );
    let followed: Vec<_> = store
        .pending(account)
        .await?
        .into_iter()
        .map(|(_, op)| op)
        .collect();
    assert_eq!(
        followed,
        [PendingOp {
            kind: norelly_domain::actions::OpKind::Flag {
                change: FlagChange {
                    flag: ActionFlag::Keyword(keyword("Neuf")?),
                    value: true,
                },
                previous: false,
                unchanged_since: t2.highest_modseq,
            },
            ..pending_old.clone()
        }],
        "l'opération en attente vise le nouveau nom"
    );
    assert!(
        flags_of(store, inbox, m2)
            .await?
            .keywords()
            .contains(&keyword("Neuf")?)
    );
    assert!(store.label_rewrites(other).await?.is_empty());
    assert_eq!(store.claim_rewrite(account, rewrite).await?, Some(1));
    assert_eq!(store.claim_rewrite(account, rewrite).await?, Some(2));
    assert_eq!(store.claim_rewrite(other, rewrite).await?, None);
    store.complete_rewrite(other, rewrite).await?;
    assert_eq!(store.label_rewrites(account).await?.len(), 1);
    store.complete_rewrite(account, rewrite).await?;
    assert!(store.label_rewrites(account).await?.is_empty());
    assert_eq!(store.claim_rewrite(account, rewrite).await?, None);

    // --- Réécriture : retrait de tous les messages ---
    let remove = LabelRewrite {
        from: keyword("Neuf")?,
        to: None,
    };
    store.record_rewrite(account, &remove).await?;
    assert_eq!(
        flags_of(store, inbox, m1).await?,
        Flags::from_tokens(["$Junk", "Autre"])
    );
    assert!(
        store.pending(account).await?.is_empty(),
        "opération sans objet retirée"
    );
    assert!(flags_of(store, inbox, m2).await?.keywords().is_empty());

    // --- Drapeaux à relire entièrement à la prochaine synchronisation ---
    assert!(
        store
            .local_mailbox(inbox)
            .await?
            .state
            .highest_modseq
            .is_some()
    );
    let name = norelly_domain::mailbox::MailboxName::parse("INBOX").map_err(|_| fail("nom"))?;
    store.forget_flag_state(other, &name).await?;
    assert!(
        store
            .local_mailbox(inbox)
            .await?
            .state
            .highest_modseq
            .is_some()
    );
    store.forget_flag_state(account, &name).await?;
    assert_eq!(store.local_mailbox(inbox).await?.state.highest_modseq, None);
    assert!(
        !store
            .stored_labels(account)
            .await?
            .iter()
            .any(|l| l.keyword == remove.from)
    );

    // --- Abandon : réécriture retirée, tous les dossiers du compte à
    // relire, d'un coup ---
    let projets = boxes
        .iter()
        .find(|m| m.mailbox.name.as_str() == "Projets")
        .map(|m| m.id)
        .ok_or_else(|| fail("Projets"))?;
    store
        .apply_sync(
            projets,
            &SyncBatch {
                state: state(9, 60, 1),
                ..SyncBatch::default()
            },
        )
        .await?;
    assert!(
        store
            .local_mailbox(projets)
            .await?
            .state
            .highest_modseq
            .is_some()
    );
    let before = store.label_rewrites(account).await?.len();
    let stalled = store
        .record_rewrite(
            account,
            &LabelRewrite {
                from: keyword("Absent")?,
                to: None,
            },
        )
        .await?;
    store.abandon_rewrite(other, stalled).await?;
    assert_eq!(
        store.label_rewrites(account).await?.len(),
        before + 1,
        "autre compte : rien"
    );
    assert!(
        store
            .local_mailbox(projets)
            .await?
            .state
            .highest_modseq
            .is_some()
    );
    store.abandon_rewrite(account, stalled).await?;
    assert_eq!(store.label_rewrites(account).await?.len(), before);
    assert_eq!(
        store.local_mailbox(projets).await?.state.highest_modseq,
        None
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail_contracts::InMemoryMailStore;

    #[tokio::test]
    async fn in_memory_store_passes_label_contract() -> Result<(), StoreError> {
        let (a, b) = (
            AccountId::new(1).map_err(|e| StoreError(e.to_string()))?,
            AccountId::new(2).map_err(|e| StoreError(e.to_string()))?,
        );
        label_store_contract(&InMemoryMailStore::default(), a, b).await
    }
}
