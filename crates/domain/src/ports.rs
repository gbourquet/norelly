//! Ports (traits) implémentés par les adaptateurs.
//!
//! Les ports asynchrones renvoient `impl Future + Send` (dispatch statique,
//! ADR-008) ; une enveloppe `dyn` pourra être ajoutée au point d'assemblage.

use std::collections::BTreeSet;
use std::fmt;
use std::future::Future;
use std::time::Duration;

use crate::account::{Account, AccountDraft, AccountId};
use crate::actions::{
    FlagChange, FolderRef, LocalMessage, Member, PendingOp, QueueChange, StoreOutcome,
};
use crate::address::EmailAddress;
use crate::auth::{AuthError, AuthMechanism};
use crate::capabilities::ServerCapabilities;
use crate::compose::ReplySource;
use crate::compose::draft::{ComposeError, Draft, DraftSync, Recipient, RemoteCopy};
use crate::compose::outbox::OutboxEntry;
use crate::connection::ConnectionError;
use crate::contact_export::{ContactSelection, ExportCard, ExportFormat};
use crate::contact_import::DecodedContacts;
use crate::contact_lists::{ContactList, ContactListDraft, ContactListFilter, ContactListId};
use crate::contacts::{
    AddressKey, Contact, ContactDraft, ContactFilter, ContactId, MAX_CONTACT_NAME_CHARS,
};
use crate::conversation::ConversationScope;
use crate::credentials::{Credentials, Secret, SecretBytes, Username};
use crate::discovery::{
    AutoconfigRequest, DiscoveredServers, ServerCandidate, SrvRecord, SrvService,
};
use crate::endpoint::{Host, Protocol, ServerEndpoint};
use crate::fetch_policy::{BodyFetch, MimePart, PartPath};
use crate::mailbox::{
    Correspondent, Flags, ListedMailbox, MailboxId, MailboxName, MailboxRole, MessageMeta, ModSeq,
    RemoteMailbox, Uid,
};
use crate::search::Query;
use crate::security::{CertificateException, PlaintextPolicy};
use crate::settings::Settings;
use crate::sync::{
    FlagUpdate, LocalMailbox, MailboxSyncState, SelectRequest, SelectedMailbox, ServerQuery,
    SyncExtensions,
};

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
// OAuth
// ---------------------------------------------------------------------------

/// Jetons délivrés par un fournisseur OAuth. `Debug` masque les jetons.
#[derive(Debug, Clone)]
pub struct OAuthGrant {
    pub access_token: crate::credentials::Secret,
    /// Durée de validité du jeton d'accès, en secondes.
    pub expires_in: u64,
    /// Nouveau jeton de rafraîchissement, quand le fournisseur en délivre
    /// un (connexion, rotation au rafraîchissement).
    pub refresh_token: Option<crate::credentials::Secret>,
}

/// Obtention et rafraîchissement des jetons OAuth (navigateur du système,
/// redirection locale, HTTPS).
pub trait OAuthClient: Send + Sync {
    /// Connexion interactive : ouvre le navigateur du système et attend le
    /// retour de l'utilisateur.
    fn authorize(
        &self,
        provider: crate::oauth::OAuthProvider,
        login_hint: &crate::address::EmailAddress,
    ) -> impl Future<Output = Result<OAuthGrant, crate::oauth::TokenError>> + Send;

    /// Nouveau jeton d'accès à partir du jeton de rafraîchissement.
    fn refresh(
        &self,
        provider: crate::oauth::OAuthProvider,
        refresh_token: &crate::credentials::Secret,
    ) -> impl Future<Output = Result<OAuthGrant, crate::oauth::TokenError>> + Send;
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

    /// Clé du secret d'un compte pour un protocole : `account:7:imap`.
    /// Propre au compte, elle ne peut pas être partagée par deux comptes
    /// (alias utilisant le même identifiant sur le même serveur).
    #[must_use]
    pub fn for_account(id: AccountId, protocol: Protocol) -> Self {
        Self(format!("account:{}:{}", id.get(), protocol.as_str()))
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

/// Un trousseau partagé est aussi un trousseau (gestionnaire de jetons et
/// commandes).
impl<T: SecretStore> SecretStore for std::sync::Arc<T> {
    fn get(
        &self,
        key: &SecretKey,
    ) -> impl Future<Output = Result<Option<Secret>, SecretStoreError>> + Send {
        (**self).get(key)
    }

    fn set(
        &self,
        key: &SecretKey,
        secret: &Secret,
    ) -> impl Future<Output = Result<(), SecretStoreError>> + Send {
        (**self).set(key, secret)
    }

    fn delete(&self, key: &SecretKey) -> impl Future<Output = Result<(), SecretStoreError>> + Send {
        (**self).delete(key)
    }
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

/// Un dépôt emprunté est aussi un dépôt (partage entre cas d'usage).
impl<T: CertificateExceptionStore> CertificateExceptionStore for &T {
    fn exceptions_for(
        &self,
        host: &Host,
    ) -> impl Future<Output = Result<Vec<CertificateException>, StoreError>> + Send {
        (**self).exceptions_for(host)
    }

    fn add(
        &self,
        exception: &CertificateException,
    ) -> impl Future<Output = Result<(), StoreError>> + Send {
        (**self).add(exception)
    }

    fn remove(
        &self,
        exception: &CertificateException,
    ) -> impl Future<Output = Result<(), StoreError>> + Send {
        (**self).remove(exception)
    }
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

impl ConnectRequest {
    /// Refuse d'envoyer un jeton OAuth hors des serveurs de son
    /// fournisseur ou en clair ([`crate::oauth::token_destination_allowed`]).
    /// Les adaptateurs l'appellent avant toute authentification.
    pub fn authorize_credentials(&self) -> Result<(), AuthError> {
        match self.credentials.oauth() {
            Some(binding) => {
                crate::oauth::token_destination_allowed(binding.provider, &self.endpoint)
            }
            None => Ok(()),
        }
    }
}

/// Nombre maximal d'UID par requête `FETCH` d'en-têtes.
pub const MAX_FETCH_BATCH: usize = 500;

/// Corps téléchargé d'un message.
#[derive(Clone, PartialEq, Eq)]
pub enum FetchedBody {
    /// Message brut complet.
    Full(Vec<u8>),
    /// En-têtes bruts et parties demandées, chacune sous forme d'entité
    /// MIME autonome : ses en-têtes MIME (`BODY[n.MIME]`), une ligne vide,
    /// puis son contenu encodé tel que reçu.
    Partial {
        header: Vec<u8>,
        parts: Vec<(PartPath, Vec<u8>)>,
    },
}

impl fmt::Debug for FetchedBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full(raw) => f.debug_tuple("Full").field(&raw.len()).finish(),
            Self::Partial { header, parts } => f
                .debug_struct("Partial")
                .field("header_len", &header.len())
                .field(
                    "parts",
                    &parts
                        .iter()
                        .map(|(p, b)| (p.to_string(), b.len()))
                        .collect::<Vec<_>>(),
                )
                .finish(),
        }
    }
}

/// Fin d'une attente IDLE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleOutcome {
    /// Le serveur a signalé un changement (`EXISTS`, `EXPUNGE`, `FETCH`,
    /// `VANISHED`) : la boîte est à resynchroniser.
    Changed,
    /// Durée maximale atteinte sans changement : IDLE à renouveler.
    Timeout,
}

/// Session IMAP authentifiée.
pub trait RemoteMailboxSession: Send {
    /// Capacités relues après TLS et authentification.
    fn capabilities(&self) -> &ServerCapabilities;

    /// Active CONDSTORE et QRESYNC si le serveur les annonce (`ENABLE`) ;
    /// renvoie les extensions effectivement utilisables.
    fn enable_sync_extensions(
        &mut self,
    ) -> impl Future<Output = Result<SyncExtensions, ConnectionError>> + Send;

