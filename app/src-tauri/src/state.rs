//! Point d'assemblage (ADR-008) : adaptateurs réels branchés sur les cas
//! d'usage.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use norelly_application::discover_settings::DiscoverSettings;
use norelly_application::oauth::{FreshTokens, TokenBroker};
use norelly_application::test_connection::TestConnection;
use norelly_blobs::{FsAccountDataStore, FsBlobStore};
use norelly_contacts_io::{ContactsIo, IcuNfc};
use norelly_discovery::{HickorySrvResolver, HttpAutoconfigClient, TcpPortProber};
use norelly_domain::address::EmailAddress;
use norelly_domain::credentials::{Secret, SecretDestination};
use norelly_domain::discovery::{SrvRecord, SrvService};
use norelly_domain::endpoint::Host;
use norelly_domain::oauth::{OAuthConfig, TokenError};
use norelly_domain::ports::{
    DiscoveryError, SecretKey, SecretStore, SecretStoreError, SrvResolver,
};
use norelly_domain::ports::{OAuthClient, OAuthGrant, PendingOpId};
use norelly_domain::security::CertificateRejection;
use norelly_imap::ImapConnector;
use norelly_keyring::{FileSecretStore, SystemSecretStore};
use norelly_oauth::{BrowserOpener, HttpOAuthClient};
use norelly_smtp::SmtpSender;
use norelly_sqlite::contacts::SqliteContactBook;
use norelly_sqlite::{Database, SqliteAccountRepository, SqliteCertificateExceptionStore};
use norelly_tls::TlsClientFactory;

use crate::carddav::{AppCardDavClient, CardDavServices};
use crate::contact_import::PendingImport;
use crate::mail::MailServices;
use crate::remote_images::RemoteImageProxy;

/// Secrets des comptes : trousseau du système d'abord ; s'il ne répond
/// pas, fichier non chiffré lisible par l'utilisateur seul, **seulement**
/// après consentement explicite (choix du mainteneur, en attendant mieux :
/// voir DECISIONS.md, ADR-002). Lecture et suppression consultent les
/// deux (un trousseau installé après coup ne fait pas perdre les secrets
/// du fichier).
#[derive(Debug)]
pub struct AppSecretStore<S> {
    system: Result<S, SecretStoreError>,
    file: FileSecretStore,
}

impl<S: SecretStore> AppSecretStore<S> {
    #[must_use]
    pub fn new(system: Result<S, SecretStoreError>, file: FileSecretStore) -> Self {
        Self { system, file }
    }

    /// Vrai si un secret peut être enregistré sans demander de
    /// consentement : trousseau joignable, ou fichier déjà consenti.
    pub async fn usable(&self) -> bool {
        self.file.consented() || self.system_reachable().await
    }

    /// Endroit annoncé par l'écran d'ajout pour les nouveaux secrets
    /// ([`SecretDestination`]) : ne lit jamais le trousseau, une lecture
    /// pouvant afficher une invite de déverrouillage. « Ouvert » veut dire
    /// que le service du trousseau a répondu à l'ouverture de la session
    /// (Secret Service : `OpenSession`) ; s'il cesse de répondre ensuite,
    /// l'enregistrement le constate et demande le consentement au fichier,
    /// jamais un repli silencieux.
    #[must_use]
    pub fn announced_destination(&self) -> SecretDestination {
        SecretDestination::announced(self.system.is_ok(), self.file.consented())
    }

    /// Sonde le trousseau du système (par une lecture) : ouvert, et
    /// joignable ? Appelée à l'enregistrement d'un compte, jamais à
    /// l'ouverture d'un écran : la lecture peut afficher une invite de
    /// déverrouillage. Une erreur
    /// autre que l'indisponibilité (refus d'accès, par exemple) compte
    /// comme joignable : le mot de passe n'ira quand même pas dans un
    /// fichier, car le repli n'a lieu que sur indisponibilité, avec
    /// consentement.
    async fn system_reachable(&self) -> bool {
        let Ok(system) = &self.system else {
            return false;
        };
        let Ok(probe) = SecretKey::parse("norelly:sonde") else {
            return false;
        };
        !matches!(
            system.get(&probe).await,
            Err(SecretStoreError::Unavailable(_))
        )
    }

    /// Consentement au fichier en clair.
    pub fn consent_to_file(&self) -> Result<(), SecretStoreError> {
        self.file.consent()
    }

