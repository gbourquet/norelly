//! Pannes réseau entre Norelly et Dovecot, simulées par Toxiproxy :
//! coupure en pleine synchronisation, latence, connexion à moitié fermée
//! (plus aucune donnée, sans fermeture) et reprise de l'acteur. Dans tous
//! les cas, l'état local converge vers celui du serveur après la reprise.
//! Les actions faites hors ligne sont appliquées localement tout de suite,
//! puis rejouées sur le serveur au retour du réseau.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{FAST, TestResult, connector, local, raw_message, request};
use norelly_application::account_actor::{ActorConfig, spawn};
use norelly_application::sync::{SyncAccount, SyncEvent, SyncFailure};
use norelly_domain::connection::ConnectionError;
use norelly_domain::fetch_policy::FetchPolicy;
use norelly_domain::ports::{RemoteMailboxConnector, RemoteMailboxSession};
use norelly_domain::sync::Backoff;
use norelly_imap::ImapConnector;
use norelly_test_support::certs::TestCa;
use norelly_test_support::containers::{
    DOVECOT_IMAP_PORT, DOVECOT_IMAPS_PORT, RunningServer, TEST_PASSWORD, TEST_USER, Toxiproxy,
    read_banner, start_dovecot_tls_on, start_toxiproxy,
};
use norelly_test_support::imap_control::{ImapControl, MailboxControl};
use tokio::sync::broadcast;

const PROXY: &str = "imap";

struct Lab {
    _dovecot: RunningServer,
    toxiproxy: Toxiproxy,
    /// Connexion directe à Dovecot (contrôle et vérifications).
    control: ImapControl,
    direct: (String, u16),
    /// Connexion de Norelly, à travers Toxiproxy.
    proxied: (String, u16),
    ca: TestCa,
}

async fn lab(tag: &str) -> Result<Lab, Box<dyn std::error::Error>> {
    let unique = format!(
        "{}-{}-{tag}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis()
    );
    let network = format!("norelly-net-{unique}");
    let name = format!("norelly-dovecot-{unique}");
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost", "127.0.0.1"])?;
    let dovecot = start_dovecot_tls_on(&cert, &network, &name).await?;
    let plain = dovecot.port(DOVECOT_IMAP_PORT).await?;
    read_banner(&dovecot.host, plain, Duration::from_secs(30)).await?;
    let direct_port = dovecot.port(DOVECOT_IMAPS_PORT).await?;
    let toxiproxy = start_toxiproxy(&network).await?;
    let proxy_port = toxiproxy
        .proxy(PROXY, &format!("{name}:{DOVECOT_IMAPS_PORT}"))
        .await?;
    let host = dovecot.host.clone();
    let control = ImapControl::new(
        &host,
        direct_port,
        TEST_USER,
        TEST_PASSWORD,
        vec![ca.root()],
    );
    let proxied_host = toxiproxy.server.host.clone();
    Ok(Lab {
        _dovecot: dovecot,
        toxiproxy,
        control,
        direct: (host, direct_port),
        proxied: (proxied_host, proxy_port),
        ca,
    })
}

impl Lab {
    fn connector(&self) -> ImapConnector {
        connector(&self.ca, FAST)
    }

    fn proxied_request(&self) -> norelly_domain::ports::ConnectRequest {
        request(&self.proxied.0, self.proxied.1)
    }

    fn direct_request(&self) -> norelly_domain::ports::ConnectRequest {
        request(&self.direct.0, self.direct.1)
    }

    async fn fill(&self, count: u32) -> TestResult {
        for n in 1..=count {
            self.control
                .append("INBOX", &raw_message(n), n % 4 == 0)
                .await?;
        }
        Ok(())
    }
}

#[tokio::test]
async fn cut_in_the_middle_of_a_sync_then_converges() -> TestResult {
    let lab = lab("cut").await?;
    lab.fill(150).await?;
    let conn = lab.connector();
    let l = local("toxi-cut").await;
    // Coupures successives à différents volumes : chaque tentative avance
    // un peu, puis échoue.
    let mut failures = 0;
    for limit in [8_000, 40_000, 120_000, 250_000] {
        lab.toxiproxy
            .add_toxic(
                PROXY,
                "coupure",
                "limit_data",
                "downstream",
                &format!("{{\"bytes\":{limit}}}"),
            )
            .await?;
        if l.sync(&conn, &lab.proxied_request()).await.is_err() {
            failures += 1;
        }
        lab.toxiproxy.remove_toxic(PROXY, "coupure").await?;
        // Changements côté serveur entre deux tentatives.
        lab.control.set_seen("INBOX", limit / 8_000, true).await?;
    }
    assert!(
        failures >= 2,
        "les coupures doivent interrompre la synchronisation"
    );
    lab.control.expunge("INBOX", 2).await?;
    lab.control
        .append("INBOX", &raw_message(500), false)
        .await?;
    l.sync(&conn, &lab.proxied_request()).await?;
    l.assert_converged(&conn, &lab.direct_request(), "INBOX")
        .await;
    assert_eq!(l.messages("INBOX").await.len(), 150);
    Ok(())
}

