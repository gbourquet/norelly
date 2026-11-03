//! Suites de contrat du stockage des messages ([`BlobStore`],
//! [`MailStore`], [`FolderTreeQuery`], [`MessageListQuery`]) et faux en
//! mémoire. Les implémentations réelles (fichiers, SQLite) passent les
//! mêmes suites.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, MutexGuard, PoisonError};

use norelly_domain::account::AccountId;
use norelly_domain::conversation::{ConversationScope, ThreadKeys, ThreadMessage, thread};
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
use norelly_domain::ports::{
    ConversationCursor, ConversationId, ConversationListItem, ConversationMessage,
    ConversationPage, ConversationQuery, MAX_CONVERSATION_MESSAGES, MAX_LISTED_SENDERS,
    MessageLocation, MessageLocator, same_sender,
};
use norelly_domain::ports::{
    MAX_INDEXED_TEXT_CHARS, MAX_SEARCH_RESULTS, MAX_SNIPPET_CHARS, SearchHit, SearchPage,
    SearchRequest, SearchScope,
};
use norelly_domain::search::{Atom, Field, Filter, tokens};
use norelly_domain::sync::{FlagUpdate, LocalMailbox, MailboxSyncState};

use norelly_domain::actions::{
    FolderRef, LocalEffect, LocalMessage, MAX_ACTION_TARGETS, Member, OpKind, PendingOp,
    QueueChange, local_effect, revert_effect,
};
use norelly_domain::ports::{PendingOpId, PendingOpStore};

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

pub(crate) fn uid(n: u32) -> Uid {
    Uid::new(n).unwrap_or_else(|| panic!("uid {n}"))
}

