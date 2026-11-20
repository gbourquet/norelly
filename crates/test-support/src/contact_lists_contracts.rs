//! Suite de contrat des listes de diffusion ([`ContactListBook`] et
//! [`ContactListDirectory`]), jouée contre le faux en mémoire
//! ([`InMemoryContactBook`]) et contre SQLite : les listes vivent dans le
//! carnet, à côté des contacts qu'elles regroupent.

use norelly_domain::address::EmailAddress;
use norelly_domain::contact_export::ContactSelection;
use norelly_domain::contact_import::{
    DuplicatePolicy, ImportPlan, ImportedCard, ImportedList, ImportedMember, import_keys,
    import_list_keys, plan_import,
};
use norelly_domain::contact_lists::{
    ContactList, ContactListDraft, ContactListFilter, ContactListId, ContactListInput,
};
use norelly_domain::contacts::{
    AddressKey, ContactId, ContactInput, EmailInput, MAX_CONTACT_NAME_CHARS,
};
use norelly_domain::ports::{
    ContactBook, ContactDirectory, ContactListBook, ContactListCursor, ContactListDirectory,
    ContactListPage, ContactListStoreError, ContactListSummary, ContactStoreError, ImportOutcome,
    MAX_CONTACT_LIST_SORT_KEY_CHARS, MAX_CONTACT_LOOKUP, MAX_PAGE_SIZE, PageSize, StoreError,
};

use crate::contact_import_contracts::FakeNfc;
use crate::contacts_contracts::{InMemoryContactBook, sample_contact};

/// Liste de test : `name` et ses membres, dans cet ordre.
#[must_use]
pub fn sample_list(name: &str, members: &[ContactId], bcc: bool) -> ContactListDraft {
    ContactListDraft::new(
        ContactListInput {
            name: name.to_owned(),
            description: String::new(),
            members: members.iter().map(|id| id.get()).collect(),
            bcc,
        },
        &FakeNfc,
    )
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
        let page = directory
            .list_page(filter, cursor, PageSize::new(limit))
            .await?;
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

/// Contrat de la taille de page (carnet vide au départ) : une demande
/// plus grande que [`MAX_PAGE_SIZE`] renvoie au plus [`MAX_PAGE_SIZE`]
/// listes, et la suite donne le reste.
pub async fn contact_list_page_size_contract<B>(book: &B) -> Result<(), ContactListStoreError>
where
    B: ContactListBook + ContactListDirectory,
{
    let all = ContactListFilter::default();
    for i in 0..=MAX_PAGE_SIZE {
        book.create_list(&sample_list(&format!("l{i:04}"), &[], false))
            .await?;
    }
    let first = book
        .list_page(&all, None, PageSize::new(MAX_PAGE_SIZE + 1))
        .await?;
    assert_eq!(first.items.len(), MAX_PAGE_SIZE as usize, "page plafonnée");
    let Some(cursor) = first.next else {
        panic!("suite attendue au-delà du plafond");
    };
    let rest = book
        .list_page(&all, Some(cursor), PageSize::new(u32::MAX))
        .await?;
    assert_eq!(names(&rest.items), vec![format!("l{MAX_PAGE_SIZE:04}")]);
    assert_eq!(rest.next, None);
    Ok(())
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
        book.list_page(&all, None, PageSize::new(10)).await?,
        ContactListPage::default(),
        "listes au départ"
    );
    let unknown = ContactListId::new(i64::MAX).unwrap_or_else(|| panic!("id"));
    assert_eq!(book.get_list(unknown).await?, None);

    // Création et relecture : nom, description, Cci, membres dans leur
    // ordre.
    let family = ContactListDraft::new(
        ContactListInput {
            name: "Famille".into(),
            description: "Les cousins aussi".into(),
            members: vec![bob.get(), alice.get()],
            bcc: true,
        },
        &FakeNfc,
    )
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
    let ghost = ContactId::new(i64::from(u32::MAX)).unwrap_or_else(|| panic!("id"));
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
    let line = book.list_page(&all, None, PageSize::new(10)).await?;
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
    let first = book.list_page(&all, None, PageSize::new(5)).await?;
    assert_eq!(first.next, None, "page complète en fin : pas de suite");
    assert_eq!(
        book.list_page(&all, None, PageSize::new(0)).await?,
        ContactListPage::default(),
        "limite nulle"
    );
    let page = book.list_page(&all, None, PageSize::new(2)).await?;
    let Some(cursor) = page.next.clone() else {
        panic!("suite attendue");
    };
    assert_eq!(page.items.last().map(|i| i.id), Some(cursor.id));
    // Une liste ajoutée avant le curseur ne décale pas la suite.
    let aaa = book.create_list(&sample_list("Aaa", &[], false)).await?;
    assert_eq!(
        names(
            &book
                .list_page(&all, Some(cursor), PageSize::new(10))
                .await?
                .items
        ),
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
    assert!(
        book.list_page(&all, None, PageSize::new(MAX_PAGE_SIZE + 1))
            .await
            .is_ok()
    );
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
        .list_page(&ContactListFilter::new("club"), None, PageSize::new(10))
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
        book.list_page(&all, None, PageSize::new(10)).await?,
        ContactListPage::default()
    );
    Ok(())
}

