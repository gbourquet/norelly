//! Suites de contrat de [`DraftStore`] et [`OutboxStore`], exécutées
//! contre le faux en mémoire ([`InMemoryComposeStore`]) et contre SQLite.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

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

fn fail(what: &str) -> StoreError {
    StoreError(format!("contrat : {what}"))
}

fn address(raw: &str) -> Result<EmailAddress, StoreError> {
    EmailAddress::parse(raw).map_err(|_| fail(raw))
}

/// Brouillon complet (tous les champs renseignés) d'un compte.
pub fn sample_draft(account: AccountId) -> Result<Draft, StoreError> {
    let mut d = Draft::empty(account);
    d.to = vec![Recipient::new(
        "Bob « l'éponge »",
        address("bob@example.com")?,
    )];
    d.cc = vec![Recipient::new("", address("carole@example.com")?)];
    d.bcc = vec![Recipient::new("Caché", address("cache@example.com")?)];
    d.subject = "Re: Réunion'); DROP TABLE drafts; --".into();
    d.text = "Bonjour\n> cité".into();
    d.html = Some("<p>Bonjour</p><blockquote><p>cité</p></blockquote>".into());
    d.threading = Threading {
        in_reply_to: Some("m3@example.com".into()),
        references: vec!["m1@example.com".into(), "m3@example.com".into()],
    };
    d.attachments = vec![DraftAttachment {
        blob: BlobId::parse(&"ab".repeat(32)).ok_or_else(|| fail("blob"))?,
        file_name: "notes été.pdf".into(),
        content_type: "application/pdf".into(),
        size: 1234,
    }];
    Ok(d)
}

fn copy(validity: u32, uid: u32) -> Result<RemoteCopy, StoreError> {
    Ok(RemoteCopy {
        validity: UidValidity::new(validity).ok_or_else(|| fail("validité"))?,
        uid: Uid::new(uid).ok_or_else(|| fail("uid"))?,
    })
}

/// Contrat de [`DraftStore`] : `account` et `other` existent.
pub async fn draft_store_contract<S: DraftStore>(
    store: &S,
    account: AccountId,
    other: AccountId,
) -> Result<(), StoreError> {
    // --- Création, relecture à l'identique ---
    let draft = sample_draft(account)?;
    let first = store.save_draft(None, &draft, 100).await?;
    assert_eq!(first.draft, draft, "relu à l'identique");
    assert_eq!(first.sync.revision, 1);
    assert_eq!(first.updated_at, 100);
    assert_eq!(store.draft(first.id).await?, Some(first.clone()));

    // --- Enregistrement suivant : révision incrémentée ---
    let mut edited = draft.clone();
    edited.subject = "Autre sujet".into();
    edited.html = None;
    edited.attachments.clear();
    let second = store.save_draft(Some(first.id), &edited, 200).await?;
    assert_eq!(second.id, first.id);
    assert_eq!(second.sync.revision, 2);
    assert_eq!(second.draft, edited);

    // --- Isolation, liste du plus récent au plus ancien ---
    let empty = Draft::empty(account);
    let newer = store.save_draft(None, &empty, 300).await?;
    let ids: Vec<DraftId> = store.drafts(account).await?.iter().map(|d| d.id).collect();
    assert_eq!(ids, [newer.id, first.id]);
    assert!(store.drafts(other).await?.is_empty());
    let mut foreign = draft.clone();
    foreign.account = other;
    assert!(
        store
            .save_draft(Some(first.id), &foreign, 400)
            .await
            .is_err(),
        "brouillon d'un autre compte"
    );
    let ghost = DraftId::new(999_999).ok_or_else(|| fail("id"))?;
    assert!(store.save_draft(Some(ghost), &draft, 400).await.is_err());
    assert_eq!(store.draft(ghost).await?, None);

    // --- Copie distante : seul l'état distant est écrit ---
    let (synced, old) = second.sync.remote_saved(2, Some(copy(7, 10)?), 250);
    assert_eq!(old, None);
    assert!(store.set_remote(account, first.id, synced, None).await?);
    // Enregistrement concurrent pendant la copie suivante.
    let third = store.save_draft(Some(first.id), &edited, 260).await?;
    assert_eq!(third.sync.revision, 3);
    let (synced, old) = third.sync.remote_saved(2, Some(copy(7, 11)?), 270);
    assert_eq!(old, Some(copy(7, 10)?));
    let stale = DraftSync {
        revision: 1,
        ..synced
    };
    assert!(store.set_remote(account, first.id, stale, old).await?);
    let reread = store
        .draft(first.id)
        .await?
        .ok_or_else(|| fail("brouillon"))?;
    assert_eq!(reread.sync.revision, 3, "révision locale jamais réécrite");
    assert_eq!(reread.sync.remote_revision, Some(2));
    assert_eq!(reread.sync.remote, Some(copy(7, 11)?));
    assert_eq!(reread.sync.last_remote_save, Some(270));
    assert_eq!(store.remote_trash(account).await?, [copy(7, 10)?]);

    // --- Suppression : la copie distante rejoint la corbeille ---
    let removed = store.delete_draft(first.id).await?;
    assert_eq!(removed.map(|d| d.id), Some(first.id));
    assert_eq!(store.delete_draft(first.id).await?, None);
    assert_eq!(store.draft(first.id).await?, None);
    let mut trash = store.remote_trash(account).await?;
    trash.sort_by_key(|c| c.uid);
    assert_eq!(trash, [copy(7, 10)?, copy(7, 11)?]);
    assert!(store.remote_trash(other).await?.is_empty());
    // Copie faite pour un brouillon supprimé entre-temps : à supprimer.
    let late = DraftSync {
        remote: Some(copy(7, 12)?),
        ..synced
    };
    assert!(
        !store.set_remote(account, first.id, late, None).await?,
        "brouillon disparu"
    );
    assert!(!store.set_remote(account, first.id, synced, None).await?);
    let mut trash = store.remote_trash(account).await?;
    trash.sort_by_key(|c| c.uid);
    assert_eq!(
        trash,
        [copy(7, 10)?, copy(7, 11)?, copy(7, 12)?],
        "ensemble, sans doublon"
    );
    store.forget_remote(other, copy(7, 10)?).await?;
    assert_eq!(store.remote_trash(account).await?.len(), 3, "autre compte");
    for uid in [10, 11, 12] {
        store.forget_remote(account, copy(7, uid)?).await?;
    }
    assert!(store.remote_trash(account).await?.is_empty());
    store.delete_draft(newer.id).await?;
    assert!(store.drafts(account).await?.is_empty());
    Ok(())
}

