//! Intégration avec Mailpit (conteneur) et la CA de test : envoi en TLS
//! implicite (465) et en STARTTLS exigé (587), message vérifié par l'API.

use std::time::Duration;

use norelly_domain::credentials::{Credentials, Secret, Username};
use norelly_domain::endpoint::{Host, Security, ServerEndpoint};
use norelly_domain::ports::{ConnectRequest, MailSender, OutgoingMessage};
use norelly_domain::security::PlaintextPolicy;
use norelly_smtp::SmtpSender;
use norelly_test_support::certs::TestCa;
use norelly_test_support::containers::{
    MAILPIT_HTTP_PORT, MAILPIT_SMTP_PORT, MailpitTls, TEST_PASSWORD, TEST_USER, http_get,
    start_mailpit_tls, wait_for_port,
};
use norelly_tls::{TlsClientFactory, TrustRoots};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn request(host: &str, port: u16, security: Security) -> ConnectRequest {
    ConnectRequest {
        endpoint: ServerEndpoint::new(
            Host::parse(host).unwrap_or_else(|_| panic!("hôte {host}")),
            port,
            security,
        )
        .unwrap_or_else(|_| panic!("point de connexion")),
        credentials: Credentials::new(
            Username::parse(TEST_USER).unwrap_or_else(|_| panic!("utilisateur")),
            Secret::new(TEST_PASSWORD.to_owned()),
        )
        .unwrap_or_else(|_| panic!("identifiants")),
        certificate_exceptions: vec![],
        plaintext: PlaintextPolicy::deny_all(),
    }
}

async fn send_and_check(mode: MailpitTls, security: Security) -> TestResult {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost", "127.0.0.1"])?;
    let server = start_mailpit_tls(&cert, mode).await?;
    let smtp_port = server.port(MAILPIT_SMTP_PORT).await?;
    let http_port = server.port(MAILPIT_HTTP_PORT).await?;
    wait_for_port(&server.host, http_port, Duration::from_secs(30)).await?;

    let sender = SmtpSender::new(TlsClientFactory::new(TrustRoots::Custom(vec![ca.root()])));
    let req = request(&server.host, smtp_port, security);
    let caps = sender.verify(&req).await?;
    assert!(caps.supports_auth(&norelly_domain::auth::AuthMechanism::Plain));

    let subject = format!("Essai Norelly {security} {}", std::process::id());
    let raw = format!(
        "From: <norelly@example.com>\r\nTo: <dest@example.com>\r\nSubject: {subject}\r\n\r\nBonjour.\r\n"
    );
    sender
        .send(
            &req,
            &OutgoingMessage {
                from: "norelly@example.com".into(),
                recipients: vec!["dest@example.com".into()],
                raw: raw.into_bytes(),
            },
        )
        .await?;

    let body = http_get(&server.host, http_port, "/api/v1/messages").await?;
    assert!(
        body.contains(&subject),
        "message absent de Mailpit : {body}"
    );
    assert!(body.contains("dest@example.com"));
    Ok(())
}

#[tokio::test]
async fn send_with_implicit_tls_465() -> TestResult {
    send_and_check(MailpitTls::Implicit, Security::Tls).await
}

#[tokio::test]
async fn send_with_required_starttls_587() -> TestResult {
    send_and_check(MailpitTls::StartTls, Security::StartTls).await
}