/// Contrat des listes proposées à la saisie
/// ([`ContactListDirectory::suggested_lists`], carnet vide au départ) : les
/// mêmes que les pages filtrées privées des listes vides, dans le même
/// ordre, coupées à la limite ; des listes vides en nombre avant les
/// suivantes ne les cachent pas.
pub async fn contact_list_suggestion_contract<B>(book: &B) -> Result<(), ContactListStoreError>
where
    B: ContactBook + ContactListBook + ContactListDirectory,
{
    let contact = |name: &'static str, address: &'static str| async move {
        book.create(&sample_contact(name, &[address]))
            .await
            .map(|c| c.id())
            .map_err(|e| ContactListStoreError::Store(StoreError(e.to_string())))
    };
    let alice = contact("Alice", "alice@example.com").await?;
    let bob = contact("Bob", "bob@example.net").await?;
    let lecture = book
        .create_list(&sample_list("Club lecture", &[alice], false))
        .await?;
    book.create_list(&sample_list("Club vide", &[], false))
        .await?;
    let bis = book
        .create_list(&sample_list("club bis", &[bob, alice], true))
        .await?;
    let voisins = book
        .create_list(&sample_list("Voisins", &[bob], false))
        .await?;
    let anciens = book
        .create_list(&sample_list("Anciens du CLUB", &[alice], false))
        .await?;
    let suggested = |text: &'static str, limit: u32| async move {
        book.suggested_lists(&ContactListFilter::new(text), PageSize::new(limit))
            .await
    };

    // Nom qui contient la saisie, sans la casse ; listes vides écartées ;
    // ordre du carnet ; résumé identique à celui des pages.
    let clubs = vec![
        ContactListSummary::from(&anciens),
        ContactListSummary::from(&bis),
        ContactListSummary::from(&lecture),
    ];
    assert_eq!(suggested("CLUB", 10).await?, clubs);
    assert_eq!(suggested("club", 2).await?, clubs[..2]);
    assert_eq!(suggested("club", 1).await?, clubs[..1]);
    assert_eq!(suggested("club", 0).await?, vec![], "limite nulle");
    assert_eq!(
        suggested("club", u32::MAX).await?,
        clubs,
        "limite plafonnée"
    );
    assert_eq!(suggested("zzz", 10).await?, vec![]);
    assert_eq!(suggested("vide", 10).await?, vec![], "liste vide proposée");
    assert_eq!(
        suggested("", 10).await?,
        vec![
            ContactListSummary::from(&anciens),
            ContactListSummary::from(&bis),
            ContactListSummary::from(&lecture),
            ContactListSummary::from(&voisins),
        ]
    );
    assert_eq!(
        suggested("club bis", 10)
            .await?
            .first()
            .map(|l| (l.members, l.bcc)),
        Some((2, true))
    );

    // Mêmes listes que les pages filtrées, privées des listes vides.
    let same_as_pages = |text: &'static str| async move {
        let filter = ContactListFilter::new(text);
        let pages = all_pages(book, &filter, 2).await?;
        for limit in [0_u32, 1, 2, 3, 10] {
            let expected: Vec<ContactListSummary> = pages
                .iter()
                .filter(|l| l.members > 0)
                .take(limit as usize)
                .cloned()
                .collect();
            assert_eq!(
                book.suggested_lists(&filter, PageSize::new(limit)).await?,
                expected,
                "{text:?}, limite {limit}"
            );
        }
        Ok::<(), StoreError>(())
    };
    for text in ["", "club", "L", "s", "zzz", "%", "_", "\\"] {
        same_as_pages(text).await?;
    }

    // Une liste dont le dernier membre est supprimé n'est plus proposée.
    book.delete_many(&[bob])
        .await
        .map_err(|e| ContactListStoreError::Store(StoreError(e.to_string())))?;
    assert_eq!(suggested("voisins", 10).await?, vec![]);
    assert_eq!(
        suggested("bis", 10).await?.first().map(|l| l.members),
        Some(1)
    );

    // Plus d'une page de listes vides entre deux listes proposées : la
    // suivante est trouvée quand même.
    for i in 0..=MAX_PAGE_SIZE {
        book.create_list(&sample_list(&format!("club {i:04}"), &[], false))
            .await?;
    }
    assert_eq!(
        names(&suggested("club", 2).await?),
        vec!["Anciens du CLUB", "club bis"]
    );
    assert_eq!(
        names(&suggested("club", 10).await?),
        vec!["Anciens du CLUB", "club bis", "Club lecture"]
    );
    Ok(())
}

