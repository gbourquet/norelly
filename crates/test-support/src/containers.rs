//! Conteneurs de serveurs mail réels pour les tests d'intégration.
//!
//! Les versions d'image sont figées pour des tests reproductibles.

use std::time::Duration;

use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use crate::certs::TestCert;

/// Utilisateur de test des serveurs conteneurisés (aucun identifiant réel).
pub const TEST_USER: &str = "norelly";
/// Mot de passe de test des serveurs conteneurisés.
pub const TEST_PASSWORD: &str = "mot-de-passe-de-test";

/// Image Dovecot (serveur IMAP de référence).
pub const DOVECOT_IMAGE: (&str, &str) = ("dovecot/dovecot", "2.4.5");
/// Port IMAP (STARTTLS) interne de l'image Dovecot en mode confiné.
pub const DOVECOT_IMAP_PORT: u16 = 31143;
/// Port IMAPS (TLS implicite) interne de l'image Dovecot.
pub const DOVECOT_IMAPS_PORT: u16 = 31993;

/// Image Mailpit (serveur SMTP de capture, avec API HTTP).
pub const MAILPIT_IMAGE: (&str, &str) = ("axllent/mailpit", "v1.33.0");
/// Port SMTP interne de Mailpit.
pub const MAILPIT_SMTP_PORT: u16 = 1025;
/// Port de l'API HTTP interne de Mailpit.
pub const MAILPIT_HTTP_PORT: u16 = 8025;

/// Erreurs des outils de conteneurs.
#[derive(Debug, thiserror::Error)]
pub enum ContainerError {
    #[error("échec du démarrage du conteneur : {0}")]
    Start(#[from] testcontainers::TestcontainersError),
    #[error("aucune bannière reçue de {addr} avant le délai")]
    BannerTimeout { addr: String },
    #[error("E/S : {0}")]
    Io(#[from] std::io::Error),
    #[error("réponse HTTP invalide")]
    Http,
}

/// Un serveur démarré dans un conteneur, avec son adresse hôte.
#[derive(Debug)]
pub struct RunningServer {
    /// Garde le conteneur en vie tant que la valeur existe.
    pub container: ContainerAsync<GenericImage>,
    pub host: String,
}

impl RunningServer {
    /// Port de l'hôte associé au port interne `internal`.
    pub async fn port(&self, internal: u16) -> Result<u16, ContainerError> {
        Ok(self.container.get_host_port_ipv4(internal.tcp()).await?)
    }
}

/// Démarre Dovecot avec sa configuration par défaut.
pub async fn start_dovecot() -> Result<RunningServer, ContainerError> {
    let (name, tag) = DOVECOT_IMAGE;
    let container = GenericImage::new(name, tag)
        .with_exposed_port(DOVECOT_IMAP_PORT.tcp())
        .with_exposed_port(DOVECOT_IMAPS_PORT.tcp())
        .start()
        .await?;
    let host = container.get_host().await?.to_string();
    Ok(RunningServer { container, host })
}

/// Démarre Dovecot avec un certificat de test (émis par la CA de test) et
/// le mot de passe [`TEST_PASSWORD`] pour tout utilisateur (passdb
/// statique de l'image).
pub async fn start_dovecot_tls(cert: &TestCert) -> Result<RunningServer, ContainerError> {
    let (name, tag) = DOVECOT_IMAGE;
    let container = GenericImage::new(name, tag)
        .with_exposed_port(DOVECOT_IMAP_PORT.tcp())
        .with_exposed_port(DOVECOT_IMAPS_PORT.tcp())
        .with_env_var("USER_PASSWORD", TEST_PASSWORD)
        .with_copy_to(
            "/etc/dovecot/ssl/tls.crt",
            cert.cert_pem.clone().into_bytes(),
        )
        .with_copy_to(
            "/etc/dovecot/ssl/tls.key",
            cert.key_pem.clone().into_bytes(),
        )
        .start()
        .await?;
    let host = container.get_host().await?.to_string();
    Ok(RunningServer { container, host })
}

/// Mode TLS de Mailpit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailpitTls {
    /// TLS dès la connexion (comme le port 465).
    Implicit,
    /// STARTTLS exigé (comme le port 587).
    StartTls,
}

/// Démarre Mailpit avec un certificat de test, l'authentification SMTP
/// obligatoire ([`TEST_USER`] / [`TEST_PASSWORD`]) et le mode TLS choisi.
pub async fn start_mailpit_tls(
    cert: &TestCert,
    mode: MailpitTls,
) -> Result<RunningServer, ContainerError> {
    let (name, tag) = MAILPIT_IMAGE;
    let require = match mode {
        MailpitTls::Implicit => "MP_SMTP_REQUIRE_TLS",
        MailpitTls::StartTls => "MP_SMTP_REQUIRE_STARTTLS",
    };
    let container = GenericImage::new(name, tag)
        .with_exposed_port(MAILPIT_SMTP_PORT.tcp())
        .with_exposed_port(MAILPIT_HTTP_PORT.tcp())
        .with_wait_for(WaitFor::message_on_either_std("[http] starting"))
        .with_copy_to("/certs/tls.crt", cert.cert_pem.clone().into_bytes())
        .with_copy_to("/certs/tls.key", cert.key_pem.clone().into_bytes())
        .with_copy_to(
            "/certs/auth",
            format!("{TEST_USER}:{TEST_PASSWORD}\n").into_bytes(),
        )
        .with_env_var("MP_SMTP_TLS_CERT", "/certs/tls.crt")
        .with_env_var("MP_SMTP_TLS_KEY", "/certs/tls.key")
        .with_env_var("MP_SMTP_AUTH_FILE", "/certs/auth")
        .with_env_var(require, "true")
        .start()
        .await?;
    let host = container.get_host().await?.to_string();
    Ok(RunningServer { container, host })
}

/// Attend qu'un port TCP accepte les connexions.
pub async fn wait_for_port(host: &str, port: u16, timeout: Duration) -> Result<(), ContainerError> {
    let addr = format!("{host}:{port}");
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(_)) =
            tokio::time::timeout(Duration::from_secs(2), TcpStream::connect(&addr)).await
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err(ContainerError::BannerTimeout { addr })
}

