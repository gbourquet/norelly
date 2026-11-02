//! Actions sur les messages de bout en bout contre Dovecot, avec le
//! stockage réel (SQLite sur disque) : chaque action est appliquée
//! localement, rejouée, puis vérifiée côté serveur ; le tout avec MOVE et
//! UIDPLUS, sans MOVE, puis sans MOVE ni UIDPLUS (capacités masquées par
//! le connecteur).

mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use common::{TestResult, connector, local, raw_message, request, server_flags};
use norelly_application::account_actor::unix_now;
use norelly_application::actions::{PerformAction, ReplayActions};
use norelly_domain::actions::MessageAction;
use norelly_domain::mailbox::{Flags, Uid};
use norelly_domain::ports::{
    MessageId, MessageListQuery, PendingOpStore, RemoteMailboxConnector, RemoteMailboxSession,
};
use norelly_imap::{ImapConnector, Timeouts};
use norelly_test_support::certs::TestCa;
use norelly_test_support::containers::{
    DOVECOT_IMAP_PORT, DOVECOT_IMAPS_PORT, TEST_PASSWORD, TEST_USER, read_banner, start_dovecot_tls,
};
use norelly_test_support::imap_control::{ImapControl, MailboxControl};

async fn scenario(tag: &str, hidden: &[&str]) -> TestResult {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost", "127.0.0.1"])?;
    let server = start_dovecot_tls(&cert).await?;
    let plain = server.port(DOVECOT_IMAP_PORT).await?;
    read_banner(&server.host, plain, Duration::from_secs(30)).await?;
    let port = server.port(DOVECOT_IMAPS_PORT).await?;
    let control = ImapControl::new(
        &server.host,
        port,
        TEST_USER,
        TEST_PASSWORD,
        vec![ca.root()],
    );
    for name in ["Trash", "Archives", "Projets"] {
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
    for name in ["Projets", "Archives", "Trash"] {
        assert_eq!(flags(name).await.len(), 1, "{tag} : {name}");
    }
    assert!(l.store.pending(l.account).await?.is_empty());

    // La synchronisation rejoint le serveur.
    l.sync(&conn, &req).await?;
    for name in ["INBOX", "Projets", "Archives", "Trash"] {
        l.assert_converged(&direct, &req, name).await;
    }

    // --- Seconde série : retour en arrière et suppression définitive ---
    act(vec![get(1)?], MessageAction::MarkSeen(false)).await?;
    act(vec![get(2)?], MessageAction::MarkFlagged(false)).await?;
    let trashed: Vec<MessageId> = ids("Trash").await?.into_values().collect();
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
        flags("Trash").await.is_empty(),
        "{tag} : supprimé définitivement"
    );
    l.sync(&conn, &req).await?;
    l.assert_converged(&direct, &req, "INBOX").await;
    l.assert_converged(&direct, &req, "Trash").await;
    Ok(())
}

#[tokio::test]
async fn every_action_with_move_and_uidplus() -> TestResult {
    scenario("actions-move", &[]).await
}

#[tokio::test]
async fn every_action_without_move() -> TestResult {
    scenario("actions-sans-move", &["MOVE"]).await
}

#[tokio::test]
async fn every_action_without_move_nor_uidplus() -> TestResult {
    scenario("actions-sans-uidplus", &["MOVE", "UIDPLUS"]).await
}
