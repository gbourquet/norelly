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
        .map(|c| {
            if c.is_control() || is_format_char(c) {
                ' '
            } else {
                c
            }
        })
        .take(MAX_SERVER_TEXT)
        .collect();
    if text.chars().count() > MAX_SERVER_TEXT {
        out.push('…');
    }
    out.trim().to_owned()
}

/// Caractères de mise en forme invisibles ou bidirectionnels (catégorie
/// Unicode Cf usuelle) : ils permettent de maquiller un texte affiché
/// (U+202E retourne la suite du texte).
pub(crate) fn is_format_char(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2069}'
            | '\u{FEFF}'
    )
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
    /// Commande refusée (`NO`) : la session reste utilisable.
    #[error("commande refusée par le serveur : {0}")]
    Rejected(String),
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
            Self::Protocol(_) | Self::Rejected(_) => FailureCategory::Protocol,
        }
    }
    /// Vrai si l'échec est passager (serveur muet, connexion coupée, réseau
    /// instable) : une nouvelle connexion a des chances d'aboutir. Un refus
    /// du serveur, un certificat ou une protection ne se réessaient pas.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Network(_) | Self::Timeout(_) | Self::Closed)
    }

    /// Vrai si le serveur a été joint puis s'est tu ou a coupé la connexion,
    /// par opposition à un serveur introuvable ou qui refuse la connexion.
    /// Réseau instable le plus souvent ; un port ou un chiffrement faux
    /// produit aussi ce silence.
    #[must_use]
    pub fn is_unresponsive(&self) -> bool {
        matches!(
            self,
            Self::Timeout(
                TimeoutStage::TlsHandshake | TimeoutStage::Response | TimeoutStage::Write
            ) | Self::Closed
        )
    }

    /// Vrai si un compte dont les serveurs sont imposés par le fournisseur
    /// (OAuth) peut être ajouté malgré cet échec du test d'envoi, la
    /// réception ayant réussi : le serveur d'envoi a été joint puis s'est tu
    /// ou a coupé la connexion (réseau instable). L'envoi sera vérifié au
    /// premier message. Un serveur introuvable, un refus ou une protection
    /// bloquent l'ajout. Pour des serveurs saisis, un port ou un chiffrement
    /// faux produit le même silence : cette règle ne s'y applique pas.
    #[must_use]
    pub fn send_check_can_wait(&self) -> bool {
        self.is_unresponsive()
    }
}

/// Essais d'une connexion dont l'échec est passager
/// ([`ConnectionError::is_transient`]), le premier compris.
pub const CONNECTION_ATTEMPTS: usize = 3;

/// Attente entre deux essais d'une connexion.
pub const CONNECTION_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

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
        assert_eq!(
            server_text("mot\u{202E}gnp.exe\u{200B}\u{2066}x\u{FEFF}".as_bytes()),
            "mot gnp.exe  x"
        );
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

    #[test]
    fn transient_failures_are_retried() {
        for e in [
            ConnectionError::Network("connexion réinitialisée".into()),
            ConnectionError::Timeout(TimeoutStage::Connect),
            ConnectionError::Timeout(TimeoutStage::TlsHandshake),
            ConnectionError::Timeout(TimeoutStage::Response),
            ConnectionError::Timeout(TimeoutStage::Write),
            ConnectionError::Closed,
        ] {
            assert!(e.is_transient(), "{e:?}");
        }
        for e in [
            ConnectionError::from(AuthError::LoginDisabled),
            ConnectionError::Tls("x".into()),
            ConnectionError::from(StartTlsUnavailable),
            ConnectionError::PlaintextInjection,
            ConnectionError::Protocol("x".into()),
            ConnectionError::Rejected("x".into()),
        ] {
            assert!(!e.is_transient(), "{e:?}");
        }
    }

    #[test]
    fn unresponsive_server_was_reached() {
        for e in [
            ConnectionError::Timeout(TimeoutStage::TlsHandshake),
            ConnectionError::Timeout(TimeoutStage::Response),
            ConnectionError::Timeout(TimeoutStage::Write),
            ConnectionError::Closed,
        ] {
            assert!(e.is_unresponsive(), "{e:?}");
        }
        for e in [
            ConnectionError::Network("nom inconnu".into()),
            ConnectionError::Timeout(TimeoutStage::Connect),
            ConnectionError::from(AuthError::LoginDisabled),
        ] {
            assert!(!e.is_unresponsive(), "{e:?}");
        }
    }

    #[test]
    fn only_a_silent_send_server_lets_the_account_be_added() {
        for e in [
            ConnectionError::Timeout(TimeoutStage::TlsHandshake),
            ConnectionError::Timeout(TimeoutStage::Response),
            ConnectionError::Closed,
        ] {
            assert!(e.send_check_can_wait(), "{e:?}");
        }
        for e in [
            ConnectionError::Network("nom inconnu".into()),
            ConnectionError::Timeout(TimeoutStage::Connect),
            ConnectionError::from(AuthError::LoginDisabled),
            ConnectionError::from(StartTlsUnavailable),
            ConnectionError::Tls("x".into()),
            ConnectionError::Protocol("x".into()),
        ] {
            assert!(!e.send_check_can_wait(), "{e:?}");
        }
    }
}
