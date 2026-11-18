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
        open: Duration::from_secs(5),
        password_auth: Duration::from_secs(5),
        open_total: Duration::from_secs(60),
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

/// Un point seul sur une ligne terminée par un LF ou un CR isolé est
/// doublé lui aussi : sinon un serveur qui accepte ces fins de ligne
/// clôt les données au milieu du message et lit la suite comme des
/// commandes (contrebande SMTP).
#[tokio::test]
async fn a_dot_after_a_bare_line_feed_or_carriage_return_is_doubled() -> TestResult {
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
        raw: b"Subject: essai\r\n\r\nligne\n.\nMAIL FROM:<mallory@example.com>\r\n\
               suite\r.\rRCPT TO:<eve@example.com>\r\nfin\r\n"
            .to_vec(),
    };
    sender(&ca)
        .send(
            &request(server.port(), Security::Tls, PlaintextPolicy::deny_all()),
            &message,
        )
        .await?;
    let outcome = server.finish().await;
    outcome.result?;
    assert!(outcome.transcript.contains(b"ligne\n..\nMAIL"));
    assert!(outcome.transcript.contains(b"suite\r..\rRCPT"));
    Ok(())
}

/// Bannière sans fin de ligne : la lecture est bornée par la taille, pas
/// seulement par le délai, et la mémoire ne gonfle pas jusqu'à lui.
#[tokio::test]
async fn an_endless_reply_line_is_refused_before_the_delay() -> TestResult {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost"])?;
    let mut banner = b"220 ".to_vec();
    banner.resize(200 * 1024, b'a');
    let script = Script::new().send_raw(banner).expect_eof();
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let result = quick_opening(&ca)
        .verify(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await;
    assert!(
        matches!(result, Err(ConnectionError::Protocol(_))),
        "{result:?}"
    );
    Ok(())
}

/// Ligne de réponse de 2 000 octets (Microsoft 365 en envoie) : acceptée.
#[tokio::test]
async fn a_long_reply_line_is_accepted() -> TestResult {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost"])?;
    let long = format!("X-LONGUE {}", "a".repeat(2000));
    let script = ehlo_once(
        Script::new().send(smtp::greeting()),
        &[long.as_str(), "AUTH PLAIN"],
    );
    let server = ScriptedServer::start_tls(auth_and_quit(script), cert.server_config()?).await?;
    let caps = sender(&ca)
        .verify(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await?;
    assert!(caps.has("AUTH=PLAIN"));
    server.finish().await.result?;
    Ok(())
}

/// Ligne de l'EHLO faite d'espaces sur le canal final : ignorée, sans
/// panique de la tâche d'envoi.
#[tokio::test]
async fn a_blank_ehlo_line_does_not_panic() -> TestResult {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost"])?;
    let script = Script::new()
        .send(smtp::greeting())
        .expect("EHLO [127.0.0.1]")
        .send("250-faux.serveur bonjour\r\n250-   \r\n250 AUTH PLAIN\r\n");
    let server = ScriptedServer::start_tls(auth_and_quit(script), cert.server_config()?).await?;
    let caps = sender(&ca)
        .verify(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await?;
    assert!(caps.has("AUTH=PLAIN"));
    server.finish().await.result?;
    Ok(())
}

/// Réponse de 16 000 lignes : refusée ou lue sans coût quadratique, bien
/// avant le délai de transmission de [`quick_opening`] (10 s), que la
/// lecture quadratique atteignait.
#[tokio::test]
async fn a_reply_of_many_lines_is_handled_quickly() -> TestResult {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost"])?;
    let mut banner = "220-a\r\n".repeat(16_000);
    banner.push_str("220 fin\r\n");
    let script = Script::new().send(banner).expect_eof();
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let started = std::time::Instant::now();
    let _ = quick_opening(&ca)
        .verify(&request(
            server.port(),
            Security::Tls,
            PlaintextPolicy::deny_all(),
        ))
        .await;
    assert!(
        started.elapsed() < ABANDONED_WITHIN,
        "{:?}",
        started.elapsed()
    );
    Ok(())
}

/// Expéditeur au délai d'ouverture court et au délai de transmission long.
fn quick_opening(ca: &TestCa) -> SmtpSender {
    let tls = TlsClientFactory::new(TrustRoots::Custom(vec![ca.root()]))
        .with_handshake_timeout(Duration::from_secs(5));
    SmtpSender::new(tls).with_timeouts(Timeouts {
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
/// délai de transmission (10 s), avec la marge d'une machine lente (CI
/// Windows, poignée de main TLS en mode debug).
const ABANDONED_WITHIN: Duration = Duration::from_secs(6);

/// Serveur muet après `AUTH` : abandon au délai d'ouverture, pas au délai
/// de transmission ; authentification interrompue, jamais rejouée.
#[tokio::test]
async fn server_silent_after_auth_is_abandoned_at_the_opening_delay() -> TestResult {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost"])?;
    let script = ehlo_once(Script::new().send(smtp::greeting()), &["AUTH PLAIN"])
        .expect(format!("AUTH PLAIN {PLAIN_B64}"))
        .wait(SLOW_SERVER)
        .send("235 2.7.0 Authentification reussie\r\n");
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let started = tokio::time::Instant::now();
    let result = quick_opening(&ca)
        .verify(&request(
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

/// Refus d'un mot de passe retardé par le serveur au-delà du délai
/// d'ouverture : le refus est lu, pas pris pour un silence.
#[tokio::test]
async fn delayed_password_refusal_is_read() -> TestResult {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost"])?;
    let script = ehlo_once(Script::new().send(smtp::greeting()), &["AUTH PLAIN"])
        .expect(format!("AUTH PLAIN {PLAIN_B64}"))
        .wait(Duration::from_millis(800))
        .send("535 5.7.8 Identifiants invalides\r\n")
        .expect_any();
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let result = quick_opening(&ca)
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
    Ok(())
}

/// Bannière envoyée octet par octet, chacun avant le délai d'ouverture :
/// l'ouverture entière reste bornée.
#[tokio::test]
async fn trickled_banner_is_bounded_by_the_total_opening_delay() -> TestResult {
    let ca = TestCa::new()?;
    let mut script = Script::new();
    for byte in b"220 banniere interminable".repeat(4) {
        script = script.send_raw(vec![byte]).wait(Duration::from_millis(100));
    }
    let server = ScriptedServer::start(script, None).await?;
    let sender = SmtpSender::new(TlsClientFactory::new(TrustRoots::Custom(vec![ca.root()])))
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
    let result = sender
        .verify(&request(
            server.port(),
            Security::StartTls,
            PlaintextPolicy::deny_all(),
        ))
        .await;
    assert!(
        matches!(
            result,
            Err(ConnectionError::Timeout(
                norelly_domain::connection::TimeoutStage::BeforeEncryption
            ))
        ),
        "{result:?}"
    );
    assert!(
        started.elapsed() < ABANDONED_WITHIN,
        "{:?}",
        started.elapsed()
    );
    Ok(())
}

/// Bannière en retard sur un canal STARTTLS : c'est encore l'ouverture.
#[tokio::test]
async fn late_banner_is_abandoned_at_the_opening_delay() -> TestResult {
    let ca = TestCa::new()?;
    let script = Script::new().wait(SLOW_SERVER).send(smtp::greeting());
    let server = ScriptedServer::start(script, None).await?;
    let started = tokio::time::Instant::now();
    let result = quick_opening(&ca)
        .verify(&request(
            server.port(),
            Security::StartTls,
            PlaintextPolicy::deny_all(),
        ))
        .await;
    assert!(
        matches!(
            result,
            Err(ConnectionError::Timeout(
                norelly_domain::connection::TimeoutStage::BeforeEncryption
            ))
        ),
        "{result:?}"
    );
    assert!(
        started.elapsed() < ABANDONED_WITHIN,
        "{:?}",
        started.elapsed()
    );
    Ok(())
}

/// Une fois la connexion ouverte, l'acceptation lente d'un message est
/// attendue jusqu'au délai de transmission.
#[tokio::test]
async fn slow_acceptance_of_a_message_is_awaited() -> TestResult {
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
        .wait(Duration::from_secs(1))
        .send("250 queued\r\n")
        .expect("QUIT")
        .send("221 bye\r\n")
        .expect_eof();
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let message = OutgoingMessage {
        from: "alice@example.com".into(),
        recipients: vec!["bob@example.com".into()],
        raw: b"Subject: essai\r\n\r\ncorps\r\n".to_vec(),
    };
    quick_opening(&ca)
        .send(
            &request(server.port(), Security::Tls, PlaintextPolicy::deny_all()),
            &message,
        )
        .await?;
    server.finish().await.result?;
    Ok(())
}

/// Refus au `RCPT` : 4xx temporaire (`Deferred`), 5xx définitif
/// (`Rejected`), texte brut du serveur conservé.
#[tokio::test]
async fn smtp_refusals_distinguish_temporary_from_permanent() -> TestResult {
    for (reply, temporary) in [
        ("451 4.7.1 Greylisting, reessayez\r\n", true),
        ("550 5.1.1 Destinataire inconnu\r\n", false),
    ] {
        let ca = TestCa::new()?;
        let cert = ca.issue(&["localhost"])?;
        let script = ehlo_once(Script::new().send(smtp::greeting()), &["AUTH PLAIN"])
            .expect("AUTH PLAIN")
            .send("235 ok\r\n")
            .expect("MAIL FROM:<alice@example.com>")
            .send("250 ok\r\n")
            .expect("RCPT TO:<bob@example.com>")
            .send(reply)
            // `lettre` abandonne proprement la transaction.
            .expect("QUIT")
            .send("221 bye\r\n")
            .expect_eof();
        let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
        let message = OutgoingMessage {
            from: "alice@example.com".into(),
            recipients: vec!["bob@example.com".into()],
            raw: b"Subject: essai\r\n\r\ncorps\r\n".to_vec(),
        };
        let result = sender(&ca)
            .send(
                &request(server.port(), Security::Tls, PlaintextPolicy::deny_all()),
                &message,
            )
            .await;
        match (&result, temporary) {
            (Err(SendError::Deferred(text)), true) => {
                assert!(text.contains("Greylisting"), "{text}")
            }
            (Err(SendError::Rejected(text)), false) => assert!(text.contains("inconnu"), "{text}"),
            _ => panic!("{reply} : {result:?}"),
        }
        server.finish().await.result?;
    }
    Ok(())
}

/// Connexion coupée après la fin du DATA, sans réponse : le message est
/// peut-être parti, l'issue est incertaine.
#[tokio::test]
async fn cut_after_data_is_an_uncertain_outcome() -> TestResult {
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
        .expect_data_until(".");
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let message = OutgoingMessage {
        from: "alice@example.com".into(),
        recipients: vec!["bob@example.com".into()],
        raw: b"Subject: essai\r\n\r\ncorps\r\n".to_vec(),
    };
    let result = sender(&ca)
        .send(
            &request(server.port(), Security::Tls, PlaintextPolicy::deny_all()),
            &message,
        )
        .await;
    assert!(
        matches!(result, Err(SendError::Interrupted(_))),
        "{result:?}"
    );
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
                Err(ConnectionError::Protocol(_) | ConnectionError::ClosedBeforeEncryption)
            ),
            "{:?} → {result:?}",
            String::from_utf8_lossy(greeting)
        );
        server.finish().await.result?;
    }
    Ok(())
}

/// Connexion réinitialisée par le serveur après STARTTLS (équipement
/// réseau instable) : le serveur a été joint, la connexion est coupée.
#[tokio::test]
async fn reset_after_starttls_is_a_closed_connection() -> TestResult {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost"])?;
    let script = Script::new()
        .send(smtp::greeting())
        .expect("EHLO")
        .send(smtp::ehlo(&["STARTTLS"]))
        .expect("STARTTLS")
        .send("220 2.0.0 Pret\r\n")
        .starttls()
        .expect("EHLO")
        .reset();
    let server = ScriptedServer::start(script, Some(cert.server_config()?)).await?;
    let result = sender(&ca)
        .verify(&request(
            server.port(),
            Security::StartTls,
            PlaintextPolicy::deny_all(),
        ))
        .await;
    assert!(matches!(result, Err(ConnectionError::Closed)), "{result:?}");
    let _ = server.finish().await;
    Ok(())
}

/// Connexion réinitialisée pendant l'échange en clair, avant STARTTLS :
/// coupure avant le chiffrement.
#[tokio::test]
async fn reset_before_starttls_is_a_cut_before_encryption() -> TestResult {
    let ca = TestCa::new()?;
    let script = Script::new().send(smtp::greeting()).expect("EHLO").reset();
    let server = ScriptedServer::start(script, None).await?;
    let result = sender(&ca)
        .verify(&request(
            server.port(),
            Security::StartTls,
            PlaintextPolicy::deny_all(),
        ))
        .await;
    assert!(
        matches!(result, Err(ConnectionError::ClosedBeforeEncryption)),
        "{result:?}"
    );
    let _ = server.finish().await;
    Ok(())
}

const TOKEN: &str = "tok-Abc.123";
/// base64("user=alice@outlook.com\x01auth=Bearer tok-Abc.123\x01\x01").
const XOAUTH2_B64: &str = "dXNlcj1hbGljZUBvdXRsb29rLmNvbQFhdXRoPUJlYXJlciB0b2stQWJjLjEyMwEB";
/// Défi d'erreur de Microsoft (JSON en base64) après un jeton refusé.
const XOAUTH2_ERROR_CHALLENGE: &str = "334 eyJzdGF0dXMiOiI0MDEiLCJzY2hlbWVzIjoiYmVhcmVyIiwic2NvcGUiOiJodHRwczovL291dGxvb2sub2ZmaWNlLmNvbS9TTVRQLlNlbmQifQ==\r\n";

/// Compte Microsoft : le point de connexion porte l'hôte SMTP du
/// fournisseur (seule destination permise au jeton) ; la connexion TCP
/// est redirigée vers le faux serveur local, dont le certificat est émis
/// pour cet hôte.
fn oauth_request(port: u16) -> ConnectRequest {
    let provider = norelly_domain::oauth::OAuthProvider::Microsoft;
    let host = Host::parse(provider.config().smtp_host).unwrap_or_else(|_| panic!("hôte"));
    ConnectRequest {
        endpoint: ServerEndpoint::new(host, port, Security::Tls)
            .unwrap_or_else(|_| panic!("point de connexion")),
        credentials: Credentials::oauth2(
            Username::parse("alice@outlook.com").unwrap_or_else(|_| panic!("utilisateur")),
            Secret::new(TOKEN.to_owned()),
            provider,
        )
        .unwrap_or_else(|_| panic!("identifiants")),
        certificate_exceptions: vec![],
        plaintext: PlaintextPolicy::deny_all(),
    }
}

async fn oauth_server(
    ca: &TestCa,
    script: Script,
) -> Result<ScriptedServer, Box<dyn std::error::Error>> {
    let host = norelly_domain::oauth::OAuthProvider::Microsoft
        .config()
        .smtp_host;
    let cert = ca.issue(&[host])?;
    Ok(ScriptedServer::start_tls(script, cert.server_config()?).await?)
}

fn oauth_sender(ca: &TestCa, server: &ScriptedServer) -> SmtpSender {
    sender(ca).connecting_to(std::net::SocketAddr::from(([127, 0, 0, 1], server.port())))
}

#[tokio::test]
async fn xoauth2_sends_the_initial_response_on_the_wire() -> TestResult {
    let ca = TestCa::new()?;
    let script = ehlo_once(
        Script::new().send(smtp::greeting()),
        &["AUTH LOGIN XOAUTH2"],
    )
    .expect(format!("AUTH XOAUTH2 {XOAUTH2_B64}"))
    .send("235 2.7.0 Authentication successful\r\n")
    .expect("QUIT")
    .send("221 2.0.0 Au revoir\r\n")
    .expect_eof();
    let server = oauth_server(&ca, script).await?;
    oauth_sender(&ca, &server)
        .verify(&oauth_request(server.port()))
        .await?;
    server.finish().await.result?;
    Ok(())
}

/// Jeton refusé : le serveur répond `334` avec l'erreur en JSON ; le
/// client répond par une ligne vide et obtient le refus final, qui est un
/// refus d'authentification (reconnexion nécessaire), pas une erreur
/// générique.
#[tokio::test]
async fn xoauth2_error_challenge_is_a_rejection() -> TestResult {
    let ca = TestCa::new()?;
    let script = ehlo_once(Script::new().send(smtp::greeting()), &["AUTH XOAUTH2"])
        .expect(format!("AUTH XOAUTH2 {XOAUTH2_B64}"))
        .send(XOAUTH2_ERROR_CHALLENGE)
        .expect_data()
        .send("535 5.7.3 Authentication unsuccessful\r\n")
        .expect_eof();
    let server = oauth_server(&ca, script).await?;
    let result = oauth_sender(&ca, &server)
        .verify(&oauth_request(server.port()))
        .await;
    match result {
        Err(ConnectionError::Authentication(AuthError::Rejected { message })) => {
            assert!(message.contains("5.7.3"), "{message}");
        }
        other => panic!("refus d'authentification attendu : {other:?}"),
    }
    let outcome = server.finish().await;
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

/// Plus d'un défi : le client répond une fois par une ligne vide, puis
/// annule l'échange (`*`) et n'envoie plus rien, jeton compris ; c'est un
/// échange inattendu, pas un succès.
#[tokio::test]
async fn xoauth2_second_challenge_is_cancelled() -> TestResult {
    let ca = TestCa::new()?;
    let script = ehlo_once(Script::new().send(smtp::greeting()), &["AUTH XOAUTH2"])
        .expect(format!("AUTH XOAUTH2 {XOAUTH2_B64}"))
        .send(XOAUTH2_ERROR_CHALLENGE)
        .expect_data()
        .send("334 encore\r\n")
        .expect_data_with("*")
        .send("501 5.7.0 Annule\r\n")
        .expect_eof();
    let server = oauth_server(&ca, script).await?;
    let result = oauth_sender(&ca, &server)
        .verify(&oauth_request(server.port()))
        .await;
    assert!(
        matches!(
            result,
            Err(ConnectionError::Authentication(
                AuthError::UnexpectedChallenge { .. }
            ))
        ),
        "{result:?}"
    );
    let outcome = server.finish().await;
    let text = outcome.transcript.text();
    assert_eq!(text.matches(XOAUTH2_B64).count(), 1, "{text}");
    assert!(
        text.ends_with(&format!("{XOAUTH2_B64}\r\n\r\n*\r\n")),
        "ligne vide puis `*` attendus en fin d'échange :\n{text}"
    );
    outcome.result?;
    Ok(())
}

/// Réponse intermédiaire autre que `334` : erreur de protocole, rien
/// d'autre n'est envoyé.
#[tokio::test]
async fn xoauth2_unexpected_intermediate_reply_is_a_protocol_error() -> TestResult {
    let ca = TestCa::new()?;
    let script = ehlo_once(Script::new().send(smtp::greeting()), &["AUTH XOAUTH2"])
        .expect(format!("AUTH XOAUTH2 {XOAUTH2_B64}"))
        .send("354 continuez\r\n")
        .expect_eof();
    let server = oauth_server(&ca, script).await?;
    let result = oauth_sender(&ca, &server)
        .verify(&oauth_request(server.port()))
        .await;
    assert!(
        matches!(result, Err(ConnectionError::Protocol(_))),
        "{result:?}"
    );
    let outcome = server.finish().await;
    let text = outcome.transcript.text();
    assert!(
        text.ends_with(&format!("AUTH XOAUTH2 {XOAUTH2_B64}\r\n")),
        "rien ne doit suivre la commande AUTH :\n{text}"
    );
    outcome.result?;
    Ok(())
}

/// Serveur muet après le défi : tout l'échange est borné par le délai de
/// réponse, et le jeton n'est pas renvoyé : authentification interrompue.
#[tokio::test]
async fn xoauth2_silent_server_after_challenge_times_out() -> TestResult {
    let ca = TestCa::new()?;
    let script = ehlo_once(Script::new().send(smtp::greeting()), &["AUTH XOAUTH2"])
        .expect(format!("AUTH XOAUTH2 {XOAUTH2_B64}"))
        .send(XOAUTH2_ERROR_CHALLENGE)
        .expect_data()
        .expect_eof();
    let server = oauth_server(&ca, script).await?;
    let result = oauth_sender(&ca, &server)
        .with_timeouts(Timeouts {
            connect: Duration::from_secs(5),
            open: Duration::from_millis(500),
            password_auth: Duration::from_millis(500),
            open_total: Duration::from_secs(60),
            response: Duration::from_millis(500),
        })
        .verify(&oauth_request(server.port()))
        .await;
    assert_eq!(
        result.err(),
        Some(ConnectionError::Authentication(AuthError::Interrupted))
    );
    let outcome = server.finish().await;
    let text = outcome.transcript.text();
    assert_eq!(text.matches(XOAUTH2_B64).count(), 1, "{text}");
    outcome.result?;
    Ok(())
}

/// Un compte OAuth sur un serveur qui ne propose pas XOAUTH2 n'envoie ni
/// commande `AUTH` ni jeton.
#[tokio::test]
async fn oauth_account_sends_nothing_without_xoauth2() -> TestResult {
    let ca = TestCa::new()?;
    let script =
        ehlo_once(Script::new().send(smtp::greeting()), &["AUTH PLAIN LOGIN"]).expect_any();
    let server = oauth_server(&ca, script).await?;
    let result = oauth_sender(&ca, &server)
        .verify(&oauth_request(server.port()))
        .await;
    assert!(
        matches!(
            result,
            Err(ConnectionError::Authentication(
                AuthError::NoSupportedMechanism { .. }
            ))
        ),
        "{result:?}"
    );
    let outcome = server.finish().await;
    for needle in ["AUTH", TOKEN, XOAUTH2_B64] {
        assert!(
            !outcome.transcript.contains(needle.as_bytes()),
            "envoyé : {needle}\n{}",
            outcome.transcript.text()
        );
    }
    Ok(())
}

/// Serveur qui coupe la connexion (fermeture ou réinitialisation) juste
/// après avoir reçu `AUTH` : certains serveurs répondent ainsi à un mot de
/// passe faux. Échec d'authentification interrompue, jamais pris pour une
/// coupure passagère (qui ferait renvoyer le mot de passe).
#[tokio::test]
async fn cut_during_auth_is_an_interrupted_authentication() -> TestResult {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost"])?;
    for cut in [Script::close, Script::reset] {
        let script = cut(
            ehlo_once(Script::new().send(smtp::greeting()), &["AUTH PLAIN"])
                .expect(format!("AUTH PLAIN {PLAIN_B64}")),
        );
        let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
        let result = sender(&ca)
            .verify(&request(
                server.port(),
                Security::Tls,
                PlaintextPolicy::deny_all(),
            ))
            .await;
        assert_eq!(
            result.err(),
            Some(ConnectionError::Authentication(AuthError::Interrupted))
        );
        let _ = server.finish().await;
    }
    Ok(())
}

/// Serveur en clair consenti : une fermeture propre pendant `AUTH` a le
/// même classement qu'une réinitialisation, jamais une « réponse
/// incomplète » prise pour un serveur non conforme.
#[tokio::test]
async fn clean_close_during_auth_on_a_consented_plaintext_server_is_an_interruption() -> TestResult
{
    let ca = TestCa::new()?;
    for cut in [Script::close, Script::reset] {
        let script = Script::new()
            .send(smtp::greeting())
            .expect("EHLO")
            .send(smtp::ehlo(&["AUTH PLAIN"]));
        let script =
            cut(ehlo_once(script, &["AUTH PLAIN"]).expect(format!("AUTH PLAIN {PLAIN_B64}")));
        let server = ScriptedServer::start(script, None).await?;
        let port = server.port();
        let consent = PlaintextPolicy::deny_all().with_consent(PlaintextConsent {
            host: Host::parse("localhost")?,
            port,
        });
        let result = sender(&ca)
            .verify(&request(port, Security::Plain, consent))
            .await;
        assert_eq!(
            result.err(),
            Some(ConnectionError::Authentication(AuthError::Interrupted))
        );
        let _ = server.finish().await;
    }
    Ok(())
}

/// Serveur en clair consenti : fermeture propre ou réinitialisation avant
/// l'authentification, même classement (`Closed`).
#[tokio::test]
async fn close_or_reset_before_auth_is_a_closed_connection() -> TestResult {
    let ca = TestCa::new()?;
    for cut in [Script::close, Script::reset] {
        let script = Script::new()
            .send(smtp::greeting())
            .expect("EHLO")
            .send(smtp::ehlo(&["AUTH PLAIN"]));
        let script = cut(script.expect("EHLO [127.0.0.1]"));
        let server = ScriptedServer::start(script, None).await?;
        let port = server.port();
        let consent = PlaintextPolicy::deny_all().with_consent(PlaintextConsent {
            host: Host::parse("localhost")?,
            port,
        });
        let result = sender(&ca)
            .verify(&request(port, Security::Plain, consent))
            .await;
        assert_eq!(result.err(), Some(ConnectionError::Closed));
        let _ = server.finish().await;
    }
    Ok(())
}

/// Coupure (fermeture ou réinitialisation) avant la fin de la mise en
/// place du chiffrement : pendant l'échange en clair, ou juste après
/// l'acceptation de STARTTLS (poignée de main) ; nommée comme telle.
#[tokio::test]
async fn cut_before_encryption_is_named_as_such() -> TestResult {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost"])?;
    let before_starttls = Script::new().send(smtp::greeting()).expect("EHLO");
    let during_handshake = Script::new()
        .send(smtp::greeting())
        .expect("EHLO")
        .send(smtp::ehlo(&["STARTTLS"]))
        .expect("STARTTLS")
        .send("220 2.0.0 Pret\r\n")
        .expect_data();
    for script in [before_starttls, during_handshake] {
        for cut in [Script::close, Script::reset] {
            let server =
                ScriptedServer::start(cut(script.clone()), Some(cert.server_config()?)).await?;
            let result = sender(&ca)
                .verify(&request(
                    server.port(),
                    Security::StartTls,
                    PlaintextPolicy::deny_all(),
                ))
                .await;
            assert_eq!(result.err(), Some(ConnectionError::ClosedBeforeEncryption));
            let outcome = server.finish().await;
            assert_no_auth_sent(&outcome);
        }
    }
    Ok(())
}

/// TLS implicite : coupure pendant la poignée de main.
#[tokio::test]
async fn cut_during_implicit_tls_handshake_is_named_as_such() -> TestResult {
    let ca = TestCa::new()?;
    for cut in [Script::close, Script::reset] {
        let server = ScriptedServer::start(cut(Script::new().expect_data()), None).await?;
        let result = sender(&ca)
            .verify(&request(
                server.port(),
                Security::Tls,
                PlaintextPolicy::deny_all(),
            ))
            .await;
        assert_eq!(result.err(), Some(ConnectionError::ClosedBeforeEncryption));
        let _ = server.finish().await;
    }
    Ok(())
}

/// Coupure pendant l'échange XOAUTH2 : authentification interrompue, le
/// jeton n'a été présenté qu'une fois.
#[tokio::test]
async fn cut_during_xoauth2_is_an_interrupted_authentication() -> TestResult {
    let ca = TestCa::new()?;
    for cut in [Script::close, Script::reset] {
        let script = cut(
            ehlo_once(Script::new().send(smtp::greeting()), &["AUTH XOAUTH2"])
                .expect(format!("AUTH XOAUTH2 {XOAUTH2_B64}")),
        );
        let server = oauth_server(&ca, script).await?;
        let result = oauth_sender(&ca, &server)
            .verify(&oauth_request(server.port()))
            .await;
        assert_eq!(
            result.err(),
            Some(ConnectionError::Authentication(AuthError::Interrupted))
        );
        let outcome = server.finish().await;
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
    let script = ehlo_once(Script::new().send(smtp::greeting()), &["AUTH PLAIN"])
        .expect(format!("AUTH PLAIN {PLAIN_B64}"))
        .wait(SLOW_SERVER);
    let server = ScriptedServer::start_tls(script, cert.server_config()?).await?;
    let tls = TlsClientFactory::new(TrustRoots::Custom(vec![ca.root()]))
        .with_handshake_timeout(Duration::from_secs(5));
    let result = SmtpSender::new(tls)
        .with_timeouts(Timeouts {
            connect: Duration::from_secs(5),
            open: Duration::from_secs(5),
            password_auth: SLOW_SERVER,
            open_total: Duration::from_secs(3),
            response: Duration::from_secs(5),
        })
        .verify(&request(
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
    for byte in b"220 banniere interminable".repeat(4) {
        script = script.send_raw(vec![byte]).wait(Duration::from_millis(100));
    }
    let server = ScriptedServer::start(script, None).await?;
    let sender = SmtpSender::new(TlsClientFactory::new(TrustRoots::Custom(vec![ca.root()])))
        .with_timeouts(Timeouts {
            connect: Duration::from_secs(5),
            open: Duration::from_millis(400),
            password_auth: Duration::from_millis(400),
            open_total: Duration::from_secs(60),
            response: Duration::from_secs(10),
        });
    let started = tokio::time::Instant::now();
    let result = sender
        .verify(&request(
            server.port(),
            Security::StartTls,
            PlaintextPolicy::deny_all(),
        ))
        .await;
    assert_eq!(
        result.err(),
        Some(ConnectionError::Timeout(
            norelly_domain::connection::TimeoutStage::BeforeEncryption
        ))
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    Ok(())
}

/// `AUTH LOGIN` en plusieurs étapes : coupure après le nom d'utilisateur
/// ou après le mot de passe.
#[tokio::test]
async fn cut_during_auth_login_steps_is_an_interrupted_authentication() -> TestResult {
    let ca = TestCa::new()?;
    let cert = ca.issue(&["localhost"])?;
    let after_user = || {
        ehlo_once(Script::new().send(smtp::greeting()), &["AUTH LOGIN"])
            .expect("AUTH LOGIN")
            .send("334 VXNlcm5hbWU6\r\n")
            .expect_any()
    };
    let after_password = || after_user().send("334 UGFzc3dvcmQ6\r\n").expect_any();
    for script in [after_user(), after_password()] {
        for cut in [Script::close, Script::reset] {
            let server =
                ScriptedServer::start_tls(cut(script.clone()), cert.server_config()?).await?;
            let result = sender(&ca)
                .verify(&request(
                    server.port(),
                    Security::Tls,
                    PlaintextPolicy::deny_all(),
                ))
                .await;
            assert_eq!(
                result.err(),
                Some(ConnectionError::Authentication(AuthError::Interrupted))
            );
            let _ = server.finish().await;
        }
    }
    Ok(())
}
