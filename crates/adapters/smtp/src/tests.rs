//! Tests contre le faux serveur SMTP scripté.

use std::time::Duration;

use norelly_domain::auth::AuthError;
use norelly_domain::connection::ConnectionError;
use norelly_domain::credentials::{Credentials, Secret, Username};
use norelly_domain::endpoint::{Host, Security, ServerEndpoint};
use norelly_domain::ports::{ConnectRequest, MailSender, OutgoingMessage, SendError};
use norelly_domain::security::{PlaintextConsent, PlaintextPolicy};
use norelly_test_support::certs::TestCa;
use norelly_test_support::fake_server::{Outcome, Script, ScriptedServer, smtp};
use norelly_tls::{TlsClientFactory, TrustRoots};

use super::{SmtpSender, Timeouts};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const PASSWORD: &str = "Tr3s-S3cret!";
/// base64("\0alice\0Tr3s-S3cret!").
const PLAIN_B64: &str = "AGFsaWNlAFRyM3MtUzNjcmV0IQ==";

fn sender(ca: &TestCa) -> SmtpSender {
    let tls = TlsClientFactory::new(TrustRoots::Custom(vec![ca.root()]))
        .with_handshake_timeout(Duration::from_secs(5));
    SmtpSender::new(tls).with_timeouts(Timeouts {
        connect: Duration::from_secs(5),
        response: Duration::from_secs(5),
    })
}

fn request(port: u16, security: Security, plaintext: PlaintextPolicy) -> ConnectRequest {
    let host = Host::parse("localhost").unwrap_or_else(|_| panic!("hôte"));
    ConnectRequest {
        endpoint: ServerEndpoint::new(host, port, security)
            .unwrap_or_else(|_| panic!("point de connexion")),
        credentials: Credentials::new(
            Username::parse("alice").unwrap_or_else(|_| panic!("utilisateur")),
            Secret::new(PASSWORD.to_owned()),
        )
        .unwrap_or_else(|_| panic!("identifiants")),
        certificate_exceptions: vec![],
        plaintext,
    }
}

fn assert_no_auth_sent(outcome: &Outcome) {
    for needle in ["AUTH", PASSWORD, PLAIN_B64] {
        assert!(
            !outcome.transcript.contains(needle.as_bytes()),
            "envoyé avant TLS : {needle}\n{}",
            outcome.transcript.text()
        );
    }
}

/// Un seul EHLO sur le canal final (celui de lettre) : un second est
/// refusé par certains serveurs (GMX : 503), et le script échoue s'il
/// arrive.
fn ehlo_once(script: Script, extensions: &[&str]) -> Script {
    script
        .expect("EHLO [127.0.0.1]")
        .send(smtp::ehlo(extensions))
}

fn auth_and_quit(script: Script) -> Script {
    script
        .expect(format!("AUTH PLAIN {PLAIN_B64}"))
        .send("235 2.7.0 Authentification reussie\r\n")
        .expect("QUIT")
        .send("221 2.0.0 Au revoir\r\n")
        .expect_eof()
}

