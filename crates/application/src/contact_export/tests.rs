//! Tests de l'export par pages, sur les faux en mémoire.

use std::sync::{Arc, Mutex, PoisonError};

use norelly_contacts_io::ContactsIo;
use norelly_domain::contact_export::{ContactSelection, ExportList};
use norelly_domain::contact_import::DecodedContacts;
use norelly_domain::contact_lists::{ContactList, ContactListFilter, ContactListId};
use norelly_domain::contacts::{ContactDraft, ContactFilter};
use norelly_domain::ports::{
    ContactBook, ContactCursor, ContactListBook, ContactListCursor, ContactListPage, ContactPage,
    MAX_CONTACT_LOOKUP, PageSize,
};
use norelly_test_support::contact_lists_contracts::sample_list;
use norelly_test_support::contact_photo_contracts::InMemoryPhotoStore;
use norelly_test_support::contacts_contracts::{InMemoryContactBook, full_contact, sample_contact};

use super::*;

/// Ce que l'export a donné à écrire.
#[derive(Debug, Default)]
struct Record {
    surveyed: Vec<ExportCard>,
    pages: Vec<Vec<ExportCard>>,
    lists: Vec<ExportList>,
    members: Vec<ExportMember>,
    finished: bool,
}

impl Record {
    fn cards(&self) -> Vec<ExportCard> {
        self.pages.concat()
    }
}

/// Faux format qui retient ce qu'on lui donne à écrire ; refuse les fiches
/// de la page `fail` (comptée à partir de 0) si elle est donnée.
#[derive(Debug, Clone, Default)]
struct Recorder {
    record: Arc<Mutex<Record>>,
    fail: Option<usize>,
}

impl Recorder {
    fn record(&self) -> std::sync::MutexGuard<'_, Record> {
        self.record.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl ContactCodec for Recorder {
    fn decode(&self, _: &[u8]) -> Result<DecodedContacts, ContactCodecError> {
        Err(ContactCodecError::UnknownFormat)
    }

    fn encode(
        &self,
        _: &[ExportCard],
        _: &[ExportList],
        _: ExportFormat,
    ) -> Result<Vec<u8>, ContactCodecError> {
        Ok(Vec::new())
    }

    type Encoder = Self;

    fn encoder(&self, _: ExportFormat) -> Self {
        self.clone()
    }
}

impl ContactEncoder for Recorder {
    fn survey(&mut self, cards: &[ExportCard]) {
        self.record().surveyed.extend_from_slice(cards);
    }

    fn cards(&mut self, cards: &[ExportCard]) -> Result<Vec<u8>, ContactCodecError> {
        let mut record = self.record();
        if self.fail == Some(record.pages.len()) {
            return Err(ContactCodecError::TooLarge);
        }
        record.pages.push(cards.to_vec());
        Ok(b"f".repeat(cards.len()))
    }

    fn lists(
        &mut self,
        lists: &[ExportList],
        members: &[ExportMember],
    ) -> Result<Vec<u8>, ContactCodecError> {
        let mut record = self.record();
        record.lists = lists.to_vec();
        record.members = members.to_vec();
        Ok(b"l".repeat(lists.len()))
    }

