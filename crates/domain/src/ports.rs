//! Ports (traits) implémentés par les adaptateurs.
//!
//! Les ports asynchrones renvoient `impl Future + Send` (dispatch statique,
//! ADR-008) ; une enveloppe `dyn` pourra être ajoutée au point d'assemblage.

use std::collections::BTreeSet;
use std::fmt;
use std::future::Future;
use std::time::Duration;

use crate::account::{Account, AccountDraft, AccountId};
use crate::address::EmailAddress;
use crate::auth::{AuthError, AuthMechanism};
use crate::capabilities::ServerCapabilities;
use crate::connection::ConnectionError;
use crate::credentials::{Credentials, Secret, SecretBytes, Username};
use crate::discovery::{
    AutoconfigRequest, DiscoveredServers, ServerCandidate, SrvRecord, SrvService,
};
use crate::endpoint::{Host, Protocol, ServerEndpoint};
use crate::fetch_policy::{BodyFetch, MimePart, PartPath};
use crate::mailbox::{
    Correspondent, Flags, ListedMailbox, MailboxId, MailboxName, MailboxRole, MessageMeta,
    RemoteMailbox, Uid,
};
use crate::security::{CertificateException, PlaintextPolicy};
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

/// Nombre maximal d'UID par requête `FETCH` d'en-têtes.
pub const MAX_FETCH_BATCH: usize = 500;

/// Corps téléchargé d'un message.
#[derive(Clone, PartialEq, Eq)]
pub enum FetchedBody {
    /// Message brut complet.
    Full(Vec<u8>),
    /// En-têtes bruts et parties demandées (contenu encodé tel que reçu).
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
    /// [`MAX_FETCH_BATCH`] UID. Les messages disparus entre-temps sont
    /// absents du résultat.
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

    /// Enregistre le corps d'un message déjà connu ; sans effet s'il a
    /// disparu.
    fn set_body(
        &self,
        mailbox: MailboxId,
        uid: Uid,
        body: &StoredBody,
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
    /// Dossiers du compte : rôles spéciaux d'abord (boîte de réception,
    /// brouillons, envoyés…), puis par nom.
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
