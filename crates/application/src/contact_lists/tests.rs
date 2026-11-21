//! Tests des cas d'usage des listes, sur le faux en mémoire.

use std::sync::{Mutex, PoisonError};

use norelly_domain::contact_lists::{
    ContactListField, MAX_CONTACT_LIST_MEMBERS, MAX_LIST_SUGGESTIONS,
};
use norelly_domain::ports::ContactBook;
use norelly_test_support::contact_import_contracts::FakeNfc;
use norelly_test_support::contact_lists_contracts::sample_list;
use norelly_test_support::contacts_contracts::{InMemoryContactBook, sample_contact};

use super::*;

fn input(name: &str, members: &[ContactId], bcc: bool) -> ContactListInput {
    ContactListInput {
        name: name.to_owned(),
        description: String::new(),
        members: members.iter().map(|m| m.get()).collect(),
        bcc,
    }
}

async fn contact(book: &InMemoryContactBook, name: &str, address: &str) -> Contact {
    book.create(&sample_contact(name, &[address]))
        .await
        .unwrap_or_else(|e| panic!("{e}"))
}

#[tokio::test]
async fn create_validates_then_stores() -> Result<(), ContactListFailure> {
    let book = InMemoryContactBook::default();
    let alice = contact(&book, "Alice", "alice@example.com").await;
    let created = CreateContactList {
        book: &book,
        normalizer: &FakeNfc,
    }
    .execute(input(" Famille ", &[alice.id()], true))
    .await?;
    assert_eq!(created.draft().name(), "Famille");
    assert_eq!(book.get_list(created.id()).await?, Some(created));
    Ok(())
}

#[tokio::test]
async fn invalid_input_writes_nothing() -> Result<(), ContactListFailure> {
    let book = InMemoryContactBook::default();
    assert_eq!(
        CreateContactList {
            book: &book,
            normalizer: &FakeNfc
        }
        .execute(input(" ", &[], false))
        .await,
        Err(ContactListFailure::Invalid(ContactListError::NoName))
    );
    assert_eq!(
        CreateContactList {
            book: &book,
            normalizer: &FakeNfc
        }
        .execute(input(&"x".repeat(1000), &[], false))
        .await,
        Err(ContactListFailure::Invalid(ContactListError::TooLong(
            ContactListField::Name
        )))
    );
    assert_eq!(
        ListContactLists { directory: &book }
            .execute("", None, PageSize::new(10))
            .await?,
        ContactListPage::default()
    );
    Ok(())
}

#[tokio::test]
async fn update_validates_then_replaces_and_reports_refusals() -> Result<(), ContactListFailure> {
    let book = InMemoryContactBook::default();
    let alice = contact(&book, "Alice", "alice@example.com").await;
    let create = CreateContactList {
        book: &book,
        normalizer: &FakeNfc,
    };
    let list = create.execute(input("Famille", &[], false)).await?;
    create.execute(input("Club", &[], false)).await?;
    let update = UpdateContactList {
        book: &book,
        normalizer: &FakeNfc,
    };
    let updated = update
        .execute(list.id(), input("Famille proche", &[alice.id()], false))
        .await?;
    assert_eq!(updated.draft().members(), [alice.id()]);
    assert_eq!(
        update.execute(list.id(), input("club", &[], false)).await,
        Err(ContactListFailure::Store(
            ContactListStoreError::DuplicateName("club".into())
        ))
    );
    assert_eq!(
        update.execute(list.id(), input("", &[], false)).await,
        Err(ContactListFailure::Invalid(ContactListError::NoName))
    );
    Ok(())
}

#[tokio::test]
async fn list_filters_on_the_name_and_delete_keeps_contacts() -> Result<(), ContactListFailure> {
    let book = InMemoryContactBook::default();
    let alice = contact(&book, "Alice", "alice@example.com").await;
    let create = CreateContactList {
        book: &book,
        normalizer: &FakeNfc,
    };
    let a = create
        .execute(input("Famille", &[alice.id()], false))
        .await?;
    create.execute(input("Club", &[], false)).await?;
    let page = ListContactLists { directory: &book }
        .execute(" FAM ", None, PageSize::new(10))
        .await?;
    assert_eq!(page.items, vec![ContactListSummary::from(&a)]);
    assert_eq!(
        DeleteContactLists { book: &book }
            .execute(&[a.id(), a.id()])
            .await?,
        1
    );
    assert!(book.get(alice.id()).await?.is_some());
    Ok(())
}