/// Boîte distante de test.
#[must_use]
pub fn mailbox(name: &str, role: MailboxRole, selectable: bool) -> RemoteMailbox {
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

pub(crate) fn state(validity: u32, modseq: u64, next: u32) -> MailboxSyncState {
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
        .set_body(inbox, uid(3), &StoredBody::Full(full), None)
        .await?;
    let partial = StoredBody::Partial {
        header: blob_id(b"en-tete")?,
        parts: vec![(
            PartPath::parse("1").unwrap_or_else(|| panic!("chemin")),
            blob_id(b"texte")?,
        )],
        deferred: vec![PartPath::parse("2").unwrap_or_else(|| panic!("chemin"))],
    };
    store.set_body(inbox, uid(1), &partial, None).await?;
    store.set_body(inbox, uid(42), &partial, None).await?; // disparu : sans effet
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
    /// Masqué par une action en attente (application optimiste).
    hidden: bool,
    /// Copie provisoire créée par ce déplacement (pas encore synchronisée).
    pending_op: Option<PendingOpId>,
    /// Texte du message, pour la recherche (vide tant que le corps n'est
    /// pas téléchargé).
    text: String,
}

impl MemMessage {
    /// Visible dans les listes : ni masqué, ni marqué `\Deleted`.
    fn visible(&self) -> bool {
        !self.hidden && !self.stored.meta.flags.deleted
    }
}

#[derive(Debug, Default)]
struct MemState {
    next_mailbox: i64,
    next_message: i64,
    mailboxes: Vec<(AccountId, StoredMailbox, MailboxSyncState)>,
    messages: BTreeMap<MailboxId, BTreeMap<Uid, MemMessage>>,
    next_op: i64,
    ops: BTreeMap<PendingOpId, PendingOp>,
    /// Essais de rejeu des opérations réservées.
    claimed: BTreeMap<PendingOpId, u32>,
    archive: BTreeMap<AccountId, MailboxId>,
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
                    .filter(|(_, m)| m.pending_op.is_none())
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
                    hidden: false,
                    pending_op: None,
                    text: String::new(),
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
                    .filter(|m| m.stored.body == StoredBody::Missing && m.pending_op.is_none())
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
        text: Option<&str>,
    ) -> Result<(), StoreError> {
        let mut s = lock(&self.state);
        if let Some(m) = s
            .messages
            .get_mut(&mailbox)
            .and_then(|msgs| msgs.get_mut(&target))
        {
            m.stored.body = body.clone();
            if let Some(text) = text {
                m.text = text.chars().take(MAX_INDEXED_TEXT_CHARS).collect();
            }
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
                    total: count(&MemMessage::visible),
                    unread: count(&|m| m.visible() && !m.stored.meta.flags.seen),
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
                    .filter(|m| m.visible())
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

// ---------------------------------------------------------------------------
// Conversations (faux : JWZ recalculé à chaque requête)
// ---------------------------------------------------------------------------

/// Message du faux, avec son compte et sa boîte.
struct Located<'a> {
    account: AccountId,
    mailbox: MailboxId,
    role: MailboxRole,
    message: &'a MemMessage,
}

impl MemState {
    /// Messages visibles (listes).
    fn located(&self) -> Vec<Located<'_>> {
        let mut all = self.located_all();
        all.retain(|l| l.message.visible());
        all
    }

    /// Tous les messages, masqués compris (rattachement aux conversations,
    /// stable quand un message est masqué).
    fn located_all(&self) -> Vec<Located<'_>> {
        let mut out = Vec::new();
        for (account, mailbox, _) in &self.mailboxes {
            if let Some(msgs) = self.messages.get(&mailbox.id) {
                for message in msgs.values() {
                    out.push(Located {
                        account: *account,
                        mailbox: mailbox.id,
                        role: mailbox.mailbox.role,
                        message,
                    });
                }
            }
        }
        out
    }

    /// Conversation de chaque message (identifiant : plus petit
    /// identifiant de message de la conversation), par l'algorithme JWZ
    /// appliqué compte par compte ; les copies d'un même `Message-ID`
    /// suivent la première.
    fn conversations(&self) -> BTreeMap<MessageId, ConversationId> {
        let located = self.located_all();
        let mut out = BTreeMap::new();
        let accounts: BTreeSet<AccountId> = located.iter().map(|l| l.account).collect();
        for account in accounts {
            let mut first_by_id: BTreeMap<String, MessageId> = BTreeMap::new();
            let mut unique = Vec::new();
            let mut copies = Vec::new();
            let mut mine: Vec<&Located<'_>> =
                located.iter().filter(|l| l.account == account).collect();
            mine.sort_by_key(|l| l.message.id);
            for l in mine {
                let h = &l.message.stored.meta.headers;
                if let Some(mid) = &h.message_id {
                    if let Some(first) = first_by_id.get(mid) {
                        copies.push((l.message.id, *first));
                        continue;
                    }
                    first_by_id.insert(mid.clone(), l.message.id);
                }
                unique.push(ThreadMessage {
                    key: l.message.id,
                    keys: ThreadKeys::of(h),
                    date: l.message.stored.meta.sort_date(),
                });
            }
            for t in thread(&unique) {
                let Some(id) = t.entries.iter().map(|e| e.key).min() else {
                    continue;
                };
                let Some(conv) = ConversationId::new(id.get()) else {
                    continue;
                };
                for e in &t.entries {
                    out.insert(e.key, conv);
                }
            }
            for (copy, first) in copies {
                if let Some(conv) = out.get(&first).copied() {
                    out.insert(copy, conv);
                }
            }
        }
        out
    }
}

