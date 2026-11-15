use norelly_domain::address::EmailAddress;
use norelly_domain::capabilities::ServerCapabilities;
use norelly_domain::compose::draft::Draft;
use norelly_domain::compose::outbox::RETRY_BASE_SECS;
use norelly_domain::credentials::{Credentials, Secret, Username};
use norelly_domain::endpoint::{Host, Protocol, Security, ServerEndpoint};
use norelly_domain::fetch_policy::FetchPolicy;
use norelly_domain::ports::{RemoteMailboxSession, SendError};
use norelly_domain::security::PlaintextPolicy;
use norelly_render::compose::MimeComposer;
use norelly_test_support::compose_contracts::InMemoryComposeStore;
use norelly_test_support::fake_imap::FakeImap;
use norelly_test_support::fakes::{FakeMailServers, FakeSmtp};
use norelly_test_support::mail_contracts::{InMemoryBlobStore, InMemoryMailStore};

use super::*;
use crate::compose::{QueueMessage, SaveDraft};
use crate::sync::SyncAccount;

const NOW: i64 = 1_800_000_000;

type Worker = ComposeWorker<
    FakeMailServers,
    FakeSmtp,
    InMemoryComposeStore,
    InMemoryMailStore,
    InMemoryBlobStore,
    MimeComposer,
>;

fn account() -> AccountId {
    AccountId::new(1).unwrap_or_else(|_| panic!("compte"))
}

fn request(host: &str) -> ConnectRequest {
    ConnectRequest {
        endpoint: ServerEndpoint::new(
            Host::parse(host).unwrap_or_else(|_| panic!("hôte")),
            993,
            Security::Tls,
        )
        .unwrap_or_else(|_| panic!("serveur")),
        credentials: Credentials::new(
            Username::parse("moi").unwrap_or_else(|_| panic!("utilisateur")),
            Secret::new("x".into()),
        )
        .unwrap_or_else(|_| panic!("identifiants")),
        certificate_exceptions: vec![],
        plaintext: PlaintextPolicy::deny_all(),
    }
}

fn me() -> Recipient {
    Recipient::new(
        "Moi",
        EmailAddress::parse("moi@example.com").unwrap_or_else(|_| panic!("adresse")),
    )
}

fn draft() -> Draft {
    let mut d = Draft::empty(account());
    d.to = vec![Recipient::new(
        "",
        EmailAddress::parse("bob@example.com").unwrap_or_else(|_| panic!("adresse")),
    )];
    d.subject = "Bonjour".into();
    d.text = "Salut".into();
    d
}

/// Serveur avec Brouillons et Envoyés (UIDPLUS annoncé), boîtes
/// synchronisées.
async fn worker() -> (Worker, FakeImap) {
    let imap = FakeImap::new();
    imap.create_mailbox("Brouillons", &["\\Drafts"]);
    imap.create_mailbox("Envoyés", &["\\Sent"]);
    let servers = FakeMailServers::with_imap(imap.clone());
    let imap_request = request("imap.example.com");
    servers.set(
        &imap_request.endpoint,
        Ok(ServerCapabilities::from_imap(["IMAP4rev1", "UIDPLUS"])),
    );
    let mail = InMemoryMailStore::default();
    let blobs = InMemoryBlobStore::default();
    let mut session = imap.session().unwrap_or_else(|e| panic!("{e}"));
    let ext = session
        .enable_sync_extensions()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    SyncAccount {
        account: account(),
        store: &mail,
        blobs: &blobs,
        policy: FetchPolicy::default(),
        text: std::sync::Arc::new(norelly_render::search_text::MimeTextExtractor),
    }
    .sync_all(&mut session, ext, |_, _| {})
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    let worker = ComposeWorker {
        account: account(),
        from: me(),
        imap: imap_request,
        smtp: request("smtp.example.com"),
        connector: Arc::new(servers),
        sender: Arc::new(FakeSmtp::default()),
        store: Arc::new(InMemoryComposeStore::default()),
        folders: Arc::new(mail),
        blobs: Arc::new(blobs),
        composer: Arc::new(MimeComposer),
    };
    (worker, imap)
}

