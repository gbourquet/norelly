//! Cas d'usage du carnet d'adresses (ADR-012) : créer
//! ([`CreateContact`]), modifier ([`UpdateContact`]), supprimer
//! ([`DeleteContacts`]), lister ([`ListContacts`]) et lire
//! ([`GetContact`]) une fiche ; nommer des correspondants d'après le
//! carnet ([`NameSenders`]).
//!
//! Les règles (validation de la fiche, nom affiché, filtre, ordre) sont
//! dans `norelly_domain::contacts` ; ce module valide la saisie par le
//! domaine avant toute écriture, puis appelle le carnet.

use norelly_domain::contacts::{
    Contact, ContactDraft, ContactError, ContactFilter, ContactId, ContactInput,
};
use norelly_domain::mailbox::Correspondent;
use norelly_domain::ports::{
    ContactBook, ContactCursor, ContactDirectory, ContactPage, ContactStoreError, StoreError,
};
use norelly_domain::sender::{SenderNames, sender_keys};

/// Échec d'un cas d'usage du carnet.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContactFailure {
    /// Fiche refusée par le domaine : rien n'a été écrit.
    #[error(transparent)]
    Invalid(#[from] ContactError),
    /// Refus du carnet (adresse déjà prise, fiche inconnue) ou stockage.
    #[error(transparent)]
    Store(#[from] ContactStoreError),
}

impl From<StoreError> for ContactFailure {
    fn from(e: StoreError) -> Self {
        Self::Store(ContactStoreError::Store(e))
    }
}

/// Cas d'usage : enregistrer une nouvelle fiche.
#[derive(Debug)]
pub struct CreateContact<'a, B> {
    pub book: &'a B,
}

impl<B: ContactBook> CreateContact<'_, B> {
    pub async fn execute(&self, input: ContactInput) -> Result<Contact, ContactFailure> {
        let draft = ContactDraft::new(input)?;
        Ok(self.book.create(&draft).await?)
    }
}

/// Cas d'usage : remplacer le contenu de la fiche `id`.
#[derive(Debug)]
pub struct UpdateContact<'a, B> {
    pub book: &'a B,
}

impl<B: ContactBook> UpdateContact<'_, B> {
    pub async fn execute(
        &self,
        id: ContactId,
        input: ContactInput,
    ) -> Result<Contact, ContactFailure> {
        let draft = ContactDraft::new(input)?;
        Ok(self.book.update(id, &draft).await?)
    }
}

/// Cas d'usage : supprimer des fiches (une ou plusieurs) ; renvoie le
/// nombre de fiches supprimées.
#[derive(Debug)]
pub struct DeleteContacts<'a, B> {
    pub book: &'a B,
}

impl<B: ContactBook> DeleteContacts<'_, B> {
    pub async fn execute(&self, ids: &[ContactId]) -> Result<usize, ContactFailure> {
        Ok(self.book.delete_many(ids).await?)
    }
}

/// Cas d'usage : une page de la liste des contacts, filtrée sur `text`
/// (vide : tous) et, au besoin, les seuls favoris.
#[derive(Debug)]
pub struct ListContacts<'a, D> {
    pub directory: &'a D,
}

impl<D: ContactDirectory> ListContacts<'_, D> {
    pub async fn execute(
        &self,
        text: &str,
        favorites_only: bool,
        cursor: Option<ContactCursor>,
        limit: u32,
    ) -> Result<ContactPage, ContactFailure> {
        let filter = ContactFilter::new(text, favorites_only);
        Ok(self.directory.page(&filter, cursor, limit).await?)
    }
}

/// Cas d'usage : la fiche complète `id` ; `NotFound` si elle n'existe
/// pas.
#[derive(Debug)]
pub struct GetContact<'a, D> {
    pub directory: &'a D,
}

impl<D: ContactDirectory> GetContact<'_, D> {
    pub async fn execute(&self, id: ContactId) -> Result<Contact, ContactFailure> {
        self.directory
            .get(id)
            .await?
            .ok_or(ContactFailure::Store(ContactStoreError::NotFound))
    }
}

/// Cas d'usage : contacts du carnet qui portent les adresses de ces
/// correspondants, pour les nommer ([`SenderNames::label`]). Sans adresse
/// valide, le carnet n'est pas interrogé.
#[derive(Debug)]
pub struct NameSenders<'a, D> {
    pub directory: &'a D,
}

impl<D: ContactDirectory> NameSenders<'_, D> {
    /// Les clés sont calculées avant l'attente : le futur ne garde pas
    /// les correspondants.
    pub fn execute<'p>(
        &self,
        people: impl IntoIterator<Item = &'p Correspondent>,
    ) -> impl Future<Output = Result<SenderNames, StoreError>> + Send + '_ {
        let keys = sender_keys(people);
        async move {
            if keys.is_empty() {
                return Ok(SenderNames::default());
            }
            Ok(SenderNames::new(self.directory.by_addresses(&keys).await?))
        }
    }
}

#[cfg(test)]
mod tests;
