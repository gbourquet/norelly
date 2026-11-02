//! Suites de contrat de la lecture : [`ConversationQuery`] et
//! [`MessageLocator`] (faux en mémoire qui recalcule JWZ à chaque requête,
//! contre SQLite qui rattache les messages au fil de l'eau), et
//! [`ReadingPreferences`].

use std::collections::BTreeSet;
use std::sync::{Mutex, PoisonError};

use norelly_domain::account::AccountId;
use norelly_domain::address::EmailAddress;
use norelly_domain::conversation::ConversationScope;
use norelly_domain::mailbox::{
    Correspondent, Flags, MailboxId, MailboxRole, MessageHeaders, MessageMeta,
};
use norelly_domain::ports::{
    ConversationId, ConversationListItem, ConversationQuery, MailStore, MessageLocator,
    ReadingPreferences, StoreError, StoredBody, StoredMailbox, StoredMessage, SyncBatch,
};
use norelly_domain::sync::FlagUpdate;

use crate::mail_contracts::{mailbox, state, uid};

/// Message de test relié par ses identifiants.
fn threaded(
    n: u32,
    date: i64,
    own: &str,
    refs: &[&str],
    subject: &str,
    from: &str,
    seen: bool,
) -> StoredMessage {
    StoredMessage {
        meta: MessageMeta {
            uid: uid(n),
            flags: Flags::from_tokens(if seen { vec!["\\Seen"] } else { vec![] }),
            modseq: None,
            internal_date: date,
            size: 100,
            headers: MessageHeaders {
                subject: subject.to_owned(),
                from: vec![Correspondent::new(
                    from,
                    &format!("{}@example.com", from.to_lowercase()),
                )],
                date: Some(date),
                message_id: Some(own.to_owned()),
                references: refs.iter().map(|r| (*r).to_owned()).collect(),
                ..MessageHeaders::default()
            },
            structure: None,
        },
        body: StoredBody::Missing,
    }
}

fn batch(new: Vec<StoredMessage>) -> SyncBatch {
    SyncBatch {
        reset: false,
        removed: vec![],
        flag_changes: vec![],
        new,
        state: state(1, 10, 100),
    }
}

fn id_of(list: &[StoredMailbox], name: &str) -> MailboxId {
    list.iter()
        .find(|m| m.mailbox.name.as_str() == name)
        .map(|m| m.id)
        .unwrap_or_else(|| panic!("boîte {name} absente"))
}

async fn all_pages<S: ConversationQuery>(
    store: &S,
    scope: ConversationScope,
    limit: u32,
) -> Result<Vec<ConversationListItem>, StoreError> {
    let mut out = Vec::new();
    let mut cursor = None;
    for _ in 0..100 {
        let page = store.conversations(scope, cursor, limit).await?;
        assert!(page.items.len() <= limit as usize);
        out.extend(page.items);
        match page.next {
            Some(next) => cursor = Some(next),
            None => return Ok(out),
        }
    }
    panic!("pagination sans fin");
}

fn senders(item: &ConversationListItem) -> Vec<&str> {
    item.senders.iter().map(|s| s.name.as_str()).collect()
}