/// Contrat d'une liste corrompue en base : elle seule est écartée (absente
/// des pages, des suggestions et de la relecture), les autres restent
/// lisibles et la pagination ne saute ni ne répète aucune liste valide.
///
/// `corrupt` abîme la liste donnée directement dans le stockage (une ligne
/// que le domaine refuse à la relecture) ; un faux, qui ne garde que des
/// listes valides, la fait disparaître. Carnet vide au départ.
pub async fn corrupted_list_contract<B, F, Fut>(
    book: &B,
    corrupt: F,
) -> Result<(), ContactListStoreError>
where
    B: ContactBook + ContactDirectory + ContactListBook + ContactListDirectory,
    F: Fn(ContactListId) -> Fut,
    Fut: Future<Output = Result<(), ContactListStoreError>>,
{
    let all = ContactListFilter::default();
    let alice = book
        .create(&sample_contact("Alice", &["alice@example.com"]))
        .await
        .map_err(|e| ContactListStoreError::Store(StoreError(e.to_string())))?
        .id();
    let mut lists = Vec::new();
    for name in ["Liste a", "Liste b", "Liste c", "Liste d", "Liste e"] {
        lists.push(
            book.create_list(&sample_list(name, &[alice], false))
                .await?,
        );
    }
    // Deux listes corrompues qui se suivent, puis la dernière.
    for index in [1, 2, 4] {
        corrupt(lists[index].id()).await?;
    }
    let valid = vec!["Liste a", "Liste d"];
    for limit in [1, 2, 3, 10] {
        assert_eq!(
            names(&all_pages(book, &all, limit).await?),
            valid,
            "pages de {limit}"
        );
    }
    // Suggestions : la page filtrée garde les listes valides.
    let suggested = book
        .list_page(&ContactListFilter::new("liste"), None, PageSize::MAX)
        .await?;
    assert_eq!(names(&suggested.items), valid);
    assert_eq!(suggested.next, None);
    // Listes proposées : les corrompues n'entament pas la limite.
    let liste = ContactListFilter::new("liste");
    for (limit, expected) in [(1, &valid[..1]), (2, &valid[..]), (10, &valid[..])] {
        assert_eq!(
            names(&book.suggested_lists(&liste, PageSize::new(limit)).await?),
            expected,
            "suggestions, limite {limit}"
        );
    }
    assert_eq!(book.get_list(lists[1].id()).await?, None);
    assert_eq!(book.get_list(lists[4].id()).await?, None);
    assert_eq!(book.get_list(lists[0].id()).await?, Some(lists[0].clone()));
    assert_eq!(book.get_list(lists[3].id()).await?, Some(lists[3].clone()));

    // Une liste écartée ne compte plus dans l'unicité du nom : une liste
    // de ce nom se crée (casse comprise), une autre peut le prendre ; le
    // nom d'une liste lisible reste pris.
    let recreated = sample_list("LISTE B", &[alice], true);
    assert_eq!(
        book.create_list(&recreated)
            .await
            .as_ref()
            .map(ContactList::draft),
        Ok(&recreated),
        "nom d'une liste écartée encore pris à la création"
    );
    let renamed = sample_list("liste E", &[], false);
    assert_eq!(
        book.update_list(lists[0].id(), &renamed)
            .await
            .as_ref()
            .map(ContactList::draft),
        Ok(&renamed),
        "nom d'une liste écartée encore pris au renommage"
    );
    assert_eq!(
        book.create_list(&sample_list("Liste D", &[], false)).await,
        Err(ContactListStoreError::DuplicateName("Liste D".into()))
    );
    assert_eq!(
        names(&all_pages(book, &all, 1).await?),
        vec!["LISTE B", "Liste d", "liste E"]
    );
    for index in [1, 2, 4] {
        assert_eq!(book.get_list(lists[index].id()).await?, None);
    }
    Ok(())
}

