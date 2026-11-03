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
use norelly_domain::auth::AuthError;
use norelly_domain::connection::{ConnectionError, FailureCategory};
use norelly_domain::discovery::{DiscoveredServer, Discovery, DiscoverySource};
use norelly_domain::endpoint::{Protocol, Security};
use norelly_domain::security::CertificateIssue;
use serde::{Deserialize, Serialize};
use specta::Type;

/// Code stable d'une erreur IPC (ADR-011) : l'interface le traduit ; Rust
/// n'envoie aucun texte destiné à l'utilisateur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCodeDto {
    /// Donnée reçue de l'interface refusée par le domaine (`field`).
    InvalidInput,
    /// Élément introuvable (`field` : message, partie, certificat…).
    NotFound,
    /// Expéditeur sans adresse valide : autorisation impossible.
    NoValidSender,
    /// Téléchargement depuis le serveur impossible (hors ligne…).
    DownloadFailed,
    /// Stockage local (base, fichiers).
    Storage,
    /// Trousseau du système.
    SecretStore,
    /// Lien refusé (schéma autre que http, https, mailto).
    LinkRefused,
    /// Pièce jointe exécutable : à enregistrer plutôt qu'ouvrir.
    ExecutableAttachment,
    /// Écriture d'un fichier impossible.
    WriteFailed,
    /// Ouverture par le système impossible.
    OpenFailed,
    /// Action impossible : aucun dossier Corbeille sur le compte.
    NoTrash,
    /// Action impossible : aucun dossier d'archives (ni repli choisi).
    NoArchive,
    /// Le message est déjà dans le dossier visé.
    SameFolder,
    /// Dossier de destination non sélectionnable.
    FolderNotSelectable,
    /// Trop de messages visés par une action.
    TooManyMessages,
    /// Trop d'opérations en attente sur le compte (serveur injoignable
    /// depuis longtemps) : action refusée.
    PendingQueueFull,
    /// Message sans destinataire.
    NoRecipient,
    /// Trop de destinataires.
    TooManyRecipients,
    /// Sujet invalide (caractère de contrôle, trop long).
    InvalidSubject,
    /// Corps trop volumineux.
    BodyTooLarge,
    /// Pièces jointes trop volumineuses.
    AttachmentsTooLarge,
    /// Envoi non annulable ou non relançable dans son état.
    OutboxBusy,
    /// Erreur interne (journalisée).
    Internal,
}

/// Champ ou objet concerné par une erreur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum FieldDto {
    Address,
    DisplayName,
    Host,
    Port,
    Username,
    /// Serveurs IMAP et SMTP fournis ensemble ou pas du tout.
    Servers,
    Account,
    Message,
    Part,
    Mailbox,
    Role,
    Cursor,
    Conversation,
    Folder,
    Certificate,
    Draft,
    Attachment,
    Outbox,
    /// Requête de recherche (trop longue, trop de clauses) ou fuseau.
    Query,
}

/// Erreur renvoyée à l'interface : un code, le champ concerné, et un détail
/// technique brut facultatif (réponse d'un serveur, erreur du système),
/// affiché tel quel et jamais traduit. Sans secret.
#[derive(Debug, Clone, Serialize, Type)]
pub struct IpcError {
    pub code: ErrorCodeDto,
    pub field: Option<FieldDto>,
    pub detail: Option<String>,
}

impl IpcError {
    #[must_use]
    pub fn new(code: ErrorCodeDto) -> Self {
        Self {
            code,
            field: None,
            detail: None,
        }
    }

    /// Donnée refusée par le domaine.
    #[must_use]
    pub fn invalid(field: FieldDto) -> Self {
        Self {
            field: Some(field),
            ..Self::new(ErrorCodeDto::InvalidInput)
        }
    }

    #[must_use]
    pub fn not_found(field: FieldDto) -> Self {
        Self {
            field: Some(field),
            ..Self::new(ErrorCodeDto::NotFound)
        }
    }

    /// Ajoute un détail technique brut (borné, nettoyé comme un texte de
    /// serveur).
    #[must_use]
    pub fn with_detail(mut self, detail: impl fmt::Display) -> Self {
        self.detail = Some(norelly_domain::connection::server_text(
            detail.to_string().as_bytes(),
        ));
        self
    }

    /// Erreur de stockage local : journalisée, sans détail transmis.
    pub fn storage(error: impl fmt::Display) -> Self {
        tracing::warn!(%error, "stockage local");
        Self::new(ErrorCodeDto::Storage)
    }

    /// Erreur interne : journalisée, sans détail transmis.
    pub fn internal(error: impl fmt::Display) -> Self {
        tracing::warn!(%error, "erreur interne");
        Self::new(ErrorCodeDto::Internal)
    }
}

impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.code)?;
        if let Some(field) = self.field {
            write!(f, " ({field:?})")?;
        }
        if let Some(detail) = &self.detail {
            write!(f, " : {detail}")?;
        }
        Ok(())
    }
}

impl From<norelly_application::actions::ActionFailure> for IpcError {
    fn from(e: norelly_application::actions::ActionFailure) -> Self {
        use norelly_application::actions::ActionFailure;
        use norelly_domain::actions::ActionError;
        match e {
            ActionFailure::Store(e) => Self::storage(e),
            ActionFailure::Invalid(e) => match e {
                ActionError::NoTrash => Self::new(ErrorCodeDto::NoTrash),
                ActionError::NoArchive => Self::new(ErrorCodeDto::NoArchive),
                ActionError::SameFolder => Self::new(ErrorCodeDto::SameFolder),
                ActionError::NotSelectable => Self::new(ErrorCodeDto::FolderNotSelectable),
                ActionError::TooManyTargets => Self::new(ErrorCodeDto::TooManyMessages),
                ActionError::UnknownFolder => Self::invalid(FieldDto::Mailbox),
                ActionError::QueueFull => Self::new(ErrorCodeDto::PendingQueueFull),
            },
        }
    }
}

impl From<norelly_application::reading::ReadError> for IpcError {
    fn from(e: norelly_application::reading::ReadError) -> Self {
        use norelly_application::reading::ReadError;
        match e {
            ReadError::Store(e) => Self::storage(e),
            ReadError::NotFound => Self::not_found(FieldDto::Message),
            ReadError::PartNotFound => Self::not_found(FieldDto::Part),
            ReadError::NoValidSender => Self::new(ErrorCodeDto::NoValidSender),
            ReadError::Download(detail) => {
                Self::new(ErrorCodeDto::DownloadFailed).with_detail(detail)
            }
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
            id: u32::try_from(a.id().get()).map_err(IpcError::internal)?,
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
    /// Paramètres refusés par le domaine.
    InvalidInput,
    /// Trousseau du système.
    SecretStore,
    /// Stockage local.
    Storage,
    Internal,
}

/// Catégorie et détail technique brut (texte du serveur ou du système)
/// d'un échec de connexion.
#[must_use]
pub fn connection_failure_of(e: &ConnectionError) -> (FailureKindDto, Option<String>) {
    let kind = match e.category() {
        FailureCategory::Network => FailureKindDto::Network,
        FailureCategory::Authentication => FailureKindDto::Authentication,
        FailureCategory::Certificate | FailureCategory::Security => FailureKindDto::Security,
        FailureCategory::Protocol => FailureKindDto::Protocol,
    };
    (kind, connection_detail(e))
}

fn connection_detail(e: &ConnectionError) -> Option<String> {
    match e {
        ConnectionError::Network(s)
        | ConnectionError::Tls(s)
        | ConnectionError::StartTlsRejected(s)
        | ConnectionError::Protocol(s)
        | ConnectionError::Rejected(s) => Some(s.clone()),
        ConnectionError::Authentication(a) => auth_detail(a),
        ConnectionError::Timeout(_)
        | ConnectionError::Closed
        | ConnectionError::Certificate(_)
        | ConnectionError::StartTlsUnavailable
        | ConnectionError::PlaintextInjection
        | ConnectionError::PlaintextRefused(_) => None,
    }
}

fn auth_detail(a: &AuthError) -> Option<String> {
    match a {
        AuthError::Rejected { message } => Some(message.clone()),
        AuthError::NoSupportedMechanism { offered } => Some(offered.join(", ")),
        AuthError::Unsupported(_)
        | AuthError::LoginDisabled
        | AuthError::UnexpectedChallenge { .. } => None,
    }
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
    /// Échec : catégorie, et détail technique brut éventuel (réponse du
    /// serveur), jamais traduit.
    Failed {
        protocol: Option<ProtocolDto>,
        failure: FailureKindDto,
        detail: Option<String>,
    },
}

fn failed(
    protocol: Option<Protocol>,
    failure: FailureKindDto,
    detail: Option<String>,
) -> AddAccountResultDto {
    AddAccountResultDto::Failed {
        protocol: protocol.map(Into::into),
        failure,
        detail: detail.map(|d| norelly_domain::connection::server_text(d.as_bytes())),
    }
}

/// Échec interne : journalisé, sans détail transmis.
fn failed_internal(failure: FailureKindDto, error: impl fmt::Display) -> AddAccountResultDto {
    tracing::warn!(%error, ?failure, "ajout de compte");
    failed(None, failure, None)
}

impl AddAccountResultDto {
    pub fn from_outcome(outcome: &AddAccountOutcome) -> Self {
        match outcome {
            AddAccountOutcome::Added(account) => match AccountDto::try_from(account) {
                Ok(account) => Self::Added { account },
                Err(e) => failed_internal(FailureKindDto::Internal, e),
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
            AddAccountOutcome::Invalid(m) => failed_internal(FailureKindDto::InvalidInput, m),
            AddAccountOutcome::SecretStoreFailed(m) => {
                failed_internal(FailureKindDto::SecretStore, m)
            }
            AddAccountOutcome::StorageFailed(m) => failed_internal(FailureKindDto::Storage, m),
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
                fingerprint_changed: r.fingerprint_changed,
                can_trust: r.can_be_excepted(),
            },
        },
        TestConnectionOutcome::Authentication(e) => {
            failed(p, FailureKindDto::Authentication, auth_detail(e))
        }
        TestConnectionOutcome::Network(e) => {
            failed(p, FailureKindDto::Network, connection_detail(e))
        }
        TestConnectionOutcome::Security(e) => {
            failed(p, FailureKindDto::Security, connection_detail(e))
        }
        TestConnectionOutcome::Protocol(e) => {
            failed(p, FailureKindDto::Protocol, connection_detail(e))
        }
        TestConnectionOutcome::Internal(m) => failed_internal(FailureKindDto::Internal, m),
        TestConnectionOutcome::Success { .. } => {
            failed_internal(FailureKindDto::Internal, "succès inattendu")
        }
    }
}

// ---------------------------------------------------------------------------
// Lecture (phase 5)
// ---------------------------------------------------------------------------

/// Identifiant local exposé à l'interface (entier positif de 32 bits).
pub fn id32(raw: i64) -> Result<u32, IpcError> {
    u32::try_from(raw).map_err(|_| IpcError::internal("identifiant hors limites"))
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
    FolderChanged {
        account: u32,
        mailbox: u32,
    },
    FoldersChanged {
        account: u32,
    },
    Online {
        account: u32,
    },
    /// Connexion perdue : catégorie et détail technique brut éventuel.
    Offline {
        account: u32,
        failure: FailureKindDto,
        detail: Option<String>,
    },
}

/// Événement de la rédaction relayé à l'interface.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ComposeEventDto {
    /// File d'envoi ou brouillons d'un compte modifiés.
    OutboxChanged { account: u32 },
    /// Fichiers déposés sur la fenêtre : à joindre par
    /// `attach_dropped(token)` (les chemins restent côté Rust).
    FilesDropped { token: u32, count: u32 },
}

/// Destinataire (adresse revalidée par le domaine en entrée).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RecipientDto {
    pub name: String,
    pub address: String,
}