#[tokio::test]
async fn get_returns_members_in_list_order() -> Result<(), ContactListFailure> {
    let book = InMemoryContactBook::default();
    let alice = contact(&book, "Alice", "alice@example.com").await;
    let bob = contact(&book, "Bob", "bob@example.net").await;
    let list = book
        .create_list(&sample_list("L", &[bob.id(), alice.id()], false))
        .await?;
    let view = GetContactList {
        lists: &book,
        contacts: &book,
    }
    .execute(list.id())
    .await?;
    assert_eq!(view.list, list);
    assert_eq!(view.members, vec![bob, alice]);
    let unknown = ContactListId::new(99).unwrap_or_else(|| panic!("id"));
    assert_eq!(
        GetContactList {
            lists: &book,
            contacts: &book
        }
        .execute(unknown)
        .await,
        Err(ContactListFailure::Store(ContactListStoreError::NotFound))
    );
    Ok(())
}

#[tokio::test]
async fn add_appends_absent_contacts_within_the_bound() -> Result<(), ContactListFailure> {
    let book = InMemoryContactBook::default();
    let alice = contact(&book, "Alice", "alice@example.com").await;
    let bob = contact(&book, "Bob", "bob@example.net").await;
    let list = book
        .create_list(&sample_list("L", &[alice.id()], false))
        .await?;
    let add = AddToContactList { book: &book };
    let updated = add.execute(list.id(), &[bob.id(), alice.id()]).await?;
    assert_eq!(updated.draft().members(), [alice.id(), bob.id()]);
    assert_eq!(book.get_list(list.id()).await?, Some(updated));
    let unknown = ContactListId::new(99).unwrap_or_else(|| panic!("id"));
    assert_eq!(
        add.execute(unknown, &[bob.id()]).await,
        Err(ContactListFailure::Store(ContactListStoreError::NotFound))
    );
    let ghost = ContactId::new(999).unwrap_or_else(|| panic!("id"));
    assert_eq!(
        add.execute(list.id(), &[ghost]).await,
        Err(ContactListFailure::Store(
            ContactListStoreError::UnknownContact(ghost)
        ))
    );
    let many: Vec<ContactId> = (1..=MAX_CONTACT_LIST_MEMBERS as i64 + 1)
        .filter_map(ContactId::new)
        .collect();
    assert_eq!(
        add.execute(list.id(), &many).await,
        Err(ContactListFailure::Invalid(
            ContactListError::TooManyMembers
        ))
    );
    Ok(())
}

#[tokio::test]
async fn expand_reads_members_at_their_preferred_address() -> Result<(), ContactListFailure> {
    let book = InMemoryContactBook::default();
    let alice = contact(&book, "Alice", "alice@example.com").await;
    let bob = contact(&book, "Bob", "bob@example.net").await;
    let list = book
        .create_list(&sample_list("L", &[bob.id(), alice.id()], true))
        .await?;
    // Adresse préférée du moment : la fiche modifiée après l'ajout.
    book.update(bob.id(), &sample_contact("Bob", &["robert@example.net"]))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let expand = ExpandContactList {
        lists: &book,
        contacts: &book,
    };
    let e = expand.execute(list.id()).await?;
    assert!(e.bcc);
    assert_eq!(
        e.recipients
            .iter()
            .map(|r| (r.name(), r.address().to_string()))
            .collect::<Vec<_>>(),
        [
            ("Bob", "robert@example.net".to_owned()),
            ("Alice", "alice@example.com".to_owned())
        ]
    );
    book.delete_many(&[alice.id(), bob.id()])
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        expand.execute(list.id()).await,
        Err(ContactListFailure::Invalid(ContactListError::Empty))
    );
    let unknown = ContactListId::new(99).unwrap_or_else(|| panic!("id"));
    assert_eq!(
        expand.execute(unknown).await,
        Err(ContactListFailure::Store(ContactListStoreError::NotFound))
    );
    Ok(())
}

