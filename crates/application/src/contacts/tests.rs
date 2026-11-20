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
        book.page(&ContactFilter::default(), None, PageSize::new(10))
            .await?,
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
        names(&list.execute("", false, None, PageSize::new(10)).await?),
        vec!["Alice", "Bob", "Carole"]
    );
    assert_eq!(
        names(
            &list
                .execute("  BO ", false, None, PageSize::new(10))
                .await?
        ),
        vec!["Bob"]
    );
    assert_eq!(
        names(&list.execute("", true, None, PageSize::new(10)).await?),
        vec!["Carole"]
    );
    let first = list.execute("", false, None, PageSize::new(1)).await?;
    assert_eq!(first.items, vec![ContactListItem::from(&a)]);
    assert_eq!(
        names(
            &list
                .execute("", false, first.next, PageSize::new(10))
                .await?
        ),
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
        names(&list.execute("", false, None, PageSize::new(10)).await?),
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
    let names = NameSenders { directory: &book }.execute(&people).await;
    let alice_label = names.label(&people[1]);
    assert_eq!(alice_label.name, "Alice Martin");
    assert_eq!(alice_label.contact, Some(alice.id()));
    assert_eq!(names.label(&people[0]).contact, None);
    assert_eq!(names.label(&people[0]).name, "Bob");
    Ok(())
}

/// Une fiche invalide en base n'écarte qu'elle-même : les autres
/// correspondants restent nommés d'après le carnet, et les autres contacts
/// restent proposés pendant la saisie.
#[tokio::test]
async fn an_invalid_contact_only_leaves_out_itself() -> Result<(), ContactFailure> {
    use norelly_domain::mailbox::Correspondent;

    let book = InMemoryContactBook::default();
    let create = CreateContact { book: &book };
    let alice = create
        .execute(input("Alice Martin", &["alice@example.com"]))
        .await?;
    let bob = create
        .execute(input("Bob Martin", &["bob@example.com"]))
        .await?;
    book.corrupt(bob.id());

    let people = [
        Correspondent::new("alice", "alice@example.com"),
        Correspondent::new("bob", "bob@example.com"),
    ];
    let names = NameSenders { directory: &book }.execute(&people).await;
    assert_eq!(names.label(&people[0]).name, "Alice Martin");
    assert_eq!(names.label(&people[0]).contact, Some(alice.id()));
    assert_eq!(names.label(&people[1]).name, "bob");
    assert_eq!(names.label(&people[1]).contact, None);

    let found = SuggestRecipients {
        collected: &Collected(Vec::new()),
        directory: &book,
    }
    .execute(account(), "martin", 10)
    .await?;
    assert_eq!(
        suggested(&found),
        vec![("Alice Martin", "alice@example.com", true)]
    );
    Ok(())
}

/// Carnet dont toute lecture échoue.
struct BrokenDirectory;

impl ContactDirectory for BrokenDirectory {
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
        Err(StoreError("carnet illisible".into()))
    }

    async fn get(&self, _: ContactId) -> Result<Option<Contact>, StoreError> {
        Err(StoreError("carnet illisible".into()))
    }

    async fn by_addresses(
        &self,
        _: &[norelly_domain::contacts::AddressKey],
    ) -> Result<Vec<Contact>, StoreError> {
        Err(StoreError("carnet illisible".into()))
    }

    async fn contacts(
        &self,
        _: &norelly_domain::contact_export::ContactSelection,
        _: usize,
    ) -> Result<Vec<Contact>, StoreError> {
        Err(StoreError("carnet illisible".into()))
    }

    async fn contact_ids(
        &self,
        _: &norelly_domain::contact_export::ContactSelection,
        _: usize,
    ) -> Result<Vec<ContactId>, StoreError> {
        Err(StoreError("carnet illisible".into()))
    }
}

#[tokio::test]
async fn an_unreadable_book_leaves_the_header_names() {
    use norelly_domain::mailbox::Correspondent;

    let people = [Correspondent::new("Bob", "bob@example.com")];
    let names = NameSenders {
        directory: &BrokenDirectory,
    }
    .execute(&people)
    .await;
    // Nommer d'après le carnet enrichit l'affichage : un carnet illisible
    // ne doit pas empêcher de lire le courrier.
    assert_eq!(names.label(&people[0]).name, "Bob");
    assert_eq!(names.label(&people[0]).contact, None);
}

/// Adresses collectées fixes, quelle que soit la saisie.
struct Collected(Vec<Correspondent>);

impl ContactQuery for Collected {
    async fn suggest(
        &self,
        _: AccountId,
        _: &str,
        limit: usize,
    ) -> Result<Vec<Correspondent>, StoreError> {
        Ok(self.0.iter().take(limit).cloned().collect())
    }
}

fn account() -> AccountId {
    AccountId::new(1).unwrap_or_else(|_| panic!("compte"))
}

fn suggested(found: &[RecipientSuggestion]) -> Vec<(&str, &str, bool)> {
    found
        .iter()
        .map(|s| (s.name.as_str(), s.address.as_str(), s.contact.is_some()))
        .collect()
}

