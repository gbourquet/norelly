//! Suite de contrat de [`PendingOpStore`] (file d'opérations et effets
//! locaux), exécutée contre le faux en mémoire et contre SQLite.

use norelly_domain::account::AccountId;
use norelly_domain::actions::{
    ActionFlag, Effect, FlagChange, LocalMessage, MessageAction, OpKind, PendingOp, resolve,
};
use norelly_domain::mailbox::{Flags, MailboxRole};
use norelly_domain::ports::{
    FolderTreeQuery, MailStore, MessageId, MessageListQuery, PendingOpStore, StoreError, SyncBatch,
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

/// Contrat de [`PendingOpStore`] pour `account` (qui doit exister et
/// n'avoir aucune boîte) ; `other` : second compte (isolation).
pub async fn pending_op_store_contract<S>(
    store: &S,
    account: AccountId,
    other: AccountId,
) -> Result<(), StoreError>
where
    S: PendingOpStore + MailStore + MessageListQuery + FolderTreeQuery,
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
    let id1 = store.record(&op1).await?;
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
    let id2 = store.record(&op2).await?;
    assert_eq!(visible_ids(store, inbox).await?, [m1], "masqué");
    assert_eq!(
        store.action_target(m2).await?,
        None,
        "plus visible pour une action"
    );
    assert_eq!(unread(store.tree(account).await?), Some((1, 0)));
    // Toujours connu de la synchronisation.
    assert_eq!(store.local_mailbox(inbox).await?.messages.len(), 2);

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

    // --- Annulation : effet inversé ---
    assert!(store.revert(id2).await?);
    assert!(!store.revert(id2).await?, "déjà retirée");
    assert_eq!(
        visible_ids(store, inbox).await?,
        [m2, m1],
        "de nouveau visible"
    );

    // --- Rejeu réussi : retirée, effet conservé ---
    assert!(store.complete(id1).await?);
    assert!(!store.complete(id1).await?);
    assert!(store.pending(account).await?.is_empty());
    assert!(flags_of(store, inbox, m1).await?.seen);

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
