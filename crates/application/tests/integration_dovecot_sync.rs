//! Synchronisation de bout en bout contre Dovecot, avec le stockage réel
//! (SQLite sur disque, fichiers .eml.zst) : initiale, incrémentale, IDLE
//! (message ajouté par APPEND côté serveur reçu en moins de 5 s),
//! UIDVALIDITY changée.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{FAST, TestResult, connector, local, raw_message, request};
use norelly_application::account_actor::{ActorConfig, spawn};
use norelly_application::sync::SyncEvent;
use norelly_domain::sync::Backoff;
use norelly_imap::Timeouts;
use norelly_test_support::certs::TestCa;
use norelly_test_support::containers::{
    DOVECOT_IMAP_PORT, DOVECOT_IMAPS_PORT, RunningServer, TEST_PASSWORD, TEST_USER, read_banner,
    start_dovecot_tls,
};
use norelly_test_support::imap_control::{ImapControl, MailboxControl};

struct Server {
    ca: TestCa,
    _server: RunningServer,
    host: String,
    port: u16,
    control: ImapControl,
}

async fn dovecot() -> Result<Server, Box<dyn std::error::Error>> {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost", "127.0.0.1"])?;
    let server = start_dovecot_tls(&cert).await?;
    let plain = server.port(DOVECOT_IMAP_PORT).await?;
    read_banner(&server.host, plain, Duration::from_secs(30)).await?;
    let port = server.port(DOVECOT_IMAPS_PORT).await?;
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

#[tokio::test]
async fn initial_then_incremental_sync() -> TestResult {
    let s = dovecot().await?;
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

#[tokio::test]
async fn uidvalidity_change_resyncs_the_mailbox() -> TestResult {
    let s = dovecot().await?;
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

#[tokio::test]
async fn idle_receives_appended_message_within_five_seconds() -> TestResult {
    let s = dovecot().await?;
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
            SyncEvent::Offline { reason, .. } => panic!("hors ligne : {reason}"),
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
