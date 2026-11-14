//! Journaux de l'application : fichier `norelly.log` du dossier des
//! données, borné en taille. Plein, il devient `norelly.log.1` (l'ancien
//! est remplacé) et un nouveau fichier commence : au plus deux fichiers.
//! Les journaux ne contiennent jamais de secret (règle du projet) ; le
//! niveau TRACE est supprimé à la compilation.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Taille maximale d'un fichier de journaux.
pub const LOG_FILE_MAX_BYTES: u64 = 2 * 1024 * 1024;

/// Nom du fichier de journaux, dans le dossier des données.
pub const LOG_FILE_NAME: &str = "norelly.log";

/// Fichier de journaux borné, avec une seule archive.
#[derive(Debug)]
pub struct BoundedLog {
    path: PathBuf,
    max_bytes: u64,
    file: File,
    written: u64,
}

impl BoundedLog {
    /// Ouvre (ou crée) le fichier `path`, en ajout.
    pub fn open(path: &Path, max_bytes: u64) -> io::Result<Self> {
        let file = open_private(path)?;
        let written = file.metadata()?.len();
        Ok(Self {
            path: path.to_path_buf(),
            max_bytes,
            file,
            written,
        })
    }
}

impl BoundedLog {
    /// Le fichier courant devient l'archive `.1` (l'ancienne est
    /// remplacée) ; un fichier vide lui succède.
    /// Archive impossible (droits, fichier verrouillé) : le fichier
    /// courant est vidé, les journaux récents restent écrits ; l'archive
    /// sera retentée à la prochaine borne.
    fn rotate(&mut self) -> io::Result<()> {
        self.file.flush()?;
        let mut archive = self.path.clone().into_os_string();
        archive.push(".1");
        if std::fs::rename(&self.path, PathBuf::from(archive)).is_ok()
            && let Ok(file) = open_private(&self.path)
        {
            self.file = file;
        } else {
            self.file.set_len(0)?;
        }
        self.written = 0;
        Ok(())
    }
}

/// Ouvre `path` en ajout, lisible par le seul utilisateur sous Unix (le
/// journal cite serveurs, dossiers et parfois des adresses).
fn open_private(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(0o600);
        let file = options.open(path)?;
        // Fichier existant créé avec d'autres droits : on les restreint.
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        options.open(path)
    }
}

impl Write for BoundedLog {
    /// Une entrée qui ferait dépasser la borne ouvre un nouveau fichier ;
    /// une entrée plus grande que la borne est tronquée.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let len = buf.len() as u64;
        if self.written > 0 && self.written.saturating_add(len) > self.max_bytes {
            self.rotate()?;
        }
        let room = usize::try_from(self.max_bytes.saturating_sub(self.written)).unwrap_or(0);
        let kept = buf.get(..buf.len().min(room)).unwrap_or_default();
        self.file.write_all(kept)?;
        self.written += kept.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// Journal partagé par les tâches qui journalisent.
#[derive(Debug, Clone)]
pub struct SharedLog(std::sync::Arc<std::sync::Mutex<BoundedLog>>);

impl SharedLog {
    #[must_use]
    pub fn new(log: BoundedLog) -> Self {
        Self(std::sync::Arc::new(std::sync::Mutex::new(log)))
    }
}

/// Écriture sous le verrou du journal.
#[derive(Debug)]
pub struct LogGuard<'a>(std::sync::MutexGuard<'a, BoundedLog>);

