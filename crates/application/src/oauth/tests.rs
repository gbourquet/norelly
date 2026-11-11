//! Tests des jetons OAuth, sur faux client, faux trousseau et faux SMTP.

use std::collections::BTreeMap;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use norelly_domain::account::AccountId;
use norelly_domain::address::EmailAddress;
use norelly_domain::credentials::{Credentials, Username};
use norelly_domain::endpoint::{Host, Protocol, Security, ServerEndpoint};
use norelly_domain::oauth::OAuthProvider;
use norelly_domain::security::PlaintextPolicy;

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

/// Faux fournisseur : accepte le jeton de rafraîchissement « r1 », le fait
/// tourner en « r2 » et délivre « a1 », « a2 »… ; refuse tout autre jeton.
#[derive(Debug, Default)]
struct Provider {
    refreshes: AtomicUsize,
    hang: bool,
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
        if self.hang {
            std::future::pending::<()>().await;
        }
        let n = self.refreshes.fetch_add(1, Ordering::SeqCst) + 1;
        if !matches!(refresh.expose(), "r1" | "r2") {
            return Err(TokenError::ReauthRequired);
        }
        Ok(OAuthGrant {
            access_token: Secret::new(format!("a{n}")),
            expires_in: 3_600,
            refresh_token: Some(Secret::new("r2".to_owned())),
        })
    }
}

fn key() -> SecretKey {
    let Ok(id) = AccountId::new(7) else {
        panic!("identifiant valide refusé");
    };
    SecretKey::for_account(id, Protocol::Imap)
}

fn source() -> OAuthSource {
    OAuthSource {
        provider: OAuthProvider::Microsoft,
        refresh_key: key(),
    }
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
    assert!(matches!(token(&f).await, Err(AuthError::Rejected { .. })));
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
        Ok(ServerCapabilities::from_smtp_ehlo(&["AUTH XOAUTH2"]))
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
