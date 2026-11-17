//! Suite de contrat des groupes de doublons écartés
//! ([`DuplicateDismissals`]), jouée contre le faux carnet en mémoire
//! ([`InMemoryContactBook`]) et contre SQLite.

use norelly_domain::contact_duplicates::{DismissedPair, MAX_DISMISSED_PAIRS};
use norelly_domain::contact_import::ContactCompletion;
use norelly_domain::contacts::{Contact, ContactId};
use norelly_domain::ports::{
    ContactBook, ContactDirectory, ContactStoreError, DuplicateDismissals, StoreError,
};

use crate::contacts_contracts::sample_contact;

fn pair(a: ContactId, b: ContactId) -> DismissedPair {
    DismissedPair::new(a, b).unwrap_or_else(|| panic!("paire {a}, {b}"))
}

/// Paires retenues, triées.
async fn sorted<B: DuplicateDismissals>(book: &B) -> Result<Vec<DismissedPair>, StoreError> {
    let mut pairs = book.dismissed().await?;
    pairs.sort_unstable();
    Ok(pairs)
}

async fn create<B: ContactBook>(book: &B, n: usize) -> Result<Vec<Contact>, ContactStoreError> {
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let address = format!("fiche{i}@example.com");
        out.push(book.create(&sample_contact("Fiche", &[&address])).await?);
    }
    Ok(out)
}

/// Contrat des paires écartées (carnet vide au départ).
pub async fn duplicate_dismissals_contract<B>(book: &B) -> Result<(), ContactStoreError>
where
    B: ContactBook + ContactDirectory + DuplicateDismissals,
{
    assert!(book.dismissed().await?.is_empty(), "paires au départ");
    let c = create(book, 8).await?;
    let id = |i: usize| c[i].id();

    // Retenues, chacune une fois ; une paire d'une fiche inconnue ignorée.
    let unknown = ContactId::new(id(7).get() + 1_000).unwrap_or_else(|| panic!("identifiant"));
    book.dismiss(&[pair(id(0), id(1)), pair(id(2), id(3))])
        .await?;
    book.dismiss(&[pair(id(1), id(0)), pair(id(4), unknown)])
        .await?;
    book.dismiss(&[]).await?;
    assert_eq!(
        sorted(book).await?,
        [pair(id(0), id(1)), pair(id(2), id(3))]
    );

    // Une fiche modifiée : ses paires sont oubliées, pas les autres.
    let edited = sample_contact("Fiche modifiée", &["fiche0@example.com"]);
    book.update(id(0), &edited).await?;
    assert_eq!(sorted(book).await?, [pair(id(2), id(3))]);

    // Une fiche supprimée, une à une ou en lot.
    book.dismiss(&[pair(id(4), id(5)), pair(id(5), id(6))])
        .await?;
    book.delete(id(3)).await?;
    assert_eq!(
        sorted(book).await?,
        [pair(id(4), id(5)), pair(id(5), id(6))]
    );
    book.delete_many(&[id(6)]).await?;
    assert_eq!(sorted(book).await?, [pair(id(4), id(5))]);

    // Une fiche complétée par un import.
    let read = book
        .get(id(4))
        .await?
        .unwrap_or_else(|| panic!("fiche 4 perdue"));
    let completed = sample_contact("Fiche complétée", &["fiche4@example.com"]);
    book.import(&[], &[ContactCompletion::new(read, completed)])
        .await?;
    assert!(sorted(book).await?.is_empty(), "import sans effet");

    // Une fusion : fiche gardée modifiée, fiche absorbée supprimée.
    book.dismiss(&[pair(id(1), id(2)), pair(id(2), id(7)), pair(id(5), id(7))])
        .await?;
    let keep = book
        .get(id(1))
        .await?
        .unwrap_or_else(|| panic!("fiche 1 perdue"));
    let gone = book
        .get(id(2))
        .await?
        .unwrap_or_else(|| panic!("fiche 2 perdue"));
    let merged = sample_contact("Fiche", &["fiche1@example.com", "fiche2@example.com"]);
    book.merge(&keep, &merged, &[gone]).await?;
    assert_eq!(sorted(book).await?, [pair(id(5), id(7))]);
    Ok(())
}

/// Contrat de la borne des paires retenues : au-delà de
/// [`MAX_DISMISSED_PAIRS`], les plus anciennes sont oubliées d'abord.
pub async fn duplicate_dismissals_bound_contract<B>(book: &B) -> Result<(), ContactStoreError>
where
    B: ContactBook + ContactDirectory + DuplicateDismissals,
{
    // Assez de fiches pour former plus de paires que la borne.
    let mut n = 2;
    while n * (n - 1) / 2 < MAX_DISMISSED_PAIRS + 10 {
        n += 1;
    }
    let c = create(book, n).await?;
    let mut all: Vec<DismissedPair> = Vec::new();
    for (at, a) in c.iter().enumerate() {
        for b in &c[at + 1..] {
            all.push(pair(a.id(), b.id()));
        }
    }
    let (first, rest) = all.split_at(MAX_DISMISSED_PAIRS);
    book.dismiss(first).await?;
    assert_eq!(book.dismissed().await?.len(), MAX_DISMISSED_PAIRS);
    // La plus ancienne, retenue de nouveau, redevient la plus récente.
    book.dismiss(&first[..1]).await?;
    book.dismiss(&rest[..5]).await?;
    let kept = sorted(book).await?;
    assert_eq!(kept.len(), MAX_DISMISSED_PAIRS, "borne dépassée");
    assert!(kept.contains(&first[0]), "paire retenue de nouveau oubliée");
    for old in &first[1..6] {
        assert!(!kept.contains(old), "plus ancienne paire gardée");
    }
    for new in &rest[..5] {
        assert!(kept.contains(new), "paire récente oubliée");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contacts_contracts::InMemoryContactBook;

    #[tokio::test]
    async fn in_memory_contact_book_passes_dismissals_contract() -> Result<(), ContactStoreError> {
        duplicate_dismissals_contract(&InMemoryContactBook::default()).await
    }

    #[tokio::test]
    async fn in_memory_contact_book_passes_dismissals_bound_contract()
    -> Result<(), ContactStoreError> {
        duplicate_dismissals_bound_contract(&InMemoryContactBook::default()).await
    }
}
