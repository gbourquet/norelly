//! Messages Gmail présents dans plusieurs dossiers, de bout en bout : vraie
//! base SQLite, vrais fichiers de messages, contre le faux serveur IMAP de
//! `test-support` (Dovecot n'annonce pas `X-GM-EXT-1` ; aucun conteneur).

use std::sync::Arc;

use norelly_application::sync::SyncAccount;
use norelly_blobs::FsBlobStore;
use norelly_domain::fetch_policy::FetchPolicy;
use norelly_domain::mailbox::{Flags, Uid};
use norelly_domain::ports::{AccountRepository, FolderTreeQuery, RemoteMailboxSession};
use norelly_render::search_text::MimeTextExtractor;
use norelly_sqlite::Database;
use norelly_sqlite::SqliteAccountRepository;
use norelly_sqlite::mail::SqliteMailStore;
use norelly_test_support::contracts::sample_account;
use norelly_test_support::fake_imap::FakeImap;

fn raw(n: u32) -> Vec<u8> {
    format!(
        "From: Expéditeur {n} <e{n}@example.com>\r\nSubject: Message {n}\r\n\
         Message-ID: <{n}@example.com>\r\n\r\nCorps du message {n}.\r\n"
    )
    .into_bytes()
}

/// Bilan d'une première synchronisation : corps téléchargés, corps
/// enregistrés dans « Projets » et « Tous les messages » pour le message 1,
/// drapeaux de chaque dossier.
struct Outcome {
    body_fetches: usize,
    same_body: bool,
    inbox_seen: bool,
    all_seen: bool,
}

/// Le message 1 est dans « Projets » (lu) et dans « Tous les messages »
/// (non lu) sous le même identifiant Gmail ; le message 2 n'est que dans
/// « Tous les messages ».
async fn first_sync(gmail: bool) -> Outcome {
    let dir =
        std::env::temp_dir().join(format!("norelly-gmail-e2e-{gmail}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let db = Database::open(&dir.join("norelly.sqlite3"), norelly_contacts_io::IcuNfc)
        .unwrap_or_else(|e| panic!("{e}"));
    let account = SqliteAccountRepository::new(db.clone())
        .insert(&sample_account("gmail@example.com", false))
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .id();
    let store = SqliteMailStore::new(db);
    let blobs = FsBlobStore::new(dir.join("accounts"));

    let imap = FakeImap::new();
    imap.set_gmail(gmail);
    imap.create_mailbox("Projets", &[]);
    imap.create_mailbox("[Gmail]/Tous les messages", &["\\All"]);
    let seen = Flags::from_tokens(["\\Seen"]);
    for (mailbox, n, flags, gmail_id) in [
        ("Projets", 1, seen, 1_278_455_344_230_334_865),
        (
            "[Gmail]/Tous les messages",
            1,
            Flags::default(),
            1_278_455_344_230_334_865,
        ),
        ("[Gmail]/Tous les messages", 2, Flags::default(), 7),
    ] {
        imap.append_gmail(mailbox, &raw(n), flags, 1_700_000_000, gmail_id)
            .unwrap_or_else(|| panic!("{mailbox}"));
    }

    let mut session = imap.session().unwrap_or_else(|e| panic!("{e}"));
    let ext = session
        .enable_sync_extensions()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    SyncAccount {
        account,
        store: &store,
        blobs: &blobs,
        policy: FetchPolicy::default(),
        text: Arc::new(MimeTextExtractor),
    }
    .sync_all(&mut session, ext, |_, _| {})
    .await
    .unwrap_or_else(|e| panic!("{e}"));

    let tree = store.tree(account).await.unwrap_or_else(|e| panic!("{e}"));
    let id = |name: &str| {
        tree.iter()
            .find(|n| n.name.as_str() == name)
            .unwrap_or_else(|| panic!("{name}"))
    };
    let (projets, all) = (id("Projets"), id("[Gmail]/Tous les messages"));
    let first = Uid::new(1).unwrap_or_else(|| panic!("uid"));
    let mut bodies = Vec::new();
    for mailbox in [projets.id, all.id] {
        bodies.push(
            store
                .body_of(mailbox, first)
                .await
                .unwrap_or_else(|e| panic!("{e}")),
        );
    }
    let (a, b) = (bodies[0].clone(), bodies[1].clone());
    assert!(
        a.is_some() && b.is_some(),
        "corps enregistrés : {a:?} {b:?}"
    );
    let outcome = Outcome {
        body_fetches: imap.body_fetches(),
        same_body: a == b,
        inbox_seen: projets.unread == 0,
        all_seen: all.unread == 0,
    };
    let _ = std::fs::remove_dir_all(&dir);
    outcome
}

#[tokio::test]
async fn gmail_message_in_two_folders_is_downloaded_once() {
    let outcome = first_sync(true).await;
    assert_eq!(outcome.body_fetches, 2, "un téléchargement par message");
    assert!(outcome.same_body);
    assert!(outcome.inbox_seen, "lu dans « Projets »");
    assert!(!outcome.all_seen, "non lu dans « Tous les messages »");
}

#[tokio::test]
async fn without_the_gmail_extension_every_copy_is_downloaded() {
    let outcome = first_sync(false).await;
    assert_eq!(outcome.body_fetches, 3);
    assert!(outcome.same_body, "même contenu, même empreinte");
    assert!(outcome.inbox_seen);
    assert!(!outcome.all_seen);
}
