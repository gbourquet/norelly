//! Ports (traits) implémentés par les adaptateurs.
//!
//! Les ports asynchrones renvoient `impl Future + Send` (dispatch statique,
//! ADR-008) ; une enveloppe `dyn` pourra être ajoutée au point d'assemblage.

use std::fmt;
use std::future::Future;

use crate::auth::{AuthError, AuthMechanism};
use crate::capabilities::ServerCapabilities;
use crate::connection::ConnectionError;
use crate::credentials::{Credentials, Secret, SecretBytes, Username};
use crate::endpoint::{Host, Protocol, ServerEndpoint};
use crate::security::{CertificateException, PlaintextPolicy};

// ---------------------------------------------------------------------------
// Authentification
// ---------------------------------------------------------------------------

/// Mécanisme SASL côté client. Implémentations pures dans
/// [`crate::auth`] : PLAIN, LOGIN, et XOAUTH2 qui renvoie
/// [`AuthError::Unsupported`].
pub trait Authenticator: fmt::Debug + Send + Sync {
    fn mechanism(&self) -> AuthMechanism;

    /// Vrai si la première réponse peut être envoyée avec la commande
    /// (SASL-IR), sans attendre de défi.
    fn supports_initial_response(&self) -> bool;

    /// Réponse au défi n° `step` (0 pour le premier ; `challenge` est déjà
    /// décodé du base64).
    fn respond(
        &self,
        credentials: &Credentials,
        step: usize,
        challenge: &[u8],
    ) -> Result<SecretBytes, AuthError>;
}

// ---------------------------------------------------------------------------
// Trousseau
// ---------------------------------------------------------------------------

/// Erreur de clé de trousseau.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("clé de trousseau invalide")]
pub struct SecretKeyError;

/// Clé d'un secret dans le trousseau, par exemple
/// `imap:alice@example.com@imap.example.com`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SecretKey(String);

impl SecretKey {
    /// Clé du secret d'un utilisateur sur un serveur.
    #[must_use]
    pub fn for_server(protocol: Protocol, username: &Username, host: &Host) -> Self {
        Self(format!("{}:{}@{}", protocol.as_str(), username, host))
    }