    /// Chemin du fichier de repli.
    #[must_use]
    pub fn file_path(&self) -> &Path {
        self.file.path()
    }
}

impl<S: SecretStore> SecretStore for AppSecretStore<S> {
    async fn get(&self, key: &SecretKey) -> Result<Option<Secret>, SecretStoreError> {
        let system = match &self.system {
            Ok(s) => s.get(key).await,
            Err(e) => Err(e.clone()),
        };
        match system {
            Ok(Some(secret)) => Ok(Some(secret)),
            Ok(None) | Err(SecretStoreError::Unavailable(_)) if self.file.consented() => {
                self.file.get(key).await
            }
            other => other,
        }
    }

    async fn set(&self, key: &SecretKey, secret: &Secret) -> Result<(), SecretStoreError> {
        let system = match &self.system {
            Ok(s) => s.set(key, secret).await,
            Err(e) => Err(e.clone()),
        };
        match system {
            Err(SecretStoreError::Unavailable(_)) if self.file.consented() => {
                self.file.set(key, secret).await
            }
            // Rangé au trousseau du système : une copie plus ancienne,
            // laissée dans le fichier quand le trousseau manquait, ne reste
            // pas lisible.
            Ok(()) if self.file.consented() => {
                if let Err(e) = self.file.delete(key).await {
                    tracing::warn!(error = %e, "ancienne copie du secret non effacée du fichier");
                }
                Ok(())
            }
            other => other,
        }
    }

    async fn delete(&self, key: &SecretKey) -> Result<(), SecretStoreError> {
        let system = match &self.system {
            Ok(s) => s.delete(key).await,
            Err(e) => Err(e.clone()),
        };
        self.file.delete(key).await?;
        match system {
            Err(SecretStoreError::Unavailable(_)) => Ok(()),
            other => other,
        }
    }
}

/// Résolveur SRV du système, ou indisponible (source sautée).
#[derive(Debug)]
pub enum AppSrvResolver {
    System(Box<HickorySrvResolver>),
    Unavailable(DiscoveryError),
}

impl SrvResolver for AppSrvResolver {
    async fn lookup(
        &self,
        service: SrvService,
        domain: &Host,
    ) -> Result<Vec<SrvRecord>, DiscoveryError> {
        match self {
            Self::System(r) => r.lookup(service, domain).await,
            Self::Unavailable(e) => Err(e.clone()),
        }
    }
}

/// Client autoconfig, ou indisponible.
#[derive(Debug)]
pub enum AppAutoconfig {
    Https(HttpAutoconfigClient),
    Unavailable(String),
}

impl norelly_domain::ports::AutoconfigClient for AppAutoconfig {
    async fn fetch(
        &self,
        request: &norelly_domain::discovery::AutoconfigRequest,
    ) -> Result<Option<norelly_domain::discovery::DiscoveredServers>, DiscoveryError> {
        match self {
            Self::Https(c) => c.fetch(request).await,
            Self::Unavailable(e) => Err(DiscoveryError::Network(e.clone())),
        }
    }
}

pub type AppTestConnection =
    TestConnection<ImapConnector, SmtpSender, SqliteCertificateExceptionStore>;

/// Navigateur du système (xdg-open, ShellExecute).
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemBrowser;

impl BrowserOpener for SystemBrowser {
    fn open(&self, url: &str) -> Result<(), String> {
        open::that_detached(url).map_err(|e| e.to_string())
    }
}

/// Client OAuth, ou la raison de son absence (TLS indisponible).
#[derive(Debug)]
pub enum AppOAuth {
    Http(HttpOAuthClient<SystemBrowser>),
    Unavailable(String),
}

impl OAuthClient for AppOAuth {
    async fn authorize(
        &self,
        config: &OAuthConfig,
        login_hint: &EmailAddress,
    ) -> Result<OAuthGrant, TokenError> {
        match self {
            Self::Http(c) => c.authorize(config, login_hint).await,
            Self::Unavailable(e) => Err(TokenError::Network(e.clone())),
        }
    }

    async fn refresh(
        &self,
        config: &OAuthConfig,
        refresh_token: &Secret,
    ) -> Result<OAuthGrant, TokenError> {
        match self {
            Self::Http(c) => c.refresh(config, refresh_token).await,
            Self::Unavailable(e) => Err(TokenError::Network(e.clone())),
        }
    }
}

