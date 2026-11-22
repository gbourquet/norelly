//! Cas d'usage de l'import d'un carnet (ADR-012) : aperçu d'un fichier
//! ([`PreviewContactImport`]) puis import en une transaction
//! ([`ApplyContactImport`]), listes de diffusion comprises.
//!
//! Le fichier est traduit par le port [`ContactCodec`] ; le plan (doublons,
//! rejets, normalisation) est celui du domaine
//! ([`norelly_domain::contact_import::plan_import`]), calculé contre les
//! contacts du carnet qui portent les adresses du fichier et les listes du
//! carnet qui portent le nom d'une de ses listes. L'import
//! recalcule le plan contre le carnet du moment : une fiche ajoutée entre
//! l'aperçu et l'import devient un doublon. Un carnet qui change entre
//! cette relecture et l'écriture (contact ou liste à compléter modifié ou
//! supprimé, adresse ou nom de liste pris entre-temps, voir
//! [`norelly_domain::contact_import::plan_outdated`]) n'est pas écrasé :
//! l'écriture est refusée, le carnet relu et le plan recalculé, au plus
//! [`MAX_IMPORT_ATTEMPTS`] fois.
//!
//! Un import dont les nouvelles fiches porteraient le carnet au-delà de
//! [`norelly_domain::contacts::MAX_CONTACTS`] est refusé en entier
//! (`BookFull`), dès l'aperçu, puis de nouveau avant toute écriture, et
//! par le carnet lui-même dans la transaction : jamais d'import partiel.
//!
//! Les photos intégrées aux fiches suivent le plan
//! ([`norelly_domain::contact_import::plan_import_with_photos`]) : chaque
//! photo retenue est réencodée par le port [`PhotoCodec`] (hors des fils
//! asynchrones), enregistrée dans le [`ContactPhotoStore`], puis donnée à
//! son contact dans la transaction de l'import. Une photo que le
//! réencodage refuse est écartée, la fiche importée sans elle.

use std::collections::{HashMap, HashSet};

use norelly_domain::contact_import::{
    ContactFormat, DecodedContacts, DuplicatePolicy, ImportPlan, ImportedCard, MAX_IMPORT_ATTEMPTS,
    PhotoPlacement, PhotoTarget, PlacedPhoto, import_keys, import_list_keys,
    plan_import_with_photos, plan_outdated,
};
use norelly_domain::contact_lists::ContactList;
use norelly_domain::contact_photo::{CardPhotos, DecodeBudget};
use norelly_domain::contacts::{Contact, fits_in_book};
use norelly_domain::ports::{
    BlobId, ContactBook, ContactCodec, ContactCodecError, ContactDirectory, ContactListDirectory,
    ContactPhotoStore, ContactStoreError, ImportOutcome, MAX_CONTACT_LOOKUP, PhotoCodec,
    StoreError, TextNormalizer,
};

/// Échec d'un import.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportFailure {
    /// Fichier refusé en entier (trop gros, trop de fiches, format
    /// inconnu) : rien n'est lu.
    #[error(transparent)]
    File(#[from] ContactCodecError),
    /// Refus du carnet (carnet plein compris) ou stockage : rien n'est
    /// écrit.
    #[error(transparent)]
    Store(#[from] ContactStoreError),
}

impl From<StoreError> for ImportFailure {
    fn from(e: StoreError) -> Self {
        Self::Store(ContactStoreError::Store(e))
    }
}

/// Aperçu d'un fichier : ses fiches, et le plan de chaque politique.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportPreview {
    pub format: ContactFormat,
    /// Fiches lues, à repasser à [`ApplyContactImport`].
    pub cards: Vec<ImportedCard>,
    /// Photos intégrées aux fiches, à repasser avec elles.
    pub photos: CardPhotos,
    /// Plan si les doublons sont ignorés.
    pub skip: ImportPlan,
    /// Plan si les contacts existants sont complétés.
    pub complete: ImportPlan,
}