/// Contrat de [`ConversationQuery`] et [`MessageLocator`] : comptes
/// `account` et `other` existants, sans boîte.
pub async fn conversation_query_contract<S>(
    store: &S,
    account: AccountId,
    other: AccountId,
) -> Result<(), StoreError>
where
    S: MailStore + ConversationQuery + MessageLocator,
{
    let boxes = store
        .replace_mailboxes(
            account,
            &[
                mailbox("INBOX", MailboxRole::Inbox, true),
                mailbox("Sent", MailboxRole::Sent, true),
                mailbox("Archives", MailboxRole::Archive, true),
            ],
        )
        .await?;
    let (inbox, sent, archives) = (
        id_of(&boxes, "INBOX"),
        id_of(&boxes, "Sent"),
        id_of(&boxes, "Archives"),
    );
    let other_inbox = id_of(
        &store
            .replace_mailboxes(other, &[mailbox("INBOX", MailboxRole::Inbox, true)])
            .await?,
        "INBOX",
    );

    // La réponse arrive avant l'original.
    store
        .apply_sync(
            inbox,
            &batch(vec![
                threaded(1, 200, "p2@a", &["p1@a"], "Re: Projet X", "Bob", false),
                threaded(2, 150, "o1@a", &[], "Autre sujet", "Carol", true),
            ]),
        )
        .await?;
    store
        .apply_sync(
            inbox,
            &batch(vec![threaded(
                3,
                100,
                "p1@a",
                &[],
                "Projet X",
                "Alice",
                true,
            )]),
        )
        .await?;
    store
        .apply_sync(
            sent,
            &batch(vec![threaded(
                1,
                300,
                "p3@a",
                &["p1@a", "p2@a"],
                "Re: Projet X",
                "Moi",
                true,
            )]),
        )
        .await?;
    // Copie du même message dans un autre dossier.
    store
        .apply_sync(
            archives,
            &batch(vec![threaded(
                1,
                150,
                "o1@a",
                &[],
                "Autre sujet",
                "Carol",
                true,
            )]),
        )
        .await?;
    // Second compte : « Re: Bonjour » sans référence rejoint « Bonjour » par
    // le sujet ; un Message-ID identique à celui d'un autre compte ne relie
    // rien entre comptes.
    store
        .apply_sync(
            other_inbox,
            &batch(vec![
                threaded(1, 250, "b1@b", &[], "Bonjour", "Dan", false),
                threaded(2, 260, "b2@b", &[], "Re: Bonjour", "Eve", false),
                threaded(3, 50, "p1@a", &[], "Projet X", "Usurpateur", false),
            ]),
        )
        .await?;

    // --- Un dossier ---
    let list = all_pages(store, ConversationScope::Mailbox(inbox), 50).await?;
    assert_eq!(list.len(), 2, "{list:?}");
    let projet = &list[0];
    assert_eq!(projet.subject, "Re: Projet X", "sujet du plus récent");
    assert_eq!((projet.date, projet.messages, projet.unread), (200, 2, 1));
    assert_eq!(senders(projet), ["Bob", "Alice"]);
    assert_eq!((projet.account, projet.mailbox), (account, inbox));
    let autre = &list[1];
    assert_eq!(
        (autre.subject.as_str(), autre.messages, autre.unread),
        ("Autre sujet", 1, 0)
    );

    // --- Boîte unifiée, paginée ---
    let unified = all_pages(store, ConversationScope::unified_inbox(), 50).await?;
    let summary: Vec<(i64, u32, AccountId)> = unified
        .iter()
        .map(|i| (i.date, i.messages, i.account))
        .collect();
    assert_eq!(
        summary,
        [
            (260, 2, other),
            (200, 2, account),
            (150, 1, account),
            (50, 1, other)
        ]
    );
    assert_eq!(senders(&unified[0]), ["Eve", "Dan"]);
    let paged = all_pages(store, ConversationScope::unified_inbox(), 1).await?;
    assert_eq!(paged, unified, "pagination stable");
    let sent_list = all_pages(store, ConversationScope::Unified(MailboxRole::Sent), 10).await?;
    assert_eq!(sent_list.len(), 1);
    assert_eq!(sent_list[0].id, projet.id, "même conversation dans Envoyés");
    assert!(
        store
            .conversations(ConversationScope::unified_inbox(), None, 0)
            .await?
            .items
            .is_empty()
    );

    // --- Messages d'une conversation (tous dossiers, sans doublon) ---
    let messages = store.conversation_messages(projet.id).await?;
    let view: Vec<(i64, MailboxId, &str)> = messages
        .iter()
        .map(|m| {
            (
                m.date,
                m.mailbox,
                m.from.as_ref().map_or("", |f| f.name.as_str()),
            )
        })
        .collect();
    assert_eq!(
        view,
        [
            (100, inbox, "Alice"),
            (200, inbox, "Bob"),
            (300, sent, "Moi")
        ]
    );
    assert_eq!(
        store.conversation_messages(autre.id).await?.len(),
        1,
        "copie dédoublonnée"
    );
    let unknown = ConversationId::new(i64::MAX).ok_or_else(|| StoreError("id".into()))?;
    assert!(store.conversation_messages(unknown).await?.is_empty());

    // --- Emplacement d'un message ---
    let first = messages.first().ok_or_else(|| StoreError("vide".into()))?;
    let location = store
        .locate(first.id)
        .await?
        .ok_or_else(|| StoreError("message introuvable".into()))?;
    assert_eq!(
        (location.account, location.mailbox, location.uid),
        (account, inbox, uid(3))
    );
    assert_eq!(location.mailbox_name.as_str(), "INBOX");
    assert_eq!(location.from.map(|f| f.name), Some("Alice".into()));
    assert_eq!(location.body, StoredBody::Missing);

    // --- Fusion : un message cite les deux conversations ---
    store
        .apply_sync(
            inbox,
            &batch(vec![threaded(
                4,
                400,
                "m1@a",
                &["o1@a", "p1@a"],
                "Re: Autre sujet",
                "Bob",
                false,
            )]),
        )
        .await?;
    let list = all_pages(store, ConversationScope::Mailbox(inbox), 50).await?;
    assert_eq!(list.len(), 1, "conversations fusionnées : {list:?}");
    let merged = &list[0];
    assert_eq!((merged.date, merged.messages, merged.unread), (400, 4, 2));
    assert_eq!(senders(merged), ["Bob", "Carol", "Alice"]);
    assert_eq!(store.conversation_messages(merged.id).await?.len(), 5);
    let sent_list = all_pages(store, ConversationScope::Unified(MailboxRole::Sent), 10).await?;
    assert_eq!(sent_list.first().map(|i| i.id), Some(merged.id));

    // --- Drapeaux : compteurs à jour ---
    let mut seen = Flags::from_tokens(["\\Seen", "\\Flagged"]);
    seen.answered = false;
    store
        .apply_sync(
            inbox,
            &SyncBatch {
                flag_changes: vec![FlagUpdate {
                    uid: uid(1),
                    flags: seen,
                    modseq: None,
                }],
                ..batch(vec![])
            },
        )
        .await?;
    let item = all_pages(store, ConversationScope::Mailbox(inbox), 50).await?;
    assert_eq!(item.first().map(|i| (i.unread, i.flagged)), Some((1, true)));

    // --- Suppressions ---
    store
        .apply_sync(
            inbox,
            &SyncBatch {
                removed: vec![uid(2)],
                ..batch(vec![])
            },
        )
        .await?;
    let item = all_pages(store, ConversationScope::Mailbox(inbox), 50).await?;
    assert_eq!(item.first().map(|i| i.messages), Some(3));
    store
        .apply_sync(
            inbox,
            &SyncBatch {
                reset: true,
                ..batch(vec![])
            },
        )
        .await?;
    assert!(
        all_pages(store, ConversationScope::Mailbox(inbox), 50)
            .await?
            .is_empty()
    );
    let unified = all_pages(store, ConversationScope::unified_inbox(), 50).await?;
    assert!(unified.iter().all(|i| i.account == other), "{unified:?}");
    assert_eq!(
        all_pages(store, ConversationScope::Unified(MailboxRole::Sent), 10)
            .await?
            .len(),
        1
    );

    // --- Boîte disparue ---
    store
        .replace_mailboxes(
            account,
            &[
                mailbox("INBOX", MailboxRole::Inbox, true),
                mailbox("Sent", MailboxRole::Sent, true),
            ],
        )
        .await?;
    assert!(
        all_pages(store, ConversationScope::Mailbox(archives), 10)
            .await?
            .is_empty()
    );
    assert!(
        all_pages(store, ConversationScope::Unified(MailboxRole::Archive), 10)
            .await?
            .is_empty()
    );
    // Rôle modifié : la boîte unifiée suit.
    store
        .replace_mailboxes(
            account,
            &[
                mailbox("INBOX", MailboxRole::Inbox, true),
                mailbox("Sent", MailboxRole::Archive, true),
            ],
        )
        .await?;
    assert!(
        all_pages(store, ConversationScope::Unified(MailboxRole::Sent), 10)
            .await?
            .is_empty()
    );
    assert_eq!(
        all_pages(store, ConversationScope::Unified(MailboxRole::Archive), 10)
            .await?
            .len(),
        1
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Préférences de lecture
// ---------------------------------------------------------------------------

/// Faux en mémoire de [`ReadingPreferences`].
#[derive(Debug, Default)]
pub struct InMemoryReadingPreferences {
    senders: Mutex<BTreeSet<String>>,
    plain_text: Mutex<bool>,
}

impl ReadingPreferences for InMemoryReadingPreferences {
    async fn remote_images_allowed(&self, sender: &EmailAddress) -> Result<bool, StoreError> {
        Ok(self
            .senders
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&sender.to_string()))
    }

    async fn allow_remote_images(&self, sender: &EmailAddress) -> Result<(), StoreError> {
        self.senders
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(sender.to_string());
        Ok(())
    }

    async fn revoke_remote_images(&self, sender: &EmailAddress) -> Result<(), StoreError> {
        self.senders
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&sender.to_string());
        Ok(())
    }

    async fn prefer_plain_text(&self) -> Result<bool, StoreError> {
        Ok(*self
            .plain_text
            .lock()
            .unwrap_or_else(PoisonError::into_inner))
    }

    async fn set_prefer_plain_text(&self, prefer: bool) -> Result<(), StoreError> {
        *self
            .plain_text
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = prefer;
        Ok(())
    }
}