#[tokio::test]
async fn implicit_tls_verify() -> TestResult {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost"])?;
    let script = ehlo_once(
        Script::new().send(smtp::greeting()),
        &["AUTH PLAIN LOGIN", "SIZE 1000"],
    );
    let server = ScriptedServer::start_tls(auth_and_quit(script), cert.server_config()?).await?;
    let caps = sender(&ca)
        .verify(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await?;
    // Capacités reprises de l'EHLO de lettre : mécanismes AUTH et
    // extensions qu'il connaît (SIZE n'en fait pas partie).
    assert!(caps.has("AUTH=PLAIN") && caps.has("AUTH=LOGIN"));
    server.finish().await.result?;
    Ok(())
}

#[tokio::test]
async fn starttls_verify_rereads_capabilities() -> TestResult {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost"])?;
    let script = Script::new()
        .send(smtp::greeting())
        // En clair : STARTTLS seulement.
        .expect("EHLO")
        .send(smtp::ehlo(&["STARTTLS"]))
        .expect("STARTTLS")
        .send("220 2.0.0 Pret pour TLS\r\n")
        .starttls();
    let script = ehlo_once(script, &["AUTH PLAIN", "8BITMIME"]);
    let server = ScriptedServer::start(auth_and_quit(script), Some(cert.server_config()?)).await?;
    let caps = sender(&ca)
        .verify(&request(
            server.port(),
            Security::StartTls,
            PlaintextPolicy::deny_all(),
        ))
        .await?;
    assert!(caps.has("8BITMIME"));
    assert!(!caps.starttls(), "capacités d'avant TLS conservées");
    server.finish().await.result?;
    Ok(())
}

#[tokio::test]
async fn starttls_stripping_sends_no_credentials() -> TestResult {
    let ca = TestCa::new()?;
    let server = ScriptedServer::start(
        Script::new()
            .send(smtp::greeting())
            .expect("EHLO")
            .send(smtp::ehlo(&["AUTH PLAIN LOGIN"]))
            .expect_eof(),
        None,
    )
    .await?;
    let result = sender(&ca)
        .verify(&request(
            server.port(),
            Security::StartTls,
            PlaintextPolicy::deny_all(),
        ))
        .await;
    assert!(
        matches!(result, Err(ConnectionError::StartTlsUnavailable)),
        "{result:?}"
    );
    let outcome = server.finish().await;
    assert_no_auth_sent(&outcome);
    outcome.result?;
    Ok(())
}

#[tokio::test]
async fn injection_after_starttls_is_rejected() -> TestResult {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost"])?;
    let server = ScriptedServer::start(
        Script::new()
            .send(smtp::greeting())
            .expect("EHLO")
            .send(smtp::ehlo(&["STARTTLS"]))
            .expect("STARTTLS")
            .send("220 Go\r\n250-injecte\r\n250 AUTH PLAIN\r\n")
            .expect_eof(),
        Some(cert.server_config()?),
    )
    .await?;
    let result = sender(&ca)
        .verify(&request(
            server.port(),
            Security::StartTls,
            PlaintextPolicy::deny_all(),
        ))
        .await;
    assert!(
        matches!(result, Err(ConnectionError::PlaintextInjection)),
        "{result:?}"
    );
    let outcome = server.finish().await;
    assert_no_auth_sent(&outcome);
    outcome.result?;
    Ok(())
}

#[tokio::test]
async fn starttls_refused_by_server() -> TestResult {
    let ca = TestCa::new()?;
    let server = ScriptedServer::start(
        Script::new()
            .send(smtp::greeting())
            .expect("EHLO")
            .send(smtp::ehlo(&["STARTTLS", "AUTH PLAIN"]))
            .expect("STARTTLS")
            .send("454 4.7.0 TLS indisponible\r\n")
            .expect_eof(),
        None,
    )
    .await?;
    let result = sender(&ca)
        .verify(&request(
            server.port(),
            Security::StartTls,
            PlaintextPolicy::deny_all(),
        ))
        .await;
    assert!(
        matches!(result, Err(ConnectionError::StartTlsRejected(_))),
        "{result:?}"
    );
    let outcome = server.finish().await;
    assert_no_auth_sent(&outcome);
    outcome.result?;
    Ok(())
}

#[tokio::test]
async fn plaintext_refused_without_consent() -> TestResult {
    let ca = TestCa::new()?;
    let result = sender(&ca)
        .verify(&request(1, Security::Plain, PlaintextPolicy::deny_all()))
        .await;
    assert!(
        matches!(result, Err(ConnectionError::PlaintextRefused(_))),
        "{result:?}"
    );
    Ok(())
}

#[tokio::test]
async fn plaintext_with_consent() -> TestResult {
    let ca = TestCa::new()?;
    let script = Script::new()
        .send(smtp::greeting())
        .expect("EHLO")
        .send(smtp::ehlo(&["AUTH PLAIN"]));
    let script = ehlo_once(script, &["AUTH PLAIN"]);
    let server = ScriptedServer::start(auth_and_quit(script), None).await?;
    let port = server.port();
    let consent = PlaintextPolicy::deny_all().with_consent(PlaintextConsent {
        host: Host::parse("localhost")?,
        port,
    });
    sender(&ca)
        .verify(&request(port, Security::Plain, consent))
        .await?;
    server.finish().await.result?;
    Ok(())
}

#[tokio::test]
async fn rejected_credentials() -> TestResult {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost"])?;
    let script = ehlo_once(Script::new().send(smtp::greeting()), &["AUTH PLAIN"])
        .expect("AUTH PLAIN")
        .send("535 5.7.8 Identifiants invalides\r\n")
        .expect_any();
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let result = sender(&ca)
        .verify(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await;
    assert!(
        matches!(
            result,
            Err(ConnectionError::Authentication(AuthError::Rejected { .. }))
        ),
        "{result:?}"
    );
    let _ = server.finish().await;
    Ok(())
}

#[tokio::test]
async fn send_message() -> TestResult {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost"])?;
    let script = ehlo_once(Script::new().send(smtp::greeting()), &["AUTH PLAIN"])
        .expect("AUTH PLAIN")
        .send("235 ok\r\n")
        .expect("MAIL FROM:<alice@example.com>")
        .send("250 ok\r\n")
        .expect("RCPT TO:<bob@example.com>")
        .send("250 ok\r\n")
        .expect("DATA")
        .send("354 go\r\n")
        .expect_data_until(".")
        .send("250 queued\r\n")
        .expect("QUIT")
        .send("221 bye\r\n")
        .expect_eof();
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let message = OutgoingMessage {
        from: "alice@example.com".into(),
        recipients: vec!["bob@example.com".into()],
        raw: b"Subject: essai\r\n\r\n.ligne avec point\r\nfin\r\n".to_vec(),
    };
    sender(&ca)
        .send(
            &request(server.port(), Security::Tls, PlaintextPolicy::deny_all()),
            &message,
        )
        .await?;
    let outcome = server.finish().await;
    outcome.result?;
    // Transparence SMTP : la ligne commençant par un point est doublée.
    assert!(outcome.transcript.contains(b"\r\n..ligne avec point\r\n"));
    Ok(())
}

#[tokio::test]
async fn invalid_address_is_rejected_before_connecting() -> TestResult {
    let ca = TestCa::new()?;
    let message = OutgoingMessage {
        from: "pas une adresse".into(),
        recipients: vec![],
        raw: vec![],
    };
    let result = sender(&ca)
        .send(
            &request(1, Security::Tls, PlaintextPolicy::deny_all()),
            &message,
        )
        .await;
    assert!(
        matches!(result, Err(SendError::InvalidAddress(_))),
        "{result:?}"
    );
    Ok(())
}

#[tokio::test]
async fn malformed_replies_do_not_panic() -> TestResult {
    let ca = TestCa::new()?;
    let greetings: [&[u8]; 7] = [
        b"garbage\r\n",
        b"554 non merci\r\n",
        b"220-debut\r\n250 fin\r\n",
        b"22\r\n",
        b"220 sans CR\n",
        b"\xff\xfe\r\n",
        b"",
    ];
    for greeting in greetings {
        let server =
            ScriptedServer::start(Script::new().send_raw(greeting.to_vec()).close(), None).await?;
        let result = sender(&ca)
            .verify(&request(
                server.port(),
                Security::StartTls,
                PlaintextPolicy::deny_all(),
            ))
            .await;
        assert!(
            matches!(
                result,
                Err(ConnectionError::Protocol(_) | ConnectionError::Closed)
            ),
            "{:?} → {result:?}",
            String::from_utf8_lossy(greeting)
        );
        server.finish().await.result?;
    }
    Ok(())
}