    fn finish(&mut self) -> Result<Vec<u8>, ContactCodecError> {
        self.record().finished = true;
        Ok(b"$".to_vec())
    }
}

/// Fichier entier d'un export écrit jusqu'au bout.
async fn drain<D, S, E>(export: &mut ContactExport<'_, D, S, E>) -> Result<Vec<u8>, ExportFailure>
where
    D: ContactDirectory + ContactListDirectory,
    S: ContactPhotoStore,
    E: ContactEncoder + 'static,
{
    let mut bytes = Vec::new();
    while let Some(chunk) = export.next_chunk().await? {
        bytes.extend(chunk);
    }
    Ok(bytes)
}

/// Export de `selection` au format `format`, écrit jusqu'au bout par le
/// faux format : ce qu'il a reçu.
async fn recorded<D: ContactDirectory + ContactListDirectory, S: ContactPhotoStore>(
    directory: &D,
    photos: &S,
    selection: &ContactSelection,
    format: ExportFormat,
) -> Result<Record, ExportFailure> {
    let codec = Recorder::default();
    let mut export = PrepareContactExport {
        directory,
        photos,
        codec: &codec,
    }
    .execute(selection, format)
    .await?;
    let bytes = drain(&mut export).await?;
    let mut record = std::mem::take(&mut *codec.record());
    assert_eq!(export.exported(), record.cards().len());
    assert_eq!(
        bytes.last(),
        Some(&b'$'),
        "fin du fichier écrite en dernier"
    );
    record.finished &= bytes.len() == record.cards().len() + record.lists.len() + 1;
    Ok(record)
}

fn names(cards: &[ExportCard]) -> Vec<&str> {
    cards
        .iter()
        .map(|c| c.full_name.as_deref().unwrap_or_default())
        .collect()
}

#[tokio::test]
async fn the_whole_book_is_written_in_list_order() -> Result<(), Box<dyn std::error::Error>> {
    let book = InMemoryContactBook::default();
    book.create(&sample_contact("Bob", &["bob@example.net"]))
        .await?;
    let alice = book.create(&full_contact()).await?;
    let record = recorded(
        &book,
        &InMemoryPhotoStore::default(),
        &ContactSelection::All,
        ExportFormat::VCard4,
    )
    .await?;
    let cards = record.cards();
    assert_eq!(names(&cards), ["Alice Martin", "Bob"]);
    // Préparées par la règle du domaine pour le format choisi, repérées
    // avant d'être écrites.
    assert_eq!(cards[0], export_card(alice.draft(), ExportFormat::VCard4));
    assert_eq!(record.surveyed, cards);
    assert!(record.lists.is_empty());
    assert!(record.finished);
    Ok(())
}

#[tokio::test]
async fn lists_go_with_the_whole_book_only() -> Result<(), Box<dyn std::error::Error>> {
    let book = InMemoryContactBook::default();
    let bob = book
        .create(&sample_contact("Bob", &["bob@example.net"]))
        .await?;
    let alice = book.create(&full_contact()).await?;
    book.create(&sample_contact("Carole", &["carole@example.org"]))
        .await?;
    book.create_list(&sample_list("Famille", &[bob.id(), alice.id()], true))
        .await?;
    let photos = InMemoryPhotoStore::default();
    let record = recorded(&book, &photos, &ContactSelection::All, ExportFormat::Ldif).await?;
    // Seuls les membres sont gardés, dans l'ordre de l'export (Alice, puis
    // Bob) ; les membres d'une liste désignent leur rang.
    let cards = record.cards();
    assert_eq!(
        record.members,
        [ExportMember::of(&cards[0]), ExportMember::of(&cards[1])]
    );
    assert_eq!(
        record.lists,
        [ExportList {
            name: "Famille".into(),
            description: None,
            bcc: true,
            members: vec![1, 0],
        }]
    );
    assert!(record.finished);
    let csv = recorded(
        &book,
        &photos,
        &ContactSelection::All,
        ExportFormat::GoogleCsv,
    )
    .await?;
    assert!(csv.lists.is_empty(), "le CSV ne porte pas de listes");
    let some = recorded(
        &book,
        &photos,
        &ContactSelection::Contacts(vec![bob.id()]),
        ExportFormat::VCard4,
    )
    .await?;
    assert!(some.lists.is_empty(), "sélection : pas de listes");
    Ok(())
}

#[tokio::test]
async fn a_selection_is_written_without_unknown_contacts() -> Result<(), Box<dyn std::error::Error>>
{
    let book = InMemoryContactBook::default();
    let bob = book
        .create(&sample_contact("Bob", &["bob@example.net"]))
        .await?;
    book.create(&sample_contact("Carole", &["carole@example.org"]))
        .await?;
    let gone = ContactId::new(999).ok_or("id")?;
    let record = recorded(
        &book,
        &InMemoryPhotoStore::default(),
        &ContactSelection::Contacts(vec![gone, bob.id()]),
        ExportFormat::GoogleCsv,
    )
    .await?;
    assert_eq!(names(&record.cards()), ["Bob"]);
    Ok(())
}

#[tokio::test]
async fn nothing_to_export_is_refused_before_writing() -> Result<(), Box<dyn std::error::Error>> {
    let book = InMemoryContactBook::default();
    let photos = InMemoryPhotoStore::default();
    assert_eq!(
        recorded(&book, &photos, &ContactSelection::All, ExportFormat::Ldif)
            .await
            .err(),
        Some(ExportFailure::Empty)
    );
    let gone = ContactId::new(7).ok_or("id")?;
    assert_eq!(
        recorded(
            &book,
            &photos,
            &ContactSelection::Contacts(vec![gone]),
            ExportFormat::Ldif
        )
        .await
        .err(),
        Some(ExportFailure::Empty)
    );
    // Une fiche invalide ne compte pas.
    let only = book
        .create(&sample_contact("Zoé", &["zoe@example.com"]))
        .await?;
    book.corrupt(only.id());
    assert_eq!(
        recorded(&book, &photos, &ContactSelection::All, ExportFormat::Ldif)
            .await
            .err(),
        Some(ExportFailure::Empty)
    );
    Ok(())
}

#[tokio::test]
async fn a_book_deleted_while_written_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let book = InMemoryContactBook::default();
    let zoe = book
        .create(&sample_contact("Zoé", &["zoe@example.com"]))
        .await?;
    let codec = Recorder::default();
    let photos = InMemoryPhotoStore::default();
    let mut export = PrepareContactExport {
        directory: &book,
        photos: &photos,
        codec: &codec,
    }
    .execute(&ContactSelection::All, ExportFormat::VCard4)
    .await?;
    book.delete(zoe.id()).await?;
    assert_eq!(drain(&mut export).await, Err(ExportFailure::Empty));
    assert!(!codec.record().finished);
    Ok(())
}

#[tokio::test]
async fn a_card_grown_while_written_is_refused_in_csv() -> Result<(), Box<dyn std::error::Error>> {
    let book = InMemoryContactBook::default();
    let zoe = book
        .create(&sample_contact("Zoé", &["zoe@example.com"]))
        .await?;
    let photos = InMemoryPhotoStore::default();
    let mut export = PrepareContactExport {
        directory: &book,
        photos: &photos,
        codec: &ContactsIo,
    }
    .execute(&ContactSelection::All, ExportFormat::GoogleCsv)
    .await?;
    // Une adresse de plus que les colonnes repérées : refusé, jamais
    // tronqué.
    book.update(
        zoe.id(),
        &sample_contact("Zoé", &["zoe@example.com", "zoe@example.org"]),
    )
    .await?;
    assert_eq!(
        drain(&mut export).await,
        Err(ExportFailure::Codec(ContactCodecError::NotSurveyed))
    );
    Ok(())
}

#[tokio::test]
async fn contacts_and_lists_deleted_while_written_are_left_out()
-> Result<(), Box<dyn std::error::Error>> {
    let book = InMemoryContactBook::default();
    let bob = book
        .create(&sample_contact("Bob", &["bob@example.net"]))
        .await?;
    let alice = book.create(&full_contact()).await?;
    let carole = book
        .create(&sample_contact("Carole", &["carole@example.org"]))
        .await?;
    book.create_list(&sample_list(
        "Famille",
        &[bob.id(), alice.id(), carole.id()],
        true,
    ))
    .await?;
    let gone = book
        .create_list(&sample_list("Ancienne", &[carole.id()], false))
        .await?;
    let codec = Recorder::default();
    let photos = InMemoryPhotoStore::default();
    let mut export = PrepareContactExport {
        directory: &book,
        photos: &photos,
        codec: &codec,
    }
    .execute(&ContactSelection::All, ExportFormat::VCard4)
    .await?;
    book.delete(alice.id()).await?;
    book.delete_lists(&[gone.id()]).await?;
    drain(&mut export).await?;
    let record = std::mem::take(&mut *codec.record());
    let cards = record.cards();
    assert_eq!(names(&cards), ["Bob", "Carole"]);
    // Alice omise de sa liste, membres toujours alignés sur leurs fiches ;
    // la liste supprimée n'est pas écrite.
    assert_eq!(
        record.members,
        [ExportMember::of(&cards[0]), ExportMember::of(&cards[1])]
    );
    assert_eq!(record.lists.len(), 1);
    assert_eq!(record.lists[0].name, "Famille");
    assert_eq!(record.lists[0].members, vec![0, 1]);
    assert_eq!(export.exported(), 2);
    Ok(())
}

/// Carnet de `count` contacts nommés dans l'ordre de la liste.
async fn numbered_book(count: usize) -> Result<InMemoryContactBook, Box<dyn std::error::Error>> {
    let book = InMemoryContactBook::default();
    let drafts: Vec<ContactDraft> = (0..count)
        .map(|i| sample_contact(&format!("c{i:05}"), &[&format!("c{i}@example.com")]))
        .collect();
    book.import(&drafts, &[], &[], &[]).await?;
    Ok(book)
}

/// Carnet qui retient la plus grande lecture de fiches complètes.
struct Watched<'a> {
    book: &'a InMemoryContactBook,
    largest: Mutex<usize>,
    reads: Mutex<usize>,
}