fn message(n: u32) -> OutgoingMessage {
    OutgoingMessage {
        from: "moi@example.com".into(),
        recipients: vec![format!("dest{n}@example.com"), "cache@example.com".into()],
        raw: format!("Subject: {n}\r\n\r\n\0corps\r\n").into_bytes(),
    }
}

/// Contrat de [`OutboxStore`] (et de son lien avec [`DraftStore`]).
pub async fn outbox_store_contract<S: OutboxStore + DraftStore>(
    store: &S,
    account: AccountId,
    other: AccountId,
) -> Result<(), StoreError> {
    // --- Mise en file : le brouillon disparaît dans la même transaction ---
    let draft = store.save_draft(None, &sample_draft(account)?, 10).await?;
    let (synced, _) = draft.sync.remote_saved(1, Some(copy(3, 5)?), 11);
    store.set_remote(account, draft.id, synced, None).await?;
    let first = store
        .enqueue(
            account,
            &message(1),
            b"copie 1",
            "Sujet 1",
            Some(draft.id),
            100,
        )
        .await?;
    assert_eq!(store.draft(draft.id).await?, None, "brouillon retiré");
    assert_eq!(store.remote_trash(account).await?, [copy(3, 5)?]);
    store.forget_remote(account, copy(3, 5)?).await?;
    let second = store
        .enqueue(account, &message(2), b"copie 2", "Sujet 2", None, 200)
        .await?;
    store
        .enqueue(other, &message(3), b"copie 3", "Sujet 3", None, 300)
        .await?;

    let items = store.outbox(account).await?;
    let ids: Vec<OutboxId> = items.iter().map(|i| i.id).collect();
    assert_eq!(ids, [first, second], "ordre de mise en file");
    let item: &OutboxItem = items.first().ok_or_else(|| fail("message"))?;
    assert_eq!(item.account, account);
    assert_eq!(
        item.message,
        message(1),
        "relu à l'identique (octet nul compris)"
    );
    assert_eq!(item.sent_copy, b"copie 1");
    assert_eq!(item.subject, "Sujet 1");
    assert_eq!(item.entry, OutboxEntry::queued(100));
    assert_eq!(item.queued_at, 100);
    assert_eq!(store.outbox(other).await?.len(), 1);

    // --- Transitions conditionnelles ---
    let queued = OutboxEntry::queued(100);
    let sending = queued.start().map_err(|e| fail(&e.to_string()))?;
    assert!(store.transition(first, queued, sending).await?);
    assert!(
        !store.transition(first, queued, sending).await?,
        "déjà réservé : refusé"
    );
    let failed = sending
        .failed(SendFailure::Temporary, 150)
        .map_err(|e| fail(&e.to_string()))?;
    assert!(store.transition(first, sending, failed).await?);
    let reread = store.outbox(account).await?;
    assert_eq!(reread.first().map(|i| i.entry), Some(failed));
    assert!(matches!(failed.state, OutboxState::Queued { not_before } if not_before > 150));
    // Tous les états sont relus à l'identique.
    for state in [
        OutboxState::Sent,
        OutboxState::Failed(SendFailure::Uncertain),
        OutboxState::Failed(SendFailure::Security),
        OutboxState::Failed(SendFailure::TooManyAttempts),
        OutboxState::Sending,
    ] {
        let current = store
            .outbox(account)
            .await?
            .first()
            .map(|i| i.entry)
            .ok_or_else(|| fail("message"))?;
        let next = OutboxEntry {
            state,
            attempts: current.attempts + 1,
        };
        assert!(store.transition(first, current, next).await?);
        assert_eq!(
            store.outbox(account).await?.first().map(|i| i.entry),
            Some(next)
        );
    }
    let ghost = OutboxId::new(999_999).ok_or_else(|| fail("id"))?;
    assert!(!store.transition(ghost, queued, sending).await?);

    // --- Retrait ---
    assert!(store.remove(first).await?);
    assert!(!store.remove(first).await?);
    let ids: Vec<OutboxId> = store.outbox(account).await?.iter().map(|i| i.id).collect();
    assert_eq!(ids, [second]);
    assert!(store.remove(second).await?);
    Ok(())
}

