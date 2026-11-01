//! Intégration avec Dovecot (conteneur) et la CA de test : TLS implicite
//! (993) et STARTTLS (143), authentification, capacités attendues.

use std::time::Duration;

use norelly_domain::auth::AuthError;
use norelly_domain::connection::ConnectionError;
use norelly_domain::credentials::{Credentials, Secret, Username};
use norelly_domain::endpoint::{Host, Security, ServerEndpoint};
use norelly_domain::ports::{ConnectRequest, RemoteMailboxConnector, RemoteMailboxSession};
use norelly_domain::security::PlaintextPolicy;
use norelly_imap::ImapConnector;
use norelly_test_support::certs::TestCa;
use norelly_test_support::containers::{
    DOVECOT_IMAP_PORT, DOVECOT_IMAPS_PORT, RunningServer, TEST_PASSWORD, TEST_USER, read_banner,
    start_dovecot_tls,
};
use norelly_tls::{TlsClientFactory, TrustRoots};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Extensions dont Norelly a besoin (ADR-002), annoncées après connexion.
const EXPECTED: [&str; 9] = [
    "IMAP4REV1",
    "IDLE",
    "CONDSTORE",
    "QRESYNC",
    "UIDPLUS",
    "MOVE",
    "SPECIAL-USE",
    "ENABLE",
    "LITERAL+",
];

async fn dovecot() -> Result<(TestCa, RunningServer), Box<dyn std::error::Error>> {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost", "127.0.0.1"])?;
    let server = start_dovecot_tls(&cert).await?;
    let port = server.port(DOVECOT_IMAP_PORT).await?;
    read_banner(&server.host, port, Duration::from_secs(30)).await?;
    Ok((ca, server))
}

fn request(host: &str, port: u16, security: Security, password: &str) -> ConnectRequest {
    ConnectRequest {
        endpoint: ServerEndpoint::new(
            Host::parse(host).unwrap_or_else(|_| panic!("hôte {host}")),
            port,
            security,
        )
        .unwrap_or_else(|_| panic!("point de connexion")),
        credentials: Credentials::new(
            Username::parse(TEST_USER).unwrap_or_else(|_| panic!("utilisateur")),
            Secret::new(password.to_owned()),
        )
        .unwrap_or_else(|_| panic!("identifiants")),
        certificate_exceptions: vec![],
        plaintext: PlaintextPolicy::deny_all(),
    }
}

fn connector(ca: &TestCa) -> ImapConnector {
    ImapConnector::new(TlsClientFactory::new(TrustRoots::Custom(vec![ca.root()])))
}

#[tokio::test]
async fn implicit_tls_993_and_starttls_143() -> TestResult {
    let (ca, server) = dovecot().await?;
    let connector = connector(&ca);
    for (internal, security) in [
        (DOVECOT_IMAPS_PORT, Security::Tls),
        (DOVECOT_IMAP_PORT, Security::StartTls),
    ] {
        let port = server.port(internal).await?;
        let session = connector
            .connect(&request(&server.host, port, security, TEST_PASSWORD))
            .await?;
        let caps = session.capabilities();
        let missing = caps.missing(&EXPECTED);
        assert!(
            missing.is_empty(),
            "{security} : capacités manquantes {missing:?} parmi {:?}",
            caps.iter().collect::<Vec<_>>()
        );
        session.logout().await?;
    }
    Ok(())
}

#[tokio::test]
async fn wrong_password_is_reported_as_authentication_failure() -> TestResult {
    let (ca, server) = dovecot().await?;
    let port = server.port(DOVECOT_IMAPS_PORT).await?;
    let result = connector(&ca)
        .connect(&request(&server.host, port, Security::Tls, "mauvais"))
        .await;
    assert!(
        matches!(
            result,
            Err(ConnectionError::Authentication(AuthError::Rejected { .. }))
        ),
        "{result:?}"
    );
    Ok(())
}

#[tokio::test]
async fn untrusted_ca_is_rejected() -> TestResult {
    let (_, server) = dovecot().await?;
    let other_ca = TestCa::new()?;
    let port = server.port(DOVECOT_IMAPS_PORT).await?;
    let result = connector(&other_ca)
        .connect(&request(&server.host, port, Security::Tls, TEST_PASSWORD))
        .await;
    assert!(
        matches!(result, Err(ConnectionError::Certificate(_))),
        "{result:?}"
    );
    Ok(())
}
