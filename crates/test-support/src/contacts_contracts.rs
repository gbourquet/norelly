//! Suite de contrat du carnet d'adresses ([`ContactBook`] et
//! [`ContactDirectory`]), jouée contre le faux en mémoire
//! ([`InMemoryContactBook`]) et contre SQLite.

use std::sync::{Mutex, MutexGuard, PoisonError};

use norelly_domain::address::EmailAddress;
use norelly_domain::contact_lists::{ContactList, ContactListDraft, ContactListInput};
use norelly_domain::contacts::{
    AddressKey, Contact, ContactDraft, ContactFilter, ContactId, ContactInput, EmailInput,
    PhoneInput,
};
use norelly_domain::ports::{
    ContactBook, ContactCursor, ContactDirectory, ContactListItem, ContactPage, ContactStoreError,
    ImportOutcome, MAX_CONTACT_LOOKUP, MAX_PAGE_SIZE, StoreError,
};

/// Fiche de test : `name` (nom affiché, vide : aucun) et ses adresses, la
/// première préférée.
#[must_use]
pub fn sample_contact(name: &str, emails: &[&str]) -> ContactDraft {
    ContactDraft::new(ContactInput {
        display_name: name.to_owned(),
        emails: emails
            .iter()
            .map(|a| EmailInput {
                address: (*a).to_owned(),
                ..EmailInput::default()
            })
            .collect(),
        ..ContactInput::default()
    })
    .unwrap_or_else(|e| panic!("fiche de test refusée : {e}"))
}

/// Fiche de test complète (tous les champs remplis).
#[must_use]
pub fn full_contact() -> ContactDraft {
    ContactDraft::new(ContactInput {
        display_name: String::new(),
        given_name: "Alice".into(),
        family_name: "Martin".into(),
        nickname: "Lili".into(),
        organization: "ACME".into(),
        notes: "Rencontrée au salon.\nAime le thé.".into(),
        emails: vec![
            EmailInput {
                address: "alice@example.com".into(),
                label: "travail".into(),
                preferred: false,
            },
            EmailInput {
                address: "Alice.Perso@Example.org".into(),
                label: "maison".into(),
                preferred: true,
            },
        ],
        phones: vec![
            PhoneInput {
                number: "+33 1 23 45 67 89".into(),
                label: "bureau".into(),
            },
            PhoneInput {
                number: "06 12 34 56 78".into(),
                label: String::new(),
            },
        ],
        favorite: true,
    })
    .unwrap_or_else(|e| panic!("fiche de test refusée : {e}"))
}

fn key(raw: &str) -> AddressKey {
    AddressKey::parse(raw).unwrap_or_else(|e| panic!("{raw} : {e}"))
}

fn address(raw: &str) -> EmailAddress {
    EmailAddress::parse(raw).unwrap_or_else(|e| panic!("{raw} : {e}"))
}