    /// Clé libre : non vide, sans caractère de contrôle, 1024 octets au plus.
    pub fn parse(raw: &str) -> Result<Self, SecretKeyError> {
        if raw.is_empty() || raw.len() > 1024 || raw.chars().any(char::is_control) {
            return Err(SecretKeyError);
        }
        Ok(Self(raw.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Erreur du trousseau. Ne contient jamais le secret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretStoreError {
    #[error("trousseau du système indisponible : {0}")]
    Unavailable(String),
    #[error("accès au trousseau refusé : {0}")]
    AccessDenied(String),
    #[error("erreur du trousseau : {0}")]
    Backend(String),
}

/// Stockage des secrets (trousseau du système). Seul endroit où un mot de
/// passe est conservé.
pub trait SecretStore: Send + Sync {
    /// Secret associé à la clé, `None` s'il n'existe pas.
    fn get(
        &self,
        key: &SecretKey,
    ) -> impl Future<Output = Result<Option<Secret>, SecretStoreError>> + Send;

    /// Enregistre ou remplace le secret.
    fn set(
        &self,
        key: &SecretKey,
        secret: &Secret,
    ) -> impl Future<Output = Result<(), SecretStoreError>> + Send;

    /// Supprime le secret ; sans effet s'il n'existe pas.
    fn delete(&self, key: &SecretKey) -> impl Future<Output = Result<(), SecretStoreError>> + Send;
}

// ---------------------------------------------------------------------------
// Exceptions de certificats
// ---------------------------------------------------------------------------

/// Erreur d'un dépôt.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("erreur de stockage : {0}")]
pub struct StoreError(pub String);

/// Exceptions de certificats acceptées par l'utilisateur (hôte + empreinte).
pub trait CertificateExceptionStore: Send + Sync {
    /// Exceptions enregistrées pour cet hôte.
    fn exceptions_for(
        &self,
        host: &Host,
    ) -> impl Future<Output = Result<Vec<CertificateException>, StoreError>> + Send;

    /// Enregistre une exception (idempotent).
    fn add(
        &self,
        exception: &CertificateException,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Retire une exception ; sans effet si elle n'existe pas.
    fn remove(
        &self,
        exception: &CertificateException,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}

// ---------------------------------------------------------------------------
// Connexion aux serveurs
// ---------------------------------------------------------------------------

/// Tout ce qu'il faut pour ouvrir une connexion authentifiée.
///
/// `Debug` ne montre pas le secret (voir [`Credentials`]).
#[derive(Debug, Clone)]
pub struct ConnectRequest {
    pub endpoint: ServerEndpoint,
    pub credentials: Credentials,
    /// Exceptions de certificats valables pour l'hôte de `endpoint`.
    pub certificate_exceptions: Vec<CertificateException>,
    /// Consentements au clair ; l'adaptateur vérifie à nouveau avant toute
    /// connexion (défense en profondeur).
    pub plaintext: PlaintextPolicy,
}

/// Session IMAP authentifiée.
pub trait RemoteMailboxSession: Send {
    /// Capacités relues après TLS et authentification.
    fn capabilities(&self) -> &ServerCapabilities;

    /// Ferme proprement la session (`LOGOUT`).
    fn logout(self) -> impl Future<Output = Result<(), ConnectionError>> + Send;
}

/// Ouverture de sessions IMAP.
pub trait RemoteMailboxConnector: Send + Sync {
    type Session: RemoteMailboxSession;

    /// Connexion, TLS (implicite ou STARTTLS), authentification.
    fn connect(
        &self,
        request: &ConnectRequest,
    ) -> impl Future<Output = Result<Self::Session, ConnectionError>> + Send;
}

/// Message prêt à l'envoi : enveloppe SMTP et message RFC 5322 brut.
#[derive(Clone, PartialEq, Eq)]
pub struct OutgoingMessage {
    pub from: String,
    pub recipients: Vec<String>,
    pub raw: Vec<u8>,
}

impl fmt::Debug for OutgoingMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutgoingMessage")
            .field("from", &self.from)
            .field("recipients", &self.recipients)
            .field("raw_len", &self.raw.len())
            .finish()
    }
}

/// Échec d'un envoi.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SendError {
    #[error(transparent)]
    Connection(#[from] ConnectionError),
    #[error("adresse invalide : {0}")]
    InvalidAddress(String),
    #[error("message refusé par le serveur : {0}")]
    Rejected(String),
}

/// Envoi de messages (SMTP).
pub trait MailSender: Send + Sync {
    /// Connexion, TLS, authentification, puis fermeture : vérifie que
    /// l'envoi est possible. Renvoie les capacités relues après TLS.
    fn verify(
        &self,
        request: &ConnectRequest,
    ) -> impl Future<Output = Result<ServerCapabilities, ConnectionError>> + Send;

    /// Envoie un message.
    fn send(
        &self,
        request: &ConnectRequest,
        message: &OutgoingMessage,
    ) -> impl Future<Output = Result<(), SendError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_key_for_server() {
        let (Ok(user), Ok(host)) = (Username::parse("alice"), Host::parse("IMAP.example.com"))
        else {
            panic!("valeurs valides refusées");
        };
        assert_eq!(
            SecretKey::for_server(Protocol::Imap, &user, &host).as_str(),
            "imap:alice@imap.example.com"
        );
    }

    #[test]
    fn secret_key_parse_validates() {
        assert!(SecretKey::parse("ok").is_ok());
        assert_eq!(SecretKey::parse(""), Err(SecretKeyError));
        assert_eq!(SecretKey::parse("a\nb"), Err(SecretKeyError));
        assert_eq!(SecretKey::parse(&"a".repeat(1025)), Err(SecretKeyError));
    }

    #[test]
    fn outgoing_message_debug_hides_body() {
        let m = OutgoingMessage {
            from: "a@b.c".into(),
            recipients: vec!["d@e.f".into()],
            raw: b"Subject: confidentiel".to_vec(),
        };
        assert!(!format!("{m:?}").contains("confidentiel"));
    }
}
