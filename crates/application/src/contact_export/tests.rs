//! Tests de la préparation de l'export, sur les faux en mémoire.

use norelly_domain::contact_export::ContactSelection;
use norelly_domain::contacts::{Contact, ContactFilter, ContactId};
use norelly_domain::ports::{
    ContactBook, ContactCursor, ContactPage, MAX_CONTACT_LOOKUP, StoreError,
};
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
    let cards = PrepareContactExport { directory: &book }
        .execute(&ContactSelection::All, ExportFormat::VCard4)
        .await?;
    assert_eq!(names(&cards), ["Alice Martin", "Bob"]);
    // Préparées par la règle du domaine pour le format choisi.
    assert_eq!(cards[0], export_card(alice.draft(), ExportFormat::VCard4));
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
    let cards = PrepareContactExport { directory: &book }
        .execute(
            &ContactSelection::Contacts(vec![gone, bob.id()]),
            ExportFormat::GoogleCsv,
        )
        .await?;
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].full_name.as_deref(), Some("Bob"));
    Ok(())
}

#[tokio::test]
async fn nothing_to_export_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let book = InMemoryContactBook::default();
    let prepare = PrepareContactExport { directory: &book };
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

/// Carnet qui rend toujours autant de fiches qu'on lui en demande.
struct EndlessDirectory(Contact);

impl ContactDirectory for EndlessDirectory {
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
        Ok(vec![self.0.clone(); limit])
    }
}

#[tokio::test]
async fn a_book_larger_than_an_import_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let one = sample_contact("Zoé", &["zoe@example.com"]).with_id(ContactId::new(1).ok_or("id")?);
    let prepare = PrepareContactExport {
        directory: &EndlessDirectory(one),
    };
    assert_eq!(
        prepare
            .execute(&ContactSelection::All, ExportFormat::VCard3)
            .await,
        Err(ExportFailure::TooMany)
    );
    Ok(())
}
