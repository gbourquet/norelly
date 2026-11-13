//! Cas d'usage des comptes : `AddAccount`, `RemoveAccount` et
//! `TrustCertificate`, et paramètres de connexion d'un compte enregistré
//! ([`connect_request`]).
//!
//! `AddAccount` : découverte (si les serveurs ne sont pas fournis), test
//! des connexions IMAP et SMTP, persistance du compte, puis enregistrement
//! du mot de passe dans le trousseau sous des clés propres au compte. Si le
//! trousseau échoue, les secrets écrits et le compte sont retirés.

use norelly_domain::account::{
    Account, AccountDraft, AccountError, AccountId, DisplayName, ServerSettings,
};
use norelly_domain::address::EmailAddress;
use norelly_domain::credentials::{Credentials, Secret};
use norelly_domain::discovery::{AuthenticationAdvice, Discovery, authentication_advice};
use norelly_domain::endpoint::{Protocol, ServerEndpoint};
use norelly_domain::ports::{
    AccountDataStore, AccountRepository, AccountStoreError, AutoconfigClient,
    CertificateExceptionStore, ConnectRequest, MailSender, PortProber, RemoteMailboxConnector,
    SecretStore, SrvResolver,
};
use norelly_domain::security::{CertificateException, CertificateRejection, PlaintextPolicy};

use norelly_domain::auth::AuthError;
use norelly_domain::credentials::Username;
use norelly_domain::endpoint::{Host, Security};
use norelly_domain::oauth::{
    OAuthProvider, OAuthSource, TokenError, is_mailbox_not_connected, is_smtp_oauth_disabled,
};
use norelly_domain::ports::OAuthClient;

use crate::discover_settings::DiscoverSettings;
use crate::oauth::TokenBroker;
use crate::test_connection::{TestConnection, TestConnectionOutcome, TestConnectionParams};

/// Demande d'ajout de compte.
#[derive(Debug, Clone)]
pub struct AddAccountRequest {
    pub address: EmailAddress,
    /// Nom affiché ; par défaut, la partie locale de l'adresse.
    pub display_name: Option<DisplayName>,
    pub password: Secret,
    /// Serveurs IMAP et SMTP choisis (découverts puis confirmés, ou
    /// saisis) ; `None` : découverte automatique.
    pub servers: Option<(ServerSettings, ServerSettings)>,
    /// Consentements au clair donnés explicitement par l'utilisateur.
    pub plaintext: PlaintextPolicy,
}

/// Résultat d'un ajout de compte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddAccountOutcome {
    Added(Account),
    /// Compte OAuth ajouté, réception vérifiée ; le serveur d'envoi du
    /// fournisseur ne répondait pas ([`ConnectionError::send_check_can_wait`]) :
    /// l'envoi sera vérifié au premier message.
    AddedSendUnchecked(Account),
    /// Un compte existe déjà pour cette adresse.
    AlreadyExists,
    /// La découverte n'a pas tout trouvé, ou un serveur doit être confirmé
    /// par l'utilisateur : saisie manuelle pré-remplie.
    NeedsReview(Discovery),
    /// Un serveur est en clair sans consentement explicite.
    PlaintextConsentRequired(ServerEndpoint),
    /// Le test de connexion a échoué (certificat, authentification…).
    ConnectionFailed {
        protocol: Protocol,
        outcome: TestConnectionOutcome,
        /// Conseil du domaine quand les identifiants sont refusés (compte
        /// Google : mot de passe d'application).
        advice: Option<AuthenticationAdvice>,
    },
    /// Paramètres invalides (nom d'utilisateur, mot de passe…).
    Invalid(String),
    /// Le trousseau du système a refusé le secret.
    SecretStoreFailed(String),
    /// Erreur de stockage local.
    StorageFailed(String),
    /// Connexion chez le fournisseur OAuth refusée, abandonnée ou
    /// impossible.
    OAuthFailed(TokenError),
}

impl AddAccountOutcome {
    /// Compte ajouté, que l'envoi ait été vérifié ou non.
    #[must_use]
    pub fn added_account(&self) -> Option<&Account> {
        match self {
            Self::Added(account) | Self::AddedSendUnchecked(account) => Some(account),
            _ => None,
        }
    }
}

