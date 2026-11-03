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
    }
    .sync_all(&mut session, ext, |_| {})
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