pub type AppSecrets = Arc<AppSecretStore<SystemSecretStore>>;
pub type AppTokens = TokenBroker<AppOAuth, AppSecrets>;
/// Connecteurs de la synchronisation et de l'envoi : jeton OAuth renouvelé
/// avant chaque connexion.
pub type AppImap = FreshTokens<ImapConnector, AppOAuth, AppSecrets>;
pub type AppSmtp = FreshTokens<SmtpSender, AppOAuth, AppSecrets>;

/// Horloge murale en secondes depuis 1970.
fn wall_clock() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
pub type AppDiscoverSettings = DiscoverSettings<AppAutoconfig, AppSrvResolver, TcpPortProber>;

/// Import et export d'un carnet (ADR-012) : lecture et écriture des
/// fichiers, normalisation NFC, aperçu en attente de confirmation.
#[derive(Debug, Default)]
pub struct ContactImports {
    pub codec: ContactsIo,
    pub nfc: IcuNfc,
    pub pending: PendingImport,
}

/// État partagé par les commandes.
#[derive(Debug)]
pub struct AppState {
    pub accounts: SqliteAccountRepository,
    pub mail: MailServices,
    pub exceptions: SqliteCertificateExceptionStore,
    /// Carnet d'adresses, commun à tous les comptes (ADR-012).
    pub contacts: SqliteContactBook,
    pub imports: ContactImports,
    /// Synchronisation CardDAV du carnet (ADR-014).
    pub carddav: CardDavServices,
    pub local_data: FsAccountDataStore,
    pub test: AppTestConnection,
    pub discover: AppDiscoverSettings,
    pub secrets: AppSecrets,
    /// Jetons des comptes OAuth.
    pub tokens: Arc<AppTokens>,
    /// Changement de mot de passe et suppression d'un même compte, l'un
    /// après l'autre.
    pub account_locks: norelly_application::accounts::AccountLocks,
    /// Certificats refusés lors des derniers tests de connexion : seuls
    /// ceux-là peuvent faire l'objet d'une exception (l'interface ne peut
    /// pas faire accepter une empreinte arbitraire).
    pending_rejections: Mutex<Vec<CertificateRejection>>,
    /// Hôtes proposés par la découverte qui exigent une confirmation (SRV
    /// hors du domaine de l'adresse) : aucune connexion tant que
    /// l'utilisateur ne les a pas confirmés explicitement.
    unconfirmed_hosts: Mutex<Vec<String>>,
    /// Opérations rendues à l'interface par les dernières actions : seules
    /// celles-là peuvent être annulées (l'interface ne peut pas annuler une
    /// opération qu'elle n'a pas reçue, d'un autre compte par exemple).
    undoable_ops: Mutex<VecDeque<PendingOpId>>,
}

/// Crée le répertoire de données, accessible au seul utilisateur sous Unix
/// (la base contient adresses, serveurs et exceptions de certificats).
pub(crate) fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        // Répertoire existant créé avec d'autres droits : on les restreint.
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)
    }
}

/// Test de connexion : il réessaie aussitôt sur une nouvelle connexion,
/// donc abandonne tôt une poignée de main ou une ouverture bloquées
/// (délais courts). La synchronisation et l'envoi gardent les délais larges
/// par défaut.
fn connection_test(
    tls: &TlsClientFactory,
    exceptions: SqliteCertificateExceptionStore,
) -> AppTestConnection {
    let tls = tls
        .clone()
        .with_handshake_timeout(norelly_tls::SHORT_HANDSHAKE_TIMEOUT);
    TestConnection::new(
        ImapConnector::new(tls.clone()).with_timeouts(norelly_imap::Timeouts::CONNECTION_TEST),
        SmtpSender::new(tls).with_timeouts(norelly_smtp::Timeouts::CONNECTION_TEST),
        exceptions,
    )
}

/// Nombre maximal de refus mémorisés.
const MAX_PENDING_REJECTIONS: usize = 16;

/// Nombre maximal d'opérations annulables mémorisées : les deux dernières
/// actions au plus grand nombre de messages.
const MAX_UNDOABLE_OPS: usize = 2 * norelly_domain::actions::MAX_ACTION_TARGETS;