/// Contacts du carnet qui portent une adresse des fiches (membres des
/// listes compris).
async fn existing<D: ContactDirectory>(
    directory: &D,
    cards: &[ImportedCard],
    normalizer: &impl TextNormalizer,
) -> Result<Vec<Contact>, StoreError> {
    let keys = import_keys(cards, normalizer);
    let mut contacts: Vec<Contact> = Vec::new();
    let mut seen = HashSet::new();
    for chunk in keys.chunks(MAX_CONTACT_LOOKUP) {
        for contact in directory.by_addresses(chunk).await? {
            if seen.insert(contact.id()) {
                contacts.push(contact);
            }
        }
    }
    Ok(contacts)
}

/// Refus d'un plan qui créerait plus de fiches que le carnet de
/// `directory` n'en peut recevoir ([`fits_in_book`]).
async fn check_room<D: ContactDirectory>(
    directory: &D,
    plans: &[&ImportPlan],
) -> Result<(), ImportFailure> {
    let current = directory.count().await?;
    if plans.iter().all(|p| fits_in_book(current, p.create.len())) {
        Ok(())
    } else {
        Err(ContactStoreError::BookFull.into())
    }
}

/// Listes du carnet qui portent le nom d'une liste du fichier.
async fn existing_lists<D: ContactListDirectory>(
    directory: &D,
    cards: &[ImportedCard],
    normalizer: &impl TextNormalizer,
) -> Result<Vec<ContactList>, StoreError> {
    let keys = import_list_keys(cards, normalizer);
    let mut lists: Vec<ContactList> = Vec::new();
    let mut seen = HashSet::new();
    for chunk in keys.chunks(MAX_CONTACT_LOOKUP) {
        for list in directory.lists_named(chunk).await? {
            if seen.insert(list.id()) {
                lists.push(list);
            }
        }
    }
    Ok(lists)
}

/// Cas d'usage : lire un fichier de contacts et en chiffrer l'import.
#[derive(Debug)]
pub struct PreviewContactImport<'a, C, D, N> {
    pub codec: &'a C,
    pub directory: &'a D,
    pub normalizer: &'a N,
}

impl<C: ContactCodec, D: ContactDirectory + ContactListDirectory, N: TextNormalizer>
    PreviewContactImport<'_, C, D, N>
{
    pub async fn execute(&self, bytes: &[u8]) -> Result<ImportPreview, ImportFailure> {
        let decoded = self.codec.decode(bytes)?;
        self.plan(decoded).await
    }

    /// Chiffre l'import de fiches déjà lues (lecture faite hors des fils
    /// asynchrones par l'appelant). Refusé (`BookFull`) si l'un des deux
    /// plans ne tient pas dans le carnet.
    pub async fn plan(&self, decoded: DecodedContacts) -> Result<ImportPreview, ImportFailure> {
        let existing = existing(self.directory, &decoded.cards, self.normalizer).await?;
        let lists = existing_lists(self.directory, &decoded.cards, self.normalizer).await?;
        let plan = |policy| {
            plan_import_with_photos(
                &existing,
                &lists,
                &decoded.cards,
                &decoded.photos,
                policy,
                self.normalizer,
            )
        };
        let (skip, complete) = (plan(DuplicatePolicy::Skip), plan(DuplicatePolicy::Complete));
        check_room(self.directory, &[&skip, &complete]).await?;
        Ok(ImportPreview {
            format: decoded.format,
            skip,
            complete,
            cards: decoded.cards,
            photos: decoded.photos,
        })
    }
}

/// Cas d'usage : importer les fiches d'un aperçu selon `policy`, en une
/// transaction.
#[derive(Debug)]
pub struct ApplyContactImport<'a, B, N, P, S> {
    pub book: &'a B,
    pub normalizer: &'a N,
    pub photo_codec: &'a P,
    pub photos: &'a S,
}

