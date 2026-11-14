//! Jetons des comptes OAuth : jeton d'accès gardé en mémoire jusqu'à son
//! expiration, rafraîchi à la demande ; jeton de rafraîchissement dans le
//! trousseau, remplacé quand le fournisseur en délivre un nouveau. Si le
//! trousseau refuse ce remplacement, le nouveau jeton reste en mémoire,
//! sert aux rafraîchissements suivants et son enregistrement est retenté à
//! chacun d'eux.
//!
//! [`FreshTokens`] enveloppe les connecteurs IMAP et SMTP : avant chaque
//! connexion d'un compte OAuth, il place un jeton valable dans la requête,
//! et après un refus du serveur, il oublie le jeton et réessaie une fois.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use norelly_domain::address::EmailAddress;
use norelly_domain::auth::AuthError;
use norelly_domain::capabilities::ServerCapabilities;
use norelly_domain::connection::ConnectionError;
use norelly_domain::credentials::Secret;
use norelly_domain::oauth::{
    AccessToken, OAuthProvider, OAuthSource, PendingAuthorization, TokenError,
};
use norelly_domain::ports::{
    ConnectRequest, MailSender, OAuthClient, OAuthGrant, OutgoingMessage, RemoteMailboxConnector,
    SecretKey, SecretStore, SecretStoreError, SendError,
};
use tokio::sync::Mutex;

/// Délai maximal d'un rafraîchissement (trousseau et fournisseur).
pub const REFRESH_TIMEOUT: Duration = Duration::from_secs(30);

/// Horloge murale en secondes (elle avance pendant la mise en veille).
pub type WallClock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// Gestionnaire des jetons d'accès.
pub struct TokenBroker<O, K> {
    client: O,
    secrets: K,
    clock: WallClock,
    /// Un verrou par jeton de rafraîchissement : la rotation d'un compte est
    /// sérialisée sans bloquer les autres comptes.
    slots: std::sync::Mutex<HashMap<SecretKey, Arc<Mutex<Slot>>>>,
    /// Connexion d'un ajout de compte qui n'a pas abouti, en mémoire
    /// seulement, jusqu'au prochain essai.
    pending: std::sync::Mutex<Option<(PendingAuthorization, OAuthGrant)>>,
}

/// Jetons d'un compte gardés en mémoire.
#[derive(Default)]
struct Slot {
    access: Option<AccessToken>,
    /// Dernier jeton de rafraîchissement délivré, que le trousseau n'a pas
    /// encore accepté : il prime sur celui du trousseau, devenu périmé.
    unsaved_refresh: Option<Secret>,
}

impl<O, K> std::fmt::Debug for TokenBroker<O, K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenBroker").finish_non_exhaustive()
    }
}

impl<O: OAuthClient, K: SecretStore> TokenBroker<O, K> {
    pub fn new(client: O, secrets: K, clock: WallClock) -> Self {
        Self {
            client,
            secrets,
            clock,
            slots: std::sync::Mutex::new(HashMap::new()),
            pending: std::sync::Mutex::new(None),
        }
    }

    /// Heure de l'horloge murale du gestionnaire (secondes).
    pub fn now(&self) -> u64 {
        (self.clock)()
    }

    /// Garde la connexion d'un ajout de compte qui a échoué après la
    /// connexion chez le fournisseur, obtenue à `obtained_at`
    /// ([`Self::now`]) : le prochain essai pour la même adresse la
    /// réutilise ([`Self::take_pending`]). Remplace la précédente.
    pub fn keep_pending(
        &self,
        provider: OAuthProvider,
        address: &EmailAddress,
        grant: OAuthGrant,
        obtained_at: u64,
    ) {
        let pending = PendingAuthorization {
            provider,
            address: address.clone(),
            obtained_at,
            expires_in: grant.expires_in,
        };
        *self.pending.lock().unwrap_or_else(PoisonError::into_inner) = Some((pending, grant));
    }

    /// Vrai si une connexion gardée peut resservir pour ajouter `address`
    /// chez `provider` (l'écran n'annonce alors pas le navigateur).
    pub fn has_pending(&self, provider: OAuthProvider, address: &EmailAddress) -> bool {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(|(pending, _)| pending.usable_for(provider, address, (self.clock)()))
    }

