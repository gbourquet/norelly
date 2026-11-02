//! Point d'assemblage (ADR-008) : adaptateurs réels branchés sur les cas
//! d'usage.

use std::path::Path;
use std::sync::{Mutex, PoisonError};

use norelly_application::discover_settings::DiscoverSettings;
use norelly_application::test_connection::TestConnection;
use norelly_blobs::FsAccountDataStore;
use norelly_discovery::{HickorySrvResolver, HttpAutoconfigClient, TcpPortProber};
use norelly_domain::credentials::Secret;
use norelly_domain::discovery::{SrvRecord, SrvService};
use norelly_domain::endpoint::Host;
use norelly_domain::ports::{
    DiscoveryError, SecretKey, SecretStore, SecretStoreError, SrvResolver,
};
use norelly_domain::security::CertificateRejection;
use norelly_imap::ImapConnector;
use norelly_keyring::SystemSecretStore;
use norelly_smtp::SmtpSender;
use norelly_sqlite::{Database, SqliteAccountRepository, SqliteCertificateExceptionStore};
use norelly_tls::TlsClientFactory;

/// Trousseau du système, ou indisponible (l'application démarre quand
/// même ; l'ajout de compte signalera l'erreur).
#[derive(Debug)]
pub enum AppSecretStore {
    System(SystemSecretStore),
    Unavailable(SecretStoreError),
}

impl SecretStore for AppSecretStore {
    async fn get(&self, key: &SecretKey) -> Result<Option<Secret>, SecretStoreError> {
        match self {
            Self::System(s) => s.get(key).await,
            Self::Unavailable(e) => Err(e.clone()),
        }
    }
    async fn set(&self, key: &SecretKey, secret: &Secret) -> Result<(), SecretStoreError> {
        match self {
            Self::System(s) => s.set(key, secret).await,
            Self::Unavailable(e) => Err(e.clone()),
        }
    }
    async fn delete(&self, key: &SecretKey) -> Result<(), SecretStoreError> {
        match self {
            Self::System(s) => s.delete(key).await,
            Self::Unavailable(e) => Err(e.clone()),
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
pub type AppDiscoverSettings = DiscoverSettings<AppAutoconfig, AppSrvResolver, TcpPortProber>;

/// État partagé par les commandes.
#[derive(Debug)]
pub struct AppState {
    pub accounts: SqliteAccountRepository,
    pub exceptions: SqliteCertificateExceptionStore,
    pub secrets: AppSecretStore,
    pub local_data: FsAccountDataStore,
    pub test: AppTestConnection,
    pub discover: AppDiscoverSettings,
    /// Certificats refusés lors des derniers tests de connexion : seuls
    /// ceux-là peuvent faire l'objet d'une exception (l'interface ne peut
    /// pas faire accepter une empreinte arbitraire).
    pending_rejections: Mutex<Vec<CertificateRejection>>,
}

/// Nombre maximal de refus mémorisés.
const MAX_PENDING_REJECTIONS: usize = 16;

impl AppState {
    /// Assemble l'application à partir du répertoire de données.
    pub fn open(data_dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(data_dir).map_err(|e| format!("{}: {e}", data_dir.display()))?;
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
        let secrets = match SystemSecretStore::open() {
            Ok(s) => AppSecretStore::System(s),
            Err(e) => AppSecretStore::Unavailable(e),
        };
        Ok(Self {
            accounts: SqliteAccountRepository::new(db),
            test: TestConnection::new(
                ImapConnector::new(tls.clone()),
                SmtpSender::new(tls),
                exceptions.clone(),
            ),
            exceptions,
            secrets,
            local_data: FsAccountDataStore::new(data_dir.join("accounts")),
            discover: DiscoverSettings::new(autoconfig, srv, TcpPortProber::default()),
            pending_rejections: Mutex::new(Vec::new()),
        })
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