fn imap_connections(w: &Worker) -> usize {
    w.connector
        .requests()
        .iter()
        .filter(|(p, _)| *p == Protocol::Imap)
        .count()
}

async fn queue(w: &Worker) {
    QueueMessage {
        store: &*w.store,
        blobs: &*w.blobs,
        composer: &*w.composer,
    }
    .execute(None, &draft(), &me(), NOW)
    .await
    .unwrap_or_else(|e| panic!("{e}"));
}

#[tokio::test]
async fn nothing_to_do_opens_no_connection() {
    let (w, _) = worker().await;
    let tick = w.run_once(NOW).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(tick, Tick::default());
    assert_eq!(imap_connections(&w), 0);
}

#[tokio::test]
async fn a_queued_message_is_sent_then_filed() {
    let (w, imap) = worker().await;
    queue(&w).await;
    let tick = w.run_once(NOW).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(tick.delivery.sent.len(), 1);
    assert!(tick.imap_used);
    assert_eq!(tick.filed, 1);
    assert_eq!(tick.next_wake, None);
    assert_eq!(w.sender.accepted().len(), 1);
    assert_eq!(imap.flags_of("Envoyés").len(), 1);
    assert!(
        w.store
            .outbox(account())
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .is_empty()
    );
}

#[tokio::test]
async fn a_temporary_failure_schedules_the_next_attempt() {
    let (w, _) = worker().await;
    queue(&w).await;
    w.sender
        .reply(Err(SendError::Deferred("451 plus tard".into())));
    let tick = w.run_once(NOW).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(tick.next_wake, Some(NOW + RETRY_BASE_SECS));
    assert!(!tick.imap_used, "rien à déposer");
}

