//! Fichiers locaux par compte : `<racine>/<id du compte>/`.
//!
//! - [`FsAccountDataStore`] : purge des données d'un compte à sa
//!   suppression ;
//! - [`FsBlobStore`] : messages bruts compressés, `blobs/<aa>/<empreinte>.eml.zst`
//!   sous le répertoire du compte, nommés par l'empreinte SHA-256 de leur
//!   contenu (non compressé). Écriture atomique : fichier temporaire dans
//!   le même répertoire, synchronisé sur disque, puis renommé.
//!
//! À la lecture, la taille décompressée est bornée (protection contre une
//! bombe de décompression déposée sur le disque) et l'empreinte est
//! revérifiée.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use norelly_domain::account::AccountId;
use norelly_domain::ports::{AccountDataStore, BlobId, BlobStore, StoreError};

/// Taille maximale d'un contenu (identique à la limite de réponse IMAP).
pub const MAX_BLOB_SIZE: u64 = 64 * 1024 * 1024;
/// Niveau de compression zstd (compromis vitesse / taille).
const ZSTD_LEVEL: i32 = 3;

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

/// Messages bruts sur disque.
#[derive(Debug, Clone)]
pub struct FsBlobStore {
    accounts: FsAccountDataStore,
}

/// Compteur des fichiers temporaires (unicité dans le processus).
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn io(what: &str, e: &std::io::Error) -> StoreError {
    StoreError(format!("{what} : {e}"))
}

/// Empreinte SHA-256 en hexadécimal minuscule.
fn digest(bytes: &[u8]) -> Result<BlobId, StoreError> {
    let hash = ring::digest::digest(&ring::digest::SHA256, bytes);
    let hex: String = hash.as_ref().iter().map(|b| format!("{b:02x}")).collect();
    BlobId::parse(&hex).ok_or_else(|| StoreError("empreinte invalide".into()))
}

impl FsBlobStore {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            accounts: FsAccountDataStore::new(root),
        }
    }

    /// Chemin d'un contenu. `BlobId` ne contient que 64 chiffres
    /// hexadécimaux : aucun séparateur ni `..` possible.
    #[must_use]
    pub fn path_of(&self, account: AccountId, id: &BlobId) -> PathBuf {
        let hex = id.as_str();
        self.accounts
            .account_dir(account)
            .join("blobs")
            .join(hex.get(..2).unwrap_or("00"))
            .join(format!("{hex}.eml.zst"))
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let dir = path
        .parent()
        .ok_or_else(|| StoreError("chemin de contenu sans parent".into()))?;
    std::fs::create_dir_all(dir).map_err(|e| io("création du répertoire", &e))?;
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = dir.join(format!(".tmp-{}-{n}", std::process::id()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| io("création du fichier temporaire", &e))?;
        let mut encoder =
            zstd::Encoder::new(&mut file, ZSTD_LEVEL).map_err(|e| io("compression", &e))?;
        encoder
            .write_all(bytes)
            .map_err(|e| io("compression", &e))?;
        encoder.finish().map_err(|e| io("compression", &e))?;
        file.sync_all().map_err(|e| io("synchronisation", &e))?;
        drop(file);
        std::fs::rename(&temp, path).map_err(|e| io("renommage", &e))?;
        // Le renommage lui-même doit atteindre le disque avant que la base
        // ne référence le contenu (coupure de courant). Sans effet sous
        // Windows, où un répertoire ne s'ouvre pas ainsi.
        #[cfg(unix)]
        std::fs::File::open(dir)
            .and_then(|d| d.sync_all())
            .map_err(|e| io("synchronisation du répertoire", &e))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>, StoreError> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io("lecture", &e)),
    };
    let decoder = zstd::Decoder::new(file).map_err(|e| io("décompression", &e))?;
    let mut out = Vec::new();
    decoder
        .take(MAX_BLOB_SIZE + 1)
        .read_to_end(&mut out)
        .map_err(|e| io("décompression", &e))?;
    if out.len() as u64 > MAX_BLOB_SIZE {
        return Err(StoreError("contenu décompressé trop grand".into()));
    }
    Ok(Some(out))
}

impl BlobStore for FsBlobStore {
    async fn put(&self, account: AccountId, bytes: Vec<u8>) -> Result<BlobId, StoreError> {
        if bytes.len() as u64 > MAX_BLOB_SIZE {
            return Err(StoreError("contenu trop grand".into()));
        }
        let store = self.clone();
        tokio::task::spawn_blocking(move || {
            let id = digest(&bytes)?;
            let path = store.path_of(account, &id);
            // Contenu adressé par empreinte : déjà présent et intact, rien
            // à faire ; présent mais illisible ou corrompu, réécrit.
            let intact = path.exists()
                && matches!(read_bounded(&path), Ok(Some(existing)) if digest(&existing)? == id);
            if !intact {
                write_atomic(&path, &bytes)?;
            }
            Ok(id)
        })
        .await
        .map_err(|e| StoreError(format!("tâche interrompue : {e}")))?
    }

