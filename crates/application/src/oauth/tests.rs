//! Tests des jetons OAuth, sur faux client, faux trousseau et faux SMTP.

use std::collections::BTreeMap;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use norelly_domain::account::AccountId;
use norelly_domain::address::EmailAddress;
use norelly_domain::credentials::{Credentials, Username};
use norelly_domain::endpoint::{Host, Protocol, Security, ServerEndpoint};
use norelly_domain::oauth::{OAuthConfig, OAuthProvider};
use norelly_domain::security::PlaintextPolicy;

use super::*;

/// Faux trousseau, que les tests peuvent rendre indisponible en lecture ou
/// en écriture.
#[derive(Debug, Default)]
struct Keyring {
    entries: StdMutex<BTreeMap<String, String>>,
    locked: AtomicBool,
    read_only: AtomicBool,
}

impl Keyring {
    fn read(&self, key: &SecretKey) -> Option<String> {
        self.entries.lock().ok()?.get(key.as_str()).cloned()
    }
    fn unavailable() -> SecretStoreError {
        SecretStoreError::Unavailable("trousseau verrouillé".to_owned())
    }
}

impl SecretStore for Keyring {
    async fn get(&self, key: &SecretKey) -> Result<Option<Secret>, SecretStoreError> {
        if self.locked.load(Ordering::SeqCst) {
            return Err(Self::unavailable());
        }
        Ok(self.read(key).map(Secret::new))
    }
    async fn set(&self, key: &SecretKey, secret: &Secret) -> Result<(), SecretStoreError> {
        if self.read_only.load(Ordering::SeqCst) {
            return Err(Self::unavailable());
        }
        if let Ok(mut m) = self.entries.lock() {
            m.insert(key.as_str().to_owned(), secret.expose().to_owned());
        }
        Ok(())
    }
    async fn delete(&self, key: &SecretKey) -> Result<(), SecretStoreError> {
        if let Ok(mut m) = self.entries.lock() {
            m.remove(key.as_str());
        }
        Ok(())
    }
}

/// Faux fournisseur : n'accepte que le dernier jeton de rafraîchissement
/// délivré (« r1 » au départ), le fait tourner (« r2 », « r3 »…) et délivre
/// « a1 », « a2 »… ; le jeton « lent » ne reçoit jamais de réponse, le
/// jeton « panne » une erreur du fournisseur.
#[derive(Debug)]
struct Provider {
    refreshes: AtomicUsize,
    current: StdMutex<String>,
    hang: bool,
    /// Temps de réponse du fournisseur.
    delay: Duration,
}

impl Default for Provider {
    fn default() -> Self {
        Self {
            refreshes: AtomicUsize::new(0),
            current: StdMutex::new("r1".to_owned()),
            hang: false,
            delay: Duration::ZERO,
        }
    }
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
        if self.hang || refresh.expose() == "lent" {
            std::future::pending::<()>().await;
        }
        tokio::time::sleep(self.delay).await;
        let n = self.refreshes.fetch_add(1, Ordering::SeqCst) + 1;
        if refresh.expose() == "panne" {
            return Err(TokenError::Network("HTTP 503".to_owned()));
        }
        let Ok(mut current) = self.current.lock() else {
            return Err(TokenError::Protocol("verrou".to_owned()));
        };
        if refresh.expose() != current.as_str() {
            return Err(TokenError::ReauthRequired);
        }
        *current = format!("r{}", n + 1);
        Ok(OAuthGrant {
            access_token: Secret::new(format!("a{n}")),
            expires_in: 3_600,
            refresh_token: Some(Secret::new(current.clone())),
            identity: None,
            granted_scopes: None,
        })
    }
}

fn key_of(account: i64) -> SecretKey {
    let Ok(id) = AccountId::new(account) else {
        panic!("identifiant valide refusé");
    };
    SecretKey::for_account(id, Protocol::Imap)
}

fn key() -> SecretKey {
    key_of(7)
}

fn source_of(account: i64) -> OAuthSource {
    OAuthSource {
        provider: OAuthProvider::Microsoft,
        refresh_key: key_of(account),
    }
}