impl AppState {
    /// Assemble l'application à partir du répertoire de données
    /// (`cache_dir` : pièces jointes ouvertes, effaçable).
    pub fn open(data_dir: &Path, cache_dir: &Path) -> Result<Self, String> {
        create_private_dir(data_dir).map_err(|e| format!("{}: {e}", data_dir.display()))?;
        let open_dir = cache_dir.join("pieces-jointes");
        create_private_dir(&open_dir).map_err(|e| format!("{}: {e}", open_dir.display()))?;
        let db = Database::open(&data_dir.join("norelly.sqlite3")).map_err(|e| e.to_string())?;
        let tls = TlsClientFactory::platform();
        let exceptions = SqliteCertificateExceptionStore::new(db.clone());
        let autoconfig = match HttpAutoconfigClient::new(&tls) {
            Ok(c) => AppAutoconfig::Https(c),
            Err(e) => AppAutoconfig::Unavailable(e.to_string()),
        };
        let srv = match HickorySrvResolver::from_system() {
            Ok(r) => AppSrvResolver::System(Box::new(r)),
            Err(e) => AppSrvResolver::Unavailable(e),
        };
        let secrets = Arc::new(AppSecretStore::new(
            SystemSecretStore::open(),
            FileSecretStore::new(data_dir.join("mots-de-passe.json")),
        ));
        let oauth = match HttpOAuthClient::new(&tls, SystemBrowser) {
            Ok(c) => AppOAuth::Http(c),
            Err(e) => AppOAuth::Unavailable(e.to_string()),
        };
        let tokens = Arc::new(TokenBroker::new(
            oauth,
            Arc::clone(&secrets),
            Arc::new(wall_clock),
        ));
        let proxy = match RemoteImageProxy::new(&tls) {
            Ok(p) => Some(p),
            Err(e) => {
                tracing::warn!(error = %e, "proxy des images distantes indisponible");
                None
            }
        };
        let mail = MailServices::new(
            &db,
            FsBlobStore::new(data_dir.join("accounts")),
            FreshTokens {
                inner: ImapConnector::new(tls.clone()),
                broker: Arc::clone(&tokens),
            },
            FreshTokens {
                inner: SmtpSender::new(tls.clone()),
                broker: Arc::clone(&tokens),
            },
            proxy,
            open_dir,
        );
        let carddav = CardDavServices::new(
            match norelly_webdav::WebDavClient::new(&tls) {
                Ok(c) => AppCardDavClient::Https(c),
                Err(e) => AppCardDavClient::Unavailable(e.to_string()),
            },
            norelly_sqlite::carddav::SqliteCardDavStore::new(db.clone()),
        );
        Ok(Self {
            mail,
            carddav,
            contacts: SqliteContactBook::new(db.clone()),
            imports: ContactImports::default(),
            accounts: SqliteAccountRepository::new(db),
            test: connection_test(&tls, exceptions.clone()),
            exceptions,
            secrets,
            tokens,
            account_locks: norelly_application::accounts::AccountLocks::default(),
            local_data: FsAccountDataStore::new(data_dir.join("accounts")),
            discover: DiscoverSettings::new(autoconfig, srv, TcpPortProber::default()),
            pending_rejections: Mutex::new(Vec::new()),
            unconfirmed_hosts: Mutex::new(Vec::new()),
            undoable_ops: Mutex::new(VecDeque::new()),
        })
    }

    /// Fichier des mots de passe (repli sans trousseau).
    #[must_use]
    pub fn secrets_file(&self) -> &Path {
        self.secrets.file_path()
    }

    /// Mémorise un certificat refusé présenté à l'utilisateur.
    pub fn remember_rejection(&self, rejection: CertificateRejection) {
        let mut pending = self
            .pending_rejections
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        pending.retain(|r| r.host != rejection.host || r.fingerprint != rejection.fingerprint);
        pending.push(rejection);
        let excess = pending.len().saturating_sub(MAX_PENDING_REJECTIONS);
        pending.drain(..excess);
    }

    /// Mémorise un hôte découvert qui exige une confirmation.
    pub fn remember_unconfirmed(&self, host: &Host) {
        let mut hosts = self
            .unconfirmed_hosts
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let host = host.to_string();
        if !hosts.contains(&host) {
            hosts.push(host);
        }
        let excess = hosts.len().saturating_sub(MAX_PENDING_REJECTIONS);
        hosts.drain(..excess);
    }