    /// Connexion gardée pour ajouter `address` chez `provider`, si elle
    /// peut resservir. Elle est oubliée dans tous les cas.
    pub fn take_pending(
        &self,
        provider: OAuthProvider,
        address: &EmailAddress,
    ) -> Option<OAuthGrant> {
        let kept = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()?;
        let (pending, grant) = kept;
        pending
            .usable_for(provider, address, (self.clock)())
            .then_some(grant)
    }

    /// Client OAuth (connexion interactive).
    pub fn client(&self) -> &O {
        &self.client
    }

    fn slot(&self, key: &SecretKey) -> Arc<Mutex<Slot>> {
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(slots.entry(key.clone()).or_default())
    }

    /// Jeton d'accès valable pour `source`. Un jeton de rafraîchissement
    /// absent ou refusé donne [`AuthError::ReauthRequired`] ; un trousseau
    /// illisible, [`AuthError::SecretStoreUnavailable`].
    pub async fn access_token(&self, source: &OAuthSource) -> Result<Secret, AuthError> {
        let slot = self.slot(&source.refresh_key);
        let mut slot = slot.lock().await;
        if let Some(token) = slot.access.as_ref()
            && token.is_fresh((self.clock)())
        {
            return Ok(token.token.clone());
        }
        let unsaved = slot.unsaved_refresh.clone();
        let grant = tokio::time::timeout(REFRESH_TIMEOUT, self.refresh(source, unsaved))
            .await
            .map_err(|_| auth_error_of(&TokenError::Network("délai dépassé".to_owned())))??;
        let token = grant.access_token.clone();
        let latest = grant
            .refresh_token
            .clone()
            .or_else(|| slot.unsaved_refresh.take());
        if let Some(latest) = latest {
            slot.unsaved_refresh = self.save_refresh(&source.refresh_key, latest).await;
        }
        slot.access = Some(self.access_of(grant));
        Ok(token)
    }

    /// Enregistre `refresh` dans le trousseau, dans le délai maximal. Rend
    /// le jeton s'il n'a pas pu l'être : le jeton d'accès reste utilisable,
    /// et le jeton gardé en mémoire servira au prochain rafraîchissement.
    async fn save_refresh(&self, key: &SecretKey, refresh: Secret) -> Option<Secret> {
        match tokio::time::timeout(REFRESH_TIMEOUT, self.secrets.set(key, &refresh)).await {
            Ok(Ok(())) => None,
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "nouveau jeton de rafraîchissement non enregistré");
                Some(refresh)
            }
            Err(_) => {
                tracing::warn!("nouveau jeton de rafraîchissement non enregistré : délai dépassé");
                Some(refresh)
            }
        }
    }

    async fn refresh(
        &self,
        source: &OAuthSource,
        unsaved: Option<Secret>,
    ) -> Result<OAuthGrant, AuthError> {
        let refresh = match unsaved {
            Some(refresh) => refresh,
            None => self
                .secrets
                .get(&source.refresh_key)
                .await
                .map_err(|e| AuthError::SecretStoreUnavailable {
                    message: e.to_string(),
                })?
                .ok_or(AuthError::ReauthRequired)?,
        };
        self.client
            .refresh(source.provider, &refresh)
            .await
            .map_err(|e| auth_error_of(&e))
    }

    fn access_of(&self, grant: OAuthGrant) -> AccessToken {
        AccessToken {
            token: grant.access_token,
            expires_at: (self.clock)().saturating_add(grant.expires_in),
        }
    }

    /// Enregistre les jetons d'une connexion : jeton de rafraîchissement
    /// dans le trousseau, jeton d'accès en mémoire.
    pub async fn store_grant(
        &self,
        source: &OAuthSource,
        grant: OAuthGrant,
    ) -> Result<(), SecretStoreError> {
        let saved_refresh = grant.refresh_token.is_some();
        if let Some(refresh) = &grant.refresh_token {
            self.secrets.set(&source.refresh_key, refresh).await?;
        }
        let access = self.access_of(grant);
        let slot = self.slot(&source.refresh_key);
        let mut slot = slot.lock().await;
        slot.access = Some(access);
        if saved_refresh {
            // Le trousseau contient désormais le jeton le plus récent.
            slot.unsaved_refresh = None;
        }
        Ok(())
    }

    /// Oublie le jeton d'accès en mémoire, refusé par le serveur. Un jeton
    /// de rafraîchissement non encore enregistré est gardé : c'est le seul
    /// que le fournisseur accepte encore.
    pub async fn forget(&self, key: &SecretKey) {
        self.slot(key).lock().await.access = None;
    }
}

