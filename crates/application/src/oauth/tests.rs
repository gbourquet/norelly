//! Tests du gestionnaire de jetons, sur faux client et faux trousseau.

use std::collections::BTreeMap;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use norelly_domain::account::AccountId;
use norelly_domain::address::EmailAddress;
use norelly_domain::endpoint::Protocol;
use norelly_domain::oauth::OAuthProvider;

use super::*;

#[derive(Debug, Default)]
struct Keyring(StdMutex<BTreeMap<String, String>>);

impl Keyring {
    fn read(&self, key: &SecretKey) -> Option<String> {
        self.0.lock().ok()?.get(key.as_str()).cloned()
    }
}

impl SecretStore for Keyring {
    async fn get(&self, key: &SecretKey) -> Result<Option<Secret>, SecretStoreError> {
        Ok(self.read(key).map(Secret::new))
    }
    async fn set(&self, key: &SecretKey, secret: &Secret) -> Result<(), SecretStoreError> {
        if let Ok(mut m) = self.0.lock() {
            m.insert(key.as_str().to_owned(), secret.expose().to_owned());
        }
        Ok(())
    }
    async fn delete(&self, key: &SecretKey) -> Result<(), SecretStoreError> {
        if let Ok(mut m) = self.0.lock() {
            m.remove(key.as_str());
        }
        Ok(())
    }
}

/// Faux fournisseur : accepte le jeton de rafraîchissement « r1 » et le
/// fait tourner en « r2 » ; refuse tout autre jeton (`invalid_grant`).
#[derive(Debug, Default)]
struct Provider {
    refreshes: AtomicUsize,
}

impl OAuthClient for Provider {
    async fn authorize(
        &self,
        _: OAuthProvider,
        _: &EmailAddress,
    ) -> Result<OAuthGrant, TokenError> {
        Err(TokenError::Cancelled("non utilisé".to_owned()))
    }
    async fn refresh(&self, _: OAuthProvider, refresh: &Secret) -> Result<OAuthGrant, TokenError> {
        let n = self.refreshes.fetch_add(1, Ordering::SeqCst) + 1;
        if refresh.expose() != "r1" {
            return Err(TokenError::ReauthRequired);
        }
        Ok(OAuthGrant {
            access_token: Secret::new(format!("a{n}")),
            expires_in: 3_600,
            refresh_token: Some(Secret::new("r2".to_owned())),
        })
    }
}

fn source() -> OAuthSource {
    let Ok(id) = AccountId::new(7) else {
        panic!("identifiant valide refusé");
    };
    OAuthSource {
        provider: OAuthProvider::Microsoft,
        refresh_key: SecretKey::for_account(id, Protocol::Imap),
    }
}

async fn broker_with_refresh(token: &str) -> TokenBroker<Provider, Keyring> {
    let keyring = Keyring::default();
    let _ = keyring
        .set(&source().refresh_key, &Secret::new(token.to_owned()))
        .await;
    TokenBroker::new(Provider::default(), keyring)
}

#[tokio::test(start_paused = true)]
async fn refreshes_once_then_serves_from_memory() {
    let broker = broker_with_refresh("r1").await;
    let first = broker
        .access_token(&source())
        .await
        .map(|s| s.expose().to_owned());
    assert_eq!(first, Ok("a1".to_owned()));
    let again = broker
        .access_token(&source())
        .await
        .map(|s| s.expose().to_owned());
    assert_eq!(again, Ok("a1".to_owned()));
    assert_eq!(broker.client().refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(
        broker.secrets.read(&source().refresh_key).as_deref(),
        Some("r2"),
        "jeton de rafraîchissement remplacé par celui du fournisseur"
    );
}

#[tokio::test(start_paused = true)]
async fn expired_token_is_refreshed() {
    let broker = TokenBroker::new(Provider::default(), Keyring::default());
    let grant = OAuthGrant {
        access_token: Secret::new("initial".to_owned()),
        expires_in: 600,
        refresh_token: Some(Secret::new("r1".to_owned())),
    };
    assert_eq!(broker.store_grant(&source(), grant).await, Ok(()));
    let t = broker
        .access_token(&source())
        .await
        .map(|s| s.expose().to_owned());
    assert_eq!(t, Ok("initial".to_owned()));
    tokio::time::advance(std::time::Duration::from_secs(600)).await;
    let t = broker
        .access_token(&source())
        .await
        .map(|s| s.expose().to_owned());
    assert_eq!(t, Ok("a1".to_owned()));
}

#[tokio::test(start_paused = true)]
async fn refused_or_missing_refresh_token_requires_reauth() {
    let broker = broker_with_refresh("révoqué").await;
    assert_eq!(
        broker.access_token(&source()).await.map(|_| ()),
        Err(AuthError::ReauthRequired)
    );
    let empty = TokenBroker::new(Provider::default(), Keyring::default());
    assert_eq!(
        empty.access_token(&source()).await.map(|_| ()),
        Err(AuthError::ReauthRequired)
    );
    assert_eq!(empty.client().refreshes.load(Ordering::SeqCst), 0);
}
