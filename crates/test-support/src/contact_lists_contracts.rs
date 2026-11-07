//! Suite de contrat des listes de diffusion ([`ContactListBook`] et
//! [`ContactListDirectory`]), jouée contre le faux en mémoire
//! ([`InMemoryContactBook`]) et contre SQLite : les listes vivent dans le
//! carnet, à côté des contacts qu'elles regroupent.

use norelly_domain::contact_lists::{
    ContactList, ContactListDraft, ContactListFilter, ContactListId, ContactListInput,
};
use norelly_domain::contacts::{ContactId, MAX_CONTACT_NAME_CHARS};
use norelly_domain::ports::{
    ContactBook, ContactDirectory, ContactListBook, ContactListCursor, ContactListDirectory,
    ContactListPage, ContactListStoreError, ContactListSummary, MAX_CONTACT_LIST_SORT_KEY_CHARS,
    MAX_PAGE_SIZE, StoreError,
};

use crate::contacts_contracts::{InMemoryContactBook, sample_contact};

/// Liste de test : `name` et ses membres, dans cet ordre.
#[must_use]
pub fn sample_list(name: &str, members: &[ContactId], bcc: bool) -> ContactListDraft {
    ContactListDraft::new(ContactListInput {
        name: name.to_owned(),
        description: String::new(),
        members: members.iter().map(|id| id.get()).collect(),
        bcc,
    })
    .unwrap_or_else(|e| panic!("liste de test refusée : {e}"))
}

/// Toutes les listes, page par page (`limit` par page).
async fn all_pages<D: ContactListDirectory>(
    directory: &D,
    filter: &ContactListFilter,
    limit: u32,
) -> Result<Vec<ContactListSummary>, StoreError> {
    let mut items = Vec::new();
    let mut cursor = None;
    loop {
        let page = directory.list_page(filter, cursor, limit).await?;
        assert!(
            page.items.len() <= limit as usize,
            "page plus longue que la limite"
        );
        items.extend(page.items);
        match page.next {
            Some(next) => cursor = Some(next),
            None => return Ok(items),
        }
    }
}

fn names(items: &[ContactListSummary]) -> Vec<&str> {
    items.iter().map(|i| i.name.as_str()).collect()
}

