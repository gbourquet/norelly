//! Tests de l'import du carnet, sur les faux en mémoire.

use std::sync::{Mutex, MutexGuard, PoisonError};

use norelly_domain::contact_export::ContactSelection;
use norelly_domain::contact_import::{
    ContactCompletion, DecodedContacts, MAX_IMPORT_ATTEMPTS, RejectReason,
};
use norelly_domain::contacts::{
    AddressKey, ContactDraft, ContactFilter, ContactId, ContactInput, EmailInput,
};
use norelly_domain::ports::{ContactCursor, ContactPage, MAX_CONTACT_LOOKUP};
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
    FakeContactCodec::new(
        b"fichier".to_vec(),
        DecodedContacts {
            format: ContactFormat::VCard,
            cards,
        },
    )
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
}

fn count(cell: &Mutex<usize>) -> MutexGuard<'_, usize> {
    cell.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ContactDirectory for EditedMeanwhile {
    async fn page(
        &self,
        filter: &ContactFilter,
        cursor: Option<ContactCursor>,
        limit: u32,
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
    ) -> Result<ImportOutcome, ContactStoreError> {
        *count(&self.imports) += 1;
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
        self.inner.import(create, complete).await
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
    }
    .execute(&cards, DuplicatePolicy::Complete)
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
    }
    .execute(&cards, DuplicatePolicy::Complete)
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
    }
    .execute(&cards, DuplicatePolicy::Complete)
    .await?;
    assert_eq!(
        outcome,
        ImportOutcome {
            created: 1,
            completed: 0
        }
    );
    assert_eq!(names(&book.inner).await?, ["Alice"]);
    Ok(())
}