    async fn get(&self, account: AccountId, id: &BlobId) -> Result<Option<Vec<u8>>, StoreError> {
        let path = self.path_of(account, id);
        let id = id.clone();
        tokio::task::spawn_blocking(move || {
            let Some(bytes) = read_bounded(&path)? else {
                return Ok(None);
            };
            if digest(&bytes)? != id {
                return Err(StoreError(format!("contenu {id} corrompu")));
            }
            Ok(Some(bytes))
        })
        .await
        .map_err(|e| StoreError(format!("tâche interrompue : {e}")))?
    }

    async fn delete(&self, account: AccountId, id: &BlobId) -> Result<(), StoreError> {
        match tokio::fs::remove_file(self.path_of(account, id)).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io("suppression", &e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use norelly_test_support::mail_contracts::blob_store_contract;

    use super::*;

    fn id(raw: i64) -> AccountId {
        AccountId::new(raw).unwrap_or_else(|_| panic!("id"))
    }

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("norelly-blobs-{name}-{}", std::process::id()))
    }

    #[tokio::test]
    async fn purge_removes_only_the_account_directory() -> Result<(), StoreError> {
        let root = temp_root("purge");
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

    #[tokio::test]
    async fn blob_store_passes_contract() -> Result<(), StoreError> {
        let root = temp_root("contract");
        let result = blob_store_contract(&FsBlobStore::new(&root)).await;
        let _ = std::fs::remove_dir_all(&root);
        result
    }

    #[tokio::test]
    async fn blobs_are_compressed_and_sharded() -> Result<(), StoreError> {
        let root = temp_root("layout");
        let store = FsBlobStore::new(&root);
        let content = "Subject: test\r\n\r\n".repeat(1000).into_bytes();
        let blob = store.put(id(3), content.clone()).await?;
        let path = store.path_of(id(3), &blob);
        assert!(path.starts_with(root.join("3").join("blobs")));
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("zst"));
        let on_disk = std::fs::metadata(&path).map_err(|e| StoreError(e.to_string()))?;
        assert!(on_disk.len() < content.len() as u64 / 10, "compressé");
        // Aucun fichier temporaire laissé.
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let leftovers = std::fs::read_dir(&dir)
            .map_err(|e| StoreError(e.to_string()))?
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[tokio::test]
    async fn corrupted_blob_is_an_error() -> Result<(), StoreError> {
        let root = temp_root("corrupt");
        let store = FsBlobStore::new(&root);
        let blob = store.put(id(1), b"original".to_vec()).await?;
        let path = store.path_of(id(1), &blob);
        // Contenu remplacé (zstd valide, empreinte différente).
        let other = zstd::encode_all(&b"falsifie"[..], 3).map_err(|e| StoreError(e.to_string()))?;
        std::fs::write(&path, other).map_err(|e| StoreError(e.to_string()))?;
        assert!(store.get(id(1), &blob).await.is_err());
        // Fichier illisible.
        std::fs::write(&path, b"pas du zstd").map_err(|e| StoreError(e.to_string()))?;
        assert!(store.get(id(1), &blob).await.is_err());
        // Un nouvel enregistrement du même contenu répare le fichier.
        assert_eq!(store.put(id(1), b"original".to_vec()).await?, blob);
        assert_eq!(store.get(id(1), &blob).await?, Some(b"original".to_vec()));
        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[tokio::test]
    async fn decompression_bomb_is_bounded() -> Result<(), StoreError> {
        let root = temp_root("bomb");
        let store = FsBlobStore::new(&root);
        let blob = store.put(id(1), b"x".to_vec()).await?;
        let path = store.path_of(id(1), &blob);
        // 80 Mio de zéros compressés en quelques Kio.
        let mut encoder =
            zstd::Encoder::new(Vec::new(), 19).map_err(|e| StoreError(e.to_string()))?;
        let zeros = vec![0u8; 1024 * 1024];
        for _ in 0..80 {
            encoder
                .write_all(&zeros)
                .map_err(|e| StoreError(e.to_string()))?;
        }
        let bomb = encoder.finish().map_err(|e| StoreError(e.to_string()))?;
        std::fs::write(&path, bomb).map_err(|e| StoreError(e.to_string()))?;
        let result = store.get(id(1), &blob).await;
        assert!(
            matches!(result, Err(StoreError(ref m)) if m.contains("trop grand")),
            "{result:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[tokio::test]
    async fn oversized_content_is_refused() {
        let store = FsBlobStore::new(temp_root("big"));
        let too_big = vec![0u8; usize::try_from(MAX_BLOB_SIZE).unwrap_or(0) + 1];
        assert!(store.put(id(1), too_big).await.is_err());
    }
}
