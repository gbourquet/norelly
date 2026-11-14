use std::time::Duration;

use norelly_domain::credentials::{Credentials, Secret, Username};
use norelly_domain::endpoint::{Host, Security, ServerEndpoint};
use norelly_domain::mailbox::{Flags, FolderChangeError};
use norelly_domain::ports::{FolderTreeQuery, MailStore};
use norelly_domain::security::PlaintextPolicy;
use norelly_test_support::fake_imap::FakeImap;
use norelly_test_support::fakes::FakeMailServers;
use norelly_test_support::mail_contracts::{InMemoryBlobStore, InMemoryMailStore};

use super::*;

fn account() -> AccountId {
    AccountId::new(1).unwrap_or_else(|_| panic!("id"))
}

fn request() -> ConnectRequest {
    ConnectRequest {
        endpoint: ServerEndpoint::new(
            Host::parse("imap.example.com").unwrap_or_else(|_| panic!("hôte")),
            993,
            Security::Tls,
        )
        .unwrap_or_else(|_| panic!("serveur")),
        credentials: Credentials::new(
            Username::parse("alice").unwrap_or_else(|_| panic!("utilisateur")),
            Secret::new("secret".into()),
        )
        .unwrap_or_else(|_| panic!("identifiants")),
        certificate_exceptions: vec![],
        plaintext: PlaintextPolicy::deny_all(),
    }
}

fn config() -> ActorConfig {
    ActorConfig {
        idle_renewal: Duration::from_millis(200),
        full_sync_every: Duration::from_secs(3600),
        backoff: Backoff::new(20, 200),
        policy: FetchPolicy::default(),
    }
}

struct Running {
    imap: FakeImap,
    store: Arc<InMemoryMailStore>,
    handle: AccountHandle,
    events: broadcast::Receiver<SyncEvent>,
}

fn start(imap: &FakeImap) -> Running {
    let store = Arc::new(InMemoryMailStore::default());
    let handle = spawn(
        account(),
        request(),
        Arc::new(FakeMailServers::with_imap(imap.clone())),
        Arc::clone(&store),
        Arc::new(InMemoryBlobStore::default()),
        Arc::new(norelly_render::search_text::MimeTextExtractor),
        config(),
    );
    let events = handle.subscribe();
    Running {
        imap: imap.clone(),
        store,
        handle,
        events,
    }
}

/// Attend un événement satisfaisant `pred`, au plus `within`.
async fn wait_for(
    events: &mut broadcast::Receiver<SyncEvent>,
    within: Duration,
    pred: impl Fn(&SyncEvent) -> bool,
) -> SyncEvent {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Ok(e)) if pred(&e) => return e,
            Ok(Ok(_) | Err(broadcast::error::RecvError::Lagged(_))) => {}
            Ok(Err(e)) => panic!("canal fermé : {e}"),
            Err(_) => panic!("aucun événement attendu dans {within:?}"),
        }
    }
}

fn online(e: &SyncEvent) -> bool {
    matches!(e, SyncEvent::Online { .. })
}

fn offline(e: &SyncEvent) -> bool {
    matches!(e, SyncEvent::Offline { .. })
}

fn folder_changed(e: &SyncEvent) -> bool {
    matches!(e, SyncEvent::FolderChanged { .. })
}

async fn inbox_count(r: &Running) -> usize {
    let tree = r
        .store
        .tree(account())
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let Some(inbox) = tree.iter().find(|n| n.name.is_inbox()) else {
        return 0;
    };
    r.store
        .local_mailbox(inbox.id)
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .messages
        .len()
}

