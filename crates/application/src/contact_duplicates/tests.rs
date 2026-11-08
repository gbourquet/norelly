//! Tests des doublons du carnet, sur les faux en mémoire.

use norelly_domain::contact_duplicates::{DuplicateReason, MergeSelection, merge_contacts};
use norelly_domain::contacts::{AddressKey, ContactFilter};
use norelly_domain::contacts::{ContactError, ContactId, ContactInput, EmailInput};
use norelly_domain::ports::{
    ContactBook, ContactCursor, ContactPage, MAX_CONTACT_LOOKUP, StoreError,
};
use norelly_test_support::contact_import_contracts::FakeNfc;
use norelly_test_support::contacts_contracts::{InMemoryContactBook, sample_contact};

use super::*;

fn names(group: &DuplicateCandidates) -> Vec<String> {
    group
        .contacts
        .iter()
        .map(|c| c.draft().preferred_email().address().to_string())
        .collect()
}

#[tokio::test]
async fn duplicates_of_the_book_are_found_with_their_cards()
-> Result<(), Box<dyn std::error::Error>> {
    let book = InMemoryContactBook::default();
    book.create(&sample_contact("Alice", &["a1@example.com"]))
        .await?;
    book.create(&sample_contact("Bob", &["bob@example.net"]))
        .await?;
    book.create(&sample_contact(
        "alice",
        &["a2@example.com", "a3@example.com"],
    ))
    .await?;
    let groups = FindDuplicates {
        directory: &book,
        normalizer: &FakeNfc,
    }
    .execute()
    .await?;
    assert_eq!(groups.len(), 1);
    // La plus complète d'abord.
    assert_eq!(names(&groups[0]), ["a2@example.com", "a1@example.com"]);
    assert_eq!(groups[0].reasons, [DuplicateReason::SameName]);
    Ok(())
}

/// Annuaire qui annonce un carnet plus grand que la borne.
struct HugeBook;

impl ContactDirectory for HugeBook {
    async fn page(
        &self,
        _: &ContactFilter,
        _: Option<ContactCursor>,
        _: u32,
    ) -> Result<ContactPage, StoreError> {
        Ok(ContactPage::default())
    }

    async fn get(&self, _: ContactId) -> Result<Option<Contact>, StoreError> {
        Ok(None)
    }

    async fn by_addresses(&self, _: &[AddressKey]) -> Result<Vec<Contact>, StoreError> {
        let _ = MAX_CONTACT_LOOKUP;
        Ok(Vec::new())
    }

    async fn contacts(
        &self,
        _: &ContactSelection,
        limit: usize,
    ) -> Result<Vec<Contact>, StoreError> {
        let id = ContactId::new(1).ok_or_else(|| StoreError("id".into()))?;
        Ok(vec![
            sample_contact("X", &["x@example.com"]).with_id(id);
            limit
        ])
    }
}

#[tokio::test]
async fn a_book_beyond_the_scan_bound_is_refused() {
    let result = FindDuplicates {
        directory: &HugeBook,
        normalizer: &FakeNfc,
    }
    .execute()
    .await;
    assert_eq!(result, Err(DuplicateScanFailure::TooMany));
}

fn ok<T, E: std::fmt::Display>(r: Result<T, E>) -> T {
    r.unwrap_or_else(|e| panic!("{e}"))
}

#[tokio::test]
async fn merging_writes_the_domain_merge_and_removes_the_others()
-> Result<(), Box<dyn std::error::Error>> {
    let book = InMemoryContactBook::default();
    let keep = book
        .create(&sample_contact("Alice", &["a1@example.com"]))
        .await?;
    let other = book
        .create(&ContactDraft::new(ContactInput {
            organization: "ACME".into(),
            notes: "note".into(),
            emails: vec![EmailInput {
                address: "a2@example.com".into(),
                ..EmailInput::default()
            }],
            ..ContactInput::default()
        })?)
        .await?;
    let selection = ok(MergeSelection::new(keep.id().get(), &[other.id().get()]));
    let merged = MergeContacts { book: &book }.execute(&selection).await?;
    let expected = ok(merge_contacts(keep.draft(), &[other.draft()]));
    assert_eq!(merged, expected.with_id(keep.id()));
    assert_eq!(book.get(other.id()).await?, None);
    assert_eq!(book.get(keep.id()).await?, Some(merged));
    Ok(())
}

