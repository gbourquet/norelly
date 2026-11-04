//! Scénarios de bout en bout contre un serveur IMAP réel (Dovecot à chaque
//! exécution, Stalwart dans le job nocturne), avec le stockage réel
//! (SQLite sur disque, fichiers .eml.zst) :
//! - synchronisation initiale, incrémentale, IDLE (message ajouté par
//!   APPEND côté serveur reçu en moins de 5 s), UIDVALIDITY changée ;
//! - actions sur les messages : appliquées localement, rejouées, puis
//!   vérifiées côté serveur, avec ou sans MOVE et UIDPLUS (capacités
//!   masquées par le connecteur).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use super::{FAST, TestResult, connector, local, raw_message, request, server_flags};
use norelly_application::account_actor::{ActorConfig, spawn, unix_now};
use norelly_application::actions::{PerformAction, ReplayActions};
use norelly_application::sync::SyncEvent;
use norelly_domain::actions::MessageAction;
use norelly_domain::mailbox::{Flags, Uid};
use norelly_domain::ports::{
    MessageId, MessageListQuery, PendingOpStore, RemoteMailboxConnector, RemoteMailboxSession,
};
use norelly_domain::sync::Backoff;
use norelly_imap::{ImapConnector, Timeouts};
use norelly_test_support::certs::TestCa;
use norelly_test_support::containers::{ImapServerKind, RunningServer, TEST_PASSWORD, TEST_USER};
use norelly_test_support::imap_control::{ImapControl, MailboxControl};

struct Server {
    ca: TestCa,
    _server: RunningServer,
    host: String,
    port: u16,
    control: ImapControl,
}

async fn start(kind: ImapServerKind) -> Result<Server, Box<dyn std::error::Error>> {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost", "127.0.0.1"])?;
    let (server, port) = kind.start_tls(&cert).await?;
    let host = server.host.clone();
    let control = ImapControl::new(&host, port, TEST_USER, TEST_PASSWORD, vec![ca.root()]);
    Ok(Server {
        ca,
        _server: server,
        host,
        port,
        control,
    })
}

pub async fn initial_then_incremental_sync(kind: ImapServerKind) -> TestResult {
    let s = start(kind).await?;
    for n in 1..=25 {
        s.control
            .append("INBOX", &raw_message(n), n % 3 == 0)
            .await?;
    }
    s.control.create("Archives").await?;
    for n in 100..103 {
        s.control.append("Archives", &raw_message(n), true).await?;
    }
    let conn = connector(&s.ca, Timeouts::default());
    let req = request(&s.host, s.port);
    let l = local("initial").await;
    l.sync(&conn, &req).await?;
    l.assert_converged(&conn, &req, "INBOX").await;
    l.assert_converged(&conn, &req, "Archives").await;
    assert_eq!(l.messages("INBOX").await.len(), 25);

    // Changements côté serveur.
    s.control.set_seen("INBOX", 1, true).await?;
    s.control.set_seen("INBOX", 3, false).await?;
    s.control.expunge("INBOX", 2).await?;
    s.control.expunge("INBOX", 25).await?;
    s.control.append("INBOX", &raw_message(26), false).await?;
    s.control
        .append("Archives", &raw_message(103), false)
        .await?;
    l.sync(&conn, &req).await?;
    l.assert_converged(&conn, &req, "INBOX").await;
    l.assert_converged(&conn, &req, "Archives").await;
    assert_eq!(l.messages("INBOX").await.len(), 24);

    // Sans changement : rien ne bouge.
    let before = l.messages("INBOX").await;
    l.sync(&conn, &req).await?;
    assert_eq!(l.messages("INBOX").await, before);
    Ok(())
}

