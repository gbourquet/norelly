//! Suites de contrat des ports : chaque implémentation (faux en mémoire et
//! adaptateur réel) doit les passer.
//!
//! Les fonctions paniquent (assertions) en cas de violation du contrat, et
//! renvoient une erreur si l'implémentation échoue.

use std::collections::BTreeSet;
use std::sync::Mutex;

use norelly_domain::credentials::Secret;
use norelly_domain::endpoint::Host;
use norelly_domain::ports::{
    CertificateExceptionStore, SecretKey, SecretStore, SecretStoreError, StoreError,
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
/// (SQLite) arrive avec le stockage (phase 4) et passera la même suite.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_exception_store_passes_contract() -> Result<(), StoreError> {
        certificate_exception_store_contract(&InMemoryCertificateExceptionStore::default()).await
    }
}
