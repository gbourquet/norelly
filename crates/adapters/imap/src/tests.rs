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
    session
        .delete_mailbox(&MailboxName::parse("a b")?)
        .await?;
    let refused = session
        .delete_mailbox(&MailboxName::parse("Absent")?)
        .await;
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