pub async fn uidvalidity_change_resyncs_the_mailbox(kind: ImapServerKind) -> TestResult {
    let s = start(kind).await?;
    s.control.create("Projet").await?;
    for n in 1..=5 {
        s.control.append("Projet", &raw_message(n), false).await?;
    }
    let conn = connector(&s.ca, Timeouts::default());
    let req = request(&s.host, s.port);
    let l = local("uidvalidity").await;
    l.sync(&conn, &req).await?;
    let first_id = l.id("Projet").await;
    assert_eq!(l.messages("Projet").await.len(), 5);

    // Boîte supprimée puis recréée sous le même nom : nouvelle
    // UIDVALIDITY, UID qui repartent de 1, autres messages.
    s.control.delete("Projet").await?;
    s.control.create("Projet").await?;
    for n in 50..52 {
        s.control.append("Projet", &raw_message(n), true).await?;
    }
    l.sync(&conn, &req).await?;
    assert_eq!(l.id("Projet").await, first_id, "même boîte locale");
    assert_eq!(l.messages("Projet").await.len(), 2);
    l.assert_converged(&conn, &req, "Projet").await;
    Ok(())
}

pub async fn idle_receives_appended_message_within_five_seconds(
    kind: ImapServerKind,
) -> TestResult {
    let s = start(kind).await?;
    s.control.append("INBOX", &raw_message(1), false).await?;
    let l = local("idle").await;
    let conn = Arc::new(connector(&s.ca, FAST));
    let req = request(&s.host, s.port);
    let handle = spawn(
        l.account,
        req.clone(),
        Arc::clone(&conn),
        Arc::clone(&l.store),
        Arc::clone(&l.blobs),
        Arc::new(norelly_render::search_text::MimeTextExtractor),
        ActorConfig {
            backoff: Backoff::new(100, 1_000),
            ..ActorConfig::default()
        },
    );
    let mut events = handle.subscribe();
    let wait = |within: Duration| tokio::time::Instant::now() + within;
    let deadline = wait(Duration::from_secs(20));
    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await?? {
            SyncEvent::Online { .. } => break,
            SyncEvent::Offline { failure, .. } => panic!("hors ligne : {failure}"),
            _ => {}
        }
    }
    // Laisse la connexion IDLE s'établir (synchronisation de rattrapage).
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(l.messages("INBOX").await.len(), 1);

    let started = tokio::time::Instant::now();
    s.control.append("INBOX", &raw_message(2), false).await?;
    let deadline = wait(Duration::from_secs(5));
    while l.messages("INBOX").await.len() != 2 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "message non reçu par IDLE en 5 s"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
    eprintln!("message reçu par IDLE en {elapsed:?}");
    l.assert_converged(&conn, &req, "INBOX").await;
    handle.shutdown().await;
    Ok(())
}

