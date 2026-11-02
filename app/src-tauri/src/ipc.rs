//! Contrat IPC (ADR-005) : types échangés avec l'interface, exportés en
//! TypeScript par `tauri-specta`.
//!
//! Le domaine ne dépend pas de `specta` : ces types sont des vues, et toute
//! donnée reçue de l'interface est revalidée par les constructeurs du
//! domaine avant usage. Le mot de passe n'apparaît que dans
//! [`AddAccountInput`], dont le `Debug` le masque.

use std::fmt;

use norelly_application::accounts::AddAccountOutcome;
use norelly_application::test_connection::TestConnectionOutcome;
use norelly_domain::account::{Account, ServerSettings};
use norelly_domain::discovery::{DiscoveredServer, Discovery, DiscoverySource};
use norelly_domain::endpoint::{Protocol, Security};
use norelly_domain::security::CertificateIssue;
use serde::{Deserialize, Serialize};
use specta::Type;

/// Erreur renvoyée à l'interface (message affichable, sans secret).
#[derive(Debug, Clone, Serialize, Type)]
pub struct IpcError {
    pub message: String,
}

impl IpcError {
    pub fn new(message: impl fmt::Display) -> Self {
        Self {
            message: message.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum ProtocolDto {
    Imap,
    Smtp,
}

impl From<Protocol> for ProtocolDto {
    fn from(p: Protocol) -> Self {
        match p {
            Protocol::Imap => Self::Imap,
            Protocol::Smtp => Self::Smtp,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum SecurityDto {
    Tls,
    Starttls,
    Plain,
}

impl From<Security> for SecurityDto {
    fn from(s: Security) -> Self {
        match s {
            Security::Tls => Self::Tls,
            Security::StartTls => Self::Starttls,
            Security::Plain => Self::Plain,
        }
    }
}

impl From<SecurityDto> for Security {
    fn from(s: SecurityDto) -> Self {
        match s {
            SecurityDto::Tls => Self::Tls,
            SecurityDto::Starttls => Self::StartTls,
            SecurityDto::Plain => Self::Plain,
        }
    }
}

/// Serveur (affichage et saisie).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ServerDto {
    pub host: String,
    pub port: u16,
    pub security: SecurityDto,
    pub username: String,
}

impl From<&ServerSettings> for ServerDto {
    fn from(s: &ServerSettings) -> Self {
        Self {
            host: s.endpoint.host().to_string(),
            port: s.endpoint.port(),
            security: s.endpoint.security().into(),
            username: s.username.to_string(),
        }
    }
}

/// Compte enregistré.
#[derive(Debug, Clone, Serialize, Type)]
pub struct AccountDto {
    pub id: u32,
    pub address: String,
    pub display_name: String,
    pub imap: ServerDto,
    pub smtp: ServerDto,
}

impl TryFrom<&Account> for AccountDto {
    type Error = IpcError;
    fn try_from(a: &Account) -> Result<Self, IpcError> {
        Ok(Self {
            id: u32::try_from(a.id().get()).map_err(IpcError::new)?,
            address: a.address().to_string(),
            display_name: a.display_name().to_string(),
            imap: a.imap().into(),
            smtp: a.smtp().into(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum DiscoverySourceDto {
    ProviderAutoconfig,
    MozillaIspdb,
    DnsSrv,
    Probe,
    Manual,
}

impl From<DiscoverySource> for DiscoverySourceDto {
    fn from(s: DiscoverySource) -> Self {
        match s {
            DiscoverySource::ProviderAutoconfig => Self::ProviderAutoconfig,
            DiscoverySource::MozillaIspdb => Self::MozillaIspdb,
            DiscoverySource::DnsSrv => Self::DnsSrv,
            DiscoverySource::Probe => Self::Probe,
            DiscoverySource::Manual => Self::Manual,
        }
    }
}

/// Serveur découvert.
#[derive(Debug, Clone, Serialize, Type)]
pub struct DiscoveredServerDto {
    pub server: ServerDto,
    pub source: DiscoverySourceDto,
    /// Serveur hors du domaine de l'adresse, trouvé par DNS : à confirmer.
    pub requires_confirmation: bool,
}

/// Résultat de la découverte (éventuellement partiel).
#[derive(Debug, Clone, Serialize, Type)]
pub struct DiscoveryDto {
    pub imap: Option<DiscoveredServerDto>,
    pub smtp: Option<DiscoveredServerDto>,
}

fn discovered(
    found: Option<&DiscoveredServer>,
    discovery: &Discovery,
) -> Option<DiscoveredServerDto> {
    let found = found?;
    let username = found.candidate.username.resolve(discovery.address()).ok()?;
    let endpoint = &found.candidate.endpoint;
    Some(DiscoveredServerDto {
        server: ServerDto {
            host: endpoint.host().to_string(),
            port: endpoint.port(),
            security: endpoint.security().into(),
            username: username.to_string(),
        },
        source: found.source.into(),
        requires_confirmation: found.requires_confirmation,
    })
}

impl From<&Discovery> for DiscoveryDto {
    fn from(d: &Discovery) -> Self {
        Self {
            imap: discovered(d.imap(), d),
            smtp: discovered(d.smtp(), d),
        }
    }
}

/// Point de connexion (consentement au clair, exception de certificat).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct EndpointDto {
    pub host: String,
    pub port: u16,
}

/// Demande d'ajout de compte.
#[derive(Clone, Deserialize, Type)]
pub struct AddAccountInput {
    pub address: String,
    pub display_name: Option<String>,
    pub password: String,
    /// Serveurs choisis ; absents : découverte automatique.
    pub imap: Option<ServerDto>,
    pub smtp: Option<ServerDto>,
    /// Serveurs en clair acceptés explicitement par l'utilisateur.
    pub plaintext_consents: Vec<EndpointDto>,
    /// Hôtes signalés par la découverte (DNS hors domaine) que
    /// l'utilisateur a confirmés explicitement.
    pub confirmed_hosts: Vec<String>,
}

impl fmt::Debug for AddAccountInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AddAccountInput")
            .field("address", &self.address)
            .field("display_name", &self.display_name)
            .field("password", &"***")
            .field("imap", &self.imap)
            .field("smtp", &self.smtp)
            .field("plaintext_consents", &self.plaintext_consents)
            .field("confirmed_hosts", &self.confirmed_hosts)
            .finish()
    }
}

/// Code stable du problème de certificat (pour l'interface).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CertificateIssueDto {
    UnknownIssuer,
    Expired,
    NotYetValid,
    HostnameMismatch,
    Revoked,
    InvalidSignature,
    Malformed,
    Other,
}

impl From<&CertificateIssue> for CertificateIssueDto {
    fn from(i: &CertificateIssue) -> Self {
        match i {
            CertificateIssue::UnknownIssuer => Self::UnknownIssuer,
            CertificateIssue::Expired => Self::Expired,
            CertificateIssue::NotYetValid => Self::NotYetValid,
            CertificateIssue::HostnameMismatch => Self::HostnameMismatch,
            CertificateIssue::Revoked => Self::Revoked,
            CertificateIssue::InvalidSignature => Self::InvalidSignature,
            CertificateIssue::Malformed => Self::Malformed,
            CertificateIssue::Other(_) => Self::Other,
        }
    }
}

/// Certificat refusé, présenté à l'utilisateur.
#[derive(Debug, Clone, Serialize, Type)]
pub struct CertificateProblemDto {
    pub protocol: ProtocolDto,
    pub host: String,
    /// Empreinte SHA-256 (`AB:CD:…`).
    pub fingerprint: String,
    pub issue: CertificateIssueDto,
    /// Raison lisible.
    pub reason: String,
    /// L'empreinte diffère d'une exception enregistrée (attaque possible).
    pub fingerprint_changed: bool,
    /// Une exception est possible (pas pour un certificat révoqué).
    pub can_trust: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum FailureKindDto {
    Network,
    Authentication,
    Security,
    Protocol,
    Internal,
}

/// Résultat d'un ajout de compte.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AddAccountResultDto {
    Added {
        account: AccountDto,
    },
    AlreadyExists,
    NeedsReview {
        discovery: DiscoveryDto,
    },
    PlaintextConsentRequired {
        endpoint: EndpointDto,
    },
    /// Aucun trousseau du système ne répond : l'utilisateur doit accepter
    /// que le mot de passe soit enregistré dans un fichier non chiffré
    /// (`path`) avant toute connexion.
    SecretStorageConsentRequired {
        path: String,
    },
    /// Serveur trouvé par le DNS hors du domaine de l'adresse, non confirmé :
    /// aucune connexion n'a été tentée.
    ConfirmationRequired {
        host: String,
    },
    Certificate {
        problem: CertificateProblemDto,
    },
    Failed {
        protocol: Option<ProtocolDto>,
        failure: FailureKindDto,
        message: String,
    },
}

fn failed(
    protocol: Option<Protocol>,
    failure: FailureKindDto,
    message: impl fmt::Display,
) -> AddAccountResultDto {
    AddAccountResultDto::Failed {
        protocol: protocol.map(Into::into),
        failure,
        message: message.to_string(),
    }
}

impl AddAccountResultDto {
    pub fn from_outcome(outcome: &AddAccountOutcome) -> Self {
        match outcome {
            AddAccountOutcome::Added(account) => match AccountDto::try_from(account) {
                Ok(account) => Self::Added { account },
                Err(e) => failed(None, FailureKindDto::Internal, e.message),
            },
            AddAccountOutcome::AlreadyExists => Self::AlreadyExists,
            AddAccountOutcome::NeedsReview(d) => Self::NeedsReview {
                discovery: d.into(),
            },
            AddAccountOutcome::PlaintextConsentRequired(e) => Self::PlaintextConsentRequired {
                endpoint: EndpointDto {
                    host: e.host().to_string(),
                    port: e.port(),
                },
            },
            AddAccountOutcome::ConnectionFailed { protocol, outcome } => {
                connection_failure(*protocol, outcome)
            }
            AddAccountOutcome::Invalid(m) => failed(None, FailureKindDto::Internal, m),
            AddAccountOutcome::SecretStoreFailed(m) => failed(
                None,
                FailureKindDto::Internal,
                format!("trousseau du système : {m}"),
            ),
            AddAccountOutcome::StorageFailed(m) => failed(None, FailureKindDto::Internal, m),
        }
    }
}

fn connection_failure(protocol: Protocol, outcome: &TestConnectionOutcome) -> AddAccountResultDto {
    let p = Some(protocol);
    match outcome {
        TestConnectionOutcome::Certificate(r) => AddAccountResultDto::Certificate {
            problem: CertificateProblemDto {
                protocol: protocol.into(),
                host: r.host.to_string(),
                fingerprint: r.fingerprint.to_string(),
                issue: (&r.issue).into(),
                reason: r.issue.to_string(),
                fingerprint_changed: r.fingerprint_changed,
                can_trust: r.can_be_excepted(),
            },
        },
        TestConnectionOutcome::Authentication(e) => failed(p, FailureKindDto::Authentication, e),
        TestConnectionOutcome::Network(e) => failed(p, FailureKindDto::Network, e),
        TestConnectionOutcome::Security(e) => failed(p, FailureKindDto::Security, e),
        TestConnectionOutcome::Protocol(e) => failed(p, FailureKindDto::Protocol, e),
        TestConnectionOutcome::Internal(m) => failed(p, FailureKindDto::Internal, m),
        TestConnectionOutcome::Success { .. } => {
            failed(p, FailureKindDto::Internal, "succès inattendu")
        }
    }
}

// ---------------------------------------------------------------------------
// Lecture (phase 5)
// ---------------------------------------------------------------------------

/// Identifiant local exposé à l'interface (entier positif de 32 bits).
pub fn id32(raw: i64) -> Result<u32, IpcError> {
    u32::try_from(raw).map_err(|_| IpcError::new("identifiant hors limites"))
}

/// Date (secondes Unix) exposée à l'interface : entier de 32 bits non
/// signé (jusqu'en 2106) ; une date hors de ces bornes est ramenée à la
/// borne la plus proche.
#[must_use]
pub fn date_dto(seconds: i64) -> u32 {
    u32::try_from(seconds.max(0)).unwrap_or(u32::MAX)
}

/// Date reçue de l'interface (curseur).
#[must_use]
pub fn date_from_dto(raw: u32) -> i64 {
    i64::from(raw)
}

/// Événement de synchronisation relayé à l'interface.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SyncEventDto {
    FolderChanged { account: u32, mailbox: u32 },
    FoldersChanged { account: u32 },
    Online { account: u32 },
    Offline { account: u32, reason: String },
}

/// Dossier d'un compte.
#[derive(Debug, Clone, Serialize, Type)]
pub struct FolderDto {
    pub id: u32,
    pub name: String,
    /// Dernier segment du nom (affichage).
    pub leaf: String,
    /// Profondeur dans la hiérarchie (0 : premier niveau).
    pub depth: u32,
    pub role: String,
    pub selectable: bool,
    pub total: u32,
    pub unread: u32,
}

/// Portée d'une liste de conversations.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScopeDto {
    Mailbox { id: u32 },
    Unified { role: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
pub struct ConversationCursorDto {
    pub date: u32,
    pub id: u32,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct CorrespondentDto {
    pub name: String,
    pub address: String,
}

impl From<&norelly_domain::mailbox::Correspondent> for CorrespondentDto {
    fn from(c: &norelly_domain::mailbox::Correspondent) -> Self {
        Self {
            name: c.name.clone(),
            address: c.address.clone(),
        }
    }
}

/// Ligne de la liste des conversations.
#[derive(Debug, Clone, Serialize, Type)]
pub struct ConversationDto {
    pub id: u32,
    pub account: u32,
    pub mailbox: u32,
    pub date: u32,
    pub subject: String,
    pub senders: Vec<CorrespondentDto>,
    pub messages: u32,
    pub unread: u32,
    pub flagged: bool,
    pub has_attachments: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ConversationPageDto {
    pub items: Vec<ConversationDto>,
    pub next: Option<ConversationCursorDto>,
}

/// Message d'une conversation.
#[derive(Debug, Clone, Serialize, Type)]
pub struct ConversationMessageDto {
    pub id: u32,
    pub account: u32,
    pub mailbox: u32,
    pub date: u32,
    pub from: Option<CorrespondentDto>,
    pub to: Vec<CorrespondentDto>,
    pub cc: Vec<CorrespondentDto>,
    pub subject: String,
    pub unread: bool,
    pub flagged: bool,
    pub has_attachments: bool,
}

/// Pièce jointe d'un message ouvert.
#[derive(Debug, Clone, Serialize, Type)]
pub struct AttachmentDto {
    /// Chemin de partie (`1.2`), identifiant pour l'ouverture.
    pub part: String,
    pub name: String,
    pub media_type: String,
    /// Taille en octets (bornée à 4 Gio).
    pub size: u32,
    /// Faux si elle reste à télécharger.
    pub available: bool,
    /// Faux pour un type exécutable : enregistrement seulement.
    pub can_open: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum BodyModeDto {
    Html,
    Text,
    Empty,
}

/// Message ouvert : métadonnées du rendu (le document lui-même est servi
/// par `mailview://`).
#[derive(Debug, Clone, Serialize, Type)]
pub struct MessageViewDto {
    pub id: u32,
    pub mode: BodyModeDto,
    pub has_html: bool,
    pub blocked_images: u32,
    pub attachments: Vec<AttachmentDto>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ReadingPreferencesDto {
    pub prefer_plain_text: bool,
}

#[cfg(test)]
mod reading_tests {
    use super::*;

    #[test]
    fn dates_and_ids_are_bounded() {
        assert_eq!(date_dto(1_700_000_000), 1_700_000_000);
        assert_eq!(date_dto(-5), 0);
        assert_eq!(date_dto(i64::MAX), u32::MAX);
        assert_eq!(date_from_dto(1_700_000_000), 1_700_000_000);
        assert!(id32(-1).is_err());
        assert!(id32(i64::from(u32::MAX) + 1).is_err());
    }
}
