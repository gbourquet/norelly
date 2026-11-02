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
    /// Hôtes proposés par la découverte qui exigent une confirmation (SRV
    /// hors du domaine de l'adresse) : aucune connexion tant que
    /// l'utilisateur ne les a pas confirmés explicitement.
    unconfirmed_hosts: Mutex<Vec<String>>,
}

/// Crée le répertoire de données, accessible au seul utilisateur sous Unix
/// (la base contient adresses, serveurs et exceptions de certificats).
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
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

/// Nombre maximal de refus mémorisés.
const MAX_PENDING_REJECTIONS: usize = 16;

impl AppState {
    /// Assemble l'application à partir du répertoire de données.
    pub fn open(data_dir: &Path) -> Result<Self, String> {
        create_private_dir(data_dir).map_err(|e| format!("{}: {e}", data_dir.display()))?;
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
            unconfirmed_hosts: Mutex::new(Vec::new()),
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

    use super::*;

    fn state() -> AppState {
        let dir = std::env::temp_dir().join(format!(
            "norelly-state-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        AppState::open(&dir).unwrap_or_else(|e| panic!("{e}"))
    }

    fn host(raw: &str) -> Host {
        Host::parse(raw).unwrap_or_else(|_| panic!("hôte"))
    }

    fn rejection(h: &str, byte: u8) -> CertificateRejection {
        CertificateRejection {
            host: host(h),
            fingerprint: CertificateFingerprint::from_bytes([byte; 32]),
            issue: CertificateIssue::UnknownIssuer,
            fingerprint_changed: false,
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
