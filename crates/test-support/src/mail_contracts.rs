//! Suites de contrat du stockage des messages ([`BlobStore`],
//! [`MailStore`], [`FolderTreeQuery`], [`MessageListQuery`]) et faux en
//! mémoire. Les implémentations réelles (fichiers, SQLite) passent les
//! mêmes suites.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, MutexGuard, PoisonError};

use norelly_domain::account::AccountId;
use norelly_domain::fetch_policy::PartPath;
use norelly_domain::mailbox::{
    Correspondent, Flags, MailboxId, MailboxName, MailboxRole, MessageHeaders, MessageMeta, ModSeq,
    RemoteMailbox, Uid, UidValidity,
};
use norelly_domain::ports::{
    BlobId, BlobStore, FolderNode, FolderTreeQuery, MAX_PAGE_SIZE, MailStore, MessageId,
    MessageListItem, MessageListQuery, MessagePage, PageCursor, PendingBody, StoreError,
    StoredBody, StoredMailbox, StoredMessage, SyncBatch,
};
use norelly_domain::sync::{FlagUpdate, LocalMailbox, MailboxSyncState};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Empreinte SHA-256 (hexadécimal minuscule) d'un contenu.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn blob_id(bytes: &[u8]) -> Result<BlobId, StoreError> {
    BlobId::parse(&sha256_hex(bytes)).ok_or_else(|| StoreError("empreinte".into()))
}

// ---------------------------------------------------------------------------
// BlobStore
// ---------------------------------------------------------------------------

/// Contrat de [`BlobStore`].
pub async fn blob_store_contract<S: BlobStore>(store: &S) -> Result<(), StoreError> {
    let a = AccountId::new(1).map_err(|e| StoreError(e.to_string()))?;
    let b = AccountId::new(2).map_err(|e| StoreError(e.to_string()))?;
    let content = b"From: a@example.com\r\nSubject: x\r\n\r\ncorps\r\n".to_vec();

    let id = store.put(a, content.clone()).await?;
    assert_eq!(
        id.as_str(),
        sha256_hex(&content),
        "identifiant = empreinte du contenu"
    );
    assert_eq!(store.get(a, &id).await?, Some(content.clone()));
    // Idempotent.
    assert_eq!(store.put(a, content.clone()).await?, id);
    // Isolation entre comptes.
    assert_eq!(store.get(b, &id).await?, None, "fuite entre comptes");

    // Contenu vide et binaire.
    let empty = store.put(a, Vec::new()).await?;
    assert_eq!(store.get(a, &empty).await?, Some(Vec::new()));
    let binary: Vec<u8> = (0..=255u8).cycle().take(100_000).collect();
    let bin = store.put(a, binary.clone()).await?;
    assert_eq!(store.get(a, &bin).await?, Some(binary));

    // Absent.
    let unknown = blob_id(b"jamais stocke")?;
    assert_eq!(store.get(a, &unknown).await?, None);

    // Suppression idempotente.
    store.delete(a, &id).await?;
    assert_eq!(store.get(a, &id).await?, None);
    store.delete(a, &id).await?;
    store.delete(a, &empty).await?;
    store.delete(a, &bin).await?;
    Ok(())
}

/// Faux [`BlobStore`] en mémoire.
#[derive(Debug, Default)]
pub struct InMemoryBlobStore {
    blobs: Mutex<BTreeMap<(AccountId, BlobId), Vec<u8>>>,
}

