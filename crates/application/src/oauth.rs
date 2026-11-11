//! Jetons des comptes OAuth : jeton d'accès gardé en mémoire jusqu'à son
//! expiration, rafraîchi à la demande ; jeton de rafraîchissement dans le
//! trousseau seulement, remplacé quand le fournisseur en délivre un
//! nouveau.

use std::collections::HashMap;

use norelly_domain::auth::AuthError;
use norelly_domain::credentials::Secret;
use norelly_domain::oauth::{AccessToken, OAuthSource, TokenError};
use norelly_domain::ports::{OAuthClient, OAuthGrant, SecretKey, SecretStore, SecretStoreError};
use tokio::sync::Mutex;
use tokio::time::Instant;

/// Gestionnaire des jetons d'accès.
#[derive(Debug)]
pub struct TokenBroker<O, K> {
    client: O,
    secrets: K,
    cache: Mutex<HashMap<SecretKey, AccessToken>>,
    origin: Instant,
}

impl<O: OAuthClient, K: SecretStore> TokenBroker<O, K> {
    pub fn new(client: O, secrets: K) -> Self {
        Self {
            client,
            secrets,
            cache: Mutex::new(HashMap::new()),
            origin: Instant::now(),
        }
    }

    /// Client OAuth (connexion interactive).
    pub fn client(&self) -> &O {
        &self.client
    }

    fn now(&self) -> u64 {
        self.origin.elapsed().as_secs()
    }

    /// Jeton d'accès valable pour `source`. Un jeton de rafraîchissement
    /// absent ou refusé donne [`AuthError::ReauthRequired`].
    pub async fn access_token(&self, source: &OAuthSource) -> Result<Secret, AuthError> {
        let mut cache = self.cache.lock().await;
        if let Some(token) = cache.get(&source.refresh_key)
            && token.is_fresh(self.now())
        {
            return Ok(token.token.clone());
        }
        let refresh = self
            .secrets
            .get(&source.refresh_key)
            .await
            .map_err(|e| AuthError::Rejected {
                message: e.to_string(),
            })?
            .ok_or(AuthError::ReauthRequired)?;
        let grant = self
            .client
            .refresh(source.provider, &refresh)
            .await
            .map_err(|e| auth_error_of(&e))?;
        let token = grant.access_token.clone();
        if let Some(rotated) = &grant.refresh_token {
            self.secrets
                .set(&source.refresh_key, rotated)
                .await
                .map_err(|e| AuthError::Rejected {
                    message: e.to_string(),
                })?;
        }
        cache.insert(source.refresh_key.clone(), self.access_of(grant));
        Ok(token)
    }

    fn access_of(&self, grant: OAuthGrant) -> AccessToken {
        AccessToken {
            token: grant.access_token,
            expires_at: self.now().saturating_add(grant.expires_in),
        }
    }

    /// Enregistre les jetons d'une connexion : jeton de rafraîchissement
    /// dans le trousseau, jeton d'accès en mémoire.
    pub async fn store_grant(
        &self,
        source: &OAuthSource,
        grant: OAuthGrant,
    ) -> Result<(), SecretStoreError> {
        if let Some(refresh) = &grant.refresh_token {
            self.secrets.set(&source.refresh_key, refresh).await?;
        }
        let access = self.access_of(grant);
        self.cache
            .lock()
            .await
            .insert(source.refresh_key.clone(), access);
        Ok(())
    }

    /// Oublie le jeton d'accès en mémoire (compte supprimé ou reconnecté).
    pub async fn forget(&self, key: &SecretKey) {
        self.cache.lock().await.remove(key);
    }
}

/// Erreur de jeton vue d'une connexion.
#[must_use]
pub fn auth_error_of(error: &TokenError) -> AuthError {
    match error {
        TokenError::ReauthRequired => AuthError::ReauthRequired,
        other => AuthError::Rejected {
            message: other.to_string(),
        },
    }
}

#[cfg(test)]
mod tests;
