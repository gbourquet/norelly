//! Tests des cas d'usage du carnet, sur le faux en mémoire.

use norelly_domain::address::{EmailAddress, EmailAddressError};
use norelly_domain::contacts::EmailInput;
use norelly_domain::ports::ContactListItem;
use norelly_test_support::contacts_contracts::InMemoryContactBook;

use super::*;

fn input(name: &str, emails: &[&str]) -> ContactInput {
    ContactInput {
        display_name: name.to_owned(),
        emails: emails
            .iter()
            .map(|a| EmailInput {
                address: (*a).to_owned(),
                ..EmailInput::default()
            })
            .collect(),
        ..ContactInput::default()
    }
}

fn names(page: &ContactPage) -> Vec<&str> {
    page.items.iter().map(|i| i.name.as_str()).collect()
}

#[tokio::test]
async fn create_validates_then_stores() -> Result<(), ContactFailure> {
    let book = InMemoryContactBook::default();
    let created = CreateContact { book: &book }
        .execute(input(" Alice ", &["alice@example.com"]))
        .await?;
    assert_eq!(created.draft().display_name(), "Alice");
    assert_eq!(book.get(created.id()).await?, Some(created));
    Ok(())
}

#[tokio::test]
async fn invalid_input_writes_nothing() -> Result<(), ContactFailure> {
    let book = InMemoryContactBook::default();
    assert_eq!(
        CreateContact { book: &book }
            .execute(input("Alice", &["alice"]))
            .await,
        Err(ContactFailure::Invalid(ContactError::InvalidAddress {
            index: 0,
            error: EmailAddressError::MissingAt
        }))
    );
    assert_eq!(
        book.page(&ContactFilter::default(), None, 10).await?,
        ContactPage::default()
    );
    Ok(())
}

#[tokio::test]
async fn duplicate_address_is_reported() -> Result<(), ContactFailure> {
    let book = InMemoryContactBook::default();
    let create = CreateContact { book: &book };
    create.execute(input("A", &["a@example.com"])).await?;
    let Ok(taken) = EmailAddress::parse("A@example.com") else {
        panic!("adresse");
    };
    assert_eq!(
        create.execute(input("B", &["A@example.com"])).await,
        Err(ContactFailure::Store(ContactStoreError::DuplicateAddress(
            taken
        )))
    );
    Ok(())
}

#[tokio::test]
async fn update_validates_then_replaces() -> Result<(), ContactFailure> {
    let book = InMemoryContactBook::default();
    let created = CreateContact { book: &book }
        .execute(input("Alice", &["alice@example.com"]))
        .await?;
    let update = UpdateContact { book: &book };
    let updated = update
        .execute(created.id(), input("Alice Martin", &["alice@example.com"]))
        .await?;
    assert_eq!(updated.id(), created.id());
    assert_eq!(book.get(created.id()).await?, Some(updated.clone()));
    // Saisie invalide : fiche inchangée.
    assert_eq!(
        update.execute(created.id(), input("Alice", &[])).await,
        Err(ContactFailure::Invalid(ContactError::NoAddress))
    );
    assert_eq!(book.get(created.id()).await?, Some(updated));
    let Some(unknown) = ContactId::new(99) else {
        panic!("id");
    };
    assert_eq!(
        update
            .execute(unknown, input("X", &["x@example.com"]))
            .await,
        Err(ContactFailure::Store(ContactStoreError::NotFound))
    );
    Ok(())
}

#[tokio::test]
async fn delete_list_and_get() -> Result<(), ContactFailure> {
    let book = InMemoryContactBook::default();
    let create = CreateContact { book: &book };
    let a = create
        .execute(input("Alice", &["alice@example.com"]))
        .await?;
    let b = create.execute(input("Bob", &["bob@example.com"])).await?;
    let mut fav = input("Carole", &["carole@example.com"]);
    fav.favorite = true;
    let c = create.execute(fav).await?;

    let list = ListContacts { directory: &book };
    assert_eq!(
        names(&list.execute("", false, None, 10).await?),
        vec!["Alice", "Bob", "Carole"]
    );
    assert_eq!(
        names(&list.execute("  BO ", false, None, 10).await?),
        vec!["Bob"]
    );
    assert_eq!(
        names(&list.execute("", true, None, 10).await?),
        vec!["Carole"]
    );
    let first = list.execute("", false, None, 1).await?;
    assert_eq!(first.items, vec![ContactListItem::from(&a)]);
    assert_eq!(
        names(&list.execute("", false, first.next, 10).await?),
        vec!["Bob", "Carole"]
    );

    let get = GetContact { directory: &book };
    assert_eq!(get.execute(b.id()).await?, b);

    assert_eq!(
        DeleteContacts { book: &book }
            .execute(&[a.id(), c.id()])
            .await?,
        2
    );
    assert_eq!(
        names(&list.execute("", false, None, 10).await?),
        vec!["Bob"]
    );
    assert_eq!(
        get.execute(a.id()).await,
        Err(ContactFailure::Store(ContactStoreError::NotFound))
    );
    Ok(())
}

#[tokio::test]
async fn senders_are_named_from_the_book() -> Result<(), ContactFailure> {
    use norelly_domain::mailbox::Correspondent;

    let book = InMemoryContactBook::default();
    let alice = CreateContact { book: &book }
        .execute(input("Alice Martin", &["alice@example.com"]))
        .await?;
    let people = [
        Correspondent::new("Bob", "bob@example.com"),
        Correspondent::new("alice", "ALICE@example.com"),
        Correspondent::new("x", "pas une adresse"),
    ];
    let names = NameSenders { directory: &book }.execute(&people).await?;
    let alice_label = names.label(&people[1]);
    assert_eq!(alice_label.name, "Alice Martin");
    assert_eq!(alice_label.contact, Some(alice.id()));
    assert_eq!(names.label(&people[0]).contact, None);
    assert_eq!(names.label(&people[0]).name, "Bob");
    Ok(())
}