impl ContactDirectory for Watched<'_> {
    async fn page(
        &self,
        filter: &ContactFilter,
        cursor: Option<ContactCursor>,
        limit: PageSize,
    ) -> Result<ContactPage, StoreError> {
        self.book.page(filter, cursor, limit).await
    }

    async fn get(&self, id: ContactId) -> Result<Option<Contact>, StoreError> {
        self.book.get(id).await
    }

    async fn by_addresses(
        &self,
        keys: &[norelly_domain::contacts::AddressKey],
    ) -> Result<Vec<Contact>, StoreError> {
        self.book.by_addresses(keys).await
    }

    async fn contacts(
        &self,
        selection: &ContactSelection,
        limit: usize,
    ) -> Result<Vec<Contact>, StoreError> {
        let read = self.book.contacts(selection, limit).await?;
        let mut largest = self.largest.lock().unwrap_or_else(PoisonError::into_inner);
        *largest = (*largest).max(read.len()).max(match selection {
            ContactSelection::All => usize::MAX,
            ContactSelection::Contacts(ids) => ids.len(),
        });
        *self.reads.lock().unwrap_or_else(PoisonError::into_inner) += 1;
        Ok(read)
    }

    async fn contact_ids(
        &self,
        selection: &ContactSelection,
        limit: usize,
    ) -> Result<Vec<ContactId>, StoreError> {
        self.book.contact_ids(selection, limit).await
    }

    async fn photo_ids(
        &self,
    ) -> Result<std::collections::BTreeSet<norelly_domain::ports::BlobId>, StoreError> {
        self.book.photo_ids().await
    }
}