/// Cas d'usage : ajouter un compte.
#[derive(Debug)]
pub struct AddAccount<'a, I, S, E, A, R, P, K, D> {
    pub test: &'a TestConnection<I, S, E>,
    pub discover: &'a DiscoverSettings<A, R, P>,
    pub secrets: &'a K,
    pub accounts: &'a D,
}

impl<I, S, E, A, R, P, K, D> AddAccount<'_, I, S, E, A, R, P, K, D>
where
    I: RemoteMailboxConnector,
    S: MailSender,
    E: CertificateExceptionStore,
    A: AutoconfigClient,
    R: SrvResolver,
    P: PortProber,
    K: SecretStore,
    D: AccountRepository,
{
    pub async fn execute(&self, request: AddAccountRequest) -> AddAccountOutcome {
        match self.accounts.find_by_address(&request.address).await {
            Ok(Some(_)) => return AddAccountOutcome::AlreadyExists,
            Ok(None) => {}
            Err(e) => return AddAccountOutcome::StorageFailed(e.to_string()),
        }

        let (imap, smtp) = match request.servers {
            Some(servers) => servers,
            None => match self.discovered(&request.address).await {
                Ok(servers) => servers,
                Err(discovery) => return AddAccountOutcome::NeedsReview(*discovery),
            },
        };

        let name = request
            .display_name
            .unwrap_or_else(|| DisplayName::from_address(&request.address));
        let draft = match AccountDraft::new(name, request.address, imap, smtp, &request.plaintext) {
            Ok(draft) => draft,
            Err(AccountError::PlaintextWithoutConsent(endpoint)) => {
                return AddAccountOutcome::PlaintextConsentRequired(endpoint);
            }
            Err(e) => return AddAccountOutcome::Invalid(e.to_string()),
        };

        for protocol in [Protocol::Imap, Protocol::Smtp] {
            let server = draft.server(protocol);
            let credentials =
                match Credentials::new(server.username.clone(), request.password.clone()) {
                    Ok(c) => c,
                    Err(e) => return AddAccountOutcome::Invalid(e.to_string()),
                };
            let outcome = self
                .test
                .execute(TestConnectionParams {
                    protocol,
                    endpoint: server.endpoint.clone(),
                    credentials,
                    plaintext: request.plaintext.clone(),
                })
                .await;
            if !matches!(outcome, TestConnectionOutcome::Success { .. }) {
                let advice = match &outcome {
                    TestConnectionOutcome::Authentication(error) => {
                        authentication_advice(draft.address(), server.endpoint.host(), error)
                    }
                    _ => None,
                };
                return AddAccountOutcome::ConnectionFailed {
                    protocol,
                    outcome,
                    advice,
                };
            }
        }

        // Insertion d'abord : un doublon (y compris deux ajouts simultanés)
        // échoue ici, sans toucher au trousseau. Les clés dépendent de
        // l'identifiant attribué, donc n'appartiennent qu'à ce compte.
        let account = match self.accounts.insert(&draft).await {
            Ok(account) => account,
            Err(AccountStoreError::Duplicate(_)) => return AddAccountOutcome::AlreadyExists,
            Err(e) => return AddAccountOutcome::StorageFailed(e.to_string()),
        };
        let keys = [
            account.secret_key(Protocol::Imap),
            account.secret_key(Protocol::Smtp),
        ];
        for (i, key) in keys.iter().enumerate() {
            if let Err(e) = self.secrets.set(key, &request.password).await {
                // Retour arrière limité à ce que ce cas d'usage a créé.
                for written in keys.iter().take(i) {
                    let _ = self.secrets.delete(written).await;
                }
                let _ = self.accounts.delete(account.id()).await;
                return AddAccountOutcome::SecretStoreFailed(e.to_string());
            }
        }
        AddAccountOutcome::Added(account)
    }

    /// Serveurs découverts, s'ils sont complets et ne demandent pas de
    /// confirmation.
    async fn discovered(
        &self,
        address: &EmailAddress,
    ) -> Result<(ServerSettings, ServerSettings), Box<Discovery>> {
        let discovery = self.discover.execute(address, |_| {}).await;
        let resolved = |found: Option<&norelly_domain::discovery::DiscoveredServer>| {
            let found = found.filter(|f| !f.requires_confirmation)?;
            let username = found.candidate.username.resolve(address).ok()?;
            Some(ServerSettings {
                protocol: found.candidate.protocol,
                endpoint: found.candidate.endpoint.clone(),
                username,
            })
        };
        match (resolved(discovery.imap()), resolved(discovery.smtp())) {
            (Some(imap), Some(smtp)) => Ok((imap, smtp)),
            _ => Err(Box::new(discovery)),
        }
    }
}

