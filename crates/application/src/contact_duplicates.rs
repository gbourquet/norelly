//! Cas d'usage des doublons du carnet (ADR-012) : chercher les groupes de
//! doublons ([`FindDuplicates`]) et fusionner des fiches
//! ([`MergeContacts`]).
//!
//! Les règles (critères de rapprochement, fiche proposée, fusion) sont
//! dans `norelly_domain::contact_duplicates` ; ce module lit le carnet,
//! applique la règle et écrit la fusion en une transaction.

use norelly_domain::contact_duplicates::{
    DuplicateReason, MAX_DUPLICATE_SCAN, MergeSelection, find_duplicates, merge_contacts,
};
use norelly_domain::contact_export::ContactSelection;
use std::collections::HashMap;

use norelly_domain::contacts::{Contact, ContactDraft, ContactId};
use norelly_domain::ports::{
    ContactBook, ContactDirectory, ContactStoreError, StoreError, TextNormalizer,
};

use crate::contacts::ContactFailure;

/// Échec d'une recherche de doublons.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DuplicateScanFailure {
    /// Plus de [`MAX_DUPLICATE_SCAN`] contacts dans le carnet.
    #[error("trop de contacts pour chercher les doublons")]
    TooMany,
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Groupe de doublons, fiches lues : la fiche proposée à garder d'abord.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateCandidates {
    pub contacts: Vec<Contact>,
    pub reasons: Vec<DuplicateReason>,
}

/// Cas d'usage : groupes de doublons du carnet.
#[derive(Debug)]
pub struct FindDuplicates<'a, D, N> {
    pub directory: &'a D,
    pub normalizer: &'a N,
}

impl<D: ContactDirectory, N: TextNormalizer> FindDuplicates<'_, D, N> {
    pub async fn execute(&self) -> Result<Vec<DuplicateCandidates>, DuplicateScanFailure> {
        // Une fiche de plus que la borne : savoir qu'elle est dépassée.
        let contacts = self
            .directory
            .contacts(&ContactSelection::All, MAX_DUPLICATE_SCAN + 1)
            .await?;
        if contacts.len() > MAX_DUPLICATE_SCAN {
            return Err(DuplicateScanFailure::TooMany);
        }
        let by_id: HashMap<ContactId, &Contact> = contacts.iter().map(|c| (c.id(), c)).collect();
        Ok(find_duplicates(&contacts, self.normalizer)
            .into_iter()
            .map(|group| DuplicateCandidates {
                contacts: group
                    .contacts
                    .iter()
                    .filter_map(|id| by_id.get(id).map(|c| (*c).clone()))
                    .collect(),
                reasons: group.reasons,
            })
            .collect())
    }
}

/// Cas d'usage : fusionner les fiches de `selection` dans sa fiche gardée.
#[derive(Debug)]
pub struct MergeContacts<'a, B> {
    pub book: &'a B,
}

impl<B: ContactBook + ContactDirectory> MergeContacts<'_, B> {
    pub async fn execute(&self, selection: &MergeSelection) -> Result<Contact, ContactFailure> {
        let read = |id| async move {
            self.book
                .get(id)
                .await?
                .ok_or(ContactFailure::Store(ContactStoreError::NotFound))
        };
        let keep = read(selection.keep()).await?;
        let mut others = Vec::with_capacity(selection.others().len());
        for &id in selection.others() {
            others.push(read(id).await?);
        }
        let drafts: Vec<&ContactDraft> = others.iter().map(Contact::draft).collect();
        let merged = merge_contacts(keep.draft(), &drafts)?;
        Ok(self
            .book
            .merge(selection.keep(), &merged, selection.others())
            .await?)
    }
}

#[cfg(test)]
mod tests;