/// Faux en mémoire des brouillons et de la file d'envoi.
#[derive(Debug, Default)]
pub struct InMemoryComposeStore {
    state: Mutex<ComposeState>,
}

#[derive(Debug, Default)]
struct ComposeState {
    next: i64,
    drafts: BTreeMap<DraftId, StoredDraft>,
    trash: Vec<(AccountId, RemoteCopy)>,
    outbox: BTreeMap<OutboxId, OutboxItem>,
}

fn lock(m: &Mutex<ComposeState>) -> MutexGuard<'_, ComposeState> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ComposeState {
    fn next_id(&mut self) -> i64 {
        self.next += 1;
        self.next
    }

    fn delete_draft(&mut self, id: DraftId) -> Option<StoredDraft> {
        let removed = self.drafts.remove(&id)?;
        if let Some(copy) = removed.sync.remote {
            self.trash_insert(removed.draft.account, copy);
        }
        Some(removed)
    }

    fn trash_insert(&mut self, account: AccountId, copy: RemoteCopy) {
        if !self.trash.contains(&(account, copy)) {
            self.trash.push((account, copy));
        }
    }
}

impl DraftStore for InMemoryComposeStore {
    async fn save_draft(
        &self,
        id: Option<DraftId>,
        draft: &Draft,
        now: i64,
    ) -> Result<StoredDraft, StoreError> {
        let mut s = lock(&self.state);
        let (id, sync) = match id {
            None => {
                let raw = s.next_id();
                (
                    DraftId::new(raw).ok_or_else(|| fail("identifiant"))?,
                    DraftSync::default(),
                )
            }
            Some(id) => {
                let existing = s.drafts.get(&id).ok_or_else(|| fail("brouillon inconnu"))?;
                if existing.draft.account != draft.account {
                    return Err(fail("brouillon d'un autre compte"));
                }
                (id, existing.sync)
            }
        };
        let stored = StoredDraft {
            id,
            draft: draft.clone(),
            sync: sync.edited(),
            updated_at: now,
        };
        s.drafts.insert(id, stored.clone());
        Ok(stored)
    }

    async fn draft(&self, id: DraftId) -> Result<Option<StoredDraft>, StoreError> {
        Ok(lock(&self.state).drafts.get(&id).cloned())
    }

