use norelly_domain::ports::AccountRepository;
use norelly_test_support::actions_contracts::pending_op_store_contract;
use norelly_test_support::contracts::sample_account;
use norelly_test_support::labels_contracts::label_store_contract;
use norelly_test_support::mail_contracts::{
    gmail_body_reuse_contract, mail_store_contract, sample_message,
};
use norelly_test_support::reading_contracts::{conversation_query_contract, forged_copy_contract};

use super::*;
use crate::SqliteAccountRepository;

fn temp_path(name: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("norelly-sqlite-mail-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    dir.join("test.sqlite3")
}

async fn two_accounts(db: &Database) -> Result<(AccountId, AccountId), StoreError> {
    let repo = SqliteAccountRepository::new(db.clone());
    let a = repo
        .insert(&sample_account("alice@example.com", false))
        .await
        .map_err(|e| StoreError(e.to_string()))?;
    let b = repo
        .insert(&sample_account("bob@example.com", false))
        .await
        .map_err(|e| StoreError(e.to_string()))?;
    Ok((a.id(), b.id()))
}

#[tokio::test]
async fn in_memory_store_passes_contract() -> Result<(), StoreError> {
    let db = Database::open_in_memory(norelly_test_support::contact_import_contracts::FakeNfc)
        .map_err(|e| StoreError(e.to_string()))?;
    let (a, b) = two_accounts(&db).await?;
    mail_store_contract(&SqliteMailStore::new(db), a, b).await
}

#[tokio::test]
async fn gmail_body_reuse_passes_contract() -> Result<(), StoreError> {
    let db = Database::open_in_memory(norelly_test_support::contact_import_contracts::FakeNfc)
        .map_err(|e| StoreError(e.to_string()))?;
    let (a, b) = two_accounts(&db).await?;
    gmail_body_reuse_contract(&SqliteMailStore::new(db.clone()), a, b).await?;
    assert_eq!(db.integrity_problems().await?, Vec::<String>::new());
    Ok(())
}

#[tokio::test]
async fn pending_ops_pass_contract() -> Result<(), StoreError> {
    let db = Database::open_in_memory(norelly_test_support::contact_import_contracts::FakeNfc)
        .map_err(|e| StoreError(e.to_string()))?;
    let (a, b) = two_accounts(&db).await?;
    pending_op_store_contract(&SqliteMailStore::new(db), a, b).await
}

#[tokio::test]
async fn labels_pass_contract() -> Result<(), StoreError> {
    let path = temp_path("labels");
    let db = Database::open(
        &path,
        norelly_test_support::contact_import_contracts::FakeNfc,
    )
    .map_err(|e| StoreError(e.to_string()))?;
    let (a, b) = two_accounts(&db).await?;
    label_store_contract(&SqliteMailStore::new(db.clone()), a, b).await?;
    assert_eq!(db.integrity_problems().await?, Vec::<String>::new());
    let _ = std::fs::remove_dir_all(path.parent().unwrap_or(&path));
    Ok(())
}

#[tokio::test]
async fn file_pending_ops_with_reader_pool_pass_contract() -> Result<(), StoreError> {
    let path = temp_path("pending-ops");
    let db = Database::open(
        &path,
        norelly_test_support::contact_import_contracts::FakeNfc,
    )
    .map_err(|e| StoreError(e.to_string()))?;
    let (a, b) = two_accounts(&db).await?;
    pending_op_store_contract(&SqliteMailStore::new(db.clone()), a, b).await?;
    // Base saine après les écritures (clés étrangères, déclencheurs).
    assert_eq!(db.integrity_problems().await?, Vec::<String>::new());
    let _ = std::fs::remove_dir_all(path.parent().unwrap_or(&path));
    Ok(())
}

#[tokio::test]
async fn conversations_pass_contract() -> Result<(), StoreError> {
    let db = Database::open_in_memory(norelly_test_support::contact_import_contracts::FakeNfc)
        .map_err(|e| StoreError(e.to_string()))?;
    let (a, b) = two_accounts(&db).await?;
    conversation_query_contract(&SqliteMailStore::new(db), a, b).await
}

#[tokio::test]
async fn forged_copies_pass_contract() -> Result<(), StoreError> {
    let db = Database::open_in_memory(norelly_test_support::contact_import_contracts::FakeNfc)
        .map_err(|e| StoreError(e.to_string()))?;
    let (a, _) = two_accounts(&db).await?;
    forged_copy_contract(&SqliteMailStore::new(db), a).await
}

#[tokio::test]
async fn file_conversations_with_reader_pool_pass_contract() -> Result<(), StoreError> {
    let path = temp_path("conversations");
    let db = Database::open(
        &path,
        norelly_test_support::contact_import_contracts::FakeNfc,
    )
    .map_err(|e| StoreError(e.to_string()))?;
    let (a, b) = two_accounts(&db).await?;
    conversation_query_contract(&SqliteMailStore::new(db), a, b).await?;
    let _ = std::fs::remove_dir_all(path.parent().unwrap_or(&path));
    Ok(())
}

#[tokio::test]
async fn file_store_with_reader_pool_passes_contract() -> Result<(), StoreError> {
    let path = temp_path("contract");
    let db = Database::open(
        &path,
        norelly_test_support::contact_import_contracts::FakeNfc,
    )
    .map_err(|e| StoreError(e.to_string()))?;
    let (a, b) = two_accounts(&db).await?;
    mail_store_contract(&SqliteMailStore::new(db), a, b).await?;
    let _ = std::fs::remove_dir_all(path.parent().unwrap_or(&path));
    Ok(())
}

#[tokio::test]
async fn deleting_an_account_removes_its_mailboxes_and_messages() -> Result<(), StoreError> {
    let db = Database::open_in_memory(norelly_test_support::contact_import_contracts::FakeNfc)
        .map_err(|e| StoreError(e.to_string()))?;
    let (a, _) = two_accounts(&db).await?;
    let store = SqliteMailStore::new(db.clone());
    let boxes = store
        .replace_mailboxes(
            a,
            &[RemoteMailbox {
                name: MailboxName::inbox(),
                delimiter: Some('/'),
                role: MailboxRole::Inbox,
                selectable: true,
            }],
        )
        .await?;
    let inbox = boxes
        .first()
        .map(|m| m.id)
        .ok_or_else(|| corrupted("boîte"))?;
    store
        .apply_sync(
            inbox,
            &SyncBatch {
                new: vec![sample_message(1, 10, false)],
                ..SyncBatch::default()
            },
        )
        .await?;
    SqliteAccountRepository::new(db.clone())
        .delete(a)
        .await
        .map_err(|e| StoreError(e.to_string()))?;
    let count: i64 = db
        .run(|c| {
            c.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
                .map_err(store_error)
        })
        .await?;
    assert_eq!(count, 0);
    assert!(store.tree(a).await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn bodies_roundtrip() -> Result<(), StoreError> {
    let db = Database::open_in_memory(norelly_test_support::contact_import_contracts::FakeNfc)
        .map_err(|e| StoreError(e.to_string()))?;
    let (a, _) = two_accounts(&db).await?;
    let store = SqliteMailStore::new(db);
    let inbox = store
        .replace_mailboxes(
            a,
            &[RemoteMailbox {
                name: MailboxName::inbox(),
                delimiter: None,
                role: MailboxRole::Inbox,
                selectable: true,
            }],
        )
        .await?
        .first()
        .map(|m| m.id)
        .ok_or_else(|| corrupted("boîte"))?;
    let blob = |c: char| BlobId::parse(&c.to_string().repeat(64)).ok_or_else(|| corrupted("blob"));
    let path = |p: &str| PartPath::parse(p).ok_or_else(|| corrupted("chemin"));
    let mut full = sample_message(1, 1, false);
    full.body = StoredBody::Full(blob('a')?);
    let mut partial = sample_message(2, 2, false);
    partial.body = StoredBody::Partial {
        header: blob('b')?,
        parts: vec![(path("1")?, blob('c')?)],
        deferred: vec![path("2")?],
    };
    store
        .apply_sync(
            inbox,
            &SyncBatch {
                new: vec![full.clone(), partial.clone(), sample_message(3, 3, false)],
                ..SyncBatch::default()
            },
        )
        .await?;
    assert_eq!(store.body_of(inbox, uid_of(1)?).await?, Some(full.body));
    assert_eq!(store.body_of(inbox, uid_of(2)?).await?, Some(partial.body));
    assert_eq!(
        store.body_of(inbox, uid_of(3)?).await?,
        Some(StoredBody::Missing)
    );
    assert_eq!(store.body_of(inbox, uid_of(4)?).await?, None);
    Ok(())
}

#[tokio::test]
async fn hand_edited_rows_are_revalidated() -> Result<(), StoreError> {
    let db = Database::open_in_memory(norelly_test_support::contact_import_contracts::FakeNfc)
        .map_err(|e| StoreError(e.to_string()))?;
    let (a, _) = two_accounts(&db).await?;
    let store = SqliteMailStore::new(db.clone());
    let inbox = store
        .replace_mailboxes(
            a,
            &[RemoteMailbox {
                name: MailboxName::inbox(),
                delimiter: None,
                role: MailboxRole::Inbox,
                selectable: true,
            }],
        )
        .await?
        .first()
        .map(|m| m.id)
        .ok_or_else(|| corrupted("boîte"))?;
    store
        .apply_sync(
            inbox,
            &SyncBatch {
                new: vec![sample_message(1, 1, false)],
                ..SyncBatch::default()
            },
        )
        .await?;
    db.run(|c| {
        c.execute(
            "UPDATE messages SET subject = 'a' || char(13, 10) || 'b' || char(8238), \
             keywords = '\\Seen bad(kw ok'",
            [],
        )
        .map(|_| ())
        .map_err(store_error)
    })
    .await?;
    let page = store.page(inbox, None, 10).await?;
    let item = page.items.first().ok_or_else(|| corrupted("ligne"))?;
    assert_eq!(item.subject, "a b");
    assert_eq!(
        item.flags
            .keywords()
            .iter()
            .map(Keyword::as_str)
            .collect::<Vec<_>>(),
        ["ok"]
    );
    // Rôle inconnu : signalé.
    db.run(|c| {
        c.execute("PRAGMA ignore_check_constraints = ON", [])
            .and_then(|_| c.execute("UPDATE mailboxes SET role = 'pirate'", []))
            .map(|_| ())
            .map_err(store_error)
    })
    .await?;
    assert!(store.tree(a).await.is_err());
    Ok(())
}

#[tokio::test]
async fn readers_see_committed_writes() -> Result<(), StoreError> {
    let path = temp_path("readers");
    let db = Database::open(
        &path,
        norelly_test_support::contact_import_contracts::FakeNfc,
    )
    .map_err(|e| StoreError(e.to_string()))?;
    let (a, _) = two_accounts(&db).await?;
    let store = SqliteMailStore::new(db);
    let inbox = store
        .replace_mailboxes(
            a,
            &[RemoteMailbox {
                name: MailboxName::inbox(),
                delimiter: None,
                role: MailboxRole::Inbox,
                selectable: true,
            }],
        )
        .await?
        .first()
        .map(|m| m.id)
        .ok_or_else(|| corrupted("boîte"))?;
    // Lectures concurrentes (plus que de lecteurs) pendant les écritures.
    let mut tasks = Vec::new();
    for n in 1..=20u32 {
        let store = store.clone();
        tasks.push(tokio::spawn(async move {
            store
                .apply_sync(
                    inbox,
                    &SyncBatch {
                        new: vec![sample_message(n, i64::from(n), false)],
                        ..SyncBatch::default()
                    },
                )
                .await?;
            store.page(inbox, None, 50).await
        }));
    }
    for t in tasks {
        let page = t.await.map_err(|e| StoreError(e.to_string()))??;
        assert!(!page.items.is_empty());
    }
    assert_eq!(store.page(inbox, None, 50).await?.items.len(), 20);
    let _ = std::fs::remove_dir_all(path.parent().unwrap_or(&path));
    Ok(())
}

#[tokio::test]
async fn record_all_is_all_or_nothing() -> Result<(), StoreError> {
    use norelly_domain::actions::{OpKind, PendingOp, QueueChange};
    use norelly_domain::mailbox::{MailboxName, Uid, UidValidity};
    use norelly_domain::ports::PendingOpStore;
    let db = Database::open_in_memory(norelly_test_support::contact_import_contracts::FakeNfc)
        .map_err(|e| StoreError(e.to_string()))?;
    let (a, _) = two_accounts(&db).await?;
    let store = SqliteMailStore::new(db);
    let op = |account: AccountId| -> Result<PendingOp, StoreError> {
        Ok(PendingOp {
            account,
            message: MessageId::new(1).ok_or_else(|| StoreError("id".into()))?,
            mailbox: MailboxName::parse("INBOX").map_err(|e| StoreError(e.to_string()))?,
            uid_validity: UidValidity::new(7).ok_or_else(|| StoreError("validité".into()))?,
            uid: Uid::new(1).ok_or_else(|| StoreError("uid".into()))?,
            kind: OpKind::Expunge,
            not_before: 0,
        })
    };
    // Second changement invalide (compte inexistant : clé étrangère).
    let ghost = AccountId::new(999).map_err(|e| StoreError(e.to_string()))?;
    let result = store
        .record_all(&[QueueChange::Push(op(a)?), QueueChange::Push(op(ghost)?)])
        .await;
    assert!(result.is_err(), "lot refusé : {result:?}");
    assert!(store.pending(a).await?.is_empty(), "rien n'est enfilé");
    Ok(())
}

/// Les compteurs de l'arbre des dossiers
/// se lisent dans un index couvrant, sans parcourir la table des messages.
#[tokio::test]
async fn folder_counts_use_a_covering_index() -> Result<(), StoreError> {
    let db = Database::open_in_memory(norelly_test_support::contact_import_contracts::FakeNfc)
        .map_err(|e| StoreError(e.to_string()))?;
    let plan = db
        .read_for_tests(|c| {
            let mut stmt = c
                .prepare(
                    "EXPLAIN QUERY PLAN SELECT COUNT(*) FROM messages WHERE mailbox_id = 1 \
                     AND hidden = 0 AND (flags & 9) = 0",
                )
                .map_err(crate::store_error)?;
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(3))
                .map_err(crate::store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(crate::store_error)?;
            Ok(rows.join(" | "))
        })
        .await?;
    assert!(plan.contains("COVERING INDEX"), "{plan}");
    Ok(())
}

/// Un verdict d'authentification modifié à la main (hors de `pass`,
/// `fail`, `unknown`) se relit `Unknown` : jamais de pastille, et la
/// conversation reste lisible.
#[tokio::test]
async fn a_hand_edited_sender_auth_reads_as_unknown() -> Result<(), StoreError> {
    use norelly_domain::conversation::ConversationScope;
    use norelly_domain::ports::ConversationQuery;
    use norelly_domain::sender_auth::SenderAuth;
    let db = Database::open_in_memory(norelly_test_support::contact_import_contracts::FakeNfc)
        .map_err(|e| StoreError(e.to_string()))?;
    let (a, _) = two_accounts(&db).await?;
    let store = SqliteMailStore::new(db.clone());
    let inbox = store
        .replace_mailboxes(
            a,
            &[RemoteMailbox {
                name: MailboxName::inbox(),
                delimiter: None,
                role: MailboxRole::Inbox,
                selectable: true,
            }],
        )
        .await?
        .first()
        .map(|m| m.id)
        .ok_or_else(|| corrupted("boîte"))?;
    let mut message = sample_message(1, 1, false);
    message.meta.headers.sender_auth = SenderAuth::Pass;
    store
        .apply_sync(
            inbox,
            &SyncBatch {
                new: vec![message],
                ..SyncBatch::default()
            },
        )
        .await?;
    db.run(|c| {
        c.execute("PRAGMA ignore_check_constraints = ON", [])
            .and_then(|_| c.execute("UPDATE messages SET sender_auth = 'verified'", []))
            .map(|_| ())
            .map_err(store_error)
    })
    .await?;
    let conversation = store
        .conversations(ConversationScope::Mailbox(inbox), None, 10)
        .await?
        .items
        .first()
        .map(|c| c.id)
        .ok_or_else(|| corrupted("conversation"))?;
    let verdicts: Vec<SenderAuth> = store
        .conversation_messages(conversation)
        .await?
        .iter()
        .map(|m| m.sender_auth)
        .collect();
    assert_eq!(verdicts, [SenderAuth::Unknown]);
    Ok(())
}

/// Les candidats d'une réutilisation de corps se lisent par l'index de
/// l'identifiant Gmail et de la taille : un serveur qui donne le même
/// identifiant à des milliers de messages de tailles différentes ne fait
/// pas parcourir ces milliers de lignes à chaque corps attendu.
#[tokio::test]
async fn body_reuse_candidates_are_found_by_gmail_id_and_size() -> Result<(), StoreError> {
    let db = Database::open_in_memory(norelly_test_support::contact_import_contracts::FakeNfc)
        .map_err(|e| StoreError(e.to_string()))?;
    let plan = db
        .read_for_tests(|c| {
            let mut stmt = c
                .prepare(&format!("EXPLAIN QUERY PLAN {}", super::REUSE_CANDIDATES))
                .map_err(crate::store_error)?;
            let rows = stmt
                .query_map([1_i64, 1, 500, 2, 8], |r| r.get::<_, String>(3))
                .map_err(crate::store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(crate::store_error)?;
            Ok(rows.join(" | "))
        })
        .await?;
    assert!(
        plan.contains("(gmail_msgid=? AND size=? AND body_state=?)"),
        "{plan}"
    );
    Ok(())
}

/// Un message qui attend une action locale (copie provisoire) ne reçoit
/// jamais un corps recopié : seule la synchronisation le complète.
#[tokio::test]
async fn a_message_waiting_for_an_action_gets_no_reused_body() -> Result<(), StoreError> {
    use norelly_domain::gmail::{BodyReuse, GmailMessageId};
    use norelly_domain::mailbox::MailboxRole;
    use norelly_test_support::mail_contracts::{blob_id, gmail_message, mailbox, state};
    let db = Database::open_in_memory(norelly_test_support::contact_import_contracts::FakeNfc)
        .map_err(|e| StoreError(e.to_string()))?;
    let (a, _) = two_accounts(&db).await?;
    let store = SqliteMailStore::new(db.clone());
    let boxes = store
        .replace_mailboxes(
            a,
            &[
                mailbox("[Gmail]/Tous les messages", MailboxRole::All, true),
                mailbox("Projets", MailboxRole::Normal, true),
            ],
        )
        .await?;
    let (all, projets) = match boxes.as_slice() {
        [x, y] => (x.id, y.id),
        _ => return Err(StoreError("boîtes".into())),
    };
    let batch = |message| SyncBatch {
        reset: false,
        removed: vec![],
        flag_changes: vec![],
        new: vec![message],
        state: state(1, 1, 100),
    };
    store
        .apply_sync(all, &batch(gmail_message(1, 42, 500, true)))
        .await?;
    store
        .apply_sync(projets, &batch(gmail_message(10, 42, 500, false)))
        .await?;
    let uid = |n| Uid::new(n).ok_or_else(|| StoreError("uid".into()));
    store
        .set_body(all, uid(1)?, &StoredBody::Full(blob_id(b"corps 42")?), None)
        .await?;
    db.write_for_tests("UPDATE messages SET pending_op = 7 WHERE uid = 10")
        .await?;
    let reuse = BodyReuse {
        account: a,
        gmail_id: GmailMessageId::new(42).ok_or_else(|| StoreError("identifiant".into()))?,
        size: 500,
    };
    assert!(!store.reuse_body(projets, uid(10)?, &reuse).await?);
    Ok(())
}

/// Une copie provisoire (message créé par une action locale en attente)
/// ne sert jamais de source à une réutilisation de corps, même si elle
/// porte le même identifiant Gmail et la même taille.
#[tokio::test]
async fn a_provisional_copy_is_never_a_body_source() -> Result<(), StoreError> {
    use norelly_domain::gmail::{BodyReuse, GmailMessageId};
    use norelly_domain::mailbox::MailboxRole;
    use norelly_test_support::mail_contracts::{blob_id, gmail_message, mailbox, state};
    let db = Database::open_in_memory(norelly_test_support::contact_import_contracts::FakeNfc)
        .map_err(|e| StoreError(e.to_string()))?;
    let (a, _) = two_accounts(&db).await?;
    let store = SqliteMailStore::new(db.clone());
    let boxes = store
        .replace_mailboxes(
            a,
            &[
                mailbox("[Gmail]/Tous les messages", MailboxRole::All, true),
                mailbox("Projets", MailboxRole::Normal, true),
            ],
        )
        .await?;
    let (all, projets) = match boxes.as_slice() {
        [x, y] => (x.id, y.id),
        _ => return Err(StoreError("boîtes".into())),
    };
    let batch = |message| SyncBatch {
        reset: false,
        removed: vec![],
        flag_changes: vec![],
        new: vec![message],
        state: state(1, 1, 100),
    };
    store
        .apply_sync(all, &batch(gmail_message(1, 42, 500, true)))
        .await?;
    store
        .apply_sync(projets, &batch(gmail_message(10, 42, 500, false)))
        .await?;
    let uid = |n| Uid::new(n).ok_or_else(|| StoreError("uid".into()));
    store
        .set_body(all, uid(1)?, &StoredBody::Full(blob_id(b"corps 42")?), None)
        .await?;
    db.write_for_tests("UPDATE messages SET pending_op = 7 WHERE uid = 1")
        .await?;
    let reuse = BodyReuse {
        account: a,
        gmail_id: GmailMessageId::new(42).ok_or_else(|| StoreError("identifiant".into()))?,
        size: 500,
    };
    assert!(!store.reuse_body(projets, uid(10)?, &reuse).await?);
    Ok(())
}

/// Un identifiant Gmail au-delà de `i64::MAX` (jusqu'à `u64::MAX`, valeur
/// qu'un serveur hostile peut envoyer) se relit tel quel, et sa
/// réutilisation de corps ne touche que le message visé.
#[tokio::test]
async fn gmail_ids_beyond_i64_round_trip_and_reuse_only_the_target() -> Result<(), StoreError> {
    use norelly_domain::gmail::{BodyReuse, GmailMessageId};
    use norelly_domain::mailbox::MailboxRole;
    use norelly_test_support::mail_contracts::{blob_id, gmail_message, mailbox, state};
    let db = Database::open_in_memory(norelly_test_support::contact_import_contracts::FakeNfc)
        .map_err(|e| StoreError(e.to_string()))?;
    let (a, _) = two_accounts(&db).await?;
    let store = SqliteMailStore::new(db.clone());
    let boxes = store
        .replace_mailboxes(
            a,
            &[
                mailbox("[Gmail]/Tous les messages", MailboxRole::All, true),
                mailbox("Projets", MailboxRole::Normal, true),
            ],
        )
        .await?;
    let (all, projets) = match boxes.as_slice() {
        [x, y] => (x.id, y.id),
        _ => return Err(StoreError("boîtes".into())),
    };
    let batch = |messages: Vec<StoredMessage>| SyncBatch {
        reset: false,
        removed: vec![],
        flag_changes: vec![],
        new: messages,
        state: state(1, 1, 100),
    };
    let big = u64::MAX;
    let just_over = (i64::MAX as u64) + 1;
    store
        .apply_sync(all, &batch(vec![gmail_message(1, big, 500, true)]))
        .await?;
    store
        .apply_sync(
            projets,
            &batch(vec![
                gmail_message(10, big, 500, false),
                gmail_message(11, just_over, 500, false),
            ]),
        )
        .await?;
    let ids: Vec<(u32, Option<u64>)> = store
        .missing_bodies(projets, 10)
        .await?
        .iter()
        .map(|p| (p.uid.get(), p.gmail_id.map(GmailMessageId::get)))
        .collect();
    assert_eq!(ids, [(11, Some(just_over)), (10, Some(big))]);

    let uid = |n| Uid::new(n).ok_or_else(|| StoreError("uid".into()));
    store
        .set_body(all, uid(1)?, &StoredBody::Full(blob_id(b"corps")?), None)
        .await?;
    let reuse = BodyReuse {
        account: a,
        gmail_id: GmailMessageId::new(big).ok_or_else(|| StoreError("identifiant".into()))?,
        size: 500,
    };
    assert!(store.reuse_body(projets, uid(10)?, &reuse).await?);
    let still_missing: Vec<u32> = store
        .missing_bodies(projets, 10)
        .await?
        .iter()
        .map(|p| p.uid.get())
        .collect();
    assert_eq!(still_missing, [11]);
    Ok(())
}
