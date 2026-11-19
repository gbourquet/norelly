//! Photos de contact réencodées ([`ContactPhotoStore`], ADR-012) :
//! `<racine>/<aa>/<empreinte>.jpg`, nommées par l'empreinte SHA-256 de leur
//! contenu, non compressées (le JPEG l'est déjà). Écriture atomique comme
//! les messages ; à la lecture, la taille est bornée et l'empreinte
//! revérifiée.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use norelly_domain::contact_photo::MAX_PHOTO_BYTES;
use norelly_domain::ports::{BlobId, ContactPhotoStore, StoreError};

use crate::{digest, io, write_file_atomic};

/// Contenu du fichier `path`, au plus [`MAX_PHOTO_BYTES`] octets ; `None`
/// s'il est absent.
fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>, StoreError> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io("lecture", &e)),
    };
    let mut out = Vec::new();
    file.take(MAX_PHOTO_BYTES as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| io("lecture", &e))?;
    if out.len() > MAX_PHOTO_BYTES {
        return Err(StoreError("photo trop grande".into()));
    }
    Ok(Some(out))
}

/// Identifiant d'un fichier de photo (`<empreinte>.jpg`) ; `None` pour
/// tout autre nom.
fn photo_of_file(name: &str) -> Option<BlobId> {
    BlobId::parse(name.strip_suffix(".jpg")?)
}

fn task_error(e: &tokio::task::JoinError) -> StoreError {
    StoreError(format!("tâche interrompue : {e}"))
}

/// Photos de contact sur disque, sous un répertoire racine.
#[derive(Debug, Clone)]
pub struct FsContactPhotoStore {
    root: PathBuf,
}

impl FsContactPhotoStore {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Chemin d'une photo. `BlobId` ne contient que 64 chiffres
    /// hexadécimaux : aucun séparateur ni `..` possible.
    #[must_use]
    pub fn path_of(&self, id: &BlobId) -> PathBuf {
        let hex = id.as_str();
        self.root
            .join(hex.get(..2).unwrap_or("00"))
            .join(format!("{hex}.jpg"))
    }
}

impl ContactPhotoStore for FsContactPhotoStore {
    async fn put(&self, bytes: Vec<u8>) -> Result<BlobId, StoreError> {
        if bytes.len() > MAX_PHOTO_BYTES {
            return Err(StoreError("photo trop grande".into()));
        }
        let store = self.clone();
        tokio::task::spawn_blocking(move || {
            let id = digest(&bytes)?;
            let path = store.path_of(&id);
            // Déjà présente et intacte : rien à faire ; présente mais
            // illisible ou altérée : réécrite.
            let intact =
                matches!(read_bounded(&path), Ok(Some(existing)) if digest(&existing)? == id);
            if intact {
                // Récente de nouveau : le ménage d'une autre instance ne la
                // supprime pas avant qu'un contact la porte.
                std::fs::File::options()
                    .append(true)
                    .open(&path)
                    .and_then(|f| f.set_modified(SystemTime::now()))
                    .map_err(|e| io("horodatage", &e))?;
            } else {
                write_file_atomic(&path, |file| {
                    file.write_all(&bytes).map_err(|e| io("écriture", &e))
                })?;
            }
            Ok(id)
        })
        .await
        .map_err(|e| task_error(&e))?
    }

    async fn get(&self, id: &BlobId) -> Result<Option<Vec<u8>>, StoreError> {
        let path = self.path_of(id);
        let id = id.clone();
        tokio::task::spawn_blocking(move || {
            let Some(bytes) = read_bounded(&path)? else {
                return Ok(None);
            };
            if digest(&bytes)? != id {
                return Err(StoreError(format!("photo {id} altérée")));
            }
            Ok(Some(bytes))
        })
        .await
        .map_err(|e| task_error(&e))?
    }

    async fn retain(
        &self,
        keep: &BTreeSet<BlobId>,
        min_age: Duration,
    ) -> Result<usize, StoreError> {
        let root = self.root.clone();
        let keep = keep.clone();
        tokio::task::spawn_blocking(move || {
            let shards = match std::fs::read_dir(&root) {
                Ok(shards) => shards,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
                Err(e) => return Err(io("lecture du répertoire", &e)),
            };
            let mut removed = 0;
            for shard in shards {
                let shard = shard.map_err(|e| io("lecture du répertoire", &e))?;
                // Répertoires de deux caractères seulement, sans suivre de
                // lien symbolique.
                let is_shard =
                    shard.file_type().is_ok_and(|t| t.is_dir()) && shard.file_name().len() == 2;
                if !is_shard {
                    continue;
                }
                let files =
                    std::fs::read_dir(shard.path()).map_err(|e| io("lecture du répertoire", &e))?;
                for file in files {
                    let file = file.map_err(|e| io("lecture du répertoire", &e))?;
                    let Some(id) = file.file_name().to_str().and_then(photo_of_file) else {
                        continue;
                    };
                    // Lien symbolique jamais suivi ; âge inconnu : gardée.
                    let old_enough = file
                        .metadata()
                        .ok()
                        .filter(std::fs::Metadata::is_file)
                        .and_then(|m| m.modified().ok())
                        .and_then(|at| SystemTime::now().duration_since(at).ok())
                        .is_some_and(|age| age >= min_age);
                    if !keep.contains(&id)
                        && file.file_type().is_ok_and(|t| t.is_file())
                        && old_enough
                    {
                        match std::fs::remove_file(file.path()) {
                            Ok(()) => removed += 1,
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                            Err(e) => return Err(io("suppression", &e)),
                        }
                    }
                }
            }
            Ok(removed)
        })
        .await
        .map_err(|e| task_error(&e))?
    }
}