impl ConversationQuery for InMemoryMailStore {
    async fn conversations(
        &self,
        scope: ConversationScope,
        cursor: Option<ConversationCursor>,
        limit: u32,
    ) -> Result<ConversationPage, StoreError> {
        let limit = limit.min(MAX_PAGE_SIZE) as usize;
        let s = lock(&self.state);
        let convs = s.conversations();
        let mut groups: BTreeMap<(ConversationId, MailboxId), Vec<&Located<'_>>> = BTreeMap::new();
        let located = s.located();
        for l in &located {
            let in_scope = match scope {
                ConversationScope::Mailbox(m) => l.mailbox == m,
                ConversationScope::Unified(role) => l.role == role,
            };
            if let (true, Some(c)) = (in_scope, convs.get(&l.message.id)) {
                groups.entry((*c, l.mailbox)).or_default().push(l);
            }
        }
        let mut items: Vec<ConversationListItem> = groups
            .into_iter()
            .filter_map(|((id, mailbox), mut members)| {
                members.sort_by_key(|l| {
                    std::cmp::Reverse((l.message.stored.meta.sort_date(), l.message.id))
                });
                let latest = members.first()?;
                let mut senders: Vec<Correspondent> = Vec::new();
                for l in &members {
                    if let Some(p) = l.message.stored.meta.headers.from.first()
                        && senders.len() < MAX_LISTED_SENDERS
                        && !senders.iter().any(|q| same_sender(q, p))
                    {
                        senders.push(p.clone());
                    }
                }
                let count = |f: &dyn Fn(&Located<'_>) -> bool| {
                    u32::try_from(members.iter().filter(|l| f(l)).count()).unwrap_or(u32::MAX)
                };
                Some(ConversationListItem {
                    id,
                    account: latest.account,
                    mailbox,
                    date: latest.message.stored.meta.sort_date(),
                    subject: latest.message.stored.meta.headers.subject.clone(),
                    senders,
                    messages: count(&|_| true),
                    unread: count(&|l| !l.message.stored.meta.flags.seen),
                    flagged: members.iter().any(|l| l.message.stored.meta.flags.flagged),
                    has_attachments: members
                        .iter()
                        .any(|l| l.message.stored.meta.has_attachments()),
                })
            })
            .collect();
        items.sort_by_key(|i| std::cmp::Reverse((i.date, i.id)));
        if let Some(c) = cursor {
            items.retain(|i| (i.date, i.id) < (c.date, c.id));
        }
        let more = items.len() > limit;
        items.truncate(limit);
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

    async fn conversation_messages(
        &self,
        conversation: ConversationId,
    ) -> Result<Vec<ConversationMessage>, StoreError> {
        let s = lock(&self.state);
        let convs = s.conversations();
        let located = s.located();
        let mut members: Vec<&Located<'_>> = located
            .iter()
            .filter(|l| convs.get(&l.message.id) == Some(&conversation))
            .collect();
        members
            .sort_by_key(|l| std::cmp::Reverse((l.message.stored.meta.sort_date(), l.message.id)));
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for l in members {
            let meta = &l.message.stored.meta;
            if let Some(mid) = &meta.headers.message_id
                && !seen.insert(mid.clone())
            {
                continue;
            }
            out.push(ConversationMessage {
                id: l.message.id,
                account: l.account,
                mailbox: l.mailbox,
                uid: meta.uid,
                date: meta.sort_date(),
                from: meta.headers.from.first().cloned(),
                to: meta.headers.to.clone(),
                cc: meta.headers.cc.clone(),
                subject: meta.headers.subject.clone(),
                flags: meta.flags.clone(),
                has_attachments: meta.has_attachments(),
            });
            if out.len() >= MAX_CONVERSATION_MESSAGES {
                break;
            }
        }
        out.reverse();
        Ok(out)
    }
}

impl MemState {
    fn message_mut(&mut self, id: MessageId) -> Option<&mut MemMessage> {
        self.messages
            .values_mut()
            .flat_map(BTreeMap::values_mut)
            .find(|m| m.id == id)
    }

    /// Copie provisoire du message déplacé dans la destination (UID
    /// synthétique tout en haut de l'espace des UID).
    fn create_placeholder(&mut self, id: PendingOpId, op: &PendingOp, to: &MailboxName) {
        let Some(target) = self
            .mailboxes
            .iter()
            .find(|(a, m, _)| *a == op.account && m.mailbox.name == *to)
            .map(|(_, m, _)| m.id)
        else {
            return;
        };
        let source = self
            .messages
            .values()
            .flat_map(BTreeMap::values)
            .find(|m| m.id == op.message)
            .cloned();
        let Some(mut copy) = source else { return };
        let raw = u32::MAX - u32::try_from(id.get().rem_euclid(100_000_000)).unwrap_or(0);
        let Some(uid) = Uid::new(raw) else { return };
        self.next_message += 1;
        let Some(new_id) = MessageId::new(self.next_message) else {
            return;
        };
        copy.id = new_id;
        copy.stored.meta.uid = uid;
        copy.stored.meta.flags.deleted = false;
        copy.hidden = false;
        copy.pending_op = Some(id);
        self.messages.entry(target).or_default().insert(uid, copy);
    }

    fn remove_placeholders(&mut self, id: PendingOpId) {
        for msgs in self.messages.values_mut() {
            msgs.retain(|_, m| m.pending_op != Some(id));
        }
    }

