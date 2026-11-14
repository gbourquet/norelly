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
        let file = OpenOptions::new().create(true).append(true).open(path)?;
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
    fn rotate(&mut self) -> io::Result<()> {
        self.file.flush()?;
        let mut archive = self.path.clone().into_os_string();
        archive.push(".1");
        std::fs::rename(&self.path, PathBuf::from(archive))?;
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        self.written = 0;
        Ok(())
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

/// Collecteur des journaux vers `dir/norelly.log` (niveau INFO au plus).
pub fn subscriber(dir: &Path) -> io::Result<impl tracing::Subscriber + Send + Sync> {
    std::fs::create_dir_all(dir)?;
    let log = BoundedLog::open(&dir.join(LOG_FILE_NAME), LOG_FILE_MAX_BYTES)?;
    Ok(tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(std::sync::Mutex::new(log))
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
