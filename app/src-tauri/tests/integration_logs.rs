//! Intégration avec Dovecot (conteneur) : un vrai refus d'authentification
//! IMAP (adaptateur réel, `imap-next`) est journalisé sans le mot de passe.

use std::sync::Arc;
use std::time::Duration;

use norelly_app_lib::logs::{LOG_FILE_NAME, subscriber};
use norelly_application::account_actor::{ActorConfig, spawn};
use norelly_application::sync::SyncEvent;
use norelly_domain::account::AccountId;
use norelly_domain::credentials::{Credentials, Secret, Username};
use norelly_domain::endpoint::{Host, Security, ServerEndpoint};
use norelly_domain::ports::ConnectRequest;
use norelly_domain::security::PlaintextPolicy;
use norelly_imap::ImapConnector;
use norelly_test_support::certs::TestCa;
use norelly_test_support::containers::{
    DOVECOT_IMAP_PORT, DOVECOT_IMAPS_PORT, TEST_USER, read_banner, start_dovecot_tls,
};
use norelly_test_support::mail_contracts::{InMemoryBlobStore, InMemoryMailStore};
use norelly_tls::{TlsClientFactory, TrustRoots};

const PASSWORD: &str = "MARQUEUR-MOT-DE-PASSE-c41e";

#[test]
fn a_real_authentication_refusal_is_logged_without_the_password() {
    let dir = std::env::temp_dir().join(format!("norelly-logs-dovecot-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let subscriber = subscriber(&dir).unwrap_or_else(|e| panic!("{e}"));
    tracing::subscriber::with_default(subscriber, || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap_or_else(|e| panic!("{e}"));
        runtime.block_on(async {
            let ca = TestCa::new().unwrap_or_else(|e| panic!("{e}"));
            let cert = ca
                .issue(&["localhost", "127.0.0.1"])
                .unwrap_or_else(|e| panic!("{e}"));
            let server = start_dovecot_tls(&cert)
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            let banner = server
                .port(DOVECOT_IMAP_PORT)
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            read_banner(&server.host, banner, Duration::from_secs(30))
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            let port = server
                .port(DOVECOT_IMAPS_PORT)
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            let request = ConnectRequest {
                endpoint: ServerEndpoint::new(
                    Host::parse(&server.host).unwrap_or_else(|_| panic!("hôte")),
                    port,
                    Security::Tls,
                )
                .unwrap_or_else(|_| panic!("serveur")),
                credentials: Credentials::new(
                    Username::parse(TEST_USER).unwrap_or_else(|_| panic!("utilisateur")),
                    Secret::new(PASSWORD.to_owned()),
                )
                .unwrap_or_else(|_| panic!("identifiants")),
                certificate_exceptions: vec![],
                plaintext: PlaintextPolicy::deny_all(),
            };
            let connector =
                ImapConnector::new(TlsClientFactory::new(TrustRoots::Custom(vec![ca.root()])));
            let handle = spawn(
                AccountId::new(1).unwrap_or_else(|_| panic!("compte")),
                request,
                Arc::new(connector),
                Arc::new(InMemoryMailStore::default()),
                Arc::new(InMemoryBlobStore::default()),
                Arc::new(norelly_render::search_text::MimeTextExtractor),
                ActorConfig::default(),
            );
            let mut events = handle.subscribe();
            let offline = tokio::time::timeout(Duration::from_secs(30), async {
                loop {
                    if let Ok(SyncEvent::Offline { .. }) = events.recv().await {
                        return;
                    }
                }
            })
            .await;
            assert!(offline.is_ok(), "refus non signalé");
            handle.shutdown().await;
        });
    });
    let text = std::fs::read_to_string(dir.join(LOG_FILE_NAME)).unwrap_or_default();
    assert!(
        text.contains("connexion perdue"),
        "refus journalisé : {text}"
    );
    assert!(
        !text.contains(PASSWORD),
        "mot de passe dans le journal : {text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Même vérification pour l'envoi : un refus d'authentification SMTP
/// (Mailpit, adaptateur réel) est journalisé sans le mot de passe.
#[test]
fn a_real_smtp_authentication_refusal_is_logged_without_the_password() {
    use norelly_domain::ports::{MailSender, OutgoingMessage};
    use norelly_smtp::SmtpSender;
    use norelly_test_support::containers::{
        MAILPIT_HTTP_PORT, MAILPIT_SMTP_PORT, MailpitTls, start_mailpit_tls, wait_for_port,
    };

    const SMTP_PASSWORD: &str = "MARQUEUR-SMTP-5b90";
    let dir = std::env::temp_dir().join(format!("norelly-logs-mailpit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let subscriber = subscriber(&dir).unwrap_or_else(|e| panic!("{e}"));
    tracing::subscriber::with_default(subscriber, || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap_or_else(|e| panic!("{e}"));
        runtime.block_on(async {
            let ca = TestCa::new().unwrap_or_else(|e| panic!("{e}"));
            let cert = ca
                .issue(&["localhost", "127.0.0.1"])
                .unwrap_or_else(|e| panic!("{e}"));
            let server = start_mailpit_tls(&cert, MailpitTls::Implicit)
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            let smtp_port = server
                .port(MAILPIT_SMTP_PORT)
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            let http_port = server
                .port(MAILPIT_HTTP_PORT)
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            wait_for_port(&server.host, http_port, Duration::from_secs(30))
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            let request = ConnectRequest {
                endpoint: ServerEndpoint::new(
                    Host::parse(&server.host).unwrap_or_else(|_| panic!("hôte")),
                    smtp_port,
                    Security::Tls,
                )
                .unwrap_or_else(|_| panic!("serveur")),
                credentials: Credentials::new(
                    Username::parse(TEST_USER).unwrap_or_else(|_| panic!("utilisateur")),
                    Secret::new(SMTP_PASSWORD.to_owned()),
                )
                .unwrap_or_else(|_| panic!("identifiants")),
                certificate_exceptions: vec![],
                plaintext: PlaintextPolicy::deny_all(),
            };
            let sender =
                SmtpSender::new(TlsClientFactory::new(TrustRoots::Custom(vec![ca.root()])));
            let sent = sender
                .send(
                    &request,
                    &OutgoingMessage {
                        from: "norelly@example.com".into(),
                        recipients: vec!["dest@example.com".into()],
                        raw: b"Subject: x\r\n\r\nBonjour.\r\n".to_vec(),
                    },
                )
                .await;
            assert!(sent.is_err(), "envoi accepté avec un mauvais mot de passe");
            // Ce que la file d'envoi journaliserait de cet échec.
            if let Err(e) = sent {
                tracing::warn!(error = %e, "envoi refusé");
            }
        });
    });
    let text = std::fs::read_to_string(dir.join(LOG_FILE_NAME)).unwrap_or_default();
    assert!(text.contains("envoi refusé"), "refus journalisé : {text}");
    assert!(
        !text.contains(SMTP_PASSWORD),
        "mot de passe dans le journal : {text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