/// Contrat de la fusion de contacts ([`ContactBook::merge`]) : listes de
/// diffusion reportées, tout ou rien (carnet vide au départ).
pub async fn contact_merge_contract<B>(book: &B) -> Result<(), ContactListStoreError>
where
    B: ContactBook + ContactDirectory + ContactListBook + ContactListDirectory,
{
    let store = |e: ContactStoreError| ContactListStoreError::Store(StoreError(e.to_string()));
    let contact = |name: &'static str, addresses: &'static [&'static str]| async move {
        book.create(&sample_contact(name, addresses))
            .await
            .map_err(store)
    };
    let first_read = contact("Alice", &["a1@example.com", "a1b@example.com"]).await?;
    let second_read = contact("Alice", &["a2@example.com"]).await?;
    let third_stale = contact("Alice", &["a3@example.com"]).await?;
    let (first, second, third) = (first_read.id(), second_read.id(), third_stale.id());
    let bob = contact("Bob", &["bob@example.net"]).await?.id();
    let club = book
        .create_list(&sample_list("Club", &[bob, second, third], false))
        .await?;
    let famille = book
        .create_list(&sample_list("Famille", &[first, second], true))
        .await?;
    let voisins = book
        .create_list(&sample_list("Voisins", &[third], false))
        .await?;
    let members = |list: ContactListId| async move {
        book.get_list(list)
            .await
            .map(|l| l.map(|l| l.draft().members().to_vec()).unwrap_or_default())
    };
    let merged = sample_contact(
        "Alice Martin",
        &[
            "a1@example.com",
            "a1b@example.com",
            "a2@example.com",
            "a3@example.com",
            "a3b@example.com",
        ],
    );
    let unknown = sample_contact("Inconnue", &["x@example.com"])
        .with_id(ContactId::new(i64::from(u32::MAX)).unwrap_or_else(|| panic!("id")));
    // La troisième fiche change après avoir été lue.
    let third_read = book
        .update(
            third,
            &sample_contact("Alice", &["a3@example.com", "a3b@example.com"]),
        )
        .await
        .map_err(store)?;

    // Tout ou rien : fiche inconnue, fiche changée depuis sa lecture ou
    // adresse d'un contact étranger à la fusion, et rien ne change.
    assert_eq!(
        book.merge(
            &first_read,
            &merged,
            &[second_read.clone(), unknown.clone()]
        )
        .await,
        Err(ContactStoreError::NotFound)
    );
    assert_eq!(
        book.merge(&unknown, &merged, std::slice::from_ref(&second_read))
            .await,
        Err(ContactStoreError::NotFound)
    );
    assert_eq!(
        book.merge(
            &first_read,
            &merged,
            &[second_read.clone(), third_stale.clone()]
        )
        .await,
        Err(ContactStoreError::Conflict)
    );
    let first_stale = sample_contact("Alice", &["a1@example.com"]).with_id(first);
    assert_eq!(
        book.merge(
            &first_stale,
            &merged,
            &[second_read.clone(), third_read.clone()]
        )
        .await,
        Err(ContactStoreError::Conflict)
    );
    let stolen = sample_contact("Alice", &["a1@example.com", "BOB@example.net"]);
    assert_eq!(
        book.merge(&first_read, &stolen, std::slice::from_ref(&second_read))
            .await,
        Err(ContactStoreError::DuplicateAddress(
            EmailAddress::parse("BOB@example.net").unwrap_or_else(|e| panic!("{e}"))
        ))
    );
    for id in [first, second, third] {
        assert!(book.get(id).await?.is_some(), "fiche touchée par un refus");
    }
    assert_eq!(book.get(first).await?, Some(first_read.clone()));
    assert_eq!(book.get(third).await?, Some(third_read.clone()));
    assert_eq!(members(club.id()).await?, [bob, second, third]);
    assert_eq!(members(famille.id()).await?, [first, second]);

    // Fusion : la fiche gardée prend le contenu et les adresses des autres,
    // qui disparaissent ; elle les remplace dans leurs listes, à leur
    // place, sans y figurer deux fois.
    let kept = book
        .merge(&first_read, &merged, &[second_read, third_read])
        .await
        .map_err(store)?;
    assert_eq!(kept, merged.clone().with_id(first));
    assert_eq!(book.get(first).await?, Some(kept.clone()));
    assert_eq!(book.get(second).await?, None);
    assert_eq!(book.get(third).await?, None);
    let owner = book
        .by_addresses(&[AddressKey::parse("a3b@example.com").unwrap_or_else(|e| panic!("{e}"))])
        .await?;
    assert_eq!(owner, [kept]);
    assert_eq!(members(club.id()).await?, [bob, first]);
    assert_eq!(members(famille.id()).await?, [first]);
    assert_eq!(members(voisins.id()).await?, [first]);
    assert_eq!(
        book.get_list(famille.id()).await?.map(|l| l.draft().bcc()),
        Some(true),
        "liste modifiée au-delà de ses membres"
    );
    assert!(book.get(bob).await?.is_some());
    Ok(())
}