/// Prénom et nom dans deux champs : « alice mar » propose Alice Martin,
/// sous son nom du carnet, même quand un autre nom affiché est saisi.
#[tokio::test]
async fn suggestions_find_a_contact_by_its_full_name() -> Result<(), ContactFailure> {
    let book = InMemoryContactBook::default();
    let create = CreateContact { book: &book };
    let mut alice = input("", &["alice@example.com"]);
    alice.given_name = "Alice".into();
    alice.family_name = "Martin".into();
    create.execute(alice.clone()).await?;
    alice.display_name = "Lili".into();
    alice.emails[0].address = "lili@example.com".into();
    create.execute(alice).await?;
    let found = SuggestRecipients {
        collected: &Collected(Vec::new()),
        directory: &book,
    }
    .execute(account(), "alice mar", 10)
    .await?;
    assert_eq!(
        suggested(&found),
        [
            ("Alice Martin", "alice@example.com", true),
            ("Lili", "lili@example.com", true),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn suggestions_put_contacts_first_then_collected_addresses() -> Result<(), ContactFailure> {
    let book = InMemoryContactBook::default();
    let create = CreateContact { book: &book };
    // Favori classé après les autres par l'ordre alphabétique, et une
    // seconde adresse qui n'est pas dans la ligne de liste.
    let mut zoe = input("Zoé Martin", &["zoe@example.com", "zoe.perso@exemple.fr"]);
    zoe.favorite = true;
    create.execute(zoe).await?;
    create
        .execute(input("Alice Martin", &["alice@example.com"]))
        .await?;
    let collected = Collected(vec![
        Correspondent::new("alice", "ALICE@example.com"),
        Correspondent::new("Marc Martin", "marc@example.com"),
    ]);
    let found = SuggestRecipients {
        collected: &collected,
        directory: &book,
    }
    .execute(account(), "martin", 10)
    .await?;
    assert_eq!(
        suggested(&found),
        [
            ("Zoé Martin", "zoe@example.com", true),
            ("Zoé Martin", "zoe.perso@exemple.fr", true),
            ("Alice Martin", "alice@example.com", true),
            ("Marc Martin", "marc@example.com", false),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn many_favorites_and_contacts_stay_bounded() -> Result<(), ContactFailure> {
    use norelly_domain::ports::MAX_CONTACT_SUGGESTIONS;

    let book = InMemoryContactBook::default();
    let create = CreateContact { book: &book };
    for i in 0..30 {
        let mut c = input(&format!("Martin {i:02}"), &[&format!("m{i}@example.com")]);
        c.favorite = i >= 25;
        create.execute(c).await?;
    }
    let found = SuggestRecipients {
        collected: &Collected(vec![]),
        directory: &book,
    }
    .execute(account(), "martin", usize::MAX)
    .await?;
    assert_eq!(found.len(), MAX_CONTACT_SUGGESTIONS);
    // Les favoris d'abord, même classés en fin de liste.
    assert_eq!(found[0].name, "Martin 25");
    assert_eq!(found[5].name, "Martin 00");
    Ok(())
}

#[tokio::test]
async fn an_unreadable_book_leaves_the_collected_addresses() -> Result<(), StoreError> {
    let collected = Collected(vec![Correspondent::new("Bob", "bob@example.com")]);
    let found = SuggestRecipients {
        collected: &collected,
        directory: &BrokenDirectory,
    }
    .execute(account(), "bob", 10)
    .await?;
    assert_eq!(suggested(&found), [("Bob", "bob@example.com", false)]);
    Ok(())
}

#[tokio::test]
async fn an_empty_text_reads_nothing() -> Result<(), StoreError> {
    let collected = Collected(vec![Correspondent::new("Bob", "bob@example.com")]);
    let found = SuggestRecipients {
        collected: &collected,
        directory: &BrokenDirectory,
    }
    .execute(account(), "  ", 10)
    .await?;
    assert!(found.is_empty());
    Ok(())
}

/// Adresses collectées : garde le texte reçu.
#[derive(Default)]
struct Recording(std::sync::Mutex<Vec<String>>);

impl ContactQuery for Recording {
    async fn suggest(
        &self,
        _: AccountId,
        text: &str,
        _: usize,
    ) -> Result<Vec<Correspondent>, StoreError> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(text.to_owned());
        Ok(Vec::new())
    }
}

#[tokio::test]
async fn collected_addresses_receive_the_bounded_text() -> Result<(), StoreError> {
    use norelly_domain::contacts::MAX_CONTACT_FILTER_CHARS;

    let collected = Recording::default();
    let book = InMemoryContactBook::default();
    let typed = format!("  {}  ", "a".repeat(10 * MAX_CONTACT_FILTER_CHARS));
    SuggestRecipients {
        collected: &collected,
        directory: &book,
    }
    .execute(account(), &typed, 10)
    .await?;
    let seen = collected
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert_eq!(seen, ["a".repeat(MAX_CONTACT_FILTER_CHARS)]);
    Ok(())
}
