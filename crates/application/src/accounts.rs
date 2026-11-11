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

use crate::discover_settings::DiscoverSettings;
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