#[tokio::test]
async fn latency_slows_but_does_not_break_sync() -> TestResult {
    let lab = lab("latency").await?;
    lab.fill(30).await?;
    lab.toxiproxy
        .add_toxic(
            PROXY,
            "lent",
            "latency",
            "downstream",
            "{\"latency\":150,\"jitter\":50}",
        )
        .await?;
    lab.toxiproxy
        .add_toxic(
            PROXY,
            "lent-montant",
            "latency",
            "upstream",
            "{\"latency\":150}",
        )
        .await?;
    let conn = lab.connector();
    let l = local("toxi-latency").await;
    l.sync(&conn, &lab.proxied_request()).await?;
    lab.control.set_seen("INBOX", 5, true).await?;
    lab.control.append("INBOX", &raw_message(31), false).await?;
    l.sync(&conn, &lab.proxied_request()).await?;
    l.assert_converged(&conn, &lab.direct_request(), "INBOX")
        .await;
    Ok(())
}

#[tokio::test]
async fn half_open_connection_times_out_then_converges() -> TestResult {
    let lab = lab("halfopen").await?;
    lab.fill(20).await?;
    let conn = lab.connector();
    let l = local("toxi-halfopen").await;
    l.sync(&conn, &lab.proxied_request()).await?;

    // 1. En pleine session : la connexion devient muette (plus aucune
    // donnée, sans fermeture) ; seul le délai de réponse le révèle.
    let mut session = conn.connect(&lab.proxied_request()).await?;
    let ext = session.enable_sync_extensions().await?;
    lab.toxiproxy
        .add_toxic(PROXY, "muet", "timeout", "downstream", "{\"timeout\":0}")
        .await?;
    lab.control.append("INBOX", &raw_message(22), false).await?;
    let started = tokio::time::Instant::now();
    let result = SyncAccount {
        account: l.account,
        store: &*l.store,
        blobs: &*l.blobs,
        policy: FetchPolicy::default(),
        text: &norelly_render::search_text::MimeTextExtractor,
    }
    .sync_all(&mut session, ext, |_| {})
    .await;
    assert!(
        matches!(
            result,
            Err(SyncFailure::Connection(ConnectionError::Timeout(_)))
        ),
        "{result:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "{:?}",
        started.elapsed()
    );
    drop(session);
    lab.toxiproxy.remove_toxic(PROXY, "muet").await?;

    // 2. À l'ouverture d'une nouvelle connexion.
    lab.toxiproxy
        .add_toxic(PROXY, "muet", "timeout", "downstream", "{\"timeout\":0}")
        .await?;
    lab.control.append("INBOX", &raw_message(21), false).await?;
    let started = tokio::time::Instant::now();
    let result = l.sync(&conn, &lab.proxied_request()).await;
    assert!(result.is_err(), "connexion muette non détectée");
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "{:?}",
        started.elapsed()
    );
    lab.toxiproxy.remove_toxic(PROXY, "muet").await?;
    l.sync(&conn, &lab.proxied_request()).await?;
    l.assert_converged(&conn, &lab.direct_request(), "INBOX")
        .await;
    Ok(())
}

async fn wait_event(
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
            Err(_) => panic!("événement attendu absent après {within:?}"),
        }
    }
}