#[tokio::test]
async fn suggestions_skip_empty_lists() -> Result<(), ContactListFailure> {
    let book = InMemoryContactBook::default();
    let alice = contact(&book, "Alice", "alice@example.com").await;
    let full = book
        .create_list(&sample_list("Club lecture", &[alice.id()], false))
        .await?;
    book.create_list(&sample_list("Club vide", &[], false))
        .await?;
    let suggest = SuggestContactLists { directory: &book };
    assert_eq!(
        suggest.execute("CLUB", 10).await?,
        vec![ContactListSummary::from(&full)]
    );
    assert!(suggest.execute("", 10).await?.is_empty());
    assert!(suggest.execute("zzz", 10).await?.is_empty());
    Ok(())
}

/// Annuaire qui compte ses lectures de fiches.
#[derive(Default)]
struct CountingDirectory {
    inner: InMemoryContactBook,
    /// Lectures fiche par fiche, lectures par identifiants.
    reads: std::sync::Mutex<(usize, usize)>,
}

impl CountingDirectory {
    fn reads(&self) -> (usize, usize) {
        *self
            .reads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn count(&self, one: usize, many: usize) {
        let mut reads = self
            .reads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        reads.0 += one;
        reads.1 += many;
    }
}

impl ContactDirectory for CountingDirectory {
    async fn page(
        &self,
        filter: &norelly_domain::contacts::ContactFilter,
        cursor: Option<norelly_domain::ports::ContactCursor>,
        limit: PageSize,
    ) -> Result<norelly_domain::ports::ContactPage, StoreError> {
        self.inner.page(filter, cursor, limit).await
    }

    async fn get(&self, id: ContactId) -> Result<Option<Contact>, StoreError> {
        self.count(1, 0);
        self.inner.get(id).await
    }

    async fn get_many(&self, ids: &[ContactId]) -> Result<Vec<Contact>, StoreError> {
        self.count(0, 1);
        self.inner.get_many(ids).await
    }

    async fn by_addresses(
        &self,
        keys: &[norelly_domain::contacts::AddressKey],
    ) -> Result<Vec<Contact>, StoreError> {
        self.inner.by_addresses(keys).await
    }

    async fn contacts(
        &self,
        selection: &norelly_domain::contact_export::ContactSelection,
        limit: usize,
    ) -> Result<Vec<Contact>, StoreError> {
        self.inner.contacts(selection, limit).await
    }

    async fn contact_ids(
        &self,
        selection: &norelly_domain::contact_export::ContactSelection,
        limit: usize,
    ) -> Result<Vec<ContactId>, StoreError> {
        self.inner.contact_ids(selection, limit).await
    }

    async fn photo_ids(
        &self,
    ) -> Result<std::collections::BTreeSet<norelly_domain::ports::BlobId>, StoreError> {
        self.inner.photo_ids().await
    }
}

/// Carnet qui note les listes proposées demandées et refuse de relire une
/// page entière.
#[derive(Default)]
struct SuggestionSpy {
    asked: Mutex<Vec<(String, u32)>>,
    found: Vec<ContactListSummary>,
}

impl ContactListDirectory for SuggestionSpy {
    async fn list_page(
        &self,
        _: &ContactListFilter,
        _: Option<ContactListCursor>,
        _: PageSize,
    ) -> Result<ContactListPage, StoreError> {
        Err(StoreError("page entière relue".into()))
    }

    async fn suggested_lists(
        &self,
        filter: &ContactListFilter,
        limit: PageSize,
    ) -> Result<Vec<ContactListSummary>, StoreError> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((filter.text().to_owned(), limit.get()));
        Ok(self.found.clone())
    }

    async fn get_list(&self, _: ContactListId) -> Result<Option<ContactList>, StoreError> {
        Ok(None)
    }

