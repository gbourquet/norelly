//! Cas d'usage des photos de contact (ADR-012) : lire la photo d'un
//! contact pour l'afficher ([`ReadContactPhoto`]) et faire le ménage du
//! magasin des photos ([`SweepContactPhotos`]).
//!
//! Les photos ne sont écrites que par l'import
//! ([`crate::contact_import::ApplyContactImport`]), déjà réencodées.

use norelly_domain::contact_photo::PHOTO_SWEEP_MIN_AGE;
use norelly_domain::contacts::ContactId;
use norelly_domain::ports::{ContactDirectory, ContactPhotoStore, StoreError};

/// Cas d'usage : la photo réencodée du contact `id` ; `None` s'il n'existe
/// pas, n'a pas de photo, ou si elle manque au magasin.
#[derive(Debug)]
pub struct ReadContactPhoto<'a, D, S> {
    pub directory: &'a D,
    pub photos: &'a S,
}

impl<D: ContactDirectory, S: ContactPhotoStore> ReadContactPhoto<'_, D, S> {
    pub async fn execute(&self, id: ContactId) -> Result<Option<Vec<u8>>, StoreError> {
        let Some(photo) = self
            .directory
            .get(id)
            .await?
            .and_then(|c| c.photo().cloned())
        else {
            return Ok(None);
        };
        self.photos.get(&photo).await
    }
}

/// Cas d'usage : supprimer du magasin les photos qu'aucun contact ne porte
/// plus (contacts supprimés ou fusionnés, import interrompu) ; renvoie
/// leur nombre. À lancer quand aucun import n'est en cours (ouverture de
/// l'application) : une photo enregistrée par un import, pas encore
/// donnée à son contact, serait supprimée.
#[derive(Debug)]
pub struct SweepContactPhotos<'a, D, S> {
    pub directory: &'a D,
    pub photos: &'a S,
}

impl<D: ContactDirectory, S: ContactPhotoStore> SweepContactPhotos<'_, D, S> {
    pub async fn execute(&self) -> Result<usize, StoreError> {
        let carried = self.directory.photo_ids().await?;
        self.photos.retain(&carried, PHOTO_SWEEP_MIN_AGE).await
    }
}

#[cfg(test)]
mod tests {
    use norelly_domain::contact_import::{PhotoTarget, PlacedPhoto};
    use norelly_domain::ports::ContactBook;
    use norelly_test_support::contact_photo_contracts::InMemoryPhotoStore;
    use norelly_test_support::contacts_contracts::{InMemoryContactBook, sample_contact};

    use super::*;

    /// Carnet d'Alice (avec la photo `photo`, enregistrée) et de Bob (sans
    /// photo).
    async fn book_with_photo(
        photo: &[u8],
    ) -> Result<(InMemoryContactBook, InMemoryPhotoStore), Box<dyn std::error::Error>> {
        let book = InMemoryContactBook::default();
        let store = InMemoryPhotoStore::default();
        let id = store.put(photo.to_vec()).await?;
        book.import(
            &[
                sample_contact("Alice", &["alice@example.com"]),
                sample_contact("Bob", &["bob@example.com"]),
            ],
            &[],
            &[],
            &[PlacedPhoto {
                target: PhotoTarget::Created(0),
                photo: id,
            }],
        )
        .await?;
        Ok((book, store))
    }

    fn id(raw: i64) -> ContactId {
        ContactId::new(raw).unwrap_or_else(|| panic!("id"))
    }

    #[tokio::test]
    async fn the_photo_of_a_contact_is_read_from_the_store()
    -> Result<(), Box<dyn std::error::Error>> {
        let (book, store) = book_with_photo(b"\xFF\xD8\xFF photo").await?;
        let read = ReadContactPhoto {
            directory: &book,
            photos: &store,
        };
        assert_eq!(
            read.execute(id(1)).await?,
            Some(b"\xFF\xD8\xFF photo".to_vec())
        );
        // Sans photo, ou contact inconnu.
        assert_eq!(read.execute(id(2)).await?, None);
        assert_eq!(read.execute(id(99)).await?, None);
        // Photo absente du magasin (ménage, disque).
        store
            .retain(&Default::default(), std::time::Duration::ZERO)
            .await?;
        assert_eq!(read.execute(id(1)).await?, None);
        Ok(())
    }

    #[tokio::test]
    async fn the_sweep_removes_photos_no_contact_carries() -> Result<(), Box<dyn std::error::Error>>
    {
        let (book, store) = book_with_photo(b"\xFF\xD8\xFF gardee").await?;
        let orphan = store.put(b"\xFF\xD8\xFF orpheline".to_vec()).await?;
        let sweep = SweepContactPhotos {
            directory: &book,
            photos: &store,
        };
        // Trop récente : peut-être celle d'un import en cours.
        assert_eq!(sweep.execute().await?, 0);
        store.backdate(PHOTO_SWEEP_MIN_AGE);
        assert_eq!(sweep.execute().await?, 1);
        assert_eq!(store.get(&orphan).await?, None);
        assert_eq!(store.ids().len(), 1);
        // Alice supprimée : sa photo part au ménage suivant.
        book.delete(id(1)).await?;
        assert_eq!(sweep.execute().await?, 1);
        assert!(store.ids().is_empty());
        Ok(())
    }
}
