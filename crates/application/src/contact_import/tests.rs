//! Tests de l'import du carnet, sur les faux en mémoire.

use std::sync::{Mutex, MutexGuard, PoisonError};

use norelly_domain::contact_export::ContactSelection;
use norelly_domain::contact_import::{
    ContactCompletion, DecodedContacts, ImportedList, ImportedMember, ListImport,
    MAX_IMPORT_ATTEMPTS, RejectReason,
};
use norelly_domain::contact_lists::{ContactListFilter, ContactListId};
use norelly_domain::contact_photo::{EmbeddedPhoto, PhotoFormat};
use norelly_domain::contacts::{
    AddressKey, ContactDraft, ContactFilter, ContactId, ContactInput, EmailInput,
};
use norelly_domain::ports::{
    ContactCursor, ContactListBook, ContactListCursor, ContactListPage, ContactPage,
    MAX_CONTACT_LOOKUP, PageSize,
};
use norelly_test_support::contact_import_contracts::{FakeContactCodec, FakeNfc};
use norelly_test_support::contact_lists_contracts::sample_list;
use norelly_test_support::contact_photo_contracts::{
    FakePhotoCodec, InMemoryPhotoStore, photo_digest,
};
use norelly_test_support::contacts_contracts::{InMemoryContactBook, sample_contact};

use super::*;

