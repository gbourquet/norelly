//! Tests contre le faux serveur IMAP scripté.

use std::io::Write;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use norelly_domain::auth::AuthError;
use norelly_domain::connection::{ConnectionError, TimeoutStage};
use norelly_domain::credentials::{Credentials, Secret, Username};
use norelly_domain::endpoint::{Host, Security, ServerEndpoint};
use norelly_domain::ports::{ConnectRequest, RemoteMailboxConnector, RemoteMailboxSession};
use norelly_domain::security::{PlaintextConsent, PlaintextPolicy};
use norelly_test_support::certs::TestCa;
use norelly_test_support::fake_server::{Outcome, Script, ScriptedServer, imap};
use norelly_tls::{TlsClientFactory, TrustRoots};

use super::{ImapConnector, Timeouts};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const USER: &str = "alice";
const PASSWORD: &str = "Tr3s-S3cret!";
/// base64("\0alice\0Tr3s-S3cret!") : réponse SASL PLAIN.
const PLAIN_B64: &str = "AGFsaWNlAFRyM3MtUzNjcmV0IQ==";
/// base64("Tr3s-S3cret!") : réponse SASL LOGIN au second défi.
const PASSWORD_B64: &str = "VHIzcy1TM2NyZXQh";

/// Tout ce qui trahirait les identifiants dans un flux ou un journal.
const SECRETS: [&str; 3] = [PASSWORD, PLAIN_B64, PASSWORD_B64];

struct Fixture {
    ca: TestCa,
    connector: ImapConnector,
}

fn fixture() -> Result<Fixture, Box<dyn std::error::Error>> {
    let ca = TestCa::new()?;
    let tls = TlsClientFactory::new(TrustRoots::Custom(vec![ca.root()]))
        .with_handshake_timeout(Duration::from_secs(5));
    let connector = ImapConnector::new(tls).with_timeouts(Timeouts {
        connect: Duration::from_secs(5),
        open: Duration::from_secs(5),
        password_auth: Duration::from_secs(5),
        open_total: Duration::from_secs(60),
        response: Duration::from_secs(5),
    });
    Ok(Fixture { ca, connector })
}

fn request(port: u16, security: Security, plaintext: PlaintextPolicy) -> ConnectRequest {
    let host = Host::parse("localhost").unwrap_or_else(|_| panic!("hote"));
    let endpoint =
        ServerEndpoint::new(host, port, security).unwrap_or_else(|_| panic!("point de connexion"));
    let credentials = Credentials::new(
        Username::parse(USER).unwrap_or_else(|_| panic!("utilisateur")),
        Secret::new(PASSWORD.to_owned()),
    )
    .unwrap_or_else(|_| panic!("identifiants"));
    ConnectRequest {
        endpoint,
        credentials,
        certificate_exceptions: vec![],
        plaintext,
    }
}

fn consent(port: u16) -> PlaintextPolicy {
    PlaintextPolicy::deny_all().with_consent(PlaintextConsent {
        host: Host::parse("localhost").unwrap_or_else(|_| panic!("hote")),
        port,
    })
}

fn assert_no_credentials_sent(outcome: &Outcome) {
    for secret in SECRETS {
        assert!(
            !outcome.transcript.contains(secret.as_bytes()),
            "identifiant envoye : {secret}\n{}",
            outcome.transcript.text()
        );
    }
    let text = outcome.transcript.text().to_uppercase();
    assert!(!text.contains("LOGIN "), "LOGIN envoye : {text}");
    assert!(
        !text.contains("AUTHENTICATE"),
        "AUTHENTICATE envoye : {text}"
    );
}

/// Fin de script commune : capacités après authentification, puis LOGOUT.
fn after_auth(script: Script) -> Script {
    script
        .expect("CAPABILITY")
        .send(imap::capability("IMAP4rev1 IDLE MOVE UIDPLUS"))
        .expect("LOGOUT")
        .send("* BYE au revoir\r\n{tag} OK LOGOUT termine\r\n")
        .expect_eof()
}

