//! Tests des cas d'usage de comptes, sur faux adaptateurs.

use std::collections::BTreeMap;
use std::sync::Mutex;

use norelly_domain::account::{AccountDraft, ServerSettings};
use norelly_domain::auth::AuthError;
use norelly_domain::connection::ConnectionError;
use norelly_domain::discovery::{
    AuthenticationAdvice, DiscoveredServers, ServerCandidate, UsernameTemplate,
};
use norelly_domain::ports::{SecretKey, SecretStoreError};
use norelly_domain::security::{CertificateFingerprint, CertificateIssue, PlaintextConsent};
use norelly_test_support::contracts::{
    InMemoryAccountRepository, InMemoryCertificateExceptionStore, RecordingAccountDataStore,
    sample_account,
};
use norelly_test_support::fakes::{FakeAutoconfig, FakeMailServers, FakeProber, FakeSrvResolver};

use super::*;

/// Trousseau en mémoire, avec panne simulable à partir de la n-ième
/// écriture ou à la suppression.
#[derive(Debug, Default)]
struct Keyring {
    secrets: Mutex<BTreeMap<String, String>>,
    fail_set_after: Mutex<Option<usize>>,
    fail_delete: Mutex<bool>,
}

impl Keyring {
    fn get_sync(&self, key: &str) -> Option<String> {
        self.secrets.lock().ok()?.get(key).cloned()
    }
    fn len(&self) -> usize {
        self.secrets.lock().map(|s| s.len()).unwrap_or(0)
    }
}

impl SecretStore for Keyring {
    async fn get(&self, key: &SecretKey) -> Result<Option<Secret>, SecretStoreError> {
        Ok(self.get_sync(key.as_str()).map(Secret::new))
    }
    async fn set(&self, key: &SecretKey, secret: &Secret) -> Result<(), SecretStoreError> {
        let mut fail = self
            .fail_set_after
            .lock()
            .map_err(|_| SecretStoreError::Backend("verrou".into()))?;
        if let Some(n) = fail.as_mut() {
            if *n == 0 {
                return Err(SecretStoreError::AccessDenied("refusé".into()));
            }
            *n -= 1;
        }
        self.secrets
            .lock()
            .map_err(|_| SecretStoreError::Backend("verrou".into()))?
            .insert(key.as_str().to_owned(), secret.expose().to_owned());
        Ok(())
    }
    async fn delete(&self, key: &SecretKey) -> Result<(), SecretStoreError> {
        if self.fail_delete.lock().map(|f| *f).unwrap_or(false) {
            return Err(SecretStoreError::Unavailable("verrouillé".into()));
        }
        self.secrets
            .lock()
            .map_err(|_| SecretStoreError::Backend("verrou".into()))?
            .remove(key.as_str());
        Ok(())
    }
}

/// Faux cache des jetons : retient les clés oubliées.
#[derive(Debug, Default)]
struct Tokens(std::sync::Mutex<Vec<String>>);

impl Tokens {
    fn discarded(&self) -> Vec<String> {
        self.0.lock().map(|v| v.clone()).unwrap_or_default()
    }
}

impl crate::oauth::TokenCache for Tokens {
    async fn discard(&self, key: &SecretKey) {
        if let Ok(mut v) = self.0.lock() {
            v.push(key.as_str().to_owned());
        }
    }
}

const PASSWORD: &str = "mot-de-passe";

struct World {
    servers: FakeMailServers,
    exceptions: InMemoryCertificateExceptionStore,
    autoconfig: FakeAutoconfig,
    keyring: Keyring,
    accounts: InMemoryAccountRepository,
    data: RecordingAccountDataStore,
    tokens: Tokens,
}

impl World {
    fn new() -> Self {
        Self {
            servers: FakeMailServers::default(),
            exceptions: InMemoryCertificateExceptionStore::default(),
            autoconfig: FakeAutoconfig::default(),
            keyring: Keyring::default(),
            accounts: InMemoryAccountRepository::default(),
            data: RecordingAccountDataStore::default(),
            tokens: Tokens::default(),
        }
    }

    async fn add(&self, request: AddAccountRequest) -> AddAccountOutcome {
        let test =
            TestConnection::new(self.servers.clone(), self.servers.clone(), &self.exceptions);
        let discover = DiscoverSettings::new(
            self.autoconfig.clone(),
            FakeSrvResolver::default(),
            FakeProber::default(),
        );
        AddAccount {
            test: &test,
            discover: &discover,
            secrets: &self.keyring,
            accounts: &self.accounts,
        }
        .execute(request)
        .await
    }

    async fn remove(&self, id: AccountId) -> Result<(), RemoveAccountError> {
        RemoveAccount {
            secrets: &self.keyring,
            accounts: &self.accounts,
            local_data: &self.data,
            tokens: &self.tokens,
        }
        .execute(id)
        .await
    }
}

fn address() -> EmailAddress {
    EmailAddress::parse("alice@example.com").unwrap_or_else(|_| panic!("adresse"))
}