/// Demande d'ajout d'un compte connecté par OAuth.
#[derive(Debug, Clone)]
pub struct AddOAuthAccountRequest {
    pub address: EmailAddress,
    pub display_name: Option<DisplayName>,
    pub provider: OAuthProvider,
}

/// Cas d'usage : ajouter un compte par « Se connecter avec Microsoft ».
///
/// Connexion dans le navigateur du système, test IMAP et SMTP avec le
/// jeton obtenu, enregistrement du compte, puis du jeton de
/// rafraîchissement dans le trousseau (sous la clé IMAP du compte).
#[derive(Debug)]
pub struct AddOAuthAccount<'a, I, S, E, O, K, D> {
    pub test: &'a TestConnection<I, S, E>,
    pub broker: &'a TokenBroker<O, K>,
    pub accounts: &'a D,
}

impl<I, S, E, O, K, D> AddOAuthAccount<'_, I, S, E, O, K, D>
where
    I: RemoteMailboxConnector,
    S: MailSender,
    E: CertificateExceptionStore,
    O: OAuthClient,
    K: SecretStore,
    D: AccountRepository,
{
    pub async fn execute(&self, request: AddOAuthAccountRequest) -> AddAccountOutcome {
        match self.accounts.find_by_address(&request.address).await {
            Ok(Some(_)) => return AddAccountOutcome::AlreadyExists,
            Ok(None) => {}
            Err(e) => return AddAccountOutcome::StorageFailed(e.to_string()),
        }
        let (imap, smtp) = match oauth_servers(request.provider, &request.address) {
            Ok(servers) => servers,
            Err(e) => return AddAccountOutcome::Invalid(e),
        };
        // Un essai précédent a pu échouer après la connexion chez le
        // fournisseur (serveur muet) : elle resservira, une seule fois, sans
        // navigateur.
        let (grant, fresh_since) =
            match self.broker.take_pending(request.provider, &request.address) {
                Some(grant) => (grant, None),
                None => match self
                    .broker
                    .client()
                    .authorize(request.provider, &request.address)
                    .await
                {
                    Ok(grant) => (grant, Some(self.broker.now())),
                    Err(e) => return AddAccountOutcome::OAuthFailed(e),
                },
            };
        if grant.refresh_token.is_none() {
            // Accès hors ligne refusé : le compte cesserait de fonctionner
            // dans l'heure.
            return AddAccountOutcome::OAuthFailed(TokenError::Protocol(
                "jeton de rafraîchissement absent".to_owned(),
            ));
        }
        let name = request
            .display_name
            .unwrap_or_else(|| DisplayName::from_address(&request.address));
        let draft = match AccountDraft::new(
            name,
            request.address,
            imap,
            smtp,
            &PlaintextPolicy::deny_all(),
        ) {
            Ok(draft) => draft.with_oauth(request.provider),
            Err(e) => return AddAccountOutcome::Invalid(e.to_string()),
        };
        let mut send_unchecked = false;
        for protocol in [Protocol::Imap, Protocol::Smtp] {
            let server = draft.server(protocol);
            let credentials = match Credentials::oauth2(
                server.username.clone(),
                grant.access_token.clone(),
                request.provider,
            ) {
                Ok(c) => c,
                Err(e) => return AddAccountOutcome::Invalid(e.to_string()),
            };
            let params = TestConnectionParams {
                protocol,
                endpoint: server.endpoint.clone(),
                credentials,
                plaintext: PlaintextPolicy::deny_all(),
            };
            let mut outcome = self.test.execute(params.clone()).await;
            // Microsoft accepte parfois le jeton sans rattacher la session à
            // la boîte, de façon intermittente : quelques nouveaux essais.
            for _ in 1..OAUTH_IMAP_ATTEMPTS {
                if !mailbox_not_connected(&outcome) {
                    break;
                }
                tokio::time::sleep(OAUTH_IMAP_RETRY_DELAY).await;
                outcome = self.test.execute(params.clone()).await;
            }
            if send_check_can_wait(protocol, &outcome) {
                send_unchecked = true;
                continue;
            }
            if !matches!(outcome, TestConnectionOutcome::Success { .. }) {
                let advice = match &outcome {
                    TestConnectionOutcome::Authentication(AuthError::Rejected { message })
                        if protocol == Protocol::Smtp && is_smtp_oauth_disabled(message) =>
                    {
                        Some(AuthenticationAdvice::MicrosoftSmtpDisabled)
                    }
                    _ if mailbox_not_connected(&outcome) => {
                        Some(AuthenticationAdvice::MicrosoftImapUnavailable)
                    }
                    _ => None,
                };
                // Gardée seulement pour un échec passager : un refus
                // (mauvais compte choisi dans le navigateur…) doit repasser
                // par le navigateur.
                if let Some(obtained_at) = fresh_since
                    && worth_retrying(&outcome)
                {
                    self.broker
                        .keep_pending(request.provider, draft.address(), grant, obtained_at);
                }
                return AddAccountOutcome::ConnectionFailed {
                    protocol,
                    outcome,
                    advice,
                };
            }
        }
        let account = match self.accounts.insert(&draft).await {
            Ok(account) => account,
            Err(AccountStoreError::Duplicate(_)) => return AddAccountOutcome::AlreadyExists,
            Err(e) => return AddAccountOutcome::StorageFailed(e.to_string()),
        };
        let source = OAuthSource {
            provider: request.provider,
            refresh_key: account.secret_key(Protocol::Imap),
        };
        if let Err(e) = self.broker.store_grant(&source, grant).await {
            let _ = self.accounts.delete(account.id()).await;
            return AddAccountOutcome::SecretStoreFailed(e.to_string());
        }
        if send_unchecked {
            AddAccountOutcome::AddedSendUnchecked(account)
        } else {
            AddAccountOutcome::Added(account)
        }
    }
}