    async fn lists_named(&self, _: &[String]) -> Result<Vec<ContactList>, StoreError> {
        Ok(Vec::new())
    }

    async fn all_lists(&self, _: usize) -> Result<Vec<ContactList>, StoreError> {
        Ok(Vec::new())
    }
}

#[tokio::test]
async fn a_full_list_is_read_in_a_single_lookup() -> Result<(), ContactListFailure> {
    let directory = CountingDirectory::default();
    let mut members = Vec::new();
    for i in 0..MAX_CONTACT_LIST_MEMBERS {
        let c = contact(
            &directory.inner,
            &format!("C{i:03}"),
            &format!("c{i}@example.com"),
        )
        .await;
        members.push(c.id());
    }
    // Ordre de la liste, pas celui du carnet.
    members.reverse();
    let list = directory
        .inner
        .create_list(&sample_list("L", &members, false))
        .await?;

    let view = GetContactList {
        lists: &directory.inner,
        contacts: &directory,
    }
    .execute(list.id())
    .await?;
    assert_eq!(
        view.members.iter().map(Contact::id).collect::<Vec<_>>(),
        members
    );
    assert_eq!(directory.reads(), (0, 1), "lecture fiche par fiche");

    let expansion = ExpandContactList {
        lists: &directory.inner,
        contacts: &directory,
    }
    .execute(list.id())
    .await?;
    assert_eq!(expansion.recipients.len(), MAX_CONTACT_LIST_MEMBERS);
    assert_eq!(directory.reads(), (0, 2), "lecture fiche par fiche");
    Ok(())
}

/// Un membre dont la fiche est invalide n'est jamais omis en silence : la
/// liste enregistrée depuis sa vue le perdrait, un envoi partirait sans
/// lui.
#[tokio::test]
async fn an_invalid_member_fails_the_read() -> Result<(), ContactListFailure> {
    let book = InMemoryContactBook::default();
    let alice = contact(&book, "Alice", "alice@example.com").await;
    let bob = contact(&book, "Bob", "bob@example.net").await;
    let list = book
        .create_list(&sample_list("L", &[bob.id(), alice.id()], false))
        .await?;
    book.corrupt(bob.id());
    let opened = GetContactList {
        lists: &book,
        contacts: &book,
    }
    .execute(list.id())
    .await;
    assert!(
        matches!(
            opened,
            Err(ContactListFailure::Store(ContactListStoreError::Store(_)))
        ),
        "{opened:?}"
    );
    let expanded = ExpandContactList {
        lists: &book,
        contacts: &book,
    }
    .execute(list.id())
    .await;
    assert!(
        matches!(
            expanded,
            Err(ContactListFailure::Store(ContactListStoreError::Store(_)))
        ),
        "{expanded:?}"
    );
    Ok(())
}

#[tokio::test]
async fn suggestions_ask_the_book_for_a_bounded_number_of_lists() -> Result<(), StoreError> {
    let summary = |id: i64| ContactListSummary {
        id: ContactListId::new(id).unwrap_or_else(|| panic!("{id}")),
        name: format!("Club {id}"),
        members: usize::from(id != 2),
        bcc: false,
    };
    let spy = SuggestionSpy {
        found: (1..=8).map(summary).collect(),
        ..SuggestionSpy::default()
    };
    let suggest = SuggestContactLists { directory: &spy };
    // Le carnet ne rend ni plus que demandé ni de liste vide ; la règle du
    // domaine le garantit quand même.
    let found = suggest.execute(" CLUB ", 10).await?;
    assert_eq!(
        found.iter().map(|l| l.id.get()).collect::<Vec<_>>(),
        [1, 3, 4, 5, 6]
    );
    suggest.execute("club", 3).await?;
    suggest.execute("  ", 10).await?;
    suggest.execute("club", 0).await?;
    let asked = spy
        .asked
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let max = u32::try_from(MAX_LIST_SUGGESTIONS).unwrap_or(u32::MAX);
    assert_eq!(asked, [("club".to_owned(), max), ("club".to_owned(), 3)]);
    Ok(())
}