    /// Toutes les boîtes (`LIST "" "*"`), avec leurs attributs.
    fn list_mailboxes(
        &mut self,
    ) -> impl Future<Output = Result<Vec<ListedMailbox>, ConnectionError>> + Send;

    /// Sélectionne une boîte en lecture seule (`EXAMINE`), avec les
    /// paramètres CONDSTORE/QRESYNC demandés.
    fn select(
        &mut self,
        mailbox: &MailboxName,
        request: &SelectRequest,
    ) -> impl Future<Output = Result<SelectedMailbox, ConnectionError>> + Send;

    /// Drapeaux de la boîte sélectionnée.
    fn fetch_flags(
        &mut self,
        query: ServerQuery,
    ) -> impl Future<Output = Result<Vec<FlagUpdate>, ConnectionError>> + Send;

    /// UID de tous les messages de la boîte sélectionnée.
    fn search_all_uids(
        &mut self,
    ) -> impl Future<Output = Result<BTreeSet<Uid>, ConnectionError>> + Send;

    /// Métadonnées (drapeaux, taille, dates, en-têtes, structure) ; au plus
    /// [`MAX_FETCH_BATCH`] UID. Résultat par UID croissant, sans doublon ;
    /// les messages disparus entre-temps en sont absents.
    fn fetch_meta(
        &mut self,
        uids: &[Uid],
    ) -> impl Future<Output = Result<Vec<MessageMeta>, ConnectionError>> + Send;

    /// Corps d'un message, sans le marquer comme lu (`BODY.PEEK`) ;
    /// `None` s'il a disparu.
    fn fetch_body(
        &mut self,
        uid: Uid,
        fetch: &BodyFetch,
    ) -> impl Future<Output = Result<Option<FetchedBody>, ConnectionError>> + Send;

    /// Sélectionne une boîte en lecture-écriture (`SELECT`, avec CONDSTORE
    /// s'il est activé), pour y appliquer des actions.
    fn select_writable(
        &mut self,
        mailbox: &MailboxName,
    ) -> impl Future<Output = Result<SelectedMailbox, ConnectionError>> + Send;

    /// Drapeaux d'un message de la boîte sélectionnée ; `None` s'il a
    /// disparu.
    fn message_flags(
        &mut self,
        uid: Uid,
    ) -> impl Future<Output = Result<Option<Flags>, ConnectionError>> + Send;

    /// Ajoute ou retire un drapeau (`UID STORE … +FLAGS.SILENT` /
    /// `-FLAGS.SILENT`), sous condition `UNCHANGEDSINCE` si `unchanged_since`
    /// est fourni et CONDSTORE actif sur la boîte.
    fn store_flag(
        &mut self,
        uid: Uid,
        change: FlagChange,
        unchanged_since: Option<ModSeq>,
    ) -> impl Future<Output = Result<StoreOutcome, ConnectionError>> + Send;

    /// Marque un message `\Deleted` (`UID STORE +FLAGS.SILENT (\Deleted)`).
    fn mark_deleted(
        &mut self,
        uid: Uid,
    ) -> impl Future<Output = Result<(), ConnectionError>> + Send;

    /// `UID MOVE` (RFC 6851).
    fn move_to(
        &mut self,
        uid: Uid,
        to: &MailboxName,
    ) -> impl Future<Output = Result<(), ConnectionError>> + Send;

    /// `APPEND` d'un message dans `mailbox` (sélection inutile), marqué
    /// `\Seen`, et `\Draft` pour un brouillon. Renvoie la copie
    /// (UIDVALIDITY et UID) quand le serveur la donne (UIDPLUS, code
    /// `APPENDUID`). Boîte absente : [`ConnectionError::Rejected`].
    fn append(
        &mut self,
        mailbox: &MailboxName,
        raw: &[u8],
        draft: bool,
    ) -> impl Future<Output = Result<Option<RemoteCopy>, ConnectionError>> + Send;

    /// Crée une boîte (`CREATE`). Nom refusé par le serveur (déjà pris,
    /// interdit, quota) : [`ConnectionError::Rejected`].
    fn create_mailbox(
        &mut self,
        mailbox: &MailboxName,
    ) -> impl Future<Output = Result<(), ConnectionError>> + Send;

    /// Renomme une boîte et ses sous-boîtes (`RENAME`). Refus du serveur
    /// (boîte absente, nom pris, droits) : [`ConnectionError::Rejected`].
    fn rename_mailbox(
        &mut self,
        from: &MailboxName,
        to: &MailboxName,
    ) -> impl Future<Output = Result<(), ConnectionError>> + Send;

    /// Supprime une boîte et ses messages (`DELETE`). Refus du serveur :
    /// [`ConnectionError::Rejected`].
    fn delete_mailbox(
        &mut self,
        mailbox: &MailboxName,
    ) -> impl Future<Output = Result<(), ConnectionError>> + Send;

    /// `UID COPY`.
    fn copy_to(
        &mut self,
        uid: Uid,
        to: &MailboxName,
    ) -> impl Future<Output = Result<(), ConnectionError>> + Send;

    /// `UID EXPUNGE` (UIDPLUS) : supprime ce seul message s'il est marqué
    /// `\Deleted`.
    fn expunge_uid(&mut self, uid: Uid)
    -> impl Future<Output = Result<(), ConnectionError>> + Send;

    /// `EXPUNGE` : supprime tous les messages marqués `\Deleted` de la
    /// boîte (voir [`crate::actions::expunge_is_safe`]). Renvoie le nombre
    /// de messages supprimés (réponses `* n EXPUNGE`).
    fn expunge_all(&mut self) -> impl Future<Output = Result<u32, ConnectionError>> + Send;

    /// UID des messages marqués `\Deleted` (`UID SEARCH DELETED`).
    fn deleted_uids(
        &mut self,
    ) -> impl Future<Output = Result<BTreeSet<Uid>, ConnectionError>> + Send;

    /// Attend un changement sur la boîte sélectionnée (`IDLE`), au plus
    /// `max`. Sans IDLE côté serveur, attend `max` puis renvoie
    /// [`IdleOutcome::Changed`] (sondage).
    fn idle(
        &mut self,
        max: Duration,
    ) -> impl Future<Output = Result<IdleOutcome, ConnectionError>> + Send;

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
    /// Refus définitif (réponse SMTP 5xx).
    #[error("message refusé par le serveur : {0}")]
    Rejected(String),
    /// Refus temporaire (réponse SMTP 4xx) : à retenter plus tard.
    #[error("message différé par le serveur : {0}")]
    Deferred(String),
    /// Connexion perdue ou délai dépassé pendant la transmission, sans
    /// réponse du serveur : le message est peut-être parti.
    #[error("transmission interrompue, issue incertaine : {0}")]
    Interrupted(String),
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

// ---------------------------------------------------------------------------
// Autodécouverte
// ---------------------------------------------------------------------------

/// Échec d'une source d'autodécouverte. Sans gravité : on passe à la
/// source suivante.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DiscoveryError {
    #[error("erreur réseau : {0}")]
    Network(String),
    #[error("délai dépassé")]
    Timeout,
    #[error("réponse invalide : {0}")]
    Invalid(String),
}

/// Client autoconfig (fichiers `config-v1.1.xml` du fournisseur ou de
/// l'ISPDB).
pub trait AutoconfigClient: Send + Sync {
    /// Télécharge et analyse le fichier. `Ok(None)` : aucun fichier à cette
    /// adresse (404, nom inconnu…).
    fn fetch(
        &self,
        request: &AutoconfigRequest,
    ) -> impl Future<Output = Result<Option<DiscoveredServers>, DiscoveryError>> + Send;
}