impl InMemoryBlobStore {
    /// Nombre de contenus stockés.
    #[must_use]
    pub fn len(&self) -> usize {
        lock(&self.blobs).len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl BlobStore for InMemoryBlobStore {
    async fn put(&self, account: AccountId, bytes: Vec<u8>) -> Result<BlobId, StoreError> {
        let id = blob_id(&bytes)?;
        lock(&self.blobs).insert((account, id.clone()), bytes);
        Ok(id)
    }

    async fn get(&self, account: AccountId, id: &BlobId) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(lock(&self.blobs).get(&(account, id.clone())).cloned())
    }

    async fn delete(&self, account: AccountId, id: &BlobId) -> Result<(), StoreError> {
        lock(&self.blobs).remove(&(account, id.clone()));
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// MailStore et requêtes
// ---------------------------------------------------------------------------

fn uid(n: u32) -> Uid {
    Uid::new(n).unwrap_or_else(|| panic!("uid {n}"))
}

fn mailbox(name: &str, role: MailboxRole, selectable: bool) -> RemoteMailbox {
    RemoteMailbox {
        name: MailboxName::parse(name).unwrap_or_else(|e| panic!("{name} : {e}")),
        delimiter: Some('/'),
        role,
        selectable,
    }
}

/// Message de test : UID `n`, date `date`, expéditeur et sujet dérivés.
#[must_use]
pub fn sample_message(n: u32, date: i64, seen: bool) -> StoredMessage {
    let flags = Flags::from_tokens(if seen { vec!["\\Seen"] } else { vec![] });
    StoredMessage {
        meta: MessageMeta {
            uid: uid(n),
            flags,
            modseq: ModSeq::new(u64::from(n) + 1),
            internal_date: date,
            size: 1000 + n,
            headers: MessageHeaders {
                subject: format!("Sujet {n}"),
                from: vec![Correspondent::new(
                    &format!("Expéditeur {n}"),
                    &format!("e{n}@example.com"),
                )],
                date: Some(date),
                message_id: Some(format!("m{n}@example.com")),
                ..MessageHeaders::default()
            },
            structure: None,
        },
        body: StoredBody::Missing,
    }
}

fn state(validity: u32, modseq: u64, next: u32) -> MailboxSyncState {
    MailboxSyncState {
        uid_validity: UidValidity::new(validity),
        highest_modseq: ModSeq::new(modseq),
        uid_next: Uid::new(next),
    }
}

/// Contrat de [`MailStore`], [`FolderTreeQuery`] et [`MessageListQuery`]
/// pour le compte `account` (qui doit exister et n'avoir aucune boîte) ;
/// `other` : un second compte, pour vérifier l'isolation.
pub async fn mail_store_contract<S>(
    store: &S,
    account: AccountId,
    other: AccountId,
) -> Result<(), StoreError>
where
    S: MailStore + FolderTreeQuery + MessageListQuery,
{
    // --- Boîtes ---
    let first = store
        .replace_mailboxes(
            account,
            &[
                mailbox("INBOX", MailboxRole::Inbox, true),
                mailbox("Archives", MailboxRole::Archive, true),
                mailbox("Sent", MailboxRole::Sent, true),
                mailbox("[Gmail]", MailboxRole::Normal, false),
                mailbox("Ancien", MailboxRole::Normal, true),
            ],
        )
        .await?;
    assert_eq!(
        first
            .iter()
            .map(|m| m.mailbox.name.as_str())
            .collect::<Vec<_>>(),
        ["INBOX", "Archives", "Sent", "[Gmail]", "Ancien"]
    );
    let ids: BTreeSet<MailboxId> = first.iter().map(|m| m.id).collect();
    assert_eq!(ids.len(), 5, "identifiants distincts");
    let id_of = |list: &[StoredMailbox], name: &str| {
        list.iter()
            .find(|m| m.mailbox.name.as_str() == name)
            .map(|m| m.id)
            .unwrap_or_else(|| panic!("boîte {name} absente"))
    };
    let inbox = id_of(&first, "INBOX");
    let sent = id_of(&first, "Sent");
    let ancien = id_of(&first, "Ancien");

    // Second compte : boîte homonyme distincte.
    let other_boxes = store
        .replace_mailboxes(other, &[mailbox("INBOX", MailboxRole::Inbox, true)])
        .await?;
    let other_inbox = id_of(&other_boxes, "INBOX");
    assert_ne!(other_inbox, inbox);

    // État initial vide.
    assert_eq!(store.local_mailbox(inbox).await?, LocalMailbox::default());

    // --- Lot initial ---
    let batch = SyncBatch {
        reset: false,
        removed: vec![],
        flag_changes: vec![],
        new: (1..=3)
            .map(|n| sample_message(n, 1_000 + i64::from(n), n == 1))
            .collect(),
        state: state(7, 10, 4),
    };
    store.apply_sync(inbox, &batch).await?;
    let local = store.local_mailbox(inbox).await?;
    assert_eq!(local.state, state(7, 10, 4));
    assert_eq!(
        local.messages.keys().map(|u| u.get()).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert!(local.messages.get(&uid(1)).is_some_and(|f| f.seen));
    // Rejouer le même lot ne duplique rien.
    store.apply_sync(inbox, &batch).await?;
    assert_eq!(store.local_mailbox(inbox).await?.messages.len(), 3);
    // Isolation.
    assert!(store.local_mailbox(other_inbox).await?.messages.is_empty());
    assert!(store.local_mailbox(sent).await?.messages.is_empty());

    // --- Drapeaux et suppressions ---
    let mut flagged = Flags::from_tokens(["\\Seen", "\\Flagged", "$Travail"]);
    flagged.answered = true;
    store
        .apply_sync(
            inbox,
            &SyncBatch {
                reset: false,
                removed: vec![uid(2), uid(99)],
                flag_changes: vec![
                    FlagUpdate {
                        uid: uid(3),
                        flags: flagged.clone(),
                        modseq: ModSeq::new(20),
                    },
                    // Message inconnu : ignoré.
                    FlagUpdate {
                        uid: uid(50),
                        flags: Flags::default(),
                        modseq: None,
                    },
                ],
                new: vec![],
                state: state(7, 20, 4),
            },
        )
        .await?;
    let local = store.local_mailbox(inbox).await?;
    assert_eq!(
        local.messages.keys().map(|u| u.get()).collect::<Vec<_>>(),
        [1, 3]
    );
    assert_eq!(
        local.messages.get(&uid(3)),
        Some(&flagged),
        "drapeaux et mots-clés conservés"
    );
    assert_eq!(local.state.highest_modseq, ModSeq::new(20));

    // --- Corps ---
    assert_eq!(
        store
            .missing_bodies(inbox, 10)
            .await?
            .iter()
            .map(|p| p.uid.get())
            .collect::<Vec<_>>(),
        [3, 1],
        "UID décroissants"
    );
    let full = blob_id(b"un")?;
    store
        .set_body(inbox, uid(3), &StoredBody::Full(full))
        .await?;
    let partial = StoredBody::Partial {
        header: blob_id(b"en-tete")?,
        parts: vec![(
            PartPath::parse("1").unwrap_or_else(|| panic!("chemin")),
            blob_id(b"texte")?,
        )],
        deferred: vec![PartPath::parse("2").unwrap_or_else(|| panic!("chemin"))],
    };
    store.set_body(inbox, uid(1), &partial).await?;
    store.set_body(inbox, uid(42), &partial).await?; // disparu : sans effet
    assert!(store.missing_bodies(inbox, 10).await?.is_empty());
    let pending: Vec<PendingBody> = store.missing_bodies(inbox, 0).await?;
    assert!(pending.is_empty());

    // --- Arborescence ---
    let tree = store.tree(account).await?;
    let summary: Vec<(&str, MailboxRole, bool, u32, u32)> = tree
        .iter()
        .map(|n| (n.name.as_str(), n.role, n.selectable, n.total, n.unread))
        .collect();
    assert_eq!(
        summary,
        [
            ("INBOX", MailboxRole::Inbox, true, 2, 0),
            ("Sent", MailboxRole::Sent, true, 0, 0),
            ("Archives", MailboxRole::Archive, true, 0, 0),
            ("Ancien", MailboxRole::Normal, true, 0, 0),
            ("[Gmail]", MailboxRole::Normal, false, 0, 0),
        ],
        "rôles d'abord, puis par nom"
    );
    assert_eq!(store.tree(other).await?.len(), 1);

    // --- Pagination ---
    // 25 messages dans « Ancien », dont des dates identiques.
    let many: Vec<StoredMessage> = (1..=25)
        .map(|n| sample_message(n, 5_000 + i64::from(n / 2), n % 3 == 0))
        .collect();
    store
        .apply_sync(
            ancien,
            &SyncBatch {
                new: many,
                state: state(1, 1, 26),
                ..SyncBatch::default()
            },
        )
        .await?;
    let mut seen: Vec<MessageListItem> = Vec::new();
    let mut cursor: Option<PageCursor> = None;
    let mut pages = 0;
    loop {
        let page: MessagePage = store.page(ancien, cursor, 10).await?;
        assert!(page.items.len() <= 10);
        pages += 1;
        seen.extend(page.items);
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
        assert!(pages < 10, "pagination sans fin");
    }
    assert_eq!(pages, 3);
    assert_eq!(seen.len(), 25, "chaque message une fois");
    let uids: BTreeSet<u32> = seen.iter().map(|i| i.uid.get()).collect();
    assert_eq!(uids.len(), 25);
    for pair in seen.windows(2) {
        if let [a, b] = pair {
            assert!(
                (a.date, a.id) > (b.date, b.id),
                "ordre décroissant (date, id)"
            );
        }
    }
    let top = seen.first().unwrap_or_else(|| panic!("liste vide"));
    assert_eq!(top.subject, "Sujet 25");
    assert_eq!(
        top.from.as_ref().map(|c| c.address.as_str()),
        Some("e25@example.com")
    );
    assert_eq!(top.date, 5_012);
    assert_eq!(top.size, 1025);
    // Taille de page bornée.
    let big = store.page(ancien, None, 100_000).await?;
    assert!(big.items.len() <= MAX_PAGE_SIZE as usize);
    assert!(store.page(ancien, None, 0).await?.items.is_empty());

    // Une boîte qui disparaît emporte ses messages ; les autres gardent
    // leur identifiant.
    let second = store
        .replace_mailboxes(
            account,
            &[
                mailbox("INBOX", MailboxRole::Inbox, true),
                mailbox("Sent", MailboxRole::Normal, true),
                mailbox("Nouveau", MailboxRole::Normal, true),
            ],
        )
        .await?;
    assert_eq!(id_of(&second, "INBOX"), inbox);
    assert_eq!(id_of(&second, "Sent"), sent);
    assert_eq!(
        second.iter().find(|m| m.id == sent).map(|m| m.mailbox.role),
        Some(MailboxRole::Normal),
        "rôle mis à jour"
    );
    assert!(store.page(ancien, None, 10).await?.items.is_empty());
    assert!(store.local_mailbox(ancien).await?.messages.is_empty());
    let names: Vec<String> = store
        .tree(account)
        .await?
        .into_iter()
        .map(|n: FolderNode| n.name.to_string())
        .collect();
    assert_eq!(names, ["INBOX", "Nouveau", "Sent"]);

    // --- Reset (UIDVALIDITY changée) ---
    store
        .apply_sync(
            inbox,
            &SyncBatch {
                reset: true,
                new: vec![sample_message(1, 9_000, false)],
                state: state(8, 2, 2),
                ..SyncBatch::default()
            },
        )
        .await?;
    let local = store.local_mailbox(inbox).await?;
    assert_eq!(
        local.messages.keys().map(|u| u.get()).collect::<Vec<_>>(),
        [1]
    );
    assert_eq!(local.state.uid_validity, UidValidity::new(8));
    let page = store.page(inbox, None, 10).await?;
    assert_eq!(
        page.items.first().map(|i| i.date),
        Some(9_000),
        "ancien message 1 remplacé"
    );
    Ok(())
}

#[derive(Debug, Clone)]
struct MemMessage {
    id: MessageId,
    stored: StoredMessage,
}

#[derive(Debug, Default)]
struct MemState {
    next_mailbox: i64,
    next_message: i64,
    mailboxes: Vec<(AccountId, StoredMailbox, MailboxSyncState)>,
    messages: BTreeMap<MailboxId, BTreeMap<Uid, MemMessage>>,
}

/// Faux [`MailStore`] et ports de requête en mémoire.
#[derive(Debug, Default)]
pub struct InMemoryMailStore {
    state: Mutex<MemState>,
}

impl MailStore for InMemoryMailStore {
    async fn replace_mailboxes(
        &self,
        account: AccountId,
        mailboxes: &[RemoteMailbox],
    ) -> Result<Vec<StoredMailbox>, StoreError> {
        let mut s = lock(&self.state);
        let wanted: BTreeSet<&MailboxName> = mailboxes.iter().map(|m| &m.name).collect();
        let removed: Vec<MailboxId> = s
            .mailboxes
            .iter()
            .filter(|(a, m, _)| *a == account && !wanted.contains(&m.mailbox.name))
            .map(|(_, m, _)| m.id)
            .collect();
        s.mailboxes.retain(|(_, m, _)| !removed.contains(&m.id));
        for id in removed {
            s.messages.remove(&id);
        }
        let mut out = Vec::new();
        for remote in mailboxes {
            let existing = s
                .mailboxes
                .iter_mut()
                .find(|(a, m, _)| *a == account && m.mailbox.name == remote.name);
            let stored = if let Some((_, m, _)) = existing {
                m.mailbox = remote.clone();
                m.clone()
            } else {
                s.next_mailbox += 1;
                let id = MailboxId::new(s.next_mailbox)
                    .ok_or_else(|| StoreError("identifiant".into()))?;
                let m = StoredMailbox {
                    id,
                    mailbox: remote.clone(),
                };
                s.mailboxes
                    .push((account, m.clone(), MailboxSyncState::default()));
                m
            };
            out.push(stored);
        }
        Ok(out)
    }

    async fn local_mailbox(&self, mailbox: MailboxId) -> Result<LocalMailbox, StoreError> {
        let s = lock(&self.state);
        let state = s
            .mailboxes
            .iter()
            .find(|(_, m, _)| m.id == mailbox)
            .map(|(_, _, st)| *st)
            .unwrap_or_default();
        let messages = s
            .messages
            .get(&mailbox)
            .map(|msgs| {
                msgs.iter()
                    .map(|(u, m)| (*u, m.stored.meta.flags.clone()))
                    .collect()
            })
            .unwrap_or_default();
        Ok(LocalMailbox { state, messages })
    }

    async fn apply_sync(&self, mailbox: MailboxId, batch: &SyncBatch) -> Result<(), StoreError> {
        let mut s = lock(&self.state);
        let Some(entry) = s.mailboxes.iter_mut().find(|(_, m, _)| m.id == mailbox) else {
            return Err(StoreError(format!("boîte {mailbox} inconnue")));
        };
        entry.2 = batch.state;
        let mut next = s.next_message;
        let msgs = s.messages.entry(mailbox).or_default();
        if batch.reset {
            msgs.clear();
        }
        for u in &batch.removed {
            msgs.remove(u);
        }
        for change in &batch.flag_changes {
            if let Some(m) = msgs.get_mut(&change.uid) {
                m.stored.meta.flags = change.flags.clone();
                m.stored.meta.modseq = change.modseq;
            }
        }
        for new in &batch.new {
            next += 1;
            let id = MessageId::new(next).ok_or_else(|| StoreError("identifiant".into()))?;
            msgs.insert(
                new.meta.uid,
                MemMessage {
                    id,
                    stored: new.clone(),
                },
            );
        }
        s.next_message = next;
        Ok(())
    }

    async fn missing_bodies(
        &self,
        mailbox: MailboxId,
        limit: usize,
    ) -> Result<Vec<PendingBody>, StoreError> {
        let s = lock(&self.state);
        Ok(s.messages
            .get(&mailbox)
            .map(|msgs| {
                msgs.values()
                    .rev()
                    .filter(|m| m.stored.body == StoredBody::Missing)
                    .take(limit)
                    .map(|m| PendingBody {
                        uid: m.stored.meta.uid,
                        size: m.stored.meta.size,
                        structure: m.stored.meta.structure.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn set_body(
        &self,
        mailbox: MailboxId,
        target: Uid,
        body: &StoredBody,
    ) -> Result<(), StoreError> {
        let mut s = lock(&self.state);
        if let Some(m) = s
            .messages
            .get_mut(&mailbox)
            .and_then(|msgs| msgs.get_mut(&target))
        {
            m.stored.body = body.clone();
        }
        Ok(())
    }
}

impl InMemoryMailStore {
    /// Corps enregistré d'un message (vérifications des tests).
    #[must_use]
    pub fn body_of(&self, mailbox: MailboxId, target: Uid) -> Option<StoredBody> {
        lock(&self.state)
            .messages
            .get(&mailbox)
            .and_then(|msgs| msgs.get(&target))
            .map(|m| m.stored.body.clone())
    }
}

impl FolderTreeQuery for InMemoryMailStore {
    async fn tree(&self, account: AccountId) -> Result<Vec<FolderNode>, StoreError> {
        let s = lock(&self.state);
        let mut out: Vec<FolderNode> = s
            .mailboxes
            .iter()
            .filter(|(a, _, _)| *a == account)
            .map(|(_, m, _)| {
                let msgs = s.messages.get(&m.id);
                let count = |f: &dyn Fn(&MemMessage) -> bool| {
                    msgs.map_or(0, |msgs| {
                        u32::try_from(msgs.values().filter(|m| f(m)).count()).unwrap_or(u32::MAX)
                    })
                };
                FolderNode {
                    id: m.id,
                    name: m.mailbox.name.clone(),
                    delimiter: m.mailbox.delimiter,
                    role: m.mailbox.role,
                    selectable: m.mailbox.selectable,
                    total: count(&|_| true),
                    unread: count(&|m| !m.stored.meta.flags.seen),
                }
            })
            .collect();
        out.sort_by(|a, b| (a.role.rank(), a.name.as_str()).cmp(&(b.role.rank(), b.name.as_str())));
        Ok(out)
    }
}

impl MessageListQuery for InMemoryMailStore {
    async fn page(
        &self,
        mailbox: MailboxId,
        cursor: Option<PageCursor>,
        limit: u32,
    ) -> Result<MessagePage, StoreError> {
        let limit = limit.min(MAX_PAGE_SIZE) as usize;
        let s = lock(&self.state);
        let mut items: Vec<MessageListItem> = s
            .messages
            .get(&mailbox)
            .map(|msgs| {
                msgs.values()
                    .map(|m| MessageListItem {
                        id: m.id,
                        uid: m.stored.meta.uid,
                        date: m.stored.meta.sort_date(),
                        from: m.stored.meta.headers.from.first().cloned(),
                        subject: m.stored.meta.headers.subject.clone(),
                        flags: m.stored.meta.flags.clone(),
                        has_attachments: m.stored.meta.has_attachments(),
                        size: m.stored.meta.size,
                    })
                    .collect()
            })
            .unwrap_or_default();
        items.sort_by_key(|i| std::cmp::Reverse((i.date, i.id)));
        if let Some(c) = cursor {
            items.retain(|i| (i.date, i.id) < (c.date, c.id));
        }
        let more = items.len() > limit;
        items.truncate(limit);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_blob_store_passes_contract() -> Result<(), StoreError> {
        blob_store_contract(&InMemoryBlobStore::default()).await
    }

    #[tokio::test]
    async fn in_memory_mail_store_passes_contract() -> Result<(), StoreError> {
        let (a, b) = (
            AccountId::new(1).map_err(|e| StoreError(e.to_string()))?,
            AccountId::new(2).map_err(|e| StoreError(e.to_string()))?,
        );
        mail_store_contract(&InMemoryMailStore::default(), a, b).await
    }
}