/// Erreur du carnet vue comme une erreur des listes (contrats).
fn store(e: ContactStoreError) -> ContactListStoreError {
    ContactListStoreError::Store(StoreError(e.to_string()))
}

/// Liste lue dans un fichier, membres désignés par leur adresse.
fn imported_list(name: &str, members: &[&str]) -> ImportedCard {
    ImportedCard::List(ImportedList {
        name: name.to_owned(),
        description: "importée".into(),
        members: members
            .iter()
            .map(|&m| ImportedMember::Address(m.to_owned()))
            .collect(),
        unresolved: 0,
        bcc: true,
    })
}

/// Fiche lue dans un fichier.
fn imported_contact(name: &str, address: &str) -> ImportedCard {
    ImportedCard::Contact(ContactInput {
        display_name: name.to_owned(),
        emails: vec![EmailInput {
            address: address.to_owned(),
            ..EmailInput::default()
        }],
        ..ContactInput::default()
    })
}

/// Plan d'import de `cards` contre le carnet `book` tel qu'il est.
async fn plan_against<B>(
    book: &B,
    cards: &[ImportedCard],
    policy: DuplicatePolicy,
) -> Result<ImportPlan, ContactListStoreError>
where
    B: ContactDirectory + ContactListDirectory,
{
    let keys = import_keys(cards, &FakeNfc);
    let existing = book.by_addresses(&keys).await?;
    let lists = book.lists_named(&import_list_keys(cards, &FakeNfc)).await?;
    Ok(plan_import(&existing, &lists, cards, policy, &FakeNfc))
}