impl<B, N, P, S> ApplyContactImport<'_, B, N, P, S>
where
    B: ContactBook + ContactDirectory + ContactListDirectory,
    N: TextNormalizer,
    P: PhotoCodec + Clone + 'static,
    S: ContactPhotoStore,
{
    pub async fn execute(
        &self,
        cards: &[ImportedCard],
        photos: &CardPhotos,
        policy: DuplicatePolicy,
    ) -> Result<ImportOutcome, ImportFailure> {
        // Photo réencodée et enregistrée de chaque fiche, une fois quel que
        // soit le nombre de tentatives (`None` : refusée au réencodage).
        let mut stored: HashMap<usize, Option<BlobId>> = HashMap::new();
        // Pixels décodés pour tout l'import, toutes tentatives comprises.
        let mut budget = DecodeBudget::default();
        let mut attempt = 1;
        loop {
            let existing = existing(self.book, cards, self.normalizer).await?;
            let lists = existing_lists(self.book, cards, self.normalizer).await?;
            let mut plan =
                plan_import_with_photos(&existing, &lists, cards, photos, policy, self.normalizer);
            // Carnet trop plein : refusé avant de réencoder les photos.
            check_room(self.book, &[&plan]).await?;
            let mut placed = Vec::with_capacity(plan.photos.len());
            for placement in &plan.photos {
                let photo = match stored.get(&placement.card) {
                    Some(photo) => photo.clone(),
                    None => {
                        let photo = self.store_photo(photos, placement, &mut budget).await?;
                        stored.insert(placement.card, photo.clone());
                        photo
                    }
                };
                if let Some(photo) = photo {
                    placed.push(PlacedPhoto {
                        target: placement.target,
                        photo,
                    });
                }
            }
            // Un contact complété par sa seule photo, refusée au
            // réencodage, n'a plus rien à recevoir.
            plan.complete.retain(|c| {
                c.draft() != c.read().draft()
                    || placed
                        .iter()
                        .any(|p| p.target == PhotoTarget::Existing(c.id()))
            });
            let written = self
                .book
                .import(&plan.create, &plan.complete, &plan.lists, &placed)
                .await;
            match written {
                // Le carnet a changé depuis sa lecture : rien n'est écrit,
                // le plan est recalculé sur le carnet relu.
                Err(e) if plan_outdated(&e) && attempt < MAX_IMPORT_ATTEMPTS => {
                    attempt += 1;
                }
                outcome => return Ok(outcome?),
            }
        }
    }
}

impl<B, N, P, S> ApplyContactImport<'_, B, N, P, S>
where
    P: PhotoCodec + Clone + 'static,
    S: ContactPhotoStore,
{
    /// Photo de la fiche `placement.card` réencodée (hors des fils
    /// asynchrones) puis enregistrée ; `None` si le réencodage la refuse,
    /// si elle dépasse le budget de décodage de l'import, ou si le
    /// décodeur échoue en paniquant.
    async fn store_photo(
        &self,
        photos: &CardPhotos,
        placement: &PhotoPlacement,
        budget: &mut DecodeBudget,
    ) -> Result<Option<BlobId>, ImportFailure> {
        let Some(photo) = photos.get(&placement.card) else {
            return Ok(None);
        };
        let codec = self.photo_codec.clone();
        let bytes = photo.bytes.clone();
        let format = placement.format;
        // En-tête d'abord : le budget décide avant le décodage des pixels.
        let header = {
            let (codec, bytes) = (codec.clone(), bytes.clone());
            tokio::task::spawn_blocking(move || codec.dimensions(&bytes, format)).await
        };
        match header {
            Ok(Ok((width, height))) if budget.take(width, height) => {}
            Ok(Ok(_)) => {
                tracing::debug!("photo de contact écartée : budget de décodage atteint");
                return Ok(None);
            }
            Ok(Err(e)) => {
                tracing::debug!(error = %e, "photo de contact écartée");
                return Ok(None);
            }
            Err(_) => {
                tracing::warn!("photo de contact écartée : décodeur interrompu");
                return Ok(None);
            }
        }
        let thumbnail = tokio::task::spawn_blocking(move || codec.thumbnail(&bytes, format)).await;
        let Ok(thumbnail) = thumbnail else {
            // Panique du décodeur : la photo est écartée, pas l'import ; ni
            // le message ni le contenu ne sont journalisés.
            tracing::warn!("photo de contact écartée : décodeur interrompu");
            return Ok(None);
        };
        match thumbnail {
            Ok(thumbnail) => Ok(Some(self.photos.put(thumbnail).await?)),
            Err(e) => {
                // Ni le contenu ni la fiche dans le journal.
                tracing::debug!(error = %e, "photo de contact écartée");
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests;
