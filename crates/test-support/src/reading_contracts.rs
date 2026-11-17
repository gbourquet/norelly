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
    ConversationId, ConversationListItem, ConversationQuery, MAX_CONVERSATION_MESSAGES, MailStore,
    MessageLocator, ReadingPreferences, SettingsStore, StoreError, StoredBody, StoredMailbox,
    StoredMessage, SyncBatch,
};
use norelly_domain::sender_auth::SenderAuth;
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

/// `message`, avec le verdict d'authentification `auth` sur son
/// expéditeur.
fn authenticated(mut message: StoredMessage, auth: SenderAuth) -> StoredMessage {
    message.meta.headers.sender_auth = auth;
    message
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
                authenticated(
                    threaded(1, 200, "p2@a", &["p1@a"], "Re: Projet X", "Bob", false),
                    SenderAuth::Pass,
                ),
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
            &batch(vec![authenticated(
                threaded(
                    1,
                    300,
                    "p3@a",
                    &["p1@a", "p2@a"],
                    "Re: Projet X",
                    "Moi",
                    true,
                ),
                SenderAuth::Fail,
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
    // Verdict d'authentification enregistré avec chaque message.
    let auth: Vec<SenderAuth> = messages.iter().map(|m| m.sender_auth).collect();
    assert_eq!(
        auth,
        [SenderAuth::Unknown, SenderAuth::Pass, SenderAuth::Fail]
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

    // --- Drapeaux : compteurs à jour, mots-clés réunis ---
    let mut seen = Flags::from_tokens(["\\Seen", "\\Flagged", "Projet"]);
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
    assert_eq!(
        item.first()
            .map(|i| i.keywords.iter().map(|k| k.as_str()).collect::<Vec<_>>()),
        Some(vec!["Projet"]),
        "mots-clés des messages de la conversation dans le dossier"
    );

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

/// Contrat de [`SettingsStore`] (stockage vide au départ).
pub async fn settings_store_contract<S: SettingsStore>(store: &S) -> Result<(), StoreError> {
    use norelly_domain::settings::{
        Language, MAX_ATTACHMENT_THRESHOLD, MAX_MARK_SEEN_DELAY_SECS, Settings, UpdateCheck,
    };
    assert_eq!(
        store.settings().await?,
        Settings::default(),
        "valeurs par défaut"
    );
    let chosen = Settings {
        language: Language::English,
        update_check: UpdateCheck::Disabled,
        notifications: false,
        notification_privacy: true,
        attachment_threshold: 8 * 1024 * 1024,
        mark_seen_delay_secs: Some(12),
    };
    store.save_settings(&chosen).await?;
    assert_eq!(store.settings().await?, chosen);
    let never = Settings {
        mark_seen_delay_secs: None,
        ..chosen
    };
    store.save_settings(&never).await?;
    assert_eq!(store.settings().await?, never, "« jamais » relu tel quel");
    store.save_settings(&chosen).await?;
    let invalid = Settings {
        attachment_threshold: MAX_ATTACHMENT_THRESHOLD + 1,
        ..chosen
    };
    assert!(
        store.save_settings(&invalid).await.is_err(),
        "hors bornes refusé"
    );
    assert_eq!(store.settings().await?, chosen, "rien enregistré");
    let invalid = Settings {
        mark_seen_delay_secs: Some(MAX_MARK_SEEN_DELAY_SECS + 1),
        ..chosen
    };
    assert!(
        store.save_settings(&invalid).await.is_err(),
        "délai hors bornes refusé"
    );
    assert_eq!(store.settings().await?, chosen, "rien enregistré");
    Ok(())
}

/// Faux [`SettingsStore`] en mémoire.
#[derive(Debug, Default)]
pub struct InMemorySettings {
    settings: Mutex<Option<norelly_domain::settings::Settings>>,
}

impl SettingsStore for InMemorySettings {
    async fn settings(&self) -> Result<norelly_domain::settings::Settings, StoreError> {
        Ok(self
            .settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .unwrap_or_default())
    }

    async fn save_settings(
        &self,
        settings: &norelly_domain::settings::Settings,
    ) -> Result<(), StoreError> {
        settings.validate().map_err(|e| StoreError(e.to_string()))?;
        *self.settings.lock().unwrap_or_else(PoisonError::into_inner) = Some(*settings);
        Ok(())
    }
}

/// Contrat du dédoublonnage de [`ConversationQuery::conversation_messages`]
/// face à un message forgé qui reprend le `Message-ID` d'un message reçu :
/// l'original reste affiché, et son repère d'exemplaires (`newest_copy`)
/// couvre tous les exemplaires présents. Compte `account` existant, sans
/// boîte.
pub async fn forged_copy_contract<S>(store: &S, account: AccountId) -> Result<(), StoreError>
where
    S: MailStore + ConversationQuery,
{
    let boxes = store
        .replace_mailboxes(
            account,
            &[
                mailbox("INBOX", MailboxRole::Inbox, true),
                mailbox("Archives", MailboxRole::Archive, true),
                mailbox("Junk", MailboxRole::Junk, true),
            ],
        )
        .await?;
    let (inbox, archives, junk) = (
        id_of(&boxes, "INBOX"),
        id_of(&boxes, "Archives"),
        id_of(&boxes, "Junk"),
    );
    let original = |n: u32| threaded(n, 100, "f1@a", &[], "Facture", "Alice", true);
    store.apply_sync(inbox, &batch(vec![original(1)])).await?;
    store
        .apply_sync(archives, &batch(vec![original(1)]))
        .await?;
    let conversation = all_pages(store, ConversationScope::Mailbox(inbox), 10)
        .await?
        .first()
        .map(|c| c.id)
        .ok_or_else(|| StoreError("conversation".into()))?;
    let view = |messages: &[norelly_domain::ports::ConversationMessage]| {
        messages
            .iter()
            .map(|m| {
                (
                    m.mailbox,
                    m.from.as_ref().map_or(String::new(), |f| f.name.clone()),
                )
            })
            .collect::<Vec<_>>()
    };
    let before = store.conversation_messages(conversation).await?;
    assert_eq!(view(&before), [(inbox, "Alice".to_owned())], "{before:?}");
    let shown = before.first().ok_or_else(|| StoreError("vide".into()))?;
    assert!(
        shown.newest_copy > shown.id,
        "l'exemplaire des Archives est couvert : {shown:?}"
    );
    assert!(shown.flags.seen);

    // Messages forgés avec le même Message-ID, remis après l'original :
    // plus récent dans la boîte de réception, daté du passé (en-tête
    // Date) dans les Indésirables.
    let forged = |n: u32, date: i64, received: i64| {
        let mut m = threaded(n, date, "f1@a", &[], "Facture", "Usurpateur", false);
        m.meta.internal_date = received;
        m
    };
    store
        .apply_sync(inbox, &batch(vec![forged(2, 10_000, 10_000)]))
        .await?;
    store
        .apply_sync(junk, &batch(vec![forged(1, 50, 600)]))
        .await?;
    let after = store.conversation_messages(conversation).await?;
    assert_eq!(view(&after), [(inbox, "Alice".to_owned())], "{after:?}");
    let still = after.first().ok_or_else(|| StoreError("vide".into()))?;
    assert_eq!(still.id, shown.id, "l'original reste affiché");
    assert!(
        still.newest_copy > shown.newest_copy,
        "les exemplaires arrivés depuis relèvent le repère : {still:?}"
    );
    assert!(!still.flags.seen, "non lu tant qu'un exemplaire l'est");

    // L'original change de dossier après l'arrivée du forgé : il reçoit
    // un nouvel identifiant, plus grand que celui du forgé, mais garde sa
    // date de remise.
    let removed = |uid_n: u32| SyncBatch {
        removed: vec![uid(uid_n)],
        ..batch(vec![])
    };
    store.apply_sync(inbox, &removed(1)).await?;
    store.apply_sync(archives, &removed(1)).await?;
    store
        .apply_sync(archives, &batch(vec![original(2)]))
        .await?;
    // Le faux renumérote les conversations : on la retrouve par le dossier.
    let conversation = all_pages(store, ConversationScope::Mailbox(archives), 10)
        .await?
        .first()
        .map(|c| c.id)
        .ok_or_else(|| StoreError("conversation déplacée".into()))?;
    let moved = store.conversation_messages(conversation).await?;
    assert_eq!(
        view(&moved),
        [(archives, "Alice".to_owned())],
        "l'original déplacé reste affiché : {moved:?}"
    );

    // Afflux de réponses récentes : l'original, ancien, sort des messages
    // affichés, sans que le forgé, daté de plus récent, prenne sa place.
    let flood: Vec<StoredMessage> = (0..u32::try_from(MAX_CONVERSATION_MESSAGES * 4)
        .unwrap_or(u32::MAX))
        .map(|i| {
            threaded(
                10 + i,
                1_000 + i64::from(i),
                &format!("r{i}@a"),
                &["f1@a"],
                "Re: Facture",
                "Bob",
                true,
            )
        })
        .collect();
    store.apply_sync(inbox, &batch(flood)).await?;
    let flooded = store.conversation_messages(conversation).await?;
    assert_eq!(flooded.len(), MAX_CONVERSATION_MESSAGES);
    assert!(
        flooded
            .iter()
            .all(|m| m.from.as_ref().is_some_and(|f| f.name == "Bob")),
        "aucun exemplaire forgé affiché"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail_contracts::InMemoryMailStore;

    #[tokio::test]
    async fn in_memory_forged_copies_pass_contract() -> Result<(), StoreError> {
        let account = AccountId::new(1).map_err(|e| StoreError(e.to_string()))?;
        forged_copy_contract(&InMemoryMailStore::default(), account).await
    }

    #[tokio::test]
    async fn in_memory_conversations_pass_contract() -> Result<(), StoreError> {
        let (a, b) = (
            AccountId::new(1).map_err(|e| StoreError(e.to_string()))?,
            AccountId::new(2).map_err(|e| StoreError(e.to_string()))?,
        );
        conversation_query_contract(&InMemoryMailStore::default(), a, b).await
    }

    #[tokio::test]
    async fn in_memory_settings_pass_contract() -> Result<(), StoreError> {
        settings_store_contract(&InMemorySettings::default()).await
    }

    #[tokio::test]
    async fn in_memory_preferences_pass_contract() -> Result<(), StoreError> {
        reading_preferences_contract(&InMemoryReadingPreferences::default()).await
    }
}
