//! Contrat de `SecretStore` contre le vrai Secret Service (trousseau du
//! système). Joué seulement si `NORELLY_SECRET_SERVICE_TEST=1` : dans le
//! conteneur des e2e (`app/e2e/in-container.sh`), où un GNOME Keyring
//! jetable est déverrouillé. Jamais contre le trousseau personnel d'un
//! poste de développement (une invite de déverrouillage pourrait s'ouvrir).

use norelly_domain::ports::SecretStoreError;
use norelly_keyring::SystemSecretStore;
use norelly_test_support::contracts::secret_store_contract;

#[tokio::test]
async fn system_secret_service_passes_contract() -> Result<(), SecretStoreError> {
    if std::env::var_os("NORELLY_SECRET_SERVICE_TEST").is_none() {
        eprintln!("trousseau de test absent : contrat non joué (voir app/e2e/in-container.sh)");
        return Ok(());
    }
    let service = format!("norelly-tests-{}", std::process::id());
    let store = SystemSecretStore::with_service(&service)?;
    secret_store_contract(&store, "contrat:").await
}
