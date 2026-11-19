//! Cas d'usage de l'export du carnet (ADR-012) : fiches et listes à
//! écrire ([`PrepareContactExport`]), préparées par les règles du domaine
//! ([`export_card`], [`export_lists`]) pour le format choisi.
//!
//! La photo d'un contact, réencodée à l'import, est lue dans le
//! [`ContactPhotoStore`] pour les formats qui la gardent
//! ([`ExportFormat::keeps_photos`]) ; une photo absente, illisible ou qui
//! n'est pas un JPEG est omise, sans empêcher l'export, comme toutes les
//! photos au-delà du budget de l'export
//! ([`norelly_domain::contact_export::MAX_EXPORT_PHOTO_BYTES`]).
//!
//! L'écriture du fichier (port [`norelly_domain::ports::ContactCodec`],
//! puis disque) est faite par l'appelant hors des fils asynchrones : elle
//! peut porter sur tout un carnet.

use norelly_domain::contact_export::{
    ContactSelection, ExportCard, ExportFormat, ExportList, MAX_EXPORT_CONTACTS, PhotoBudget,
    export_card, export_lists, exports_lists,
};
use norelly_domain::contact_photo::is_kept_photo;
use norelly_domain::ports::{
    ContactDirectory, ContactListDirectory, ContactPhotoStore, StoreError,
};

/// Échec d'un export.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExportFailure {
    /// Aucun contact à exporter (carnet vide, sélection supprimée).
    #[error("aucun contact à exporter")]
    Empty,
    /// Plus de [`MAX_EXPORT_CONTACTS`] contacts et listes.
    #[error("trop de contacts à exporter")]
    TooMany,
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Fiches et listes à écrire.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreparedExport {
    /// Fiches, dans l'ordre de la liste des contacts.
    pub cards: Vec<ExportCard>,
    /// Listes de diffusion (tout le carnet, format qui les porte), dans
    /// l'ordre de leur nom.
    pub lists: Vec<ExportList>,
}

/// Cas d'usage : lire les contacts visés (et les listes avec tout le
/// carnet) et les préparer pour `format`.
#[derive(Debug)]
pub struct PrepareContactExport<'a, D, S> {
    pub directory: &'a D,
    pub photos: &'a S,
}

impl<D: ContactDirectory + ContactListDirectory, S: ContactPhotoStore>
    PrepareContactExport<'_, D, S>
{
    /// Fiches et listes à écrire ; ensemble au plus
    /// [`MAX_EXPORT_CONTACTS`].
    pub async fn execute(
        &self,
        selection: &ContactSelection,
        format: ExportFormat,
    ) -> Result<PreparedExport, ExportFailure> {
        // Une fiche de plus que la borne : savoir qu'elle est dépassée.
        let contacts = self
            .directory
            .contacts(selection, MAX_EXPORT_CONTACTS + 1)
            .await?;
        if contacts.is_empty() {
            return Err(ExportFailure::Empty);
        }
        if contacts.len() > MAX_EXPORT_CONTACTS {
            return Err(ExportFailure::TooMany);
        }
        let lists = if exports_lists(selection, format) {
            // Une liste de plus que la place restante : savoir qu'elle est
            // dépassée.
            let room = MAX_EXPORT_CONTACTS - contacts.len();
            let lists = self.directory.all_lists(room + 1).await?;
            if lists.len() > room {
                return Err(ExportFailure::TooMany);
            }
            export_lists(&lists, &contacts)
        } else {
            Vec::new()
        };
        let mut cards = Vec::with_capacity(contacts.len());
        let mut budget = PhotoBudget::default();
        for contact in &contacts {
            let mut card = export_card(contact.draft(), format);
            if format.keeps_photos()
                && !budget.exhausted()
                && let Some(photo) = contact.photo()
            {
                card.photo = match self.photos.get(photo).await {
                    // Seule une photo gardée (JPEG) est écrite, dans la
                    // limite du budget des photos.
                    Ok(Some(bytes)) if is_kept_photo(&bytes) && budget.take(bytes.len()) => {
                        Some(bytes)
                    }
                    Ok(_) => None,
                    Err(e) => {
                        // Ni le contenu ni la fiche dans le journal.
                        tracing::warn!(error = %e, "photo de contact illisible, omise de l'export");
                        None
                    }
                };
            }
            cards.push(card);
        }
        Ok(PreparedExport { cards, lists })
    }
}

#[cfg(test)]
mod tests;