/// Contrat des listes de diffusion (carnet vide au départ).
pub async fn contact_list_contract<B>(book: &B) -> Result<(), ContactListStoreError>
where
    B: ContactBook + ContactDirectory + ContactListBook + ContactListDirectory,
{
    let all = ContactListFilter::default();
    let contact = |name: &'static str, address: &'static str| async move {
        book.create(&sample_contact(name, &[address]))
            .await
            .map(|c| c.id())
            .map_err(|e| ContactListStoreError::Store(StoreError(e.to_string())))
    };
    let alice = contact("Alice", "alice@example.com").await?;
    let bob = contact("Bob", "bob@example.net").await?;
    let carol = contact("Carol", "carol@example.org").await?;

    assert_eq!(
        book.list_page(&all, None, 10).await?,
        ContactListPage::default(),
        "listes au départ"
    );
    let unknown = ContactListId::new(i64::MAX).unwrap_or_else(|| panic!("id"));
    assert_eq!(book.get_list(unknown).await?, None);

    // Création et relecture : nom, description, Cci, membres dans leur
    // ordre.
    let family = ContactListDraft::new(ContactListInput {
        name: "Famille".into(),
        description: "Les cousins aussi".into(),
        members: vec![bob.get(), alice.get()],
        bcc: true,
    })
    .unwrap_or_else(|e| panic!("{e}"));
    let famille = book.create_list(&family).await?;
    assert_eq!(famille.draft(), &family, "liste relue différente");
    assert_eq!(book.get_list(famille.id()).await?, Some(famille.clone()));
    let club = book
        .create_list(&sample_list("club de lecture", &[carol], false))
        .await?;
    assert_ne!(club.id(), famille.id(), "identifiants identiques");

    // Nom unique à la casse près ; le refus cite le nom saisi, sans effet.
    assert_eq!(
        book.create_list(&sample_list("FAMILLE", &[carol], false))
            .await,
        Err(ContactListStoreError::DuplicateName("FAMILLE".into()))
    );
    assert_eq!(all_pages(book, &all, 10).await?.len(), 2);

    // Membre absent du carnet : refus sans effet.
    let ghost = ContactId::new(i64::MAX).unwrap_or_else(|| panic!("id"));
    assert_eq!(
        book.create_list(&sample_list("Fantômes", &[alice, ghost], false))
            .await,
        Err(ContactListStoreError::UnknownContact(ghost))
    );
    assert_eq!(all_pages(book, &all, 10).await?.len(), 2);

    // Modification : contenu remplacé ; changer la casse de son propre nom
    // est permis, prendre celui d'une autre liste non.
    let renamed = sample_list("FAMILLE", &[carol, alice], false);
    let updated = book.update_list(famille.id(), &renamed).await?;
    assert_eq!(updated.id(), famille.id());
    assert_eq!(updated.draft(), &renamed);
    assert_eq!(book.get_list(famille.id()).await?, Some(updated.clone()));
    assert_eq!(
        book.update_list(famille.id(), &sample_list("Club de Lecture", &[], false))
            .await,
        Err(ContactListStoreError::DuplicateName(
            "Club de Lecture".into()
        ))
    );
    assert_eq!(
        book.update_list(famille.id(), &sample_list("Famille", &[ghost], false))
            .await,
        Err(ContactListStoreError::UnknownContact(ghost))
    );
    assert_eq!(book.get_list(famille.id()).await?, Some(updated.clone()));
    assert_eq!(
        book.update_list(unknown, &renamed).await,
        Err(ContactListStoreError::NotFound)
    );

    // Ligne de liste : nom, nombre de membres, Cci.
    let line = book.list_page(&all, None, 10).await?;
    assert_eq!(
        line.items,
        vec![
            ContactListSummary::from(&club),
            ContactListSummary::from(&updated)
        ]
    );
    assert_eq!(line.items[1].members, 2);
    assert!(!line.items[1].bcc);

    // Ordre : nom sans la casse, puis identifiant ; pagination sans
    // doublon ni oubli ; filtre sur le nom.
    for name in ["Voisins", "anciens collègues", "Zumba"] {
        book.create_list(&sample_list(name, &[], false)).await?;
    }
    let expected = vec![
        "anciens collègues",
        "club de lecture",
        "FAMILLE",
        "Voisins",
        "Zumba",
    ];
    for limit in [1, 2, 3, 10] {
        assert_eq!(
            names(&all_pages(book, &all, limit).await?),
            expected,
            "pages de {limit}"
        );
    }
    let first = book.list_page(&all, None, 5).await?;
    assert_eq!(first.next, None, "page complète en fin : pas de suite");
    assert_eq!(
        book.list_page(&all, None, 0).await?,
        ContactListPage::default(),
        "limite nulle"
    );
    let page = book.list_page(&all, None, 2).await?;
    let Some(cursor) = page.next.clone() else {
        panic!("suite attendue");
    };
    assert_eq!(page.items.last().map(|i| i.id), Some(cursor.id));
    // Une liste ajoutée avant le curseur ne décale pas la suite.
    let aaa = book.create_list(&sample_list("Aaa", &[], false)).await?;
    assert_eq!(
        names(&book.list_page(&all, Some(cursor), 10).await?.items),
        vec!["FAMILLE", "Voisins", "Zumba"]
    );
    assert_eq!(
        ContactListCursor::of(&aaa),
        ContactListCursor {
            sort_key: "aaa".into(),
            id: aaa.id()
        }
    );
    assert_eq!(
        names(&all_pages(book, &ContactListFilter::new("L"), 1).await?),
        vec!["anciens collègues", "club de lecture", "FAMILLE"]
    );
    for joker in ["%", "_", "\\"] {
        assert!(
            all_pages(book, &ContactListFilter::new(joker), 10)
                .await?
                .is_empty(),
            "{joker:?}"
        );
    }
    assert!(book.list_page(&all, None, MAX_PAGE_SIZE + 1).await.is_ok());
    assert!(ContactListCursor::parse("a".repeat(MAX_CONTACT_LIST_SORT_KEY_CHARS + 1), 1).is_none());
    assert!(ContactListCursor::parse("a".repeat(MAX_CONTACT_NAME_CHARS), 1).is_some());

    // Supprimer un contact le retire de toutes ses listes, sans toucher
    // aux autres membres ni à leur ordre.
    let both = book
        .update_list(
            club.id(),
            &sample_list("club de lecture", &[alice, carol, bob], false),
        )
        .await?;
    assert_eq!(both.draft().members(), [alice, carol, bob]);
    assert_eq!(
        book.delete_many(&[carol])
            .await
            .map_err(|e| ContactListStoreError::Store(StoreError(e.to_string())))?,
        1
    );
    assert_eq!(
        book.get_list(club.id())
            .await?
            .map(|l| l.draft().members().to_vec()),
        Some(vec![alice, bob])
    );
    assert_eq!(
        book.get_list(famille.id())
            .await?
            .map(|l| l.draft().members().to_vec()),
        Some(vec![alice])
    );
    let line = book
        .list_page(&ContactListFilter::new("club"), None, 10)
        .await?;
    assert_eq!(line.items.first().map(|i| i.members), Some(2));

    // Suppression de listes, idempotente ; les contacts restent.
    assert_eq!(
        book.delete_lists(&[club.id(), unknown, aaa.id(), club.id()])
            .await?,
        2
    );
    assert_eq!(book.delete_lists(&[club.id()]).await?, 0);
    assert_eq!(book.delete_lists(&[]).await?, 0);
    assert_eq!(book.get_list(club.id()).await?, None);
    assert!(
        book.get(alice).await?.is_some(),
        "contact supprimé avec la liste"
    );
    assert!(
        book.get(bob).await?.is_some(),
        "contact supprimé avec la liste"
    );
    // Le nom d'une liste supprimée est libre ; l'identifiant n'est pas
    // réutilisé.
    let again = book
        .create_list(&sample_list("Club de lecture", &[bob], false))
        .await?;
    assert!(again.id() != club.id(), "identifiant réutilisé");
    assert_eq!(
        book.update_list(club.id(), &renamed).await,
        Err(ContactListStoreError::NotFound)
    );
    let rest: Vec<ContactListId> = all_pages(book, &all, 10)
        .await?
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(book.delete_lists(&rest).await?, rest.len());
    assert_eq!(
        book.list_page(&all, None, 10).await?,
        ContactListPage::default()
    );
    Ok(())
}

