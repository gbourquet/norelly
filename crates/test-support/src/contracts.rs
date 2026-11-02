//! Suites de contrat des ports : chaque implémentation (faux en mémoire et
//! adaptateur réel) doit les passer.
//!
//! Les fonctions paniquent (assertions) en cas de violation du contrat, et
//! renvoient une erreur si l'implémentation échoue.

use std::collections::BTreeSet;
use std::sync::Mutex;

use norelly_domain::account::{Account, AccountDraft, AccountId, DisplayName, ServerSettings};
use norelly_domain::address::EmailAddress;
use norelly_domain::credentials::{Secret, Username};
use norelly_domain::endpoint::{Host, Protocol, Security, ServerEndpoint};
use norelly_domain::ports::{
    AccountDataStore, AccountRepository, AccountStoreError, CertificateExceptionStore, SecretKey,
    SecretStore, SecretStoreError, StoreError,
};
use norelly_domain::security::{CertificateException, CertificateFingerprint};

/// Contrat de [`SecretStore`]. `prefix` isole les clés du test (utile
/// contre un vrai trousseau partagé) ; toutes les clés créées sont
/// supprimées à la fin.
pub async fn secret_store_contract<S: SecretStore>(
    store: &S,
    prefix: &str,
) -> Result<(), SecretStoreError> {
    let key = |name: &str| {
        SecretKey::parse(&format!("{prefix}{name}"))
            .unwrap_or_else(|_| panic!("clé de test invalide : {prefix}{name}"))
    };
    let (a, b) = (key("a"), key("b"));

    // Absent au départ ; delete d'un absent sans effet.
    store.delete(&a).await?;
    store.delete(&b).await?;
    assert!(store.get(&a).await?.is_none(), "clé absente lue");

    // set puis get.
    store
        .set(&a, &Secret::new("première valeur".to_owned()))
        .await?;
    assert_eq!(
        store.get(&a).await?.as_ref().map(Secret::expose),
        Some("première valeur")
    );

    // Remplacement.
    store
        .set(&a, &Secret::new("deuxième «valeur» ✓".to_owned()))
        .await?;
    assert_eq!(
        store.get(&a).await?.as_ref().map(Secret::expose),
        Some("deuxième «valeur» ✓")
    );

    // Isolation entre clés.
    assert!(store.get(&b).await?.is_none(), "fuite entre clés");
    store.set(&b, &Secret::new("autre".to_owned())).await?;
    assert_eq!(
        store.get(&a).await?.as_ref().map(Secret::expose),
        Some("deuxième «valeur» ✓")
    );

    // Suppression, idempotente.
    store.delete(&a).await?;
    assert!(store.get(&a).await?.is_none(), "secret supprimé encore lu");
    store.delete(&a).await?;
    assert_eq!(
        store.get(&b).await?.as_ref().map(Secret::expose),
        Some("autre")
    );
    store.delete(&b).await?;
    Ok(())
}

/// Contrat de [`CertificateExceptionStore`].
pub async fn certificate_exception_store_contract<S: CertificateExceptionStore>(
    store: &S,
) -> Result<(), StoreError> {
    let host = |h: &str| Host::parse(h).unwrap_or_else(|_| panic!("hôte de test invalide"));
    let exc = |h: &str, b: u8| CertificateException {
        host: host(h),
        fingerprint: CertificateFingerprint::from_bytes([b; 32]),
    };
    let set = |v: Vec<CertificateException>| v.into_iter().collect::<BTreeSet<_>>();

    assert!(store.exceptions_for(&host("a.test")).await?.is_empty());

    store.add(&exc("a.test", 1)).await?;
    store.add(&exc("a.test", 1)).await?; // idempotent
    store.add(&exc("a.test", 2)).await?;
    store.add(&exc("b.test", 3)).await?;

    assert_eq!(
        set(store.exceptions_for(&host("A.test")).await?),
        set(vec![exc("a.test", 1), exc("a.test", 2)])
    );
    assert_eq!(
        store.exceptions_for(&host("b.test")).await?,
        vec![exc("b.test", 3)]
    );

    store.remove(&exc("a.test", 1)).await?;
    store.remove(&exc("a.test", 1)).await?; // idempotent
    assert_eq!(
        store.exceptions_for(&host("a.test")).await?,
        vec![exc("a.test", 2)]
    );
    store.remove(&exc("a.test", 2)).await?;
    store.remove(&exc("b.test", 3)).await?;
    assert!(store.exceptions_for(&host("a.test")).await?.is_empty());
    assert!(store.exceptions_for(&host("b.test")).await?.is_empty());
    Ok(())
}