#[tokio::test]
async fn actor_reconnects_after_outage_and_converges() -> TestResult {
    let lab = lab("actor").await?;
    lab.fill(5).await?;
    let l = local("toxi-actor").await;
    let conn = Arc::new(lab.connector());
    let handle = spawn(
        l.account,
        lab.proxied_request(),
        Arc::clone(&conn),
        Arc::clone(&l.store),
        Arc::clone(&l.blobs),
        Arc::new(norelly_render::search_text::MimeTextExtractor),
        ActorConfig {
            backoff: Backoff::new(200, 2_000),
            ..ActorConfig::default()
        },
    );
    let mut events = handle.subscribe();
    wait_event(&mut events, Duration::from_secs(30), |e| {
        matches!(e, SyncEvent::Online { .. })
    })
    .await;

    // Panne : connexions coupées et refusées.
    lab.toxiproxy.set_enabled(PROXY, false).await?;
    wait_event(&mut events, Duration::from_secs(30), |e| {
        matches!(e, SyncEvent::Offline { .. })
    })
    .await;
    // Changements pendant la panne.
    lab.control.append("INBOX", &raw_message(6), false).await?;
    lab.control.set_seen("INBOX", 1, true).await?;
    lab.control.expunge("INBOX", 3).await?;
    lab.toxiproxy.set_enabled(PROXY, true).await?;
    wait_event(&mut events, Duration::from_secs(30), |e| {
        matches!(e, SyncEvent::Online { .. })
    })
    .await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let local = l.messages("INBOX").await;
        let server = common::server_flags(&conn, &lab.direct_request(), "INBOX").await;
        if local == server {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "pas de convergence : local {local:?}, serveur {server:?}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    l.assert_converged(&conn, &lab.direct_request(), "INBOX")
        .await;
    handle.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn offline_actions_are_replayed_when_the_network_comes_back() -> TestResult {
    use norelly_application::account_actor::unix_now;
    use norelly_application::actions::PerformAction;
    use norelly_domain::actions::MessageAction;
    use norelly_domain::mailbox::Uid;
    use norelly_domain::ports::{MessageListQuery, PendingOpStore};

    let lab = lab("actions").await?;
    lab.fill(4).await?;
    lab.control.create("Trash").await?;
    lab.control.create("Archives").await?;
    let l = local("toxi-actions").await;
    let conn = Arc::new(lab.connector());
    let handle = spawn(
        l.account,
        lab.proxied_request(),
        Arc::clone(&conn),
        Arc::clone(&l.store),
        Arc::clone(&l.blobs),
        Arc::new(norelly_render::search_text::MimeTextExtractor),
        ActorConfig {
            backoff: Backoff::new(200, 2_000),
            ..ActorConfig::default()
        },
    );
    let mut events = handle.subscribe();
    wait_event(&mut events, Duration::from_secs(30), |e| {
        matches!(e, SyncEvent::Online { .. })
    })
    .await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while l.messages("INBOX").await.len() < 4 || l.id("Archives").await.is_none() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "synchronisation initiale"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Panne réseau, puis actions locales.
    lab.toxiproxy.set_enabled(PROXY, false).await?;
    wait_event(&mut events, Duration::from_secs(30), |e| {
        matches!(e, SyncEvent::Offline { .. })
    })
    .await;
    let inbox = l.id("INBOX").await.ok_or("INBOX")?;
    let page = l.store.page(inbox, None, 10).await?;
    let by_uid = |n: u32| {
        page.items
            .iter()
            .find(|i| i.uid.get() == n)
            .map(|i| i.id)
            .ok_or(format!("UID {n}"))
    };
    let act = |ids: Vec<norelly_domain::ports::MessageId>, action| {
        let store = Arc::clone(&l.store);
        async move {
            // Fenêtre d'annulation écoulée : opérations dues.
            PerformAction { store: &*store }
                .execute(&ids, action, unix_now() - 60)
                .await
        }
    };
    act(vec![by_uid(1)?], MessageAction::Delete).await?;
    act(vec![by_uid(2)?], MessageAction::Archive).await?;
    act(vec![by_uid(3)?], MessageAction::MarkFlagged(true)).await?;
    act(vec![by_uid(4)?], MessageAction::MarkSeen(false)).await?;
    handle.replay_soon();
    // Effets locaux immédiats ; rien n'est parti.
    let visible = l.store.page(inbox, None, 10).await?;
    assert_eq!(visible.items.len(), 2, "supprimé et archivé masqués");
    let server = common::server_flags(&conn, &lab.direct_request(), "INBOX").await;
    assert_eq!(server.len(), 4, "serveur inchangé pendant la panne");

    // Retour du réseau : rejeu, puis convergence.
    lab.toxiproxy.set_enabled(PROXY, true).await?;
    let uid = |n: u32| Uid::new(n).ok_or("uid");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let inbox_server = common::server_flags(&conn, &lab.direct_request(), "INBOX").await;
        let trash = common::server_flags(&conn, &lab.direct_request(), "Trash").await;
        let archives = common::server_flags(&conn, &lab.direct_request(), "Archives").await;
        let pending = l.store.pending(l.account).await?;
        let done = inbox_server.keys().map(|u| u.get()).collect::<Vec<_>>() == [3, 4]
            && trash.len() == 1
            && archives.len() == 1
            && inbox_server.get(&uid(3)?).is_some_and(|f| f.flagged)
            && inbox_server.get(&uid(4)?).is_some_and(|f| !f.seen)
            && pending.is_empty();
        if done {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "rejeu incomplet : INBOX {inbox_server:?}, Trash {trash:?}, \
             Archives {archives:?}, file {pending:?}"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    // L'état local rejoint celui du serveur.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while l.messages("INBOX").await
        != common::server_flags(&conn, &lab.direct_request(), "INBOX").await
    {
        assert!(tokio::time::Instant::now() < deadline, "convergence locale");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    handle.shutdown().await;
    Ok(())
}
