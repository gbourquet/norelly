//! Conteneurs de serveurs mail réels pour les tests d'intégration.
//!
//! Les versions d'image sont figées pour des tests reproductibles.

use std::time::Duration;

use testcontainers::core::IntoContainerPort;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpStream;

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
