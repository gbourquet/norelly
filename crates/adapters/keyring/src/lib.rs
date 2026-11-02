//! Adaptateur `SecretStore` : trousseau du système (Windows Credential
//! Manager, Secret Service sous Linux via libdbus) avec `keyring-core`, et
//! faux en mémoire.
//!
//! Les appels au trousseau sont bloquants : ils passent par
//! `spawn_blocking`. Les erreurs ne recopient jamais les octets du secret
//! (certaines variantes de `keyring_core::Error` les contiennent).

mod file;

pub use file::FileSecretStore;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use keyring_core::{CredentialStore, Entry};

use norelly_domain::credentials::Secret;
use norelly_domain::ports::{SecretKey, SecretStore, SecretStoreError};
use zeroize::Zeroize;

/// Service sous lequel Norelly range ses secrets (identifiant
/// d'application, ADR-006).
pub const SERVICE: &str = "io.github.gbourquet.Norelly";

/// Explication donnée quand aucun trousseau ne répond.
#[cfg(not(windows))]
const NO_SERVICE: &str = "aucun service de trousseau (Secret Service) ne répond ; \
     installez et démarrez GNOME Keyring ou KWallet, puis réessayez";
#[cfg(windows)]
const NO_SERVICE: &str = "le Gestionnaire d'identification de Windows ne répond pas";

/// Traduit une erreur `keyring` sans en recopier le contenu sensible.
fn map_error(error: keyring_core::Error) -> SecretStoreError {
    use keyring_core::Error as E;
    match error {
        E::NoStorageAccess(e) => SecretStoreError::AccessDenied(e.to_string()),
        E::PlatformFailure(e) => {
            // Détail technique (en anglais, D-Bus ou Win32) gardé pour le
            // journal ; l'utilisateur reçoit une explication.
            tracing::warn!(error = %e, "trousseau du système injoignable");
            SecretStoreError::Unavailable(NO_SERVICE.into())
        }
        E::NoDefaultStore => SecretStoreError::Unavailable(NO_SERVICE.into()),
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

fn variant_name(error: &keyring_core::Error) -> &'static str {
    match error {
        keyring_core::Error::BadStoreFormat(_) => "format du magasin",
        _ => "inconnue",
    }
}

/// Magasin natif de la plateforme.
fn platform_store() -> Result<Arc<CredentialStore>, SecretStoreError> {
    #[cfg(all(
        unix,
        not(any(target_os = "macos", target_os = "ios", target_os = "android"))
    ))]
    {
        let store: Arc<CredentialStore> =
            dbus_secret_service_keyring_store::Store::new().map_err(map_error)?;
        Ok(store)
    }
    #[cfg(windows)]
    {
        let store: Arc<CredentialStore> =
            windows_native_keyring_store::Store::new().map_err(map_error)?;
        Ok(store)
    }
    #[cfg(not(any(
        windows,
        all(
            unix,
            not(any(target_os = "macos", target_os = "ios", target_os = "android"))
        )
    )))]
    {
        Err(SecretStoreError::Unavailable(
            "plateforme non prise en charge".into(),
        ))
    }
}

/// Trousseau du système.
#[derive(Clone)]
pub struct SystemSecretStore {
    store: Arc<CredentialStore>,
    service: String,
}

impl std::fmt::Debug for SystemSecretStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemSecretStore")
            .field("service", &self.service)
            .finish_non_exhaustive()
    }
}

impl SystemSecretStore {
    /// Ouvre le trousseau du système pour le service [`SERVICE`].
    pub fn open() -> Result<Self, SecretStoreError> {
        Self::with_service(SERVICE)
    }

    /// Ouvre le trousseau du système pour un autre service (tests).
    pub fn with_service(service: &str) -> Result<Self, SecretStoreError> {
        Ok(Self {
            store: platform_store()?,
            service: service.to_owned(),
        })
    }

    async fn blocking<T, F>(&self, key: &SecretKey, f: F) -> Result<T, SecretStoreError>
    where
        T: Send + 'static,
        F: FnOnce(Entry) -> Result<T, SecretStoreError> + Send + 'static,
    {
        let store = Arc::clone(&self.store);
        let service = self.service.clone();
        let key = key.as_str().to_owned();
        tokio::task::spawn_blocking(move || {
            let entry = store.build(&service, &key, None).map_err(map_error)?;
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
            Err(keyring_core::Error::NoEntry) => Ok(None),
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
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
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
        let rendered = format!("{:?}", map_error(keyring_core::Error::BadEncoding(secret)));
        assert!(!rendered.contains("Tr3s"), "{rendered}");
        assert!(!rendered.contains("84, 114, 51"), "{rendered}");
    }
}