/// Essais du test IMAP d'un compte OAuth quand Microsoft accepte le jeton
/// sans rattacher la session à la boîte (défaillance intermittente).
pub const OAUTH_IMAP_ATTEMPTS: usize = 3;

/// Attente entre deux essais du test IMAP d'un compte OAuth.
const OAUTH_IMAP_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

/// Échec du test d'envoi qui n'empêche pas l'ajout d'un compte OAuth : la
/// réception a réussi et le serveur d'envoi, imposé par le fournisseur (ni
/// port ni chiffrement à mal saisir), ne répondait pas.
fn send_check_can_wait(protocol: Protocol, outcome: &TestConnectionOutcome) -> bool {
    protocol == Protocol::Smtp
        && matches!(outcome, TestConnectionOutcome::Network(e) if e.send_check_can_wait())
}

/// Échec qu'un nouvel essai avec la même connexion chez le fournisseur peut
/// surmonter : réseau instable, serveur muet, boîte momentanément non
/// rattachée.
fn worth_retrying(outcome: &TestConnectionOutcome) -> bool {
    matches!(outcome, TestConnectionOutcome::Network(e) if e.is_transient())
        || mailbox_not_connected(outcome)
}

fn mailbox_not_connected(outcome: &TestConnectionOutcome) -> bool {
    matches!(
        outcome,
        TestConnectionOutcome::Authentication(AuthError::Rejected { message })
            if is_mailbox_not_connected(message)
    )
}