    async fn drafts(&self, account: AccountId) -> Result<Vec<StoredDraft>, StoreError> {
        let mut list: Vec<StoredDraft> = lock(&self.state)
            .drafts
            .values()
            .filter(|d| d.draft.account == account)
            .cloned()
            .collect();
        list.sort_by_key(|d| std::cmp::Reverse((d.updated_at, d.id)));
        Ok(list)
    }

    async fn set_remote(
        &self,
        account: AccountId,
        id: DraftId,
        sync: DraftSync,
        replaced: Option<RemoteCopy>,
    ) -> Result<bool, StoreError> {
        let mut s = lock(&self.state);
        let exists = s.drafts.get(&id).is_some_and(|d| d.draft.account == account);
        if !exists {
            // Brouillon disparu : sa nouvelle copie est à supprimer.
            for copy in sync.remote.into_iter().chain(replaced) {
                s.trash_insert(account, copy);
            }
            return Ok(false);
        }
        if let Some(stored) = s.drafts.get_mut(&id) {
            stored.sync.remote_revision = sync.remote_revision;
            stored.sync.remote = sync.remote;
            stored.sync.last_remote_save = sync.last_remote_save;
        }
        if let Some(old) = replaced {
            s.trash_insert(account, old);
        }
        Ok(true)
    }

    async fn delete_draft(&self, id: DraftId) -> Result<Option<StoredDraft>, StoreError> {
        Ok(lock(&self.state).delete_draft(id))
    }

    async fn remote_trash(&self, account: AccountId) -> Result<Vec<RemoteCopy>, StoreError> {
        Ok(lock(&self.state)
            .trash
            .iter()
            .filter(|(a, _)| *a == account)
            .map(|(_, c)| *c)
            .collect())
    }

    async fn forget_remote(&self, account: AccountId, copy: RemoteCopy) -> Result<(), StoreError> {
        lock(&self.state)
            .trash
            .retain(|(a, c)| !(*a == account && *c == copy));
        Ok(())
    }
}

impl OutboxStore for InMemoryComposeStore {
    async fn enqueue(
        &self,
        account: AccountId,
        message: &OutgoingMessage,
        sent_copy: &[u8],
        subject: &str,
        draft: Option<DraftId>,
        now: i64,
    ) -> Result<OutboxId, StoreError> {
        let mut s = lock(&self.state);
        if let Some(draft) = draft {
            s.delete_draft(draft);
        }
        let id = OutboxId::new(s.next_id()).ok_or_else(|| fail("identifiant"))?;
        s.outbox.insert(
            id,
            OutboxItem {
                id,
                account,
                message: message.clone(),
                sent_copy: sent_copy.to_vec(),
                subject: subject.to_owned(),
                entry: OutboxEntry::queued(now),
                queued_at: now,
            },
        );
        Ok(id)
    }

    async fn outbox(&self, account: AccountId) -> Result<Vec<OutboxItem>, StoreError> {
        Ok(lock(&self.state)
            .outbox
            .values()
            .filter(|i| i.account == account)
            .cloned()
            .collect())
    }

    async fn transition(
        &self,
        id: OutboxId,
        from: OutboxEntry,
        to: OutboxEntry,
    ) -> Result<bool, StoreError> {
        let mut s = lock(&self.state);
        match s.outbox.get_mut(&id) {
            Some(item) if item.entry == from => {
                item.entry = to;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    async fn remove(&self, id: OutboxId) -> Result<bool, StoreError> {
        Ok(lock(&self.state).outbox.remove(&id).is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accounts() -> Result<(AccountId, AccountId), StoreError> {
        Ok((
            AccountId::new(1).map_err(|e| StoreError(e.to_string()))?,
            AccountId::new(2).map_err(|e| StoreError(e.to_string()))?,
        ))
    }

    #[tokio::test]
    async fn in_memory_store_passes_draft_contract() -> Result<(), StoreError> {
        let (a, b) = accounts()?;
        draft_store_contract(&InMemoryComposeStore::default(), a, b).await
    }

    #[tokio::test]
    async fn in_memory_store_passes_outbox_contract() -> Result<(), StoreError> {
        let (a, b) = accounts()?;
        outbox_store_contract(&InMemoryComposeStore::default(), a, b).await
    }
}