async fn write<B: ContactBook>(
    book: &B,
    plan: &ImportPlan,
) -> Result<ImportOutcome, ContactStoreError> {
    book.import(&plan.create, &plan.complete, &plan.lists, &[])
        .await
}

/// Contrat de l'import des listes ([`ContactBook::import`],
/// [`ContactListDirectory::lists_named`], [`ContactListDirectory::all_lists`]),
/// carnet vide au départ : listes créées avec leurs membres (carnet et
/// fiches créées), tout ou rien ; une liste existante n'est jamais touchée.
pub async fn contact_list_import_contract<B>(book: &B) -> Result<(), ContactListStoreError>
where
    B: ContactBook + ContactDirectory + ContactListBook + ContactListDirectory,
{
    let alice = book
        .create(&sample_contact("Alice", &["alice@example.com"]))
        .await
        .map_err(store)?;
    assert_eq!(book.all_lists(10).await?, Vec::new());
    assert_eq!(book.lists_named(&["club".into()]).await?, Vec::new());

    // Création : un membre du carnet, une fiche créée par l'import.
    let cards = [
        imported_list("Club", &["alice@example.com", "bob@example.net"]),
        imported_contact("Bob", "bob@example.net"),
    ];
    let plan = plan_against(book, &cards, DuplicatePolicy::Skip).await?;
    assert_eq!(
        write(book, &plan).await.map_err(store)?,
        ImportOutcome {
            created: 1,
            completed: 0,
            lists_created: 1,
        }
    );
    let bob = book
        .by_addresses(&[AddressKey::parse("bob@example.net").unwrap_or_else(|e| panic!("{e}"))])
        .await?
        .pop()
        .unwrap_or_else(|| panic!("Bob créé"));
    let [club] = &book.lists_named(&["club".into()]).await?[..] else {
        panic!("liste Club");
    };
    assert_eq!(club.draft().name(), "Club");
    assert_eq!(club.draft().description(), Some("importée"));
    assert!(club.draft().bcc());
    assert_eq!(club.draft().members(), [alice.id(), bob.id()]);
    assert_eq!(book.get_list(club.id()).await?.as_ref(), Some(club));

    // Une liste du même nom, même en complétant : la liste du carnet ne
    // change pas.
    let cards = [
        imported_contact("Carol", "carol@example.org"),
        imported_list("CLUB", &["carol@example.org"]),
    ];
    let again = plan_against(book, &cards, DuplicatePolicy::Complete).await?;
    assert_eq!(write(book, &again).await.map_err(store)?.lists_created, 0);
    assert_eq!(book.get_list(club.id()).await?.as_ref(), Some(club));

    let contacts_before = book.contacts(&ContactSelection::All, 100).await?;
    // Nom pris entre le plan et l'écriture : rien n'est écrit, fiches
    // comprises.
    let cards = [
        imported_contact("Eve", "eve@example.org"),
        imported_list("Famille", &["eve@example.org"]),
    ];
    let late = plan_against(book, &cards, DuplicatePolicy::Skip).await?;
    let famille = book
        .create_list(&sample_list("famille", &[alice.id()], false))
        .await?;
    assert_eq!(write(book, &late).await, Err(ContactStoreError::Conflict));
    // Membre du carnet supprimé entre le plan et l'écriture : de même.
    let cards = [imported_list("Amis", &["alice@example.com"])];
    let gone = plan_against(book, &cards, DuplicatePolicy::Skip).await?;
    book.delete_many(&[alice.id()]).await.map_err(store)?;
    assert_eq!(write(book, &gone).await, Err(ContactStoreError::NotFound));
    let contacts_after = book.contacts(&ContactSelection::All, 100).await?;
    assert_eq!(
        contacts_after,
        contacts_before
            .into_iter()
            .filter(|c| c.id() != alice.id())
            .collect::<Vec<_>>(),
        "fiches écrites malgré le refus"
    );
    let club = book
        .get_list(club.id())
        .await?
        .unwrap_or_else(|| panic!("Club relue"));
    let famille = book
        .get_list(famille.id())
        .await?
        .unwrap_or_else(|| panic!("Famille relue"));
    assert_eq!(
        book.all_lists(10).await?,
        vec![club.clone(), famille.clone()]
    );

    // Toutes les listes, dans l'ordre du nom, bornées.
    let zeta = book.create_list(&sample_list("Zêta", &[], false)).await?;
    let aa = book.create_list(&sample_list("aa", &[], false)).await?;
    assert_eq!(
        book.all_lists(10).await?,
        vec![aa.clone(), club.clone(), famille, zeta.clone()]
    );
    assert_eq!(book.all_lists(2).await?, vec![aa.clone(), club]);
    assert_eq!(
        book.lists_named(&["zêta".into(), "aa".into(), "inconnue".into()])
            .await?,
        vec![aa, zeta]
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
        limit: PageSize,
    ) -> Result<ContactListPage, StoreError> {
        let limit = limit.get() as usize;
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

    async fn suggested_lists(
        &self,
        filter: &ContactListFilter,
        limit: PageSize,
    ) -> Result<Vec<ContactListSummary>, StoreError> {
        Ok(sorted(&self.lock_lists().1)
            .iter()
            .filter(|l| filter.matches(l.draft()) && !l.draft().members().is_empty())
            .take(limit.get() as usize)
            .map(ContactListSummary::from)
            .collect())
    }

    async fn get_list(&self, id: ContactListId) -> Result<Option<ContactList>, StoreError> {
        Ok(self.lock_lists().1.iter().find(|l| l.id() == id).cloned())
    }

    async fn lists_named(&self, keys: &[String]) -> Result<Vec<ContactList>, StoreError> {
        let keys = &keys[..keys.len().min(MAX_CONTACT_LOOKUP)];
        Ok(sorted(&self.lock_lists().1)
            .into_iter()
            .filter(|l| keys.contains(&l.draft().name_key()))
            .collect())
    }

    async fn all_lists(&self, limit: usize) -> Result<Vec<ContactList>, StoreError> {
        Ok(sorted(&self.lock_lists().1)
            .into_iter()
            .take(limit)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_contact_lists_pass_contract() -> Result<(), ContactListStoreError> {
        contact_list_contract(&InMemoryContactBook::default()).await
    }

    #[tokio::test]
    async fn in_memory_corrupted_list_passes_contract() -> Result<(), ContactListStoreError> {
        let book = InMemoryContactBook::default();
        corrupted_list_contract(&book, |id| {
            let book = &book;
            async move { book.delete_lists(&[id]).await.map(|_| ()) }
        })
        .await
    }

    #[tokio::test]
    async fn in_memory_list_import_passes_contract() -> Result<(), ContactListStoreError> {
        contact_list_import_contract(&InMemoryContactBook::default()).await
    }

    #[tokio::test]
    async fn in_memory_contact_merge_passes_contract() -> Result<(), ContactListStoreError> {
        contact_merge_contract(&InMemoryContactBook::default()).await
    }

    #[tokio::test]
    async fn in_memory_list_suggestions_pass_contract() -> Result<(), ContactListStoreError> {
        contact_list_suggestion_contract(&InMemoryContactBook::default()).await
    }

    #[tokio::test]
    async fn in_memory_contact_lists_pass_page_size_contract() -> Result<(), ContactListStoreError>
    {
        contact_list_page_size_contract(&InMemoryContactBook::default()).await
    }
}