#[tokio::test]
async fn implicit_tls_with_authenticate_plain_and_sasl_ir() -> TestResult {
    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let script = Script::new()
        .send(imap::greeting("IMAP4rev1 AUTH=PLAIN SASL-IR"))
        .expect(format!("AUTHENTICATE PLAIN {PLAIN_B64}"))
        .send(imap::ok("authentifie"));
    let server = ScriptedServer::start_tls(after_auth(script), cert.server_config()?).await?;
    let session = f
        .connector
        .connect(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await?;
    assert!(session.capabilities().has("IDLE"));
    assert!(!session.capabilities().has("AUTH=PLAIN"));
    session.logout().await?;
    server.finish().await.result?;
    Ok(())
}

#[tokio::test]
async fn starttls_then_capabilities_reread_then_authenticate_without_sasl_ir() -> TestResult {
    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let script = Script::new()
        // Avant TLS : LOGINDISABLED, aucun mécanisme.
        .send(imap::greeting("IMAP4rev1 STARTTLS LOGINDISABLED"))
        .expect("STARTTLS")
        .send(imap::ok("Debut de la negociation TLS"))
        .starttls()
        // Après TLS : capacités différentes, relues par le client.
        .expect("CAPABILITY")
        .send(imap::capability("IMAP4rev1 AUTH=PLAIN"))
        .expect("AUTHENTICATE PLAIN")
        .send("+ \r\n")
        .expect_data_with(PLAIN_B64)
        .send(imap::ok("authentifie"));
    let server = ScriptedServer::start(after_auth(script), Some(cert.server_config()?)).await?;
    let session = f
        .connector
        .connect(&request(
            server.port(),
            Security::StartTls,
            PlaintextPolicy::deny_all(),
        ))
        .await?;
    session.logout().await?;
    server.finish().await.result?;
    Ok(())
}

#[tokio::test]
async fn login_command_when_no_mechanism_is_advertised() -> TestResult {
    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let script = Script::new()
        .send(imap::greeting("IMAP4rev1"))
        .expect(format!("LOGIN alice {PASSWORD}"))
        .send(imap::ok("connecte"));
    let server = ScriptedServer::start_tls(after_auth(script), cert.server_config()?).await?;
    let session = f
        .connector
        .connect(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await?;
    session.logout().await?;
    server.finish().await.result?;
    Ok(())
}

#[tokio::test]
async fn authenticate_login_answers_both_challenges() -> TestResult {
    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let script = Script::new()
        .send(imap::greeting("IMAP4rev1 LOGINDISABLED AUTH=LOGIN"))
        .expect("AUTHENTICATE LOGIN")
        .send("+ VXNlcm5hbWU6\r\n")
        .expect_data_with("YWxpY2U=")
        .send("+ UGFzc3dvcmQ6\r\n")
        .expect_data_with(PASSWORD_B64)
        .send(imap::ok("authentifie"));
    let server = ScriptedServer::start_tls(after_auth(script), cert.server_config()?).await?;
    let session = f
        .connector
        .connect(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await?;
    session.logout().await?;
    server.finish().await.result?;
    Ok(())
}

/// STARTTLS stripping : un attaquant retire STARTTLS des capacités. Le
/// client s'arrête sans rien envoyer d'autre.
#[tokio::test]
async fn starttls_stripping_in_greeting_sends_no_credentials() -> TestResult {
    let f = fixture()?;
    let server = ScriptedServer::start(
        Script::new()
            .send(imap::greeting("IMAP4rev1 AUTH=PLAIN"))
            .expect_eof(),
        None,
    )
    .await?;
    let result = f
        .connector
        .connect(&request(
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
    assert_no_credentials_sent(&outcome);
    outcome.result?;
    Ok(())
}

#[tokio::test]
async fn starttls_stripping_in_capability_response_sends_no_credentials() -> TestResult {
    let f = fixture()?;
    let server = ScriptedServer::start(
        Script::new()
            .send("* OK pret\r\n")
            .expect("CAPABILITY")
            .send(imap::capability("IMAP4rev1 AUTH=PLAIN"))
            .expect_eof(),
        None,
    )
    .await?;
    let result = f
        .connector
        .connect(&request(
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
    assert_no_credentials_sent(&outcome);
    outcome.result?;
    Ok(())
}

#[tokio::test]
async fn starttls_refused_by_server_sends_no_credentials() -> TestResult {
    let f = fixture()?;
    let server = ScriptedServer::start(
        Script::new()
            .send(imap::greeting("IMAP4rev1 STARTTLS AUTH=PLAIN"))
            .expect("STARTTLS")
            .send("{tag} NO TLS indisponible\r\n")
            .expect_eof(),
        None,
    )
    .await?;
    let result = f
        .connector
        .connect(&request(
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
    assert_no_credentials_sent(&outcome);
    outcome.result?;
    Ok(())
}

/// Injection après STARTTLS : des réponses envoyées en clair juste après le
/// OK seraient lues comme venant du canal chiffré. Le client refuse.
#[tokio::test]
async fn injection_after_starttls_is_rejected() -> TestResult {
    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let server = ScriptedServer::start(
        Script::new()
            .send(imap::greeting("IMAP4rev1 STARTTLS LOGINDISABLED"))
            .expect("STARTTLS")
            .send("{tag} OK Debut TLS\r\n* CAPABILITY IMAP4rev1 AUTH=PLAIN\r\n")
            .expect_eof(),
        Some(cert.server_config()?),
    )
    .await?;
    let result = f
        .connector
        .connect(&request(
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
    assert_no_credentials_sent(&outcome);
    outcome.result?;
    Ok(())
}

/// Injection cachée derrière un faux littéral : le OK de STARTTLS finit par
/// `{5}`, mais un texte d'état n'annonce pas de littéral. Les cinq octets
/// envoyés en clair restent des données injectées, refusées comme telles.
#[tokio::test]
async fn injection_behind_a_literal_like_starttls_reply_is_rejected() -> TestResult {
    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let server = ScriptedServer::start(
        Script::new()
            .send(imap::greeting("IMAP4rev1 STARTTLS LOGINDISABLED"))
            .expect("STARTTLS")
            .send("{tag} OK Debut TLS {5}\r\nXXXXX")
            .expect_eof(),
        Some(cert.server_config()?),
    )
    .await?;
    let result = f
        .connector
        .connect(&request(
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
    assert_no_credentials_sent(&outcome);
    outcome.result?;
    Ok(())
}

#[tokio::test]
async fn logindisabled_without_mechanism_is_respected() -> TestResult {
    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let server = ScriptedServer::start_tls(
        Script::new()
            .send(imap::greeting("IMAP4rev1 LOGINDISABLED"))
            .expect_eof(),
        cert.server_config()?,
    )
    .await?;
    let result = f
        .connector
        .connect(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await;
    assert_eq!(
        result.err(),
        Some(ConnectionError::Authentication(AuthError::LoginDisabled))
    );
    let outcome = server.finish().await;
    assert_no_credentials_sent(&outcome);
    outcome.result?;
    Ok(())
}

#[tokio::test]
async fn plaintext_refused_without_consent_before_any_connection() -> TestResult {
    let f = fixture()?;
    // Aucun serveur : la politique doit refuser avant toute connexion.
    let result = f
        .connector
        .connect(&request(1, Security::Plain, PlaintextPolicy::deny_all()))
        .await;
    assert!(
        matches!(result, Err(ConnectionError::PlaintextRefused(_))),
        "{result:?}"
    );
    Ok(())
}

#[tokio::test]
async fn plaintext_with_consent_authenticates_in_clear() -> TestResult {
    let f = fixture()?;
    let script = Script::new()
        .send(imap::greeting("IMAP4rev1"))
        .expect("LOGIN alice")
        .send(imap::ok("connecte"));
    let server = ScriptedServer::start(after_auth(script), None).await?;
    let port = server.port();
    let session = f
        .connector
        .connect(&request(port, Security::Plain, consent(port)))
        .await?;
    session.logout().await?;
    server.finish().await.result?;
    Ok(())
}

/// STARTTLS est obligatoire dès qu'il est annoncé, même en clair consenti.
#[tokio::test]
async fn plaintext_with_consent_still_upgrades_when_starttls_is_offered() -> TestResult {
    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let script = Script::new()
        .send(imap::greeting("IMAP4rev1 STARTTLS"))
        .expect("STARTTLS")
        .send(imap::ok("TLS"))
        .starttls()
        .expect("CAPABILITY")
        .send(imap::capability("IMAP4rev1"))
        .expect("LOGIN alice")
        .send(imap::ok("connecte"));
    let server = ScriptedServer::start(after_auth(script), Some(cert.server_config()?)).await?;
    let port = server.port();
    let session = f
        .connector
        .connect(&request(port, Security::Plain, consent(port)))
        .await?;
    session.logout().await?;
    server.finish().await.result?;
    Ok(())
}

#[tokio::test]
async fn preauth_in_clear_is_refused_in_starttls_mode() -> TestResult {
    let f = fixture()?;
    let server = ScriptedServer::start(
        Script::new()
            .send("* PREAUTH [CAPABILITY IMAP4rev1] deja connecte\r\n")
            .expect_eof(),
        None,
    )
    .await?;
    let result = f
        .connector
        .connect(&request(
            server.port(),
            Security::StartTls,
            PlaintextPolicy::deny_all(),
        ))
        .await;
    assert!(
        matches!(result, Err(ConnectionError::StartTlsUnavailable)),
        "{result:?}"
    );
    server.finish().await.result?;
    Ok(())
}

#[tokio::test]
async fn rejected_credentials_are_reported() -> TestResult {
    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let server = ScriptedServer::start_tls(
        Script::new()
            .send(imap::greeting("IMAP4rev1 AUTH=PLAIN SASL-IR"))
            .expect("AUTHENTICATE")
            .send("{tag} NO [AUTHENTICATIONFAILED] Identifiants\r\n\x1b[31m invalides\r\n")
            .expect_eof(),
        cert.server_config()?,
    )
    .await?;
    let result = f
        .connector
        .connect(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await;
    match result {
        Err(ConnectionError::Authentication(AuthError::Rejected { message })) => {
            assert_eq!(message, "Identifiants");
        }
        other => panic!("refus d'authentification attendu : {other:?}"),
    }
    server.finish().await.result?;
    Ok(())
}

#[tokio::test]
async fn silent_server_times_out() -> TestResult {
    let ca = TestCa::new()?;
    let connector = ImapConnector::new(TlsClientFactory::new(TrustRoots::Custom(vec![ca.root()])))
        .with_timeouts(Timeouts {
            connect: Duration::from_secs(5),
            open: Duration::from_millis(200),
            password_auth: Duration::from_millis(200),
            open_total: Duration::from_secs(60),
            response: Duration::from_millis(200),
        });
    let server =
        ScriptedServer::start(Script::new().wait(Duration::from_secs(1)).close(), None).await?;
    let result = connector
        .connect(&request(
            server.port(),
            Security::StartTls,
            PlaintextPolicy::deny_all(),
        ))
        .await;
    assert_eq!(
        result.err(),
        Some(ConnectionError::Timeout(TimeoutStage::BeforeEncryption))
    );
    Ok(())
}

/// Connecteur au délai d'ouverture court et au délai de réponse long.
fn quick_opening(ca: &TestCa) -> ImapConnector {
    let tls = TlsClientFactory::new(TrustRoots::Custom(vec![ca.root()]))
        .with_handshake_timeout(Duration::from_secs(5));
    ImapConnector::new(tls).with_timeouts(Timeouts {
        connect: Duration::from_secs(5),
        open: Duration::from_millis(300),
        password_auth: Duration::from_millis(1500),
        open_total: Duration::from_secs(60),
        response: Duration::from_secs(10),
    })
}

/// Silence d'un serveur lent, bien au-delà des délais d'ouverture de
/// [`quick_opening`].
const SLOW_SERVER: Duration = Duration::from_secs(9);
/// Abandon attendu au délai d'ouverture : en deçà de [`SLOW_SERVER`] et du
/// délai de réponse (10 s), avec la marge d'une machine lente (CI Windows,
/// poignée de main TLS en mode debug).
const ABANDONED_WITHIN: Duration = Duration::from_secs(6);

/// Serveur muet après `AUTHENTICATE` : abandon au plancher de
/// l'authentification par mot de passe, pas au délai de réponse ;
/// authentification interrompue, jamais rejouée.
#[tokio::test]
async fn server_silent_after_authenticate_is_abandoned_at_the_opening_delay() -> TestResult {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost"])?;
    let script = Script::new()
        .send(imap::greeting("IMAP4rev1 AUTH=PLAIN SASL-IR"))
        .expect(format!("AUTHENTICATE PLAIN {PLAIN_B64}"))
        .wait(SLOW_SERVER)
        .send(imap::ok("authentifie"));
    let server = ScriptedServer::start_tls(after_auth(script), cert.server_config()?).await?;
    let started = tokio::time::Instant::now();
    let result = quick_opening(&ca)
        .connect(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await;
    assert_eq!(
        result.err(),
        Some(ConnectionError::Authentication(AuthError::Interrupted))
    );
    // Pas avant le plancher de l'authentification par mot de passe.
    assert!(
        (Duration::from_millis(1500)..ABANDONED_WITHIN).contains(&started.elapsed()),
        "{:?}",
        started.elapsed()
    );
    Ok(())
}

/// Serveur muet aux capacités relues après l'authentification : c'est
/// encore l'ouverture.
#[tokio::test]
async fn server_silent_on_capabilities_after_login_is_abandoned_early() -> TestResult {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost"])?;
    let script = Script::new()
        .send(imap::greeting("IMAP4rev1 AUTH=PLAIN SASL-IR"))
        .expect(format!("AUTHENTICATE PLAIN {PLAIN_B64}"))
        .send(imap::ok("authentifie"))
        .expect("CAPABILITY")
        .wait(SLOW_SERVER)
        .send(imap::capability("IMAP4rev1"));
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let started = tokio::time::Instant::now();
    let result = quick_opening(&ca)
        .connect(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await;
    assert_eq!(
        result.err(),
        Some(ConnectionError::Timeout(TimeoutStage::Response))
    );
    assert!(
        started.elapsed() < ABANDONED_WITHIN,
        "{:?}",
        started.elapsed()
    );
    Ok(())
}

/// Refus d'un mot de passe retardé par le serveur (pénalité après des
/// échecs) au-delà du délai d'ouverture : le refus est lu, pas pris pour un
/// silence qui ferait renvoyer le mot de passe.
#[tokio::test]
async fn delayed_password_refusal_is_read() -> TestResult {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost"])?;
    let script = Script::new()
        .send(imap::greeting("IMAP4rev1 AUTH=PLAIN SASL-IR"))
        .expect(format!("AUTHENTICATE PLAIN {PLAIN_B64}"))
        .wait(Duration::from_millis(800))
        .send("{tag} NO [AUTHENTICATIONFAILED] Identifiants invalides\r\n")
        .expect_eof();
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let result = quick_opening(&ca)
        .connect(&request(
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
        "{:?}",
        result.err()
    );
    server.finish().await.result?;
    Ok(())
}

/// Salutation envoyée octet par octet, chacun avant le délai d'ouverture :
/// l'ouverture entière reste bornée.
#[tokio::test]
async fn trickled_greeting_is_bounded_by_the_total_opening_delay() -> TestResult {
    let ca = TestCa::new()?;
    let mut script = Script::new();
    for byte in b"* OK salutation interminable".repeat(4) {
        script = script.send_raw(vec![byte]).wait(Duration::from_millis(100));
    }
    let server = ScriptedServer::start(script, None).await?;
    let connector = ImapConnector::new(TlsClientFactory::new(TrustRoots::Custom(vec![ca.root()])))
        .with_timeouts(Timeouts {
            connect: Duration::from_secs(5),
            open: Duration::from_millis(300),
            password_auth: Duration::from_millis(300),
            open_total: Duration::from_secs(1),
            response: Duration::from_secs(10),
        })
        // Adresse IPv4 explicite : sous Windows, « localhost » essaie
        // d'abord `::1`, où le faux serveur n'écoute pas, et ce refus prend
        // environ une seconde, soit toute la durée totale d'ouverture.
        .connecting_to(std::net::SocketAddr::from(([127, 0, 0, 1], server.port())));
    let started = tokio::time::Instant::now();
    let result = connector
        .connect(&request(
            server.port(),
            Security::StartTls,
            PlaintextPolicy::deny_all(),
        ))
        .await;
    assert_eq!(
        result.err(),
        Some(ConnectionError::Timeout(TimeoutStage::BeforeEncryption))
    );
    assert!(
        started.elapsed() < ABANDONED_WITHIN,
        "{:?}",
        started.elapsed()
    );
    Ok(())
}

/// Salutation en retard sur un canal STARTTLS : c'est encore l'ouverture.
#[tokio::test]
async fn late_plaintext_greeting_is_abandoned_at_the_opening_delay() -> TestResult {
    let ca = TestCa::new()?;
    let script = Script::new()
        .wait(SLOW_SERVER)
        .send(imap::greeting("IMAP4rev1 STARTTLS"));
    let server = ScriptedServer::start(script, None).await?;
    let started = tokio::time::Instant::now();
    let result = quick_opening(&ca)
        .connect(&request(
            server.port(),
            Security::StartTls,
            PlaintextPolicy::deny_all(),
        ))
        .await;
    assert_eq!(
        result.err(),
        Some(ConnectionError::Timeout(TimeoutStage::BeforeEncryption))
    );
    assert!(
        started.elapsed() < ABANDONED_WITHIN,
        "{:?}",
        started.elapsed()
    );
    Ok(())
}

/// Une fois la session ouverte, une réponse lente (grosse boîte) est
/// attendue jusqu'au délai de réponse.
#[tokio::test]
async fn opened_session_waits_for_a_slow_reply() -> TestResult {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost"])?;
    let script = Script::new()
        .send(imap::greeting("IMAP4rev1 AUTH=PLAIN SASL-IR"))
        .expect(format!("AUTHENTICATE PLAIN {PLAIN_B64}"))
        .send(imap::ok("authentifie"))
        .expect("CAPABILITY")
        .send(imap::capability("IMAP4rev1"))
        .expect("LIST")
        .wait(Duration::from_secs(1))
        .send("* LIST () \"/\" INBOX\r\n{tag} OK LIST termine\r\n")
        .expect("LOGOUT")
        .send("* BYE au revoir\r\n{tag} OK LOGOUT termine\r\n")
        .expect_eof();
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let mut session = quick_opening(&ca)
        .connect(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await?;
    let mailboxes = session.list_mailboxes().await?;
    assert_eq!(mailboxes.len(), 1);
    session.logout().await?;
    server.finish().await.result?;
    Ok(())
}

/// Réponses hostiles ou malformées : erreur propre, jamais de panique.
#[tokio::test]
async fn malformed_responses_do_not_panic() -> TestResult {
    let f = fixture()?;
    let greetings: [&[u8]; 9] = [
        b"garbage\r\n",
        b"* OK [CAPABILITY\r\n",
        b"\xff\xfe\x00\r\n",
        b"* OK {99999999999}\r\n",
        b"* OK sans CR\n",
        b"* BYE va-t'en\r\n",
        b"+ continuation inattendue\r\n",
        b"A1 OK salutation etiquetee\r\n",
        b"",
    ];
    for greeting in greetings {
        let server =
            ScriptedServer::start(Script::new().send_raw(greeting.to_vec()).close(), None).await?;
        let result = f
            .connector
            .connect(&request(
                server.port(),
                Security::StartTls,
                PlaintextPolicy::deny_all(),
            ))
            .await;
        assert!(
            matches!(
                result,
                Err(ConnectionError::Protocol(_) | ConnectionError::ClosedBeforeEncryption)
            ),
            "{:?}  {result:?}",
            String::from_utf8_lossy(greeting)
        );
        server.finish().await.result?;
    }

    let responses: [&str; 5] = [
        "* CAPABILITY \x00\r\n{tag} OK\r\n",
        "{tag} OK sans donnees\r\n",
        "* CAPABILITY IMAP4rev1\r\nZZZ OK mauvaise etiquette\r\n",
        "{tag} BAD non\r\n",
        "* 1 FETCH (UID {5}\r\n",
    ];
    for response in responses {
        let server = ScriptedServer::start(
            Script::new()
                .send("* OK pret\r\n")
                .expect("CAPABILITY")
                .send(response)
                .close(),
            None,
        )
        .await?;
        let result = f
            .connector
            .connect(&request(
                server.port(),
                Security::StartTls,
                PlaintextPolicy::deny_all(),
            ))
            .await;
        assert!(
            matches!(
                result,
                Err(ConnectionError::Protocol(_) | ConnectionError::ClosedBeforeEncryption)
            ),
            "{response:?}  {result:?}"
        );
        server.finish().await.result?;
    }
    Ok(())
}

/// Journal capturé en mémoire.
#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl CapturedLogs {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap_or_else(PoisonError::into_inner)).into_owned()
    }
}

/// Les identifiants n'apparaissent ni dans les journaux `tracing` (niveau
/// le plus verbeux), ni dans les `Debug` des requêtes, sessions et erreurs.
#[tokio::test]
async fn credentials_never_appear_in_logs_or_debug() -> TestResult {
    let logs = CapturedLogs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let mut debug_dumps = Vec::new();

    // 1. STARTTLS + AUTHENTICATE PLAIN, puis LOGOUT.
    let script = Script::new()
        .send(imap::greeting("IMAP4rev1 STARTTLS"))
        .expect("STARTTLS")
        .send(imap::ok("TLS"))
        .starttls()
        .expect("CAPABILITY")
        .send(imap::capability("IMAP4rev1 AUTH=PLAIN SASL-IR"))
        .expect("AUTHENTICATE")
        .send(imap::ok("authentifie"));
    let server = ScriptedServer::start(after_auth(script), Some(cert.server_config()?)).await?;
    let req = request(
        server.port(),
        Security::StartTls,
        PlaintextPolicy::deny_all(),
    );
    debug_dumps.push(format!("{req:?} {req:#?}"));
    let session = f.connector.connect(&req).await?;
    debug_dumps.push(format!("{session:?}"));
    session.logout().await?;
    server.finish().await.result?;

    // 2. LOGIN refusé : l'erreur ne contient pas le secret.
    let server = ScriptedServer::start_tls(
        Script::new()
            .send(imap::greeting("IMAP4rev1"))
            .expect("LOGIN")
            .send("{tag} NO refuse\r\n")
            .expect_eof(),
        cert.server_config()?,
    )
    .await?;
    let req = request(server.port(), Security::Tls, PlaintextPolicy::deny_all());
    let err = f.connector.connect(&req).await.err();
    debug_dumps.push(format!("{err:?}"));
    server.finish().await.result?;

    let logs = logs.text();
    assert!(
        logs.contains("session IMAP ouverte"),
        "journal vide : {logs}"
    );
    assert!(
        logs.contains(USER),
        "l'utilisateur doit rester visible : {logs}"
    );
    for secret in SECRETS {
        assert!(
            !logs.contains(secret),
            "secret dans les journaux : {secret}\n{logs}"
        );
        for dump in &debug_dumps {
            assert!(!dump.contains(secret), "secret dans un Debug : {dump}");
        }
    }
    Ok(())
}

/// Flot sans fin de réponses non étiquetées avant STARTTLS : erreur bornée,
/// pas de saturation mémoire.
#[tokio::test]
async fn untagged_flood_before_tls_is_bounded() -> TestResult {
    let f = fixture()?;
    let flood = "* CAPABILITY IMAP4rev1 X\r\n".repeat(10_100);
    let server = ScriptedServer::start(
        Script::new()
            .send("* OK pret\r\n")
            .expect("CAPABILITY")
            .send(flood)
            .close(),
        None,
    )
    .await?;
    let result = f
        .connector
        .connect(&request(
            server.port(),
            Security::StartTls,
            PlaintextPolicy::deny_all(),
        ))
        .await;
    assert_eq!(
        result.err(),
        Some(ConnectionError::Protocol(
            "trop de réponses non étiquetées".into()
        ))
    );
    let _ = server.finish().await;
    Ok(())
}

/// Ligne énorme sans fin de ligne avant TLS : refusée dès 64 Kio.
#[tokio::test]
async fn huge_line_before_tls_is_rejected() -> TestResult {
    let f = fixture()?;
    let server = ScriptedServer::start(
        Script::new()
            .send(format!("* OK {}", "A".repeat(512 * 1024)))
            .wait(Duration::from_secs(2))
            .close(),
        None,
    )
    .await?;
    let result = f
        .connector
        .connect(&request(
            server.port(),
            Security::StartTls,
            PlaintextPolicy::deny_all(),
        ))
        .await;
    assert!(
        matches!(result, Err(ConnectionError::Protocol(_))),
        "{result:?}"
    );
    let _ = server.finish().await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Synchronisation
// ---------------------------------------------------------------------------

/// Session authentifiée (PLAIN + SASL-IR) puis `script`.
fn authenticated(script: Script) -> Script {
    Script::new()
        .send(imap::greeting(
            "IMAP4rev1 AUTH=PLAIN SASL-IR IDLE ENABLE CONDSTORE QRESYNC",
        ))
        .expect("AUTHENTICATE PLAIN")
        .send(imap::ok("authentifie"))
        .expect("CAPABILITY")
        .send(imap::capability("IMAP4rev1 IDLE ENABLE CONDSTORE QRESYNC"))
        .steps_from(script)
}

/// Un IDLE abandonné (futur annulé) ne laisse pas la session bloquée : la
/// fermeture est immédiate, sans attendre le délai de l'IDLE.
#[tokio::test]
async fn cancelled_idle_does_not_block_logout() -> TestResult {
    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let script = authenticated(
        Script::new()
            .expect("IDLE")
            .send("+ idling\r\n")
            .wait(Duration::from_secs(8)),
    );
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let mut session = f
        .connector
        .connect(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await?;
    let cancelled = tokio::time::timeout(
        Duration::from_millis(300),
        session.idle(Duration::from_secs(600)),
    )
    .await;
    assert!(cancelled.is_err(), "IDLE toujours en cours");
    // Toute nouvelle commande échoue aussitôt.
    let started = std::time::Instant::now();
    assert!(session.search_all_uids().await.is_err());
    tokio::time::timeout(Duration::from_secs(1), session.logout())
        .await
        .map_err(|_| "LOGOUT bloqué après un IDLE annulé")??;
    assert!(started.elapsed() < Duration::from_secs(1));
    Ok(())
}

/// Sous-dossiers relus avant suppression : `LIST "" "<parent>/%"` en UTF-7
/// modifié ; un nom illisible ou 8 bits compte, un nom sous le parent à la
/// casse près (ASCII ou non, autre codage) compte, seul un voisin
/// sûrement hors du parent, rapporté par un joker de son nom (« 50% »
/// couvre « 50x »), ne compte pas.
#[tokio::test]
async fn subfolders_counted_under_the_parent_only() -> TestResult {
    use norelly_domain::mailbox::MailboxName;

    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let script = authenticated(
        Script::new()
            .expect("LIST \"\" 50%/%")
            .send(
                "* LIST () \"/\" \"50%/a\"\r\n\
                 * LIST (\\Noselect) \"/\" \"50%/&AGE-\"\r\n\
                 * LIST () \"/\" \"50x/b\"\r\n\
                 {tag} OK LIST termine\r\n",
            )
            .expect("LIST \"\" &AMk-t&AOk-/%")
            .send(
                "* LIST () \"/\" \"&AMk-t&AOk-/x\"\r\n\
                 * LIST () \"/\" \"&AOk-t&AOk-/y\"\r\n\
                 * LIST () \"/\" \"&AMk-&AHQ-&AOk-/w\"\r\n\
                 * LIST () \"/\" {7}\r\n\u{c9}t\u{e9}/z\r\n\
                 {tag} OK LIST termine\r\n",
            )
            .expect("LIST \"\" Projets/%")
            .send("* LIST () \"/\" \"PROJETS/a\"\r\n{tag} OK LIST termine\r\n")
            .expect("LOGOUT")
            .send("* BYE au revoir\r\n{tag} OK termine\r\n")
            .expect_eof(),
    );
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let mut session = f
        .connector
        .connect(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await
        .map_err(|e| format!("CONNECT : {e}"))?;
    let count = session
        .count_subfolders(&MailboxName::parse("50%")?, '/')
        .await
        .map_err(|e| format!("LIST : {e}"))?;
    assert_eq!(count, 2, "nom illisible compté, voisin écarté");
    let count = session
        .count_subfolders(&MailboxName::parse("Été")?, '/')
        .await
        .map_err(|e| format!("LIST : {e}"))?;
    assert_eq!(count, 4, "casse non ASCII, autre codage, nom 8 bits");
    let count = session
        .count_subfolders(&MailboxName::parse("Projets")?, '/')
        .await
        .map_err(|e| format!("LIST : {e}"))?;
    assert_eq!(count, 1, "casse ASCII");
    session.logout().await?;
    server.finish().await.result?;
    Ok(())
}

/// Réponses hostiles à LIST et EXAMINE : noms invalides ignorés, VANISHED
/// démesuré borné, FETCH sans UID ignoré, codes lus.
#[tokio::test]
async fn hostile_sync_responses_are_contained() -> TestResult {
    use norelly_domain::mailbox::{MailboxName, ModSeq, Uid, UidValidity};
    use norelly_domain::sync::{QresyncParams, SelectRequest};

    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let script = authenticated(
        Script::new()
            .expect("ENABLE")
            .send("* ENABLED CONDSTORE QRESYNC\r\n{tag} OK active\r\n")
            .expect("LIST")
            .send(
                "* LIST () \"/\" INBOX\r\n\
                 * LIST (\\Sent) \"/\" \"Envoy&AOk-s\"\r\n\
                 * LIST () \"/\" \"&AGE-\"\r\n\
                 * LIST () \"/\" \"a&IC4-b\"\r\n\
                 {tag} OK LIST termine\r\n",
            )
            .expect("EXAMINE INBOX (QRESYNC (7 10))")
            // Plages énormes répétées : ni dépliées ni reparcourues.
            .send("* VANISHED (EARLIER) 1:1999999\r\n".repeat(10_000))
            .send(
                "* OK [UIDVALIDITY 7] ok\r\n\
                 * OK [UIDNEXT 100] ok\r\n\
                 * OK [HIGHESTMODSEQ 20] ok\r\n\
                 * 3 EXISTS\r\n\
                 * VANISHED (EARLIER) 1:4294967295\r\n\
                 * 1 FETCH (FLAGS (\\Seen) MODSEQ (15))\r\n\
                 * 2 FETCH (UID 5 FLAGS (\\Flagged) MODSEQ (16))\r\n\
                 {tag} OK [READ-ONLY] termine\r\n",
            )
            .expect("LOGOUT")
            .send("* BYE au revoir\r\n{tag} OK termine\r\n")
            .expect_eof(),
    );
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let mut session = f
        .connector
        .connect(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await
        .map_err(|e| format!("CONNECT : {e}"))?;
    let ext = session
        .enable_sync_extensions()
        .await
        .map_err(|e| format!("ENABLE : {e}"))?;
    assert!(ext.qresync);
    let names: Vec<String> = session
        .list_mailboxes()
        .await
        .map_err(|e| format!("LIST : {e}"))?
        .into_iter()
        .map(|m| m.name.to_string())
        .collect();
    assert_eq!(names, ["INBOX", "Envoyés"], "noms invalides ignorés");
    let request = SelectRequest {
        condstore: false,
        qresync: Some(QresyncParams {
            uid_validity: UidValidity::new(7).ok_or("uidvalidity")?,
            modseq: ModSeq::new(10).ok_or("modseq")?,
        }),
    };
    let started = std::time::Instant::now();
    let selected = session
        .select(&MailboxName::inbox(), &request)
        .await
        .map_err(|e| format!("EXAMINE : {e}"))?;
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(selected.uid_validity, UidValidity::new(7));
    assert_eq!(selected.uid_next, Uid::new(100));
    assert_eq!(selected.highest_modseq, ModSeq::new(20));
    assert_eq!(selected.exists, 3);
    assert_eq!(
        selected.vanished.ranges(),
        [(Uid::new(1).ok_or("uid")?, Uid::new(u32::MAX).ok_or("uid")?)],
        "VANISHED gardé en plages"
    );
    assert_eq!(
        selected
            .changed
            .iter()
            .map(|c| c.uid.get())
            .collect::<Vec<_>>(),
        [5],
        "FETCH sans UID ignoré"
    );
    session.logout().await?;
    server.finish().await.result?;
    Ok(())
}

/// BODYSTRUCTURE `message/rfc822` imbriquée à 500 niveaux : `imap-codec`
/// déborderait de la pile (arrêt du processus). Le préfiltre la refuse.
fn deep_rfc822(levels: usize) -> String {
    let mut s = String::new();
    for _ in 0..levels {
        s.push_str(
            "(\"message\" \"rfc822\" NIL NIL NIL \"7bit\" 1 \
             (NIL NIL NIL NIL NIL NIL NIL NIL NIL NIL) ",
        );
    }
    s.push_str("(\"text\" \"plain\" NIL NIL NIL \"7bit\" 1 1)");
    for _ in 0..levels {
        s.push_str(" 1)");
    }
    s
}

/// Multipart imbriqué sur `levels` niveaux (au-delà de la limite de
/// récursion d'`imap-codec` : réponse indécodable).
fn deep_multipart(levels: usize) -> String {
    let mut s = "(\"text\" \"plain\" NIL NIL NIL \"7bit\" 1 1)".to_owned();
    for _ in 0..levels {
        s = format!("({s} \"mixed\")");
    }
    s
}

#[tokio::test]
async fn deeply_nested_response_before_tls_is_refused_without_crash() -> TestResult {
    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let script = Script::new()
        .send("* OK pret\r\n")
        .expect("CAPABILITY")
        .send(format!(
            "* 1 FETCH (BODYSTRUCTURE {})\r\n* CAPABILITY IMAP4rev1 STARTTLS\r\n{{tag}} OK\r\n",
            deep_rfc822(500)
        ));
    let server = ScriptedServer::start(script, Some(cert.server_config()?)).await?;
    let result = f
        .connector
        .connect(&request(
            server.port(),
            Security::StartTls,
            PlaintextPolicy::deny_all(),
        ))
        .await;
    assert!(
        matches!(&result, Err(ConnectionError::Protocol(m)) if m.contains("imbriqu")),
        "{result:?}"
    );
    assert_no_credentials_sent(&server.finish().await);
    Ok(())
}

#[tokio::test]
async fn hostile_bodystructures_do_not_block_metadata() -> TestResult {
    use norelly_domain::mailbox::Uid;

    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let header = "Subject: poison\r\n\r\n";
    let fetch = |uid: u32, extra: &str| {
        format!(
            "* {uid} FETCH (UID {uid} FLAGS () INTERNALDATE \"02-Nov-2026 10:00:00 +0000\" \
             RFC822.SIZE 10 {extra}BODY[HEADER] {{{}}}\r\n{header})\r\n",
            header.len()
        )
    };
    let script = authenticated(
        Script::new()
            .expect("UID FETCH 1:2")
            // UID 1 : rfc822 à 500 niveaux ; UID 2 : multipart à 12 niveaux.
            .send(format!(
                "{}{}{{tag}} OK termine\r\n",
                fetch(1, &format!("BODYSTRUCTURE {} ", deep_rfc822(500))),
                fetch(2, &format!("BODYSTRUCTURE {} ", deep_multipart(12)))
            ))
            // Seconde demande, sans structure.
            .expect("UID FETCH")
            .send(format!(
                "{}{}{{tag}} OK termine\r\n",
                fetch(1, ""),
                fetch(2, "")
            ))
            .expect("LOGOUT")
            .send("* BYE au revoir\r\n{tag} OK termine\r\n")
            .expect_eof(),
    );
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let mut session = f
        .connector
        .connect(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await
        .map_err(|e| format!("connect : {e}"))?;
    let uids: Vec<Uid> = [1, 2].into_iter().filter_map(Uid::new).collect();
    let metas = session
        .fetch_meta(&uids)
        .await
        .map_err(|e| format!("fetch_meta : {e}"))?;
    assert_eq!(
        metas.iter().map(|m| m.uid.get()).collect::<Vec<_>>(),
        [1, 2]
    );
    assert!(metas.iter().all(|m| m.structure.is_none()));
    assert!(metas.iter().all(|m| m.headers.subject == "poison"));
    session
        .logout()
        .await
        .map_err(|e| format!("logout : {e}"))?;
    let outcome = server.finish().await;
    let text = outcome.transcript.text();
    outcome.result?;
    assert!(
        text.matches("BODYSTRUCTURE").count() == 1,
        "la seconde demande omet BODYSTRUCTURE : {text}"
    );
    Ok(())
}

/// `EXPUNGE` sans UIDPLUS : chaque réponse `* n EXPUNGE` est comptée, pour
/// détecter les messages marqués par un autre client entre-temps.
#[tokio::test]
async fn expunge_counts_untagged_expunge_responses() -> TestResult {
    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let script = authenticated(
        Script::new()
            .expect("EXPUNGE")
            .send("* 3 EXPUNGE\r\n* 3 EXPUNGE\r\n* 7 EXISTS\r\n{tag} OK EXPUNGE termine\r\n"),
    );
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let mut session = f
        .connector
        .connect(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await?;
    assert_eq!(session.expunge_all().await?, 2);
    session.logout().await?;
    server.finish().await.result?;
    Ok(())
}

/// `APPEND` : littéral envoyé après la continuation, drapeaux `\Seen` et
/// `\Draft`, UID lu dans `APPENDUID` ; refus `NO` → `Rejected`.
#[tokio::test]
async fn append_sends_a_literal_and_reads_appenduid() -> TestResult {
    use norelly_domain::connection::ConnectionError;
    use norelly_domain::mailbox::{MailboxName, Uid};

    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let raw = b"Subject: brouillon\r\n\r\ncorps du brouillon\r\n";
    let script = authenticated(
        Script::new()
            .expect(format!(
                "APPEND Brouillons (\\Seen \\Draft) {{{}}}",
                raw.len()
            ))
            .send("+ pret\r\n")
            .expect_data_until("corps du brouillon")
            // Fin de la commande, après le littéral.
            .expect_data()
            .send("{tag} OK [APPENDUID 7 42] APPEND termine\r\n")
            .expect("APPEND Envoy&AOk-s (\\Seen) {")
            .send("+ pret\r\n")
            .expect_data_until("corps du brouillon")
            .expect_data()
            .send("{tag} OK APPEND termine\r\n")
            .expect("APPEND Absente")
            .send("{tag} NO [TRYCREATE] boite absente\r\n")
            // Littéral jamais envoyé : `imap-next` ne libère la commande
            // que sur `BAD` ; la session est fermée sans LOGOUT.
            .expect_eof(),
    );
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let mut session = f
        .connector
        .connect(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await?;
    let drafts = MailboxName::parse("Brouillons")?;
    let copy = session.append(&drafts, raw, true).await?;
    assert_eq!(
        copy.map(|c| (c.validity.get(), c.uid)),
        Some((7, Uid::new(42).ok_or("uid")?))
    );
    let sent = MailboxName::parse("Envoyés")?;
    assert_eq!(session.append(&sent, raw, false).await?, None);
    let missing = MailboxName::parse("Absente")?;
    let refused = session.append(&missing, raw, true).await;
    assert!(
        matches!(refused, Err(ConnectionError::Rejected(_))),
        "{refused:?}"
    );
    assert!(
        session.search_all_uids().await.is_err(),
        "session inutilisable après un littéral refusé"
    );
    session.logout().await?;
    server.finish().await.result?;
    Ok(())
}

/// Avec QRESYNC activé, le serveur annonce les suppressions par
/// `* VANISHED` (RFC 7162) au lieu de `* n EXPUNGE` : elles sont comptées
/// aussi.
#[tokio::test]
async fn expunge_counts_vanished_responses_under_qresync() -> TestResult {
    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let script = authenticated(
        Script::new()
            .expect("EXPUNGE")
            .send("* VANISHED 3:4,7\r\n* VANISHED (EARLIER) 1\r\n{tag} OK EXPUNGE termine\r\n")
            .expect("LOGOUT")
            .send("* BYE au revoir\r\n{tag} OK LOGOUT termine\r\n"),
    );
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let mut session = f
        .connector
        .connect(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await?;
    // VANISHED (EARLIER) ne décrit pas cette commande : non compté.
    assert_eq!(session.expunge_all().await?, 3);
    session.logout().await?;
    server.finish().await.result?;
    Ok(())
}

/// Session authentifiée sur un script ; renvoie la session et le serveur.
async fn scripted_session(
    script: Script,
) -> Result<(super::ImapSession, ScriptedServer), Box<dyn std::error::Error>> {
    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let server = ScriptedServer::start_tls(authenticated(script), cert.server_config()?).await?;
    let session = f
        .connector
        .connect(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await?;
    Ok((session, server))
}

/// IDLE : un changement signalé (`EXISTS`) termine l'attente par DONE.
#[tokio::test]
async fn idle_returns_on_a_signalled_change() -> TestResult {
    use norelly_domain::ports::IdleOutcome;
    let script = Script::new()
        .expect("IDLE")
        .send("+ idling\r\n* 1 RECENT\r\n* 5 EXISTS\r\n")
        .expect_data_with("DONE")
        .send("{tag} OK IDLE termine\r\n");
    let (mut session, server) = scripted_session(script).await?;
    let outcome = session.idle(Duration::from_secs(30)).await?;
    assert_eq!(outcome, IdleOutcome::Changed);
    drop(session);
    let _ = server.finish().await;
    Ok(())
}

/// IDLE sans changement : DONE à l'échéance, `Timeout`.
#[tokio::test]
async fn idle_times_out_without_change() -> TestResult {
    use norelly_domain::ports::IdleOutcome;
    let script = Script::new()
        .expect("IDLE")
        .send("+ idling\r\n")
        .expect_data_with("DONE")
        .send("* OK encore la\r\n{tag} OK IDLE termine\r\n");
    let (mut session, server) = scripted_session(script).await?;
    let outcome = session.idle(Duration::from_millis(300)).await?;
    assert_eq!(outcome, IdleOutcome::Timeout);
    drop(session);
    let _ = server.finish().await;
    Ok(())
}

/// IDLE refusé par le serveur, ou BYE pendant l'attente : erreurs nettes.
#[tokio::test]
async fn idle_refusal_and_bye_are_errors() -> TestResult {
    use norelly_domain::connection::ConnectionError;
    let refused = Script::new()
        .expect("IDLE")
        .send("{tag} BAD IDLE indisponible\r\n");
    let (mut session, server) = scripted_session(refused).await?;
    let result = session.idle(Duration::from_secs(5)).await;
    assert!(
        matches!(result, Err(ConnectionError::Protocol(_))),
        "{result:?}"
    );
    drop(session);
    let _ = server.finish().await;

    let bye = Script::new()
        .expect("IDLE")
        .send("+ idling\r\n* BYE arret du serveur\r\n");
    let (mut session, server) = scripted_session(bye).await?;
    let result = session.idle(Duration::from_secs(5)).await;
    assert_eq!(result, Err(ConnectionError::Closed));
    drop(session);
    let _ = server.finish().await;
    Ok(())
}

/// Une réponse indécodable pendant
/// `EXAMINE … QRESYNC` (Stalwart : `VANISHED (EARLIER) 0:1`, UID 0
/// interdit) rend la sélection incomplète ; le plan compare alors les UID.
#[tokio::test]
async fn undecodable_examine_response_marks_the_selection_incomplete() -> TestResult {
    use norelly_domain::mailbox::{MailboxName, ModSeq, UidValidity};
    use norelly_domain::sync::{QresyncParams, SelectRequest};

    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let examine = |vanished: &str| {
        format!(
            "* OK [UIDVALIDITY 7] ok\r\n\
             * OK [HIGHESTMODSEQ 20] ok\r\n\
             * 1 EXISTS\r\n\
             {vanished}\
             {{tag}} OK [READ-ONLY] termine\r\n"
        )
    };
    let script = authenticated(
        Script::new()
            .expect("ENABLE")
            .send("* ENABLED CONDSTORE QRESYNC\r\n{tag} OK active\r\n")
            .expect("EXAMINE INBOX (QRESYNC (7 10))")
            .send(examine("* VANISHED (EARLIER) 0:1\r\n"))
            .expect("EXAMINE INBOX (QRESYNC (7 10))")
            .send(examine("* VANISHED (EARLIER) 1\r\n"))
            .expect("LOGOUT")
            .send("* BYE au revoir\r\n{tag} OK termine\r\n")
            .expect_eof(),
    );
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let mut session = f
        .connector
        .connect(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await
        .map_err(|e| format!("CONNECT : {e}"))?;
    session
        .enable_sync_extensions()
        .await
        .map_err(|e| format!("ENABLE : {e}"))?;
    let request = SelectRequest {
        condstore: false,
        qresync: Some(QresyncParams {
            uid_validity: UidValidity::new(7).ok_or("uidvalidity")?,
            modseq: ModSeq::new(10).ok_or("modseq")?,
        }),
    };
    let broken = session
        .select(&MailboxName::inbox(), &request)
        .await
        .map_err(|e| format!("EXAMINE : {e}"))?;
    assert!(broken.incomplete, "réponse écartée : sélection incomplète");
    assert_eq!(broken.highest_modseq, ModSeq::new(20));
    let sound = session
        .select(&MailboxName::inbox(), &request)
        .await
        .map_err(|e| format!("EXAMINE : {e}"))?;
    assert!(!sound.incomplete, "compteur remis à zéro à chaque commande");
    session
        .logout()
        .await
        .map_err(|e| format!("LOGOUT : {e}"))?;
    Ok(())
}

/// `CREATE` : nom envoyé en UTF-7 modifié ; refus `NO` → `Rejected`, la
/// session reste utilisable.
#[tokio::test]
async fn create_mailbox_encodes_the_name_and_reports_refusal() -> TestResult {
    use norelly_domain::connection::ConnectionError;
    use norelly_domain::mailbox::MailboxName;

    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let script = authenticated(
        Script::new()
            .expect("CREATE Voyages/&AMk-t&AOk-")
            .send("{tag} OK CREATE termine\r\n")
            .expect("CREATE Projets")
            .send("{tag} NO [ALREADYEXISTS] existe deja\r\n")
            // Guillemet, barre oblique inverse, espace, accolade : chaîne
            // entre guillemets échappée, jamais d'argument supplémentaire.
            .expect("CREATE \"a\\\" b\\\\c{d}\"")
            .send("{tag} OK CREATE termine\r\n")
            .expect("EXPUNGE")
            .send("{tag} OK EXPUNGE termine\r\n"),
    );
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let mut session = f
        .connector
        .connect(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await?;
    session
        .create_mailbox(&MailboxName::parse("Voyages/Été")?)
        .await?;
    let refused = session
        .create_mailbox(&MailboxName::parse("Projets")?)
        .await;
    assert!(
        matches!(refused, Err(ConnectionError::Rejected(_))),
        "{refused:?}"
    );
    session
        .create_mailbox(&MailboxName::parse("a\" b\\c{d}")?)
        .await?;
    assert_eq!(session.expunge_all().await?, 0, "session utilisable");
    session.logout().await?;
    server.finish().await.result?;
    Ok(())
}

/// `RENAME` et `DELETE` : noms envoyés en UTF-7 modifié ; refus `NO` →
/// `Rejected`, la session reste utilisable.
#[tokio::test]
async fn rename_and_delete_mailbox_encode_names_and_report_refusal() -> TestResult {
    use norelly_domain::connection::ConnectionError;
    use norelly_domain::mailbox::MailboxName;

    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let script = authenticated(
        Script::new()
            .expect("RENAME Projets Voyages/&AMk-t&AOk-")
            .send("{tag} OK RENAME termine\r\n")
            .expect("RENAME Absent Autre")
            .send("{tag} NO [NONEXISTENT] inconnu\r\n")
            .expect("DELETE \"a b\"")
            .send("{tag} OK DELETE termine\r\n")
            .expect("DELETE Absent")
            .send("{tag} NO [NONEXISTENT] inconnu\r\n")
            .expect("EXPUNGE")
            .send("{tag} OK EXPUNGE termine\r\n"),
    );
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let mut session = f
        .connector
        .connect(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await?;
    session
        .rename_mailbox(
            &MailboxName::parse("Projets")?,
            &MailboxName::parse("Voyages/Été")?,
        )
        .await?;
    let refused = session
        .rename_mailbox(
            &MailboxName::parse("Absent")?,
            &MailboxName::parse("Autre")?,
        )
        .await;
    assert!(
        matches!(refused, Err(ConnectionError::Rejected(_))),
        "{refused:?}"
    );
    session.delete_mailbox(&MailboxName::parse("a b")?).await?;
    let refused = session.delete_mailbox(&MailboxName::parse("Absent")?).await;
    assert!(
        matches!(refused, Err(ConnectionError::Rejected(_))),
        "{refused:?}"
    );
    assert_eq!(session.expunge_all().await?, 0, "session utilisable");
    session.logout().await?;
    server.finish().await.result?;
    Ok(())
}

/// Session authentifiée annonçant `capabilities` après connexion.
fn authenticated_with(capabilities: &str, script: Script) -> Script {
    Script::new()
        .send(imap::greeting("IMAP4rev1 AUTH=PLAIN SASL-IR"))
        .expect("AUTHENTICATE PLAIN")
        .send(imap::ok("authentifie"))
        .expect("CAPABILITY")
        .send(imap::capability(capabilities))
        .steps_from(script)
}

const SELECTED: &str = "* OK [UIDVALIDITY 7] ok\r\n* 1 EXISTS\r\n{tag} OK [READ-WRITE] termine\r\n";

/// Une boîte sélectionnée est quittée avant `RENAME` ou `DELETE` (certains
/// serveurs, dont Dovecot, coupent la session si la boîte sélectionnée
/// disparaît) : `UNSELECT` si annoncé, qui n'efface rien, sinon `EXAMINE`
/// de la boîte de réception (jamais `CLOSE`, qui effacerait les messages
/// marqués supprimés).
#[tokio::test]
async fn selected_mailbox_is_left_before_rename_or_delete() -> TestResult {
    use norelly_domain::mailbox::MailboxName;

    for (capabilities, leave) in [
        ("IMAP4rev1 UNSELECT", "UNSELECT"),
        ("IMAP4rev1", "EXAMINE INBOX"),
    ] {
        let f = fixture()?;
        let cert = f.ca.issue(&["localhost"])?;
        let left = if leave == "UNSELECT" {
            "{tag} OK UNSELECT termine\r\n"
        } else {
            "* OK [UIDVALIDITY 1] ok\r\n* 0 EXISTS\r\n{tag} OK [READ-ONLY] termine\r\n"
        };
        let script = authenticated_with(
            capabilities,
            Script::new()
                .expect("SELECT Projets")
                .send(SELECTED)
                .expect(leave)
                .send(left)
                .expect("RENAME Projets Chantiers")
                .send("{tag} OK RENAME termine\r\n")
                // Plus rien de sélectionné par Norelly : rien à quitter.
                .expect("DELETE Chantiers")
                .send("{tag} OK DELETE termine\r\n")
                .expect("SELECT Notes")
                .send(SELECTED)
                .expect(leave)
                .send(left)
                .expect("DELETE Notes")
                .send("{tag} OK DELETE termine\r\n")
                .expect("LOGOUT")
                .send("* BYE au revoir\r\n{tag} OK LOGOUT termine\r\n")
                .expect_eof(),
        );
        let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
        let mut session = f
            .connector
            .connect(&request(
                server.port(),
                Security::Tls,
                PlaintextPolicy::deny_all(),
            ))
            .await?;
        let projets = MailboxName::parse("Projets")?;
        let chantiers = MailboxName::parse("Chantiers")?;
        session.select_writable(&projets).await?;
        session.rename_mailbox(&projets, &chantiers).await?;
        session.delete_mailbox(&chantiers).await?;
        let notes = MailboxName::parse("Notes")?;
        session.select_writable(&notes).await?;
        session.delete_mailbox(&notes).await?;
        session.logout().await?;
        server.finish().await.result?;
    }
    Ok(())
}

/// Texte de refus en UTF-8 brut (Dovecot y cite le nom de la boîte, comme
/// IMAP4rev2 le permet) : refus reçu tout de suite, caractères non ASCII
/// remplacés, au lieu d'une réponse écartée et d'un délai dépassé.
#[tokio::test]
async fn utf8_refusal_text_is_received_as_a_refusal() -> TestResult {
    use norelly_domain::connection::ConnectionError;
    use norelly_domain::mailbox::MailboxName;

    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let script = authenticated(
        Script::new()
            .expect("DELETE &AMk-t&AOk-")
            .send("{tag} NO [NONEXISTENT] Mailbox doesn't exist: Été\r\n")
            .expect("CREATE Projets")
            .send("* OK [ALERT] Quota presque atteint : réduisez\r\n{tag} OK CREATE termine\r\n")
            .expect("LOGOUT")
            .send("* BYE au revoir\r\n{tag} OK LOGOUT termine\r\n")
            .expect_eof(),
    );
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let mut session = f
        .connector
        .connect(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await?;
    let refused = tokio::time::timeout(
        Duration::from_secs(5),
        session.delete_mailbox(&MailboxName::parse("Été")?),
    )
    .await?;
    let Err(ConnectionError::Rejected(text)) = refused else {
        panic!("refus attendu : {refused:?}");
    };
    assert!(text.contains("Mailbox doesn't exist: ??t??"), "{text}");
    session
        .create_mailbox(&MailboxName::parse("Projets")?)
        .await?;
    session.logout().await?;
    server.finish().await.result?;
    Ok(())
}

/// Texte d'état qui cite un nom de dossier finissant par `{3}` (créé par un
/// autre client) : ce n'est pas l'annonce d'un littéral, la grammaire n'en
/// prévoit pas dans un texte d'état. Réponse reçue tout de suite, au lieu
/// d'attendre trois octets jusqu'au délai.
#[tokio::test]
async fn status_text_ending_like_a_literal_is_not_a_literal() -> TestResult {
    use norelly_domain::connection::ConnectionError;
    use norelly_domain::mailbox::MailboxName;

    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let script = authenticated(
        Script::new()
            .expect("DELETE \"Notes{3}\"")
            .send("{tag} NO [NONEXISTENT] Mailbox doesn't exist: Notes{3}\r\n")
            .expect("CREATE Projets")
            .send("* OK [ALERT] Quota atteint : Notes{3+}\r\n{tag} OK Created Notes{3}\r\n")
            .expect("LOGOUT")
            .send("* BYE au revoir\r\n{tag} OK LOGOUT termine\r\n")
            .expect_eof(),
    );
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let mut session = f
        .connector
        .connect(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await?;
    let refused = tokio::time::timeout(
        Duration::from_secs(5),
        session.delete_mailbox(&MailboxName::parse("Notes{3}")?),
    )
    .await?;
    let Err(ConnectionError::Rejected(text)) = refused else {
        panic!("refus attendu : {refused:?}");
    };
    assert!(text.contains("Mailbox doesn't exist: Notes{3}"), "{text}");
    tokio::time::timeout(
        Duration::from_secs(5),
        session.create_mailbox(&MailboxName::parse("Projets")?),
    )
    .await??;
    session.logout().await?;
    server.finish().await.result?;
    Ok(())
}

const TOKEN: &str = "tok-Abc.123";
/// base64("user=alice@outlook.com\x01auth=Bearer tok-Abc.123\x01\x01").
const XOAUTH2_B64: &str = "dXNlcj1hbGljZUBvdXRsb29rLmNvbQFhdXRoPUJlYXJlciB0b2stQWJjLjEyMwEB";
/// Défi d'erreur de Microsoft (JSON en base64) après un jeton refusé.
const XOAUTH2_ERROR_CHALLENGE: &str = "+ eyJzdGF0dXMiOiI0MDEiLCJzY2hlbWVzIjoiYmVhcmVyIiwic2NvcGUiOiJodHRwczovL291dGxvb2sub2ZmaWNlLmNvbS9JTUFQLkFjY2Vzc0FzVXNlci5BbGwifQ==\r\n";

fn microsoft_imap_host() -> &'static str {
    norelly_domain::oauth::OAuthProvider::Microsoft
        .config()
        .imap_host
}

/// Compte Microsoft : le point de connexion porte l'hôte IMAP du
/// fournisseur (seule destination permise au jeton) ; la connexion TCP est
/// redirigée vers le faux serveur local, dont le certificat est émis pour
/// cet hôte.
fn oauth_request(port: u16) -> ConnectRequest {
    let host = Host::parse(microsoft_imap_host()).unwrap_or_else(|_| panic!("hote"));
    ConnectRequest {
        endpoint: ServerEndpoint::new(host, port, Security::Tls)
            .unwrap_or_else(|_| panic!("point de connexion")),
        credentials: Credentials::oauth2(
            Username::parse("alice@outlook.com").unwrap_or_else(|_| panic!("utilisateur")),
            Secret::new(TOKEN.to_owned()),
            norelly_domain::oauth::OAuthProvider::Microsoft,
        )
        .unwrap_or_else(|_| panic!("identifiants")),
        certificate_exceptions: vec![],
        plaintext: PlaintextPolicy::deny_all(),
    }
}

/// Connexion du compte Microsoft au faux serveur, puis `LOGOUT` si elle
/// réussit ; renvoie le résultat et la transcription du serveur.
async fn oauth_connect(
    script: Script,
) -> Result<(Result<(), ConnectionError>, Outcome), Box<dyn std::error::Error>> {
    let f = fixture()?;
    let cert = f.ca.issue(&[microsoft_imap_host()])?;
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let connector = f
        .connector
        .connecting_to(std::net::SocketAddr::from(([127, 0, 0, 1], server.port())));
    let result = match connector.connect(&oauth_request(server.port())).await {
        Ok(session) => session.logout().await,
        Err(e) => Err(e),
    };
    Ok((result, server.finish().await))
}

#[tokio::test]
async fn xoauth2_sends_the_initial_response_with_sasl_ir() -> TestResult {
    let script = Script::new()
        .send(imap::greeting("IMAP4rev1 AUTH=XOAUTH2 SASL-IR"))
        .expect(format!("AUTHENTICATE XOAUTH2 {XOAUTH2_B64}"))
        .send(imap::ok("authentifie"));
    let (result, outcome) = oauth_connect(after_auth(script)).await?;
    result?;
    outcome.result?;
    Ok(())
}

/// Sans SASL-IR, la réponse initiale part au premier défi (vide).
#[tokio::test]
async fn xoauth2_without_sasl_ir_answers_the_first_challenge() -> TestResult {
    let script = Script::new()
        .send(imap::greeting("IMAP4rev1 AUTH=XOAUTH2"))
        .expect("AUTHENTICATE XOAUTH2")
        .send("+ \r\n")
        .expect_data_with(XOAUTH2_B64)
        .send(imap::ok("authentifie"));
    let (result, outcome) = oauth_connect(after_auth(script)).await?;
    result?;
    outcome.result?;
    Ok(())
}

/// Jeton refusé : le serveur envoie l'erreur en JSON dans un défi ; le
/// client répond par une ligne vide et obtient le refus, qui est un refus
/// d'authentification (reconnexion nécessaire).
#[tokio::test]
async fn xoauth2_error_challenge_gets_an_empty_answer_then_is_a_rejection() -> TestResult {
    let script = Script::new()
        .send(imap::greeting("IMAP4rev1 AUTH=XOAUTH2 SASL-IR"))
        .expect(format!("AUTHENTICATE XOAUTH2 {XOAUTH2_B64}"))
        .send(XOAUTH2_ERROR_CHALLENGE)
        .expect_data()
        .send("{tag} NO [AUTHENTICATIONFAILED] AUTHENTICATE failed.\r\n")
        .expect_eof();
    let (result, outcome) = oauth_connect(script).await?;
    match result {
        Err(ConnectionError::Authentication(AuthError::Rejected { message })) => {
            assert_eq!(message, "AUTHENTICATE failed.");
        }
        other => panic!("refus d'authentification attendu : {other:?}"),
    }
    assert!(
        outcome
            .transcript
            .contains(format!("{XOAUTH2_B64}\r\n\r\n").as_bytes()),
        "réponse vide au défi attendue :\n{}",
        outcome.transcript.text()
    );
    outcome.result?;
    Ok(())
}

/// Un compte OAuth sur un serveur qui ne propose pas XOAUTH2 n'envoie ni
/// `AUTHENTICATE`, ni `LOGIN`, ni jeton.
#[tokio::test]
async fn oauth_account_sends_nothing_without_xoauth2() -> TestResult {
    let script = Script::new()
        .send(imap::greeting("IMAP4rev1 AUTH=PLAIN SASL-IR"))
        .expect_eof();
    let (result, outcome) = oauth_connect(script).await?;
    assert!(
        matches!(
            result,
            Err(ConnectionError::Authentication(
                AuthError::NoSupportedMechanism { .. }
            ))
        ),
        "{result:?}"
    );
    let text = outcome.transcript.text().to_uppercase();
    for needle in [
        "AUTHENTICATE",
        "LOGIN",
        &TOKEN.to_uppercase(),
        &XOAUTH2_B64.to_uppercase(),
    ] {
        assert!(!text.contains(needle), "envoyé : {needle}\n{text}");
    }
    outcome.result?;
    Ok(())
}

/// Manières dont un serveur coupe la connexion : fermeture, réinitialisation,
/// `* BYE` puis fermeture.
fn cuts() -> [fn(Script) -> Script; 3] {
    [Script::close, Script::reset, |s: Script| {
        s.send("* BYE trop d'echecs\r\n").close()
    }]
}

/// Connexion en TLS implicite au faux serveur ; résultat de l'ouverture.
async fn connect_tls(
    script: Script,
) -> Result<Result<(), ConnectionError>, Box<dyn std::error::Error>> {
    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let result = f
        .connector
        .connect(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await
        .map(|_| ());
    let _ = server.finish().await;
    Ok(result)
}

/// Coupure juste après `AUTHENTICATE` : certains serveurs répondent ainsi à
/// un mot de passe faux. Authentification interrompue, jamais prise pour
/// une coupure passagère (qui ferait renvoyer le mot de passe).
#[tokio::test]
async fn cut_during_authenticate_is_an_interrupted_authentication() -> TestResult {
    for cut in cuts() {
        let script = cut(Script::new()
            .send(imap::greeting("IMAP4rev1 AUTH=PLAIN SASL-IR"))
            .expect(format!("AUTHENTICATE PLAIN {PLAIN_B64}")));
        assert_eq!(
            connect_tls(script).await?.err(),
            Some(ConnectionError::Authentication(AuthError::Interrupted))
        );
    }
    Ok(())
}

/// Même règle pour la commande `LOGIN`.
#[tokio::test]
async fn cut_during_login_is_an_interrupted_authentication() -> TestResult {
    for cut in cuts() {
        let script = cut(Script::new()
            .send(imap::greeting("IMAP4rev1"))
            .expect(format!("LOGIN alice {PASSWORD}")));
        assert_eq!(
            connect_tls(script).await?.err(),
            Some(ConnectionError::Authentication(AuthError::Interrupted))
        );
    }
    Ok(())
}

/// Coupure une fois le canal chiffré, avant l'envoi des identifiants :
/// coupure passagère ordinaire, quelle que soit la manière.
#[tokio::test]
async fn cut_after_encryption_before_auth_is_a_closed_connection() -> TestResult {
    for cut in cuts() {
        let script = cut(Script::new().send("* OK pret\r\n").expect("CAPABILITY"));
        assert_eq!(
            connect_tls(script).await?.err(),
            Some(ConnectionError::Closed)
        );
    }
    Ok(())
}

/// Coupure avant la fin de la mise en place du chiffrement : échange en
/// clair avant STARTTLS, ou poignée de main après son acceptation ; nommée
/// comme telle, et aucun identifiant n'est parti.
#[tokio::test]
async fn cut_before_encryption_is_named_as_such() -> TestResult {
    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let before_starttls = Script::new().send("* OK pret\r\n").expect("CAPABILITY");
    let during_handshake = Script::new()
        .send(imap::greeting("IMAP4rev1 STARTTLS"))
        .expect("STARTTLS")
        .send(imap::ok("Debut de la negociation TLS"))
        .expect_data();
    for script in [before_starttls, during_handshake] {
        for cut in [Script::close, Script::reset] {
            let server =
                ScriptedServer::start(cut(script.clone()), Some(cert.server_config()?)).await?;
            let result = f
                .connector
                .connect(&request(
                    server.port(),
                    Security::StartTls,
                    PlaintextPolicy::deny_all(),
                ))
                .await;
            assert_eq!(result.err(), Some(ConnectionError::ClosedBeforeEncryption));
            assert_no_credentials_sent(&server.finish().await);
        }
    }
    Ok(())
}

/// Serveur en clair consenti : il n'y a pas de chiffrement à attendre,
/// une coupure avant l'authentification est une coupure ordinaire.
#[tokio::test]
async fn cut_on_a_consented_plaintext_server_is_a_closed_connection() -> TestResult {
    let f = fixture()?;
    let server = ScriptedServer::start(
        Script::new()
            .send("* OK pret\r\n")
            .expect("CAPABILITY")
            .reset(),
        None,
    )
    .await?;
    let port = server.port();
    let result = f
        .connector
        .connect(&request(port, Security::Plain, consent(port)))
        .await;
    assert_eq!(result.err(), Some(ConnectionError::Closed));
    let _ = server.finish().await;
    Ok(())
}

/// Coupure pendant l'échange XOAUTH2 : authentification interrompue, le
/// jeton n'a été présenté qu'une fois.
#[tokio::test]
async fn cut_during_xoauth2_is_an_interrupted_authentication() -> TestResult {
    for cut in cuts() {
        let script = cut(Script::new()
            .send(imap::greeting("IMAP4rev1 AUTH=XOAUTH2 SASL-IR"))
            .expect(format!("AUTHENTICATE XOAUTH2 {XOAUTH2_B64}")));
        let (result, outcome) = oauth_connect(script).await?;
        assert_eq!(
            result.err(),
            Some(ConnectionError::Authentication(AuthError::Interrupted))
        );
        assert_eq!(outcome.transcript.text().matches(XOAUTH2_B64).count(), 1);
    }
    Ok(())
}

/// Durée totale d'ouverture écoulée pendant l'authentification :
/// authentification interrompue, pas délai passager.
#[tokio::test]
async fn total_opening_delay_reached_during_auth_is_an_interruption() -> TestResult {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost"])?;
    let script = Script::new()
        .send(imap::greeting("IMAP4rev1 AUTH=PLAIN SASL-IR"))
        .expect(format!("AUTHENTICATE PLAIN {PLAIN_B64}"))
        .wait(SLOW_SERVER);
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let tls = TlsClientFactory::new(TrustRoots::Custom(vec![ca.root()]))
        .with_handshake_timeout(Duration::from_secs(5));
    let result = ImapConnector::new(tls)
        .with_timeouts(Timeouts {
            connect: Duration::from_secs(5),
            open: Duration::from_secs(5),
            password_auth: SLOW_SERVER,
            open_total: Duration::from_secs(3),
            response: Duration::from_secs(5),
        })
        .connect(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await;
    assert_eq!(
        result.err(),
        Some(ConnectionError::Authentication(AuthError::Interrupted))
    );
    Ok(())
}

/// Échange en clair avant STARTTLS distillé octet par octet, chacun avant
/// le délai d'ouverture : la phase entière est bornée par ce délai, bien
/// avant la durée totale ; silence avant le chiffrement.
#[tokio::test]
async fn trickled_plaintext_exchange_is_bounded_as_a_whole() -> TestResult {
    let ca = TestCa::new()?;
    let mut script = Script::new();
    for byte in b"* OK salutation interminable".repeat(4) {
        script = script.send_raw(vec![byte]).wait(Duration::from_millis(100));
    }
    let server = ScriptedServer::start(script, None).await?;
    let connector = ImapConnector::new(TlsClientFactory::new(TrustRoots::Custom(vec![ca.root()])))
        .with_timeouts(Timeouts {
            connect: Duration::from_secs(5),
            open: Duration::from_millis(400),
            password_auth: Duration::from_millis(400),
            open_total: Duration::from_secs(60),
            response: Duration::from_secs(10),
        });
    let started = tokio::time::Instant::now();
    let result = connector
        .connect(&request(
            server.port(),
            Security::StartTls,
            PlaintextPolicy::deny_all(),
        ))
        .await;
    assert_eq!(
        result.err(),
        Some(ConnectionError::Timeout(TimeoutStage::BeforeEncryption))
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    Ok(())
}

/// `AUTHENTICATE` sans SASL-IR : coupure après la réponse au défi.
#[tokio::test]
async fn cut_after_the_challenge_answer_is_an_interrupted_authentication() -> TestResult {
    for cut in cuts() {
        let script = cut(Script::new()
            .send(imap::greeting("IMAP4rev1 AUTH=PLAIN"))
            .expect("AUTHENTICATE PLAIN")
            .send("+ \r\n")
            .expect_data_with(PLAIN_B64));
        assert_eq!(
            connect_tls(script).await?.err(),
            Some(ConnectionError::Authentication(AuthError::Interrupted))
        );
    }
    Ok(())
}

/// Serveur muet après `LOGIN` : authentification interrompue.
#[tokio::test]
async fn silence_after_login_is_an_interrupted_authentication() -> TestResult {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost"])?;
    let script = Script::new()
        .send(imap::greeting("IMAP4rev1"))
        .expect(format!("LOGIN alice {PASSWORD}"))
        .wait(SLOW_SERVER);
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let result = quick_opening(&ca)
        .connect(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await;
    assert_eq!(
        result.err(),
        Some(ConnectionError::Authentication(AuthError::Interrupted))
    );
    Ok(())
}

/// Étiquettes : `PERMANENTFLAGS` lu au `SELECT` en écriture et ignoré en
/// lecture seule ; mot-clé posé sur un message (`store_flag`) et sur un
/// ensemble d'UID compact (`store_keyword`) ; recherche des messages qui le
/// portent (`UID SEARCH KEYWORD`).
#[tokio::test]
async fn keywords_are_stored_searched_and_permanent_flags_read() -> TestResult {
    use norelly_domain::actions::{ActionFlag, FlagChange};
    use norelly_domain::labels::KeywordPermission;
    use norelly_domain::mailbox::{Keyword, MailboxName, Uid};
    use norelly_domain::sync::SelectRequest;

    let script = Script::new()
        .expect("EXAMINE Projets")
        .send(
            "* 3 EXISTS\r\n* OK [UIDVALIDITY 7] v\r\n* OK [PERMANENTFLAGS ()] lecture seule\r\n\
             {tag} OK [READ-ONLY] EXAMINE termine\r\n",
        )
        .expect("SELECT Projets")
        .send(
            "* 3 EXISTS\r\n* OK [UIDVALIDITY 7] v\r\n\
             * OK [PERMANENTFLAGS (\\Seen \\Flagged $label1 Projet)] limites\r\n\
             {tag} OK [READ-WRITE] SELECT termine\r\n",
        )
        .expect("UID STORE 4 +FLAGS.SILENT (Projet)")
        .send("{tag} OK STORE termine\r\n")
        .expect("UID STORE 1:3,7 -FLAGS.SILENT (&AMAAIA-faire)")
        .send("{tag} OK STORE termine\r\n")
        .expect("UID SEARCH KEYWORD Projet")
        .send("* SEARCH 4 9\r\n{tag} OK SEARCH termine\r\n")
        .expect("LOGOUT")
        .send("* BYE au revoir\r\n{tag} OK LOGOUT termine\r\n");
    let (mut session, server) = scripted_session(script).await?;
    let projets = MailboxName::parse("Projets")?;
    let examined = session
        .select(
            &projets,
            &SelectRequest {
                condstore: false,
                qresync: None,
            },
        )
        .await?;
    assert_eq!(examined.keywords, KeywordPermission::Unknown);
    let selected = session.select_writable(&projets).await?;
    assert_eq!(
        selected.keywords,
        KeywordPermission::from_tokens(["$label1", "Projet"])
    );
    let projet = Keyword::parse("Projet").ok_or("mot-clé")?;
    let uid = |n: u32| Uid::new(n).ok_or("uid");
    session
        .store_flag(
            uid(4)?,
            FlagChange {
                flag: ActionFlag::Keyword(projet.clone()),
                value: true,
            },
            None,
        )
        .await?;
    let encoded = Keyword::parse("&AMAAIA-faire").ok_or("mot-clé")?;
    session
        .store_keyword(&[uid(7)?, uid(1)?, uid(2)?, uid(3)?], &encoded, false)
        .await?;
    // Ensemble vide : rien n'est envoyé.
    session.store_keyword(&[], &encoded, true).await?;
    assert_eq!(
        session.search_keyword(&projet).await?,
        [uid(4)?, uid(9)?].into_iter().collect()
    );
    session.logout().await?;
    server.finish().await.result?;
    Ok(())
}

/// Métadonnées de deux messages par une session aux capacités
/// `capabilities` ; le serveur répond `X-GM-MSGID` dans tous les cas
/// (réponse non sollicitée quand l'extension n'est pas annoncée).
async fn metas_with_gmail_ids(
    capabilities: &str,
) -> Result<(Vec<norelly_domain::mailbox::MessageMeta>, String), Box<dyn std::error::Error>> {
    use norelly_domain::mailbox::Uid;

    let f = fixture()?;
    let cert = f.ca.issue(&["localhost"])?;
    let fetch = |uid: u32, gmail_id: &str| {
        format!(
            "* {uid} FETCH (UID {uid} X-GM-MSGID {gmail_id} FLAGS () \
             INTERNALDATE \"02-Nov-2026 10:00:00 +0000\" RFC822.SIZE 10)\r\n"
        )
    };
    let script = authenticated_with(
        capabilities,
        Script::new()
            .expect("UID FETCH 1:2")
            .send(format!(
                "{}{}{{tag}} OK termine\r\n",
                fetch(1, "1278455344230334865"),
                // Zéro : aucun message Gmail n'a cet identifiant.
                fetch(2, "0")
            ))
            .expect("LOGOUT")
            .send("* BYE au revoir\r\n{tag} OK termine\r\n")
            .expect_eof(),
    );
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let mut session = f
        .connector
        .connect(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await
        .map_err(|e| format!("connect : {e}"))?;
    let uids: Vec<Uid> = [1, 2].into_iter().filter_map(Uid::new).collect();
    let metas = session
        .fetch_meta(&uids)
        .await
        .map_err(|e| format!("fetch_meta : {e}"))?;
    session
        .logout()
        .await
        .map_err(|e| format!("logout : {e}"))?;
    let outcome = server.finish().await;
    let text = outcome.transcript.text();
    outcome.result?;
    Ok((metas, text))
}

/// `X-GM-EXT-1` annoncé : l'identifiant Gmail est demandé avec les
/// métadonnées et enregistré (zéro écarté).
#[tokio::test]
async fn gmail_message_id_is_requested_when_announced() -> TestResult {
    use norelly_domain::gmail::GmailMessageId;

    let (metas, text) = metas_with_gmail_ids("IMAP4rev1 X-GM-EXT-1").await?;
    assert!(text.contains("X-GM-MSGID"), "{text}");
    assert_eq!(
        metas.iter().map(|m| m.gmail_id).collect::<Vec<_>>(),
        [GmailMessageId::new(1_278_455_344_230_334_865), None]
    );
    Ok(())
}

/// Sans `X-GM-EXT-1` : rien ne change dans la commande, et un
/// `X-GM-MSGID` non sollicité est ignoré (un serveur quelconque ne peut
/// pas désigner un message par cet identifiant).
#[tokio::test]
async fn gmail_message_id_is_ignored_without_the_extension() -> TestResult {
    let (metas, text) = metas_with_gmail_ids("IMAP4rev1").await?;
    assert!(!text.contains("X-GM-MSGID"), "{text}");
    assert_eq!(metas.len(), 2);
    assert!(metas.iter().all(|m| m.gmail_id.is_none()), "{metas:?}");
    Ok(())
}
