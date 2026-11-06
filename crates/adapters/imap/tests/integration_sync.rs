//! Suite de contrat de la session IMAP contre Dovecot (la même que celle
//! du faux serveur) : LIST, SELECT CONDSTORE/QRESYNC, FETCH, SEARCH, IDLE,
//! UTF-7 modifié, UIDVALIDITY changée ; actions (STORE, COPY, MOVE,
//! EXPUNGE), avec et sans MOVE/UIDPLUS.

use std::time::Duration;

use norelly_domain::credentials::{Credentials, Secret, Username};
use norelly_domain::endpoint::{Host, Security, ServerEndpoint};
use norelly_domain::ports::ConnectRequest;
use norelly_domain::security::PlaintextPolicy;
use norelly_imap::ImapConnector;
use norelly_test_support::certs::TestCa;
use norelly_test_support::containers::{
    DOVECOT_IMAP_PORT, DOVECOT_IMAPS_PORT, TEST_PASSWORD, TEST_USER, read_banner, start_dovecot_tls,
};
use norelly_test_support::imap_control::ImapControl;
use norelly_test_support::session_contract::{
    actions_session_contract, folders_session_contract, remote_session_contract,
};
use norelly_tls::{TlsClientFactory, TrustRoots};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[tokio::test]
async fn dovecot_passes_session_contract() -> TestResult {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost", "127.0.0.1"])?;
    let server = start_dovecot_tls(&cert).await?;
    let plain = server.port(DOVECOT_IMAP_PORT).await?;
    read_banner(&server.host, plain, Duration::from_secs(30)).await?;
    let port = server.port(DOVECOT_IMAPS_PORT).await?;
    let request = ConnectRequest {
        endpoint: ServerEndpoint::new(Host::parse(&server.host)?, port, Security::Tls)?,
        credentials: Credentials::new(
            Username::parse(TEST_USER)?,
            Secret::new(TEST_PASSWORD.to_owned()),
        )?,
        certificate_exceptions: vec![],
        plaintext: PlaintextPolicy::deny_all(),
    };
    let connector = ImapConnector::new(TlsClientFactory::new(TrustRoots::Custom(vec![ca.root()])));
    let control = ImapControl::new(
        &server.host,
        port,
        TEST_USER,
        TEST_PASSWORD,
        vec![ca.root()],
    );
    remote_session_contract(&connector, &request, &control, "Contrat").await?;
    // Actions : avec MOVE et UIDPLUS, puis capacités masquées (repli
    // COPY + UID EXPUNGE, puis COPY + EXPUNGE).
    actions_session_contract(&connector, &request, &control, "Actions").await?;
    let tls = || TlsClientFactory::new(TrustRoots::Custom(vec![ca.root()]));
    let no_move = ImapConnector::new(tls()).hiding_capabilities(&["MOVE"]);
    actions_session_contract(&no_move, &request, &control, "ActionsSansMove").await?;
    let bare = ImapConnector::new(tls()).hiding_capabilities(&["MOVE", "UIDPLUS"]);
    actions_session_contract(&bare, &request, &control, "ActionsSansUidplus").await?;
    folders_session_contract(&connector, &request, &control, "Nouveau").await?;
    Ok(())
}

/// ADR-002 : SPECIAL-USE n'est que partiellement modélisé par imap-codec ;
/// on vérifie que les attributs (`\\Sent`, `\\Trash`…) arrivent bien par
/// un `LIST` ordinaire et que les rôles sont reconnus.
#[tokio::test]
async fn dovecot_special_use_attributes_are_read_from_plain_list() -> TestResult {
    use norelly_domain::mailbox::{MailboxRole, classify_mailboxes};
    use norelly_domain::ports::{RemoteMailboxConnector, RemoteMailboxSession};

    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost", "127.0.0.1"])?;
    let server = start_dovecot_tls(&cert).await?;
    let plain = server.port(DOVECOT_IMAP_PORT).await?;
    read_banner(&server.host, plain, Duration::from_secs(30)).await?;
    let port = server.port(DOVECOT_IMAPS_PORT).await?;
    let request = ConnectRequest {
        endpoint: ServerEndpoint::new(Host::parse(&server.host)?, port, Security::Tls)?,
        credentials: Credentials::new(
            Username::parse(TEST_USER)?,
            Secret::new(TEST_PASSWORD.to_owned()),
        )?,
        certificate_exceptions: vec![],
        plaintext: PlaintextPolicy::deny_all(),
    };
    let connector = ImapConnector::new(TlsClientFactory::new(TrustRoots::Custom(vec![ca.root()])));
    let mut session = connector.connect(&request).await?;
    let listed = session.list_mailboxes().await?;
    // Attributs annoncés par le serveur (et non devinés par le nom).
    for (name, attribute) in [
        ("Sent", "\\Sent"),
        ("Drafts", "\\Drafts"),
        ("Trash", "\\Trash"),
        ("Junk", "\\Junk"),
    ] {
        let found = listed.iter().find(|m| m.name.as_str() == name);
        assert!(
            found.is_some_and(|m| m.attributes.iter().any(|a| a == attribute)),
            "{name} sans {attribute} : {listed:?}"
        );
    }
    let classified = classify_mailboxes(listed);
    let role = |name: &str| {
        classified
            .iter()
            .find(|m| m.name.as_str() == name)
            .map(|m| m.role)
    };
    assert_eq!(role("INBOX"), Some(MailboxRole::Inbox));
    assert_eq!(role("Sent"), Some(MailboxRole::Sent));
    assert_eq!(role("Drafts"), Some(MailboxRole::Drafts));
    assert_eq!(role("Trash"), Some(MailboxRole::Trash));
    assert_eq!(role("Junk"), Some(MailboxRole::Junk));
    session.logout().await?;
    Ok(())
}
