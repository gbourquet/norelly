//! Tests de la préparation de l'export, sur les faux en mémoire.

use norelly_domain::contact_export::ContactSelection;
use norelly_domain::contact_lists::{ContactList, ContactListFilter, ContactListId};
use norelly_domain::contacts::{Contact, ContactFilter, ContactId};
use norelly_domain::ports::{
    ContactBook, ContactCursor, ContactListBook, ContactListCursor, ContactListPage, ContactPage,
    MAX_CONTACT_LOOKUP, PageSize, StoreError,
};
use norelly_test_support::contact_lists_contracts::sample_list;
use norelly_test_support::contact_photo_contracts::InMemoryPhotoStore;
use norelly_test_support::contacts_contracts::{InMemoryContactBook, full_contact, sample_contact};

use super::*;

fn names(cards: &[ExportCard]) -> Vec<&str> {
    cards
        .iter()
        .map(|c| c.full_name.as_deref().unwrap_or_default())
        .collect()
}

#[tokio::test]
async fn the_whole_book_is_prepared_in_list_order() -> Result<(), Box<dyn std::error::Error>> {
    let book = InMemoryContactBook::default();
    book.create(&sample_contact("Bob", &["bob@example.net"]))
        .await?;
    let alice = book.create(&full_contact()).await?;
    let prepared = PrepareContactExport {
        directory: &book,
        photos: &norelly_test_support::contact_photo_contracts::InMemoryPhotoStore::default(),
    }
    .execute(&ContactSelection::All, ExportFormat::VCard4)
    .await?;
    assert_eq!(names(&prepared.cards), ["Alice Martin", "Bob"]);
    // Préparées par la règle du domaine pour le format choisi.
    assert_eq!(
        prepared.cards[0],
        export_card(alice.draft(), ExportFormat::VCard4)
    );
    assert!(prepared.lists.is_empty());
    Ok(())
}

#[tokio::test]
async fn lists_go_with_the_whole_book_only() -> Result<(), Box<dyn std::error::Error>> {
    let book = InMemoryContactBook::default();
    let bob = book
        .create(&sample_contact("Bob", &["bob@example.net"]))
        .await?;
    let alice = book.create(&full_contact()).await?;
    book.create_list(&sample_list("Famille", &[bob.id(), alice.id()], true))
        .await?;
    let prepared = PrepareContactExport {
        directory: &book,
        photos: &norelly_test_support::contact_photo_contracts::InMemoryPhotoStore::default(),
    }
    .execute(&ContactSelection::All, ExportFormat::Ldif)
    .await?;
    // Membres désignés par leur rang dans les fiches (Alice, puis Bob).
    assert_eq!(
        prepared.lists,
        [ExportList {
            name: "Famille".into(),
            description: None,
            bcc: true,
            members: vec![1, 0],
        }]
    );
    let csv = PrepareContactExport {
        directory: &book,
        photos: &norelly_test_support::contact_photo_contracts::InMemoryPhotoStore::default(),
    }
    .execute(&ContactSelection::All, ExportFormat::GoogleCsv)
    .await?;
    assert!(csv.lists.is_empty(), "le CSV ne porte pas de listes");
    let some = PrepareContactExport {
        directory: &book,
        photos: &norelly_test_support::contact_photo_contracts::InMemoryPhotoStore::default(),
    }
    .execute(
        &ContactSelection::Contacts(vec![bob.id()]),
        ExportFormat::VCard4,
    )
    .await?;
    assert!(some.lists.is_empty(), "sélection : pas de listes");
    Ok(())
}

#[tokio::test]
async fn a_selection_is_prepared_without_unknown_contacts() -> Result<(), Box<dyn std::error::Error>>
{
    let book = InMemoryContactBook::default();
    let bob = book
        .create(&sample_contact("Bob", &["bob@example.net"]))
        .await?;
    book.create(&sample_contact("Carole", &["carole@example.org"]))
        .await?;
    let gone = ContactId::new(999).ok_or("id")?;
    let prepared = PrepareContactExport {
        directory: &book,
        photos: &norelly_test_support::contact_photo_contracts::InMemoryPhotoStore::default(),
    }
    .execute(
        &ContactSelection::Contacts(vec![gone, bob.id()]),
        ExportFormat::GoogleCsv,
    )
    .await?;
    assert_eq!(prepared.cards.len(), 1);
    assert_eq!(prepared.cards[0].full_name.as_deref(), Some("Bob"));
    Ok(())
}

