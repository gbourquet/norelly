//! Adaptateur `SecretStore` : trousseau du système (Windows Credential
//! Manager, Secret Service sous Linux) via `keyring`, et faux en mémoire.
//!
//! Les appels au trousseau sont bloquants : ils passent par
//! `spawn_blocking`. Les erreurs ne recopient jamais les octets du secret
//! (certaines variantes de `keyring::Error` les contiennent).

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use norelly_domain::credentials::Secret;
use norelly_domain::ports::{SecretKey, SecretStore, SecretStoreError};
use zeroize::Zeroize;

/// Service sous lequel Norelly range ses secrets (identifiant
/// d'application, ADR-006).
pub const SERVICE: &str = "io.github.gbourquet.Norelly";

/// Traduit une erreur `keyring` sans en recopier le contenu sensible.
fn map_error(error: keyring::Error) -> SecretStoreError {
    use keyring::Error as E;
    match error {
        E::NoStorageAccess(e) => SecretStoreError::AccessDenied(e.to_string()),
        E::PlatformFailure(e) => SecretStoreError::Unavailable(e.to_string()),
        E::NoDefaultStore => SecretStoreError::Unavailable("aucun trousseau par défaut".into()),
        E::BadEncoding(mut bytes) | E::BadDataFormat(mut bytes, _) => {
            bytes.zeroize();
            SecretStoreError::Backend("secret illisible (encodage)".into())
        }
        E::TooLong(attribute, max) => {
            SecretStoreError::Backend(format!("{attribute} trop long (maximum {max})"))
        }
        E::Invalid(attribute, reason) => {
            SecretStoreError::Backend(format!("{attribute} invalide : {reason}"))
        }
        E::Ambiguous(_) => SecretStoreError::Backend("plusieurs secrets pour la même clé".into()),
        E::NoEntry => SecretStoreError::Backend("secret absent".into()),
        other => {
            SecretStoreError::Backend(format!("erreur du trousseau ({})", variant_name(&other)))
        }
    }
}

fn variant_name(error: &keyring::Error) -> &'static str {
    match error {
        keyring::Error::BadStoreFormat(_) => "format du magasin",
        _ => "inconnue",
    }
}

/// Trousseau du système.
#[derive(Debug, Clone)]
pub struct SystemSecretStore {
    service: String,
}

impl Default for SystemSecretStore {
    fn default() -> Self {
        Self::new(SERVICE)
    }
}

impl SystemSecretStore {
    #[must_use]
    pub fn new(service: &str) -> Self {
        Self {
            service: service.to_owned(),
        }
    }

    /// Vérifie que le trousseau du système est utilisable.
    pub fn availability() -> Result<(), SecretStoreError> {
        match keyring::Entry::store_status() {
            Ok(()) => Ok(()),
            Err(e) => Err(SecretStoreError::Unavailable(e.to_string())),
        }
    }

    async fn blocking<T, F>(&self, key: &SecretKey, f: F) -> Result<T, SecretStoreError>
    where
        T: Send + 'static,
        F: FnOnce(keyring::Entry) -> Result<T, SecretStoreError> + Send + 'static,
    {
        let service = self.service.clone();
        let key = key.as_str().to_owned();
        tokio::task::spawn_blocking(move || {
            let entry = keyring::Entry::new(&service, &key).map_err(map_error)?;
            f(entry)
        })
        .await
        .map_err(|_| SecretStoreError::Backend("tâche du trousseau interrompue".into()))?
    }
}

impl SecretStore for SystemSecretStore {
    async fn get(&self, key: &SecretKey) -> Result<Option<Secret>, SecretStoreError> {
        self.blocking(key, |entry| match entry.get_password() {
            Ok(password) => Ok(Some(Secret::new(password))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(map_error(e)),
        })
        .await
    }

    async fn set(&self, key: &SecretKey, secret: &Secret) -> Result<(), SecretStoreError> {
        let secret = secret.clone();
        self.blocking(key, move |entry| {
            entry.set_password(secret.expose()).map_err(map_error)
        })
        .await
    }

    async fn delete(&self, key: &SecretKey) -> Result<(), SecretStoreError> {
        self.blocking(key, |entry| match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(map_error(e)),
        })
        .await
    }
}

/// Faux trousseau en mémoire (tests et développement).
#[derive(Debug, Default)]
pub struct InMemorySecretStore {
    secrets: Mutex<HashMap<SecretKey, Secret>>,
}

impl InMemorySecretStore {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<SecretKey, Secret>> {
        self.secrets.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl SecretStore for InMemorySecretStore {
    async fn get(&self, key: &SecretKey) -> Result<Option<Secret>, SecretStoreError> {
        Ok(self.lock().get(key).cloned())
    }

    async fn set(&self, key: &SecretKey, secret: &Secret) -> Result<(), SecretStoreError> {
        self.lock().insert(key.clone(), secret.clone());
        Ok(())
    }

    async fn delete(&self, key: &SecretKey) -> Result<(), SecretStoreError> {
        self.lock().remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use norelly_test_support::contracts::secret_store_contract;

    #[tokio::test]
    async fn in_memory_store_passes_contract() -> Result<(), SecretStoreError> {
        secret_store_contract(&InMemorySecretStore::default(), "test:").await
    }

    #[test]
    fn errors_never_copy_secret_bytes() {
        let secret = b"Tr3s-S3cret!".to_vec();
        let rendered = format!("{:?}", map_error(keyring::Error::BadEncoding(secret)));
        assert!(!rendered.contains("Tr3s"), "{rendered}");
        assert!(!rendered.contains("84, 114, 51"), "{rendered}");
    }
}
