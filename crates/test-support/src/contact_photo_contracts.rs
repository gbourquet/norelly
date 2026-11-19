//! Suites de contrat des ports des photos de contact ([`PhotoCodec`],
//! [`ContactPhotoStore`]), jouées contre les faux ([`FakePhotoCodec`],
//! [`InMemoryPhotoStore`]) et contre les adaptateurs `contacts-io` et
//! `blobs`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use norelly_domain::contact_photo::{MAX_PHOTO_BYTES, PhotoFormat};
use norelly_domain::ports::{BlobId, ContactPhotoStore, PhotoCodec, PhotoCodecError, StoreError};

/// Empreinte SHA-256 de `bytes`, identifiant d'une photo.
#[must_use]
pub fn photo_digest(bytes: &[u8]) -> BlobId {
    let hash = ring::digest::digest(&ring::digest::SHA256, bytes);
    let hex: String = hash.as_ref().iter().map(|b| format!("{b:02x}")).collect();
    BlobId::parse(&hex).unwrap_or_else(|| panic!("empreinte invalide"))
}

/// Faux réencodage : un contenu dont la signature est celle du format
/// annoncé et qui se termine par « contenu » devient une « vignette » JPEG factice (signature JPEG, puis le
/// format et la taille d'origine) ; tout autre contenu est illisible.
#[derive(Debug, Default, Clone, Copy)]
pub struct FakePhotoCodec;

impl PhotoCodec for FakePhotoCodec {
    /// Dimensions factices : la longueur du contenu sur un pixel de haut.
    fn dimensions(&self, bytes: &[u8], format: PhotoFormat) -> Result<(u32, u32), PhotoCodecError> {
        if PhotoFormat::sniff(bytes) != Some(format) || !bytes.ends_with(b"contenu") {
            return Err(PhotoCodecError::Undecodable);
        }
        Ok((u32::try_from(bytes.len()).unwrap_or(u32::MAX), 1))
    }

    fn thumbnail(&self, bytes: &[u8], format: PhotoFormat) -> Result<Vec<u8>, PhotoCodecError> {
        if PhotoFormat::sniff(bytes) != Some(format) || !bytes.ends_with(b"contenu") {
            return Err(PhotoCodecError::Undecodable);
        }
        let mut out = vec![0xFF, 0xD8, 0xFF, 0xE0];
        out.extend_from_slice(format!("vignette {format:?} {}", bytes.len()).as_bytes());
        Ok(out)
    }
}

/// Photo d'exemple du contrat : contenu, format reconnu, dimensions.
pub type PhotoSample = (Vec<u8>, PhotoFormat, (u32, u32));

/// Contrat du réencodage : chaque photo de `samples` annonce ses
/// dimensions et devient un JPEG d'au plus [`MAX_PHOTO_BYTES`] octets ; un
/// contenu tronqué, ou lu avec le décodeur d'un autre format, est
/// illisible.
pub fn photo_codec_contract(codec: &impl PhotoCodec, samples: &[PhotoSample]) {
    for (bytes, format, size) in samples {
        assert_eq!(codec.dimensions(bytes, *format), Ok(*size), "{format:?}");
        let out = codec
            .thumbnail(bytes, *format)
            .unwrap_or_else(|e| panic!("{format:?} : {e}"));
        assert_eq!(
            PhotoFormat::sniff(&out),
            Some(PhotoFormat::Jpeg),
            "{format:?}"
        );
        assert!(out.len() <= MAX_PHOTO_BYTES, "{format:?}");
    }
    assert_eq!(
        codec.thumbnail(b"\x89PNG\r\n\x1a\n tronque", PhotoFormat::Png),
        Err(PhotoCodecError::Undecodable)
    );
    assert_eq!(
        codec.dimensions(b"\x89PNG\r\n\x1a\n tronque", PhotoFormat::Png),
        Err(PhotoCodecError::Undecodable)
    );
    if let Some((bytes, format, _)) = samples.iter().find(|(_, f, _)| *f != PhotoFormat::Webp) {
        let other = if *format == PhotoFormat::Png {
            PhotoFormat::Webp
        } else {
            PhotoFormat::Png
        };
        assert_eq!(
            codec.thumbnail(bytes, other),
            Err(PhotoCodecError::Undecodable),
            "{format:?} lu comme {other:?}"
        );
    }
}

/// Magasin des photos en mémoire.
#[derive(Debug, Default)]
pub struct InMemoryPhotoStore {
    photos: Mutex<BTreeMap<BlobId, (Vec<u8>, Instant)>>,
}