#[tokio::test]
async fn nothing_to_export_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let book = InMemoryContactBook::default();
    let prepare = PrepareContactExport {
        directory: &book,
        photos: &InMemoryPhotoStore::default(),
    };
    assert_eq!(
        prepare
            .execute(&ContactSelection::All, ExportFormat::Ldif)
            .await,
        Err(ExportFailure::Empty)
    );
    let gone = ContactId::new(7).ok_or("id")?;
    assert_eq!(
        prepare
            .execute(&ContactSelection::Contacts(vec![gone]), ExportFormat::Ldif)
            .await,
        Err(ExportFailure::Empty)
    );
    Ok(())
}

/// Carnet qui rend au plus `contacts` fiches et `lists` listes, autant
/// qu'on lui en demande dans ces limites.
struct CrowdedDirectory {
    contact: Contact,
    contacts: usize,
    list: ContactList,
    lists: usize,
}

impl ContactDirectory for CrowdedDirectory {
    async fn count(&self) -> Result<usize, StoreError> {
        Ok(self.contacts)
    }

    async fn photo_ids(
        &self,
    ) -> Result<std::collections::BTreeSet<norelly_domain::ports::BlobId>, StoreError> {
        Ok(std::collections::BTreeSet::new())
    }

    async fn page(
        &self,
        _: &ContactFilter,
        _: Option<ContactCursor>,
        _: PageSize,
    ) -> Result<ContactPage, StoreError> {
        Ok(ContactPage::default())
    }

    async fn get(&self, _: ContactId) -> Result<Option<Contact>, StoreError> {
        Ok(None)
    }

    async fn by_addresses(
        &self,
        _: &[norelly_domain::contacts::AddressKey],
    ) -> Result<Vec<Contact>, StoreError> {
        let _ = MAX_CONTACT_LOOKUP;
        Ok(Vec::new())
    }

    async fn contacts(
        &self,
        _: &ContactSelection,
        limit: usize,
    ) -> Result<Vec<Contact>, StoreError> {
        Ok(vec![self.contact.clone(); limit.min(self.contacts)])
    }
}

impl ContactListDirectory for CrowdedDirectory {
    async fn list_page(
        &self,
        _: &ContactListFilter,
        _: Option<ContactListCursor>,
        _: PageSize,
    ) -> Result<ContactListPage, StoreError> {
        Ok(ContactListPage::default())
    }

    async fn get_list(&self, _: ContactListId) -> Result<Option<ContactList>, StoreError> {
        Ok(None)
    }

    async fn lists_named(&self, _: &[String]) -> Result<Vec<ContactList>, StoreError> {
        Ok(Vec::new())
    }

    async fn all_lists(&self, limit: usize) -> Result<Vec<ContactList>, StoreError> {
        Ok(vec![self.list.clone(); limit.min(self.lists)])
    }
}

fn crowded(contacts: usize, lists: usize) -> Result<CrowdedDirectory, Box<dyn std::error::Error>> {
    let id = ContactId::new(1).ok_or("id")?;
    Ok(CrowdedDirectory {
        contact: sample_contact("Zoé", &["zoe@example.com"]).with_id(id),
        contacts,
        list: sample_list("L", &[id], false).with_id(ContactListId::new(1).ok_or("id")?),
        lists,
    })
}

#[tokio::test]
async fn a_book_larger_than_an_import_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let directory = crowded(usize::MAX, 0)?;
    let prepare = PrepareContactExport {
        directory: &directory,
        photos: &norelly_test_support::contact_photo_contracts::InMemoryPhotoStore::default(),
    };
    assert_eq!(
        prepare
            .execute(&ContactSelection::All, ExportFormat::VCard3)
            .await,
        Err(ExportFailure::TooMany)
    );
    Ok(())
}

#[tokio::test]
async fn lists_count_with_the_cards_in_the_bound() -> Result<(), Box<dyn std::error::Error>> {
    let full = crowded(MAX_EXPORT_CONTACTS - 2, 3)?;
    assert_eq!(
        PrepareContactExport {
            directory: &full,
            photos: &norelly_test_support::contact_photo_contracts::InMemoryPhotoStore::default()
        }
        .execute(&ContactSelection::All, ExportFormat::VCard4)
        .await,
        Err(ExportFailure::TooMany)
    );
    let exact = crowded(MAX_EXPORT_CONTACTS - 2, 2)?;
    let prepared = PrepareContactExport {
        directory: &exact,
        photos: &norelly_test_support::contact_photo_contracts::InMemoryPhotoStore::default(),
    }
    .execute(&ContactSelection::All, ExportFormat::VCard4)
    .await?;
    assert_eq!(
        prepared.cards.len() + prepared.lists.len(),
        MAX_EXPORT_CONTACTS
    );
    // Sans listes écrites, leur nombre ne compte pas.
    let csv = PrepareContactExport {
        directory: &full,
        photos: &norelly_test_support::contact_photo_contracts::InMemoryPhotoStore::default(),
    }
    .execute(&ContactSelection::All, ExportFormat::GoogleCsv)
    .await?;
    assert_eq!(csv.cards.len(), MAX_EXPORT_CONTACTS - 2);
    Ok(())
}