impl ContactListDirectory for Watched<'_> {
    async fn list_page(
        &self,
        filter: &ContactListFilter,
        cursor: Option<ContactListCursor>,
        limit: PageSize,
    ) -> Result<ContactListPage, StoreError> {
        self.book.list_page(filter, cursor, limit).await
    }

    async fn get_list(&self, id: ContactListId) -> Result<Option<ContactList>, StoreError> {
        self.book.get_list(id).await
    }

    async fn lists_named(&self, keys: &[String]) -> Result<Vec<ContactList>, StoreError> {
        self.book.lists_named(keys).await
    }

    async fn all_lists(&self, limit: usize) -> Result<Vec<ContactList>, StoreError> {
        self.book.all_lists(limit).await
    }
}

#[tokio::test]
async fn the_book_is_read_and_written_one_page_at_a_time() -> Result<(), Box<dyn std::error::Error>>
{
    let count = 2 * EXPORT_PAGE_SIZE + 3;
    let book = numbered_book(count).await?;
    let watched = Watched {
        book: &book,
        largest: Mutex::new(0),
        reads: Mutex::new(0),
    };
    let record = recorded(
        &watched,
        &InMemoryPhotoStore::default(),
        &ContactSelection::All,
        ExportFormat::VCard3,
    )
    .await?;
    // Jamais plus d'une page de fiches lue ou écrite à la fois : trois
    // pages repérées, puis trois écrites.
    assert_eq!(
        *watched
            .largest
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
        EXPORT_PAGE_SIZE
    );
    assert_eq!(
        *watched.reads.lock().unwrap_or_else(PoisonError::into_inner),
        6
    );
    assert_eq!(
        record.pages.iter().map(Vec::len).collect::<Vec<_>>(),
        [EXPORT_PAGE_SIZE, EXPORT_PAGE_SIZE, 3]
    );
    let expected: Vec<String> = (0..count).map(|i| format!("c{i:05}")).collect();
    assert_eq!(names(&record.cards()), expected);
    assert_eq!(record.surveyed, record.cards());
    Ok(())
}

#[tokio::test]
async fn a_refused_page_stops_the_export() -> Result<(), Box<dyn std::error::Error>> {
    let book = numbered_book(EXPORT_PAGE_SIZE + 1).await?;
    let codec = Recorder {
        fail: Some(1),
        ..Recorder::default()
    };
    let photos = InMemoryPhotoStore::default();
    let mut export = PrepareContactExport {
        directory: &book,
        photos: &photos,
        codec: &codec,
    }
    .execute(&ContactSelection::All, ExportFormat::Ldif)
    .await?;
    assert_eq!(
        drain(&mut export).await,
        Err(ExportFailure::Codec(ContactCodecError::TooLarge))
    );
    assert!(!codec.record().finished);
    Ok(())
}