    fn apply(&mut self, effect: LocalEffect) {
        match effect {
            LocalEffect::SetFlag {
                message,
                flag,
                value,
            } => {
                if let Some(m) = self.message_mut(message) {
                    flag.set(&mut m.stored.meta.flags, value);
                }
            }
            LocalEffect::Hide { message } => {
                if let Some(m) = self.message_mut(message) {
                    m.hidden = true;
                }
            }
            LocalEffect::Show { message } => {
                if let Some(m) = self.message_mut(message) {
                    m.hidden = false;
                }
            }
        }
    }
}

fn folder_ref(m: &StoredMailbox) -> FolderRef {
    FolderRef {
        id: m.id,
        name: m.mailbox.name.clone(),
        role: m.mailbox.role,
        selectable: m.mailbox.selectable,
    }
}

impl PendingOpStore for InMemoryMailStore {
    async fn action_target(&self, message: MessageId) -> Result<Option<LocalMessage>, StoreError> {
        let s = lock(&self.state);
        for (account, mailbox, state) in &s.mailboxes {
            if let Some(m) = s.messages.get(&mailbox.id).and_then(|msgs| {
                msgs.values()
                    .find(|m| m.id == message && !m.hidden && m.pending_op.is_none())
            }) {
                return Ok(Some(LocalMessage {
                    id: message,
                    account: *account,
                    mailbox: folder_ref(mailbox),
                    uid: m.stored.meta.uid,
                    flags: m.stored.meta.flags.clone(),
                    uid_validity: state.uid_validity,
                    highest_modseq: state.highest_modseq,
                }));
            }
        }
        Ok(None)
    }

    async fn conversation_members(
        &self,
        conversation: ConversationId,
    ) -> Result<Vec<Member>, StoreError> {
        let s = lock(&self.state);
        let convs = s.conversations();
        let mut out: Vec<Member> = s
            .located()
            .iter()
            .filter(|l| convs.get(&l.message.id) == Some(&conversation))
            .filter(|l| l.message.pending_op.is_none())
            .map(|l| Member {
                id: l.message.id,
                mailbox: l.mailbox,
                role: l.role,
                flags: l.message.stored.meta.flags.clone(),
                date: l.message.stored.meta.sort_date(),
            })
            .collect();
        out.sort_by_key(|m| (m.date, m.id));
        out.truncate(MAX_ACTION_TARGETS);
        Ok(out)
    }

    async fn action_folders(&self, account: AccountId) -> Result<Vec<FolderRef>, StoreError> {
        Ok(lock(&self.state)
            .mailboxes
            .iter()
            .filter(|(a, _, _)| *a == account)
            .map(|(_, m, _)| folder_ref(m))
            .collect())
    }

    async fn record_all(&self, changes: &[QueueChange]) -> Result<Vec<PendingOpId>, StoreError> {
        // Un seul verrou pour tout le lot : atomique vu des autres appels.
        let mut s = lock(&self.state);
        let mut recorded = Vec::new();
        for change in changes {
            let op = match change {
                QueueChange::Push(op) => op,
                QueueChange::CancelOrPush { existing, op } => {
                    if !s.claimed.contains_key(existing)
                        && let Some(old) = s.ops.remove(existing)
                    {
                        s.apply(revert_effect(&old));
                        s.remove_placeholders(*existing);
                        continue;
                    }
                    op
                }
            };
            s.next_op += 1;
            let id = PendingOpId::new(s.next_op).ok_or_else(|| StoreError("identifiant".into()))?;
            s.ops.insert(id, op.clone());
            s.apply(local_effect(op));
            if let OpKind::Move { to } = &op.kind {
                s.create_placeholder(id, op, to);
            }
            recorded.push(id);
        }
        Ok(recorded)
    }

    async fn purge_placeholders(
        &self,
        account: AccountId,
        mailbox: Option<MailboxId>,
    ) -> Result<usize, StoreError> {
        let mut s = lock(&self.state);
        let ids: Vec<MailboxId> = s
            .mailboxes
            .iter()
            .filter(|(a, m, _)| *a == account && mailbox.is_none_or(|x| x == m.id))
            .map(|(_, m, _)| m.id)
            .collect();
        let live: BTreeSet<PendingOpId> = s.ops.keys().copied().collect();
        let mut n = 0;
        for id in ids {
            if let Some(msgs) = s.messages.get_mut(&id) {
                let before = msgs.len();
                msgs.retain(|_, m| m.pending_op.is_none_or(|op| live.contains(&op)));
                n += before - msgs.len();
            }
        }
        Ok(n)
    }