impl InMemoryPhotoStore {
    fn lock(&self) -> MutexGuard<'_, BTreeMap<BlobId, (Vec<u8>, Instant)>> {
        self.photos.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Vieillit toutes les photos de `by` (ménage, [`ContactPhotoStore::retain`]).
    pub fn backdate(&self, by: Duration) {
        for (_, at) in self.lock().values_mut() {
            *at = at.checked_sub(by).unwrap_or(*at);
        }
    }

    /// Photos enregistrées.
    #[must_use]
    pub fn ids(&self) -> BTreeSet<BlobId> {
        self.lock().keys().cloned().collect()
    }
}

impl ContactPhotoStore for InMemoryPhotoStore {
    async fn put(&self, bytes: Vec<u8>) -> Result<BlobId, StoreError> {
        if bytes.len() > MAX_PHOTO_BYTES {
            return Err(StoreError("photo trop grande".into()));
        }
        let id = photo_digest(&bytes);
        self.lock()
            .entry(id.clone())
            .or_insert_with(|| (bytes, Instant::now()));
        Ok(id)
    }

    async fn get(&self, id: &BlobId) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self.lock().get(id).map(|(bytes, _)| bytes.clone()))
    }

    async fn retain(
        &self,
        keep: &BTreeSet<BlobId>,
        min_age: Duration,
    ) -> Result<usize, StoreError> {
        let mut photos = self.lock();
        let before = photos.len();
        photos.retain(|id, (_, at)| keep.contains(id) || at.elapsed() < min_age);
        Ok(before - photos.len())
    }
}

/// Contrat du magasin des photos (vide au départ) : adressé par
/// l'empreinte SHA-256 du contenu, idempotent ; photo inconnue absente ;
/// taille bornée ; ménage des photos qui ne sont plus portées.
pub async fn contact_photo_store_contract(
    store: &impl ContactPhotoStore,
) -> Result<(), StoreError> {
    let a = b"\xFF\xD8\xFF photo a".to_vec();
    let b = b"\xFF\xD8\xFF photo b".to_vec();
    let id_a = store.put(a.clone()).await?;
    assert_eq!(id_a, photo_digest(&a));
    assert_eq!(store.put(a.clone()).await?, id_a, "idempotent");
    let id_b = store.put(b.clone()).await?;
    assert_ne!(id_a, id_b);
    assert_eq!(store.get(&id_a).await?, Some(a.clone()));
    assert_eq!(store.get(&photo_digest(b"inconnue")).await?, None);

    let mut big = a.clone();
    big.resize(MAX_PHOTO_BYTES, 0);
    let id_big = store.put(big.clone()).await?;
    assert_eq!(store.get(&id_big).await?, Some(big.clone()));
    big.push(0);
    assert!(store.put(big).await.is_err(), "photo trop grande");

    let keep = BTreeSet::from([id_a.clone()]);
    // Trop récentes pour le ménage : gardées.
    let hour = Duration::from_secs(3600);
    assert_eq!(store.retain(&keep, hour).await?, 0);
    assert_eq!(store.get(&id_b).await?, Some(b.clone()));
    assert_eq!(store.retain(&keep, Duration::ZERO).await?, 2);
    assert_eq!(store.get(&id_a).await?, Some(a));
    assert_eq!(store.get(&id_b).await?, None);
    assert_eq!(store.get(&id_big).await?, None);
    assert_eq!(store.retain(&keep, Duration::ZERO).await?, 0);
    assert_eq!(store.retain(&BTreeSet::new(), Duration::ZERO).await?, 1);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_codec_passes_contract() {
        photo_codec_contract(
            &FakePhotoCodec,
            &[
                (
                    b"\x89PNG\r\n\x1a\n contenu".to_vec(),
                    PhotoFormat::Png,
                    (16, 1),
                ),
                (
                    b"\xFF\xD8\xFF\xE0 contenu".to_vec(),
                    PhotoFormat::Jpeg,
                    (12, 1),
                ),
                (
                    b"RIFF\0\0\0\0WEBP contenu".to_vec(),
                    PhotoFormat::Webp,
                    (20, 1),
                ),
            ],
        );
    }

    #[tokio::test]
    async fn in_memory_store_passes_contract() -> Result<(), StoreError> {
        contact_photo_store_contract(&InMemoryPhotoStore::default()).await
    }
}