/// Pièce jointe d'un brouillon (contenu déjà stocké).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DraftAttachmentDto {
    /// Identifiant du contenu stocké (empreinte).
    pub blob: String,
    pub file_name: String,
    pub content_type: String,
    pub size: u32,
}

/// Brouillon échangé avec l'éditeur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DraftDto {
    /// Absent : pas encore enregistré.
    pub id: Option<u32>,
    pub account: u32,
    pub to: Vec<RecipientDto>,
    pub cc: Vec<RecipientDto>,
    pub bcc: Vec<RecipientDto>,
    pub subject: String,
    /// HTML de l'éditeur (réassaini en Rust avant tout envoi).
    pub html: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub attachments: Vec<DraftAttachmentDto>,
}

/// Brouillon enregistré.
#[derive(Debug, Clone, Serialize, Type)]
pub struct SavedDraftDto {
    pub id: u32,
    pub revision: u32,
}

/// Nature d'une rédaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ComposeKindDto {
    New,
    Reply,
    ReplyAll,
    Forward,
}

/// Cause d'échec d'un envoi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SendFailureDto {
    Unreachable,
    Temporary,
    Rejected,
    Authentication,
    Security,
    Uncertain,
    TooManyAttempts,
}

/// État d'un message de la file d'envoi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum OutboxStateDto {
    Queued { not_before: u32 },
    Sending,
    Sent,
    Failed { failure: SendFailureDto },
}

/// Message de la file d'envoi.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct OutboxItemDto {
    pub id: u32,
    pub subject: String,
    pub recipients: Vec<String>,
    pub state: OutboxStateDto,
    pub attempts: u32,
    pub queued_at: u32,
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

/// Où chercher.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SearchScopeDto {
    All,
    Account { id: u32 },
    Mailbox { id: u32 },
}

/// Morceau d'un texte surligné (à afficher comme du texte, jamais comme du
/// HTML).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct SegmentDto {
    pub text: String,
    pub hit: bool,
}

/// Message trouvé par une recherche.
#[derive(Debug, Clone, Serialize, Type)]
pub struct SearchHitDto {
    pub message: u32,
    /// Conversation à ouvrir (absente : message pas encore rattaché).
    pub conversation: Option<u32>,
    pub account: u32,
    pub mailbox: u32,
    pub date: u32,
    pub from: Option<CorrespondentDto>,
    pub subject: Vec<SegmentDto>,
    pub snippet: Vec<SegmentDto>,
    pub unread: bool,
    pub flagged: bool,
    pub has_attachments: bool,
}