/// Résolution DNS SRV.
pub trait SrvResolver: Send + Sync {
    /// Enregistrements de `<service>.<domaine>` ; liste vide si aucun.
    fn lookup(
        &self,
        service: SrvService,
        domain: &Host,
    ) -> impl Future<Output = Result<Vec<SrvRecord>, DiscoveryError>> + Send;
}

/// Sondage de serveurs (connexion TCP et salutation, délai court).
pub trait PortProber: Send + Sync {
    /// Candidats qui répondent, dans l'ordre d'entrée.
    fn probe(
        &self,
        candidates: Vec<ServerCandidate>,
    ) -> impl Future<Output = Vec<ServerCandidate>> + Send;
}

// ---------------------------------------------------------------------------
// Comptes
// ---------------------------------------------------------------------------

/// Erreur du dépôt de comptes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AccountStoreError {
    #[error("un compte existe déjà pour {0}")]
    Duplicate(EmailAddress),
    #[error("données de compte corrompues : {0}")]
    Corrupted(String),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Dépôt des comptes (sans secret).
pub trait AccountRepository: Send + Sync {
    /// Comptes, par ordre de création.
    fn list(&self) -> impl Future<Output = Result<Vec<Account>, AccountStoreError>> + Send;

    fn get(
        &self,
        id: AccountId,
    ) -> impl Future<Output = Result<Option<Account>, AccountStoreError>> + Send;

    fn find_by_address(
        &self,
        address: &EmailAddress,
    ) -> impl Future<Output = Result<Option<Account>, AccountStoreError>> + Send;

    /// Enregistre un compte ; une adresse déjà présente est refusée
    /// ([`AccountStoreError::Duplicate`]).
    fn insert(
        &self,
        draft: &AccountDraft,
    ) -> impl Future<Output = Result<Account, AccountStoreError>> + Send;

    /// Supprime un compte et ses données en base ; `false` s'il n'existait
    /// pas.
    fn delete(&self, id: AccountId)
    -> impl Future<Output = Result<bool, AccountStoreError>> + Send;
}

/// Données locales d'un compte hors base (messages bruts, caches).
pub trait AccountDataStore: Send + Sync {
    /// Supprime toutes les données locales du compte ; sans effet s'il n'y
    /// en a pas.
    fn purge(&self, id: AccountId) -> impl Future<Output = Result<(), StoreError>> + Send;
}

// ---------------------------------------------------------------------------
// Messages bruts
// ---------------------------------------------------------------------------

/// Identifiant d'un message brut stocké : empreinte SHA-256 de son
/// contenu, en hexadécimal minuscule (64 caractères).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlobId(String);

impl BlobId {
    /// `None` si ce n'est pas exactement 64 chiffres hexadécimaux
    /// minuscules (l'identifiant sert de nom de fichier).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let valid = raw.len() == 64 && raw.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        valid.then(|| Self(raw.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BlobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Stockage des messages bruts, par compte et par empreinte de contenu.
pub trait BlobStore: Send + Sync {
    /// Enregistre un contenu (idempotent : même contenu, même
    /// identifiant) ; écriture atomique.
    fn put(
        &self,
        account: AccountId,
        bytes: Vec<u8>,
    ) -> impl Future<Output = Result<BlobId, StoreError>> + Send;

    /// Contenu, `None` s'il n'existe pas. Un contenu corrompu (empreinte
    /// différente) est une erreur.
    fn get(
        &self,
        account: AccountId,
        id: &BlobId,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, StoreError>> + Send;

    /// Supprime un contenu ; sans effet s'il n'existe pas.
    fn delete(
        &self,
        account: AccountId,
        id: &BlobId,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}

// ---------------------------------------------------------------------------
// Stockage de la synchronisation
// ---------------------------------------------------------------------------

/// Où trouver le corps d'un message localement.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum StoredBody {
    /// Pas encore téléchargé.
    #[default]
    Missing,
    /// Message brut complet.
    Full(BlobId),
    /// En-têtes et parties téléchargées ; `deferred` : parties à
    /// télécharger à la demande.
    Partial {
        header: BlobId,
        parts: Vec<(PartPath, BlobId)>,
        deferred: Vec<PartPath>,
    },
}

/// Message à enregistrer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredMessage {
    pub meta: MessageMeta,
    pub body: StoredBody,
}

/// Lot de changements d'une boîte, enregistré atomiquement.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncBatch {
    /// Supprimer d'abord tous les messages de la boîte.
    pub reset: bool,
    pub removed: Vec<Uid>,
    pub flag_changes: Vec<FlagUpdate>,
    /// Nouveaux messages (remplacent un message de même UID).
    pub new: Vec<StoredMessage>,
    pub state: MailboxSyncState,
}

/// Message connu dont le corps n'est pas encore téléchargé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingBody {
    pub uid: Uid,
    pub size: u32,
    pub structure: Option<MimePart>,
}

/// Boîte locale d'un compte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredMailbox {
    pub id: MailboxId,
    pub mailbox: RemoteMailbox,
}

/// Stockage de la synchronisation (côté écriture).
pub trait MailStore: Send + Sync {
    /// Remplace la liste des boîtes d'un compte : ajoute les nouvelles, met
    /// à jour rôle et attributs, supprime (avec leurs messages) celles qui
    /// ont disparu. Renvoie les boîtes avec leur identifiant local, dans
    /// l'ordre reçu ; un identifiant est stable tant que le nom ne change
    /// pas.
    fn replace_mailboxes(
        &self,
        account: AccountId,
        mailboxes: &[RemoteMailbox],
    ) -> impl Future<Output = Result<Vec<StoredMailbox>, StoreError>> + Send;

    /// État et messages connus d'une boîte.
    fn local_mailbox(
        &self,
        mailbox: MailboxId,
    ) -> impl Future<Output = Result<LocalMailbox, StoreError>> + Send;

    /// Applique un lot, d'un bloc (tout ou rien).
    fn apply_sync(
        &self,
        mailbox: MailboxId,
        batch: &SyncBatch,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Messages dont le corps reste à télécharger (synchronisation
    /// interrompue), par UID décroissant, au plus `limit`.
    fn missing_bodies(
        &self,
        mailbox: MailboxId,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<PendingBody>, StoreError>> + Send;

    /// Enregistre le corps d'un message déjà connu, avec son texte
    /// cherchable (`None` : texte inchangé) ; sans effet s'il a disparu.
    fn set_body(
        &self,
        mailbox: MailboxId,
        uid: Uid,
        body: &StoredBody,
        text: Option<&str>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}

// ---------------------------------------------------------------------------
// Requêtes d'écran (CQRS léger)
// ---------------------------------------------------------------------------

/// Nœud de l'arborescence des dossiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderNode {
    pub id: MailboxId,
    pub name: MailboxName,
    pub delimiter: Option<char>,
    pub role: MailboxRole,
    pub selectable: bool,
    pub total: u32,
    pub unread: u32,
}

/// Arborescence des dossiers d'un compte.
pub trait FolderTreeQuery: Send + Sync {
    /// Dossiers du compte dans l'ordre d'affichage, en arbre
    /// ([`crate::mailbox::folder_display_order`]).
    fn tree(
        &self,
        account: AccountId,
    ) -> impl Future<Output = Result<Vec<FolderNode>, StoreError>> + Send;
}

/// Identifiant local d'un message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MessageId(i64);

impl MessageId {
    #[must_use]
    pub fn new(raw: i64) -> Option<Self> {
        (raw > 0).then_some(Self(raw))
    }

    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }
}

/// Position dans une liste triée par date décroissante (pagination par
/// curseur : stable même si des messages arrivent entre deux pages).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageCursor {
    pub date: i64,
    pub id: MessageId,
}

/// Ligne de la liste des messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageListItem {
    pub id: MessageId,
    pub uid: Uid,
    pub date: i64,
    pub from: Option<Correspondent>,
    pub subject: String,
    pub flags: Flags,
    pub has_attachments: bool,
    pub size: u32,
}