/// Toute la liste, page par page (`limit` par page).
async fn all_pages<D: ContactDirectory>(
    directory: &D,
    filter: &ContactFilter,
    limit: u32,
) -> Result<Vec<ContactListItem>, StoreError> {
    let mut items = Vec::new();
    let mut cursor = None;
    loop {
        let page = directory.page(filter, cursor, limit).await?;
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

fn names(items: &[ContactListItem]) -> Vec<&str> {
    items.iter().map(|i| i.name.as_str()).collect()
}

/// Contrat du carnet d'adresses (carnet vide au départ).
pub async fn contact_book_contract<B: ContactBook + ContactDirectory>(
    book: &B,
) -> Result<(), ContactStoreError> {
    let all = ContactFilter::default();
    assert_eq!(
        book.page(&all, None, 10).await?,
        ContactPage::default(),
        "carnet non vide au départ"
    );
    let unknown = ContactId::new(i64::MAX).unwrap_or_else(|| panic!("id"));
    assert_eq!(book.get(unknown).await?, None);

    // Création et relecture, tous champs compris.
    let alice = book.create(&full_contact()).await?;
    assert_eq!(alice.draft(), &full_contact(), "fiche relue différente");
    assert_eq!(book.get(alice.id()).await?, Some(alice.clone()));
    let bob = book
        .create(&sample_contact("Bob", &["bob@example.net"]))
        .await?;
    assert_ne!(alice.id(), bob.id(), "identifiants identiques");

    // Une adresse n'appartient qu'à un contact, à la casse près ; le refus
    // est sans effet.
    let clash = sample_contact("Autre", &["autre@example.com", "ALICE@EXAMPLE.COM"]);
    assert_eq!(
        book.create(&clash).await,
        Err(ContactStoreError::DuplicateAddress(address(
            "ALICE@EXAMPLE.COM"
        )))
    );
    assert_eq!(all_pages(book, &all, 10).await?.len(), 2);
    assert_eq!(
        book.by_addresses(&[key("autre@example.com")]).await?,
        Vec::<Contact>::new(),
        "création refusée mais adresse enregistrée"
    );

    // Modification : contenu remplacé, ses propres adresses conservables.
    let renamed = sample_contact(
        "Alice M.",
        &["alice.perso@example.org", "neuve@example.com"],
    );
    let updated = book.update(alice.id(), &renamed).await?;
    assert_eq!(updated.id(), alice.id());
    assert_eq!(updated.draft(), &renamed);
    assert_eq!(book.get(alice.id()).await?, Some(updated.clone()));
    // L'adresse retirée est libre.
    assert!(
        book.by_addresses(&[key("alice@example.com")])
            .await?
            .is_empty()
    );
    // Adresse d'un autre contact : refus sans effet.
    let steal = sample_contact("Alice M.", &["alice.perso@example.org", "Bob@example.net"]);
    assert_eq!(
        book.update(alice.id(), &steal).await,
        Err(ContactStoreError::DuplicateAddress(address(
            "Bob@example.net"
        )))
    );
    assert_eq!(book.get(alice.id()).await?, Some(updated.clone()));
    assert_eq!(
        book.update(unknown, &renamed).await,
        Err(ContactStoreError::NotFound)
    );

    // Recherche par adresse : chaque contact une fois, dans l'ordre de la
    // liste, inconnues ignorées.
    let found = book
        .by_addresses(&[
            key("BOB@example.net"),
            key("inconnu@example.com"),
            key("NEUVE@example.com"),
            key("alice.perso@example.org"),
        ])
        .await?;
    assert_eq!(found, vec![updated.clone(), bob.clone()]);
    assert!(book.by_addresses(&[]).await?.is_empty());
    let mut many: Vec<AddressKey> = (0..MAX_CONTACT_LOOKUP)
        .map(|i| key(&format!("x{i}@example.com")))
        .collect();
    many.push(key("bob@example.net"));
    assert!(
        book.by_addresses(&many).await?.is_empty(),
        "au-delà de MAX_CONTACT_LOOKUP adresses, le reste est ignoré"
    );

    // Ordre de la liste : clé de tri (nom affiché en minuscules), puis
    // identifiant ; pagination sans doublon ni oubli.
    let zoe = book
        .create(&sample_contact("zoé", &["zoe@example.com"]))
        .await?;
    let anon = book
        .create(&sample_contact("", &["anonyme@example.com"]))
        .await?;
    let bob2 = book
        .create(&sample_contact("bob", &["bob2@example.com"]))
        .await?;
    let expected = vec!["Alice M.", "anonyme@example.com", "Bob", "bob", "zoé"];
    for limit in [1, 2, 3, 10] {
        assert_eq!(
            names(&all_pages(book, &all, limit).await?),
            expected,
            "pages de {limit}"
        );
    }
    let first = book.page(&all, None, 5).await?;
    assert_eq!(first.items.len(), 5);
    assert_eq!(
        first.next, None,
        "page complète en fin de liste : pas de suite"
    );
    assert_eq!(
        book.page(&all, None, 0).await?,
        ContactPage::default(),
        "limite nulle"
    );
    let page = book.page(&all, None, 2).await?;
    let Some(cursor) = page.next.clone() else {
        panic!("suite attendue");
    };
    assert_eq!(page.items.last().map(|i| i.id), Some(cursor.id));
    // Ligne de liste : nom, adresse préférée, organisation, favori.
    let line = book
        .page(&ContactFilter::new("acme", false), None, 10)
        .await?;
    assert!(
        line.items.is_empty(),
        "organisation retirée par la modification"
    );
    let favorite = book.update(alice.id(), &full_contact()).await?;
    let line = book
        .page(&ContactFilter::new("acme", false), None, 10)
        .await?;
    assert_eq!(line.items, vec![ContactListItem::from(&favorite)]);
    assert_eq!(line.items[0].name, "Alice Martin");
    assert_eq!(
        line.items[0].address,
        address("Alice.Perso@example.org"),
        "adresse préférée"
    );
    assert_eq!(line.items[0].organization.as_deref(), Some("ACME"));
    assert!(line.items[0].favorite);
    // Recherche sur chaque nom (prénom, nom, surnom) et sur toutes les
    // adresses, pas seulement la préférée : base des suggestions de
    // destinataires.
    for text in ["ALICE", "martin", "lili", "alice@example.com", "perso@"] {
        let found = book
            .page(&ContactFilter::new(text, false), None, 10)
            .await?;
        assert_eq!(
            found.items.iter().map(|i| i.id).collect::<Vec<_>>(),
            vec![favorite.id()],
            "{text}"
        );
        let favorites = book.page(&ContactFilter::new(text, true), None, 10).await?;
        assert_eq!(favorites.items.len(), 1, "{text} parmi les favoris");
    }

    // Un contact ajouté avant le curseur ne décale pas la suite.
    let page = book.page(&all, None, 2).await?;
    let Some(cursor) = page.next else {
        panic!("suite attendue");
    };
    let aaron = book
        .create(&sample_contact("Aaron", &["aaron@example.com"]))
        .await?;
    let rest = book.page(&all, Some(cursor), 10).await?;
    assert_eq!(names(&rest.items), vec!["Bob", "bob", "zoé"]);
    assert_eq!(
        ContactCursor::of(&aaron),
        ContactCursor {
            sort_key: "aaron".into(),
            id: aaron.id()
        }
    );

    // Filtres : texte (noms, organisation, adresses, sans la casse),
    // favoris, combinés.
    let filtered = |text: &str, fav: bool| ContactFilter::new(text, fav);
    assert_eq!(
        names(&all_pages(book, &filtered("MART", false), 1).await?),
        vec!["Alice Martin"]
    );
    assert_eq!(
        names(&all_pages(book, &filtered("example.net", false), 1).await?),
        vec!["Bob"]
    );
    assert_eq!(
        names(&all_pages(book, &filtered("bo", false), 1).await?),
        vec!["Bob", "bob"]
    );
    assert_eq!(
        names(&all_pages(book, &filtered("", true), 1).await?),
        vec!["Alice Martin"]
    );
    assert!(all_pages(book, &filtered("bob", true), 1).await?.is_empty());
    assert!(
        all_pages(book, &filtered("thé", false), 1)
            .await?
            .is_empty()
    );
    // Jokers SQL pris littéralement.
    for joker in ["%", "_", "\\"] {
        assert!(
            all_pages(book, &filtered(joker, false), 10)
                .await?
                .is_empty(),
            "{joker:?}"
        );
    }

    // Suppression, idempotente ; les adresses sont libérées.
    assert!(book.delete(zoe.id()).await?);
    assert!(!book.delete(zoe.id()).await?);
    assert_eq!(book.get(zoe.id()).await?, None);
    assert_eq!(
        book.delete_many(&[bob.id(), unknown, bob2.id(), bob.id()])
            .await?,
        2
    );
    assert_eq!(book.delete_many(&[]).await?, 0);
    assert_eq!(
        names(&all_pages(book, &all, 10).await?),
        vec!["Aaron", "Alice Martin", "anonyme@example.com"]
    );
    let again = book
        .create(&sample_contact("Bob", &["BOB@example.net"]))
        .await?;
    assert!(again.id() != bob.id(), "identifiant réutilisé");
    assert_eq!(
        book.update(zoe.id(), &renamed).await,
        Err(ContactStoreError::NotFound)
    );
    assert_eq!(
        book.delete_many(&[again.id(), aaron.id(), anon.id(), alice.id()])
            .await?,
        4
    );
    assert_eq!(book.page(&all, None, 10).await?, ContactPage::default());
    Ok(())
}

/// Contrat de l'import en une transaction ([`ContactBook::import`]),
/// carnet vide au départ : tout ou rien.
pub async fn contact_import_contract<B: ContactBook + ContactDirectory>(
    book: &B,
) -> Result<(), ContactStoreError> {
    let all = ContactFilter::default();
    assert_eq!(book.import(&[], &[]).await?, ImportOutcome::default());

    let outcome = book
        .import(
            &[
                sample_contact("Alice", &["alice@example.com"]),
                full_contact(),
            ],
            &[],
        )
        .await;
    // `full_contact` porte aussi alice@example.com : rien n'est écrit.
    assert_eq!(
        outcome,
        Err(ContactStoreError::DuplicateAddress(address(
            "alice@example.com"
        )))
    );
    assert_eq!(book.page(&all, None, 10).await?, ContactPage::default());

    let outcome = book
        .import(
            &[
                sample_contact("Alice", &["alice@example.com"]),
                sample_contact("Bob", &["bob@example.net"]),
            ],
            &[],
        )
        .await?;
    assert_eq!(
        outcome,
        ImportOutcome {
            created: 2,
            completed: 0
        }
    );
    let listed = all_pages(book, &all, 10).await?;
    assert_eq!(names(&listed), vec!["Alice", "Bob"]);
    let alice = book
        .get(listed[0].id)
        .await?
        .unwrap_or_else(|| panic!("Alice relue"));
    let bob_id = listed[1].id;

    // Création et complément ensemble.
    let completed =
        sample_contact("Alice", &["alice@example.com", "alice@work.example"]).with_id(alice.id());
    let outcome = book
        .import(
            &[sample_contact("Carole", &["carole@example.org"])],
            std::slice::from_ref(&completed),
        )
        .await?;
    assert_eq!(
        outcome,
        ImportOutcome {
            created: 1,
            completed: 1
        }
    );
    assert_eq!(book.get(alice.id()).await?, Some(completed.clone()));
    assert_eq!(all_pages(book, &all, 10).await?.len(), 3);

    // Un complément qui prendrait l'adresse d'un autre contact, ou un
    // contact inconnu, annule tout l'import.
    let stolen =
        sample_contact("Alice", &["alice@example.com", "BOB@example.net"]).with_id(alice.id());
    assert_eq!(
        book.import(
            &[sample_contact("Denis", &["denis@example.org"])],
            &[stolen]
        )
        .await,
        Err(ContactStoreError::DuplicateAddress(address(
            "BOB@example.net"
        )))
    );
    let unknown = sample_contact("Zoé", &["zoe@example.org"])
        .with_id(ContactId::new(i64::MAX).unwrap_or_else(|| panic!("id")));
    assert_eq!(
        book.import(
            &[sample_contact("Denis", &["denis@example.org"])],
            &[unknown]
        )
        .await,
        Err(ContactStoreError::NotFound)
    );
    // Une création qui reprend une adresse complétée dans le même import
    // est refusée aussi.
    let again =
        sample_contact("Alice", &["alice@example.com", "a3@example.com"]).with_id(alice.id());
    assert_eq!(
        book.import(&[sample_contact("Autre", &["A3@example.com"])], &[again])
            .await,
        Err(ContactStoreError::DuplicateAddress(address(
            "A3@example.com"
        )))
    );
    assert_eq!(book.get(alice.id()).await?, Some(completed));
    assert!(book.get(bob_id).await?.is_some());
    assert_eq!(
        names(&all_pages(book, &all, 10).await?),
        vec!["Alice", "Bob", "Carole"]
    );
    Ok(())
}

/// Faux carnet d'adresses en mémoire, listes de diffusion comprises
/// ([`crate::contact_lists_contracts`]).
#[derive(Debug, Default)]
pub struct InMemoryContactBook {
    state: Mutex<(i64, Vec<Contact>)>,
    /// Listes : dernier identifiant attribué, listes. Verrouillé après
    /// `state` quand les deux le sont.
    pub(crate) lists: Mutex<(i64, Vec<ContactList>)>,
}

impl InMemoryContactBook {
    pub(crate) fn lock(&self) -> MutexGuard<'_, (i64, Vec<Contact>)> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn lock_lists(&self) -> MutexGuard<'_, (i64, Vec<ContactList>)> {
        self.lists.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Première adresse de `draft` déjà portée par un contact autre que
/// `except`.
fn taken(
    contacts: &[Contact],
    draft: &ContactDraft,
    except: Option<ContactId>,
) -> Option<EmailAddress> {
    draft
        .emails()
        .iter()
        .find(|e| {
            contacts
                .iter()
                .any(|c| Some(c.id()) != except && c.draft().has_address(e.key()))
        })
        .map(|e| e.address().clone())
}

fn sorted(contacts: &[Contact]) -> Vec<Contact> {
    let mut out = contacts.to_vec();
    out.sort_by_key(|c| SortKey(ContactCursor::of(c)));
    out
}

/// Curseur comparé dans l'ordre de la liste.
#[derive(Debug, PartialEq, Eq)]
struct SortKey(ContactCursor);

impl Ord for SortKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (&self.0.sort_key, self.0.id).cmp(&(&other.0.sort_key, other.0.id))
    }
}

impl PartialOrd for SortKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl ContactBook for InMemoryContactBook {
    async fn create(&self, draft: &ContactDraft) -> Result<Contact, ContactStoreError> {
        let mut state = self.lock();
        if let Some(address) = taken(&state.1, draft, None) {
            return Err(ContactStoreError::DuplicateAddress(address));
        }
        state.0 += 1;
        let id = ContactId::new(state.0)
            .ok_or_else(|| ContactStoreError::Corrupted("identifiant".into()))?;
        let contact = draft.clone().with_id(id);
        state.1.push(contact.clone());
        Ok(contact)
    }

    async fn update(
        &self,
        id: ContactId,
        draft: &ContactDraft,
    ) -> Result<Contact, ContactStoreError> {
        let mut state = self.lock();
        let Some(index) = state.1.iter().position(|c| c.id() == id) else {
            return Err(ContactStoreError::NotFound);
        };
        if let Some(address) = taken(&state.1, draft, Some(id)) {
            return Err(ContactStoreError::DuplicateAddress(address));
        }
        let contact = draft.clone().with_id(id);
        state.1[index] = contact.clone();
        Ok(contact)
    }

    async fn delete(&self, id: ContactId) -> Result<bool, ContactStoreError> {
        Ok(self.delete_many(&[id]).await? == 1)
    }

    async fn import(
        &self,
        create: &[ContactDraft],
        complete: &[Contact],
    ) -> Result<ImportOutcome, ContactStoreError> {
        let mut state = self.lock();
        // Écrit sur une copie, gardée seulement si tout passe.
        let (mut last, mut contacts) = state.clone();
        for contact in complete {
            let Some(index) = contacts.iter().position(|c| c.id() == contact.id()) else {
                return Err(ContactStoreError::NotFound);
            };
            if let Some(address) = taken(&contacts, contact.draft(), Some(contact.id())) {
                return Err(ContactStoreError::DuplicateAddress(address));
            }
            contacts[index] = contact.clone();
        }
        for draft in create {
            if let Some(address) = taken(&contacts, draft, None) {
                return Err(ContactStoreError::DuplicateAddress(address));
            }
            last += 1;
            let id = ContactId::new(last)
                .ok_or_else(|| ContactStoreError::Corrupted("identifiant".into()))?;
            contacts.push(draft.clone().with_id(id));
        }
        *state = (last, contacts);
        Ok(ImportOutcome {
            created: create.len(),
            completed: complete.len(),
        })
    }

    async fn delete_many(&self, ids: &[ContactId]) -> Result<usize, ContactStoreError> {
        let mut state = self.lock();
        let before = state.1.len();
        state.1.retain(|c| !ids.contains(&c.id()));
        // Un contact supprimé quitte toutes ses listes.
        let mut lists = self.lock_lists();
        for list in &mut lists.1 {
            let kept: Vec<i64> = list
                .draft()
                .members()
                .iter()
                .filter(|m| !ids.contains(m))
                .map(|m| m.get())
                .collect();
            if kept.len() != list.draft().members().len() {
                let draft = ContactListDraft::new(ContactListInput {
                    members: kept,
                    ..list.draft().to_input()
                })
                .map_err(|e| ContactStoreError::Corrupted(e.to_string()))?;
                *list = draft.with_id(list.id());
            }
        }
        Ok(before - state.1.len())
    }
}

impl ContactDirectory for InMemoryContactBook {
    async fn page(
        &self,
        filter: &ContactFilter,
        cursor: Option<ContactCursor>,
        limit: u32,
    ) -> Result<ContactPage, StoreError> {
        let limit = limit.min(MAX_PAGE_SIZE) as usize;
        let after = cursor.map(SortKey);
        let mut rest = sorted(&self.lock().1)
            .into_iter()
            .filter(|c| filter.matches(c.draft()))
            .filter(|c| {
                after
                    .as_ref()
                    .is_none_or(|a| SortKey(ContactCursor::of(c)) > *a)
            });
        let page: Vec<Contact> = rest.by_ref().take(limit).collect();
        let next = match (rest.next(), page.last()) {
            (Some(_), Some(last)) => Some(ContactCursor::of(last)),
            _ => None,
        };
        Ok(ContactPage {
            items: page.iter().map(ContactListItem::from).collect(),
            next,
        })
    }

    async fn get(&self, id: ContactId) -> Result<Option<Contact>, StoreError> {
        Ok(self.lock().1.iter().find(|c| c.id() == id).cloned())
    }

    async fn by_addresses(&self, keys: &[AddressKey]) -> Result<Vec<Contact>, StoreError> {
        let keys = &keys[..keys.len().min(MAX_CONTACT_LOOKUP)];
        Ok(sorted(&self.lock().1)
            .into_iter()
            .filter(|c| keys.iter().any(|k| c.draft().has_address(k)))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_contact_book_passes_contract() -> Result<(), ContactStoreError> {
        contact_book_contract(&InMemoryContactBook::default()).await
    }

    #[tokio::test]
    async fn in_memory_contact_book_passes_import_contract() -> Result<(), ContactStoreError> {
        contact_import_contract(&InMemoryContactBook::default()).await
    }
}
