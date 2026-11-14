//! Repli sans trousseau du système : secrets dans un fichier **non
//! chiffré**, lisible par l'utilisateur seul (droits 0600 sous Unix),
//! utilisé seulement après consentement explicite (choix du mainteneur,
//! 2026-11-02, en attendant mieux ; voir DECISIONS.md, ADR-002).
//!
//! Format : JSON `{ "clé": "secret" }`. Chaque écriture réécrit le fichier
//! de façon atomique (fichier temporaire créé avec les droits restreints,
//! puis renommage). Les copies en mémoire sont effacées après usage.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use norelly_domain::credentials::Secret;
use norelly_domain::ports::{SecretKey, SecretStore, SecretStoreError};
use zeroize::Zeroize;

/// Taille maximale du fichier relu.
const MAX_FILE_LEN: u64 = 1024 * 1024;

/// Secrets dans un fichier en clair (repli consenti).
#[derive(Debug, Clone)]
pub struct FileSecretStore {
    path: PathBuf,
    /// Sérialise les lectures-écritures du fichier.
    lock: Arc<Mutex<()>>,
    order: Arc<crate::order::WriteOrder>,
}

fn io_error(e: &std::io::Error) -> SecretStoreError {
    SecretStoreError::Backend(format!("fichier des mots de passe : {e}"))
}

/// Contenu du fichier ; effacé de la mémoire à la destruction.
struct Contents(BTreeMap<String, String>);

impl Drop for Contents {
    fn drop(&mut self) {
        for value in self.0.values_mut() {
            value.zeroize();
        }
    }
}

impl FileSecretStore {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Arc::new(Mutex::new(())),
            order: Arc::default(),
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Vrai si l'utilisateur a consenti (le fichier existe).
    #[must_use]
    pub fn consented(&self) -> bool {
        self.path.is_file()
    }

    /// Enregistre le consentement : crée le fichier vide (droits
    /// restreints) s'il n'existe pas.
    pub fn consent(&self) -> Result<(), SecretStoreError> {
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        if self.path.is_file() {
            return Ok(());
        }
        write_atomic(&self.path, &Contents(BTreeMap::new()))
    }

    fn read(&self) -> Result<Contents, SecretStoreError> {
        let mut file = match std::fs::File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(SecretStoreError::Unavailable(
                    "aucun trousseau et pas de consentement au fichier".into(),
                ));
            }
            Err(e) => return Err(io_error(&e)),
        };
        let len = file.metadata().map_err(|e| io_error(&e))?.len();
        if len > MAX_FILE_LEN {
            return Err(SecretStoreError::Backend(
                "fichier des mots de passe trop gros".into(),
            ));
        }
        let mut raw = String::new();
        std::io::Read::read_to_string(&mut file, &mut raw).map_err(|e| io_error(&e))?;
        let parsed = serde_json::from_str::<BTreeMap<String, String>>(&raw)
            .map_err(|_| SecretStoreError::Backend("fichier des mots de passe illisible".into()));
        raw.zeroize();
        parsed.map(Contents)
    }

    fn update(
        &self,
        f: impl FnOnce(&mut BTreeMap<String, String>),
    ) -> Result<(), SecretStoreError> {
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut contents = self.read()?;
        f(&mut contents.0);
        write_atomic(&self.path, &contents)
    }
}

/// Écrit `contents` dans `path` : fichier temporaire voisin aux droits
/// restreints, synchronisé, puis renommé.
fn write_atomic(path: &Path, contents: &Contents) -> Result<(), SecretStoreError> {
    let mut json =
        serde_json::to_string(&contents.0).map_err(|e| SecretStoreError::Backend(e.to_string()))?;
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    json.zeroize();
    result.map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io_error(&e)
    })
}

impl SecretStore for FileSecretStore {
    async fn get(&self, key: &SecretKey) -> Result<Option<Secret>, SecretStoreError> {
        let this = self.clone();
        let key = key.as_str().to_owned();
        tokio::task::spawn_blocking(move || {
            let _guard = this.lock.lock().unwrap_or_else(PoisonError::into_inner);
            let contents = this.read()?;
            Ok(contents.0.get(&key).map(|s| Secret::new(s.clone())))
        })
        .await
        .map_err(|_| SecretStoreError::Backend("tâche interrompue".into()))?
    }

    async fn set(&self, key: &SecretKey, secret: &Secret) -> Result<(), SecretStoreError> {
        let this = self.clone();
        let key = key.as_str().to_owned();
        let mut value = secret.expose().to_owned();
        let ticket = self.order.ticket();
        tokio::task::spawn_blocking(move || {
            let result = this.order.run(&key, ticket, || {
                this.update(|map| {
                    if let Some(mut old) = map.insert(key.clone(), value.clone()) {
                        old.zeroize();
                    }
                })
            });
            value.zeroize();
            result.map(|_| ())
        })
        .await
        .map_err(|_| SecretStoreError::Backend("tâche interrompue".into()))?
    }

    async fn delete(&self, key: &SecretKey) -> Result<(), SecretStoreError> {
        let this = self.clone();
        let key = key.as_str().to_owned();
        let ticket = self.order.ticket();
        tokio::task::spawn_blocking(move || {
            if !this.consented() {
                return Ok(());
            }
            this.order
                .run(&key, ticket, || {
                    this.update(|map| {
                        if let Some(mut old) = map.remove(&key) {
                            old.zeroize();
                        }
                    })
                })
                .map(|_| ())
        })
        .await
        .map_err(|_| SecretStoreError::Backend("tâche interrompue".into()))?
    }
}

#[cfg(test)]
mod tests {
    use norelly_test_support::contracts::secret_store_contract;

    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "norelly-file-secrets-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        dir
    }

    #[tokio::test]
    async fn file_store_passes_contract_after_consent() -> Result<(), SecretStoreError> {
        let d = dir("contract");
        let store = FileSecretStore::new(d.join("secrets.json"));
        assert!(!store.consented());
        let key = SecretKey::parse("account:1:imap")
            .map_err(|e| SecretStoreError::Backend(e.to_string()))?;
        // Sans consentement : rien n'est écrit.
        assert!(matches!(
            store.set(&key, &Secret::new("x".into())).await,
            Err(SecretStoreError::Unavailable(_))
        ));
        assert!(!store.path().exists());
        store.consent()?;
        assert!(store.consented());
        secret_store_contract(&store, "contrat-fichier:").await?;
        let _ = std::fs::remove_dir_all(&d);
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn file_is_readable_by_the_user_only() -> Result<(), SecretStoreError> {
        use std::os::unix::fs::PermissionsExt;
        let d = dir("mode");
        let store = FileSecretStore::new(d.join("secrets.json"));
        store.consent()?;
        let key = SecretKey::parse("account:1:imap")
            .map_err(|e| SecretStoreError::Backend(e.to_string()))?;
        store.set(&key, &Secret::new("mot de passe".into())).await?;
        let mode = std::fs::metadata(store.path())
            .map(|m| m.permissions().mode() & 0o777)
            .unwrap_or(0);
        assert_eq!(mode, 0o600);
        assert_eq!(
            store.get(&key).await?.map(|s| s.expose().to_owned()),
            Some("mot de passe".into())
        );
        let _ = std::fs::remove_dir_all(&d);
        Ok(())
    }
}