    async fn pending(
        &self,
        account: AccountId,
    ) -> Result<Vec<(PendingOpId, PendingOp)>, StoreError> {
        Ok(lock(&self.state)
            .ops
            .iter()
            .filter(|(_, op)| op.account == account)
            .map(|(id, op)| (*id, op.clone()))
            .collect())
    }

    async fn complete(&self, id: PendingOpId) -> Result<bool, StoreError> {
        let mut s = lock(&self.state);
        s.claimed.remove(&id);
        Ok(s.ops.remove(&id).is_some())
    }

    async fn revert(&self, id: PendingOpId) -> Result<bool, StoreError> {
        let mut s = lock(&self.state);
        s.claimed.remove(&id);
        let Some(op) = s.ops.remove(&id) else {
            return Ok(false);
        };
        s.apply(revert_effect(&op));
        s.remove_placeholders(id);
        Ok(true)
    }

    async fn cancel(&self, id: PendingOpId) -> Result<bool, StoreError> {
        let mut s = lock(&self.state);
        if s.claimed.contains_key(&id) {
            return Ok(false);
        }
        let Some(op) = s.ops.remove(&id) else {
            return Ok(false);
        };
        s.apply(revert_effect(&op));
        s.remove_placeholders(id);
        Ok(true)
    }

    async fn claim(&self, id: PendingOpId) -> Result<Option<u32>, StoreError> {
        let mut s = lock(&self.state);
        if !s.ops.contains_key(&id) {
            return Ok(None);
        }
        let attempts = s.claimed.entry(id).or_insert(0);
        *attempts += 1;
        Ok(Some(*attempts))
    }

    async fn reapply(&self, account: AccountId) -> Result<(), StoreError> {
        let mut s = lock(&self.state);
        let effects: Vec<LocalEffect> = s
            .ops
            .values()
            .filter(|op| op.account == account)
            .map(local_effect)
            .collect();
        for effect in effects {
            s.apply(effect);
        }
        Ok(())
    }

    async fn archive_fallback(&self, account: AccountId) -> Result<Option<MailboxId>, StoreError> {
        Ok(lock(&self.state).archive.get(&account).copied())
    }

    async fn set_archive_fallback(
        &self,
        account: AccountId,
        mailbox: Option<MailboxId>,
    ) -> Result<(), StoreError> {
        let mut s = lock(&self.state);
        match mailbox {
            Some(m) => {
                if !s
                    .mailboxes
                    .iter()
                    .any(|(a, b, _)| *a == account && b.id == m)
                {
                    return Err(StoreError("dossier d'un autre compte".to_owned()));
                }
                s.archive.insert(account, m);
            }
            None => {
                s.archive.remove(&account);
            }
        }
        Ok(())
    }
}

impl norelly_domain::ports::ContactQuery for InMemoryMailStore {
    async fn suggest(
        &self,
        account: AccountId,
        text: &str,
        limit: usize,
    ) -> Result<Vec<Correspondent>, StoreError> {
        let needle = text.to_lowercase();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let s = lock(&self.state);
        // Adresse en minuscules → (nombre, correspondant).
        let mut seen: BTreeMap<String, (usize, Correspondent)> = BTreeMap::new();
        for (owner, mailbox, _) in &s.mailboxes {
            if *owner != account
                || matches!(mailbox.mailbox.role, MailboxRole::Junk | MailboxRole::Trash)
            {
                continue;
            }
            let Some(messages) = s.messages.get(&mailbox.id) else {
                continue;
            };
            for m in messages.values().filter(|m| m.pending_op.is_none()) {
                let h = &m.stored.meta.headers;
                let people: Vec<&Correspondent> = if mailbox.mailbox.role == MailboxRole::Sent {
                    h.to.iter().chain(&h.cc).collect()
                } else {
                    h.from.iter().collect()
                };
                for c in people {
                    let key = c.address.to_lowercase();
                    if key.is_empty() {
                        continue;
                    }
                    let entry = seen.entry(key).or_insert_with(|| (0, c.clone()));
                    entry.0 += 1;
                }
            }
        }
        let mut found: Vec<(usize, Correspondent)> = seen
            .into_iter()
            .filter(|(key, (_, c))| {
                key.contains(&needle) || c.name.to_lowercase().contains(&needle)
            })
            .map(|(_, v)| v)
            .collect();
        found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.address.cmp(&b.1.address)));
        Ok(found
            .into_iter()
            .take(limit.min(norelly_domain::ports::MAX_CONTACT_SUGGESTIONS))
            .map(|(_, c)| c)
            .collect())
    }
}