async fn wait_inbox(r: &Running, expected: usize, within: Duration) {
    let deadline = tokio::time::Instant::now() + within;
    while inbox_count(r).await != expected {
        assert!(
            tokio::time::Instant::now() < deadline,
            "boîte de réception : {} messages au lieu de {expected}",
            inbox_count(r).await
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

const WAIT: Duration = Duration::from_secs(5);

#[tokio::test]
async fn initial_sync_then_idle_brings_new_messages() {
    let imap = FakeImap::new();
    imap.append("INBOX", b"Subject: un\r\n\r\nx", Flags::default(), 1);
    let mut r = start(&imap);
    wait_for(&mut r.events, WAIT, online).await;
    wait_inbox(&r, 1, WAIT).await;

    // Message ajouté côté serveur : reçu par IDLE en moins de 5 s.
    let started = tokio::time::Instant::now();
    r.imap
        .append("INBOX", b"Subject: deux\r\n\r\ny", Flags::default(), 2);
    wait_for(&mut r.events, WAIT, folder_changed).await;
    wait_inbox(&r, 2, WAIT).await;
    assert!(started.elapsed() < WAIT);
    r.handle.shutdown().await;
}

#[tokio::test]
async fn idle_is_renewed_without_spurious_changes() {
    let imap = FakeImap::new();
    let mut r = start(&imap);
    wait_for(&mut r.events, WAIT, online).await;
    // Laisse passer la synchronisation de rattrapage de la connexion IDLE.
    tokio::time::sleep(Duration::from_millis(100)).await;
    while r.events.try_recv().is_ok() {}
    // Plusieurs renouvellements (200 ms) sans changement : aucun
    // événement de dossier ni de coupure.
    tokio::time::sleep(Duration::from_millis(900)).await;
    while let Ok(e) = r.events.try_recv() {
        assert!(!folder_changed(&e) && !offline(&e), "{e:?}");
    }
    r.imap
        .append("INBOX", b"Subject: x\r\n\r\n", Flags::default(), 1);
    wait_inbox(&r, 1, WAIT).await;
    r.handle.shutdown().await;
}

#[tokio::test]
async fn reconnects_with_backoff_and_converges() {
    let imap = FakeImap::new();
    imap.set_offline(true);
    let mut r = start(&imap);
    // Hors ligne : tentatives espacées, plafonnées.
    let event = wait_for(&mut r.events, WAIT, offline).await;
    let SyncEvent::Offline { retry_in_ms, .. } = event else {
        panic!("{event:?}");
    };
    assert!(retry_in_ms.is_some_and(|delay| delay <= 200));
    // Changements pendant la panne, puis retour du serveur.
    r.imap
        .append("INBOX", b"Subject: a\r\n\r\n", Flags::default(), 1);
    r.imap.set_offline(false);
    wait_for(&mut r.events, WAIT, online).await;
    wait_inbox(&r, 1, WAIT).await;

    // Coupure des sessions ouvertes pendant le fonctionnement.
    r.imap.set_offline(true);
    wait_for(&mut r.events, WAIT, offline).await;
    r.imap
        .append("INBOX", b"Subject: b\r\n\r\n", Flags::default(), 2);
    r.imap.set_offline(false);
    wait_inbox(&r, 2, WAIT).await;
    r.handle.shutdown().await;
}

#[tokio::test]
async fn provider_missing_from_this_version_stops_the_actor_without_retrying() {
    use norelly_domain::auth::AuthError;
    let imap = FakeImap::new();
    let refused = ConnectionError::Authentication(AuthError::ProviderNotConfigured);
    imap.set_connect_error(Some(refused.clone()));
    let mut r = start(&imap);
    let event = wait_for(&mut r.events, WAIT, offline).await;
    let SyncEvent::Offline {
        failure,
        retry_in_ms,
        ..
    } = event
    else {
        panic!("{event:?}");
    };
    assert_eq!(failure, SyncFailure::Connection(refused.clone()));
    assert_eq!(retry_in_ms, None, "aucun nouvel essai annoncé");
    // Un seul essai par connexion de l'acteur (commandes et surveillance),
    // puis plus rien : l'attente plafonnée à 200 ms en aurait fait d'autres.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let attempts = r.imap.connect_attempts();
    assert!(attempts <= 2, "{attempts} essais");
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(r.imap.connect_attempts(), attempts, "aucun nouvel essai");
    // Une demande adressée à l'acteur arrêté rend la cause, pas une coupure.
    let fetched = r
        .handle
        .fetch_body(
            MailboxName::inbox(),
            Uid::new(1).unwrap_or_else(|| panic!("uid")),
            BodyFetch::Full,
        )
        .await;
    assert_eq!(fetched.map(|_| ()), Err(refused));
    r.handle.shutdown().await;
}

/// Événements jusqu'à la fin du tour complet (progression `done == total`).
async fn until_synced(r: &mut Running) -> Vec<SyncEvent> {
    let mut seen = Vec::new();
    loop {
        let event = wait_for(&mut r.events, WAIT, |_| true).await;
        let end = matches!(event, SyncEvent::SyncProgress { done, total, .. } if done == total);
        seen.push(event);
        if end {
            return seen;
        }
    }
}

fn refresh(e: &SyncEvent) -> bool {
    matches!(
        e,
        SyncEvent::FoldersChanged { .. } | SyncEvent::FolderChanged { .. }
    )
}

#[tokio::test]
async fn first_sync_reports_progress_and_shows_folders_and_messages_before_the_end() {
    let imap = FakeImap::new();
    imap.create_mailbox("Archives", &[]);
    imap.append("INBOX", b"Subject: a\r\n\r\n", Flags::default(), 1);
    imap.append("Archives", b"Subject: b\r\n\r\n", Flags::default(), 2);
    let mut r = start(&imap);
    let seen = until_synced(&mut r).await;
    let position = |pred: &dyn Fn(&SyncEvent) -> bool| seen.iter().position(pred);
    let started = position(&|e| matches!(e, SyncEvent::SyncProgress { done: 0, .. }));
    let Some(started) = started else {
        panic!("progression non annoncée : {seen:?}");
    };
    assert!(
        seen[..started]
            .iter()
            .any(|e| matches!(e, SyncEvent::FoldersChanged { .. })),
        "dossiers affichés dès le début : {seen:?}"
    );
    let last = seen.len() - 1;
    let ends_of_mailboxes = position(&|e| matches!(e, SyncEvent::FolderChanged { .. }));
    assert!(
        seen[started + 1..last].iter().any(refresh),
        "messages affichés avant la fin : {seen:?}"
    );
    // La fin n'est annoncée qu'après la mise à jour finale des dossiers.
    assert!(
        ends_of_mailboxes.is_some_and(|c| c < last),
        "fin annoncée en dernier : {seen:?}"
    );
    assert_eq!(
        seen.last().map(|e| matches!(
            e,
            SyncEvent::SyncProgress {
                done: 2,
                total: 2,
                ..
            }
        )),
        Some(true)
    );
    // Rien ne suit la fin annoncée : l'écran est déjà à jour.
    let after = tokio::time::timeout(Duration::from_millis(300), async {
        loop {
            match r.events.recv().await {
                Ok(e) if refresh(&e) => return e,
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(e) => panic!("canal fermé : {e}"),
            }
        }
    })
    .await;
    assert!(
        after.is_err(),
        "mise à jour après la fin annoncée : {after:?}"
    );
    r.handle.shutdown().await;
}

// Horloge en pause : tout le tour se passe dans le même instant.
#[tokio::test(start_paused = true)]
async fn refreshes_during_the_sync_are_grouped() {
    let imap = FakeImap::new();
    // Trois lots d'en-têtes (500 par lot) dans la même boîte.
    for n in 1..=1_200_i64 {
        imap.append(
            "INBOX",
            format!("Subject: m{n}\r\n\r\n").as_bytes(),
            Flags::default(),
            n,
        );
    }
    let mut r = start(&imap);
    let seen = until_synced(&mut r).await;
    let started = seen
        .iter()
        .position(|e| matches!(e, SyncEvent::SyncProgress { done: 0, .. }))
        .unwrap_or_else(|| panic!("progression non annoncée : {seen:?}"));
    // Avant la rafale de fin (une mise à jour par boîte modifiée) : le
    // premier lot, puis la fin des en-têtes (lots écartés), pas un par lot.
    let during = seen[started..]
        .iter()
        .take_while(|e| !matches!(e, SyncEvent::FolderChanged { .. }))
        .filter(|e| refresh(e))
        .count();
    assert_eq!(during, 2, "signaux pendant le tour : {seen:?}");
    r.handle.shutdown().await;
}

#[tokio::test]
async fn sync_now_and_shutdown() {
    let imap = FakeImap::new();
    imap.create_mailbox("Archives", &[]);
    let mut r = start(&imap);
    wait_for(&mut r.events, WAIT, online).await;
    // Pas d'IDLE sur Archives : la synchronisation est demandée.
    r.imap
        .append("Archives", b"Subject: a\r\n\r\n", Flags::default(), 1);
    r.handle.sync_now();
    wait_for(&mut r.events, WAIT, folder_changed).await;
    // L'arrêt se termine rapidement, même en plein IDLE.
    tokio::time::timeout(Duration::from_secs(2), r.handle.shutdown())
        .await
        .unwrap_or_else(|_| panic!("arrêt trop long"));
}

#[tokio::test]
async fn shutdown_interrupts_a_long_sync() {
    let imap = FakeImap::new();
    for n in 0..300 {
        imap.append("INBOX", b"Subject: x\r\n\r\n", Flags::default(), n);
    }
    // 300 corps à 20 ms l'un : la première synchronisation dure ~6 s.
    imap.set_delay(Duration::from_millis(20));
    let r = start(&imap);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let started = tokio::time::Instant::now();
    r.handle.shutdown().await;
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "arrêt en {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn bodies_are_fetched_on_demand() {
    let imap = FakeImap::new();
    let raw = b"Subject: pj\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\n\
Content-Type: text/plain\r\n\r\ntexte\r\n--b\r\nContent-Type: application/pdf\r\n\r\nPDF\r\n--b--\r\n";
    let uid = imap
        .append("INBOX", raw, Flags::default(), 1)
        .unwrap_or_else(|| panic!("uid"));
    let mut r = start(&imap);
    wait_for(&mut r.events, WAIT, online).await;
    let two =
        norelly_domain::fetch_policy::PartPath::parse("2").unwrap_or_else(|| panic!("chemin"));
    let fetched = tokio::time::timeout(
        WAIT,
        r.handle.fetch_body(
            MailboxName::inbox(),
            uid,
            BodyFetch::Partial {
                parts: vec![two.clone()],
                deferred: vec![],
            },
        ),
    )
    .await
    .unwrap_or_else(|_| panic!("délai"));
    let Ok(Some(FetchedBody::Partial { parts, .. })) = fetched else {
        panic!("partie attendue : {fetched:?}");
    };
    let entity = parts
        .into_iter()
        .find(|(p, _)| *p == two)
        .map(|(_, b)| b)
        .unwrap_or_default();
    assert!(entity.starts_with(b"Content-Type: application/pdf"));
    assert!(entity.ends_with(b"PDF"));
    // Message disparu : None.
    let gone = r
        .handle
        .fetch_body(
            MailboxName::inbox(),
            Uid::new(999).unwrap_or_else(|| panic!("uid")),
            BodyFetch::Full,
        )
        .await;
    assert_eq!(gone, Ok(None));
    r.handle.shutdown().await;
}

/// Attend que `cond` soit vrai, au plus `within`.
async fn eventually(within: Duration, what: &str, cond: impl Fn() -> bool) {
    let deadline = tokio::time::Instant::now() + within;
    while !cond() {
        assert!(tokio::time::Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn offline_actions_are_replayed_on_reconnection() {
    use crate::actions::PerformAction;
    use norelly_domain::actions::MessageAction;
    use norelly_domain::ports::{MessageListQuery, PendingOpStore};

    let imap = FakeImap::new();
    imap.create_mailbox("Corbeille", &["\\Trash"]);
    imap.append("INBOX", b"Subject: a\r\n\r\nx", Flags::default(), 1);
    imap.append("INBOX", b"Subject: b\r\n\r\ny", Flags::default(), 2);
    let mut r = start(&imap);
    wait_for(&mut r.events, WAIT, online).await;
    wait_inbox(&r, 2, WAIT).await;

    // Hors ligne : action appliquée localement, enfilée.
    r.imap.set_offline(true);
    wait_for(&mut r.events, WAIT, offline).await;
    let tree = r
        .store
        .tree(account())
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let inbox = tree
        .iter()
        .find(|n| n.name.is_inbox())
        .map(|n| n.id)
        .unwrap_or_else(|| panic!("INBOX"));
    let page = r
        .store
        .page(inbox, None, 10)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let ids: Vec<_> = page.items.iter().map(|i| i.id).collect();
    // Fenêtre d'annulation déjà écoulée : l'opération est due.
    let report = PerformAction { store: &*r.store }
        .execute(&ids, MessageAction::Delete, super::unix_now() - 60)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(report.recorded.len(), 2);
    r.handle.replay_soon();
    assert_eq!(r.imap.flags_of("INBOX").len(), 2, "rien n'est parti");

    // Retour du serveur : rejoué à la reconnexion.
    r.imap.set_offline(false);
    let imap = r.imap.clone();
    eventually(WAIT, "messages déplacés vers la Corbeille", || {
        imap.flags_of("INBOX").is_empty() && imap.flags_of("Corbeille").len() == 2
    })
    .await;
    let deadline = tokio::time::Instant::now() + WAIT;
    while !r
        .store
        .pending(account())
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .is_empty()
    {
        assert!(tokio::time::Instant::now() < deadline, "file vidée");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    r.handle.shutdown().await;
}

#[tokio::test]
async fn actions_wait_for_the_undo_window_then_replay_without_request() {
    use crate::actions::PerformAction;
    use norelly_domain::actions::MessageAction;
    use norelly_domain::ports::MessageListQuery;

    let imap = FakeImap::new();
    imap.append("INBOX", b"Subject: a\r\n\r\nx", Flags::default(), 1);
    let mut r = start(&imap);
    wait_for(&mut r.events, WAIT, online).await;
    wait_inbox(&r, 1, WAIT).await;
    let tree = r
        .store
        .tree(account())
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let inbox = tree
        .iter()
        .find(|n| n.name.is_inbox())
        .map(|n| n.id)
        .unwrap_or_else(|| panic!("INBOX"));
    let id = r
        .store
        .page(inbox, None, 10)
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .items
        .first()
        .map(|i| i.id)
        .unwrap_or_else(|| panic!("message"));
    // Due dans deux secondes (fenêtre d'annulation presque écoulée).
    let now = super::unix_now() - norelly_domain::actions::UNDO_WINDOW_SECS + 2;
    PerformAction { store: &*r.store }
        .execute(&[id], MessageAction::MarkSeen(true), now)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    r.handle.replay_soon();
    let uid1 = Uid::new(1).unwrap_or_else(|| panic!("uid"));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !r.imap.flags_of("INBOX")[&uid1].seen,
        "pas avant l'échéance"
    );
    let imap = r.imap.clone();
    eventually(Duration::from_secs(5), "rejoué à l'échéance", || {
        imap.flags_of("INBOX")[&uid1].seen
    })
    .await;
    r.handle.shutdown().await;
}

/// Message déplacé vers un dossier, puis ce dossier déplacé avant que
/// l'action soit rejouée : le message arrive dans le dossier à sa nouvelle
/// place, il ne reste pas dans son dossier d'origine.
#[tokio::test]
async fn pending_move_follows_a_folder_moved_before_replay() {
    use crate::actions::PerformAction;
    use norelly_domain::actions::MessageAction;
    use norelly_domain::ports::MessageListQuery;

    let imap = FakeImap::new();
    imap.create_mailbox("Bla", &[]);
    imap.create_mailbox("Bli", &[]);
    imap.append("INBOX", b"Subject: a\r\n\r\nx", Flags::default(), 1);
    let mut r = start(&imap);
    wait_for(&mut r.events, WAIT, online).await;
    wait_inbox(&r, 1, WAIT).await;
    let tree = r
        .store
        .tree(account())
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let id_of = |name: &str| {
        tree.iter()
            .find(|n| n.name.as_str() == name)
            .map(|n| n.id)
            .unwrap_or_else(|| panic!("{name}"))
    };
    let (inbox, bli) = (id_of("INBOX"), id_of("Bli"));
    let id = r
        .store
        .page(inbox, None, 10)
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .items
        .first()
        .map(|i| i.id)
        .unwrap_or_else(|| panic!("message"));
    // Vers « Bli », rejoué dans deux secondes (fenêtre d'annulation).
    let now = super::unix_now() - norelly_domain::actions::UNDO_WINDOW_SECS + 2;
    PerformAction { store: &*r.store }
        .execute(&[id], MessageAction::Move(bli), now)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    // « Bli » rangé dans « Bla » avant l'échéance.
    let name = |raw: &str| MailboxName::parse(raw).unwrap_or_else(|e| panic!("{e}"));
    let moved = tokio::time::timeout(
        WAIT,
        r.handle
            .client()
            .move_folder(name("Bli"), Some(name("Bla"))),
    )
    .await
    .unwrap_or_else(|_| panic!("délai"));
    assert_eq!(moved, Ok(name("Bla/Bli")));
    r.handle.replay_soon();
    let imap = r.imap.clone();
    eventually(
        Duration::from_secs(8),
        "message arrivé dans Bla/Bli",
        || imap.flags_of("Bla/Bli").len() == 1,
    )
    .await;
    assert!(r.imap.flags_of("INBOX").is_empty(), "plus dans INBOX");
    r.handle.shutdown().await;
}

/// Un message non lu arrivé par IDLE est signalé (notification),
/// pas ceux de la première synchronisation.
#[tokio::test]
async fn unread_arrivals_in_the_inbox_are_signalled() {
    let imap = FakeImap::new();
    imap.append("INBOX", b"Subject: ancien\r\n\r\nx", Flags::default(), 1);
    let mut r = start(&imap);
    wait_for(&mut r.events, WAIT, online).await;
    wait_inbox(&r, 1, WAIT).await;
    r.imap.append(
        "INBOX",
        b"From: Bob <bob@example.com>\r\nSubject: nouveau\r\n\r\ny",
        Flags::default(),
        2,
    );
    let event = wait_for(&mut r.events, WAIT, |e| {
        matches!(e, SyncEvent::NewMail { .. })
    })
    .await;
    let SyncEvent::NewMail { mail, .. } = event else {
        panic!("{event:?}");
    };
    assert_eq!(mail.count, 1);
    assert_eq!(mail.latest.map(|l| l.subject), Some("nouveau".to_owned()));
    r.handle.shutdown().await;
}

#[tokio::test]
async fn folders_are_created_then_listed() {
    use crate::folders::FolderFailure;
    use norelly_domain::mailbox::NewFolderError;

    let imap = FakeImap::new();
    let mut r = start(&imap);
    wait_for(&mut r.events, WAIT, online).await;
    let client = r.handle.client();
    let created = tokio::time::timeout(WAIT, client.create_folder("Factures".into(), None))
        .await
        .unwrap_or_else(|_| panic!("délai"));
    let factures = MailboxName::parse("Factures").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(created, Ok(factures.clone()));
    wait_for(&mut r.events, WAIT, |e| {
        matches!(e, SyncEvent::FoldersChanged { .. })
    })
    .await;
    let tree = r
        .store
        .tree(account())
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        tree.iter().any(|n| n.name == factures),
        "dossier enregistré localement"
    );

    // Refus : l'acteur reste en ligne et sert la demande suivante.
    let again = client.create_folder("factures".into(), None).await;
    assert_eq!(
        again,
        Err(FolderFailure::Invalid(NewFolderError::AlreadyExists))
    );
    let child = client
        .create_folder("2027".into(), Some(factures.clone()))
        .await;
    assert_eq!(
        child,
        Ok(MailboxName::parse("Factures/2027").unwrap_or_else(|e| panic!("{e}")))
    );
    r.handle.shutdown().await;
}

#[tokio::test]
async fn folder_created_is_reported_even_if_the_list_cannot_be_reread() {
    let imap = FakeImap::new();
    let mut r = start(&imap);
    wait_for(&mut r.events, WAIT, online).await;
    // LIST et CREATE passent, la relecture de la liste échoue.
    imap.cut_after(Some(2));
    let created = tokio::time::timeout(
        WAIT,
        r.handle.client().create_folder("Factures".into(), None),
    )
    .await
    .unwrap_or_else(|_| panic!("délai"));
    assert_eq!(
        created,
        Ok(MailboxName::parse("Factures").unwrap_or_else(|e| panic!("{e}")))
    );
    imap.cut_after(None);
    // Reconnexion, puis la liste est relue.
    wait_for(&mut r.events, WAIT, |e| {
        matches!(e, SyncEvent::FoldersChanged { .. })
    })
    .await;
    r.handle.shutdown().await;
}

#[tokio::test]
async fn abandoned_folder_request_is_not_sent() {
    let imap = FakeImap::new();
    let mut r = start(&imap);
    wait_for(&mut r.events, WAIT, online).await;
    // Synchronisation lente : la demande attend son tour, puis l'appelant
    // abandonne (délai de l'interface dépassé).
    imap.set_delay(Duration::from_millis(100));
    r.handle.sync_now();
    // La synchronisation a commencé : la demande attend dans la file.
    tokio::time::sleep(Duration::from_millis(30)).await;
    let abandoned = tokio::time::timeout(
        Duration::from_millis(50),
        r.handle.client().create_folder("Factures".into(), None),
    )
    .await;
    assert!(abandoned.is_err(), "demande abandonnée");
    imap.set_delay(Duration::ZERO);
    // Une demande suivante est servie : la précédente a été écartée.
    let next = tokio::time::timeout(WAIT, r.handle.client().create_folder("Notes".into(), None))
        .await
        .unwrap_or_else(|_| panic!("délai"));
    assert!(next.is_ok(), "{next:?}");
    let mut session = imap.session().unwrap_or_else(|e| panic!("{e}"));
    let names: Vec<String> = session
        .list_mailboxes()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .into_iter()
        .map(|m| m.name.as_str().to_owned())
        .collect();
    assert!(!names.contains(&"Factures".to_owned()), "{names:?}");
    r.handle.shutdown().await;
}

/// Total local du dossier `raw`, s'il est connu.
async fn local_total(r: &Running, raw: &str) -> Option<u32> {
    r.store
        .tree(account())
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .into_iter()
        .find(|n| n.name.as_str() == raw)
        .map(|n| n.total)
}

#[tokio::test]
async fn renamed_folder_is_listed_and_synchronised_at_once() {
    use crate::folders::FolderFailure;

    let imap = FakeImap::new();
    imap.create_mailbox("Projets", &[]);
    imap.create_mailbox("Projets/2026", &[]);
    imap.append("Projets", b"Subject: un\r\n\r\nx", Flags::default(), 1);
    imap.append(
        "Projets/2026",
        b"Subject: deux\r\n\r\ny",
        Flags::default(),
        2,
    );
    let mut r = start(&imap);
    wait_for(&mut r.events, WAIT, online).await;
    let client = r.handle.client();
    let renamed = tokio::time::timeout(
        WAIT,
        client.rename_folder(
            MailboxName::parse("Projets").unwrap_or_else(|e| panic!("{e}")),
            "Chantiers".into(),
        ),
    )
    .await
    .unwrap_or_else(|_| panic!("délai"));
    assert_eq!(
        renamed,
        Ok(MailboxName::parse("Chantiers").unwrap_or_else(|e| panic!("{e}")))
    );
    // Sans attendre la synchronisation complète (une heure) : le dossier
    // renommé et son sous-dossier sont relus aussitôt.
    let deadline = tokio::time::Instant::now() + WAIT;
    while (
        local_total(&r, "Chantiers").await,
        local_total(&r, "Chantiers/2026").await,
    ) != (Some(1), Some(1))
    {
        assert!(tokio::time::Instant::now() < deadline, "dossiers relus");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(local_total(&r, "Projets").await, None, "ancien nom oublié");

    // Refus du domaine : l'acteur reste en ligne et sert la suite.
    let inbox = client
        .rename_folder(MailboxName::inbox(), "Reçus".into())
        .await;
    assert_eq!(
        inbox,
        Err(FolderFailure::Change(FolderChangeError::Protected))
    );
    r.handle.shutdown().await;
}

#[tokio::test]
async fn moved_folder_is_listed_and_synchronised_at_once() {
    let imap = FakeImap::new();
    imap.create_mailbox("Bla", &[]);
    imap.create_mailbox("Bli", &[]);
    imap.create_mailbox("Bli/2026", &[]);
    imap.append("Bli", b"Subject: un\r\n\r\nx", Flags::default(), 1);
    imap.append("Bli/2026", b"Subject: deux\r\n\r\ny", Flags::default(), 2);
    let mut r = start(&imap);
    wait_for(&mut r.events, WAIT, online).await;
    let name = |raw: &str| MailboxName::parse(raw).unwrap_or_else(|e| panic!("{e}"));
    let moved = tokio::time::timeout(
        WAIT,
        r.handle
            .client()
            .move_folder(name("Bli"), Some(name("Bla"))),
    )
    .await
    .unwrap_or_else(|_| panic!("délai"));
    assert_eq!(moved, Ok(name("Bla/Bli")));
    // Le dossier déplacé et son sous-dossier sont relus aussitôt.
    let deadline = tokio::time::Instant::now() + WAIT;
    while (
        local_total(&r, "Bla/Bli").await,
        local_total(&r, "Bla/Bli/2026").await,
    ) != (Some(1), Some(1))
    {
        assert!(tokio::time::Instant::now() < deadline, "dossiers relus");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(local_total(&r, "Bli").await, None, "ancien nom oublié");
    r.handle.shutdown().await;
}

#[tokio::test]
async fn deleted_folder_disappears_locally() {
    let imap = FakeImap::new();
    imap.create_mailbox("Projets", &[]);
    imap.append("Projets", b"Subject: un\r\n\r\nx", Flags::default(), 1);
    let mut r = start(&imap);
    wait_for(&mut r.events, WAIT, online).await;
    let projets = MailboxName::parse("Projets").unwrap_or_else(|e| panic!("{e}"));
    let deadline = tokio::time::Instant::now() + WAIT;
    while local_total(&r, "Projets").await != Some(1) {
        assert!(tokio::time::Instant::now() < deadline, "synchronisé");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let deleted = tokio::time::timeout(WAIT, r.handle.client().delete_folder(projets.clone()))
        .await
        .unwrap_or_else(|_| panic!("délai"));
    assert_eq!(deleted, Ok(projets));
    wait_for(&mut r.events, WAIT, |e| {
        matches!(e, SyncEvent::FoldersChanged { .. })
    })
    .await;
    assert_eq!(local_total(&r, "Projets").await, None);
    r.handle.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn a_batch_skipped_by_grouping_is_shown_when_the_headers_are_done() {
    let mut live = LiveRefresh::default();
    assert!(live.refresh_due(), "premier lot signalé");
    assert!(!live.refresh_due(), "lot suivant regroupé");
    assert!(
        live.headers_done_due(),
        "lot écarté : signal avant les corps"
    );
    assert!(!live.headers_done_due(), "rien d'écarté depuis");
    tokio::time::advance(LIVE_REFRESH_INTERVAL).await;
    assert!(live.refresh_due());
    assert!(!live.headers_done_due(), "lot déjà signalé");
}
