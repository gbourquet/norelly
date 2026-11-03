//! Suite de contrat de [`PendingOpStore`] (file d'opérations et effets
//! locaux), exécutée contre le faux en mémoire et contre SQLite.

use norelly_domain::account::AccountId;
use norelly_domain::actions::{
    ActionFlag, Effect, FlagChange, LocalMessage, MessageAction, OpKind, PendingOp, QueueChange,
    resolve,
};
use norelly_domain::conversation::ConversationScope;
use norelly_domain::mailbox::{Flags, MailboxRole};
use norelly_domain::ports::{
    ConversationQuery, FolderTreeQuery, MailStore, MessageId, MessageListQuery, PendingOpStore,
    StoreError, SyncBatch,
};
use norelly_domain::sync::FlagUpdate;

use crate::mail_contracts::{mailbox, sample_message, state, uid};

fn fail(what: &str) -> StoreError {
    StoreError(format!("contrat : {what}"))
}

async fn visible_ids<S: MessageListQuery>(
    store: &S,
    mailbox: norelly_domain::mailbox::MailboxId,
) -> Result<Vec<MessageId>, StoreError> {
    Ok(store
        .page(mailbox, None, 100)
        .await?
        .items
        .into_iter()
        .map(|i| i.id)
        .collect())
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

async fn target<S: PendingOpStore>(store: &S, id: MessageId) -> Result<LocalMessage, StoreError> {
    store
        .action_target(id)
        .await?
        .ok_or_else(|| fail("message visé introuvable"))
}

/// Enfile une seule opération.
async fn record_one<S: PendingOpStore>(
    store: &S,
    op: &PendingOp,
) -> Result<norelly_domain::ports::PendingOpId, StoreError> {
    match store.record_all(&[QueueChange::Push(op.clone())]).await?[..] {
        [id] => Ok(id),
        _ => Err(fail("une opération enfilée attendue")),
    }
}

/// Contrat de [`PendingOpStore`] pour `account` (qui doit exister et
/// n'avoir aucune boîte) ; `other` : second compte (isolation).
pub async fn pending_op_store_contract<S>(
    store: &S,
    account: AccountId,
    other: AccountId,
) -> Result<(), StoreError>
where
    S: PendingOpStore + MailStore + MessageListQuery + FolderTreeQuery + ConversationQuery,
{
    let boxes = store
        .replace_mailboxes(
            account,
            &[
                mailbox("INBOX", MailboxRole::Inbox, true),
                mailbox("Corbeille", MailboxRole::Trash, true),
                mailbox("Projets", MailboxRole::Normal, true),
            ],
        )
        .await?;
    let id_of = |name: &str| {
        boxes
            .iter()
            .find(|m| m.mailbox.name.as_str() == name)
            .map(|m| m.id)
            .ok_or_else(|| fail(name))
    };
    let (inbox, trash, projets) = (id_of("INBOX")?, id_of("Corbeille")?, id_of("Projets")?);
    store
        .apply_sync(
            inbox,
            &SyncBatch {
                new: vec![
                    sample_message(1, 1_000, false),
                    sample_message(2, 2_000, false),
                ],
                state: state(7, 50, 3),
                ..SyncBatch::default()
            },
        )
        .await?;
    let ids = visible_ids(store, inbox).await?;
    let (m2, m1) = match ids.as_slice() {
        [a, b] => (*a, *b),
        _ => return Err(fail("deux messages attendus")),
    };

    // --- Message visé et dossiers ---
    let t1 = target(store, m1).await?;
    assert_eq!(t1.account, account);
    assert_eq!(t1.mailbox.id, inbox);
    assert_eq!(t1.mailbox.role, MailboxRole::Inbox);
    assert_eq!(t1.uid, uid(1));
    assert_eq!(t1.uid_validity.map(|v| v.get()), Some(7));
    assert_eq!(t1.highest_modseq.map(|m| m.get()), Some(50));
    assert!(!t1.flags.seen);
    let folders = store.action_folders(account).await?;
    assert_eq!(folders.len(), 3);
    assert!(store.action_folders(other).await?.is_empty());
    assert_eq!(
        store
            .action_target(MessageId::new(999_999).ok_or_else(|| fail("id"))?)
            .await?,
        None
    );

    // --- Drapeau : application optimiste ---
    let seen = Effect::SetFlag(FlagChange {
        flag: ActionFlag::Seen,
        value: true,
    });
    let op1 = PendingOp::new(&seen, &t1, 100).ok_or_else(|| fail("opération"))?;
    let id1 = record_one(store, &op1).await?;
    assert!(flags_of(store, inbox, m1).await?.seen, "lu tout de suite");
    let unread = |nodes: Vec<norelly_domain::ports::FolderNode>| {
        nodes
            .iter()
            .find(|n| n.id == inbox)
            .map(|n| (n.total, n.unread))
    };
    assert_eq!(unread(store.tree(account).await?), Some((2, 1)));

    // --- Déplacement vers la Corbeille : message masqué ---
    let t2 = target(store, m2).await?;
    let delete = resolve(MessageAction::Delete, &t2.mailbox, &folders, None)
        .map_err(|e| fail(&e.to_string()))?;
    assert!(matches!(&delete, Effect::MoveTo(f) if f.id == trash));
    let op2 = PendingOp::new(&delete, &t2, 100).ok_or_else(|| fail("opération"))?;
    let id2 = record_one(store, &op2).await?;
    assert_eq!(visible_ids(store, inbox).await?, [m1], "masqué");
    assert_eq!(
        store.action_target(m2).await?,
        None,
        "plus visible pour une action"
    );
    assert_eq!(unread(store.tree(account).await?), Some((1, 0)));
    // Toujours connu de la synchronisation.
    assert_eq!(store.local_mailbox(inbox).await?.messages.len(), 2);
    // Copie provisoire visible tout de suite dans la Corbeille (liste,
    // compteurs), mais ignorée par la synchronisation et par les actions.
    let trash_page = store.page(trash, None, 10).await?;
    assert_eq!(
        trash_page.items.len(),
        1,
        "copie provisoire dans la destination"
    );
    assert_eq!(
        trash_page.items.first().map(|i| i.subject.as_str()),
        Some("Sujet 2")
    );
    let copy = trash_page
        .items
        .first()
        .map(|i| i.id)
        .ok_or_else(|| fail("copie"))?;
    let trash_node = |nodes: Vec<norelly_domain::ports::FolderNode>| {
        nodes
            .iter()
            .find(|n| n.id == trash)
            .map(|n| (n.total, n.unread))
    };
    assert_eq!(trash_node(store.tree(account).await?), Some((1, 1)));
    assert!(
        store.local_mailbox(trash).await?.messages.is_empty(),
        "inconnue de la synchronisation"
    );
    assert!(store.missing_bodies(trash, 10).await?.is_empty());
    assert_eq!(
        store.action_target(copy).await?,
        None,
        "pas d'action sur une copie"
    );

    // --- File durable, dans l'ordre ---
    let pending = store.pending(account).await?;
    assert_eq!(
        pending.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        [id1, id2]
    );
    assert_eq!(pending.first().map(|(_, op)| op), Some(&op1));
    assert_eq!(pending.get(1).map(|(_, op)| op), Some(&op2));
    assert!(matches!(
        pending.get(1).map(|(_, op)| &op.kind),
        Some(OpKind::Move { to }) if to.as_str() == "Corbeille"
    ));
    assert!(store.pending(other).await?.is_empty());

    // --- La synchronisation rapporte l'état du serveur, puis réapplication ---
    store
        .apply_sync(
            inbox,
            &SyncBatch {
                flag_changes: vec![FlagUpdate {
                    uid: uid(1),
                    flags: Flags::default(),
                    modseq: None,
                }],
                state: state(7, 51, 3),
                ..SyncBatch::default()
            },
        )
        .await?;
    assert!(!flags_of(store, inbox, m1).await?.seen);
    store.reapply(account).await?;
    assert!(flags_of(store, inbox, m1).await?.seen, "effet réappliqué");
    assert_eq!(visible_ids(store, inbox).await?, [m1], "toujours masqué");

    // --- Réservation par le rejeu : plus annulable par l'utilisateur ---
    assert_eq!(store.claim(id2).await?, Some(1));
    assert_eq!(store.claim(id2).await?, Some(2), "chaque essai compté");
    assert!(
        !store.cancel(id2).await?,
        "réservée : trop tard pour annuler"
    );
    assert_eq!(visible_ids(store, inbox).await?, [m1], "toujours masqué");
    assert_eq!(
        store
            .claim(norelly_domain::ports::PendingOpId::new(999_999).ok_or_else(|| fail("id"))?)
            .await?,
        None
    );

    // --- Abandon par le rejeu : effet inversé, même réservée ---
    assert!(store.revert(id2).await?);
    assert!(!store.revert(id2).await?, "déjà retirée");
    assert_eq!(
        visible_ids(store, inbox).await?,
        [m2, m1],
        "de nouveau visible"
    );
    assert!(visible_ids(store, trash).await?.is_empty(), "copie retirée");

    // --- Rejeu réussi d'un déplacement : la copie reste jusqu'à la
    // synchronisation de la destination, puis la purge la retire ---
    let t2 = target(store, m2).await?;
    let op3 = PendingOp::new(&delete, &t2, 100).ok_or_else(|| fail("opération"))?;
    let id3 = record_one(store, &op3).await?;
    assert_eq!(visible_ids(store, trash).await?.len(), 1);
    assert_eq!(
        store.purge_placeholders(account, None).await?,
        0,
        "opération en file"
    );
    assert!(store.complete(id3).await?);
    assert_eq!(
        visible_ids(store, trash).await?.len(),
        1,
        "pas de trou avant la synchronisation"
    );
    store
        .apply_sync(
            trash,
            &SyncBatch {
                new: vec![sample_message(1, 2_000, false)],
                state: state(11, 3, 2),
                ..SyncBatch::default()
            },
        )
        .await?;
    assert_eq!(
        store.purge_placeholders(other, None).await?,
        0,
        "autre compte"
    );
    assert_eq!(
        store.purge_placeholders(account, Some(inbox)).await?,
        0,
        "autre dossier"
    );
    assert_eq!(store.purge_placeholders(account, Some(trash)).await?, 1);
    assert_eq!(
        visible_ids(store, trash).await?.len(),
        1,
        "le vrai message, sans doublon"
    );
    // Corbeille vidée pour la suite du contrat.
    store
        .apply_sync(
            trash,
            &SyncBatch {
                reset: true,
                state: state(11, 4, 2),
                ..SyncBatch::default()
            },
        )
        .await?;

    // --- Rejeu réussi : retirée, effet conservé ---
    assert!(store.complete(id1).await?);
    assert!(!store.complete(id1).await?);
    assert!(store.pending(account).await?.is_empty());
    assert!(flags_of(store, inbox, m1).await?.seen);

    // --- Membres d'une conversation : exemplaires compris, masqués exclus ---
    // Copie du message 1 dans « Projets » (même Message-ID).
    store
        .apply_sync(
            projets,
            &SyncBatch {
                new: vec![sample_message(1, 1_000, true)],
                state: state(9, 5, 2),
                ..SyncBatch::default()
            },
        )
        .await?;
    let conversation = store
        .conversations(ConversationScope::Mailbox(inbox), None, 10)
        .await?
        .items
        .into_iter()
        .find(|c| c.subject == "Sujet 1")
        .map(|c| c.id)
        .ok_or_else(|| fail("conversation du message 1"))?;
    let members = store.conversation_members(conversation).await?;
    assert_eq!(
        members.iter().map(|m| m.mailbox).collect::<Vec<_>>().len(),
        2,
        "les deux exemplaires : {members:?}"
    );
    assert!(
        members
            .iter()
            .any(|m| m.mailbox == projets && m.role == MailboxRole::Normal)
    );
    assert!(members.iter().any(|m| m.id == m1 && m.flags.seen));
    let t1 = target(store, m1).await?;
    let hide = Effect::MoveTo(norelly_domain::actions::FolderRef {
        id: trash,
        name: norelly_domain::mailbox::MailboxName::parse("Corbeille").map_err(|_| fail("nom"))?,
        role: MailboxRole::Trash,
        selectable: true,
    });
    let op = PendingOp::new(&hide, &t1, 100).ok_or_else(|| fail("opération"))?;
    let hidden = record_one(store, &op).await?;
    assert_eq!(
        store.conversation_members(conversation).await?.len(),
        1,
        "masqué exclu"
    );
    assert!(store.cancel(hidden).await?, "non réservée : annulable");
    assert!(!store.cancel(hidden).await?, "déjà annulée");
    assert_eq!(
        store.conversation_members(conversation).await?.len(),
        2,
        "de nouveau visible"
    );

    // --- Lot de changements : fusion et ajout en une seule transaction ---
    let unseen = Effect::SetFlag(FlagChange {
        flag: ActionFlag::Seen,
        value: false,
    });
    let op_a =
        PendingOp::new(&unseen, &target(store, m1).await?, 100).ok_or_else(|| fail("opération"))?;
    let id_a = record_one(store, &op_a).await?;
    assert!(!flags_of(store, inbox, m1).await?.seen);
    let op_b =
        PendingOp::new(&seen, &target(store, m1).await?, 100).ok_or_else(|| fail("opération"))?;
    let op_c =
        PendingOp::new(&delete, &target(store, m1).await?, 100).ok_or_else(|| fail("opération"))?;
    let ids = store
        .record_all(&[
            QueueChange::CancelOrPush {
                existing: id_a,
                op: op_b,
            },
            QueueChange::Push(op_c.clone()),
        ])
        .await?;
    let [id_c] = ids[..] else {
        return Err(fail("seul le déplacement est enfilé"));
    };
    assert_eq!(store.pending(account).await?, vec![(id_c, op_c)]);
    assert!(
        visible_ids(store, inbox).await?.is_empty(),
        "déplacement appliqué"
    );
    assert!(store.revert(id_c).await?);
    assert!(flags_of(store, inbox, m1).await?.seen, "fusion : annulée");
    // Opération réservée par le rejeu : trop tard, la nouvelle est enfilée.
    let op_d =
        PendingOp::new(&unseen, &target(store, m1).await?, 100).ok_or_else(|| fail("opération"))?;
    let id_d = record_one(store, &op_d).await?;
    assert_eq!(store.claim(id_d).await?, Some(1));
    let op_e =
        PendingOp::new(&seen, &target(store, m1).await?, 100).ok_or_else(|| fail("opération"))?;
    let ids = store
        .record_all(&[QueueChange::CancelOrPush {
            existing: id_d,
            op: op_e.clone(),
        }])
        .await?;
    let [id_e] = ids[..] else {
        return Err(fail("réservée : la nouvelle est enfilée"));
    };
    assert_eq!(
        store.pending(account).await?,
        vec![(id_d, op_d), (id_e, op_e)]
    );
    assert!(flags_of(store, inbox, m1).await?.seen);
    assert!(store.record_all(&[]).await?.is_empty());
    assert!(store.revert(id_e).await? && store.revert(id_d).await?);
    assert!(store.pending(account).await?.is_empty());

    // --- Dossier de repli d'« archiver » ---
    assert_eq!(store.archive_fallback(account).await?, None);
    store.set_archive_fallback(account, Some(projets)).await?;
    assert_eq!(store.archive_fallback(account).await?, Some(projets));
    assert!(
        store
            .set_archive_fallback(other, Some(projets))
            .await
            .is_err(),
        "dossier d'un autre compte refusé"
    );
    store.set_archive_fallback(account, None).await?;
    assert_eq!(store.archive_fallback(account).await?, None);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail_contracts::InMemoryMailStore;

    #[tokio::test]
    async fn in_memory_store_passes_pending_op_contract() -> Result<(), StoreError> {
        let (a, b) = (
            AccountId::new(1).map_err(|e| StoreError(e.to_string()))?,
            AccountId::new(2).map_err(|e| StoreError(e.to_string()))?,
        );
        pending_op_store_contract(&InMemoryMailStore::default(), a, b).await
    }
}
