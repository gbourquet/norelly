//! Cas d'usage des listes de diffusion (ADR-012) : créer
//! ([`CreateContactList`]), modifier ([`UpdateContactList`]), supprimer
//! ([`DeleteContactLists`]), lister ([`ListContactLists`]) et lire
//! ([`GetContactList`]) une liste ; y ajouter des contacts
//! ([`AddToContactList`]) ; la déplier en destinataires
//! ([`ExpandContactList`]) ; proposer des listes pendant la saisie
//! ([`SuggestContactLists`]).
//!
//! Les règles (validation, nom unique, expansion, listes proposées) sont
//! dans `norelly_domain::contact_lists` ; ce module valide la saisie par
//! le domaine avant toute écriture, puis appelle le carnet.

use norelly_domain::contact_lists::{
    ContactList, ContactListDraft, ContactListError, ContactListFilter, ContactListId,
    ContactListInput, Expansion, expand, list_suggestions,
};
use norelly_domain::contacts::{Contact, ContactId};
use norelly_domain::ports::{
    ContactDirectory, ContactListBook, ContactListCursor, ContactListDirectory, ContactListPage,
    ContactListStoreError, ContactListSummary, MAX_PAGE_SIZE, StoreError,
};

/// Échec d'un cas d'usage des listes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContactListFailure {
    /// Liste refusée par le domaine (ou vide à déplier) : rien n'a été
    /// écrit.
    #[error(transparent)]
    Invalid(#[from] ContactListError),
    /// Refus du carnet (nom pris, membre ou liste inconnus) ou stockage.
    #[error(transparent)]
    Store(#[from] ContactListStoreError),
}

impl From<StoreError> for ContactListFailure {
    fn from(e: StoreError) -> Self {
        Self::Store(ContactListStoreError::Store(e))
    }
}

/// Liste ouverte : la liste et les fiches de ses membres, dans son ordre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactListView {
    pub list: ContactList,
    pub members: Vec<Contact>,
}

/// Cas d'usage : enregistrer une nouvelle liste.
#[derive(Debug)]
pub struct CreateContactList<'a, B> {
    pub book: &'a B,
}

impl<B: ContactListBook> CreateContactList<'_, B> {
    pub async fn execute(
        &self,
        input: ContactListInput,
    ) -> Result<ContactList, ContactListFailure> {
        let draft = ContactListDraft::new(input)?;
        Ok(self.book.create_list(&draft).await?)
    }
}

/// Cas d'usage : remplacer le contenu de la liste `id`.
#[derive(Debug)]
pub struct UpdateContactList<'a, B> {
    pub book: &'a B,
}

impl<B: ContactListBook> UpdateContactList<'_, B> {
    pub async fn execute(
        &self,
        id: ContactListId,
        input: ContactListInput,
    ) -> Result<ContactList, ContactListFailure> {
        let draft = ContactListDraft::new(input)?;
        Ok(self.book.update_list(id, &draft).await?)
    }
}

/// Cas d'usage : supprimer des listes ; renvoie le nombre supprimé. Les
/// contacts restent.
#[derive(Debug)]
pub struct DeleteContactLists<'a, B> {
    pub book: &'a B,
}

impl<B: ContactListBook> DeleteContactLists<'_, B> {
    pub async fn execute(&self, ids: &[ContactListId]) -> Result<usize, ContactListFailure> {
        Ok(self.book.delete_lists(ids).await?)
    }
}

/// Cas d'usage : une page des listes, filtrée sur le nom (`text` vide :
/// toutes).
#[derive(Debug)]
pub struct ListContactLists<'a, D> {
    pub directory: &'a D,
}

impl<D: ContactListDirectory> ListContactLists<'_, D> {
    pub async fn execute(
        &self,
        text: &str,
        cursor: Option<ContactListCursor>,
        limit: u32,
    ) -> Result<ContactListPage, ContactListFailure> {
        let filter = ContactListFilter::new(text);
        Ok(self.directory.list_page(&filter, cursor, limit).await?)
    }
}

/// Cas d'usage : la liste `id` et les fiches de ses membres ; `NotFound`
/// si elle n'existe pas.
#[derive(Debug)]
pub struct GetContactList<'a, L, D> {
    pub lists: &'a L,
    pub contacts: &'a D,
}

impl<L: ContactListDirectory, D: ContactDirectory> GetContactList<'_, L, D> {
    pub async fn execute(&self, id: ContactListId) -> Result<ContactListView, ContactListFailure> {
        let list = read_list(self.lists, id).await?;
        let members = read_members(self.contacts, &list).await?;
        Ok(ContactListView { list, members })
    }
}

/// Cas d'usage : ajouter des contacts à la liste `id` (ceux qui y sont
/// déjà sont ignorés) ; renvoie la liste modifiée.
#[derive(Debug)]
pub struct AddToContactList<'a, B> {
    pub book: &'a B,
}

impl<B: ContactListBook + ContactListDirectory> AddToContactList<'_, B> {
    pub async fn execute(
        &self,
        id: ContactListId,
        contacts: &[ContactId],
    ) -> Result<ContactList, ContactListFailure> {
        let list = read_list(self.book, id).await?;
        let draft = list.draft().with_members(contacts)?;
        Ok(self.book.update_list(id, &draft).await?)
    }
}

/// Cas d'usage : déplier la liste `id` en destinataires
/// ([`norelly_domain::contact_lists::expand`]), fiches relues du carnet.
#[derive(Debug)]
pub struct ExpandContactList<'a, L, D> {
    pub lists: &'a L,
    pub contacts: &'a D,
}

impl<L: ContactListDirectory, D: ContactDirectory> ExpandContactList<'_, L, D> {
    pub async fn execute(&self, id: ContactListId) -> Result<Expansion, ContactListFailure> {
        let list = read_list(self.lists, id).await?;
        let members = read_members(self.contacts, &list).await?;
        Ok(expand(list.draft(), &members)?)
    }
}

/// Cas d'usage : listes proposées pendant la saisie de `text`
/// ([`norelly_domain::contact_lists::list_suggestions`]).
#[derive(Debug)]
pub struct SuggestContactLists<'a, D> {
    pub directory: &'a D,
}

impl<D: ContactListDirectory> SuggestContactLists<'_, D> {
    pub async fn execute(
        &self,
        text: &str,
        limit: usize,
    ) -> Result<Vec<ContactListSummary>, StoreError> {
        let filter = ContactListFilter::new(text);
        if filter.text().is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        // Les listes vides sont écartées après lecture : en lire un peu
        // plus que demandé.
        let page = self
            .directory
            .list_page(&filter, None, MAX_PAGE_SIZE)
            .await?;
        Ok(list_suggestions(text, &page.items, limit))
    }
}

/// La liste `id` ; `NotFound` si elle n'existe pas.
async fn read_list<L: ContactListDirectory>(
    lists: &L,
    id: ContactListId,
) -> Result<ContactList, ContactListFailure> {
    lists
        .get_list(id)
        .await?
        .ok_or(ContactListFailure::Store(ContactListStoreError::NotFound))
}

/// Fiches des membres de `list`, dans son ordre ; un contact supprimé
/// entre-temps est omis.
async fn read_members<D: ContactDirectory>(
    contacts: &D,
    list: &ContactList,
) -> Result<Vec<Contact>, StoreError> {
    let mut found = Vec::with_capacity(list.draft().members().len());
    for &id in list.draft().members() {
        if let Some(contact) = contacts.get(id).await? {
            found.push(contact);
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests;
