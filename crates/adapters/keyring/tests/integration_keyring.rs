//! Suite de contrat du `SecretStore` contre le vrai trousseau du système,
//! s'il est disponible (Secret Service sous Linux, Credential Manager sous
//! Windows). Sans trousseau (CI sans session D-Bus), le test le signale et
//! s'arrête sans échouer : la suite tourne toujours sur le faux en mémoire.

use norelly_domain::credentials::Secret;
use norelly_domain::ports::{SecretKey, SecretStore, SecretStoreError};
use norelly_keyring::SystemSecretStore;
use norelly_test_support::contracts::secret_store_contract;

const SERVICE: &str = "io.github.gbourquet.Norelly.tests";

#[tokio::test]
async fn system_store_passes_contract_when_available() -> Result<(), SecretStoreError> {
    let store = match SystemSecretStore::with_service(SERVICE) {
        Ok(store) => store,
        Err(e) => {
            eprintln!("trousseau indisponible, contrat non exécuté : {e}");
            return Ok(());
        }
    };
    // Sonde : un trousseau verrouillé ou inaccessible n'est pas « disponible ».
    let Ok(probe) = SecretKey::parse("norelly-test:sonde") else {
        panic!("clé de sonde invalide");
    };
    if let Err(e) = store.set(&probe, &Secret::new("sonde".into())).await {
        eprintln!("trousseau inaccessible, contrat non exécuté : {e}");
        return Ok(());
    }
    store.delete(&probe).await?;
    let prefix = format!("norelly-test-{}:", std::process::id());
    secret_store_contract(&store, &prefix).await
}
