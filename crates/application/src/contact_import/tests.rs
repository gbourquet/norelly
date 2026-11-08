//! Tests de l'import du carnet, sur les faux en mémoire.

use norelly_domain::contact_import::{DecodedContacts, RejectReason};
use norelly_domain::contacts::{ContactFilter, ContactInput, EmailInput};
use norelly_domain::ports::MAX_CONTACT_LOOKUP;
use norelly_test_support::contact_import_contracts::{FakeContactCodec, FakeNfc};
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
    FakeContactCodec {
        known: b"fichier".to_vec(),
        decoded: DecodedContacts {
            format: ContactFormat::VCard,
            cards,
        },
    }
}

async fn names(book: &InMemoryContactBook) -> Result<Vec<String>, StoreError> {
    let page = book.page(&ContactFilter::default(), None, 100).await?;
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
        }
        .execute(&cards, policy)
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
    }
    .execute(&cards, DuplicatePolicy::Skip)
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
    }
    .execute(&cards, DuplicatePolicy::Skip)
    .await?;
    assert_eq!(outcome.created, count - 1);
    Ok(())
}