/// Une page de messages.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MessagePage {
    pub items: Vec<MessageListItem>,
    /// Curseur de la page suivante ; `None` à la fin.
    pub next: Option<PageCursor>,
}

/// Taille maximale d'une page.
pub const MAX_PAGE_SIZE: u32 = 500;

/// Liste paginée des messages d'un dossier.
pub trait MessageListQuery: Send + Sync {
    /// Au plus `limit` messages (borné à [`MAX_PAGE_SIZE`]) après
    /// `cursor`, du plus récent au plus ancien.
    fn page(
        &self,
        mailbox: MailboxId,
        cursor: Option<PageCursor>,
        limit: u32,
    ) -> impl Future<Output = Result<MessagePage, StoreError>> + Send;
}

// ---------------------------------------------------------------------------
// Conversations (CQRS léger)
// ---------------------------------------------------------------------------

/// Identifiant local d'une conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConversationId(i64);

impl ConversationId {
    #[must_use]
    pub fn new(raw: i64) -> Option<Self> {
        (raw > 0).then_some(Self(raw))
    }

    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }
}

/// Position dans une liste de conversations (date du message le plus
/// récent décroissante, puis identifiant décroissant).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConversationCursor {
    pub date: i64,
    pub id: ConversationId,
}

/// Nombre maximal d'expéditeurs affichés par conversation.
pub const MAX_LISTED_SENDERS: usize = 3;

/// Vrai si deux correspondants désignent le même expéditeur : même adresse
/// (casse ignorée), ou même nom si l'un n'a pas d'adresse.
#[must_use]
pub fn same_sender(a: &Correspondent, b: &Correspondent) -> bool {
    if a.address.is_empty() || b.address.is_empty() {
        a.address.is_empty() && b.address.is_empty() && a.name == b.name
    } else {
        a.address.eq_ignore_ascii_case(&b.address)
    }
}

/// Ligne de la liste des conversations. Les compteurs et la date portent
/// sur les messages de la conversation présents dans le dossier de la
/// ligne.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationListItem {
    pub id: ConversationId,
    pub account: AccountId,
    pub mailbox: MailboxId,
    /// Date du message le plus récent.
    pub date: i64,
    /// Sujet du message le plus récent.
    pub subject: String,
    /// Expéditeurs distincts, du plus récent au plus ancien, au plus
    /// [`MAX_LISTED_SENDERS`].
    pub senders: Vec<Correspondent>,
    pub messages: u32,
    pub unread: u32,
    pub flagged: bool,
    pub has_attachments: bool,
}

/// Une page de conversations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConversationPage {
    pub items: Vec<ConversationListItem>,
    pub next: Option<ConversationCursor>,
}

/// Message d'une conversation (vue empilée).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationMessage {
    pub id: MessageId,
    pub account: AccountId,
    pub mailbox: MailboxId,
    pub uid: Uid,
    pub date: i64,
    pub from: Option<Correspondent>,
    pub to: Vec<Correspondent>,
    pub cc: Vec<Correspondent>,
    pub subject: String,
    pub flags: Flags,
    pub has_attachments: bool,
}

/// Nombre maximal de messages renvoyés pour une conversation.
pub const MAX_CONVERSATION_MESSAGES: usize = 500;

/// Listes de conversations, précalculées (ADR-003).
pub trait ConversationQuery: Send + Sync {
    /// Au plus `limit` conversations (borné à [`MAX_PAGE_SIZE`]) après
    /// `cursor`, de la plus récente à la plus ancienne.
    fn conversations(
        &self,
        scope: ConversationScope,
        cursor: Option<ConversationCursor>,
        limit: u32,
    ) -> impl Future<Output = Result<ConversationPage, StoreError>> + Send;

    /// Messages d'une conversation, tous dossiers confondus, du plus
    /// ancien au plus récent ; un même message présent dans plusieurs
    /// dossiers (même `Message-ID`) n'apparaît qu'une fois. Au plus
    /// [`MAX_CONVERSATION_MESSAGES`] (les plus récents).
    fn conversation_messages(
        &self,
        conversation: ConversationId,
    ) -> impl Future<Output = Result<Vec<ConversationMessage>, StoreError>> + Send;
}

/// Emplacement local d'un message et de son corps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageLocation {
    pub id: MessageId,
    pub account: AccountId,
    pub mailbox: MailboxId,
    pub mailbox_name: MailboxName,
    pub uid: Uid,
    pub from: Option<Correspondent>,
    /// `RFC822.SIZE`, en octets.
    pub size: u32,
    /// Structure annoncée par le serveur (parties différées).
    pub structure: Option<MimePart>,
    pub body: StoredBody,
}

/// Accès à l'emplacement d'un message (lecture du corps).
pub trait MessageLocator: Send + Sync {
    fn locate(
        &self,
        message: MessageId,
    ) -> impl Future<Output = Result<Option<MessageLocation>, StoreError>> + Send;
}

/// Téléchargement à la demande d'un corps (parties différées), par la
/// connexion IMAP du compte.
pub trait RemoteBodyFetcher: Send + Sync {
    /// Corps du message `uid` de la boîte `mailbox` ; `None` s'il a
    /// disparu.
    fn fetch(
        &self,
        account: AccountId,
        mailbox: &MailboxName,
        uid: Uid,
        fetch: &BodyFetch,
    ) -> impl Future<Output = Result<Option<FetchedBody>, ConnectionError>> + Send;
}

// ---------------------------------------------------------------------------
// Actions et file d'opérations
// ---------------------------------------------------------------------------

/// Identifiant d'une opération en attente.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PendingOpId(i64);

impl PendingOpId {
    #[must_use]
    pub fn new(raw: i64) -> Option<Self> {
        (raw > 0).then_some(Self(raw))
    }

    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }
}

/// File durable des opérations (`pending_ops`) et application locale de
/// leurs effets. Chaque méthode qui touche la file applique l'effet local
/// correspondant dans la même transaction : la base ne montre jamais une
/// action sans l'opération qui la porte, ni l'inverse.
pub trait PendingOpStore: Send + Sync {
    /// Message visé par une action, avec l'état de sa boîte ; `None` s'il
    /// n'existe pas ou est déjà masqué (action en attente).
    fn action_target(
        &self,
        message: MessageId,
    ) -> impl Future<Output = Result<Option<LocalMessage>, StoreError>> + Send;

    /// Messages visibles d'une conversation, tous dossiers et exemplaires
    /// compris (sans dédoublonnage par `Message-ID`), au plus
    /// [`crate::actions::MAX_ACTION_TARGETS`].
    fn conversation_members(
        &self,
        conversation: ConversationId,
    ) -> impl Future<Output = Result<Vec<Member>, StoreError>> + Send;

    /// Dossiers d'un compte (résolution des actions).
    fn action_folders(
        &self,
        account: AccountId,
    ) -> impl Future<Output = Result<Vec<FolderRef>, StoreError>> + Send;

    /// Applique un lot de changements de la file
    /// ([`crate::actions::plan_queue`]) en une seule transaction : tout ou
    /// rien. Renvoie les identifiants des opérations enfilées, dans l'ordre.
    ///
    /// - [`QueueChange::Push`] enfile l'opération et applique son effet
    ///   local ([`crate::actions::local_effect`]). Un déplacement crée
    ///   aussi une copie provisoire du message dans le dossier de
    ///   destination, que [`PendingOpStore::revert`] et
    ///   [`PendingOpStore::cancel`] retirent ; ces copies sont ignorées par
    ///   la synchronisation et par les actions.
    /// - [`QueueChange::CancelOrPush`] annule l'opération existante comme
    ///   [`PendingOpStore::cancel`] ; si elle est réservée par le rejeu,
    ///   enfile la nouvelle à la place.
    fn record_all(
        &self,
        changes: &[QueueChange],
    ) -> impl Future<Output = Result<Vec<PendingOpId>, StoreError>> + Send;

