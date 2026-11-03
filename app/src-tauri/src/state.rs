//! Point d'assemblage (ADR-008) : adaptateurs réels branchés sur les cas
//! d'usage.

use std::path::Path;
use std::sync::{Mutex, PoisonError};

use norelly_application::discover_settings::DiscoverSettings;
use norelly_application::test_connection::TestConnection;
use norelly_blobs::{FsAccountDataStore, FsBlobStore};
use norelly_discovery::{HickorySrvResolver, HttpAutoconfigClient, TcpPortProber};
use norelly_domain::credentials::Secret;
use norelly_domain::discovery::{SrvRecord, SrvService};
use norelly_domain::endpoint::Host;
use norelly_domain::ports::{
    DiscoveryError, SecretKey, SecretStore, SecretStoreError, SrvResolver,
};
use norelly_domain::security::CertificateRejection;
use norelly_imap::ImapConnector;
use norelly_keyring::{FileSecretStore, SystemSecretStore};
use norelly_smtp::SmtpSender;
use norelly_sqlite::{Database, SqliteAccountRepository, SqliteCertificateExceptionStore};
use norelly_tls::TlsClientFactory;

use crate::mail::MailServices;
use crate::remote_images::RemoteImageProxy;

/// Secrets des comptes : trousseau du système d'abord ; s'il ne répond
/// pas, fichier non chiffré lisible par l'utilisateur seul, **seulement**
/// après consentement explicite (choix du mainteneur, en attendant mieux :
/// voir DECISIONS.md, ADR-002). Lecture et suppression consultent les
/// deux (un trousseau installé après coup ne fait pas perdre les secrets
/// du fichier).
#[derive(Debug)]
pub struct AppSecretStore {
    system: Result<SystemSecretStore, SecretStoreError>,
    file: FileSecretStore,
}

impl AppSecretStore {
    #[must_use]
    pub fn new(system: Result<SystemSecretStore, SecretStoreError>, file: FileSecretStore) -> Self {
        Self { system, file }
    }

    /// Vrai si un secret peut être enregistré sans demander de
    /// consentement : trousseau joignable, ou fichier déjà consenti.
    pub async fn usable(&self) -> bool {
        if self.file.consented() {
            return true;
        }
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

impl SecretStore for AppSecretStore {
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
pub type AppDiscoverSettings = DiscoverSettings<AppAutoconfig, AppSrvResolver, TcpPortProber>;

/// État partagé par les commandes.
#[derive(Debug)]
pub struct AppState {
    pub accounts: SqliteAccountRepository,
    pub mail: MailServices,
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

/// Nombre maximal de refus mémorisés.
const MAX_PENDING_REJECTIONS: usize = 16;

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
        let secrets = AppSecretStore::new(
            SystemSecretStore::open(),
            FileSecretStore::new(data_dir.join("mots-de-passe.json")),
        );
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
            ImapConnector::new(tls.clone()),
            SmtpSender::new(tls.clone()),
            proxy,
            open_dir,
        );
        Ok(Self {
            mail,
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
        AppState::open(&dir, &dir.join("cache")).unwrap_or_else(|e| panic!("{e}"))
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

    #[tokio::test]
    async fn without_keyring_the_file_needs_explicit_consent() {
        let dir = std::env::temp_dir().join(format!("norelly-secrets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let store = AppSecretStore::new(
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