fn source() -> OAuthSource {
    source_of(7)
}

struct Fixture {
    broker: Arc<TokenBroker<Provider, Keyring>>,
    now: Arc<AtomicU64>,
}

async fn fixture(refresh: Option<&str>, provider: Provider) -> Fixture {
    let keyring = Keyring::default();
    if let Some(r) = refresh {
        let _ = keyring.set(&key(), &Secret::new(r.to_owned())).await;
    }
    let now = Arc::new(AtomicU64::new(1_000_000));
    let clock = Arc::clone(&now);
    let broker = TokenBroker::new(
        provider,
        keyring,
        Arc::new(move || clock.load(Ordering::SeqCst)),
    );
    Fixture {
        broker: Arc::new(broker),
        now,
    }
}

async fn token(f: &Fixture) -> Result<String, AuthError> {
    f.broker
        .access_token(&source())
        .await
        .map(|s| s.expose().to_owned())
}

#[tokio::test]
async fn refreshes_once_then_serves_from_memory() {
    let f = fixture(Some("r1"), Provider::default()).await;
    assert_eq!(token(&f).await, Ok("a1".to_owned()));
    assert_eq!(token(&f).await, Ok("a1".to_owned()));
    assert_eq!(f.broker.client().refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.broker.secrets.read(&key()).as_deref(),
        Some("r2"),
        "jeton de rafraîchissement remplacé par celui du fournisseur"
    );
}

#[tokio::test]
async fn expiry_follows_the_wall_clock() {
    let f = fixture(None, Provider::default()).await;
    let grant = OAuthGrant {
        access_token: Secret::new("initial".to_owned()),
        expires_in: 600,
        refresh_token: Some(Secret::new("r1".to_owned())),
        identity: None,
        granted_scopes: None,
    };
    assert_eq!(f.broker.store_grant(&source(), grant).await, Ok(()));
    assert_eq!(token(&f).await, Ok("initial".to_owned()));
    // Mise en veille de deux heures : l'horloge murale a avancé.
    f.now.fetch_add(7_200, Ordering::SeqCst);
    assert_eq!(token(&f).await, Ok("a1".to_owned()));
}

