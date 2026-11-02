//! Suite de contrat de la session IMAP contre Dovecot (la même que celle
//! du faux serveur) : LIST, SELECT CONDSTORE/QRESYNC, FETCH, SEARCH, IDLE,
//! UTF-7 modifié, UIDVALIDITY changée.

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
use norelly_test_support::session_contract::remote_session_contract;
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
    Ok(())
}