#[tokio::test]
async fn merging_an_unknown_card_writes_nothing() -> Result<(), Box<dyn std::error::Error>> {
    let book = InMemoryContactBook::default();
    let keep = book
        .create(&sample_contact("Alice", &["a1@example.com"]))
        .await?;
    let selection = ok(MergeSelection::new(keep.id().get(), &[9_999]));
    assert_eq!(
        MergeContacts { book: &book }.execute(&selection).await,
        Err(ContactFailure::Store(ContactStoreError::NotFound))
    );
    let selection = ok(MergeSelection::new(9_999_i64, &[keep.id().get()]));
    assert_eq!(
        MergeContacts { book: &book }.execute(&selection).await,
        Err(ContactFailure::Store(ContactStoreError::NotFound))
    );
    assert_eq!(book.get(keep.id()).await?, Some(keep));
    Ok(())
}

#[tokio::test]
async fn a_merge_beyond_the_bounds_of_a_card_is_refused_and_writes_nothing()
-> Result<(), Box<dyn std::error::Error>> {
    let book = InMemoryContactBook::default();
    let many = |prefix: &str, n: usize| {
        let addresses: Vec<String> = (0..n).map(|i| format!("{prefix}{i}@example.com")).collect();
        let refs: Vec<&str> = addresses.iter().map(String::as_str).collect();
        sample_contact("Alice", &refs)
    };
    let keep = book.create(&many("a", 15)).await?;
    let other = book.create(&many("b", 6)).await?;
    let selection = ok(MergeSelection::new(keep.id().get(), &[other.id().get()]));
    assert_eq!(
        MergeContacts { book: &book }.execute(&selection).await,
        Err(ContactFailure::Invalid(ContactError::TooManyAddresses))
    );
    assert!(book.get(other.id()).await?.is_some());
    Ok(())
}

#[tokio::test]
async fn the_preview_is_the_domain_merge_and_writes_nothing()
-> Result<(), Box<dyn std::error::Error>> {
    let book = InMemoryContactBook::default();
    let keep = book
        .create(&sample_contact("", &["a1@example.com"]))
        .await?;
    let other = book
        .create(&sample_contact("Alice", &["a2@example.com"]))
        .await?;
    let selection = ok(MergeSelection::new(keep.id().get(), &[other.id().get()]));
    let preview = PreviewMerge { directory: &book }
        .execute(&selection)
        .await?;
    assert_eq!(preview, ok(merge_contacts(keep.draft(), &[other.draft()])));
    // Le nom vient de l'autre fiche, l'adresse préférée de la fiche gardée.
    assert_eq!(preview.display_name(), "Alice");
    assert_eq!(
        preview.preferred_email().address().to_string(),
        "a1@example.com"
    );
    assert_eq!(book.get(other.id()).await?, Some(other));
    assert_eq!(book.get(keep.id()).await?, Some(keep));
    Ok(())
}

#[tokio::test]
async fn a_preview_beyond_the_bounds_of_a_card_is_refused() -> Result<(), Box<dyn std::error::Error>>
{
    let book = InMemoryContactBook::default();
    let many = |prefix: &str, n: usize| {
        let addresses: Vec<String> = (0..n).map(|i| format!("{prefix}{i}@example.com")).collect();
        let refs: Vec<&str> = addresses.iter().map(String::as_str).collect();
        sample_contact("Alice", &refs)
    };
    let keep = book.create(&many("a", 15)).await?;
    let other = book.create(&many("b", 6)).await?;
    let selection = ok(MergeSelection::new(keep.id().get(), &[other.id().get()]));
    assert_eq!(
        PreviewMerge { directory: &book }.execute(&selection).await,
        Err(ContactFailure::Invalid(ContactError::TooManyAddresses))
    );
    Ok(())
}