/// Fichier attendu : celui que l'écriture d'un seul tenant fait de toutes
/// les fiches de `contacts` (et des listes `lists`, membres désignés par
/// leur fiche), photos lues dans `photos`.
async fn written_at_once(
    contacts: &[Contact],
    lists: &[ContactList],
    photos: &InMemoryPhotoStore,
    format: ExportFormat,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut cards = Vec::new();
    for contact in contacts {
        let mut card = export_card(contact.draft(), format);
        if format.keeps_photos()
            && let Some(id) = contact.photo()
        {
            card.photo = photos.get(id).await?;
        }
        cards.push(card);
    }
    let ids: Vec<ContactId> = contacts.iter().map(Contact::id).collect();
    Ok(ContactsIo.encode(&cards, &export_lists(lists, &ids), format)?)
}

#[tokio::test]
async fn a_file_written_by_pages_is_the_one_written_at_once()
-> Result<(), Box<dyn std::error::Error>> {
    use norelly_domain::contact_import::{PhotoTarget, PlacedPhoto};
    let count = EXPORT_PAGE_SIZE + 7;
    let book = InMemoryContactBook::default();
    let photos = InMemoryPhotoStore::default();
    let mut drafts = vec![full_contact()];
    drafts.extend(
        (1..count).map(|i| sample_contact(&format!("c{i:05}"), &[&format!("c{i}@example.com")])),
    );
    let placed = vec![
        PlacedPhoto {
            target: PhotoTarget::Created(0),
            photo: photos.put(b"\xFF\xD8\xFF premiere".to_vec()).await?,
        },
        PlacedPhoto {
            target: PhotoTarget::Created(count - 1),
            photo: photos.put(b"\xFF\xD8\xFF derniere".to_vec()).await?,
        },
    ];
    book.import(&drafts, &[], &[], &placed).await?;
    let all = book.contacts(&ContactSelection::All, count).await?;
    // Une liste dont les membres sont de part et d'autre d'une fin de page.
    let ends = [all[count - 1].id(), all[0].id(), all[EXPORT_PAGE_SIZE].id()];
    book.create_list(&sample_list("Bords", &ends, true)).await?;
    book.create_list(&sample_list("Seule", &[all[3].id()], false))
        .await?;
    let lists = book.all_lists(10).await?;
    let chosen = vec![all[EXPORT_PAGE_SIZE + 2].id(), all[1].id()];
    for format in [
        ExportFormat::VCard4,
        ExportFormat::VCard3,
        ExportFormat::GoogleCsv,
        ExportFormat::Ldif,
    ] {
        for selection in [
            ContactSelection::All,
            ContactSelection::Contacts(chosen.clone()),
        ] {
            let (contacts, lists) = match &selection {
                ContactSelection::All if format != ExportFormat::GoogleCsv => {
                    (all.clone(), lists.clone())
                }
                ContactSelection::All => (all.clone(), Vec::new()),
                ContactSelection::Contacts(_) => (
                    vec![all[1].clone(), all[EXPORT_PAGE_SIZE + 2].clone()],
                    Vec::new(),
                ),
            };
            let mut export = PrepareContactExport {
                directory: &book,
                photos: &photos,
                codec: &ContactsIo,
            }
            .execute(&selection, format)
            .await?;
            let bytes = drain(&mut export).await?;
            assert!(
                bytes == written_at_once(&contacts, &lists, &photos, format).await?,
                "{format:?}, {selection:?} : fichier différent"
            );
            assert_eq!(export.exported(), contacts.len());
        }
    }
    Ok(())
}

/// Carnet qui rend au plus `contacts` fiches et `lists` listes, autant
/// qu'on lui en demande dans ces limites.
struct CrowdedDirectory {
    contact: ContactDraft,
    contacts: usize,
    /// Les premiers identifiants, de fiches invalides (écartées à la
    /// lecture).
    invalid: i64,
    list: ContactList,
    lists: usize,
}