/// Serveurs d'un compte OAuth : ceux du fournisseur, en TLS (IMAP 993) et
/// STARTTLS (SMTP 587), avec l'adresse comme identifiant.
pub fn oauth_servers(
    provider: OAuthProvider,
    address: &EmailAddress,
) -> Result<(ServerSettings, ServerSettings), String> {
    let config = provider.config();
    let username = Username::parse(&address.to_string()).map_err(|e| e.to_string())?;
    let server = |protocol, host: &str, port, security| -> Result<ServerSettings, String> {
        let host = Host::parse(host).map_err(|e| e.to_string())?;
        Ok(ServerSettings {
            protocol,
            endpoint: ServerEndpoint::new(host, port, security).map_err(|e| e.to_string())?,
            username: username.clone(),
        })
    };
    Ok((
        server(Protocol::Imap, config.imap_host, 993, Security::Tls)?,
        server(Protocol::Smtp, config.smtp_host, 587, Security::StartTls)?,
    ))
}

/// Échec d'une reconnexion OAuth.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReconnectError {
    #[error("compte introuvable")]
    NotFound,
    #[error("ce compte n'utilise pas de connexion OAuth")]
    NotOAuth,
    #[error(transparent)]
    OAuth(#[from] TokenError),
    #[error("trousseau : {0}")]
    SecretStore(String),
    #[error("stockage : {0}")]
    Storage(String),
}

/// Cas d'usage : se reconnecter chez le fournisseur quand le jeton ne peut
/// plus être rafraîchi (« Reconnexion nécessaire »).
pub async fn reconnect_oauth_account<O: OAuthClient, K: SecretStore, D: AccountRepository>(
    broker: &TokenBroker<O, K>,
    accounts: &D,
    id: AccountId,
) -> Result<Account, ReconnectError> {
    let account = accounts
        .get(id)
        .await
        .map_err(|e| ReconnectError::Storage(e.to_string()))?
        .ok_or(ReconnectError::NotFound)?;
    let provider = account.oauth_provider().ok_or(ReconnectError::NotOAuth)?;
    let grant = broker
        .client()
        .authorize(provider, account.address())
        .await?;
    if grant.refresh_token.is_none() {
        return Err(TokenError::Protocol("jeton de rafraîchissement absent".to_owned()).into());
    }
    let source = OAuthSource {
        provider,
        refresh_key: account.secret_key(Protocol::Imap),
    };
    broker
        .store_grant(&source, grant)
        .await
        .map_err(|e| ReconnectError::SecretStore(e.to_string()))?;
    Ok(account)
}

/// Erreur de suppression de compte.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RemoveAccountError {
    #[error("compte introuvable")]
    NotFound,
    /// Le compte est conservé : rien n'a été supprimé en base.
    #[error("suppression du mot de passe impossible : {0}")]
    SecretStore(String),
    #[error("erreur de stockage : {0}")]
    Storage(String),
    /// Le compte est supprimé, mais des fichiers locaux restent.
    #[error("compte supprimé, mais données locales non effacées : {0}")]
    LocalData(String),
}

/// Cas d'usage : supprimer un compte, ses secrets et ses données locales.
#[derive(Debug)]
pub struct RemoveAccount<'a, K, D, L> {
    pub secrets: &'a K,
    pub accounts: &'a D,
    pub local_data: &'a L,
}

impl<K, D, L> RemoveAccount<'_, K, D, L>
where
    K: SecretStore,
    D: AccountRepository,
    L: AccountDataStore,
{
    /// Ordre : secrets d'abord (un échec laisse le compte intact, l'action
    /// peut être relancée), puis la base, puis les fichiers.
    pub async fn execute(&self, id: AccountId) -> Result<(), RemoveAccountError> {
        let account = self
            .accounts
            .get(id)
            .await
            .map_err(|e| RemoveAccountError::Storage(e.to_string()))?
            .ok_or(RemoveAccountError::NotFound)?;
        for protocol in [Protocol::Imap, Protocol::Smtp] {
            self.secrets
                .delete(&account.secret_key(protocol))
                .await
                .map_err(|e| RemoveAccountError::SecretStore(e.to_string()))?;
        }
        self.accounts
            .delete(id)
            .await
            .map_err(|e| RemoveAccountError::Storage(e.to_string()))?;
        self.local_data
            .purge(id)
            .await
            .map_err(|e| RemoveAccountError::LocalData(e.to_string()))
    }
}