fn servers_of(draft: &AccountDraft) -> (ServerSettings, ServerSettings) {
    (draft.imap().clone(), draft.smtp().clone())
}

fn request(servers: Option<(ServerSettings, ServerSettings)>) -> AddAccountRequest {
    AddAccountRequest {
        address: address(),
        display_name: None,
        password: Secret::new(PASSWORD.to_owned()),
        servers,
        plaintext: PlaintextPolicy::deny_all(),
    }
}

#[tokio::test]
async fn add_with_explicit_servers_stores_secret_and_account() {
    let w = World::new();
    let draft = sample_account("alice@example.com", false);
    let outcome = w.add(request(Some(servers_of(&draft)))).await;
    let AddAccountOutcome::Added(account) = outcome else {
        panic!("échec : {outcome:?}");
    };
    assert_eq!(account.display_name().as_str(), "alice");
    assert_eq!(account.imap(), draft.imap());
    // Deux tests de connexion (IMAP puis SMTP), avec le mot de passe.
    let requests = w.servers.requests();
    assert_eq!(
        requests.iter().map(|(p, _)| *p).collect::<Vec<_>>(),
        [Protocol::Imap, Protocol::Smtp]
    );
    assert!(
        requests
            .iter()
            .all(|(_, r)| r.credentials.secret().expose() == PASSWORD)
    );
    // Secrets dans le trousseau, sous les clés du compte.
    for protocol in [Protocol::Imap, Protocol::Smtp] {
        assert_eq!(
            w.keyring.get_sync(account.secret_key(protocol).as_str()),
            Some(PASSWORD.to_owned())
        );
    }
    assert_eq!(w.accounts.list().await, Ok(vec![account]));
}