impl MessageLocator for InMemoryMailStore {
    async fn locate(&self, message: MessageId) -> Result<Option<MessageLocation>, StoreError> {
        let s = lock(&self.state);
        for (account, mailbox, _) in &s.mailboxes {
            if let Some(m) = s
                .messages
                .get(&mailbox.id)
                .and_then(|msgs| msgs.values().find(|m| m.id == message))
            {
                return Ok(Some(MessageLocation {
                    id: message,
                    account: *account,
                    mailbox: mailbox.id,
                    mailbox_name: mailbox.mailbox.name.clone(),
                    uid: m.stored.meta.uid,
                    from: m.stored.meta.headers.from.first().cloned(),
                    size: m.stored.meta.size,
                    structure: m.stored.meta.structure.clone(),
                    body: m.stored.body.clone(),
                }));
            }
        }
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// Recherche plein texte (faux : évaluation directe de la requête)
// ---------------------------------------------------------------------------

/// Colonnes cherchables d'un message, en jetons pliés, avec leur poids
/// dans le classement.
struct SearchDoc {
    columns: [(Field, u32, Vec<String>); 4],
}

impl SearchDoc {
    fn of(message: &StoredMessage, body: &str) -> Self {
        let h = &message.meta.headers;
        let people = |list: &[Correspondent]| -> Vec<String> {
            list.iter()
                .flat_map(|c| tokens(&c.name).into_iter().chain(tokens(&c.address)))
                .collect()
        };
        let recipients: Vec<Correspondent> = h.to.iter().chain(&h.cc).cloned().collect();
        Self {
            columns: [
                (Field::Subject, 10, tokens(&h.subject)),
                (Field::From, 5, people(&h.from)),
                (Field::To, 3, people(&recipients)),
                (Field::Any, 1, tokens(body)),
            ],
        }
    }

    /// Poids de la meilleure colonne où figurent les mots `words` (le
    /// dernier en préfixe sauf pour une phrase), `None` sinon.
    fn score(&self, field: Field, words: &[String], phrase: bool) -> Option<u32> {
        self.columns
            .iter()
            .filter(|(f, _, _)| field == Field::Any || *f == field)
            .filter(|(_, _, doc)| contains_words(doc, words, phrase))
            .map(|(_, weight, _)| *weight)
            .max()
    }
}

fn contains_words(doc: &[String], words: &[String], phrase: bool) -> bool {
    let Some((last, head)) = words.split_last() else {
        return false;
    };
    doc.windows(words.len()).any(|w| {
        w.iter().zip(head).all(|(d, t)| d == t)
            && w.last().is_some_and(|d| {
                if phrase {
                    d == last
                } else {
                    d.starts_with(last.as_str())
                }
            })
    })
}

impl norelly_domain::ports::SearchIndex for InMemoryMailStore {
    async fn search(&self, request: SearchRequest) -> Result<SearchPage, StoreError> {
        let limit = request.limit.min(MAX_PAGE_SIZE);
        if limit == 0 || request.offset >= MAX_SEARCH_RESULTS {
            return Ok(SearchPage::default());
        }
        // Clauses effectives : une clause de texte sans mot est ignorée.
        let clauses: Vec<(bool, Atom, Vec<String>)> = request
            .query
            .clauses()
            .iter()
            .map(|c| {
                let words = match &c.atom {
                    Atom::Text { text, .. } => tokens(text),
                    Atom::Filter(_) => Vec::new(),
                };
                (c.negated, c.atom.clone(), words)
            })
            .filter(|(_, atom, words)| matches!(atom, Atom::Filter(_)) || !words.is_empty())
            .collect();
        if clauses.is_empty() {
            return Ok(SearchPage::default());
        }
        let ranked = clauses
            .iter()
            .any(|(negated, atom, _)| !negated && matches!(atom, Atom::Text { .. }));
        let terms: Vec<&String> = clauses
            .iter()
            .filter(|(negated, atom, _)| !negated && matches!(atom, Atom::Text { .. }))
            .flat_map(|(_, _, words)| words)
            .collect();
        let s = lock(&self.state);
        let convs = s.conversations();
        let located = s.located();
        let outside_all: BTreeSet<(AccountId, &str)> = located
            .iter()
            .filter(|l| l.role != MailboxRole::All)
            .filter_map(|l| {
                let mid = l.message.stored.meta.headers.message_id.as_deref()?;
                Some((l.account, mid))
            })
            .collect();
        let mut hits: Vec<(u32, &Located<'_>)> = Vec::new();
        'messages: for l in &located {
            let in_scope = match request.scope {
                SearchScope::All => true,
                SearchScope::Account(a) => l.account == a,
                SearchScope::Mailbox(m) => l.mailbox == m,
            };
            if !in_scope {
                continue;
            }
            let meta = &l.message.stored.meta;
            if !matches!(request.scope, SearchScope::Mailbox(_))
                && l.role == MailboxRole::All
                && meta
                    .headers
                    .message_id
                    .as_deref()
                    .is_some_and(|mid| outside_all.contains(&(l.account, mid)))
            {
                continue;
            }
            let doc = SearchDoc::of(&l.message.stored, &l.message.text);
            let date = meta.sort_date();
            let mut score = 0;
            for (negated, atom, words) in &clauses {
                let matched = match atom {
                    Atom::Text { field, phrase, .. } => {
                        let found = doc.score(*field, words, *phrase);
                        if !negated {
                            score += found.unwrap_or(0);
                        }
                        found.is_some()
                    }
                    Atom::Filter(filter) => match filter {
                        Filter::HasAttachment => meta.has_attachments(),
                        Filter::Unread => !meta.flags.seen,
                        Filter::Read => meta.flags.seen,
                        Filter::Flagged => meta.flags.flagged,
                        Filter::Before(d) => date < d.start_timestamp(request.utc_offset_seconds),
                        Filter::After(d) => date >= d.start_timestamp(request.utc_offset_seconds),
                    },
                };
                if matched == *negated {
                    continue 'messages;
                }
            }
            hits.push((score, l));
        }
        hits.sort_by(|(sa, a), (sb, b)| {
            let by_date = (b.message.stored.meta.sort_date(), b.message.id)
                .cmp(&(a.message.stored.meta.sort_date(), a.message.id));
            if ranked {
                sb.cmp(sa).then(by_date)
            } else {
                by_date
            }
        });
        let total = hits.len().min(MAX_SEARCH_RESULTS as usize);
        let start = request.offset as usize;
        let end = (start + limit as usize).min(total);
        let items = hits
            .get(start..end)
            .unwrap_or_default()
            .iter()
            .map(|(_, l)| {
                let meta = &l.message.stored.meta;
                SearchHit {
                    id: l.message.id,
                    account: l.account,
                    mailbox: l.mailbox,
                    conversation: convs.get(&l.message.id).copied(),
                    uid: meta.uid,
                    date: meta.sort_date(),
                    from: meta.headers.from.first().cloned(),
                    subject: meta.headers.subject.clone(),
                    snippet: snippet(&l.message.text, &terms),
                    flags: meta.flags.clone(),
                    has_attachments: meta.has_attachments(),
                }
            })
            .collect();
        let next = (end < total).then(|| u32::try_from(end).unwrap_or(MAX_SEARCH_RESULTS));
        Ok(SearchPage { items, next })
    }
}

/// Passage du texte borné à [`MAX_SNIPPET_CHARS`] caractères : quelques
/// mots avant le premier mot qui commence par l'un des `terms`, sinon le
/// début.
fn snippet(text: &str, terms: &[&String]) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let first_hit = words.iter().position(|w| {
        tokens(w)
            .iter()
            .any(|t| terms.iter().any(|term| t.starts_with(term.as_str())))
    });
    let start = first_hit.map_or(0, |i| i.saturating_sub(4));
    words
        .get(start..)
        .unwrap_or_default()
        .join(" ")
        .chars()
        .take(MAX_SNIPPET_CHARS)
        .collect()
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
