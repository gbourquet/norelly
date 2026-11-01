//! Erreurs de connexion structurées, communes à IMAP et SMTP.

use serde::{Deserialize, Serialize};

use crate::auth::AuthError;
use crate::security::{CertificateRejection, PlaintextRefused, StartTlsUnavailable};

/// Longueur maximale d'un texte fourni par le serveur dans une erreur.
const MAX_SERVER_TEXT: usize = 200;

/// Rend affichable un texte fourni par un serveur (non fiable) : caractères
/// de contrôle remplacés, longueur bornée.
#[must_use]
pub fn server_text(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let mut out: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_SERVER_TEXT)
        .collect();
    if text.chars().count() > MAX_SERVER_TEXT {
        out.push('…');
    }
    out.trim().to_owned()
}

/// Étape pendant laquelle un délai a expiré.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeoutStage {
    Connect,
    TlsHandshake,
    Response,
    Write,
}

/// Échec d'une connexion à un serveur.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum ConnectionError {
    #[error("erreur réseau : {0}")]
    Network(String),
    #[error("délai dépassé ({0:?})")]
    Timeout(TimeoutStage),
    #[error("le serveur a fermé la connexion")]
    Closed,
    #[error("certificat refusé pour {} : {}", .0.host, .0.issue)]
    Certificate(CertificateRejection),
    #[error("échec de la négociation TLS : {0}")]
    Tls(String),
    #[error("le serveur n'annonce pas STARTTLS : connexion interrompue (aucun repli en clair)")]
    StartTlsUnavailable,
    #[error("le serveur a refusé STARTTLS : {0}")]
    StartTlsRejected(String),
    #[error(
        "données reçues avant la négociation TLS : connexion interrompue (tentative d'injection)"
    )]
    PlaintextInjection,
    #[error(transparent)]
    PlaintextRefused(#[from] PlaintextRefused),
    #[error(transparent)]
    Authentication(#[from] AuthError),
    #[error("réponse du serveur invalide : {0}")]
    Protocol(String),
}

impl From<StartTlsUnavailable> for ConnectionError {
    fn from(_: StartTlsUnavailable) -> Self {
        Self::StartTlsUnavailable
    }
}

/// Catégorie d'échec, pour l'affichage et le diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailureCategory {
    /// Serveur injoignable, délai, coupure.
    Network,
    /// Certificat refusé (exception possible) ou négociation TLS impossible.
    Certificate,
    /// Identifiants refusés ou méthode d'authentification impossible.
    Authentication,
    /// Protection de sécurité déclenchée (clair refusé, STARTTLS absent,
    /// injection).
    Security,
    /// Serveur non conforme.
    Protocol,
}

impl ConnectionError {
    #[must_use]
    pub fn category(&self) -> FailureCategory {
        match self {
            Self::Network(_) | Self::Timeout(_) | Self::Closed => FailureCategory::Network,
            Self::Certificate(_) | Self::Tls(_) => FailureCategory::Certificate,
            Self::Authentication(_) => FailureCategory::Authentication,
            Self::StartTlsUnavailable
            | Self::StartTlsRejected(_)
            | Self::PlaintextInjection
            | Self::PlaintextRefused(_) => FailureCategory::Security,
            Self::Protocol(_) => FailureCategory::Protocol,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_text_is_sanitized_and_bounded() {
        assert_eq!(
            server_text(b"Bad\r\npassword\x1b[31m"),
            "Bad  password [31m"
        );
        let long = server_text(&[b'a'; 1000]);
        assert_eq!(long.chars().count(), MAX_SERVER_TEXT + 1);
        assert!(long.ends_with('…'));
        assert_eq!(server_text(&[0xff, b'o', b'k']), "\u{fffd}ok");
    }

    #[test]
    fn categories() {
        assert_eq!(ConnectionError::Closed.category(), FailureCategory::Network);
        assert_eq!(
            ConnectionError::Timeout(TimeoutStage::Connect).category(),
            FailureCategory::Network
        );
        assert_eq!(
            ConnectionError::PlaintextInjection.category(),
            FailureCategory::Security
        );
        assert_eq!(
            ConnectionError::from(StartTlsUnavailable).category(),
            FailureCategory::Security
        );
        assert_eq!(
            ConnectionError::from(AuthError::LoginDisabled).category(),
            FailureCategory::Authentication
        );
        assert_eq!(
            ConnectionError::Tls("x".into()).category(),
            FailureCategory::Certificate
        );
        assert_eq!(
            ConnectionError::Protocol("x".into()).category(),
            FailureCategory::Protocol
        );
    }
}
