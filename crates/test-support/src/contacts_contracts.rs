//! Suite de contrat du carnet d'adresses ([`ContactBook`] et
//! [`ContactDirectory`]), jouée contre le faux en mémoire
//! ([`InMemoryContactBook`]) et contre SQLite.

use std::sync::{Mutex, MutexGuard, PoisonError};

use norelly_domain::address::EmailAddress;
use norelly_domain::contact_duplicates::{DismissedPair, MAX_DISMISSED_PAIRS};
use norelly_domain::contact_export::ContactSelection;
use norelly_domain::contact_import::ContactCompletion;
use norelly_domain::contact_lists::{ContactList, ContactListDraft, ContactListInput};
use norelly_domain::contacts::{
    AddressKey, Contact, ContactDraft, ContactFilter, ContactId, ContactInput, EmailInput,
    PhoneInput,
};
use norelly_domain::ports::{
    ContactBook, ContactCursor, ContactDirectory, ContactListItem, ContactPage, ContactStoreError,
    DuplicateDismissals, ImportOutcome, MAX_CONTACT_LOOKUP, MAX_PAGE_SIZE, PageSize, StoreError,
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
        let page = directory.page(filter, cursor, PageSize::new(limit)).await?;
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
        book.page(&all, None, PageSize::new(10)).await?,
        ContactPage::default(),
        "carnet non vide au départ"
    );
    let unknown = ContactId::new(i64::from(u32::MAX)).unwrap_or_else(|| panic!("id"));
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
    let first = book.page(&all, None, PageSize::new(5)).await?;
    assert_eq!(first.items.len(), 5);
    assert_eq!(
        first.next, None,
        "page complète en fin de liste : pas de suite"
    );
    assert_eq!(
        book.page(&all, None, PageSize::new(0)).await?,
        ContactPage::default(),
        "limite nulle"
    );
    let page = book.page(&all, None, PageSize::new(2)).await?;
    let Some(cursor) = page.next.clone() else {
        panic!("suite attendue");
    };
    assert_eq!(page.items.last().map(|i| i.id), Some(cursor.id));
    // Ligne de liste : nom, adresse préférée, organisation, favori.
    let line = book
        .page(&ContactFilter::new("acme", false), None, PageSize::new(10))
        .await?;
    assert!(
        line.items.is_empty(),
        "organisation retirée par la modification"
    );
    let favorite = book.update(alice.id(), &full_contact()).await?;
    let line = book
        .page(&ContactFilter::new("acme", false), None, PageSize::new(10))
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
            .page(&ContactFilter::new(text, false), None, PageSize::new(10))
            .await?;
        assert_eq!(
            found.items.iter().map(|i| i.id).collect::<Vec<_>>(),
            vec![favorite.id()],
            "{text}"
        );
        let favorites = book
            .page(&ContactFilter::new(text, true), None, PageSize::new(10))
            .await?;
        assert_eq!(favorites.items.len(), 1, "{text} parmi les favoris");
    }

    // Un contact ajouté avant le curseur ne décale pas la suite.
    let page = book.page(&all, None, PageSize::new(2)).await?;
    let Some(cursor) = page.next else {
        panic!("suite attendue");
    };
    let aaron = book
        .create(&sample_contact("Aaron", &["aaron@example.com"]))
        .await?;
    let rest = book.page(&all, Some(cursor), PageSize::new(10)).await?;
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
    assert_eq!(
        book.page(&all, None, PageSize::new(10)).await?,
        ContactPage::default()
    );
    Ok(())
}

/// Contrat de la lecture en bloc des fiches (export), carnet vide au
/// départ : tout le carnet ou une sélection, fiches complètes dans l'ordre
/// de la liste, inconnues omises, au plus `limit`.
pub async fn contact_selection_contract<B: ContactBook + ContactDirectory>(
    book: &B,
) -> Result<(), ContactStoreError> {
    assert!(book.contacts(&ContactSelection::All, 10).await?.is_empty());
    let bob = book
        .create(&sample_contact("Bob", &["bob@example.net"]))
        .await?;
    let alice = book.create(&full_contact()).await?;
    let carole = book
        .create(&sample_contact("Carole", &["carole@example.org"]))
        .await?;

    let all = book.contacts(&ContactSelection::All, 10).await?;
    assert_eq!(all, vec![alice.clone(), bob.clone(), carole.clone()]);
    assert_eq!(
        book.contacts(&ContactSelection::All, 2).await?,
        vec![alice.clone(), bob.clone()]
    );
    assert!(book.contacts(&ContactSelection::All, 0).await?.is_empty());

    let unknown = ContactId::new(carole.id().get() + 1000).unwrap_or_else(|| panic!("id"));
    let chosen = ContactSelection::Contacts(vec![carole.id(), unknown, alice.id()]);
    assert_eq!(
        book.contacts(&chosen, 10).await?,
        vec![alice.clone(), carole.clone()]
    );
    assert_eq!(book.contacts(&chosen, 1).await?, vec![alice]);
    Ok(())
}