impl Write for LogGuard<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SharedLog {
    type Writer = LogGuard<'a>;
    fn make_writer(&'a self) -> Self::Writer {
        // Une tâche qui a paniqué en écrivant n'arrête pas le journal.
        LogGuard(
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
}

/// Collecteur des journaux vers `dir/norelly.log` (niveau INFO au plus).
pub fn subscriber(dir: &Path) -> io::Result<impl tracing::Subscriber + Send + Sync> {
    crate::state::create_private_dir(dir)?;
    let log = BoundedLog::open(&dir.join(LOG_FILE_NAME), LOG_FILE_MAX_BYTES)?;
    Ok(tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(SharedLog::new(log))
        .finish())
}

/// Installe le collecteur pour toute l'application ; sans fichier
/// utilisable (droits, disque plein), l'application continue sans
/// journaux.
pub fn init(dir: &Path) {
    if let Ok(subscriber) = subscriber(dir) {
        let _ = tracing::subscriber::set_global_default(subscriber);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o777)
            .unwrap_or(0)
    }

    #[cfg(unix)]
    #[test]
    fn the_log_and_its_folder_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = dir("droits");
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755));
        let path = dir.join(LOG_FILE_NAME);
        std::fs::write(&path, b"ancien\n").unwrap_or_else(|e| panic!("{e}"));
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644));
        let subscriber = subscriber(&dir).unwrap_or_else(|e| panic!("{e}"));
        drop(subscriber);
        assert_eq!(mode(&dir), 0o700, "dossier privé");
        assert_eq!(mode(&path), 0o600, "fichier existant rendu privé");
        let mut log = BoundedLog::open(&path, 20).unwrap_or_else(|e| panic!("{e}"));
        for _ in 0..5 {
            log.write_all(b"ligne de journal\n")
                .unwrap_or_else(|e| panic!("{e}"));
        }
        assert_eq!(mode(&path), 0o600, "nouveau fichier privé");
        assert_eq!(
            mode(&dir.join(format!("{LOG_FILE_NAME}.1"))),
            0o600,
            "archive privée"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_rotation_keeps_the_log_bounded_and_recent() {
        let dir = dir("rotation-ratee");
        let path = dir.join(LOG_FILE_NAME);
        // L'archive ne peut pas être remplacée (dossier non vide).
        let blocked = dir.join(format!("{LOG_FILE_NAME}.1"));
        std::fs::create_dir_all(blocked.join("x")).unwrap_or_else(|e| panic!("{e}"));
        let mut log = BoundedLog::open(&path, 100).unwrap_or_else(|e| panic!("{e}"));
        for n in 0..30 {
            log.write_all(format!("ligne {n:02}\n").as_bytes())
                .unwrap_or_else(|e| panic!("écriture refusée : {e}"));
        }
        assert!(size(&path) <= 100, "borné : {}", size(&path));
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            text.contains("ligne 29"),
            "journaux récents gardés : {text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_poisoned_lock_does_not_stop_the_log() {
        use tracing_subscriber::fmt::MakeWriter;
        let dir = dir("verrou");
        let path = dir.join(LOG_FILE_NAME);
        let shared =
            SharedLog::new(BoundedLog::open(&path, 1_000).unwrap_or_else(|e| panic!("{e}")));
        let inner = shared.0.clone();
        let _ = std::thread::spawn(move || {
            let _guard = inner.lock();
            panic!("panique pendant une écriture");
        })
        .join();
        let written = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            shared.make_writer().write_all(b"toujours journalise\n")
        }));
        assert!(
            matches!(written, Ok(Ok(()))),
            "écriture après un verrou empoisonné"
        );
        assert!(
            std::fs::read_to_string(&path)
                .unwrap_or_default()
                .contains("toujours journalise")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Un refus d'authentification réel (acteur de compte, faux serveur
    /// IMAP) est journalisé sans le mot de passe ni l'identifiant.
    #[test]
    fn credentials_never_reach_the_log() {
        use std::sync::Arc;

        use norelly_application::account_actor::{ActorConfig, spawn};
        use norelly_application::sync::SyncEvent;
        use norelly_domain::account::AccountId;
        use norelly_domain::auth::AuthError;
        use norelly_domain::connection::ConnectionError;
        use norelly_domain::credentials::{Credentials, Secret, Username};
        use norelly_domain::endpoint::{Host, Security, ServerEndpoint};
        use norelly_domain::ports::ConnectRequest;
        use norelly_domain::security::PlaintextPolicy;
        use norelly_test_support::fake_imap::FakeImap;
        use norelly_test_support::fakes::FakeMailServers;
        use norelly_test_support::mail_contracts::{InMemoryBlobStore, InMemoryMailStore};

        const PASSWORD: &str = "MARQUEUR-MOT-DE-PASSE-7f3a";
        const LOGIN: &str = "marqueur-identifiant-91c2";
        let dir = dir("secrets");
        let subscriber = subscriber(&dir).unwrap_or_else(|e| panic!("{e}"));
        tracing::subscriber::with_default(subscriber, || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap_or_else(|e| panic!("{e}"));
            runtime.block_on(async {
                let imap = FakeImap::new();
                imap.set_connect_error(Some(ConnectionError::Authentication(
                    AuthError::Rejected {
                        message: "NO [AUTHENTICATIONFAILED] Invalid credentials".into(),
                    },
                )));
                let request = ConnectRequest {
                    endpoint: ServerEndpoint::new(
                        Host::parse("imap.example.com").unwrap_or_else(|_| panic!("hôte")),
                        993,
                        Security::Tls,
                    )
                    .unwrap_or_else(|_| panic!("serveur")),
                    credentials: Credentials::new(
                        Username::parse(LOGIN).unwrap_or_else(|_| panic!("identifiant")),
                        Secret::new(PASSWORD.into()),
                    )
                    .unwrap_or_else(|_| panic!("identifiants")),
                    certificate_exceptions: vec![],
                    plaintext: PlaintextPolicy::deny_all(),
                };
                let handle = spawn(
                    AccountId::new(1).unwrap_or_else(|_| panic!("compte")),
                    request,
                    Arc::new(FakeMailServers::with_imap(imap)),
                    Arc::new(InMemoryMailStore::default()),
                    Arc::new(InMemoryBlobStore::default()),
                    Arc::new(norelly_render::search_text::MimeTextExtractor),
                    ActorConfig::default(),
                );
                let mut events = handle.subscribe();
                let offline = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    loop {
                        if let Ok(SyncEvent::Offline { .. }) = events.recv().await {
                            return;
                        }
                    }
                })
                .await;
                assert!(offline.is_ok(), "refus non signalé");
                handle.shutdown().await;
            });
        });
        let text = std::fs::read_to_string(dir.join(LOG_FILE_NAME)).unwrap_or_default();
        assert!(
            text.contains("connexion perdue"),
            "refus journalisé : {text}"
        );
        assert!(
            !text.contains(PASSWORD),
            "mot de passe dans le journal : {text}"
        );
        assert!(
            !text.contains(LOGIN),
            "identifiant dans le journal : {text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn warnings_reach_the_log_file_but_not_debug_details() {
        let dir = dir("collecte");
        let subscriber = subscriber(&dir).unwrap_or_else(|e| panic!("{e}"));
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(dossier = "INBOX", "boîte ignorée");
            tracing::debug!("détail de mise au point");
        });
        let text = std::fs::read_to_string(dir.join(LOG_FILE_NAME)).unwrap_or_default();
        assert!(text.contains("boîte ignorée"), "{text}");
        assert!(text.contains("WARN"), "{text}");
        assert!(!text.contains("détail de mise au point"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("norelly-logs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        dir
    }

    fn size(path: &Path) -> u64 {
        std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
    }

    #[test]
    fn a_full_log_rotates_and_never_exceeds_two_files() {
        let dir = dir("rotation");
        let path = dir.join(LOG_FILE_NAME);
        let archive = dir.join(format!("{LOG_FILE_NAME}.1"));
        let mut log = BoundedLog::open(&path, 100).unwrap_or_else(|e| panic!("{e}"));
        for _ in 0..30 {
            log.write_all(b"ligne de journal\n")
                .unwrap_or_else(|e| panic!("{e}"));
        }
        assert!(
            size(&path) <= 100,
            "fichier courant borné : {}",
            size(&path)
        );
        assert!(size(&archive) <= 100, "archive bornée : {}", size(&archive));
        assert!(size(&archive) > 0, "archive créée");
        let entries = std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0);
        assert_eq!(entries, 2, "deux fichiers au plus");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_existing_log_is_continued() {
        let dir = dir("reprise");
        let path = dir.join(LOG_FILE_NAME);
        std::fs::write(&path, b"avant\n").unwrap_or_else(|e| panic!("{e}"));
        let mut log = BoundedLog::open(&path, 1_000).unwrap_or_else(|e| panic!("{e}"));
        log.write_all(b"apres\n").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap_or_default(),
            "avant\napres\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