/// Contrat de [`ReadingPreferences`].
pub async fn reading_preferences_contract<P: ReadingPreferences>(
    prefs: &P,
) -> Result<(), StoreError> {
    let parse = |raw: &str| EmailAddress::parse(raw).map_err(|e| StoreError(e.to_string()));
    let alice = parse("alice@Example.com")?;
    let alice_lower = parse("alice@example.com")?;
    let bob = parse("bob@example.com")?;

    assert!(
        !prefs.remote_images_allowed(&alice).await?,
        "bloqué par défaut"
    );
    prefs.allow_remote_images(&alice).await?;
    prefs.allow_remote_images(&alice).await?;
    assert!(prefs.remote_images_allowed(&alice).await?);
    assert!(
        prefs.remote_images_allowed(&alice_lower).await?,
        "domaine sans casse"
    );
    assert!(!prefs.remote_images_allowed(&bob).await?, "par expéditeur");
    prefs.revoke_remote_images(&alice).await?;
    prefs.revoke_remote_images(&alice).await?;
    assert!(!prefs.remote_images_allowed(&alice).await?);

    assert!(!prefs.prefer_plain_text().await?, "HTML par défaut");
    prefs.set_prefer_plain_text(true).await?;
    assert!(prefs.prefer_plain_text().await?);
    prefs.set_prefer_plain_text(false).await?;
    assert!(!prefs.prefer_plain_text().await?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail_contracts::InMemoryMailStore;

    #[tokio::test]
    async fn in_memory_conversations_pass_contract() -> Result<(), StoreError> {
        let (a, b) = (
            AccountId::new(1).map_err(|e| StoreError(e.to_string()))?,
            AccountId::new(2).map_err(|e| StoreError(e.to_string()))?,
        );
        conversation_query_contract(&InMemoryMailStore::default(), a, b).await
    }

    #[tokio::test]
    async fn in_memory_preferences_pass_contract() -> Result<(), StoreError> {
        reading_preferences_contract(&InMemoryReadingPreferences::default()).await
    }
}