/// GET HTTP/1.0 minimal (API Mailpit locale) ; renvoie le corps.
pub async fn http_get(host: &str, port: u16, path: &str) -> Result<String, ContainerError> {
    let mut tcp = TcpStream::connect((host, port)).await?;
    tcp.write_all(format!("GET {path} HTTP/1.0\r\nHost: {host}\r\n\r\n").as_bytes())
        .await?;
    let mut response = Vec::new();
    tcp.read_to_end(&mut response).await?;
    let response = String::from_utf8_lossy(&response).into_owned();
    let (head, body) = response
        .split_once("\r\n\r\n")
        .ok_or(ContainerError::Http)?;
    if !head.starts_with("HTTP/1.") || !head.contains(" 200 ") {
        return Err(ContainerError::Http);
    }
    Ok(body.to_owned())
}

/// Démarre Mailpit.
pub async fn start_mailpit() -> Result<RunningServer, ContainerError> {
    let (name, tag) = MAILPIT_IMAGE;
    let container = GenericImage::new(name, tag)
        .with_exposed_port(MAILPIT_SMTP_PORT.tcp())
        .with_exposed_port(MAILPIT_HTTP_PORT.tcp())
        .start()
        .await?;
    let host = container.get_host().await?.to_string();
    Ok(RunningServer { container, host })
}

/// Lit la première ligne envoyée par un serveur (bannière IMAP ou SMTP), en
/// réessayant jusqu'à `timeout` le temps que le service démarre.
pub async fn read_banner(
    host: &str,
    port: u16,
    timeout: Duration,
) -> Result<String, ContainerError> {
    let addr = format!("{host}:{port}");
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(stream)) =
            tokio::time::timeout(Duration::from_secs(2), TcpStream::connect(&addr)).await
        {
            let mut line = String::new();
            let mut reader = BufReader::new(stream);
            if let Ok(Ok(n)) =
                tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut line)).await
                && n > 0
            {
                return Ok(line.trim_end().to_owned());
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err(ContainerError::BannerTimeout { addr })
}