    /// Opérations en attente d'un compte, dans l'ordre d'enregistrement.
    fn pending(
        &self,
        account: AccountId,
    ) -> impl Future<Output = Result<Vec<(PendingOpId, PendingOp)>, StoreError>> + Send;

    /// Opération rejouée avec succès : retirée de la file (l'état local
    /// reflète déjà l'action). `false` si elle n'existait plus.
    fn complete(&self, id: PendingOpId) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Opération abandonnée par le rejeu (conflit, refus, trop d'essais) :
    /// retirée de la file et effet local inversé
    /// ([`crate::actions::revert_effect`]), qu'elle soit réservée ou non.
    /// `false` si elle n'existait plus.
    fn revert(&self, id: PendingOpId) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Opération dont le dossier source ou la destination a été renommé (ou
    /// déplacé) sur le serveur ([`crate::actions::PendingOp::follow_folder`]) :
    /// nouveau dossier source `mailbox`, et nouvelle destination `target`
    /// pour un déplacement (ignorée sinon ; `None` : inchangée). `false` si
    /// elle n'existait plus.
    fn relocate(
        &self,
        id: PendingOpId,
        mailbox: &MailboxName,
        target: Option<&MailboxName>,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Annulation par l'utilisateur (ou fusion) : comme
    /// [`PendingOpStore::revert`], mais seulement si l'opération n'est pas
    /// réservée par le rejeu (`false` sinon : trop tard).
    fn cancel(&self, id: PendingOpId) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Réserve l'opération pour le rejeu, avant tout envoi au serveur, et
    /// compte un essai : renvoie le nombre d'essais (celui-ci compris),
    /// `None` si l'opération n'existe plus.
    fn claim(
        &self,
        id: PendingOpId,
    ) -> impl Future<Output = Result<Option<u32>, StoreError>> + Send;

    /// Retire les copies provisoires des déplacements déjà rejoués (leur
    /// opération a quitté la file), dans `mailbox` ou dans tout le compte.
    /// À appeler juste après la synchronisation de la destination, qui
    /// apporte le vrai message : ni trou ni doublon visible.
    fn purge_placeholders(
        &self,
        account: AccountId,
        mailbox: Option<MailboxId>,
    ) -> impl Future<Output = Result<usize, StoreError>> + Send;

    /// Réapplique les effets locaux des opérations en attente d'un compte
    /// (après une synchronisation qui a rapporté l'état du serveur).
    fn reapply(&self, account: AccountId) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Dossier de repli de « archiver » (compte sans `\Archive`).
    fn archive_fallback(
        &self,
        account: AccountId,
    ) -> impl Future<Output = Result<Option<MailboxId>, StoreError>> + Send;

    fn set_archive_fallback(
        &self,
        account: AccountId,
        mailbox: Option<MailboxId>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}

// ---------------------------------------------------------------------------
// Rédaction : brouillons et file d'envoi
// ---------------------------------------------------------------------------

/// Identifiant local d'un brouillon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DraftId(i64);

impl DraftId {
    #[must_use]
    pub fn new(raw: i64) -> Option<Self> {
        (raw > 0).then_some(Self(raw))
    }

    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }
}

/// Brouillon enregistré.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredDraft {
    pub id: DraftId,
    pub draft: Draft,
    pub sync: DraftSync,
    /// Dernier enregistrement (secondes Unix).
    pub updated_at: i64,
}

/// Brouillons locaux et copies distantes à supprimer.
pub trait DraftStore: Send + Sync {
    /// Enregistre un brouillon (le crée si `id` est absent) et incrémente
    /// sa révision ([`DraftSync::edited`]). Un brouillon inconnu, ou d'un
    /// autre compte, est une erreur.
    fn save_draft(
        &self,
        id: Option<DraftId>,
        draft: &Draft,
        now: i64,
    ) -> impl Future<Output = Result<StoredDraft, StoreError>> + Send;

    fn draft(
        &self,
        id: DraftId,
    ) -> impl Future<Output = Result<Option<StoredDraft>, StoreError>> + Send;

    /// Brouillons d'un compte, du plus récent au plus ancien.
    fn drafts(
        &self,
        account: AccountId,
    ) -> impl Future<Output = Result<Vec<StoredDraft>, StoreError>> + Send;

    /// Copie distante faite : n'écrit que l'état distant de `sync`
    /// (révision copiée, copie, instant), jamais la révision locale. Si
    /// `replaced` est donnée (ancienne copie), elle rejoint les copies à
    /// supprimer dans la même transaction. `false` si le brouillon de
    /// `account` n'existe plus : la nouvelle copie rejoint alors aussi les
    /// copies à supprimer. Les copies à supprimer forment un ensemble.
    fn set_remote(
        &self,
        account: AccountId,
        id: DraftId,
        sync: DraftSync,
        replaced: Option<RemoteCopy>,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Supprime un brouillon ; sa copie distante rejoint les copies à
    /// supprimer, dans la même transaction. Renvoie le brouillon retiré.
    fn delete_draft(
        &self,
        id: DraftId,
    ) -> impl Future<Output = Result<Option<StoredDraft>, StoreError>> + Send;

    /// Copies distantes (dossier « Brouillons ») à supprimer du serveur.
    fn remote_trash(
        &self,
        account: AccountId,
    ) -> impl Future<Output = Result<Vec<RemoteCopy>, StoreError>> + Send;

    /// Copie supprimée du serveur (ou disparue) : oubliée.
    fn forget_remote(
        &self,
        account: AccountId,
        copy: RemoteCopy,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}

/// Pièce jointe relue du stockage, à joindre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposedAttachment {
    pub file_name: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// Message à construire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Composition {
    pub from: Recipient,
    pub draft: Draft,
    pub attachments: Vec<ComposedAttachment>,
    /// Date, secondes Unix.
    pub date: i64,
    /// `Message-ID`, sans chevrons ([`MessageComposer::new_message_id`]).
    pub message_id: String,
}

/// Message construit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposedMessage {
    /// Enveloppe (Cci compris) et message transmis, sans `Bcc`.
    pub outgoing: OutgoingMessage,
    /// Copie locale (brouillon, Envoyés), avec `Bcc`.
    pub local_copy: Vec<u8>,
}

/// Échec de construction d'un message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ComposeBuildError {
    #[error("adresse refusée : {0}")]
    Address(String),
    #[error("construction MIME impossible : {0}")]
    Mime(String),
    #[error("aléa du système indisponible")]
    Random,
}

/// Écriture et lecture MIME pour la rédaction (adaptateur `render`).
pub trait MessageComposer: Send + Sync {
    /// `Message-ID` aléatoire dans le domaine de l'expéditeur.
    fn new_message_id(&self, from: &EmailAddress) -> Result<String, ComposeBuildError>;

    /// Construit le message (HTML réassaini, version texte, pièces
    /// jointes).
    fn compose(&self, composition: &Composition) -> Result<ComposedMessage, ComposeBuildError>;

    /// Relit un message reçu pour y répondre ou le transférer (en-têtes
    /// assainis, `Reply-To`, texte). `None` s'il est illisible.
    fn reply_source(&self, raw: &[u8]) -> Option<ReplySource>;

    /// Pièces jointes d'un message reçu, décodées, pour le transférer :
    /// nom assaini, type, contenu ; corps texte et HTML exclus. Vide s'il
    /// est illisible. Mêmes bornes que l'envoi, vérifiées pendant
    /// l'itération : au-delà de `MAX_ATTACHMENTS` pièces ou de
    /// `MAX_ATTACHMENTS_BYTES` octets décodés, l'analyse s'arrête sur
    /// `TooManyAttachments` ou `AttachmentsTooLarge`.
    fn forwarded_attachments(&self, raw: &[u8]) -> Result<Vec<ComposedAttachment>, ComposeError>;