/// Refus d'une écriture : nom pris par une autre liste que `except`, ou
/// membre absent du carnet.
fn refused(
    book: &InMemoryContactBook,
    lists: &[ContactList],
    draft: &ContactListDraft,
    except: Option<ContactListId>,
) -> Option<ContactListStoreError> {
    let key = draft.name_key();
    if lists
        .iter()
        .any(|l| Some(l.id()) != except && l.draft().name_key() == key)
    {
        return Some(ContactListStoreError::DuplicateName(
            draft.name().to_owned(),
        ));
    }
    let contacts = book.lock();
    draft
        .members()
        .iter()
        .find(|id| !contacts.1.iter().any(|c| c.id() == **id))
        .map(|id| ContactListStoreError::UnknownContact(*id))
}

fn sorted(lists: &[ContactList]) -> Vec<ContactList> {
    let mut out = lists.to_vec();
    out.sort_by_key(|a| (a.draft().name_key(), a.id()));
    out
}

impl ContactListBook for InMemoryContactBook {
    async fn create_list(
        &self,
        draft: &ContactListDraft,
    ) -> Result<ContactList, ContactListStoreError> {
        let mut lists = self.lock_lists();
        if let Some(e) = refused(self, &lists.1, draft, None) {
            return Err(e);
        }
        lists.0 += 1;
        let id = ContactListId::new(lists.0)
            .ok_or_else(|| ContactListStoreError::Store(StoreError("identifiant".into())))?;
        let list = draft.clone().with_id(id);
        lists.1.push(list.clone());
        Ok(list)
    }

    async fn update_list(
        &self,
        id: ContactListId,
        draft: &ContactListDraft,
    ) -> Result<ContactList, ContactListStoreError> {
        let mut lists = self.lock_lists();
        let Some(index) = lists.1.iter().position(|l| l.id() == id) else {
            return Err(ContactListStoreError::NotFound);
        };
        if let Some(e) = refused(self, &lists.1, draft, Some(id)) {
            return Err(e);
        }
        let list = draft.clone().with_id(id);
        lists.1[index] = list.clone();
        Ok(list)
    }

    async fn delete_lists(&self, ids: &[ContactListId]) -> Result<usize, ContactListStoreError> {
        let mut lists = self.lock_lists();
        let before = lists.1.len();
        lists.1.retain(|l| !ids.contains(&l.id()));
        Ok(before - lists.1.len())
    }
}

impl ContactListDirectory for InMemoryContactBook {
    async fn list_page(
        &self,
        filter: &ContactListFilter,
        cursor: Option<ContactListCursor>,
        limit: u32,
    ) -> Result<ContactListPage, StoreError> {
        let limit = limit.min(MAX_PAGE_SIZE) as usize;
        let mut rest = sorted(&self.lock_lists().1)
            .into_iter()
            .filter(|l| filter.matches(l.draft()))
            .filter(|l| {
                cursor
                    .as_ref()
                    .is_none_or(|c| (l.draft().name_key(), l.id()) > (c.sort_key.clone(), c.id))
            });
        let page: Vec<ContactList> = rest.by_ref().take(limit).collect();
        let next = match (rest.next(), page.last()) {
            (Some(_), Some(last)) => Some(ContactListCursor::of(last)),
            _ => None,
        };
        Ok(ContactListPage {
            items: page.iter().map(ContactListSummary::from).collect(),
            next,
        })
    }

    async fn get_list(&self, id: ContactListId) -> Result<Option<ContactList>, StoreError> {
        Ok(self.lock_lists().1.iter().find(|l| l.id() == id).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_contact_lists_pass_contract() -> Result<(), ContactListStoreError> {
        contact_list_contract(&InMemoryContactBook::default()).await
    }
}
