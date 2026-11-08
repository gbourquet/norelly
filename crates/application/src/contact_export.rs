//! Cas d'usage de l'export du carnet (ADR-012) : fiches à écrire
//! ([`PrepareContactExport`]), préparées par la règle du domaine
//! ([`export_card`]) pour le format choisi.
//!
//! L'écriture du fichier (port [`norelly_domain::ports::ContactCodec`],
//! puis disque) est faite par l'appelant hors des fils asynchrones : elle
//! peut porter sur tout un carnet.

use norelly_domain::contact_export::{
    ContactSelection, ExportCard, ExportFormat, MAX_EXPORT_CONTACTS, export_card,
};
use norelly_domain::ports::{ContactDirectory, StoreError};

/// Échec d'un export.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExportFailure {
    /// Aucun contact à exporter (carnet vide, sélection supprimée).
    #[error("aucun contact à exporter")]
    Empty,
    /// Plus de [`MAX_EXPORT_CONTACTS`] contacts.
    #[error("trop de contacts à exporter")]
    TooMany,
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Cas d'usage : lire les contacts visés et les préparer pour `format`.
#[derive(Debug)]
pub struct PrepareContactExport<'a, D> {
    pub directory: &'a D,
}

impl<D: ContactDirectory> PrepareContactExport<'_, D> {
    /// Fiches à écrire, dans l'ordre de la liste.
    pub async fn execute(
        &self,
        selection: &ContactSelection,
        format: ExportFormat,
    ) -> Result<Vec<ExportCard>, ExportFailure> {
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
        Ok(contacts
            .iter()
            .map(|c| export_card(c.draft(), format))
            .collect())
    }
}

#[cfg(test)]
mod tests;