#[tokio::test]
async fn vcards_carry_the_reencoded_photo() -> Result<(), Box<dyn std::error::Error>> {
    use norelly_domain::contact_import::{PhotoTarget, PlacedPhoto};
    let book = InMemoryContactBook::default();
    let store = InMemoryPhotoStore::default();
    let photo = b"\xFF\xD8\xFF vignette".to_vec();
    let kept = store.put(photo.clone()).await?;
    let lost = store.put(b"\xFF\xD8\xFF perdue".to_vec()).await?;
    book.import(
        &[
            sample_contact("Alice", &["alice@example.com"]),
            sample_contact("Bob", &["bob@example.com"]),
            sample_contact("Carole", &["carole@example.com"]),
        ],
        &[],
        &[],
        &[
            PlacedPhoto {
                target: PhotoTarget::Created(0),
                photo: kept,
            },
            PlacedPhoto {
                target: PhotoTarget::Created(2),
                photo: lost.clone(),
            },
        ],
    )
    .await?;
    // La photo de Carole manque au magasin : omise, l'export continue.
    store
        .retain(
            &[store.ids().into_iter().find(|i| *i != lost).ok_or("id")?].into(),
            std::time::Duration::ZERO,
        )
        .await?;
    let prepare = PrepareContactExport {
        directory: &book,
        photos: &store,
    };
    for format in [ExportFormat::VCard4, ExportFormat::VCard3] {
        let cards = prepare.execute(&ContactSelection::All, format).await?.cards;
        let photos: Vec<_> = cards.iter().map(|c| c.photo.clone()).collect();
        assert_eq!(photos, [Some(photo.clone()), None, None], "{format:?}");
    }
    for format in [ExportFormat::GoogleCsv, ExportFormat::Ldif] {
        let cards = prepare.execute(&ContactSelection::All, format).await?.cards;
        assert!(cards.iter().all(|c| c.photo.is_none()), "{format:?}");
    }
    Ok(())
}

#[tokio::test]
async fn photos_are_checked_and_bounded_in_total() -> Result<(), Box<dyn std::error::Error>> {
    use norelly_domain::contact_export::MAX_EXPORT_PHOTO_BYTES;
    use norelly_domain::contact_import::{PhotoTarget, PlacedPhoto};
    use norelly_domain::contact_photo::MAX_PHOTO_BYTES;
    let book = InMemoryContactBook::default();
    let store = InMemoryPhotoStore::default();
    let count = MAX_EXPORT_PHOTO_BYTES / MAX_PHOTO_BYTES + 2;
    let mut drafts = Vec::new();
    let mut placed = Vec::new();
    for i in 0..count {
        drafts.push(sample_contact(
            &format!("c{i:02}"),
            &[&format!("c{i}@example.com")],
        ));
        let mut photo = vec![0xFF, 0xD8, 0xFF, u8::try_from(i).unwrap_or(0)];
        photo.resize(MAX_PHOTO_BYTES, 0);
        if i == 0 {
            // Contenu relu qui n'est pas un JPEG (magasin altéré) : omis.
            photo[0] = b'<';
        }
        placed.push(PlacedPhoto {
            target: PhotoTarget::Created(i),
            photo: store.put(photo).await?,
        });
    }
    book.import(&drafts, &[], &[], &placed).await?;
    let cards = PrepareContactExport {
        directory: &book,
        photos: &store,
    }
    .execute(&ContactSelection::All, ExportFormat::VCard4)
    .await?
    .cards;
    let kept: Vec<bool> = cards.iter().map(|c| c.photo.is_some()).collect();
    let fits = MAX_EXPORT_PHOTO_BYTES / MAX_PHOTO_BYTES;
    let mut expected = vec![false];
    expected.extend(std::iter::repeat_n(true, fits));
    expected.push(false);
    assert_eq!(kept, expected);
    Ok(())
}
