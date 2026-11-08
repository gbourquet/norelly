//! Tests des cas d'usage des listes, sur le faux en mémoire.

use norelly_domain::contact_lists::{ContactListField, MAX_CONTACT_LIST_MEMBERS};
use norelly_domain::ports::ContactBook;
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
    let created = CreateContactList { book: &book }
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
        CreateContactList { book: &book }
            .execute(input(" ", &[], false))
            .await,
        Err(ContactListFailure::Invalid(ContactListError::NoName))
    );
    assert_eq!(
        CreateContactList { book: &book }
            .execute(input(&"x".repeat(1000), &[], false))
            .await,
        Err(ContactListFailure::Invalid(ContactListError::TooLong(
            ContactListField::Name
        )))
    );
    assert_eq!(
        ListContactLists { directory: &book }
            .execute("", None, 10)
            .await?,
        ContactListPage::default()
    );
    Ok(())
}

#[tokio::test]
async fn update_validates_then_replaces_and_reports_refusals() -> Result<(), ContactListFailure> {
    let book = InMemoryContactBook::default();
    let alice = contact(&book, "Alice", "alice@example.com").await;
    let create = CreateContactList { book: &book };
    let list = create.execute(input("Famille", &[], false)).await?;
    create.execute(input("Club", &[], false)).await?;
    let update = UpdateContactList { book: &book };
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
    let create = CreateContactList { book: &book };
    let a = create
        .execute(input("Famille", &[alice.id()], false))
        .await?;
    create.execute(input("Club", &[], false)).await?;
    let page = ListContactLists { directory: &book }
        .execute(" FAM ", None, 10)
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
            .map(|r| (r.name.as_str(), r.address.to_string()))
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