impl ContactDirectory for CrowdedDirectory {
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
        selection: &ContactSelection,
        limit: usize,
    ) -> Result<Vec<Contact>, StoreError> {
        let ContactSelection::Contacts(ids) = selection else {
            return Err(StoreError("tout le carnet lu d'un coup".into()));
        };
        Ok(ids
            .iter()
            .filter(|id| id.get() > self.invalid)
            .take(limit)
            .map(|&id| self.contact.clone().with_id(id))
            .collect())
    }

    async fn contact_ids(
        &self,
        _: &ContactSelection,
        limit: usize,
    ) -> Result<Vec<ContactId>, StoreError> {
        Ok((1..)
            .filter_map(ContactId::new)
            .take(limit.min(self.contacts))
            .collect())
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

    async fn get_list(&self, id: ContactListId) -> Result<Option<ContactList>, StoreError> {
        Ok((self.lists > 0 && id == self.list.id()).then(|| self.list.clone()))
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
        contact: sample_contact("Zoé", &["zoe@example.com"]),
        contacts,
        invalid: 0,
        list: sample_list("L", &[id], false).with_id(ContactListId::new(1).ok_or("id")?),
        lists,
    })
}

#[tokio::test]
async fn a_book_larger_than_an_import_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let directory = crowded(usize::MAX, 0)?;
    assert_eq!(
        recorded(
            &directory,
            &InMemoryPhotoStore::default(),
            &ContactSelection::All,
            ExportFormat::VCard3
        )
        .await
        .err(),
        Some(ExportFailure::TooMany)
    );
    Ok(())
}

#[tokio::test]
async fn invalid_cards_never_hide_a_book_over_the_bound() -> Result<(), Box<dyn std::error::Error>>
{
    // Deux fiches invalides en tête, une fiche valide de trop au-delà.
    let directory = CrowdedDirectory {
        invalid: 2,
        ..crowded(MAX_EXPORT_CONTACTS + 3, 0)?
    };
    assert_eq!(
        recorded(
            &directory,
            &InMemoryPhotoStore::default(),
            &ContactSelection::All,
            ExportFormat::Ldif
        )
        .await
        .err(),
        Some(ExportFailure::TooMany)
    );
    // Juste à la borne : tout est écrit.
    let directory = CrowdedDirectory {
        invalid: 2,
        ..crowded(MAX_EXPORT_CONTACTS + 2, 0)?
    };
    let record = recorded(
        &directory,
        &InMemoryPhotoStore::default(),
        &ContactSelection::All,
        ExportFormat::Ldif,
    )
    .await?;
    assert_eq!(record.cards().len(), MAX_EXPORT_CONTACTS);
    Ok(())
}

#[tokio::test]
async fn lists_count_with_the_cards_in_the_bound() -> Result<(), Box<dyn std::error::Error>> {
    let photos = InMemoryPhotoStore::default();
    let full = crowded(MAX_EXPORT_CONTACTS - 2, 3)?;
    assert_eq!(
        recorded(&full, &photos, &ContactSelection::All, ExportFormat::VCard4)
            .await
            .err(),
        Some(ExportFailure::TooMany)
    );
    let exact = crowded(MAX_EXPORT_CONTACTS - 2, 2)?;
    let record = recorded(
        &exact,
        &photos,
        &ContactSelection::All,
        ExportFormat::VCard4,
    )
    .await?;
    assert_eq!(
        record.cards().len() + record.lists.len(),
        MAX_EXPORT_CONTACTS
    );
    // Sans listes écrites, leur nombre ne compte pas.
    let csv = recorded(
        &full,
        &photos,
        &ContactSelection::All,
        ExportFormat::GoogleCsv,
    )
    .await?;
    assert_eq!(csv.cards().len(), MAX_EXPORT_CONTACTS - 2);
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
    for format in [ExportFormat::VCard4, ExportFormat::VCard3] {
        let cards = recorded(&book, &store, &ContactSelection::All, format)
            .await?
            .cards();
        let photos: Vec<_> = cards.iter().map(|c| c.photo.clone()).collect();
        assert_eq!(photos, [Some(photo.clone()), None, None], "{format:?}");
    }
    for format in [ExportFormat::GoogleCsv, ExportFormat::Ldif] {
        let cards = recorded(&book, &store, &ContactSelection::All, format)
            .await?
            .cards();
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
    let cards = recorded(&book, &store, &ContactSelection::All, ExportFormat::VCard4)
        .await?
        .cards();
    let kept: Vec<bool> = cards.iter().map(|c| c.photo.is_some()).collect();
    let fits = MAX_EXPORT_PHOTO_BYTES / MAX_PHOTO_BYTES;
    let mut expected = vec![false];
    expected.extend(std::iter::repeat_n(true, fits));
    expected.push(false);
    assert_eq!(kept, expected);
    Ok(())
}