    /// HTML d'un texte brut (échappé, paragraphes), pour reprendre un
    /// brouillon dans l'éditeur.
    fn text_html(&self, text: &str) -> String;

    /// HTML d'éditeur d'une citation : `attribution` puis `text` dans un
    /// `<blockquote>`, échappés.
    fn quote_html(&self, text: &str, attribution: &str) -> String;
}

/// Suggestions au plus, quelle que soit la demande.
pub const MAX_CONTACT_SUGGESTIONS: usize = 20;

/// Adresses connues, pour l'autocomplétion des destinataires.
pub trait ContactQuery: Send + Sync {
    /// Correspondants d'un compte dont le nom ou l'adresse contient `text`
    /// (sans distinction de casse ; texte vide : aucun), les plus
    /// fréquents d'abord, une seule fois par adresse ; au plus `limit`
    /// (borné à [`MAX_CONTACT_SUGGESTIONS`]). Sources : expéditeurs des
    /// messages reçus, hors Indésirables et Corbeille (un inconnu ne peut
    /// pas s'inviter dans les suggestions), destinataires (À, Cc) des
    /// messages du dossier Envoyés.
    fn suggest(
        &self,
        account: AccountId,
        text: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<Correspondent>, StoreError>> + Send;
}

// ---------------------------------------------------------------------------
// Carnet d'adresses (ADR-012)
// ---------------------------------------------------------------------------

/// Erreur du carnet d'adresses.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContactStoreError {
    /// L'adresse (à la casse près) appartient déjà à un autre contact.
    #[error("adresse déjà dans le carnet : {0}")]
    DuplicateAddress(EmailAddress),
    #[error("contact inconnu")]
    NotFound,
    /// Une fiche a changé depuis sa lecture (fusion).
    #[error("contact modifié entre-temps")]
    Conflict,
    #[error("données de contact corrompues : {0}")]
    Corrupted(String),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Écritures du carnet d'adresses (unique, commun à tous les comptes).
///
/// Une adresse n'appartient qu'à un contact du carnet, à la casse près
/// ([`AddressKey`]) : l'écriture qui la donnerait à un second contact est
/// refusée sans effet.
pub trait ContactBook: Send + Sync {
    /// Enregistre une nouvelle fiche.
    fn create(
        &self,
        draft: &ContactDraft,
    ) -> impl Future<Output = Result<Contact, ContactStoreError>> + Send;

    /// Remplace le contenu de la fiche `id` (adresses et téléphones
    /// compris). `NotFound` si elle n'existe pas.
    fn update(
        &self,
        id: ContactId,
        draft: &ContactDraft,
    ) -> impl Future<Output = Result<Contact, ContactStoreError>> + Send;

    /// Supprime une fiche ; `false` si elle n'existait pas.
    fn delete(&self, id: ContactId)
    -> impl Future<Output = Result<bool, ContactStoreError>> + Send;

    /// Supprime des fiches en une fois (inconnues ignorées) ; renvoie le
    /// nombre de fiches supprimées.
    fn delete_many(
        &self,
        ids: &[ContactId],
    ) -> impl Future<Output = Result<usize, ContactStoreError>> + Send;

    /// Importe en une transaction : enregistre les fiches `create`,
    /// remplace le contenu des contacts `complete` (identifiant existant,
    /// nouveau contenu). Tout ou rien : une adresse déjà portée par un
    /// autre contact (`DuplicateAddress`) ou un contact inconnu
    /// (`NotFound`) annule tout l'import.
    fn import(
        &self,
        create: &[ContactDraft],
        complete: &[Contact],
    ) -> impl Future<Output = Result<ImportOutcome, ContactStoreError>> + Send;

    /// Fusionne en une transaction ([`crate::contact_duplicates`]) : la
    /// fiche `keep` prend le contenu `draft`, elle remplace chaque fiche
    /// `absorbed` dans ses listes de diffusion (à sa place, sauf si `keep`
    /// y figure déjà), puis les fiches `absorbed` sont supprimées. `keep`
    /// et `absorbed` sont les fiches telles que lues pour calculer
    /// `draft`. Tout ou rien : une fiche inconnue (`NotFound`), une fiche
    /// qui a changé depuis (`Conflict`) ou une adresse de `draft` portée
    /// par un contact étranger à la fusion (`DuplicateAddress`) annule
    /// tout.
    fn merge(
        &self,
        keep: &Contact,
        draft: &ContactDraft,
        absorbed: &[Contact],
    ) -> impl Future<Output = Result<Contact, ContactStoreError>> + Send;
}

/// Bilan d'un import écrit dans le carnet.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportOutcome {
    /// Fiches créées.
    pub created: usize,
    /// Contacts existants complétés.
    pub completed: usize,
}

/// Fichier de contacts refusé en entier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ContactCodecError {
    /// Plus de [`crate::contact_import::MAX_IMPORT_FILE_BYTES`] octets.
    #[error("fichier de contacts trop gros")]
    TooLarge,
    /// Plus de [`crate::contact_import::MAX_IMPORT_CARDS`] fiches.
    #[error("trop de fiches dans le fichier")]
    TooManyCards,
    /// Ni vCard, ni LDIF, ni CSV d'un format reconnu.
    #[error("format de fichier de contacts inconnu")]
    UnknownFormat,
}

/// Traduction d'un fichier de contacts (vCard, CSV, LDIF) en fiches à
/// importer, et de fiches à exporter en fichier (adaptateur
/// `contacts-io`). Ne décide rien : les fiches lues sont revalidées,
/// dédoublonnées et normalisées par
/// [`crate::contact_import::plan_import`] ; les fiches écrites sont
/// préparées par [`crate::contact_export::export_card`].
pub trait ContactCodec: Send + Sync {
    /// Fiches du fichier, dans son ordre ; une fiche illisible est rendue
    /// comme telle (sans arrêter la lecture des suivantes).
    fn decode(&self, bytes: &[u8]) -> Result<DecodedContacts, ContactCodecError>;

    /// Fichier des fiches `cards` (préparées par
    /// [`crate::contact_export::export_card`] pour `format`), dans leur
    /// ordre. Refusé au-delà de
    /// [`crate::contact_export::MAX_EXPORT_CONTACTS`] fiches
    /// (`TooManyCards`) ou de
    /// [`crate::contact_export::MAX_EXPORT_FILE_BYTES`] octets (`TooLarge`,
    /// sans produire plus).
    fn encode(
        &self,
        cards: &[ExportCard],
        format: ExportFormat,
    ) -> Result<Vec<u8>, ContactCodecError>;
}

/// Normalisation Unicode d'un texte (forme NFC), fournie au domaine qui
/// n'embarque pas les tables Unicode.
pub trait TextNormalizer: Send + Sync {
    /// Le texte en forme normalisée NFC (composée).
    fn nfc(&self, text: &str) -> String;
}

/// Position dans la liste des contacts, triée par [`ContactDraft::sort_key`]
/// puis par identifiant (pagination par curseur).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactCursor {
    pub sort_key: String,
    pub id: ContactId,
}

/// Longueur maximale d'une clé de tri reçue dans un curseur, en
/// caractères : celle d'un prénom et d'un nom de longueur maximale, que
/// le passage en minuscules peut allonger.
pub const MAX_CONTACT_SORT_KEY_CHARS: usize = 4 * MAX_CONTACT_NAME_CHARS + 1;

impl ContactCursor {
    /// Curseur reçu de l'interface : identifiant valide, clé de tri
    /// bornée à [`MAX_CONTACT_SORT_KEY_CHARS`] ; sinon `None`.
    #[must_use]
    pub fn parse(sort_key: String, id: i64) -> Option<Self> {
        let id = ContactId::new(id)?;
        (sort_key.chars().count() <= MAX_CONTACT_SORT_KEY_CHARS).then_some(Self { sort_key, id })
    }