#[tokio::test]
async fn add_uses_discovery_when_no_servers_are_given() {
    let w = World::new();
    let draft = sample_account("alice@example.com", false);
    let mut found = DiscoveredServers::default();
    for s in [draft.imap(), draft.smtp()] {
        found.push(ServerCandidate {
            protocol: s.protocol,
            endpoint: s.endpoint.clone(),
            username: UsernameTemplate::email_address(),
        });
    }
    w.autoconfig.set(
        "https://autoconfig.example.com/mail/config-v1.1.xml?emailaddress=alice%40example.com",
        Ok(Some(found)),
    );
    let outcome = w.add(request(None)).await;
    assert!(
        matches!(outcome, AddAccountOutcome::Added(_)),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn incomplete_discovery_asks_for_review() {
    let w = World::new();
    let outcome = w.add(request(None)).await;
    let AddAccountOutcome::NeedsReview(discovery) = outcome else {
        panic!("{outcome:?}");
    };
    assert!(discovery.imap().is_none());
    assert_eq!(w.keyring.len(), 0);
    assert!(w.servers.requests().is_empty());
}

#[tokio::test]
async fn duplicate_address_is_refused_before_any_connection() {
    let w = World::new();
    let draft = sample_account("alice@example.com", false);
    let _ = w.accounts.insert(&draft).await;
    let outcome = w.add(request(Some(servers_of(&draft)))).await;
    assert_eq!(outcome, AddAccountOutcome::AlreadyExists);
    assert!(w.servers.requests().is_empty());
}

#[tokio::test]
async fn plaintext_without_consent_is_refused_before_any_connection() {
    let w = World::new();
    let draft = sample_account("alice@example.com", true);
    let outcome = w.add(request(Some(servers_of(&draft)))).await;
    assert_eq!(
        outcome,
        AddAccountOutcome::PlaintextConsentRequired(draft.imap().endpoint.clone())
    );
    assert!(w.servers.requests().is_empty());

    // Avec le consentement explicite pour ce serveur : accepté.
    let mut req = request(Some(servers_of(&draft)));
    req.plaintext = PlaintextPolicy::deny_all().with_consent(PlaintextConsent {
        host: draft.imap().endpoint.host().clone(),
        port: draft.imap().endpoint.port(),
    });
    let outcome = w.add(req).await;
    assert!(
        matches!(outcome, AddAccountOutcome::Added(_)),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn certificate_problem_is_reported_then_trusted() {
    let w = World::new();
    let draft = sample_account("alice@example.com", false);
    let rejection = CertificateRejection {
        host: draft.imap().endpoint.host().clone(),
        fingerprint: CertificateFingerprint::from_bytes([7; 32]),
        issue: CertificateIssue::UnknownIssuer,
        fingerprint_changed: false,
    };
    w.servers.set(
        &draft.imap().endpoint,
        Err(ConnectionError::Certificate(rejection.clone())),
    );
    let outcome = w.add(request(Some(servers_of(&draft)))).await;
    assert_eq!(
        outcome,
        AddAccountOutcome::ConnectionFailed {
            protocol: Protocol::Imap,
            outcome: TestConnectionOutcome::Certificate(rejection.clone()),
            advice: None,
        }
    );
    assert_eq!(w.keyring.len(), 0, "secret écrit malgré l'échec");
    assert!(w.accounts.list().await.is_ok_and(|l| l.is_empty()));

    // L'utilisateur fait confiance : l'exception est enregistrée.
    assert_eq!(trust_certificate(&w.exceptions, &rejection).await, Ok(()));
    let stored = w.exceptions.exceptions_for(&rejection.host).await;
    assert_eq!(stored.map(|v| v.len()), Ok(1));

    // Un certificat révoqué ne peut pas faire l'objet d'une exception.
    let revoked = CertificateRejection {
        issue: CertificateIssue::Revoked,
        ..rejection
    };
    assert_eq!(
        trust_certificate(&w.exceptions, &revoked).await,
        Err(TrustCertificateError::NotExceptable)
    );
}

#[tokio::test]
async fn smtp_authentication_failure_is_reported() {
    let w = World::new();
    let draft = sample_account("alice@example.com", false);
    let auth = AuthError::Rejected {
        message: "mauvais mot de passe".into(),
    };
    w.servers.set(
        &draft.smtp().endpoint,
        Err(ConnectionError::Authentication(auth.clone())),
    );
    let outcome = w.add(request(Some(servers_of(&draft)))).await;
    assert_eq!(
        outcome,
        AddAccountOutcome::ConnectionFailed {
            protocol: Protocol::Smtp,
            outcome: TestConnectionOutcome::Authentication(auth),
            advice: None,
        }
    );
    assert_eq!(w.keyring.len(), 0);
}

/// Serveurs saisis ou découverts : un port ou un chiffrement faux fait
/// aussi taire le serveur d'envoi, l'ajout exige donc un envoi vérifié.
#[tokio::test(start_paused = true)]
async fn silent_send_server_blocks_a_password_account() {
    let w = World::new();
    let draft = sample_account("alice@example.com", false);
    w.servers
        .set(&draft.smtp().endpoint, Err(ConnectionError::Closed));
    let outcome = w.add(request(Some(servers_of(&draft)))).await;
    assert_eq!(
        outcome,
        AddAccountOutcome::ConnectionFailed {
            protocol: Protocol::Smtp,
            outcome: TestConnectionOutcome::Network(ConnectionError::Closed),
            advice: None,
        }
    );
    assert_eq!(w.keyring.len(), 0);
}

#[tokio::test(start_paused = true)]
async fn unreachable_send_server_blocks_the_account() {
    let w = World::new();
    let draft = sample_account("alice@example.com", false);
    let unknown = ConnectionError::Network("nom inconnu".into());
    w.servers.set(&draft.smtp().endpoint, Err(unknown.clone()));
    let outcome = w.add(request(Some(servers_of(&draft)))).await;
    assert_eq!(
        outcome,
        AddAccountOutcome::ConnectionFailed {
            protocol: Protocol::Smtp,
            outcome: TestConnectionOutcome::Network(unknown),
            advice: None,
        }
    );
    assert_eq!(w.keyring.len(), 0);
}

#[tokio::test]
async fn gmail_rejection_advises_an_app_password() {
    let w = World::new();
    let draft = sample_account("alice@gmail.com", false);
    let auth = AuthError::Rejected {
        message: "[AUTHENTICATIONFAILED] Invalid credentials".into(),
    };
    w.servers.set(
        &draft.imap().endpoint,
        Err(ConnectionError::Authentication(auth.clone())),
    );
    let gmail = AddAccountRequest {
        address: EmailAddress::parse("alice@gmail.com").unwrap_or_else(|_| panic!("adresse")),
        ..request(Some(servers_of(&draft)))
    };
    let outcome = w.add(gmail).await;
    assert_eq!(
        outcome,
        AddAccountOutcome::ConnectionFailed {
            protocol: Protocol::Imap,
            outcome: TestConnectionOutcome::Authentication(auth),
            advice: Some(AuthenticationAdvice::GoogleAppPassword),
        }
    );
    assert_eq!(w.keyring.len(), 0);
}

#[tokio::test]
async fn keyring_failure_rolls_back_written_secrets() {
    let w = World::new();
    if let Ok(mut f) = w.keyring.fail_set_after.lock() {
        *f = Some(1);
    }
    let draft = sample_account("alice@example.com", false);
    let outcome = w.add(request(Some(servers_of(&draft)))).await;
    assert!(
        matches!(outcome, AddAccountOutcome::SecretStoreFailed(_)),
        "{outcome:?}"
    );
    assert_eq!(w.keyring.len(), 0, "secret IMAP non retiré");
    assert!(w.accounts.list().await.is_ok_and(|l| l.is_empty()));
}

#[tokio::test]
async fn invalid_username_is_reported() {
    let w = World::new();
    let draft = sample_account("alice@example.com", false);
    let (imap, smtp) = servers_of(&draft);
    let mut req = request(Some((imap, smtp)));
    req.password = Secret::new(String::new());
    let outcome = w.add(req).await;
    assert!(
        matches!(outcome, AddAccountOutcome::Invalid(_)),
        "{outcome:?}"
    );
    assert!(w.servers.requests().is_empty());
}

#[tokio::test]
async fn remove_deletes_secrets_account_and_local_data() {
    let w = World::new();
    let draft = sample_account("alice@example.com", false);
    let AddAccountOutcome::Added(account) = w.add(request(Some(servers_of(&draft)))).await else {
        panic!("ajout");
    };
    let other = w.add({
        let d = sample_account("bob@example.org", false);
        AddAccountRequest {
            address: d.address().clone(),
            ..request(Some(servers_of(&d)))
        }
    });
    let AddAccountOutcome::Added(other) = other.await else {
        panic!("ajout");
    };
    assert_eq!(w.keyring.len(), 4);

    assert_eq!(w.remove(account.id()).await, Ok(()));
    assert_eq!(w.keyring.len(), 2, "secrets de l'autre compte touchés");
    assert!(
        w.keyring
            .get_sync(other.secret_key(Protocol::Imap).as_str())
            .is_some()
    );
    assert_eq!(w.accounts.list().await, Ok(vec![other]));
    assert_eq!(w.data.purged(), vec![account.id()]);
    assert!(
        w.tokens
            .discarded()
            .contains(&account.secret_key(Protocol::Imap).as_str().to_owned()),
        "jetons OAuth gardés en mémoire oubliés"
    );
    assert_eq!(
        w.remove(account.id()).await,
        Err(RemoveAccountError::NotFound)
    );
}

#[tokio::test]
async fn remove_keeps_the_account_if_the_keyring_fails() {
    let w = World::new();
    let draft = sample_account("alice@example.com", false);
    let AddAccountOutcome::Added(account) = w.add(request(Some(servers_of(&draft)))).await else {
        panic!("ajout");
    };
    if let Ok(mut f) = w.keyring.fail_delete.lock() {
        *f = true;
    }
    assert!(matches!(
        w.remove(account.id()).await,
        Err(RemoveAccountError::SecretStore(_))
    ));
    assert_eq!(w.accounts.list().await.map(|l| l.len()), Ok(1));
    assert!(w.data.purged().is_empty());
}

/// Deux adresses (alias) avec le même identifiant sur le même serveur : un
/// secret chacune, et supprimer l'une ne touche pas l'autre.
#[tokio::test]
async fn aliases_on_the_same_login_keep_separate_secrets() {
    let w = World::new();
    let draft = sample_account("alice@example.com", false);
    let AddAccountOutcome::Added(alice) = w.add(request(Some(servers_of(&draft)))).await else {
        panic!("ajout");
    };
    let alias = AddAccountRequest {
        address: EmailAddress::parse("contact@example.com").unwrap_or_else(|_| panic!("adresse")),
        password: Secret::new("autre".to_owned()),
        ..request(Some(servers_of(&draft)))
    };
    let AddAccountOutcome::Added(contact) = w.add(alias).await else {
        panic!("ajout de l'alias");
    };
    assert_ne!(
        alice.secret_key(Protocol::Imap),
        contact.secret_key(Protocol::Imap)
    );
    assert_eq!(
        w.keyring
            .get_sync(alice.secret_key(Protocol::Imap).as_str()),
        Some(PASSWORD.to_owned())
    );
    assert_eq!(w.remove(contact.id()).await, Ok(()));
    assert_eq!(
        w.keyring
            .get_sync(alice.secret_key(Protocol::Smtp).as_str()),
        Some(PASSWORD.to_owned()),
        "secret de l'autre compte supprimé"
    );
}

/// Un doublon détecté à l'insertion (ajouts simultanés) n'écrit ni ne
/// supprime aucun secret.
#[tokio::test]
async fn duplicate_at_insert_leaves_the_keyring_untouched() {
    let w = World::new();
    let draft = sample_account("alice@example.com", false);
    let AddAccountOutcome::Added(first) = w.add(request(Some(servers_of(&draft)))).await else {
        panic!("ajout");
    };
    // Simule la course : le dépôt refuse l'insertion alors que
    // `find_by_address` n'a rien vu.
    let racing = InMemoryAccountRepository::default();
    let _ = racing.insert(&draft).await;
    let test = TestConnection::new(w.servers.clone(), w.servers.clone(), &w.exceptions);
    let discover = DiscoverSettings::new(
        w.autoconfig.clone(),
        FakeSrvResolver::default(),
        FakeProber::default(),
    );
    let before = w.keyring.len();
    let outcome = AddAccount {
        test: &test,
        discover: &discover,
        secrets: &w.keyring,
        accounts: &RacingRepository(racing),
    }
    .execute(request(Some(servers_of(&draft))))
    .await;
    assert_eq!(outcome, AddAccountOutcome::AlreadyExists);
    assert_eq!(w.keyring.len(), before);
    assert_eq!(
        w.keyring
            .get_sync(first.secret_key(Protocol::Imap).as_str()),
        Some(PASSWORD.to_owned())
    );
}

/// Dépôt dont `find_by_address` ne voit rien (course entre deux ajouts).
struct RacingRepository(InMemoryAccountRepository);

impl AccountRepository for RacingRepository {
    async fn list(&self) -> Result<Vec<Account>, AccountStoreError> {
        self.0.list().await
    }
    async fn get(&self, id: AccountId) -> Result<Option<Account>, AccountStoreError> {
        self.0.get(id).await
    }
    async fn find_by_address(
        &self,
        _: &EmailAddress,
    ) -> Result<Option<Account>, AccountStoreError> {
        Ok(None)
    }
    async fn insert(&self, draft: &AccountDraft) -> Result<Account, AccountStoreError> {
        self.0.insert(draft).await
    }
    async fn delete(&self, id: AccountId) -> Result<bool, AccountStoreError> {
        self.0.delete(id).await
    }
}

/// Découverte avec SRV hors domaine : aucune connexion, révision demandée.
#[tokio::test]
async fn srv_outside_domain_is_never_used_without_review() {
    use norelly_domain::discovery::{SrvRecord, SrvService};
    use norelly_domain::endpoint::Host;
    let w = World::new();
    let srv = FakeSrvResolver::default();
    let rec = |host: &str, port| SrvRecord {
        priority: 0,
        weight: 0,
        port,
        target: Host::parse(host).ok(),
    };
    srv.set(
        SrvService::Imaps,
        "example.com",
        Ok(vec![rec("imap.attaquant.net", 993)]),
    );
    srv.set(
        SrvService::Submissions,
        "example.com",
        Ok(vec![rec("smtp.example.com", 465)]),
    );
    let test = TestConnection::new(w.servers.clone(), w.servers.clone(), &w.exceptions);
    let discover = DiscoverSettings::new(w.autoconfig.clone(), srv, FakeProber::default());
    let outcome = AddAccount {
        test: &test,
        discover: &discover,
        secrets: &w.keyring,
        accounts: &w.accounts,
    }
    .execute(request(None))
    .await;
    assert!(
        matches!(outcome, AddAccountOutcome::NeedsReview(_)),
        "{outcome:?}"
    );
    assert!(
        w.servers.requests().is_empty(),
        "connexion vers un hôte non confirmé"
    );
}

#[tokio::test]
async fn trusting_a_changed_fingerprint_replaces_the_old_one() {
    use norelly_domain::endpoint::Host;
    let w = World::new();
    let host = Host::parse("imap.example.com").unwrap_or_else(|_| panic!("hôte"));
    let old = CertificateException {
        host: host.clone(),
        fingerprint: CertificateFingerprint::from_bytes([1; 32]),
    };
    let _ = w.exceptions.add(&old).await;
    let changed = CertificateRejection {
        host: host.clone(),
        fingerprint: CertificateFingerprint::from_bytes([2; 32]),
        issue: CertificateIssue::UnknownIssuer,
        fingerprint_changed: true,
    };
    assert_eq!(trust_certificate(&w.exceptions, &changed).await, Ok(()));
    assert_eq!(
        w.exceptions.exceptions_for(&host).await,
        Ok(vec![CertificateException {
            host,
            fingerprint: changed.fingerprint,
        }])
    );
}

#[tokio::test]
async fn connect_request_uses_keyring_exceptions_and_plaintext_consent() {
    let account = sample_account("alice@example.com", true)
        .with_id(AccountId::new(3).unwrap_or_else(|_| panic!("id")));
    let keyring = Keyring::default();
    let exceptions = InMemoryCertificateExceptionStore::default();
    assert_eq!(
        connect_request(&account, Protocol::Imap, &keyring, &exceptions)
            .await
            .map(|_| ()),
        Err(ConnectRequestError::MissingSecret)
    );
    keyring
        .set(
            &account.secret_key(Protocol::Imap),
            &Secret::new("mdp".into()),
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let exception = CertificateException {
        host: account.imap().endpoint.host().clone(),
        fingerprint: CertificateFingerprint::from_bytes([7; 32]),
    };
    exceptions
        .add(&exception)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let request = connect_request(&account, Protocol::Imap, &keyring, &exceptions)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(request.endpoint, account.imap().endpoint);
    assert_eq!(request.credentials.secret().expose(), "mdp");
    assert_eq!(request.certificate_exceptions, [exception]);
    assert_eq!(request.plaintext, account.plaintext_policy());
}

// ---------------------------------------------------------------------------
// Comptes OAuth
// ---------------------------------------------------------------------------

mod oauth {
    use std::sync::Arc;

    use norelly_domain::oauth::{OAuthIdentity, OAuthProvider, TokenError};
    use norelly_domain::ports::OAuthGrant;
    use norelly_test_support::contracts::FakeOAuthClient;

    use super::*;
    use crate::oauth::TokenBroker;

    fn outlook() -> EmailAddress {
        EmailAddress::parse("alice@outlook.com").unwrap_or_else(|_| panic!("adresse"))
    }

    fn broker(grant: Option<OAuthGrant>) -> TokenBroker<FakeOAuthClient, Keyring> {
        let client = FakeOAuthClient::default();
        if let Ok(mut g) = client.grant.lock() {
            *g = grant;
        }
        TokenBroker::new(client, Keyring::default(), Arc::new(|| 1_000))
    }

    fn grant() -> Option<OAuthGrant> {
        grant_of("alice@outlook.com")
    }

    /// Jetons du compte `login` (identité de l'`id_token`).
    fn grant_of(login: &str) -> Option<OAuthGrant> {
        Some(OAuthGrant {
            access_token: Secret::new("acc".to_owned()),
            expires_in: 3_600,
            refresh_token: Some(Secret::new("r1".to_owned())),
            identity: Some(OAuthIdentity {
                preferred_username: Some(login.to_owned()),
                email: None,
            }),
        })
    }

    async fn add(w: &World, b: &TokenBroker<FakeOAuthClient, Keyring>) -> AddAccountOutcome {
        let test = TestConnection::new(w.servers.clone(), w.servers.clone(), &w.exceptions);
        AddOAuthAccount {
            test: &test,
            broker: b,
            accounts: &w.accounts,
        }
        .execute(AddOAuthAccountRequest {
            address: outlook(),
            display_name: None,
            provider: OAuthProvider::Microsoft,
        })
        .await
    }

    #[tokio::test]
    async fn adds_microsoft_account_with_provider_servers_and_token() {
        let w = World::new();
        let b = broker(grant());
        let AddAccountOutcome::Added(account) = add(&w, &b).await else {
            panic!("compte non ajouté");
        };
        assert_eq!(account.oauth_provider(), Some(OAuthProvider::Microsoft));
        assert_eq!(
            account.imap().endpoint.host().as_str(),
            "outlook.office365.com"
        );
        assert_eq!(account.smtp().endpoint.security(), Security::StartTls);
        assert_eq!(account.imap().username.as_str(), "alice@outlook.com");
        let tested = w.servers.requests();
        assert_eq!(tested.len(), 2, "IMAP et SMTP testés");
        assert!(
            tested
                .iter()
                .all(|(_, r)| r.credentials.secret().expose() == "acc"
                    && r.credentials.kind() == norelly_domain::credentials::CredentialKind::OAuth2)
        );
        let key = account.secret_key(Protocol::Imap);
        assert_eq!(
            b.access_token(&OAuthSource {
                provider: OAuthProvider::Microsoft,
                refresh_key: key.clone(),
            })
            .await
            .map(|t| t.expose().to_owned()),
            Ok("acc".to_owned())
        );
    }

    #[tokio::test]
    async fn refused_sign_in_adds_nothing() {
        let w = World::new();
        assert_eq!(
            add(&w, &broker(None)).await,
            AddAccountOutcome::OAuthFailed(TokenError::Cancelled("access_denied".to_owned()))
        );
        assert!(w.accounts.list().await.is_ok_and(|l| l.is_empty()));
        let without_refresh = Some(OAuthGrant {
            refresh_token: None,
            ..grant().unwrap_or_else(|| panic!())
        });
        assert_eq!(
            add(&w, &broker(without_refresh)).await,
            AddAccountOutcome::OAuthFailed(TokenError::OfflineAccessRefused)
        );
        assert!(w.accounts.list().await.is_ok_and(|l| l.is_empty()));
    }

    #[tokio::test]
    async fn smtp_disabled_on_mailbox_is_explained() {
        let w = World::new();
        let (_, smtp) =
            oauth_servers(OAuthProvider::Microsoft, &outlook()).unwrap_or_else(|e| panic!("{e}"));
        w.servers.set(
            &smtp.endpoint,
            Err(ConnectionError::Authentication(AuthError::Rejected {
                message:
                    "535 5.7.139 Authentication unsuccessful, SmtpClientAuthentication is disabled"
                        .to_owned(),
            })),
        );
        let outcome = add(&w, &broker(grant())).await;
        assert!(
            matches!(
                outcome,
                AddAccountOutcome::ConnectionFailed {
                    protocol: Protocol::Smtp,
                    advice: Some(AuthenticationAdvice::MicrosoftSmtpDisabled),
                    ..
                }
            ),
            "{outcome:?}"
        );
        assert!(w.accounts.list().await.is_ok_and(|l| l.is_empty()));
    }

    fn not_connected() -> ConnectionError {
        ConnectionError::Authentication(AuthError::Rejected {
            message: "User is authenticated but not connected.".to_owned(),
        })
    }

    fn imap_endpoint() -> norelly_domain::endpoint::ServerEndpoint {
        let (imap, _) =
            oauth_servers(OAuthProvider::Microsoft, &outlook()).unwrap_or_else(|e| panic!("{e}"));
        imap.endpoint
    }

    fn imap_attempts(w: &World) -> usize {
        w.servers
            .requests()
            .iter()
            .filter(|(p, _)| *p == Protocol::Imap)
            .count()
    }

    #[tokio::test(start_paused = true)]
    async fn intermittent_mailbox_binding_failure_is_retried() {
        let w = World::new();
        w.servers
            .fail_first(&imap_endpoint(), vec![not_connected(), not_connected()]);
        assert!(matches!(
            add(&w, &broker(grant())).await,
            AddAccountOutcome::Added(_)
        ));
        assert_eq!(imap_attempts(&w), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn persistent_mailbox_binding_failure_is_explained() {
        let w = World::new();
        w.servers.set(&imap_endpoint(), Err(not_connected()));
        let outcome = add(&w, &broker(grant())).await;
        assert!(
            matches!(
                outcome,
                AddAccountOutcome::ConnectionFailed {
                    protocol: Protocol::Imap,
                    advice: Some(AuthenticationAdvice::MicrosoftImapUnavailable),
                    ..
                }
            ),
            "{outcome:?}"
        );
        assert_eq!(imap_attempts(&w), OAUTH_IMAP_ATTEMPTS);
        assert!(w.accounts.list().await.is_ok_and(|l| l.is_empty()));
    }

    /// Gestionnaire de jetons dont l'horloge avance à la demande.
    fn broker_with_clock(
        grant: Option<OAuthGrant>,
    ) -> (
        TokenBroker<FakeOAuthClient, Keyring>,
        Arc<std::sync::atomic::AtomicU64>,
    ) {
        let client = FakeOAuthClient::default();
        if let Ok(mut g) = client.grant.lock() {
            *g = grant;
        }
        let now = Arc::new(std::sync::atomic::AtomicU64::new(1_000));
        let clock = Arc::clone(&now);
        let broker = TokenBroker::new(
            client,
            Keyring::default(),
            Arc::new(move || clock.load(std::sync::atomic::Ordering::SeqCst)),
        );
        (broker, now)
    }

    fn browser_sign_ins(b: &TokenBroker<FakeOAuthClient, Keyring>) -> usize {
        b.client().authorizations.lock().map_or(0, |a| a.len())
    }

    fn silent_imap(w: &World) {
        let silent = ConnectionError::Timeout(norelly_domain::connection::TimeoutStage::Response);
        w.servers.fail_first(
            &imap_endpoint(),
            vec![silent; norelly_domain::connection::CONNECTION_ATTEMPTS],
        );
    }

    #[tokio::test(start_paused = true)]
    async fn retry_after_a_failed_add_does_not_reopen_the_browser() {
        let w = World::new();
        let (b, _) = broker_with_clock(grant());
        silent_imap(&w);
        assert!(matches!(
            add(&w, &b).await,
            AddAccountOutcome::ConnectionFailed { .. }
        ));
        assert!(b.has_pending(OAuthProvider::Microsoft, &outlook()));
        assert!(matches!(add(&w, &b).await, AddAccountOutcome::Added(_)));
        assert_eq!(browser_sign_ins(&b), 1);
        assert!(!b.has_pending(OAuthProvider::Microsoft, &outlook()));
    }

    #[tokio::test(start_paused = true)]
    async fn kept_sign_in_expires() {
        let w = World::new();
        let (b, now) = broker_with_clock(grant());
        silent_imap(&w);
        assert!(matches!(
            add(&w, &b).await,
            AddAccountOutcome::ConnectionFailed { .. }
        ));
        now.fetch_add(
            norelly_domain::oauth::PENDING_AUTHORIZATION_MAX_AGE_SECS,
            std::sync::atomic::Ordering::SeqCst,
        );
        assert!(!b.has_pending(OAuthProvider::Microsoft, &outlook()));
        assert!(matches!(add(&w, &b).await, AddAccountOutcome::Added(_)));
        assert_eq!(browser_sign_ins(&b), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn refused_sign_in_is_not_kept() {
        let w = World::new();
        let (b, _) = broker_with_clock(grant());
        w.servers.set(
            &imap_endpoint(),
            Err(ConnectionError::Authentication(AuthError::Rejected {
                message: "AUTHENTICATE failed.".to_owned(),
            })),
        );
        assert!(matches!(
            add(&w, &b).await,
            AddAccountOutcome::ConnectionFailed { .. }
        ));
        assert!(!b.has_pending(OAuthProvider::Microsoft, &outlook()));
    }

    #[tokio::test(start_paused = true)]
    async fn reused_sign_in_is_not_kept_again() {
        let w = World::new();
        let (b, _) = broker_with_clock(grant());
        silent_imap(&w);
        assert!(matches!(
            add(&w, &b).await,
            AddAccountOutcome::ConnectionFailed { .. }
        ));
        silent_imap(&w);
        assert!(matches!(
            add(&w, &b).await,
            AddAccountOutcome::ConnectionFailed { .. }
        ));
        assert!(!b.has_pending(OAuthProvider::Microsoft, &outlook()));
        assert!(matches!(add(&w, &b).await, AddAccountOutcome::Added(_)));
        assert_eq!(browser_sign_ins(&b), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn kept_sign_in_age_counts_from_the_browser_sign_in() {
        let w = World::new();
        let (b, now) = broker_with_clock(grant());
        // L'horloge avance pendant les essais : 9 min après la connexion
        // dans le navigateur, puis encore 2 min avant le nouvel essai.
        let clock = Arc::clone(&now);
        w.servers.on_connect(move || {
            clock.fetch_add(180, std::sync::atomic::Ordering::SeqCst);
        });
        silent_imap(&w);
        assert!(matches!(
            add(&w, &b).await,
            AddAccountOutcome::ConnectionFailed { .. }
        ));
        now.fetch_add(120, std::sync::atomic::Ordering::SeqCst);
        assert!(!b.has_pending(OAuthProvider::Microsoft, &outlook()));
    }

    #[tokio::test(start_paused = true)]
    async fn silent_send_server_still_adds_the_oauth_account() {
        let w = World::new();
        let (b, _) = broker_with_clock(grant());
        let (_, smtp) =
            oauth_servers(OAuthProvider::Microsoft, &outlook()).unwrap_or_else(|e| panic!("{e}"));
        w.servers.set(
            &smtp.endpoint,
            Err(ConnectionError::Timeout(
                norelly_domain::connection::TimeoutStage::TlsHandshake,
            )),
        );
        let outcome = add(&w, &b).await;
        assert!(
            matches!(outcome, AddAccountOutcome::AddedSendUnchecked(_)),
            "{outcome:?}"
        );
        assert!(!b.has_pending(OAuthProvider::Microsoft, &outlook()));
        assert!(w.accounts.list().await.is_ok_and(|l| l.len() == 1));
    }

    #[tokio::test]
    async fn reconnect_replaces_refresh_token() {
        let w = World::new();
        let b = broker(grant());
        let AddAccountOutcome::Added(account) = add(&w, &b).await else {
            panic!("compte non ajouté");
        };
        if let Ok(mut g) = b.client().grant.lock() {
            *g = Some(OAuthGrant {
                access_token: Secret::new("neuf".to_owned()),
                expires_in: 3_600,
                refresh_token: Some(Secret::new("r-neuf".to_owned())),
                identity: grant().and_then(|g| g.identity),
            });
        }
        let reconnected = reconnect_oauth_account(&b, &w.accounts, account.id()).await;
        assert_eq!(reconnected.map(|a| a.id()), Ok(account.id()));
        let source = OAuthSource {
            provider: OAuthProvider::Microsoft,
            refresh_key: account.secret_key(Protocol::Imap),
        };
        assert_eq!(
            b.access_token(&source).await.map(|t| t.expose().to_owned()),
            Ok("neuf".to_owned())
        );
        if let Ok(mut g) = b.client().grant.lock() {
            *g = Some(OAuthGrant {
                refresh_token: None,
                ..grant().unwrap_or_else(|| panic!())
            });
        }
        assert_eq!(
            reconnect_oauth_account(&b, &w.accounts, account.id())
                .await
                .map(|_| ()),
            Err(ReconnectError::OAuth(TokenError::OfflineAccessRefused))
        );
        let Ok(missing) = AccountId::new(999) else {
            panic!()
        };
        assert_eq!(
            reconnect_oauth_account(&b, &w.accounts, missing)
                .await
                .map(|_| ()),
            Err(ReconnectError::NotFound)
        );
    }

    #[tokio::test]
    async fn sign_in_with_another_account_adds_nothing() {
        let w = World::new();
        let b = broker(grant_of("mallory@outlook.com"));
        assert_eq!(
            add(&w, &b).await,
            AddAccountOutcome::OAuthFailed(TokenError::WrongAccount)
        );
        assert!(w.accounts.list().await.is_ok_and(|l| l.is_empty()));
        // Le jeton d'un autre compte n'est présenté à aucun serveur.
        assert!(w.servers.requests().is_empty());
        // Les jetons d'un autre compte ne resservent pas au prochain essai.
        assert!(!b.has_pending(OAuthProvider::Microsoft, &outlook()));
        let without_identity = Some(OAuthGrant {
            identity: None,
            ..grant().unwrap_or_else(|| panic!())
        });
        assert!(matches!(
            add(&w, &broker(without_identity)).await,
            AddAccountOutcome::OAuthFailed(TokenError::Protocol(_))
        ));
        assert!(w.accounts.list().await.is_ok_and(|l| l.is_empty()));
    }

    #[tokio::test]
    async fn reconnect_with_another_account_keeps_the_old_token() {
        let w = World::new();
        let b = broker(grant());
        let AddAccountOutcome::Added(account) = add(&w, &b).await else {
            panic!("compte non ajouté");
        };
        if let Ok(mut g) = b.client().grant.lock() {
            *g = Some(OAuthGrant {
                access_token: Secret::new("autre".to_owned()),
                ..grant_of("mallory@outlook.com").unwrap_or_else(|| panic!())
            });
        }
        assert_eq!(
            reconnect_oauth_account(&b, &w.accounts, account.id())
                .await
                .map(|a| a.id()),
            Err(ReconnectError::OAuth(TokenError::WrongAccount))
        );
        let source = OAuthSource {
            provider: OAuthProvider::Microsoft,
            refresh_key: account.secret_key(Protocol::Imap),
        };
        assert_eq!(
            b.access_token(&source).await.map(|t| t.expose().to_owned()),
            Ok("acc".to_owned())
        );
    }
}