#[tokio::test]
async fn refused_or_missing_refresh_token_requires_reauth() {
    let f = fixture(Some("révoqué"), Provider::default()).await;
    assert_eq!(token(&f).await, Err(AuthError::ReauthRequired));
    let empty = fixture(None, Provider::default()).await;
    assert_eq!(token(&empty).await, Err(AuthError::ReauthRequired));
    assert_eq!(empty.broker.client().refreshes.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn hanging_provider_times_out() {
    let f = fixture(
        Some("r1"),
        Provider {
            hang: true,
            ..Provider::default()
        },
    )
    .await;
    assert!(matches!(
        token(&f).await,
        Err(AuthError::ProviderUnavailable { .. })
    ));
}

#[tokio::test]
async fn unreachable_provider_is_not_a_rejected_password() {
    let f = fixture(Some("panne"), Provider::default()).await;
    let smtp = FreshTokens {
        inner: Smtp::default(),
        broker: Arc::clone(&f.broker),
    };
    assert!(matches!(
        smtp.verify(&oauth_request()).await,
        Err(ConnectionError::Authentication(
            AuthError::ProviderUnavailable { .. }
        ))
    ));
    assert_eq!(
        f.broker.client().refreshes.load(Ordering::SeqCst),
        1,
        "pas de second rafraîchissement : le fournisseur est en cause"
    );
    assert!(seen(&smtp).is_empty(), "aucun jeton présenté au serveur");
}

#[tokio::test(start_paused = true)]
async fn a_slow_account_does_not_block_the_others() {
    let f = fixture(Some("r1"), Provider::default()).await;
    let _ = f
        .broker
        .secrets
        .set(&key_of(8), &Secret::new("lent".to_owned()))
        .await;
    let broker = Arc::clone(&f.broker);
    let slow = tokio::spawn(async move { broker.access_token(&source_of(8)).await });
    tokio::task::yield_now().await;
    let other = tokio::time::timeout(Duration::from_secs(1), token(&f)).await;
    assert_eq!(
        other,
        Ok(Ok("a1".to_owned())),
        "autre compte servi sans attendre"
    );
    slow.abort();
}

#[tokio::test]
async fn locked_keyring_is_reported_without_asking_the_provider() {
    let f = fixture(Some("r1"), Provider::default()).await;
    f.broker.secrets.locked.store(true, Ordering::SeqCst);
    assert!(matches!(
        token(&f).await,
        Err(AuthError::SecretStoreUnavailable { .. })
    ));
    assert_eq!(f.broker.client().refreshes.load(Ordering::SeqCst), 0);
    let smtp = FreshTokens {
        inner: Smtp::default(),
        broker: Arc::clone(&f.broker),
    };
    assert!(matches!(
        smtp.verify(&oauth_request()).await,
        Err(ConnectionError::Authentication(
            AuthError::SecretStoreUnavailable { .. }
        ))
    ));
    assert!(seen(&smtp).is_empty(), "aucun jeton présenté au serveur");
}

#[tokio::test]
async fn unsaved_rotation_is_kept_in_memory_and_saved_later() {
    let f = fixture(Some("r1"), Provider::default()).await;
    f.broker.secrets.read_only.store(true, Ordering::SeqCst);
    assert_eq!(
        token(&f).await,
        Ok("a1".to_owned()),
        "jeton servi malgré tout"
    );
    assert_eq!(f.broker.secrets.read(&key()).as_deref(), Some("r1"));
    // Jeton d'accès expiré : le rafraîchissement suivant présente « r2 »,
    // gardé en mémoire, et non « r1 », que le fournisseur refuse désormais.
    f.now.fetch_add(7_200, Ordering::SeqCst);
    assert_eq!(token(&f).await, Ok("a2".to_owned()));
    // Trousseau de nouveau disponible : le dernier jeton y est enregistré.
    f.broker.secrets.read_only.store(false, Ordering::SeqCst);
    f.now.fetch_add(7_200, Ordering::SeqCst);
    assert_eq!(token(&f).await, Ok("a3".to_owned()));
    assert_eq!(f.broker.secrets.read(&key()).as_deref(), Some("r4"));
    f.broker.forget(&key()).await;
    assert_eq!(
        token(&f).await,
        Ok("a4".to_owned()),
        "relu dans le trousseau"
    );
}

#[tokio::test]
async fn removed_account_tokens_are_discarded() {
    let f = fixture(Some("r1"), Provider::default()).await;
    f.broker.secrets.read_only.store(true, Ordering::SeqCst);
    assert_eq!(token(&f).await, Ok("a1".to_owned()));
    // Suppression du compte : jetons oubliés ; le trousseau n'est pas encore
    // vidé, mais plus rien n'est servi ni enregistré pour ce compte.
    f.broker.discard(&key()).await;
    f.broker.secrets.read_only.store(false, Ordering::SeqCst);
    assert_eq!(token(&f).await, Err(AuthError::ReauthRequired));
    let grant = OAuthGrant {
        access_token: Secret::new("tardif".to_owned()),
        expires_in: 600,
        refresh_token: Some(Secret::new("r-tardif".to_owned())),
        identity: None,
        granted_scopes: None,
    };
    assert_eq!(f.broker.store_grant(&source(), grant).await, Ok(()));
    assert_eq!(f.broker.secrets.read(&key()).as_deref(), Some("r1"));
    let _ = f.broker.secrets.delete(&key()).await;
    assert_eq!(
        f.broker.client().refreshes.load(Ordering::SeqCst),
        1,
        "jeton gardé en mémoire jamais présenté après la suppression"
    );
    assert_eq!(f.broker.secrets.read(&key()), None, "rien réécrit");
}

#[tokio::test]
async fn reconnection_replaces_an_unsaved_refresh_token() {
    let f = fixture(Some("r1"), Provider::default()).await;
    f.broker.secrets.read_only.store(true, Ordering::SeqCst);
    assert_eq!(token(&f).await, Ok("a1".to_owned()));
    // Reconnexion : nouvelle chaîne de jetons chez le fournisseur.
    f.broker.secrets.read_only.store(false, Ordering::SeqCst);
    if let Ok(mut current) = f.broker.client().current.lock() {
        *current = "neuf".to_owned();
    }
    let grant = OAuthGrant {
        access_token: Secret::new("reconnecté".to_owned()),
        expires_in: 600,
        refresh_token: Some(Secret::new("neuf".to_owned())),
        identity: None,
        granted_scopes: None,
    };
    assert_eq!(f.broker.store_grant(&source(), grant).await, Ok(()));
    f.now.fetch_add(7_200, Ordering::SeqCst);
    assert_eq!(
        token(&f).await,
        Ok("a2".to_owned()),
        "jeton de la reconnexion présenté, pas l'ancien gardé en mémoire"
    );
}

#[tokio::test]
async fn server_refusal_keeps_the_unsaved_refresh_token() {
    let f = fixture(Some("r1"), Provider::default()).await;
    f.broker.secrets.read_only.store(true, Ordering::SeqCst);
    let smtp = FreshTokens {
        inner: Smtp {
            refused: vec!["a1"],
            ..Smtp::default()
        },
        broker: Arc::clone(&f.broker),
    };
    // « a1 » refusé : jeton oublié, nouveau rafraîchissement avec « r2 »,
    // gardé en mémoire (le trousseau contient toujours « r1 », périmé).
    assert!(smtp.verify(&oauth_request()).await.is_ok());
    assert_eq!(seen(&smtp), vec!["a1".to_owned(), "a2".to_owned()]);
}

#[tokio::test]
async fn refused_unsaved_refresh_token_requires_reauth() {
    let f = fixture(Some("r1"), Provider::default()).await;
    f.broker.secrets.read_only.store(true, Ordering::SeqCst);
    assert_eq!(token(&f).await, Ok("a1".to_owned()));
    // Accès retiré chez le fournisseur : plus aucun jeton accepté.
    if let Ok(mut current) = f.broker.client().current.lock() {
        *current = "révoqué".to_owned();
    }
    f.now.fetch_add(7_200, Ordering::SeqCst);
    assert_eq!(token(&f).await, Err(AuthError::ReauthRequired));
}

#[tokio::test]
async fn abandoned_account_leaves_nothing_in_the_keyring() {
    let f = fixture(Some("r1"), Provider::default()).await;
    f.broker.abandon(&key()).await;
    assert_eq!(
        f.broker.secrets.read(&key()),
        None,
        "jeton effacé du trousseau"
    );
    assert_eq!(token(&f).await, Err(AuthError::ReauthRequired));
}

#[tokio::test]
async fn failed_removal_can_be_reconnected() {
    let f = fixture(Some("r1"), Provider::default()).await;
    f.broker.discard(&key()).await;
    f.broker.restore(&key()).await;
    let grant = OAuthGrant {
        access_token: Secret::new("reconnecté".to_owned()),
        expires_in: 600,
        refresh_token: Some(Secret::new("r-neuf".to_owned())),
        identity: None,
        granted_scopes: None,
    };
    assert_eq!(f.broker.store_grant(&source(), grant).await, Ok(()));
    assert_eq!(f.broker.secrets.read(&key()).as_deref(), Some("r-neuf"));
    assert_eq!(token(&f).await, Ok("reconnecté".to_owned()));
}

#[tokio::test(start_paused = true)]
async fn discard_waits_for_a_refresh_in_progress() {
    let f = fixture(
        Some("r1"),
        Provider {
            delay: Duration::from_secs(20),
            ..Provider::default()
        },
    )
    .await;
    let broker = Arc::clone(&f.broker);
    let refresh = tokio::spawn(async move { broker.access_token(&source()).await.is_ok() });
    tokio::task::yield_now().await;
    f.broker.discard(&key()).await;
    assert!(
        refresh.is_finished(),
        "rafraîchissement achevé avant l'oubli"
    );
}

#[tokio::test(start_paused = true)]
async fn reconnection_does_not_wait_forever_on_the_keyring() {
    let broker = TokenBroker::new(
        Provider::default(),
        HangingWrites::default(),
        Arc::new(|| 1_000_000),
    );
    let grant = OAuthGrant {
        access_token: Secret::new("a".to_owned()),
        expires_in: 600,
        refresh_token: Some(Secret::new("r".to_owned())),
        identity: None,
        granted_scopes: None,
    };
    let stored =
        tokio::time::timeout(REFRESH_TIMEOUT * 2, broker.store_grant(&source(), grant)).await;
    assert!(matches!(stored, Ok(Err(_))), "{stored:?}");
}

/// Faux trousseau dont l'écriture ne répond jamais.
#[derive(Debug, Default)]
struct HangingWrites(Keyring);

impl SecretStore for HangingWrites {
    async fn get(&self, key: &SecretKey) -> Result<Option<Secret>, SecretStoreError> {
        self.0.get(key).await
    }
    async fn set(&self, _: &SecretKey, _: &Secret) -> Result<(), SecretStoreError> {
        std::future::pending().await
    }
    async fn delete(&self, key: &SecretKey) -> Result<(), SecretStoreError> {
        self.0.delete(key).await
    }
}

#[tokio::test(start_paused = true)]
async fn refresh_and_save_share_one_deadline() {
    let keyring = HangingWrites::default();
    if let Ok(mut m) = keyring.0.entries.lock() {
        m.insert(key().as_str().to_owned(), "r1".to_owned());
    }
    let slow = Provider {
        delay: Duration::from_secs(20),
        ..Provider::default()
    };
    let broker = TokenBroker::new(slow, keyring, Arc::new(|| 1_000_000));
    let started = tokio::time::Instant::now();
    let token = broker.access_token(&source()).await;
    assert_eq!(token.map(|t| t.expose().to_owned()), Ok("a1".to_owned()));
    assert!(
        started.elapsed() <= REFRESH_TIMEOUT,
        "attente totale bornée à REFRESH_TIMEOUT : {:?}",
        started.elapsed()
    );
}

/// Faux SMTP : refuse les jetons de `refused`, retient les jetons reçus.
#[derive(Debug, Default)]
struct Smtp {
    refused: Vec<&'static str>,
    seen: StdMutex<Vec<String>>,
}

impl MailSender for Smtp {
    async fn verify(
        &self,
        request: &ConnectRequest,
    ) -> Result<ServerCapabilities, ConnectionError> {
        let t = request.credentials.secret().expose().to_owned();
        if let Ok(mut seen) = self.seen.lock() {
            seen.push(t.clone());
        }
        if self.refused.contains(&t.as_str()) {
            return Err(AuthError::Rejected {
                message: "535 5.7.3".to_owned(),
            }
            .into());
        }
        Ok(ServerCapabilities::from_smtp_ehlo(["AUTH XOAUTH2"]))
    }
    async fn send(&self, request: &ConnectRequest, _: &OutgoingMessage) -> Result<(), SendError> {
        self.verify(request).await.map(|_| ())?;
        Ok(())
    }
}

fn oauth_request() -> ConnectRequest {
    let (Ok(h), Ok(u)) = (
        Host::parse("smtp-mail.outlook.com"),
        Username::parse("a@outlook.com"),
    ) else {
        panic!("valeurs valides refusées");
    };
    let Ok(endpoint) = ServerEndpoint::new(h, 587, Security::StartTls) else {
        panic!("point d'accès valide refusé");
    };
    let Ok(credentials) = Credentials::oauth2(
        u,
        Secret::new("perime".to_owned()),
        OAuthProvider::Microsoft,
    ) else {
        panic!("jeton valide refusé");
    };
    ConnectRequest {
        endpoint,
        credentials: credentials.with_refresh_key(key()),
        certificate_exceptions: Vec::new(),
        plaintext: PlaintextPolicy::deny_all(),
    }
}

fn seen(s: &FreshTokens<Smtp, Provider, Keyring>) -> Vec<String> {
    s.inner.seen.lock().map(|v| v.clone()).unwrap_or_default()
}

#[tokio::test]
async fn connection_gets_a_fresh_token() {
    let f = fixture(Some("r1"), Provider::default()).await;
    let smtp = FreshTokens {
        inner: Smtp::default(),
        broker: f.broker,
    };
    assert!(smtp.verify(&oauth_request()).await.is_ok());
    assert_eq!(seen(&smtp), vec!["a1".to_owned()]);
}

#[tokio::test]
async fn token_refused_by_server_is_refreshed_once() {
    let f = fixture(Some("r1"), Provider::default()).await;
    let smtp = FreshTokens {
        inner: Smtp {
            refused: vec!["a1"],
            ..Smtp::default()
        },
        broker: f.broker,
    };
    let message = OutgoingMessage {
        from: "a@outlook.com".to_owned(),
        recipients: vec!["b@example.com".to_owned()],
        raw: b"x".to_vec(),
    };
    assert_eq!(smtp.send(&oauth_request(), &message).await, Ok(()));
    assert_eq!(seen(&smtp), vec!["a1".to_owned(), "a2".to_owned()]);
    let always = FreshTokens {
        inner: Smtp {
            refused: vec!["a1", "a2", "a3"],
            ..Smtp::default()
        },
        broker: fixture(Some("r1"), Provider::default()).await.broker,
    };
    assert!(always.verify(&oauth_request()).await.is_err());
    assert_eq!(seen(&always).len(), 2, "une seule nouvelle tentative");
}

#[tokio::test]
async fn password_requests_pass_through() {
    let f = fixture(None, Provider::default()).await;
    let smtp = FreshTokens {
        inner: Smtp::default(),
        broker: f.broker,
    };
    let mut request = oauth_request();
    let (Ok(u),) = (Username::parse("a@example.com"),) else {
        panic!("utilisateur valide refusé");
    };
    let Ok(pw) = Credentials::new(u, Secret::new("pw".to_owned())) else {
        panic!("identifiants valides refusés");
    };
    request.credentials = pw;
    assert!(smtp.verify(&request).await.is_ok());
    assert_eq!(seen(&smtp), vec!["pw".to_owned()]);
}

#[tokio::test]
async fn unconfigured_provider_is_refused_without_keyring_or_provider() {
    let keyring = Keyring::default();
    // Une lecture du trousseau donnerait `SecretStoreUnavailable`.
    keyring.locked.store(true, Ordering::SeqCst);
    let broker = TokenBroker::new(Provider::default(), keyring, Arc::new(|| 1_000_000))
        .with_configs(unconfigured_google_and_microsoft);
    assert_eq!(
        broker.access_token(&source()).await.map(|_| ()),
        Err(AuthError::ProviderNotConfigured)
    );
    assert_eq!(broker.client().refreshes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn only_the_unconfigured_provider_is_refused() {
    let f = fixture(Some("r1"), Provider::default()).await;
    let broker = Arc::try_unwrap(f.broker)
        .unwrap_or_else(|_| panic!("gestionnaire partagé"))
        .with_configs(unconfigured_google);
    assert!(broker.is_usable(OAuthProvider::Microsoft));
    assert!(!broker.is_usable(OAuthProvider::Google));
    assert_eq!(
        broker
            .access_token(&source())
            .await
            .map(|t| t.expose().to_owned()),
        Ok("a1".to_owned())
    );
}

#[test]
fn by_default_a_provider_is_usable_when_its_identifiers_were_compiled_in() {
    let broker = TokenBroker::new(Provider::default(), Keyring::default(), Arc::new(|| 0));
    for provider in [OAuthProvider::Microsoft, OAuthProvider::Google] {
        assert_eq!(broker.is_usable(provider), provider.config().is_usable());
    }
}

/// Version sans identifiants Google.
fn unconfigured_google(provider: OAuthProvider) -> OAuthConfig {
    match provider {
        OAuthProvider::Google => OAuthConfig {
            client_id: "",
            client_secret: Some(""),
            ..provider.config().clone()
        },
        OAuthProvider::Microsoft => provider.config().clone(),
    }
}

/// Aucun fournisseur utilisable.
fn unconfigured_google_and_microsoft(provider: OAuthProvider) -> OAuthConfig {
    OAuthConfig {
        client_id: "",
        ..provider.config().clone()
    }
}