    /// Position du contact dans la liste.
    #[must_use]
    pub fn of(contact: &Contact) -> Self {
        Self {
            sort_key: contact.draft().sort_key(),
            id: contact.id(),
        }
    }
}

/// Ligne de la liste des contacts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactListItem {
    pub id: ContactId,
    /// Nom à afficher ([`ContactDraft::display_name`]).
    pub name: String,
    /// Adresse préférée.
    pub address: EmailAddress,
    pub organization: Option<String>,
    pub favorite: bool,
}

impl From<&Contact> for ContactListItem {
    fn from(contact: &Contact) -> Self {
        let draft = contact.draft();
        Self {
            id: contact.id(),
            name: draft.display_name(),
            address: draft.preferred_email().address().clone(),
            organization: draft.organization().map(str::to_owned),
            favorite: draft.favorite(),
        }
    }
}

/// Une page de contacts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContactPage {
    pub items: Vec<ContactListItem>,
    /// Curseur de la page suivante ; `None` à la fin.
    pub next: Option<ContactCursor>,
}

/// Adresses cherchées au plus par [`ContactDirectory::by_addresses`].
pub const MAX_CONTACT_LOOKUP: usize = 500;

/// Lectures d'écran du carnet d'adresses.
pub trait ContactDirectory: Send + Sync {
    /// Au plus `limit` contacts (borné à [`MAX_PAGE_SIZE`]) qui passent
    /// `filter` ([`ContactFilter::matches`]), après `cursor`, dans l'ordre
    /// de [`ContactCursor`].
    fn page(
        &self,
        filter: &ContactFilter,
        cursor: Option<ContactCursor>,
        limit: u32,
    ) -> impl Future<Output = Result<ContactPage, StoreError>> + Send;

    /// Fiche complète.
    fn get(
        &self,
        id: ContactId,
    ) -> impl Future<Output = Result<Option<Contact>, StoreError>> + Send;

    /// Contacts qui portent l'une des adresses (les
    /// [`MAX_CONTACT_LOOKUP`] premières), chacun une fois, dans l'ordre de
    /// la liste.
    fn by_addresses(
        &self,
        keys: &[AddressKey],
    ) -> impl Future<Output = Result<Vec<Contact>, StoreError>> + Send;

    /// Fiches complètes de `selection` (inconnues omises), dans l'ordre
    /// de [`ContactCursor`], au plus `limit` (export).
    fn contacts(
        &self,
        selection: &ContactSelection,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<Contact>, StoreError>> + Send;
}

// ---------------------------------------------------------------------------
// Listes de diffusion (ADR-012)
// ---------------------------------------------------------------------------

/// Erreur des listes de diffusion.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContactListStoreError {
    /// Une autre liste porte déjà ce nom, à la casse près (nom refusé
    /// cité tel que saisi).
    #[error("liste déjà dans le carnet : {0}")]
    DuplicateName(String),
    /// Un membre n'est pas (ou plus) dans le carnet.
    #[error("contact inconnu : {0}")]
    UnknownContact(ContactId),
    #[error("liste inconnue")]
    NotFound,
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Écritures des listes de diffusion du carnet.
///
/// Un nom n'appartient qu'à une liste, à la casse près
/// ([`ContactListDraft::name_key`]) ; les membres sont des contacts du
/// carnet. Une écriture refusée est sans effet. Supprimer un contact le
/// retire de toutes les listes ; supprimer une liste ne supprime aucun
/// contact.
pub trait ContactListBook: Send + Sync {
    /// Enregistre une nouvelle liste.
    fn create_list(
        &self,
        draft: &ContactListDraft,
    ) -> impl Future<Output = Result<ContactList, ContactListStoreError>> + Send;

    /// Remplace le contenu de la liste `id` (membres compris). `NotFound`
    /// si elle n'existe pas.
    fn update_list(
        &self,
        id: ContactListId,
        draft: &ContactListDraft,
    ) -> impl Future<Output = Result<ContactList, ContactListStoreError>> + Send;

    /// Supprime des listes en une fois (inconnues ignorées) ; renvoie le
    /// nombre de listes supprimées.
    fn delete_lists(
        &self,
        ids: &[ContactListId],
    ) -> impl Future<Output = Result<usize, ContactListStoreError>> + Send;
}

/// Position dans la liste des listes, triée par
/// [`ContactListDraft::name_key`] puis par identifiant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactListCursor {
    pub sort_key: String,
    pub id: ContactListId,
}

/// Longueur maximale d'une clé de tri de liste reçue dans un curseur, en
/// caractères (le passage en minuscules peut allonger le nom).
pub const MAX_CONTACT_LIST_SORT_KEY_CHARS: usize = 3 * MAX_CONTACT_NAME_CHARS;

impl ContactListCursor {
    /// Curseur reçu de l'interface : identifiant valide, clé de tri
    /// bornée à [`MAX_CONTACT_LIST_SORT_KEY_CHARS`] ; sinon `None`.
    #[must_use]
    pub fn parse(sort_key: String, id: i64) -> Option<Self> {
        let id = ContactListId::new(id)?;
        (sort_key.chars().count() <= MAX_CONTACT_LIST_SORT_KEY_CHARS)
            .then_some(Self { sort_key, id })
    }

    /// Position de la liste.
    #[must_use]
    pub fn of(list: &ContactList) -> Self {
        Self {
            sort_key: list.draft().name_key(),
            id: list.id(),
        }
    }
}

/// Ligne de la liste des listes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactListSummary {
    pub id: ContactListId,
    pub name: String,
    /// Nombre de membres.
    pub members: usize,
    pub bcc: bool,
}

impl From<&ContactList> for ContactListSummary {
    fn from(list: &ContactList) -> Self {
        let draft = list.draft();
        Self {
            id: list.id(),
            name: draft.name().to_owned(),
            members: draft.members().len(),
            bcc: draft.bcc(),
        }
    }
}

/// Une page de listes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContactListPage {
    pub items: Vec<ContactListSummary>,
    /// Curseur de la page suivante ; `None` à la fin.
    pub next: Option<ContactListCursor>,
}

/// Lectures d'écran des listes de diffusion.
pub trait ContactListDirectory: Send + Sync {
    /// Au plus `limit` listes (borné à [`MAX_PAGE_SIZE`]) qui passent
    /// `filter`, après `cursor`, dans l'ordre de [`ContactListCursor`].
    fn list_page(
        &self,
        filter: &ContactListFilter,
        cursor: Option<ContactListCursor>,
        limit: u32,
    ) -> impl Future<Output = Result<ContactListPage, StoreError>> + Send;

    /// Liste complète (membres dans leur ordre).
    fn get_list(
        &self,
        id: ContactListId,
    ) -> impl Future<Output = Result<Option<ContactList>, StoreError>> + Send;
}

// ---------------------------------------------------------------------------
// Recherche plein texte
// ---------------------------------------------------------------------------

/// Où chercher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchScope {
    /// Tous les comptes. Un message du dossier `\All` (Gmail : « Tous les
    /// messages ») n'est listé que s'il n'a pas de copie (même
    /// `Message-ID`) dans un autre dossier du compte.
    All,
    /// Un compte (même règle pour `\All`).
    Account(AccountId),
    /// Un dossier.
    Mailbox(MailboxId),
}

/// Nombre maximal de résultats d'une recherche, toutes pages comprises.
pub const MAX_SEARCH_RESULTS: u32 = 1000;
/// Longueur maximale d'un extrait, en caractères.
pub const MAX_SNIPPET_CHARS: usize = 200;
/// Longueur maximale du texte indexé d'un message, en caractères (au-delà,
/// le texte est tronqué).
pub const MAX_INDEXED_TEXT_CHARS: usize = 100_000;

