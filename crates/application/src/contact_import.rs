//! Cas d'usage de l'import d'un carnet (ADR-012) : aperçu d'un fichier
//! ([`PreviewContactImport`]) puis import en une transaction
//! ([`ApplyContactImport`]).
//!
//! Le fichier est traduit par le port [`ContactCodec`] ; le plan (doublons,
//! rejets, normalisation) est celui du domaine
//! ([`norelly_domain::contact_import::plan_import`]), calculé contre les
//! contacts du carnet qui portent les adresses du fichier. L'import
//! recalcule le plan contre le carnet du moment : une fiche ajoutée entre
//! l'aperçu et l'import devient un doublon.

use std::collections::HashSet;

use norelly_domain::contact_import::{
    ContactFormat, DuplicatePolicy, ImportPlan, ImportedCard, import_keys, plan_import,
};
use norelly_domain::contacts::Contact;
use norelly_domain::ports::{
    ContactBook, ContactCodec, ContactCodecError, ContactDirectory, ContactStoreError,
    ImportOutcome, MAX_CONTACT_LOOKUP, StoreError, TextNormalizer,
};

/// Échec d'un import.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportFailure {
    /// Fichier refusé en entier (trop gros, trop de fiches, format
    /// inconnu) : rien n'est lu.
    #[error(transparent)]
    File(#[from] ContactCodecError),
    /// Refus du carnet ou stockage : rien n'est écrit.
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
    /// Plan si les doublons sont ignorés.
    pub skip: ImportPlan,
    /// Plan si les contacts existants sont complétés.
    pub complete: ImportPlan,
}

/// Contacts du carnet qui portent une adresse des fiches.
async fn existing<D: ContactDirectory>(
    directory: &D,
    cards: &[ImportedCard],
) -> Result<Vec<Contact>, StoreError> {
    let keys = import_keys(cards);
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

/// Cas d'usage : lire un fichier de contacts et en chiffrer l'import.
#[derive(Debug)]
pub struct PreviewContactImport<'a, C, D, N> {
    pub codec: &'a C,
    pub directory: &'a D,
    pub normalizer: &'a N,
}

impl<C: ContactCodec, D: ContactDirectory, N: TextNormalizer> PreviewContactImport<'_, C, D, N> {
    pub async fn execute(&self, bytes: &[u8]) -> Result<ImportPreview, ImportFailure> {
        let decoded = self.codec.decode(bytes)?;
        let existing = existing(self.directory, &decoded.cards).await?;
        let plan = |policy| plan_import(&existing, &decoded.cards, policy, self.normalizer);
        Ok(ImportPreview {
            format: decoded.format,
            skip: plan(DuplicatePolicy::Skip),
            complete: plan(DuplicatePolicy::Complete),
            cards: decoded.cards,
        })
    }
}

/// Cas d'usage : importer les fiches d'un aperçu selon `policy`, en une
/// transaction.
#[derive(Debug)]
pub struct ApplyContactImport<'a, B, N> {
    pub book: &'a B,
    pub normalizer: &'a N,
}

impl<B: ContactBook + ContactDirectory, N: TextNormalizer> ApplyContactImport<'_, B, N> {
    pub async fn execute(
        &self,
        cards: &[ImportedCard],
        policy: DuplicatePolicy,
    ) -> Result<ImportOutcome, ImportFailure> {
        let existing = existing(self.book, cards).await?;
        let plan = plan_import(&existing, cards, policy, self.normalizer);
        Ok(self.book.import(&plan.create, &plan.complete).await?)
    }
}

#[cfg(test)]
mod tests;