    /// Premier hôte de `hosts` qui exige une confirmation absente de
    /// `confirmed`. Les hôtes confirmés cessent d'exiger une confirmation.
    pub fn check_confirmations<'h>(
        &self,
        hosts: &[&'h Host],
        confirmed: &[String],
    ) -> Option<&'h Host> {
        let mut pending = self
            .unconfirmed_hosts
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let missing = hosts
            .iter()
            .find(|h| {
                let name = h.to_string();
                pending.contains(&name) && !confirmed.contains(&name)
            })
            .copied();
        if missing.is_none() {
            pending.retain(|p| !hosts.iter().any(|h| &h.to_string() == p));
        }
        missing
    }

    /// Mémorise les opérations rendues à l'interface par une action.
    pub fn remember_undoable(&self, ops: &[PendingOpId]) {
        let mut known = self
            .undoable_ops
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        known.extend(ops.iter().copied());
        let excess = known.len().saturating_sub(MAX_UNDOABLE_OPS);
        known.drain(..excess);
    }

    /// Retire et renvoie, parmi `ops`, celles qui ont bien été rendues à
    /// l'interface (chacune une seule fois).
    pub fn take_undoable(&self, ops: &[PendingOpId]) -> Vec<PendingOpId> {
        let mut known = self
            .undoable_ops
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut taken = Vec::new();
        for op in ops {
            if let Some(i) = known.iter().position(|k| k == op) {
                known.remove(i);
                taken.push(*op);
            }
        }
        taken
    }

    /// Retire et renvoie le refus correspondant, s'il a bien été présenté.
    pub fn take_rejection(&self, host: &str, fingerprint: &str) -> Option<CertificateRejection> {
        let mut pending = self
            .pending_rejections
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let index = pending
            .iter()
            .position(|r| r.host.to_string() == host && r.fingerprint.to_string() == fingerprint)?;
        Some(pending.remove(index))
    }
}

#[cfg(test)]
mod tests {
    use norelly_domain::security::{CertificateFingerprint, CertificateIssue};
    use norelly_keyring::InMemorySecretStore;

    use super::*;

