//! Cas d'usage de l'export du carnet (ADR-012) : fiches et listes à
//! écrire ([`PrepareContactExport`]), préparées par les règles du domaine
//! ([`export_card`], [`export_lists`]) pour le format choisi.
//!
//! L'écriture du fichier (port [`norelly_domain::ports::ContactCodec`],
//! puis disque) est faite par l'appelant hors des fils asynchrones : elle
//! peut porter sur tout un carnet.

use norelly_domain::contact_export::{
    ContactSelection, ExportCard, ExportFormat, ExportList, MAX_EXPORT_CONTACTS, export_card,
    export_lists, exports_lists,
};
use norelly_domain::ports::{ContactDirectory, ContactListDirectory, StoreError};

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
pub struct PrepareContactExport<'a, D> {
    pub directory: &'a D,
}

impl<D: ContactDirectory + ContactListDirectory> PrepareContactExport<'_, D> {
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
        Ok(PreparedExport {
            cards: contacts
                .iter()
                .map(|c| export_card(c.draft(), format))
                .collect(),
            lists,
        })
    }
}

#[cfg(test)]
mod tests;