pub async fn actions(kind: ImapServerKind, tag: &str, hidden: &[&str]) -> TestResult {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost", "127.0.0.1"])?;
    let (server, port) = kind.start_tls(&cert).await?;
    let control = ImapControl::new(
        &server.host,
        port,
        TEST_USER,
        TEST_PASSWORD,
        vec![ca.root()],
    );
    // Corbeille : créée pour Dovecot ; celle de Stalwart existe déjà
    // (« Deleted Items », attribut \Trash).
    let trash: &'static str = match kind {
        ImapServerKind::Dovecot => {
            control.create("Trash").await?;
            "Trash"
        }
        ImapServerKind::Stalwart => "Deleted Items",
    };
    for name in ["Archives", "Projets"] {
        control.create(name).await?;
    }
    for n in 1..=6 {
        control.append("INBOX", &raw_message(n), false).await?;
    }
    let req = request(&server.host, port);
    let conn: ImapConnector = connector(&ca, Timeouts::default()).hiding_capabilities(hidden);
    let direct = connector(&ca, Timeouts::default());
    let l = local(tag).await;
    l.sync(&conn, &req).await?;

    let ids = |name: &'static str| {
        let l = &l;
        async move {
            let id = l.id(name).await.ok_or(name)?;
            let page = l.store.page(id, None, 100).await?;
            Ok::<BTreeMap<u32, MessageId>, Box<dyn std::error::Error>>(
                page.items.iter().map(|i| (i.uid.get(), i.id)).collect(),
            )
        }
    };
    let act = |targets: Vec<MessageId>, action: MessageAction| {
        let store = &l.store;
        async move {
            PerformAction { store: &**store }
                .execute(&targets, action, unix_now())
                .await
        }
    };
    let replay = || async {
        let mut session = conn.connect(&req).await?;
        session.enable_sync_extensions().await?;
        // Après la fenêtre d'annulation.
        let report = ReplayActions {
            account: l.account,
            store: &*l.store,
        }
        .execute(&mut session, unix_now() + 60)
        .await?;
        let _ = session.logout().await;
        Ok::<_, Box<dyn std::error::Error>>(report)
    };
    let flags = |name: &'static str| server_flags(&direct, &req, name);
    let uid = |n: u32| Uid::new(n).ok_or("uid");

    // --- Première série ---
    let inbox = ids("INBOX").await?;
    let get = |n: u32| inbox.get(&n).copied().ok_or(format!("UID {n}"));
    let projets = l.id("Projets").await.ok_or("Projets")?;
    act(vec![get(1)?], MessageAction::MarkSeen(true)).await?;
    act(vec![get(2)?], MessageAction::MarkFlagged(true)).await?;
    act(vec![get(3)?], MessageAction::Move(projets)).await?;
    act(vec![get(4)?], MessageAction::Archive).await?;
    act(vec![get(5)?], MessageAction::Delete).await?;
    assert_eq!(ids("INBOX").await?.len(), 3, "{tag} : masqués localement");
    for name in ["Projets", "Archives", trash] {
        assert_eq!(
            ids(name).await?.len(),
            1,
            "{tag} : visible tout de suite dans {name}"
        );
    }
    let report = replay().await?;
    assert_eq!(report.applied, 5, "{tag} : {report:?}");
    assert!(report.dropped.is_empty(), "{tag} : {report:?}");

    let server_inbox = flags("INBOX").await;
    assert_eq!(
        server_inbox.keys().map(|u| u.get()).collect::<Vec<_>>(),
        [1, 2, 6],
        "{tag} : INBOX {server_inbox:?}"
    );
    assert!(
        server_inbox
            .get(&uid(1)?)
            .is_some_and(|f| f.seen && !f.flagged)
    );
    assert!(
        server_inbox
            .get(&uid(2)?)
            .is_some_and(|f| f.flagged && !f.seen)
    );
    for name in ["Projets", "Archives", trash] {
        assert_eq!(flags(name).await.len(), 1, "{tag} : {name}");
    }
    assert!(l.store.pending(l.account).await?.is_empty());

    // La synchronisation rejoint le serveur.
    l.sync(&conn, &req).await?;
    for name in ["INBOX", "Projets", "Archives", trash] {
        l.assert_converged(&direct, &req, name).await;
    }

    // --- Seconde série : retour en arrière et suppression définitive ---
    act(vec![get(1)?], MessageAction::MarkSeen(false)).await?;
    act(vec![get(2)?], MessageAction::MarkFlagged(false)).await?;
    let trashed: Vec<MessageId> = ids(trash).await?.into_values().collect();
    assert_eq!(trashed.len(), 1);
    act(trashed, MessageAction::Delete).await?;
    let report = replay().await?;
    assert_eq!(report.applied, 3, "{tag} : {report:?}");
    let server_inbox = flags("INBOX").await;
    assert!(
        server_inbox.values().all(|f| *f == Flags::default()),
        "{tag} : {server_inbox:?}"
    );
    assert!(
        flags(trash).await.is_empty(),
        "{tag} : supprimé définitivement"
    );
    l.sync(&conn, &req).await?;
    l.assert_converged(&direct, &req, "INBOX").await;
    l.assert_converged(&direct, &req, trash).await;
    Ok(())
}