/// Une page de résultats.
#[derive(Debug, Clone, Serialize, Type)]
pub struct SearchPageDto {
    pub items: Vec<SearchHitDto>,
    /// Décalage de la page suivante.
    pub next: Option<u32>,
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

/// Avis sur un corps non affiché (texte localisé par l'interface).
#[derive(Debug, Clone, Copy, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum BodyNoticeDto {
    TooComplex,
    TooLong,
    NotAvailable,
}

impl From<norelly_render::RenderNotice> for BodyNoticeDto {
    fn from(n: norelly_render::RenderNotice) -> Self {
        match n {
            norelly_render::RenderNotice::TooComplex => Self::TooComplex,
            norelly_render::RenderNotice::TooLong => Self::TooLong,
            norelly_render::RenderNotice::NotAvailable => Self::NotAvailable,
        }
    }
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
    pub notice: Option<BodyNoticeDto>,
    pub has_html: bool,
    pub blocked_images: u32,
    pub attachments: Vec<AttachmentDto>,
}

/// Action demandée par l'interface (revalidée par le domaine).
#[derive(Debug, Clone, Copy, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionDto {
    MarkSeen { value: bool },
    MarkFlagged { value: bool },
    Move { mailbox: u32 },
    Delete,
    Archive,
}

/// Résultat d'une action : opérations enregistrées (annulables) et
/// nombre de messages concernés.
#[derive(Debug, Clone, Serialize, Type)]
pub struct ActionResultDto {
    pub ops: Vec<u32>,
    pub messages: u32,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ReadingPreferencesDto {
    pub prefer_plain_text: bool,
}

#[cfg(test)]
mod error_tests {
    use super::*;

    #[test]
    fn errors_are_codes_without_user_text() {
        let json = |e: &IpcError| serde_json::to_value(e).unwrap_or_default();
        assert_eq!(
            json(&IpcError::invalid(FieldDto::Address)),
            serde_json::json!({ "code": "invalid_input", "field": "address", "detail": null })
        );
        // Détail brut nettoyé et borné comme un texte de serveur.
        let e = IpcError::new(ErrorCodeDto::OpenFailed).with_detail("a\u{202E}b\r\nc");
        assert_eq!(e.detail.as_deref(), Some("a b  c"));
        assert_eq!(
            json(&IpcError::storage("disque plein")),
            serde_json::json!({ "code": "storage", "field": null, "detail": null })
        );
    }

    #[test]
    fn action_errors_are_projected_to_their_own_codes() {
        use norelly_application::actions::ActionFailure;
        use norelly_domain::actions::ActionError;
        let code = |e: ActionError| IpcError::from(ActionFailure::Invalid(e)).code;
        assert_eq!(code(ActionError::NoTrash), ErrorCodeDto::NoTrash);
        assert_eq!(code(ActionError::NoArchive), ErrorCodeDto::NoArchive);
        assert_eq!(code(ActionError::SameFolder), ErrorCodeDto::SameFolder);
        assert_eq!(
            code(ActionError::NotSelectable),
            ErrorCodeDto::FolderNotSelectable
        );
        assert_eq!(
            code(ActionError::TooManyTargets),
            ErrorCodeDto::TooManyMessages
        );
        assert_eq!(code(ActionError::QueueFull), ErrorCodeDto::PendingQueueFull);
        let e = IpcError::from(ActionFailure::Invalid(ActionError::UnknownFolder));
        assert_eq!(
            (e.code, e.field),
            (ErrorCodeDto::InvalidInput, Some(FieldDto::Mailbox))
        );
        assert_eq!(
            IpcError::from(ActionFailure::Store(norelly_domain::ports::StoreError(
                "x".into()
            )))
            .code,
            ErrorCodeDto::Storage
        );
    }

    #[test]
    fn connection_failures_keep_only_raw_server_text() {
        let (kind, detail) =
            connection_failure_of(&ConnectionError::Authentication(AuthError::Rejected {
                message: "NO [AUTHENTICATIONFAILED]".into(),
            }));
        assert_eq!(kind, FailureKindDto::Authentication);
        assert_eq!(detail.as_deref(), Some("NO [AUTHENTICATIONFAILED]"));
        let (kind, detail) = connection_failure_of(&ConnectionError::StartTlsUnavailable);
        assert_eq!(kind, FailureKindDto::Security);
        assert_eq!(detail, None);
        let (kind, _) = connection_failure_of(&ConnectionError::Tls("handshake".into()));
        assert_eq!(kind, FailureKindDto::Security);
    }
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
