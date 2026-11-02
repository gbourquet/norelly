//! Adaptateur de fichiers locaux par compte : `<racine>/<id du compte>/`.
//!
//! Phase 3 : purge des données d'un compte à sa suppression. Le stockage
//! des messages bruts (`.eml.zst`, `BlobStore`) arrive en phase 4 dans la
//! même arborescence.

use std::path::{Path, PathBuf};

use norelly_domain::account::AccountId;
use norelly_domain::ports::{AccountDataStore, StoreError};

/// Données locales des comptes, sous un répertoire racine.
#[derive(Debug, Clone)]
pub struct FsAccountDataStore {
    root: PathBuf,
}

impl FsAccountDataStore {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Répertoire d'un compte. L'identifiant est un entier strictement
    /// positif : aucun séparateur ni `..` possible.
    #[must_use]
    pub fn account_dir(&self, id: AccountId) -> PathBuf {
        self.root.join(id.get().to_string())
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl AccountDataStore for FsAccountDataStore {
    async fn purge(&self, id: AccountId) -> Result<(), StoreError> {
        match tokio::fs::remove_dir_all(self.account_dir(id)).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(StoreError(format!(
                "suppression des données locales du compte {id} : {e}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: i64) -> AccountId {
        AccountId::new(raw).unwrap_or_else(|_| panic!("id"))
    }

    #[tokio::test]
    async fn purge_removes_only_the_account_directory() -> Result<(), StoreError> {
        let root = std::env::temp_dir().join(format!("norelly-blobs-{}", std::process::id()));
        let store = FsAccountDataStore::new(&root);
        let (a, b) = (store.account_dir(id(1)), store.account_dir(id(2)));
        for dir in [&a, &b] {
            std::fs::create_dir_all(dir.join("sous-dossier"))
                .map_err(|e| StoreError(e.to_string()))?;
            std::fs::write(dir.join("sous-dossier/x.eml.zst"), b"x")
                .map_err(|e| StoreError(e.to_string()))?;
        }
        store.purge(id(1)).await?;
        assert!(!a.exists());
        assert!(b.join("sous-dossier/x.eml.zst").exists());
        // Idempotent.
        store.purge(id(1)).await?;
        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn account_dir_is_a_direct_child() {
        let store = FsAccountDataStore::new("/racine");
        assert_eq!(store.account_dir(id(42)), PathBuf::from("/racine/42"));
    }
}