/// Identifiants refusés ou interrompus à l'envoi : les messages suivants
/// n'en présentent pas d'autres avant l'attente (ni dans la tournée, ni à
/// la suivante), puis partent.
#[tokio::test]
async fn a_credentials_failure_holds_the_queue() {
    use norelly_domain::auth::AuthError;
    use norelly_domain::compose::outbox::{CREDENTIALS_HOLD_SECS, credentials_hold};
    use norelly_domain::connection::ConnectionError;
    let (w, _) = worker().await;
    for _ in 0..3 {
        queue(&w).await;
    }
    w.sender
        .reply(Err(SendError::Connection(ConnectionError::Authentication(
            AuthError::Interrupted,
        ))));
    let tick = w
        .run_once_after(NOW, Holds::default())
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(tick.delivery.failed.len(), 1);
    let hold = credentials_hold(tick.delivery.failed.iter().map(|(_, f)| f), NOW);
    assert_eq!(hold, Some(NOW + CREDENTIALS_HOLD_SECS));
    let tick = w
        .run_once_after(
            NOW + 1,
            Holds {
                send: hold,
                imap: None,
            },
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(tick.delivery.failed.is_empty() && tick.delivery.sent.is_empty());
    assert!(
        w.sender.accepted().is_empty(),
        "rien envoyé pendant l'attente"
    );
    assert_eq!(tick.next_wake, hold);
    let tick = w
        .run_once_after(
            NOW + CREDENTIALS_HOLD_SECS,
            Holds {
                send: hold,
                imap: None,
            },
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(tick.delivery.sent.len(), 2);
}

#[tokio::test]
async fn drafts_are_copied_and_the_next_copy_is_scheduled() {
    let (w, imap) = worker().await;
    let save = SaveDraft { drafts: &*w.store };
    let saved = save
        .execute(None, &draft(), NOW)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let tick = w.run_once(NOW).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(tick.drafts_saved, 1);
    assert_eq!(imap.flags_of("Brouillons").len(), 1);
    assert_eq!(tick.next_wake, None, "à jour sur le serveur");
    // Modifié juste après : copie à l'échéance de l'intervalle.
    save.execute(Some(saved.id), &draft(), NOW + 1)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let tick = w.run_once(NOW + 1).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(tick.drafts_saved, 0);
    assert!(!tick.imap_used, "trop tôt : pas de connexion");
    assert_eq!(tick.next_wake, Some(NOW + REMOTE_SAVE_INTERVAL_SECS));
}

/// Travailleur lancé : après des identifiants interrompus, un réveil
/// ordinaire (brouillon enregistré) n'envoie rien ; une action de
/// l'utilisateur (envoi, « Réessayer ») lève l'attente.
#[tokio::test]
async fn only_a_user_action_lifts_the_credentials_hold() {
    use norelly_domain::auth::AuthError;
    use norelly_domain::connection::ConnectionError;
    let (w, _) = worker().await;
    let store = Arc::clone(&w.store);
    let sender = Arc::clone(&w.sender);
    for _ in 0..2 {
        QueueMessage {
            store: &*store,
            blobs: &*w.blobs,
            composer: &*w.composer,
        }
        .execute(None, &draft(), &me(), unix_now())
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    }
    sender.reply(Err(SendError::Connection(ConnectionError::Authentication(
        AuthError::Interrupted,
    ))));
    let failed = || async {
        store
            .outbox(account())
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .iter()
            .any(|i| matches!(i.entry.state, OutboxState::Failed(_)))
    };
    let handle = spawn(w);
    // Premier passage : un échec définitif, la tournée s'arrête.
    tokio::time::timeout(Duration::from_secs(5), async {
        while !failed().await {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("aucun échec"));
    handle.trigger();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        sender.accepted().is_empty(),
        "réveil ordinaire : rien envoyé"
    );
    handle.retry_now();
    tokio::time::timeout(Duration::from_secs(5), async {
        while sender.accepted().is_empty() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("rien envoyé après « Réessayer »"));
    handle.shutdown().await;
}

/// Pendant l'attente qui suit des identifiants IMAP refusés, aucun
/// travail distant (copie dans « Envoyés », brouillons) n'ouvre de
/// connexion ; l'attente passée, il reprend.
#[tokio::test]
async fn an_imap_hold_skips_the_remote_work() {
    let (w, _) = worker().await;
    queue(&w).await;
    let held = Holds {
        send: None,
        imap: Some(NOW + 100),
    };
    let tick = w
        .run_once_after(NOW, held)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(tick.delivery.sent.len(), 1, "l'envoi n'est pas concerné");
    assert!(!tick.imap_used);
    assert_eq!(imap_connections(&w), 0);
    assert_eq!(tick.next_wake, Some(NOW + 100));
    let tick = w
        .run_once_after(NOW + 100, held)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(tick.filed, 1);
}

/// Travailleur lancé, serveur de réception qui coupe après `LOGIN` :
/// les réveils ordinaires (brouillon enregistré toutes les deux
/// secondes) ne renvoient pas le mot de passe.
#[tokio::test]
async fn interrupted_imap_credentials_are_not_replayed_on_every_wake() {
    use norelly_domain::auth::AuthError;
    use norelly_domain::connection::ConnectionError;
    let (w, _) = worker().await;
    let store = Arc::clone(&w.store);
    QueueMessage {
        store: &*store,
        blobs: &*w.blobs,
        composer: &*w.composer,
    }
    .execute(None, &draft(), &me(), unix_now())
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    let servers = Arc::clone(&w.connector);
    servers.set(
        &w.imap.endpoint,
        Err(ConnectionError::Authentication(AuthError::Interrupted)),
    );
    let handle = spawn(w);
    let imap = || {
        servers
            .requests()
            .iter()
            .filter(|(p, _)| *p == Protocol::Imap)
            .count()
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        while imap() == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("aucune connexion IMAP"));
    for _ in 0..5 {
        handle.trigger();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(imap(), 1, "mot de passe IMAP renvoyé");
    handle.shutdown().await;
}

#[tokio::test]
async fn spawned_worker_recovers_then_sends_on_trigger() {
    let (w, _) = worker().await;
    let store = Arc::clone(&w.store);
    let sender = Arc::clone(&w.sender);
    let handle = spawn(w);
    let mut events = handle.subscribe();
    QueueMessage {
        store: &*store,
        blobs: &InMemoryBlobStore::default(),
        composer: &MimeComposer,
    }
    .execute(None, &draft(), &me(), unix_now())
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    handle.trigger();
    let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap_or_else(|_| panic!("aucun événement"))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(event, ComposeEvent::OutboxChanged { account: account() });
    assert_eq!(sender.accepted().len(), 1);
    handle.shutdown().await;
}