/// Recherche demandée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRequest {
    pub query: Query,
    pub scope: SearchScope,
    /// Décalage du fuseau de l'utilisateur, pour `before:` et `after:`.
    pub utc_offset_seconds: i32,
    /// Résultats à sauter (pages précédentes).
    pub offset: u32,
    /// Au plus [`MAX_PAGE_SIZE`].
    pub limit: u32,
}

/// Message trouvé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub id: MessageId,
    pub account: AccountId,
    pub mailbox: MailboxId,
    pub conversation: Option<ConversationId>,
    pub uid: Uid,
    pub date: i64,
    pub from: Option<Correspondent>,
    pub subject: String,
    /// Passage du texte du message (au plus [`MAX_SNIPPET_CHARS`]
    /// caractères), vide si le texte n'est pas encore connu.
    pub snippet: String,
    pub flags: Flags,
    pub has_attachments: bool,
}

/// Une page de résultats.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchPage {
    pub items: Vec<SearchHit>,
    /// Décalage de la page suivante ; `None` à la fin (ou au-delà de
    /// [`MAX_SEARCH_RESULTS`]).
    pub next: Option<u32>,
}

/// Paramètres de l'application.
pub trait SettingsStore: Send + Sync {
    /// Paramètres enregistrés ; une valeur absente, illisible ou hors
    /// bornes reprend sa valeur par défaut.
    fn settings(&self) -> impl Future<Output = Result<Settings, StoreError>> + Send;

    /// Enregistre des paramètres ; refusés s'ils sont hors bornes
    /// ([`Settings::validate`]).
    fn save_settings(
        &self,
        settings: &Settings,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}

/// Texte cherchable d'un corps téléchargé (adaptateur `render`). Appelé
/// de façon synchrone par les cas d'usage, au moment d'enregistrer le
/// corps ; le texte part avec lui dans [`MailStore::set_body`].
pub trait SearchTextExtractor: fmt::Debug + Send + Sync {
    /// Texte lisible du message (sa partie texte, sinon son HTML réduit
    /// au texte, sans scripts ni styles ; pièces jointes exclues), au plus
    /// [`MAX_INDEXED_TEXT_CHARS`] caractères ; vide s'il est illisible.
    fn searchable_text(&self, body: &FetchedBody, structure: Option<&MimePart>) -> String;
}

/// Index plein texte des messages locaux (fonctionne hors ligne).
pub trait SearchIndex: Send + Sync {
    /// Messages visibles (ni masqués par une action en attente, ni
    /// `\Deleted`) qui satisfont toutes les clauses de la requête. Avec
    /// au moins un terme de texte : les plus pertinents d'abord (le sujet
    /// compte plus que l'expéditeur, puis les destinataires, puis le
    /// texte) ; sinon du plus récent au plus ancien. Une requête vide ne
    /// trouve rien. Les termes de l'utilisateur ne sont jamais interprétés
    /// comme une syntaxe du moteur.
    fn search(
        &self,
        request: SearchRequest,
    ) -> impl Future<Output = Result<SearchPage, StoreError>> + Send;
}

/// Identifiant d'un message de la file d'envoi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OutboxId(i64);

impl OutboxId {
    #[must_use]
    pub fn new(raw: i64) -> Option<Self> {
        (raw > 0).then_some(Self(raw))
    }

    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }
}

/// Message de la file d'envoi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxItem {
    pub id: OutboxId,
    pub account: AccountId,
    /// Enveloppe et message transmis (sans `Bcc`).
    pub message: OutgoingMessage,
    /// Copie pour « Envoyés » (avec `Bcc`).
    pub sent_copy: Vec<u8>,
    /// Sujet, pour l'affichage de la file.
    pub subject: String,
    pub entry: OutboxEntry,
    /// Mise en file (secondes Unix).
    pub queued_at: i64,
}

/// File d'envoi durable.
pub trait OutboxStore: Send + Sync {
    /// Met un message en file ; le brouillon `draft` éventuel est supprimé
    /// dans la même transaction (comme [`DraftStore::delete_draft`]).
    fn enqueue(
        &self,
        account: AccountId,
        message: &OutgoingMessage,
        sent_copy: &[u8],
        subject: &str,
        draft: Option<DraftId>,
        now: i64,
    ) -> impl Future<Output = Result<OutboxId, StoreError>> + Send;

    /// Met en file un message programmé (`entry` : en attente jusqu'à son
    /// échéance), en gardant son brouillon `content` pour pouvoir le
    /// modifier avant l'échéance ; le brouillon `draft` éventuel est
    /// supprimé dans la même transaction.
    #[allow(clippy::too_many_arguments)]
    fn schedule(
        &self,
        account: AccountId,
        message: &OutgoingMessage,
        sent_copy: &[u8],
        subject: &str,
        draft: Option<DraftId>,
        content: &Draft,
        entry: OutboxEntry,
        now: i64,
    ) -> impl Future<Output = Result<OutboxId, StoreError>> + Send;

    /// Retire de la file un message programmé encore en attente et recrée
    /// son brouillon, dans la même transaction. `None` s'il n'existe plus,
    /// n'est plus en attente (envoi en cours, envoyé, en échec) ou n'a pas
    /// de brouillon gardé (message mis en file sans échéance).
    fn unschedule(
        &self,
        id: OutboxId,
        now: i64,
    ) -> impl Future<Output = Result<Option<StoredDraft>, StoreError>> + Send;

    /// Messages d'un compte, dans l'ordre de mise en file.
    fn outbox(
        &self,
        account: AccountId,
    ) -> impl Future<Output = Result<Vec<OutboxItem>, StoreError>> + Send;

    /// Remplace l'état `from` par `to` seulement si l'état courant est
    /// encore `from` (transitions décidées par le domaine, sans course
    /// entre deux appelants). `false` sinon, ou si le message n'existe
    /// plus.
    fn transition(
        &self,
        id: OutboxId,
        from: OutboxEntry,
        to: OutboxEntry,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Retire un message de la file. `false` s'il n'existait plus.
    fn remove(&self, id: OutboxId) -> impl Future<Output = Result<bool, StoreError>> + Send;
}

// ---------------------------------------------------------------------------
// Préférences de lecture
// ---------------------------------------------------------------------------

/// Préférences de lecture : expéditeurs dont les images distantes sont
/// autorisées, préférence pour le texte brut.
pub trait ReadingPreferences: Send + Sync {
    fn remote_images_allowed(
        &self,
        sender: &EmailAddress,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Autorise les images distantes d'un expéditeur (idempotent).
    fn allow_remote_images(
        &self,
        sender: &EmailAddress,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Retire l'autorisation (sans effet si absente).
    fn revoke_remote_images(
        &self,
        sender: &EmailAddress,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Vrai si l'utilisateur préfère lire le texte brut (faux par défaut).
    fn prefer_plain_text(&self) -> impl Future<Output = Result<bool, StoreError>> + Send;

    fn set_prefer_plain_text(
        &self,
        prefer: bool,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
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
    fn secret_key_for_account() {
        let Ok(id) = AccountId::new(7) else {
            panic!("id");
        };
        assert_eq!(
            SecretKey::for_account(id, Protocol::Smtp).as_str(),
            "account:7:smtp"
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
    fn blob_id_is_strict_hex() {
        let ok = "a".repeat(64);
        assert!(BlobId::parse(&ok).is_some());
        for bad in [
            "",
            "A".repeat(64).as_str(),
            "../".repeat(21).as_str(),
            &"a".repeat(63),
        ] {
            assert!(BlobId::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn fetched_body_debug_hides_content() {
        let b = FetchedBody::Full(b"secret".to_vec());
        assert!(!format!("{b:?}").contains("secret"));
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