/// Refus d'une exception de certificat.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TrustCertificateError {
    #[error("ce certificat ne peut pas faire l'objet d'une exception (révoqué)")]
    NotExceptable,
    #[error("erreur de stockage : {0}")]
    Storage(String),
}

/// Cas d'usage : l'utilisateur fait confiance, en connaissance de cause, à
/// un certificat refusé (hôte + empreinte présentés à l'écran).
pub async fn trust_certificate<E: CertificateExceptionStore>(
    exceptions: &E,
    rejection: &CertificateRejection,
) -> Result<(), TrustCertificateError> {
    if !rejection.can_be_excepted() {
        return Err(TrustCertificateError::NotExceptable);
    }
    // Empreinte changée : la nouvelle remplace les anciennes (sinon un
    // certificat d'interception accepté une fois resterait valide en plus
    // de l'ancien, ou l'inverse).
    if rejection.fingerprint_changed {
        let previous = exceptions
            .exceptions_for(&rejection.host)
            .await
            .map_err(|e| TrustCertificateError::Storage(e.to_string()))?;
        for old in previous {
            exceptions
                .remove(&old)
                .await
                .map_err(|e| TrustCertificateError::Storage(e.to_string()))?;
        }
    }
    exceptions
        .add(&CertificateException {
            host: rejection.host.clone(),
            fingerprint: rejection.fingerprint,
        })
        .await
        .map_err(|e| TrustCertificateError::Storage(e.to_string()))
}

/// Jeton provisoire d'un compte OAuth, remplacé avant toute connexion par
/// [`crate::oauth::FreshTokens`] (refusé par le serveur sinon).
pub const UNFETCHED_TOKEN: &str = "unfetched";

/// Accès impossible aux paramètres de connexion d'un compte.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConnectRequestError {
    #[error("mot de passe absent du trousseau")]
    MissingSecret,
    #[error("trousseau : {0}")]
    Secrets(String),
    #[error("identifiants invalides : {0}")]
    Credentials(String),
    #[error("stockage : {0}")]
    Storage(String),
}

/// Requête de connexion d'un compte enregistré pour `protocol` : serveur,
/// identifiant et secret du trousseau, exceptions de certificats de
/// l'hôte, consentement au clair du compte.
pub async fn connect_request<K: SecretStore, E: CertificateExceptionStore>(
    account: &Account,
    protocol: Protocol,
    secrets: &K,
    exceptions: &E,
) -> Result<ConnectRequest, ConnectRequestError> {
    let server = account.server(protocol);
    let credentials = match account.oauth_provider() {
        // Compte OAuth : le jeton d'accès est placé avant chaque connexion
        // par [`crate::oauth::FreshTokens`], à partir du jeton de
        // rafraîchissement rangé sous la clé IMAP du compte.
        Some(provider) => Credentials::oauth2(
            server.username.clone(),
            Secret::new(UNFETCHED_TOKEN.to_owned()),
            provider,
        )
        .map(|c| c.with_refresh_key(account.secret_key(Protocol::Imap))),
        None => {
            let secret = secrets
                .get(&account.secret_key(protocol))
                .await
                .map_err(|e| ConnectRequestError::Secrets(e.to_string()))?
                .ok_or(ConnectRequestError::MissingSecret)?;
            Credentials::new(server.username.clone(), secret)
        }
    }
    .map_err(|e| ConnectRequestError::Credentials(e.to_string()))?;
    let certificate_exceptions = exceptions
        .exceptions_for(server.endpoint.host())
        .await
        .map_err(|e| ConnectRequestError::Storage(e.to_string()))?;
    Ok(ConnectRequest {
        endpoint: server.endpoint.clone(),
        credentials,
        certificate_exceptions,
        plaintext: account.plaintext_policy(),
    })
}

#[cfg(test)]
mod tests;
