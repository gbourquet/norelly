//! Tests des cas d'usage de comptes, sur faux adaptateurs.

use std::collections::BTreeMap;
use std::sync::Mutex;

use norelly_domain::account::{AccountDraft, ServerSettings};
use norelly_domain::auth::AuthError;
use norelly_domain::connection::ConnectionError;
use norelly_domain::discovery::{DiscoveredServers, ServerCandidate, UsernameTemplate};
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

const PASSWORD: &str = "mot-de-passe";

struct World {
    servers: FakeMailServers,
    exceptions: InMemoryCertificateExceptionStore,
    autoconfig: FakeAutoconfig,
    keyring: Keyring,
    accounts: InMemoryAccountRepository,
    data: RecordingAccountDataStore,
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
