//! Cas d'usage du carnet d'adresses (ADR-012) : créer
//! ([`CreateContact`]), modifier ([`UpdateContact`]), supprimer
//! ([`DeleteContacts`]), lister ([`ListContacts`]) et lire
//! ([`GetContact`]) une fiche ; nommer des correspondants d'après le
//! carnet ([`NameSenders`]) ; suggérer des destinataires
//! ([`SuggestRecipients`]).
//!
//! Les règles (validation de la fiche, nom affiché, filtre, ordre) sont
//! dans `norelly_domain::contacts` ; ce module valide la saisie par le
//! domaine avant toute écriture, puis appelle le carnet.

use norelly_domain::account::AccountId;
use norelly_domain::contacts::{
    Contact, ContactDraft, ContactError, ContactFilter, ContactId, ContactInput,
};
use norelly_domain::mailbox::Correspondent;
use norelly_domain::ports::{
    ContactBook, ContactCursor, ContactDirectory, ContactPage, ContactQuery, ContactStoreError,
    MAX_CONTACT_SUGGESTIONS, StoreError,
};
use norelly_domain::sender::{SenderNames, sender_keys};
use norelly_domain::suggestions::{RecipientSuggestion, recipient_suggestions};

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
/// valide, le carnet n'est pas interrogé. Nommer enrichit l'affichage :
/// un carnet illisible laisse les noms des en-têtes (avertissement
/// journalisé, sans adresse) au lieu d'empêcher la lecture du courrier.
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
    ) -> impl Future<Output = SenderNames> + Send + '_ {
        let keys = sender_keys(people);
        async move {
            if keys.is_empty() {
                return SenderNames::default();
            }
            match self.directory.by_addresses(&keys).await {
                Ok(contacts) => SenderNames::new(contacts),
                Err(e) => {
                    tracing::warn!(error = %e, "carnet illisible : noms des en-têtes gardés");
                    SenderNames::default()
                }
            }
        }
    }
}

/// Cas d'usage : au plus `limit` destinataires proposés pendant la saisie
/// de `text` ([`recipient_suggestions`]) :
/// contacts du carnet (lus par la liste, favoris puis tous, puis fiche par
/// fiche), puis adresses collectées du compte. Un carnet illisible laisse
/// les seules adresses collectées (avertissement journalisé).
#[derive(Debug)]
pub struct SuggestRecipients<'a, Q, D> {
    pub collected: &'a Q,
    pub directory: &'a D,
}

impl<Q: ContactQuery, D: ContactDirectory> SuggestRecipients<'_, Q, D> {
    pub async fn execute(
        &self,
        account: AccountId,
        text: &str,
        limit: usize,
    ) -> Result<Vec<RecipientSuggestion>, StoreError> {
        let limit = limit.min(MAX_CONTACT_SUGGESTIONS);
        let filter = ContactFilter::new(text, false);
        if filter.text().is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let contacts = match self.contacts(&filter, limit).await {
            Ok(contacts) => contacts,
            Err(e) => {
                tracing::warn!(error = %e, "carnet illisible : adresses collectées seules");
                Vec::new()
            }
        };
        let collected = self
            .collected
            .suggest(account, filter.text(), limit)
            .await?;
        Ok(recipient_suggestions(text, &contacts, &collected, limit))
    }

    /// Fiches complètes des contacts qui passent `filter` : les `limit`
    /// premiers favoris, puis les `limit` premiers de la liste, chacun une
    /// fois.
    async fn contacts(
        &self,
        filter: &ContactFilter,
        limit: usize,
    ) -> Result<Vec<Contact>, StoreError> {
        let limit = u32::try_from(limit).unwrap_or(u32::MAX);
        let favorites = ContactFilter::new(filter.text(), true);
        let mut ids = Vec::new();
        for f in [&favorites, filter] {
            for item in self.directory.page(f, None, limit).await?.items {
                if !ids.contains(&item.id) {
                    ids.push(item.id);
                }
            }
        }
        let mut contacts = Vec::with_capacity(ids.len());
        for id in ids {
            // Fiche supprimée entre les deux lectures : ignorée.
            if let Some(contact) = self.directory.get(id).await? {
                contacts.push(contact);
            }
        }
        Ok(contacts)
    }
}

#[cfg(test)]
mod tests;
