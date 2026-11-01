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
        Some(ConnectionError::Timeout(TimeoutStage::Response))
    );
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
                Err(ConnectionError::Protocol(_) | ConnectionError::Closed)
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
                Err(ConnectionError::Protocol(_) | ConnectionError::Closed)
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