/// Faux [`CertificateExceptionStore`] en mémoire. L'implémentation réelle
/// (SQLite) passe la même suite.
#[derive(Debug, Default)]
pub struct InMemoryCertificateExceptionStore {
    exceptions: Mutex<BTreeSet<CertificateException>>,
}

impl InMemoryCertificateExceptionStore {
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeSet<CertificateException>> {
        self.exceptions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl CertificateExceptionStore for InMemoryCertificateExceptionStore {
    async fn exceptions_for(&self, host: &Host) -> Result<Vec<CertificateException>, StoreError> {
        Ok(self
            .lock()
            .iter()
            .filter(|e| &e.host == host)
            .cloned()
            .collect())
    }

    async fn add(&self, exception: &CertificateException) -> Result<(), StoreError> {
        self.lock().insert(exception.clone());
        Ok(())
    }

    async fn remove(&self, exception: &CertificateException) -> Result<(), StoreError> {
        self.lock().remove(exception);
        Ok(())
    }
}

/// Compte de test valide. `plain_imap` : IMAP en clair (consentement
/// donné).
#[must_use]
pub fn sample_account(address: &str, plain_imap: bool) -> AccountDraft {
    let address = EmailAddress::parse(address).unwrap_or_else(|_| panic!("adresse de test"));
    let domain = address.domain().as_str().to_owned();
    let server = |protocol, prefix: &str, port, security| {
        let host = Host::parse(&format!("{prefix}.{domain}")).unwrap_or_else(|_| panic!("hôte"));
        ServerSettings {
            protocol,
            endpoint: ServerEndpoint::new(host, port, security).unwrap_or_else(|_| panic!("port")),
            username: Username::parse(&address.to_string()).unwrap_or_else(|_| panic!("user")),
        }
    };
    let imap = if plain_imap {
        server(Protocol::Imap, "imap", 143, Security::Plain)
    } else {
        server(Protocol::Imap, "imap", 993, Security::Tls)
    };
    let smtp = server(Protocol::Smtp, "smtp", 587, Security::StartTls);
    AccountDraft::restore(
        DisplayName::parse("Compte « de test »").unwrap_or_else(|_| panic!("nom")),
        address,
        imap,
        smtp,
    )
    .unwrap_or_else(|e| panic!("compte de test : {e}"))
}

/// Contrat de [`AccountRepository`] (dépôt vide au départ).
pub async fn account_repository_contract<R: AccountRepository>(
    repo: &R,
) -> Result<(), AccountStoreError> {
    assert!(repo.list().await?.is_empty(), "dépôt non vide au départ");

    let a = sample_account("alice@example.com", false);
    let b = sample_account("bob@example.org", true);
    let stored_a = repo.insert(&a).await?;
    assert_eq!(stored_a.draft(), &a, "compte relu différent");
    let stored_b = repo.insert(&b).await?;
    assert_ne!(stored_a.id(), stored_b.id(), "identifiants identiques");

    // Relecture, y compris d'un serveur en clair consenti.
    assert_eq!(repo.get(stored_a.id()).await?, Some(stored_a.clone()));
    assert_eq!(repo.get(stored_b.id()).await?, Some(stored_b.clone()));
    assert!(
        stored_b
            .plaintext_policy()
            .authorize(&stored_b.imap().endpoint)
            .is_ok()
    );

    // Recherche par adresse (domaine insensible à la casse).
    let upper = EmailAddress::parse("alice@EXAMPLE.com").unwrap_or_else(|_| panic!("adresse"));
    assert_eq!(repo.find_by_address(&upper).await?, Some(stored_a.clone()));
    let absent = EmailAddress::parse("carol@example.com").unwrap_or_else(|_| panic!("adresse"));
    assert_eq!(repo.find_by_address(&absent).await?, None);

    // Doublon refusé, sans effet.
    assert_eq!(
        repo.insert(&a).await,
        Err(AccountStoreError::Duplicate(a.address().clone()))
    );
    assert_eq!(repo.list().await?, vec![stored_a.clone(), stored_b.clone()]);

    // Suppression, idempotente.
    assert!(repo.delete(stored_a.id()).await?);
    assert!(!repo.delete(stored_a.id()).await?);
    assert_eq!(repo.get(stored_a.id()).await?, None);
    assert_eq!(repo.list().await?, vec![stored_b.clone()]);
    let unknown = AccountId::new(i64::MAX).unwrap_or_else(|_| panic!("id"));
    assert!(!repo.delete(unknown).await?);

    // L'adresse supprimée peut être réutilisée, avec un nouvel identifiant.
    let again = repo.insert(&a).await?;
    assert_ne!(again.id(), stored_a.id(), "identifiant réutilisé");
    assert!(repo.delete(again.id()).await?);
    assert!(repo.delete(stored_b.id()).await?);
    assert!(repo.list().await?.is_empty());
    Ok(())
}

/// Faux [`AccountRepository`] en mémoire.
#[derive(Debug, Default)]
pub struct InMemoryAccountRepository {
    state: Mutex<(i64, Vec<Account>)>,
}

impl InMemoryAccountRepository {
    fn lock(&self) -> std::sync::MutexGuard<'_, (i64, Vec<Account>)> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl AccountRepository for InMemoryAccountRepository {
    async fn list(&self) -> Result<Vec<Account>, AccountStoreError> {
        Ok(self.lock().1.clone())
    }

    async fn get(&self, id: AccountId) -> Result<Option<Account>, AccountStoreError> {
        Ok(self.lock().1.iter().find(|a| a.id() == id).cloned())
    }

    async fn find_by_address(
        &self,
        address: &EmailAddress,
    ) -> Result<Option<Account>, AccountStoreError> {
        Ok(self
            .lock()
            .1
            .iter()
            .find(|a| a.address() == address)
            .cloned())
    }

    async fn insert(&self, draft: &AccountDraft) -> Result<Account, AccountStoreError> {
        let mut state = self.lock();
        if state.1.iter().any(|a| a.address() == draft.address()) {
            return Err(AccountStoreError::Duplicate(draft.address().clone()));
        }
        state.0 += 1;
        let id =
            AccountId::new(state.0).map_err(|e| AccountStoreError::Corrupted(e.to_string()))?;
        let account = draft.clone().with_id(id);
        state.1.push(account.clone());
        Ok(account)
    }

    async fn delete(&self, id: AccountId) -> Result<bool, AccountStoreError> {
        let mut state = self.lock();
        let before = state.1.len();
        state.1.retain(|a| a.id() != id);
        Ok(state.1.len() != before)
    }
}

/// Faux [`AccountDataStore`] : mémorise les purges demandées.
#[derive(Debug, Default)]
pub struct RecordingAccountDataStore {
    purged: Mutex<Vec<AccountId>>,
}

impl RecordingAccountDataStore {
    /// Comptes purgés, dans l'ordre.
    #[must_use]
    pub fn purged(&self) -> Vec<AccountId> {
        self.purged
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl AccountDataStore for RecordingAccountDataStore {
    async fn purge(&self, id: AccountId) -> Result<(), StoreError> {
        self.purged
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_exception_store_passes_contract() -> Result<(), StoreError> {
        certificate_exception_store_contract(&InMemoryCertificateExceptionStore::default()).await
    }

    #[tokio::test]
    async fn in_memory_account_repository_passes_contract() -> Result<(), AccountStoreError> {
        account_repository_contract(&InMemoryAccountRepository::default()).await
    }
}