/// Erreur de jeton vue d'une connexion : seul un jeton de rafraîchissement
/// refusé met le compte en cause ; le reste est une panne du fournisseur.
#[must_use]
pub fn auth_error_of(error: &TokenError) -> AuthError {
    match error {
        TokenError::ReauthRequired => AuthError::ReauthRequired,
        other => AuthError::ProviderUnavailable {
            message: other.to_string(),
        },
    }
}

/// Connecteur qui place un jeton valable dans la requête d'un compte OAuth.
pub struct FreshTokens<C, O, K> {
    pub inner: C,
    pub broker: Arc<TokenBroker<O, K>>,
}

impl<C, O, K> std::fmt::Debug for FreshTokens<C, O, K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FreshTokens").finish_non_exhaustive()
    }
}

impl<C: Sync, O: OAuthClient, K: SecretStore> FreshTokens<C, O, K> {
    fn source(request: &ConnectRequest) -> Option<OAuthSource> {
        let binding = request.credentials.oauth()?;
        Some(OAuthSource {
            provider: binding.provider,
            refresh_key: binding.refresh_key.clone()?,
        })
    }

    /// Requête avec un jeton valable (inchangée pour un mot de passe).
    async fn fresh(&self, request: &ConnectRequest) -> Result<ConnectRequest, ConnectionError> {
        let Some(source) = Self::source(request) else {
            return Ok(request.clone());
        };
        let token = self.broker.access_token(&source).await?;
        let credentials =
            request
                .credentials
                .with_access_token(token)
                .map_err(|e| AuthError::Rejected {
                    message: e.to_string(),
                })?;
        Ok(ConnectRequest {
            credentials,
            ..request.clone()
        })
    }

    /// Exécute `attempt`, et après un refus d'authentification d'un compte
    /// OAuth, une seconde fois avec un jeton rafraîchi.
    async fn with_retry<T, F, Fut>(
        &self,
        request: &ConnectRequest,
        attempt: F,
    ) -> Result<T, ConnectionError>
    where
        F: Fn(ConnectRequest) -> Fut,
        Fut: Future<Output = Result<T, ConnectionError>>,
    {
        let result = attempt(self.fresh(request).await?).await;
        let Some(source) = Self::source(request) else {
            return result;
        };
        match result {
            Err(ConnectionError::Authentication(AuthError::Rejected { .. })) => {
                self.broker.forget(&source.refresh_key).await;
                attempt(self.fresh(request).await?).await
            }
            other => other,
        }
    }
}

impl<C, O, K> RemoteMailboxConnector for FreshTokens<C, O, K>
where
    C: RemoteMailboxConnector,
    O: OAuthClient,
    K: SecretStore,
{
    type Session = C::Session;

    async fn connect(&self, request: &ConnectRequest) -> Result<C::Session, ConnectionError> {
        self.with_retry(request, |r| async move { self.inner.connect(&r).await })
            .await
    }
}

impl<C, O, K> MailSender for FreshTokens<C, O, K>
where
    C: MailSender,
    O: OAuthClient,
    K: SecretStore,
{
    async fn verify(
        &self,
        request: &ConnectRequest,
    ) -> Result<ServerCapabilities, ConnectionError> {
        self.with_retry(request, |r| async move { self.inner.verify(&r).await })
            .await
    }

    async fn send(
        &self,
        request: &ConnectRequest,
        message: &OutgoingMessage,
    ) -> Result<(), SendError> {
        // Un refus d'authentification arrive avant toute transmission : le
        // second essai ne peut pas envoyer le message deux fois.
        let result = self
            .with_retry(request, |r| async move {
                match self.inner.send(&r, message).await {
                    Err(SendError::Connection(e)) => Err(e),
                    other => Ok(other),
                }
            })
            .await;
        match result {
            Ok(inner) => inner,
            Err(e) => Err(SendError::Connection(e)),
        }
    }
}

#[cfg(test)]
mod tests;