#[cfg(test)]
mod tests {
    use norelly_test_support::contact_photo_contracts::contact_photo_store_contract;

    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("norelly-photos-{name}-{}", std::process::id()))
    }

    #[tokio::test]
    async fn photo_store_passes_contract() -> Result<(), StoreError> {
        let root = temp_root("contract");
        let result = contact_photo_store_contract(&FsContactPhotoStore::new(&root)).await;
        let _ = std::fs::remove_dir_all(&root);
        result
    }

    #[tokio::test]
    async fn photos_are_sharded_jpeg_files() -> Result<(), StoreError> {
        let root = temp_root("layout");
        let store = FsContactPhotoStore::new(&root);
        let photo = b"\xFF\xD8\xFF photo".to_vec();
        let id = store.put(photo.clone()).await?;
        let path = store.path_of(&id);
        assert!(path.starts_with(root.join(&id.as_str()[..2])));
        assert_eq!(std::fs::read(&path).ok(), Some(photo));
        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[tokio::test]
    async fn a_tampered_or_oversized_file_is_an_error() -> Result<(), StoreError> {
        let root = temp_root("tampered");
        let store = FsContactPhotoStore::new(&root);
        let id = store.put(b"\xFF\xD8\xFF originale".to_vec()).await?;
        let path = store.path_of(&id);
        std::fs::write(&path, b"<svg onload=alert(1)>").map_err(|e| StoreError(e.to_string()))?;
        assert!(store.get(&id).await.is_err());
        let huge = vec![0u8; norelly_domain::contact_photo::MAX_PHOTO_BYTES + 1];
        std::fs::write(&path, huge).map_err(|e| StoreError(e.to_string()))?;
        assert!(store.get(&id).await.is_err());
        // Un nouvel enregistrement du même contenu répare le fichier.
        assert_eq!(store.put(b"\xFF\xD8\xFF originale".to_vec()).await?, id);
        assert!(store.get(&id).await?.is_some());
        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[tokio::test]
    async fn the_cleanup_spares_recent_photos() -> Result<(), StoreError> {
        let root = temp_root("recent");
        let store = FsContactPhotoStore::new(&root);
        let id = store.put(b"\xFF\xD8\xFF recente".to_vec()).await?;
        let hour = Duration::from_secs(3600);
        assert_eq!(store.retain(&BTreeSet::new(), hour).await?, 0);
        assert!(store.path_of(&id).exists());
        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_cleanup_never_follows_links() -> Result<(), StoreError> {
        let root = temp_root("links");
        let outside = temp_root("links-outside");
        let err = |e: std::io::Error| StoreError(e.to_string());
        std::fs::create_dir_all(outside.join("cd")).map_err(err)?;
        let victim_id = "cd".repeat(32);
        let victim = outside.join("cd").join(format!("{victim_id}.jpg"));
        std::fs::write(&victim, b"x").map_err(err)?;
        std::fs::create_dir_all(root.join("ab")).map_err(err)?;
        // Répartiteur qui est un lien vers un autre dossier, et photo qui est
        // un lien vers un fichier du dehors.
        std::os::unix::fs::symlink(outside.join("cd"), root.join("cd")).map_err(err)?;
        let linked = root.join("ab").join(format!("{}.jpg", "ab".repeat(32)));
        std::os::unix::fs::symlink(&victim, &linked).map_err(err)?;
        let store = FsContactPhotoStore::new(&root);
        assert_eq!(store.retain(&BTreeSet::new(), Duration::ZERO).await?, 0);
        assert!(victim.exists());
        assert!(linked.symlink_metadata().is_ok());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
        Ok(())
    }

    #[tokio::test]
    async fn the_cleanup_leaves_foreign_files_alone() -> Result<(), StoreError> {
        let root = temp_root("foreign");
        let store = FsContactPhotoStore::new(&root);
        let id = store.put(b"\xFF\xD8\xFF garde".to_vec()).await?;
        let gone = store.put(b"\xFF\xD8\xFF partie".to_vec()).await?;
        let shard = root.join("ab");
        std::fs::create_dir_all(&shard).map_err(|e| StoreError(e.to_string()))?;
        let foreign = shard.join("notes.txt");
        std::fs::write(&foreign, b"x").map_err(|e| StoreError(e.to_string()))?;
        assert_eq!(
            store
                .retain(&BTreeSet::from([id.clone()]), Duration::ZERO)
                .await?,
            1
        );
        assert!(store.path_of(&id).exists());
        assert!(!store.path_of(&gone).exists());
        assert!(foreign.exists());
        // Racine absente : rien à faire.
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(store.retain(&BTreeSet::new(), Duration::ZERO).await?, 0);
        Ok(())
    }
}