    fn state() -> AppState {
        let dir = std::env::temp_dir().join(format!(
            "norelly-state-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        AppState::open(&dir, &dir.join("cache")).unwrap_or_else(|e| panic!("{e}"))
    }

    fn host(raw: &str) -> Host {
        Host::parse(raw).unwrap_or_else(|_| panic!("hôte"))
    }

    /// Le test de connexion, qui réessaie, abandonne tôt une poignée de
    /// main ou une ouverture bloquées ; la synchronisation et l'envoi
    /// gardent des délais larges.
    #[test]
    fn connection_test_has_its_own_short_delays() {
        let s = state();
        let (imap, smtp) = s.test.connectors();
        assert_eq!(
            imap.tls().handshake_timeout(),
            norelly_tls::SHORT_HANDSHAKE_TIMEOUT
        );
        assert_eq!(
            smtp.tls().handshake_timeout(),
            norelly_tls::SHORT_HANDSHAKE_TIMEOUT
        );
        assert_eq!(imap.timeouts(), norelly_imap::Timeouts::CONNECTION_TEST);
        assert_eq!(smtp.timeouts(), norelly_smtp::Timeouts::CONNECTION_TEST);
        assert!(norelly_tls::SHORT_HANDSHAKE_TIMEOUT < norelly_tls::DEFAULT_HANDSHAKE_TIMEOUT);
        assert!(
            norelly_imap::Timeouts::CONNECTION_TEST.open < norelly_imap::Timeouts::default().open
        );
        // Jusqu'à la fin du chiffrement (connexion, échange en clair,
        // poignée de main), un essai tient dans le plafond du test avec de
        // la marge : c'est l'erreur de l'étape bloquée qui est rendue, pas
        // un délai dépassé générique (qui laisserait ajouter un compte OAuth
        // « envoi à vérifier »).
        let margin = std::time::Duration::from_secs(5);
        for (connect, open) in [
            (
                norelly_imap::Timeouts::CONNECTION_TEST.connect,
                norelly_imap::Timeouts::CONNECTION_TEST.open,
            ),
            (
                norelly_smtp::Timeouts::CONNECTION_TEST.connect,
                norelly_smtp::Timeouts::CONNECTION_TEST.open,
            ),
        ] {
            assert!(
                connect + open + norelly_tls::SHORT_HANDSHAKE_TIMEOUT + margin
                    <= norelly_domain::connection::CONNECTION_TEST_BUDGET
            );
        }
    }

    fn rejection(h: &str, byte: u8) -> CertificateRejection {
        CertificateRejection {
            host: host(h),
            fingerprint: CertificateFingerprint::from_bytes([byte; 32]),
            issue: CertificateIssue::UnknownIssuer,
            fingerprint_changed: false,
            name_mismatch: false,
        }
    }

    #[cfg(unix)]
    #[test]
    fn data_dir_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("norelly-private-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755));
        assert!(create_private_dir(&dir).is_ok());
        let mode = std::fs::metadata(&dir)
            .map(|m| m.permissions().mode() & 0o777)
            .unwrap_or(0);
        assert_eq!(mode, 0o700);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn without_keyring_the_file_needs_explicit_consent() {
        let dir = std::env::temp_dir().join(format!("norelly-secrets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let store: AppSecretStore<SystemSecretStore> = AppSecretStore::new(
            Err(SecretStoreError::Unavailable("pas de trousseau".into())),
            FileSecretStore::new(dir.join("mots-de-passe.json")),
        );
        let key = SecretKey::parse("account:1:imap").unwrap_or_else(|_| panic!("clé"));
        let secret = Secret::new("mdp".into());
        assert!(!store.usable().await);
        assert!(
            store.set(&key, &secret).await.is_err(),
            "rien sans consentement"
        );
        assert!(!dir.join("mots-de-passe.json").exists());
        assert!(store.consent_to_file().is_ok());
        assert!(store.usable().await);
        assert!(store.set(&key, &secret).await.is_ok());
        assert_eq!(
            store
                .get(&key)
                .await
                .ok()
                .flatten()
                .map(|s| s.expose().to_owned()),
            Some("mdp".into())
        );
        assert!(store.delete(&key).await.is_ok());
        assert!(store.get(&key).await.ok().flatten().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Trousseau du système espion : compte les lectures, qui peuvent
    /// afficher une invite de déverrouillage.
    #[derive(Debug, Default)]
    struct CountingKeyring {
        reads: std::sync::atomic::AtomicUsize,
        inner: InMemorySecretStore,
    }

    impl SecretStore for CountingKeyring {
        async fn get(&self, key: &SecretKey) -> Result<Option<Secret>, SecretStoreError> {
            self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.get(key).await
        }
        async fn set(&self, key: &SecretKey, secret: &Secret) -> Result<(), SecretStoreError> {
            self.inner.set(key, secret).await
        }
        async fn delete(&self, key: &SecretKey) -> Result<(), SecretStoreError> {
            self.inner.delete(key).await
        }
    }

    #[test]
    fn the_announced_destination_never_reads_the_keyring() {
        let dir = std::env::temp_dir().join(format!(
            "norelly-secrets-announce-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let file = || FileSecretStore::new(dir.join("mots-de-passe.json"));

        // Trousseau introuvable, fichier pas encore accepté : rien n'est
        // annoncé, le consentement viendra à l'enregistrement.
        let without: AppSecretStore<SystemSecretStore> = AppSecretStore::new(
            Err(SecretStoreError::Unavailable("pas de trousseau".into())),
            file(),
        );
        assert_eq!(
            without.announced_destination(),
            SecretDestination::Undecided
        );

        // Trousseau ouvert : annoncé, sans aucune lecture.
        let with = AppSecretStore::new(Ok(CountingKeyring::default()), file());
        assert_eq!(
            with.announced_destination(),
            SecretDestination::SystemKeyring
        );
        // Fichier accepté, trousseau ouvert : rien n'est promis.
        assert!(with.consent_to_file().is_ok());
        assert_eq!(with.announced_destination(), SecretDestination::Undecided);
        // Fichier accepté, aucun trousseau : le fichier est annoncé.
        assert_eq!(without.announced_destination(), SecretDestination::File);
        assert_eq!(
            with.system
                .as_ref()
                .map(|s| s.reads.load(std::sync::atomic::Ordering::SeqCst))
                .ok(),
            Some(0),
            "le trousseau a été lu pour l'annonce"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_keyring_write_removes_the_stale_file_copy() {
        let dir = std::env::temp_dir().join(format!(
            "norelly-secrets-stale-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("mots-de-passe.json");
        let key = SecretKey::parse("account:1:imap").unwrap_or_else(|_| panic!("clé"));
        // Compte ajouté sans trousseau : mot de passe dans le fichier.
        let without: AppSecretStore<SystemSecretStore> = AppSecretStore::new(
            Err(SecretStoreError::Unavailable("pas de trousseau".into())),
            FileSecretStore::new(path.clone()),
        );
        assert!(without.consent_to_file().is_ok());
        assert!(
            without
                .set(&key, &Secret::new("ancien-secret".into()))
                .await
                .is_ok()
        );
        // Trousseau revenu : le nouveau mot de passe y va, l'ancien ne
        // reste pas lisible dans le fichier.
        let with = AppSecretStore::new(
            Ok(InMemorySecretStore::default()),
            FileSecretStore::new(path.clone()),
        );
        assert!(with.set(&key, &Secret::new("nouveau".into())).await.is_ok());
        let raw = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            !raw.contains("ancien-secret"),
            "ancien mot de passe resté dans le fichier"
        );
        assert_eq!(
            with.get(&key)
                .await
                .ok()
                .flatten()
                .map(|s| s.expose().to_owned()),
            Some("nouveau".into())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_a_presented_rejection_can_be_taken_once() {
        let s = state();
        let r = rejection("imap.example.com", 1);
        let fp = r.fingerprint.to_string();
        // Empreinte jamais présentée : refusée.
        assert!(s.take_rejection("imap.example.com", &fp).is_none());
        s.remember_rejection(r.clone());
        // Autre hôte ou autre empreinte : refusés.
        assert!(s.take_rejection("smtp.example.com", &fp).is_none());
        let other = CertificateFingerprint::from_bytes([2; 32]).to_string();
        assert!(s.take_rejection("imap.example.com", &other).is_none());
        // Consommée une seule fois.
        assert_eq!(s.take_rejection("imap.example.com", &fp), Some(r));
        assert!(s.take_rejection("imap.example.com", &fp).is_none());
    }

    fn op(raw: i64) -> PendingOpId {
        PendingOpId::new(raw).unwrap_or_else(|| panic!("opération"))
    }

    #[test]
    fn only_returned_ops_can_be_undone_once() {
        let s = state();
        assert!(s.take_undoable(&[op(1)]).is_empty(), "jamais rendue");
        s.remember_undoable(&[op(1), op(2)]);
        assert_eq!(s.take_undoable(&[op(3), op(2), op(2)]), [op(2)]);
        assert!(s.take_undoable(&[op(2)]).is_empty(), "une seule fois");
        assert_eq!(s.take_undoable(&[op(1)]), [op(1)]);
        // Mémoire bornée : les plus anciennes sont oubliées.
        let many: Vec<PendingOpId> = (10..10 + MAX_UNDOABLE_OPS as i64 + 1).map(op).collect();
        s.remember_undoable(&many);
        assert!(s.take_undoable(&[op(10)]).is_empty());
        assert_eq!(s.take_undoable(&[op(11)]), [op(11)]);
    }

    #[test]
    fn pending_rejections_are_bounded() {
        let s = state();
        for i in 0..40u8 {
            s.remember_rejection(rejection("imap.example.com", i));
        }
        let oldest = CertificateFingerprint::from_bytes([0; 32]).to_string();
        let newest = CertificateFingerprint::from_bytes([39; 32]).to_string();
        assert!(s.take_rejection("imap.example.com", &oldest).is_none());
        assert!(s.take_rejection("imap.example.com", &newest).is_some());
    }

    #[test]
    fn unconfirmed_hosts_block_until_confirmed() {
        let s = state();
        let (bad, good) = (host("imap.attaquant.net"), host("smtp.example.com"));
        assert_eq!(s.check_confirmations(&[&bad, &good], &[]), None);
        s.remember_unconfirmed(&bad);
        assert_eq!(s.check_confirmations(&[&good, &bad], &[]), Some(&bad));
        let wrong = vec!["autre.net".to_owned()];
        assert_eq!(s.check_confirmations(&[&bad], &wrong), Some(&bad));
        let confirmed = vec!["imap.attaquant.net".to_owned()];
        assert_eq!(s.check_confirmations(&[&bad, &good], &confirmed), None);
        // Confirmé une fois : n'exige plus de confirmation.
        assert_eq!(s.check_confirmations(&[&bad], &[]), None);
    }
}
