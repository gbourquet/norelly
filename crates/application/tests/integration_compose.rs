//! Rédaction de bout en bout contre Dovecot (IMAP) et Mailpit (SMTP), en
//! conteneurs, avec le stockage réel (SQLite et fichiers) :
//! - un brouillon enregistré est copié dans « Drafts » (UIDPLUS) ;
//! - envoyé, il part par Mailpit (vérifié par son API, Cci dans
//!   l'enveloppe mais pas dans les en-têtes), sa copie arrive dans « Sent »
//!   et sa copie de brouillon est supprimée de « Drafts ».

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{TestResult, connector, local, request};
use norelly_application::compose::{QueueMessage, SaveDraft};
use norelly_application::compose_worker::ComposeWorker;
use norelly_domain::address::EmailAddress;
use norelly_domain::compose::draft::{Draft, Recipient};
use norelly_domain::endpoint::{Host, Security, ServerEndpoint};
use norelly_domain::mailbox::MailboxName;
use norelly_domain::ports::{ConnectRequest, RemoteMailboxConnector, RemoteMailboxSession};
use norelly_imap::{ImapConnector, Timeouts};
use norelly_render::compose::MimeComposer;
use norelly_smtp::SmtpSender;
use norelly_sqlite::compose::SqliteComposeStore;
use norelly_test_support::certs::TestCa;
use norelly_test_support::containers::{
    DOVECOT_IMAP_PORT, DOVECOT_IMAPS_PORT, MAILPIT_HTTP_PORT, MAILPIT_SMTP_PORT, MailpitTls,
    http_get, read_banner, start_dovecot_tls, start_mailpit_tls, wait_for_port,
};
use norelly_tls::{TlsClientFactory, TrustRoots};

fn address(raw: &str) -> EmailAddress {
    EmailAddress::parse(raw).unwrap_or_else(|_| panic!("{raw}"))
}

async fn count(imap: &ImapConnector, req: &ConnectRequest, mailbox: &str) -> u32 {
    let mut session = imap.connect(req).await.unwrap_or_else(|e| panic!("{e}"));
    let name = MailboxName::parse(mailbox).unwrap_or_else(|_| panic!("{mailbox}"));
    let exists = session
        .select_writable(&name)
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .exists;
    let _ = session.logout().await;
    exists
}

#[tokio::test]
async fn draft_copied_then_sent_through_mailpit_and_filed_in_sent() -> TestResult {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost", "127.0.0.1"])?;
    let dovecot = start_dovecot_tls(&cert).await?;
    read_banner(
        &dovecot.host,
        dovecot.port(DOVECOT_IMAP_PORT).await?,
        Duration::from_secs(30),
    )
    .await?;
    let imap_req = request(&dovecot.host, dovecot.port(DOVECOT_IMAPS_PORT).await?);
    let mailpit = start_mailpit_tls(&cert, MailpitTls::Implicit).await?;
    let http = mailpit.port(MAILPIT_HTTP_PORT).await?;
    wait_for_port(&mailpit.host, http, Duration::from_secs(30)).await?;
    let mut smtp_req = request(&mailpit.host, mailpit.port(MAILPIT_SMTP_PORT).await?);
    smtp_req.endpoint = ServerEndpoint::new(
        Host::parse(&mailpit.host)?,
        mailpit.port(MAILPIT_SMTP_PORT).await?,
        Security::Tls,
    )?;

    let l = local("compose").await;
    let imap = connector(&ca, Timeouts::default());
    l.sync(&imap, &imap_req).await?;
    let compose = Arc::new(SqliteComposeStore::new(l.db.clone()));
    let me = Recipient::new("Norelly", address("norelly@example.com"));
    let worker = ComposeWorker {
        account: l.account,
        from: me.clone(),
        imap: imap_req.clone(),
        smtp: smtp_req,
        connector: Arc::new(connector(&ca, Timeouts::default())),
        sender: Arc::new(SmtpSender::new(TlsClientFactory::new(TrustRoots::Custom(
            vec![ca.root()],
        )))),
        store: Arc::clone(&compose),
        folders: Arc::clone(&l.store),
        blobs: Arc::clone(&l.blobs),
        composer: Arc::new(MimeComposer),
    };

    // --- Brouillon copié sur le serveur ---
    let subject = format!("Intégration rédaction {}", std::process::id());
    let mut draft = Draft::empty(l.account);
    draft.to = vec![Recipient::new("Dest", address("dest@example.com"))];
    draft.bcc = vec![Recipient::new("", address("cache@example.com"))];
    draft.subject.clone_from(&subject);
    draft.html = Some("<p>Bonjour <strong>Dest</strong></p>".into());
    let now = 1_800_000_000;
    let saved = SaveDraft { drafts: &*compose }
        .execute(None, &draft, now)
        .await?;
    let tick = worker.run_once(now).await?;
    assert_eq!(tick.drafts_saved, 1);
    assert_eq!(
        count(&imap, &imap_req, "Drafts").await,
        1,
        "brouillon sur Dovecot"
    );

    // --- Envoi ---
    QueueMessage {
        store: &*compose,
        blobs: &*l.blobs,
        composer: &MimeComposer,
    }
    .execute(Some(saved.id), &draft, &me, now + 1)
    .await?;
    let tick = worker.run_once(now + 1).await?;
    assert_eq!(tick.delivery.sent.len(), 1, "{tick:?}");
    assert_eq!(tick.filed, 1);
    let messages = http_get(&mailpit.host, http, "/api/v1/messages").await?;
    assert!(
        messages.contains(&subject),
        "absent de Mailpit : {messages}"
    );
    assert!(messages.contains("dest@example.com"));
    assert!(
        messages.contains("cache@example.com"),
        "Cci dans l'enveloppe"
    );
    assert_eq!(count(&imap, &imap_req, "Sent").await, 1, "copie dans Sent");
    assert_eq!(
        count(&imap, &imap_req, "Drafts").await,
        0,
        "copie du brouillon supprimée"
    );
    Ok(())
}