fn card(name: &str, emails: &[&str]) -> ImportedCard {
    ImportedCard::Contact(ContactInput {
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
}

fn codec(cards: Vec<ImportedCard>) -> FakeContactCodec {
    FakeContactCodec::new(
        b"fichier".to_vec(),
        DecodedContacts {
            format: ContactFormat::VCard,
            cards,
            photos: CardPhotos::new(),
        },
    )
}

async fn names(book: &InMemoryContactBook) -> Result<Vec<String>, StoreError> {
    let page = book
        .page(&ContactFilter::default(), None, PageSize::new(100))
        .await?;
    Ok(page.items.into_iter().map(|i| i.name).collect())
}

#[tokio::test]
async fn preview_plans_both_policies_against_the_book() -> Result<(), ImportFailure> {
    let book = InMemoryContactBook::default();
    book.create(&sample_contact("Alice", &["alice@example.com"]))
        .await?;
    let codec = codec(vec![
        card("Alice M.", &["ALICE@example.com", "alice@work.example"]),
        card("Bob", &["bob@example.net"]),
        ImportedCard::Unreadable,
    ]);
    let preview = PreviewContactImport {
        codec: &codec,
        directory: &book,
        normalizer: &FakeNfc,
    }
    .execute(b"fichier")
    .await?;
    assert_eq!(preview.format, ContactFormat::VCard);
    assert_eq!(preview.cards.len(), 3);
    assert_eq!(
        (
            preview.skip.create.len(),
            preview.skip.complete.len(),
            preview.skip.duplicates
        ),
        (1, 0, 1)
    );
    assert_eq!(
        (
            preview.complete.create.len(),
            preview.complete.complete.len(),
            preview.complete.duplicates
        ),
        (1, 1, 0)
    );
    assert_eq!(preview.skip.rejected[0].reason, RejectReason::Unreadable);
    // L'aperçu n'écrit rien.
    assert_eq!(names(&book).await?, ["Alice"]);
    Ok(())
}

#[tokio::test]
async fn an_unreadable_file_is_refused() {
    let book = InMemoryContactBook::default();
    let preview = PreviewContactImport {
        codec: &codec(Vec::new()),
        directory: &book,
        normalizer: &FakeNfc,
    }
    .execute(b"autre chose")
    .await;
    assert_eq!(
        preview,
        Err(ImportFailure::File(ContactCodecError::UnknownFormat))
    );
}

#[tokio::test]
async fn apply_writes_the_plan_of_the_chosen_policy() -> Result<(), ImportFailure> {
    let cards = vec![
        card("", &["alice@example.com", "alice@work.example"]),
        card("Bob", &["bob@example.net"]),
    ];
    for (policy, alice_addresses) in [(DuplicatePolicy::Skip, 1), (DuplicatePolicy::Complete, 2)] {
        let book = InMemoryContactBook::default();
        let alice = book
            .create(&sample_contact("Alice", &["alice@example.com"]))
            .await?;
        let outcome = ApplyContactImport {
            book: &book,
            normalizer: &FakeNfc,
            photo_codec: &FakePhotoCodec,
            photos: &InMemoryPhotoStore::default(),
        }
        .execute(&cards, &CardPhotos::new(), policy)
        .await?;
        assert_eq!(outcome.created, 1);
        assert_eq!(outcome.completed, alice_addresses - 1);
        assert_eq!(names(&book).await?, ["Alice", "Bob"]);
        let alice = book
            .get(alice.id())
            .await?
            .unwrap_or_else(|| panic!("Alice"));
        assert_eq!(alice.draft().emails().len(), alice_addresses, "{policy:?}");
    }
    Ok(())
}

#[tokio::test]
async fn apply_replans_against_the_book_of_the_moment() -> Result<(), ImportFailure> {
    let book = InMemoryContactBook::default();
    let cards = vec![card("Bob", &["bob@example.net"])];
    // Bob ajouté après l'aperçu : doublon, pas d'erreur.
    book.create(&sample_contact("Bob", &["BOB@example.net"]))
        .await?;
    let outcome = ApplyContactImport {
        book: &book,
        normalizer: &FakeNfc,
        photo_codec: &FakePhotoCodec,
        photos: &InMemoryPhotoStore::default(),
    }
    .execute(&cards, &CardPhotos::new(), DuplicatePolicy::Skip)
    .await?;
    assert_eq!(outcome, ImportOutcome::default());
    assert_eq!(names(&book).await?, ["Bob"]);
    Ok(())
}

#[tokio::test]
async fn duplicates_are_found_beyond_one_lookup() -> Result<(), ImportFailure> {
    let book = InMemoryContactBook::default();
    let count = MAX_CONTACT_LOOKUP + 50;
    let last = format!("c{}@example.com", count - 1);
    book.create(&sample_contact("Dernier", &[&last])).await?;
    let cards: Vec<ImportedCard> = (0..count)
        .map(|i| card("", &[&format!("c{i}@example.com")]))
        .collect();
    let outcome = ApplyContactImport {
        book: &book,
        normalizer: &FakeNfc,
        photo_codec: &FakePhotoCodec,
        photos: &InMemoryPhotoStore::default(),
    }
    .execute(&cards, &CardPhotos::new(), DuplicatePolicy::Skip)
    .await?;
    assert_eq!(outcome.created, count - 1);
    Ok(())
}

/// Carnet dont la fiche `target` est modifiée (ou supprimée, si `delete`)
/// par un tiers juste avant chacun des `edits` premiers imports : entre la
/// lecture qui a servi au plan et l'écriture.
#[derive(Debug)]
struct EditedMeanwhile {
    inner: InMemoryContactBook,
    target: ContactId,
    delete: bool,
    edits: Mutex<usize>,
    imports: Mutex<usize>,
    /// Liste créée par un tiers juste avant le premier import (nom pris
    /// entre la lecture et l'écriture).
    list_meanwhile: Mutex<Option<String>>,
}

fn count(cell: &Mutex<usize>) -> MutexGuard<'_, usize> {
    cell.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ContactDirectory for EditedMeanwhile {
    async fn photo_ids(
        &self,
    ) -> Result<std::collections::BTreeSet<norelly_domain::ports::BlobId>, StoreError> {
        self.inner.photo_ids().await
    }

    async fn page(
        &self,
        filter: &ContactFilter,
        cursor: Option<ContactCursor>,
        limit: PageSize,
    ) -> Result<ContactPage, StoreError> {
        self.inner.page(filter, cursor, limit).await
    }

    async fn get(&self, id: ContactId) -> Result<Option<Contact>, StoreError> {
        self.inner.get(id).await
    }

    async fn by_addresses(&self, keys: &[AddressKey]) -> Result<Vec<Contact>, StoreError> {
        self.inner.by_addresses(keys).await
    }

    async fn contacts(
        &self,
        selection: &ContactSelection,
        limit: usize,
    ) -> Result<Vec<Contact>, StoreError> {
        self.inner.contacts(selection, limit).await
    }
}

impl ContactListDirectory for EditedMeanwhile {
    async fn list_page(
        &self,
        filter: &ContactListFilter,
        cursor: Option<ContactListCursor>,
        limit: PageSize,
    ) -> Result<ContactListPage, StoreError> {
        self.inner.list_page(filter, cursor, limit).await
    }

    async fn get_list(&self, id: ContactListId) -> Result<Option<ContactList>, StoreError> {
        self.inner.get_list(id).await
    }

    async fn lists_named(&self, keys: &[String]) -> Result<Vec<ContactList>, StoreError> {
        self.inner.lists_named(keys).await
    }

    async fn all_lists(&self, limit: usize) -> Result<Vec<ContactList>, StoreError> {
        self.inner.all_lists(limit).await
    }
}

impl ContactBook for EditedMeanwhile {
    async fn create(&self, draft: &ContactDraft) -> Result<Contact, ContactStoreError> {
        self.inner.create(draft).await
    }

    async fn update(
        &self,
        id: ContactId,
        draft: &ContactDraft,
    ) -> Result<Contact, ContactStoreError> {
        self.inner.update(id, draft).await
    }

    async fn delete(&self, id: ContactId) -> Result<bool, ContactStoreError> {
        self.inner.delete(id).await
    }

    async fn delete_many(&self, ids: &[ContactId]) -> Result<usize, ContactStoreError> {
        self.inner.delete_many(ids).await
    }

    async fn import(
        &self,
        create: &[ContactDraft],
        complete: &[ContactCompletion],
        lists: &[ListImport],
        photos: &[PlacedPhoto],
    ) -> Result<ImportOutcome, ContactStoreError> {
        *count(&self.imports) += 1;
        let meanwhile = self
            .list_meanwhile
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(name) = meanwhile {
            self.inner
                .create_list(&sample_list(&name, &[self.target], false))
                .await
                .map_err(|e| ContactStoreError::Store(StoreError(e.to_string())))?;
        }
        let edit = {
            let mut edits = count(&self.edits);
            let edit = *edits > 0;
            *edits = edits.saturating_sub(1);
            edit
        };
        if edit && self.delete {
            self.inner.delete(self.target).await?;
        } else if edit {
            let current = self
                .inner
                .get(self.target)
                .await?
                .ok_or(ContactStoreError::NotFound)?;
            let mut input = current.draft().to_input();
            input.organization.push('+');
            let draft = ContactDraft::new(input)
                .map_err(|e| ContactStoreError::Corrupted(e.to_string()))?;
            self.inner.update(self.target, &draft).await?;
        }
        self.inner.import(create, complete, lists, photos).await
    }

    async fn merge(
        &self,
        keep: &Contact,
        draft: &ContactDraft,
        absorbed: &[Contact],
    ) -> Result<Contact, ContactStoreError> {
        self.inner.merge(keep, draft, absorbed).await
    }
}

async fn edited_meanwhile(edits: usize) -> Result<EditedMeanwhile, ImportFailure> {
    let inner = InMemoryContactBook::default();
    let alice = inner
        .create(&sample_contact("Alice", &["alice@example.com"]))
        .await?;
    Ok(EditedMeanwhile {
        inner,
        target: alice.id(),
        delete: false,
        edits: Mutex::new(edits),
        imports: Mutex::new(0),
        list_meanwhile: Mutex::new(None),
    })
}

#[tokio::test]
async fn a_contact_edited_meanwhile_is_completed_without_losing_the_edit()
-> Result<(), ImportFailure> {
    let book = edited_meanwhile(1).await?;
    let cards = vec![card("", &["alice@example.com", "alice@work.example"])];
    let outcome = ApplyContactImport {
        book: &book,
        normalizer: &FakeNfc,
        photo_codec: &FakePhotoCodec,
        photos: &InMemoryPhotoStore::default(),
    }
    .execute(&cards, &CardPhotos::new(), DuplicatePolicy::Complete)
    .await?;
    assert_eq!(outcome.completed, 1);
    let alice = book
        .get(book.target)
        .await?
        .unwrap_or_else(|| panic!("Alice"));
    // La modification faite entre la lecture et l'import est gardée, et
    // le complément s'y ajoute.
    assert_eq!(alice.draft().organization(), Some("+"));
    assert_eq!(alice.draft().emails().len(), 2);
    Ok(())
}

#[tokio::test]
async fn a_contact_edited_at_every_attempt_cancels_the_import() -> Result<(), ImportFailure> {
    let book = edited_meanwhile(usize::MAX).await?;
    let cards = vec![
        card("", &["alice@example.com", "alice@work.example"]),
        card("Bob", &["bob@example.net"]),
    ];
    let outcome = ApplyContactImport {
        book: &book,
        normalizer: &FakeNfc,
        photo_codec: &FakePhotoCodec,
        photos: &InMemoryPhotoStore::default(),
    }
    .execute(&cards, &CardPhotos::new(), DuplicatePolicy::Complete)
    .await;
    assert_eq!(
        outcome,
        Err(ImportFailure::Store(ContactStoreError::Conflict))
    );
    assert_eq!(*count(&book.imports), MAX_IMPORT_ATTEMPTS);
    let alice = book
        .get(book.target)
        .await?
        .unwrap_or_else(|| panic!("Alice"));
    assert_eq!(alice.draft().emails().len(), 1);
    assert_eq!(names(&book.inner).await?, ["Alice"]);
    Ok(())
}

#[tokio::test]
async fn a_contact_deleted_meanwhile_is_created_again() -> Result<(), ImportFailure> {
    let book = EditedMeanwhile {
        delete: true,
        ..edited_meanwhile(1).await?
    };
    let cards = vec![card("Alice", &["alice@example.com", "alice@work.example"])];
    let outcome = ApplyContactImport {
        book: &book,
        normalizer: &FakeNfc,
        photo_codec: &FakePhotoCodec,
        photos: &InMemoryPhotoStore::default(),
    }
    .execute(&cards, &CardPhotos::new(), DuplicatePolicy::Complete)
    .await?;
    assert_eq!(
        outcome,
        ImportOutcome {
            created: 1,
            completed: 0,
            ..ImportOutcome::default()
        }
    );
    assert_eq!(names(&book.inner).await?, ["Alice"]);
    Ok(())
}

fn list(name: &str, members: &[&str]) -> ImportedCard {
    ImportedCard::List(ImportedList {
        name: name.to_owned(),
        members: members
            .iter()
            .map(|&m| ImportedMember::Address(m.to_owned()))
            .collect(),
        ..ImportedList::default()
    })
}

#[tokio::test]
async fn preview_plans_lists_against_the_book_lists() -> Result<(), ImportFailure> {
    let book = InMemoryContactBook::default();
    let alice = book
        .create(&sample_contact("Alice", &["alice@example.com"]))
        .await?;
    book.create_list(&sample_list("Famille", &[alice.id()], false))
        .await
        .map_err(|e| StoreError(e.to_string()))?;
    let codec = codec(vec![
        card("Bob", &["bob@example.net"]),
        list("famille", &["bob@example.net", "alice@example.com"]),
        list("Club", &["alice@example.com", "inconnu@example.org"]),
    ]);
    let preview = PreviewContactImport {
        codec: &codec,
        directory: &book,
        normalizer: &FakeNfc,
    }
    .execute(b"fichier")
    .await?;
    // La liste du carnet n'est jamais touchée, quelle que soit la
    // politique.
    for plan in [&preview.skip, &preview.complete] {
        assert_eq!(plan.lists.len(), 1);
        assert_eq!(plan.lists[0].draft().name(), "Club");
        assert_eq!(plan.duplicates, 1);
        assert_eq!(plan.missing_members, 1);
    }
    Ok(())
}

#[tokio::test]
async fn apply_writes_lists_with_their_new_and_existing_members() -> Result<(), ImportFailure> {
    let book = InMemoryContactBook::default();
    let alice = book
        .create(&sample_contact("Alice", &["alice@example.com"]))
        .await?;
    let cards = vec![
        list("Club", &["bob@example.net", "alice@example.com"]),
        card("Bob", &["bob@example.net"]),
    ];
    let outcome = ApplyContactImport {
        book: &book,
        normalizer: &FakeNfc,
        photo_codec: &norelly_test_support::contact_photo_contracts::FakePhotoCodec,
        photos: &norelly_test_support::contact_photo_contracts::InMemoryPhotoStore::default(),
    }
    .execute(
        &cards,
        &norelly_domain::contact_photo::CardPhotos::new(),
        DuplicatePolicy::Skip,
    )
    .await?;
    assert_eq!((outcome.created, outcome.lists_created), (1, 1));
    let [club] = &book.all_lists(10).await?[..] else {
        panic!("liste Club")
    };
    assert_eq!(club.draft().name(), "Club");
    assert_eq!(club.draft().members().len(), 2);
    assert_eq!(club.draft().members()[1], alice.id());
    Ok(())
}

#[tokio::test]
async fn a_list_name_taken_meanwhile_is_replanned_and_left_aside() -> Result<(), ImportFailure> {
    let book = edited_meanwhile(0).await?;
    *book
        .list_meanwhile
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some("Club".into());
    let cards = vec![
        card("Bob", &["bob@example.net"]),
        list("CLUB", &["bob@example.net"]),
    ];
    let outcome = ApplyContactImport {
        book: &book,
        normalizer: &FakeNfc,
        photo_codec: &norelly_test_support::contact_photo_contracts::FakePhotoCodec,
        photos: &norelly_test_support::contact_photo_contracts::InMemoryPhotoStore::default(),
    }
    .execute(
        &cards,
        &norelly_domain::contact_photo::CardPhotos::new(),
        DuplicatePolicy::Complete,
    )
    .await?;
    assert_eq!(*count(&book.imports), 2, "plan recalculé une fois");
    assert_eq!((outcome.created, outcome.lists_created), (1, 0));
    // La liste créée entre-temps reste telle quelle.
    let [club] = &book.inner.all_lists(10).await?[..] else {
        panic!("liste Club")
    };
    assert_eq!(club.draft().name(), "Club");
    assert_eq!(club.draft().members(), [book.target]);
    Ok(())
}

fn embedded(bytes: &[u8]) -> EmbeddedPhoto {
    EmbeddedPhoto {
        declared: None,
        bytes: bytes.to_vec(),
    }
}

/// Photo que le faux réencodage accepte.
const JPEG: &[u8] = b"\xFF\xD8\xFF\xE0 contenu";

#[tokio::test]
async fn apply_gives_imported_contacts_their_reencoded_photo() -> Result<(), ImportFailure> {
    let book = InMemoryContactBook::default();
    let store = InMemoryPhotoStore::default();
    let cards = vec![
        card("Alice", &["alice@example.com"]),
        card("Bob", &["bob@example.net"]),
        card("Carole", &["carole@example.org"]),
        card("Denis", &["denis@example.org"]),
    ];
    let photos: CardPhotos = [
        (0, embedded(JPEG)),
        // Refusée par le domaine (GIF), puis par le réencodage (tronquée).
        (1, embedded(b"GIF89a contenu")),
        (2, embedded(b"\xFF\xD8\xFF\xE0 tronque")),
    ]
    .into_iter()
    .collect();
    let outcome = ApplyContactImport {
        book: &book,
        normalizer: &FakeNfc,
        photo_codec: &FakePhotoCodec,
        photos: &store,
    }
    .execute(&cards, &photos, DuplicatePolicy::Skip)
    .await?;
    assert_eq!(outcome.created, 4);
    let thumbnail = FakePhotoCodec
        .thumbnail(JPEG, PhotoFormat::Jpeg)
        .unwrap_or_else(|e| panic!("{e}"));
    let id = photo_digest(&thumbnail);
    // Seule la photo réencodée est enregistrée, pas l'originale.
    assert_eq!(
        store.ids().into_iter().collect::<Vec<_>>(),
        std::slice::from_ref(&id)
    );
    assert_eq!(store.get(&id).await?, Some(thumbnail));
    let page = book
        .page(&ContactFilter::default(), None, PageSize::new(10))
        .await?;
    let mut with_photo = Vec::new();
    for item in page.items {
        let contact = book.get(item.id).await?.unwrap_or_else(|| panic!("fiche"));
        with_photo.push((item.name, contact.photo().cloned()));
    }
    assert_eq!(
        with_photo,
        [
            ("Alice".to_owned(), Some(id)),
            ("Bob".to_owned(), None),
            ("Carole".to_owned(), None),
            ("Denis".to_owned(), None),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn a_photo_alone_completes_a_contact_without_one() -> Result<(), ImportFailure> {
    let book = InMemoryContactBook::default();
    let store = InMemoryPhotoStore::default();
    let alice = book
        .create(&sample_contact("Alice", &["alice@example.com"]))
        .await?;
    let cards = vec![card("Alice", &["alice@example.com"])];
    let photos: CardPhotos = [(0, embedded(JPEG))].into_iter().collect();
    let codec = FakeContactCodec::new(
        b"fichier".to_vec(),
        DecodedContacts {
            format: ContactFormat::VCard,
            cards: cards.clone(),
            photos: photos.clone(),
        },
    );
    let preview = PreviewContactImport {
        codec: &codec,
        directory: &book,
        normalizer: &FakeNfc,
    }
    .execute(b"fichier")
    .await?;
    assert_eq!(preview.photos, photos);
    assert_eq!(
        (preview.complete.complete.len(), preview.complete.duplicates),
        (1, 0)
    );
    assert_eq!(
        (preview.skip.complete.len(), preview.skip.duplicates),
        (0, 1)
    );
    let outcome = ApplyContactImport {
        book: &book,
        normalizer: &FakeNfc,
        photo_codec: &FakePhotoCodec,
        photos: &store,
    }
    .execute(&preview.cards, &preview.photos, DuplicatePolicy::Complete)
    .await?;
    assert_eq!((outcome.created, outcome.completed), (0, 1));
    let read = book
        .get(alice.id())
        .await?
        .unwrap_or_else(|| panic!("Alice"));
    assert_eq!(read.draft(), alice.draft());
    assert!(read.photo().is_some());
    Ok(())
}

/// Réencodage qui panique (décodeur défaillant).
#[derive(Debug, Clone, Copy)]
struct PanickingCodec;

impl PhotoCodec for PanickingCodec {
    fn dimensions(
        &self,
        _: &[u8],
        _: PhotoFormat,
    ) -> Result<(u32, u32), norelly_domain::ports::PhotoCodecError> {
        Ok((1, 1))
    }

    fn thumbnail(
        &self,
        _: &[u8],
        _: PhotoFormat,
    ) -> Result<Vec<u8>, norelly_domain::ports::PhotoCodecError> {
        panic!("décodeur défaillant")
    }
}

/// Réencodage dont chaque photo annonce les dimensions maximales.
#[derive(Debug, Clone, Copy)]
struct HugeCodec;

impl PhotoCodec for HugeCodec {
    fn dimensions(
        &self,
        _: &[u8],
        _: PhotoFormat,
    ) -> Result<(u32, u32), norelly_domain::ports::PhotoCodecError> {
        Ok((4096, 4096))
    }

    fn thumbnail(
        &self,
        bytes: &[u8],
        format: PhotoFormat,
    ) -> Result<Vec<u8>, norelly_domain::ports::PhotoCodecError> {
        FakePhotoCodec.thumbnail(bytes, format)
    }
}

fn numbered(n: usize) -> (Vec<ImportedCard>, CardPhotos) {
    let cards: Vec<ImportedCard> = (0..n)
        .map(|i| card(&format!("c{i}"), &[&format!("c{i}@example.com")]))
        .collect();
    let photos: CardPhotos = (0..n)
        .map(|i| {
            let mut bytes = b"\xFF\xD8\xFF\xE0 ".to_vec();
            bytes.extend_from_slice(format!("{i} contenu").as_bytes());
            (i, embedded(&bytes))
        })
        .collect();
    (cards, photos)
}

#[tokio::test]
async fn a_panicking_decoder_leaves_the_photo_out_not_the_import() -> Result<(), ImportFailure> {
    let book = InMemoryContactBook::default();
    let store = InMemoryPhotoStore::default();
    let (cards, photos) = numbered(2);
    let outcome = ApplyContactImport {
        book: &book,
        normalizer: &FakeNfc,
        photo_codec: &PanickingCodec,
        photos: &store,
    }
    .execute(&cards, &photos, DuplicatePolicy::Skip)
    .await?;
    assert_eq!(outcome.created, 2);
    assert!(store.ids().is_empty());
    assert!(book.photo_ids().await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn the_decoding_work_of_an_import_is_bounded() -> Result<(), ImportFailure> {
    use norelly_domain::contact_photo::{MAX_IMPORT_PHOTO_PIXELS, MAX_PHOTO_SOURCE_PIXELS};
    let book = InMemoryContactBook::default();
    let store = InMemoryPhotoStore::default();
    let fits = usize::try_from(MAX_IMPORT_PHOTO_PIXELS / MAX_PHOTO_SOURCE_PIXELS).unwrap_or(0);
    let (cards, photos) = numbered(fits + 3);
    let outcome = ApplyContactImport {
        book: &book,
        normalizer: &FakeNfc,
        photo_codec: &HugeCodec,
        photos: &store,
    }
    .execute(&cards, &photos, DuplicatePolicy::Skip)
    .await?;
    assert_eq!(outcome.created, fits + 3);
    let with_photo = book
        .contacts(&ContactSelection::All, 1000)
        .await?
        .iter()
        .filter(|c| c.photo().is_some())
        .count();
    assert_eq!(with_photo, fits);
    Ok(())
}

#[tokio::test]
async fn a_refused_photo_alone_completes_nothing() -> Result<(), ImportFailure> {
    let book = InMemoryContactBook::default();
    book.create(&sample_contact("Alice", &["alice@example.com"]))
        .await?;
    let cards = vec![card("Alice", &["alice@example.com"])];
    // Signature JPEG, mais illisible pour le réencodage.
    let photos: CardPhotos = [(0, embedded(b"\xFF\xD8\xFF\xE0 tronque"))]
        .into_iter()
        .collect();
    let outcome = ApplyContactImport {
        book: &book,
        normalizer: &FakeNfc,
        photo_codec: &FakePhotoCodec,
        photos: &InMemoryPhotoStore::default(),
    }
    .execute(&cards, &photos, DuplicatePolicy::Complete)
    .await?;
    assert_eq!((outcome.created, outcome.completed), (0, 0));
    Ok(())
}