/// Contrat de la taille de page (carnet vide au départ) : une demande
/// plus grande que [`MAX_PAGE_SIZE`] renvoie au plus [`MAX_PAGE_SIZE`]
/// contacts, et la suite donne le reste.
pub async fn contact_page_size_contract<B: ContactBook + ContactDirectory>(
    book: &B,
) -> Result<(), ContactStoreError> {
    let all = ContactFilter::default();
    let drafts: Vec<ContactDraft> = (0..=MAX_PAGE_SIZE)
        .map(|i| sample_contact(&format!("c{i:04}"), &[&format!("c{i}@example.com")]))
        .collect();
    book.import(&drafts, &[]).await?;
    let first = book
        .page(&all, None, PageSize::new(MAX_PAGE_SIZE + 1))
        .await?;
    assert_eq!(first.items.len(), MAX_PAGE_SIZE as usize, "page plafonnée");
    let Some(cursor) = first.next else {
        panic!("suite attendue au-delà du plafond");
    };
    let rest = book
        .page(&all, Some(cursor), PageSize::new(u32::MAX))
        .await?;
    assert_eq!(names(&rest.items), vec![format!("c{MAX_PAGE_SIZE:04}")]);
    assert_eq!(rest.next, None);
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
    assert_eq!(
        book.page(&all, None, PageSize::new(10)).await?,
        ContactPage::default()
    );

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
    let bob = book
        .get(listed[1].id)
        .await?
        .unwrap_or_else(|| panic!("Bob relu"));
    let bob_id = bob.id();

    // Création et complément ensemble.
    let completed =
        sample_contact("Alice", &["alice@example.com", "alice@work.example"]).with_id(alice.id());
    let outcome = book
        .import(
            &[sample_contact("Carole", &["carole@example.org"])],
            &[ContactCompletion::new(
                alice.clone(),
                completed.draft().clone(),
            )],
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
    let stolen = ContactCompletion::new(
        completed.clone(),
        sample_contact("Alice", &["alice@example.com", "BOB@example.net"]),
    );
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
    let zoe = sample_contact("Zoé", &["zoe@example.org"]);
    let unknown = ContactCompletion::new(
        zoe.clone()
            .with_id(ContactId::new(i64::from(u32::MAX)).unwrap_or_else(|| panic!("id"))),
        zoe,
    );
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
    let again = ContactCompletion::new(
        completed.clone(),
        sample_contact("Alice", &["alice@example.com", "a3@example.com"]),
    );
    assert_eq!(
        book.import(&[sample_contact("Autre", &["A3@example.com"])], &[again])
            .await,
        Err(ContactStoreError::DuplicateAddress(address(
            "A3@example.com"
        )))
    );
    // Un contact modifié depuis la lecture qui a servi au complément
    // n'est pas écrasé : tout l'import est annulé.
    let stale = ContactCompletion::new(
        alice.clone(),
        sample_contact("Alice", &["alice@example.com", "a4@example.com"]),
    );
    assert_eq!(
        book.import(&[sample_contact("Denis", &["denis@example.org"])], &[stale])
            .await,
        Err(ContactStoreError::Conflict)
    );
    // Les fiches lues sont toutes vérifiées avant toute écriture : un
    // contact modifié l'emporte sur l'adresse prise d'un complément qui le
    // précède.
    let takes_carole = ContactCompletion::new(
        completed.clone(),
        sample_contact("Alice", &["alice@example.com", "carole@example.org"]),
    );
    let stale_bob = ContactCompletion::new(
        sample_contact("Bob ancien", &["bob@example.net"]).with_id(bob_id),
        sample_contact("Bob", &["bob@example.net", "b2@example.net"]),
    );
    assert_eq!(
        book.import(&[], &[takes_carole, stale_bob]).await,
        Err(ContactStoreError::Conflict)
    );
    assert_eq!(book.get(alice.id()).await?, Some(completed));
    // Aucun complément n'est écrit, celui de Bob compris.
    assert_eq!(book.get(bob_id).await?, Some(bob));
    assert_eq!(
        names(&all_pages(book, &all, 10).await?),
        vec!["Alice", "Bob", "Carole"]
    );
    Ok(())
}

/// Page qui doit être lue malgré les fiches invalides.
async fn readable_page<D: ContactDirectory>(
    directory: &D,
    filter: &ContactFilter,
    cursor: Option<ContactCursor>,
    limit: u32,
) -> ContactPage {
    directory
        .page(filter, cursor, PageSize::new(limit))
        .await
        .unwrap_or_else(|e| panic!("une fiche invalide fait échouer la page : {e}"))
}

/// Contrat d'une fiche relue invalide (carnet vide au départ) : `corrupt`
/// rend illisible en base la fiche désignée. Elle seule est écartée de la
/// liste (pages pleines, pagination menée jusqu'au bout) et des fiches
/// cherchées par adresse ; la lecture de cette fiche reste une erreur.
pub async fn contact_corruption_contract<B, F, Fut>(
    book: &B,
    corrupt: F,
) -> Result<(), ContactStoreError>
where
    B: ContactBook + ContactDirectory,
    F: Fn(ContactId) -> Fut,
    Fut: Future<Output = Result<(), StoreError>>,
{
    let all = ContactFilter::default();
    let mut ids = Vec::new();
    for name in ["Alice", "Bob", "Carole", "David", "Zoe"] {
        let address = format!("{}@example.com", name.to_lowercase());
        ids.push(book.create(&sample_contact(name, &[&address])).await?.id());
    }
    // Bob, au milieu de la liste, et Zoe, la dernière fiche.
    corrupt(ids[1]).await?;
    corrupt(ids[4]).await?;

    // Page pleine malgré la fiche écartée, et suite annoncée.
    let first = readable_page(book, &all, None, 2).await;
    assert_eq!(names(&first.items), vec!["Alice", "Carole"]);
    assert!(first.next.is_some(), "suite non annoncée");
    // La dernière fiche, illisible, n'annonce pas une page vide.
    let second = readable_page(book, &all, first.next, 2).await;
    assert_eq!(names(&second.items), vec!["David"]);
    assert_eq!(second.next, None, "suite annoncée sans fiche valide");
    for limit in 1..=5 {
        let items = all_pages(book, &all, limit)
            .await
            .unwrap_or_else(|e| panic!("pagination interrompue ({limit}) : {e}"));
        assert_eq!(names(&items), vec!["Alice", "Carole", "David"], "{limit}");
    }
    // Filtre : seule la fiche invalide correspondait.
    let bob = readable_page(book, &ContactFilter::new("bob", false), None, 10).await;
    assert_eq!(bob, ContactPage::default());

    // Fiches par adresse : la fiche invalide est écartée, pas les autres.
    let keys = [
        key("alice@example.com"),
        key("bob@example.com"),
        key("david@example.com"),
    ];
    let found = book
        .by_addresses(&keys)
        .await
        .unwrap_or_else(|e| panic!("une fiche invalide fait échouer la recherche : {e}"));
    assert_eq!(
        found.iter().map(Contact::id).collect::<Vec<_>>(),
        vec![ids[0], ids[3]]
    );

    assert!(book.get(ids[1]).await.is_err(), "fiche invalide relue");
    let alice = book
        .get(ids[0])
        .await?
        .unwrap_or_else(|| panic!("fiche valide perdue"));
    // Export et recherche des doublons : la fiche invalide est écartée,
    // les autres restent lues.
    let bob = ContactSelection::Contacts(vec![ids[1], ids[3]]);
    let exported = book
        .contacts(&bob, 10)
        .await
        .unwrap_or_else(|e| panic!("une fiche invalide fait échouer l'export : {e}"));
    assert_eq!(
        exported.iter().map(Contact::id).collect::<Vec<_>>(),
        vec![ids[3]]
    );
    let everyone = book
        .contacts(&ContactSelection::All, 10)
        .await
        .unwrap_or_else(|e| panic!("une fiche invalide fait échouer l'export : {e}"));
    assert_eq!(
        everyone.iter().map(Contact::id).collect::<Vec<_>>(),
        vec![ids[0], ids[2], ids[3]]
    );
    // La fusion, qui écrit, refuse une fiche invalide.
    let stale = sample_contact("Bob", &["bob@example.com"]).with_id(ids[1]);
    assert!(
        book.merge(&alice, alice.draft(), &[stale]).await.is_err(),
        "fiche invalide fusionnée"
    );
    assert_eq!(book.get(ids[0]).await?, Some(alice));
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
    /// Fiches rendues illisibles par [`Self::corrupt`].
    unreadable: Mutex<Vec<ContactId>>,
    /// Paires de doublons écartées, de la plus ancienne à la plus récente
    /// ([`crate::contact_dismissals_contracts`]).
    dismissed: Mutex<Vec<DismissedPair>>,
}

impl InMemoryContactBook {
    /// Rend la fiche `id` illisible, comme une ligne invalide en base
    /// ([`contact_corruption_contract`]).
    pub fn corrupt(&self, id: ContactId) {
        self.unreadable
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(id);
    }

    fn is_unreadable(&self, id: ContactId) -> bool {
        self.unreadable
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&id)
    }

    /// Fiches lisibles, dans l'ordre de la liste : une fiche illisible est
    /// écartée, comme par SQLite.
    fn readable(&self) -> Vec<Contact> {
        let mut contacts = sorted(&self.lock().1);
        contacts.retain(|c| !self.is_unreadable(c.id()));
        contacts
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, (i64, Vec<Contact>)> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_dismissed(&self) -> MutexGuard<'_, Vec<DismissedPair>> {
        self.dismissed.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Oublie les paires écartées des fiches `ids`, modifiées ou
    /// supprimées.
    fn forget_dismissed(&self, ids: &[ContactId]) {
        self.lock_dismissed()
            .retain(|p| !ids.contains(&p.low()) && !ids.contains(&p.high()));
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
        self.forget_dismissed(&[id]);
        Ok(contact)
    }

    async fn delete(&self, id: ContactId) -> Result<bool, ContactStoreError> {
        Ok(self.delete_many(&[id]).await? == 1)
    }

    async fn import(
        &self,
        create: &[ContactDraft],
        complete: &[ContactCompletion],
    ) -> Result<ImportOutcome, ContactStoreError> {
        let mut state = self.lock();
        // Écrit sur une copie, gardée seulement si tout passe.
        let (mut last, mut contacts) = state.clone();
        // Chaque contact à compléter tel qu'il a été lu pour calculer le
        // plan, vérifié avant toute écriture.
        for contact in complete {
            match state.1.iter().find(|c| c.id() == contact.id()) {
                None => return Err(ContactStoreError::NotFound),
                Some(c) if c != contact.read() => return Err(ContactStoreError::Conflict),
                Some(_) => {}
            }
        }
        for contact in complete {
            let Some(index) = contacts.iter().position(|c| c.id() == contact.id()) else {
                return Err(ContactStoreError::NotFound);
            };
            if let Some(address) = taken(&contacts, contact.draft(), Some(contact.id())) {
                return Err(ContactStoreError::DuplicateAddress(address));
            }
            contacts[index] = contact.draft().clone().with_id(contact.id());
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
        let completed: Vec<ContactId> = complete.iter().map(ContactCompletion::id).collect();
        self.forget_dismissed(&completed);
        Ok(ImportOutcome {
            created: create.len(),
            completed: complete.len(),
        })
    }

    async fn merge(
        &self,
        keep: &Contact,
        draft: &ContactDraft,
        absorbed: &[Contact],
    ) -> Result<Contact, ContactStoreError> {
        let read: Vec<Contact> = std::iter::once(keep).chain(absorbed).cloned().collect();
        let keep = keep.id();
        let absorbed: Vec<ContactId> = absorbed.iter().map(Contact::id).collect();
        if let Some(id) = std::iter::once(&keep)
            .chain(&absorbed)
            .find(|id| self.is_unreadable(**id))
        {
            return Err(StoreError(format!("contact {id} corrompu")).into());
        }
        let mut state = self.lock();
        let known = |id: &ContactId| state.1.iter().any(|c| c.id() == *id);
        if !known(&keep) || !absorbed.iter().all(known) {
            return Err(ContactStoreError::NotFound);
        }
        // Chaque fiche telle qu'elle a été lue pour calculer la fusion.
        if read
            .iter()
            .any(|r| state.1.iter().find(|c| c.id() == r.id()) != Some(r))
        {
            return Err(ContactStoreError::Conflict);
        }
        let others: Vec<Contact> = state
            .1
            .iter()
            .filter(|c| c.id() != keep && !absorbed.contains(&c.id()))
            .cloned()
            .collect();
        if let Some(address) = taken(&others, draft, None) {
            return Err(ContactStoreError::DuplicateAddress(address));
        }
        let mut lists = self.lock_lists();
        for list in &mut lists.1 {
            let mut members: Vec<ContactId> = list.draft().members().to_vec();
            for gone in &absorbed {
                let Some(at) = members.iter().position(|m| m == gone) else {
                    continue;
                };
                if members.contains(&keep) {
                    members.remove(at);
                } else {
                    members[at] = keep;
                }
            }
            if members != list.draft().members() {
                let draft = ContactListDraft::new(ContactListInput {
                    members: members.iter().map(|m| m.get()).collect(),
                    ..list.draft().to_input()
                })
                .map_err(|e| ContactStoreError::Corrupted(e.to_string()))?;
                *list = draft.with_id(list.id());
            }
        }
        state.1.retain(|c| !absorbed.contains(&c.id()));
        let contact = draft.clone().with_id(keep);
        if let Some(slot) = state.1.iter_mut().find(|c| c.id() == keep) {
            *slot = contact.clone();
        }
        let written: Vec<ContactId> = std::iter::once(keep).chain(absorbed).collect();
        self.forget_dismissed(&written);
        Ok(contact)
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
        self.forget_dismissed(ids);
        Ok(before - state.1.len())
    }
}

impl DuplicateDismissals for InMemoryContactBook {
    async fn dismiss(&self, pairs: &[DismissedPair]) -> Result<(), StoreError> {
        let state = self.lock();
        let known = |id: ContactId| state.1.iter().any(|c| c.id() == id);
        let mut dismissed = self.lock_dismissed();
        for p in pairs {
            if known(p.low()) && known(p.high()) {
                dismissed.retain(|d| d != p);
                dismissed.push(*p);
            }
        }
        let excess = dismissed.len().saturating_sub(MAX_DISMISSED_PAIRS);
        dismissed.drain(..excess);
        Ok(())
    }

    async fn dismissed(&self) -> Result<Vec<DismissedPair>, StoreError> {
        Ok(self.lock_dismissed().clone())
    }
}

impl ContactDirectory for InMemoryContactBook {
    async fn page(
        &self,
        filter: &ContactFilter,
        cursor: Option<ContactCursor>,
        limit: PageSize,
    ) -> Result<ContactPage, StoreError> {
        let limit = limit.get() as usize;
        let after = cursor.map(SortKey);
        let mut rest = self
            .readable()
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
        if self.is_unreadable(id) {
            return Err(StoreError(format!("contact {id} corrompu")));
        }
        Ok(self.lock().1.iter().find(|c| c.id() == id).cloned())
    }

    async fn by_addresses(&self, keys: &[AddressKey]) -> Result<Vec<Contact>, StoreError> {
        let keys = &keys[..keys.len().min(MAX_CONTACT_LOOKUP)];
        Ok(self
            .readable()
            .into_iter()
            .filter(|c| keys.iter().any(|k| c.draft().has_address(k)))
            .collect())
    }

    async fn contacts(
        &self,
        selection: &ContactSelection,
        limit: usize,
    ) -> Result<Vec<Contact>, StoreError> {
        Ok(self
            .readable()
            .into_iter()
            .filter(|c| match selection {
                ContactSelection::All => true,
                ContactSelection::Contacts(ids) => ids.contains(&c.id()),
            })
            .take(limit)
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

    #[tokio::test]
    async fn in_memory_contact_book_passes_selection_contract() -> Result<(), ContactStoreError> {
        contact_selection_contract(&InMemoryContactBook::default()).await
    }

    #[tokio::test]
    async fn in_memory_contact_book_passes_page_size_contract() -> Result<(), ContactStoreError> {
        contact_page_size_contract(&InMemoryContactBook::default()).await
    }

    #[tokio::test]
    async fn in_memory_contact_book_passes_corruption_contract() -> Result<(), ContactStoreError> {
        let book = InMemoryContactBook::default();
        contact_corruption_contract(&book, |id| {
            book.corrupt(id);
            async { Ok(()) }
        })
        .await
    }
}
