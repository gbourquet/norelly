//! Cas d'usage des comptes : `AddAccount`, `RemoveAccount` et
//! `TrustCertificate`.
//!
//! `AddAccount` : découverte (si les serveurs ne sont pas fournis), test
//! des connexions IMAP et SMTP, enregistrement du mot de passe dans le
//! trousseau, puis persistance du compte. En cas d'échec après l'écriture
//! dans le trousseau, les secrets écrits sont retirés.

use norelly_domain::account::{
    Account, AccountDraft, AccountError, AccountId, DisplayName, ServerSettings,
};
use norelly_domain::address::EmailAddress;
use norelly_domain::credentials::{Credentials, Secret};
use norelly_domain::discovery::Discovery;
use norelly_domain::endpoint::{Protocol, ServerEndpoint};
use norelly_domain::ports::{
    AccountDataStore, AccountRepository, AccountStoreError, AutoconfigClient,
    CertificateExceptionStore, MailSender, PortProber, RemoteMailboxConnector, SecretStore,
    SrvResolver,
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
                return AddAccountOutcome::ConnectionFailed { protocol, outcome };
            }
        }

        let keys = [
            draft.secret_key(Protocol::Imap),
            draft.secret_key(Protocol::Smtp),
        ];
        for (i, key) in keys.iter().enumerate() {
            if let Err(e) = self.secrets.set(key, &request.password).await {
                for written in keys.iter().take(i) {
                    let _ = self.secrets.delete(written).await;
                }
                return AddAccountOutcome::SecretStoreFailed(e.to_string());
            }
        }

        match self.accounts.insert(&draft).await {
            Ok(account) => AddAccountOutcome::Added(account),
            Err(e) => {
                for key in &keys {
                    let _ = self.secrets.delete(key).await;
                }
                match e {
                    AccountStoreError::Duplicate(_) => AddAccountOutcome::AlreadyExists,
                    other => AddAccountOutcome::StorageFailed(other.to_string()),
                }
            }
        }
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
    exceptions
        .add(&CertificateException {
            host: rejection.host.clone(),
            fingerprint: rejection.fingerprint,
        })
        .await
        .map_err(|e| TrustCertificateError::Storage(e.to_string()))
}

#[cfg(test)]
mod tests;
