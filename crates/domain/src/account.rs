//! Compte de messagerie : adresse, nom affiché, serveurs IMAP et SMTP.
//!
//! Le mot de passe n'appartient pas au compte : il vit uniquement dans le
//! trousseau du système, sous les clés données par [`Account::secret_key`]
//! (propres au compte enregistré).

use std::fmt;

use crate::address::EmailAddress;
use crate::credentials::Username;
use crate::endpoint::{Protocol, Security, ServerEndpoint};
use crate::oauth::{OAuthProvider, OAuthSubject};
use crate::ports::SecretKey;
use crate::security::{PlaintextConsent, PlaintextPolicy};
use crate::sender::{imitates_address, is_address_look_alike};

/// Longueur maximale d'un nom affiché, en caractères.
const MAX_DISPLAY_NAME_CHARS: usize = 128;

/// Identifiant local d'un compte, attribué par le dépôt (strictement
/// positif).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AccountId(i64);

/// Identifiant de compte invalide.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("identifiant de compte invalide")]
pub struct AccountIdError;

impl AccountId {
    pub fn new(raw: i64) -> Result<Self, AccountIdError> {
        if raw > 0 {
            Ok(Self(raw))
        } else {
            Err(AccountIdError)
        }
    }

    #[must_use]
    pub fn get(self) -> i64 {
        self.0
    }
}

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Erreur de validation d'un compte.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AccountError {
    #[error("le nom affiché est vide")]
    EmptyDisplayName,
    #[error("le nom affiché dépasse {MAX_DISPLAY_NAME_CHARS} caractères")]
    DisplayNameTooLong,
    #[error("le nom affiché contient un caractère non autorisé")]
    DisplayNameInvalidCharacter,
    #[error("le nom affiché imite une adresse (« @ », « < » ou « > »)")]
    DisplayNameLooksLikeAddress,
    #[error("le serveur {0} doit être un serveur {1}")]
    WrongProtocol(Protocol, Protocol),
    #[error("connexion en clair vers {0} sans consentement explicite")]
    PlaintextWithoutConsent(ServerEndpoint),
    #[error("identifiant stable sans fournisseur qui en délivre un")]
    OAuthSubjectNotSupported,
}

/// Nom affiché à côté de l'adresse (« Alice Martin »). Sans caractère de
/// contrôle ni de mise en forme invisible : il finira dans un en-tête
/// `From`. Saisi par l'utilisateur, il n'imite pas non plus une adresse
/// ([`imitates_address`]) : un tel nom serait écarté à l'envoi, et le
/// message partirait sans nom.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DisplayName(String);

impl DisplayName {
    /// Valide un nom saisi ; les espaces de début et de fin sont retirés.
    pub fn parse(raw: &str) -> Result<Self, AccountError> {
        let name = Self::from_stored(raw)?;
        if imitates_address(name.as_str()) {
            return Err(AccountError::DisplayNameLooksLikeAddress);
        }
        Ok(name)
    }

    /// Nom relu par un adaptateur de stockage, et seulement pour cela (toute
    /// saisie passe par [`Self::parse`]) : mêmes règles que `parse`, sauf
    /// l'imitation d'une adresse. Un compte enregistré avec un tel nom
    /// s'ouvre donc normalement ; ce nom reste écarté à l'envoi, et le
    /// message part avec l'adresse seule.
    pub fn from_stored(raw: &str) -> Result<Self, AccountError> {
        let name = raw.trim();
        if name.is_empty() {
            return Err(AccountError::EmptyDisplayName);
        }
        if name.chars().count() > MAX_DISPLAY_NAME_CHARS {
            return Err(AccountError::DisplayNameTooLong);
        }
        if name
            .chars()
            .any(|c| c.is_control() || crate::connection::is_format_char(c))
        {
            return Err(AccountError::DisplayNameInvalidCharacter);
        }
        Ok(Self(name.to_owned()))
    }

    /// Nom par défaut : la partie locale de l'adresse, sans les caractères
    /// qui imitent une adresse ; s'il n'en reste rien, le domaine, borné à
    /// la longueur d'un nom.
    #[must_use]
    pub fn from_address(address: &EmailAddress) -> Self {
        let local: String = address
            .local_part()
            .chars()
            .filter(|&c| !is_address_look_alike(c))
            .collect();
        Self::parse(&local).unwrap_or_else(|_| {
            // Nom d'hôte ASCII (lettres, chiffres, « - », « . », « _ ») :
            // jamais vide, sans caractère interdit ni sosie d'adresse.
            Self(
                address
                    .domain()
                    .as_str()
                    .chars()
                    .take(MAX_DISPLAY_NAME_CHARS)
                    .collect(),
            )
        })
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DisplayName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Réglages d'un serveur pour un compte.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerSettings {
    pub protocol: Protocol,
    pub endpoint: ServerEndpoint,
    pub username: Username,
}

/// Compte à créer (pas encore d'identifiant).
///
/// Invariant : un serveur en clair n'est accepté qu'avec le consentement
/// explicite de l'utilisateur ; le compte enregistré vaut alors
/// consentement pour ce serveur (voir [`Account::plaintext_policy`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountDraft {
    display_name: DisplayName,
    address: EmailAddress,
    imap: ServerSettings,
    smtp: ServerSettings,
    oauth: Option<OAuthProvider>,
    oauth_subject: Option<OAuthSubject>,
}

impl AccountDraft {
    /// Construit un compte. `plaintext` contient les consentements donnés
    /// par l'utilisateur ; un serveur en clair sans consentement est
    /// refusé.
    pub fn new(
        display_name: DisplayName,
        address: EmailAddress,
        imap: ServerSettings,
        smtp: ServerSettings,
        plaintext: &PlaintextPolicy,
    ) -> Result<Self, AccountError> {
        for (settings, expected) in [(&imap, Protocol::Imap), (&smtp, Protocol::Smtp)] {
            if settings.protocol != expected {
                return Err(AccountError::WrongProtocol(settings.protocol, expected));
            }
            if plaintext.authorize(&settings.endpoint).is_err() {
                return Err(AccountError::PlaintextWithoutConsent(
                    settings.endpoint.clone(),
                ));
            }
        }
        Ok(Self {
            display_name,
            address,
            imap,
            smtp,
            oauth: None,
            oauth_subject: None,
        })
    }

    /// Compte relu depuis le stockage : les serveurs en clair y ont déjà
    /// fait l'objet d'un consentement.
    pub fn restore(
        display_name: DisplayName,
        address: EmailAddress,
        imap: ServerSettings,
        smtp: ServerSettings,
    ) -> Result<Self, AccountError> {
        let consents = plaintext_policy_for([&imap, &smtp]);
        Self::new(display_name, address, imap, smtp, &consents)
    }

    /// Compte connecté par OAuth chez `provider` (jeton au lieu du mot de
    /// passe).
    #[must_use]
    pub fn with_oauth(self, provider: OAuthProvider) -> Self {
        Self {
            oauth: Some(provider),
            ..self
        }
    }

    /// Identifiant stable du compte chez son fournisseur OAuth, qui le
    /// reconnaîtra aux reconnexions. Refusé si le fournisseur du compte n'en
    /// délivre pas ([`OAuthProvider::has_stable_subject`]), ou sans
    /// fournisseur.
    pub fn with_oauth_subject(self, subject: OAuthSubject) -> Result<Self, AccountError> {
        if !self.oauth.is_some_and(OAuthProvider::has_stable_subject) {
            return Err(AccountError::OAuthSubjectNotSupported);
        }
        Ok(Self {
            oauth_subject: Some(subject),
            ..self
        })
    }

    /// Identifiant stable du compte chez son fournisseur OAuth, `None` tant
    /// qu'aucun n'est enregistré (compte à mot de passe ou Google, compte
    /// Microsoft avant sa première reconnexion qui en enregistre un).
    #[must_use]
    pub fn oauth_subject(&self) -> Option<&OAuthSubject> {
        self.oauth_subject.as_ref()
    }

    /// Fournisseur OAuth du compte, `None` pour un compte à mot de passe.
    #[must_use]
    pub fn oauth_provider(&self) -> Option<OAuthProvider> {
        self.oauth
    }

    #[must_use]
    pub fn display_name(&self) -> &DisplayName {
        &self.display_name
    }

    #[must_use]
    pub fn address(&self) -> &EmailAddress {
        &self.address
    }

    #[must_use]
    pub fn imap(&self) -> &ServerSettings {
        &self.imap
    }

    #[must_use]
    pub fn smtp(&self) -> &ServerSettings {
        &self.smtp
    }

    /// Réglages du serveur pour `protocol`.
    #[must_use]
    pub fn server(&self, protocol: Protocol) -> &ServerSettings {
        match protocol {
            Protocol::Imap => &self.imap,
            Protocol::Smtp => &self.smtp,
        }
    }

    /// Consentements au clair déduits du compte (serveurs en clair
    /// enregistrés).
    #[must_use]
    pub fn plaintext_policy(&self) -> PlaintextPolicy {
        plaintext_policy_for([&self.imap, &self.smtp])
    }

    /// Associe un identifiant au compte enregistré.
    #[must_use]
    pub fn with_id(self, id: AccountId) -> Account {
        Account { id, draft: self }
    }
}

fn plaintext_policy_for<'a>(
    servers: impl IntoIterator<Item = &'a ServerSettings>,
) -> PlaintextPolicy {
    servers
        .into_iter()
        .filter(|s| s.endpoint.security() == Security::Plain)
        .fold(PlaintextPolicy::deny_all(), |policy, s| {
            policy.with_consent(PlaintextConsent {
                host: s.endpoint.host().clone(),
                port: s.endpoint.port(),
            })
        })
}

/// Compte enregistré.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    id: AccountId,
    draft: AccountDraft,
}

impl Account {
    #[must_use]
    pub fn id(&self) -> AccountId {
        self.id
    }

    #[must_use]
    pub fn draft(&self) -> &AccountDraft {
        &self.draft
    }

    /// Clé du secret de ce compte pour `protocol` dans le trousseau (propre
    /// au compte, voir [`SecretKey::for_account`]).
    #[must_use]
    pub fn secret_key(&self, protocol: Protocol) -> SecretKey {
        SecretKey::for_account(self.id, protocol)
    }
}

impl std::ops::Deref for Account {
    type Target = AccountDraft;
    fn deref(&self) -> &AccountDraft {
        &self.draft
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::endpoint::Host;

    fn settings(protocol: Protocol, host: &str, port: u16, security: Security) -> ServerSettings {
        let (Ok(h), Ok(u)) = (Host::parse(host), Username::parse("alice@example.com")) else {
            panic!("valeurs valides refusées");
        };
        let Ok(endpoint) = ServerEndpoint::new(h, port, security) else {
            panic!("point de connexion valide refusé");
        };
        ServerSettings {
            protocol,
            endpoint,
            username: u,
        }
    }

    fn address() -> EmailAddress {
        EmailAddress::parse("alice@example.com").unwrap_or_else(|_| panic!("adresse"))
    }

    fn name() -> DisplayName {
        DisplayName::parse("Alice").unwrap_or_else(|_| panic!("nom"))
    }

    #[test]
    fn display_name_validation() {
        assert_eq!(
            DisplayName::parse("  Alice Martin ").map(|n| n.to_string()),
            Ok("Alice Martin".to_owned())
        );
        assert_eq!(DisplayName::parse(" "), Err(AccountError::EmptyDisplayName));
        assert_eq!(
            DisplayName::parse(&"é".repeat(129)),
            Err(AccountError::DisplayNameTooLong)
        );
        for raw in ["a\r\nBcc: x", "a\0", "a\u{202E}b"] {
            assert_eq!(
                DisplayName::parse(raw),
                Err(AccountError::DisplayNameInvalidCharacter)
            );
        }
        assert_eq!(DisplayName::from_address(&address()).as_str(), "alice");
    }

    #[test]
    fn display_name_imitating_an_address_is_refused() {
        for raw in [
            "ceo@banque.fr",
            "Alice <alice@example.com>",
            "Service client \u{FF20} banque",
            "a > b",
        ] {
            assert_eq!(
                DisplayName::parse(raw),
                Err(AccountError::DisplayNameLooksLikeAddress),
                "{raw}"
            );
        }
    }

    #[test]
    fn stored_display_name_imitating_an_address_still_loads() {
        assert_eq!(
            DisplayName::from_stored(" ceo@banque.fr ").map(|n| n.to_string()),
            Ok("ceo@banque.fr".to_owned())
        );
        assert_eq!(
            DisplayName::from_stored(" "),
            Err(AccountError::EmptyDisplayName)
        );
        assert_eq!(
            DisplayName::from_stored(&"é".repeat(129)),
            Err(AccountError::DisplayNameTooLong)
        );
        assert_eq!(
            DisplayName::from_stored("a\r\nBcc: x"),
            Err(AccountError::DisplayNameInvalidCharacter)
        );
    }

    #[test]
    fn default_display_name_never_imitates_an_address() {
        let parse = |raw: &str| EmailAddress::parse(raw).unwrap_or_else(|_| panic!("{raw}"));
        assert_eq!(
            DisplayName::from_address(&parse("a\u{FF20}b@example.com")).as_str(),
            "ab"
        );
        assert_eq!(
            DisplayName::from_address(&parse("\u{FF1C}\u{FF1E}@example.com")).as_str(),
            "example.com"
        );
        // Domaine plus long qu'un nom : tronqué, toujours relisible.
        let long = format!("\u{FF1C}@{}.{}.com", "a".repeat(63), "b".repeat(63));
        let name = DisplayName::from_address(&parse(&long));
        assert_eq!(name.as_str().chars().count(), MAX_DISPLAY_NAME_CHARS);
        assert_eq!(DisplayName::parse(name.as_str()), Ok(name.clone()));
        assert_eq!(DisplayName::from_stored(name.as_str()), Ok(name));
    }

    #[test]
    fn account_id_must_be_positive() {
        assert!(AccountId::new(1).is_ok());
        assert_eq!(AccountId::new(0), Err(AccountIdError));
        assert_eq!(AccountId::new(-3), Err(AccountIdError));
    }

    #[test]
    fn draft_checks_protocols() {
        let imap = settings(Protocol::Imap, "imap.example.com", 993, Security::Tls);
        let smtp = settings(Protocol::Smtp, "smtp.example.com", 465, Security::Tls);
        let deny = PlaintextPolicy::deny_all();
        assert!(AccountDraft::new(name(), address(), imap.clone(), smtp.clone(), &deny).is_ok());
        assert_eq!(
            AccountDraft::new(name(), address(), smtp.clone(), imap.clone(), &deny),
            Err(AccountError::WrongProtocol(Protocol::Smtp, Protocol::Imap))
        );
    }

    #[test]
    fn plaintext_requires_explicit_consent() {
        let imap = settings(Protocol::Imap, "imap.example.com", 143, Security::Plain);
        let smtp = settings(Protocol::Smtp, "smtp.example.com", 587, Security::StartTls);
        assert!(matches!(
            AccountDraft::new(
                name(),
                address(),
                imap.clone(),
                smtp.clone(),
                &PlaintextPolicy::deny_all()
            ),
            Err(AccountError::PlaintextWithoutConsent(_))
        ));
        // Consentement pour un autre port : refusé.
        let wrong_port = PlaintextPolicy::deny_all().with_consent(PlaintextConsent {
            host: imap.endpoint.host().clone(),
            port: 144,
        });
        assert!(
            AccountDraft::new(name(), address(), imap.clone(), smtp.clone(), &wrong_port).is_err()
        );
        let consent = PlaintextPolicy::deny_all().with_consent(PlaintextConsent {
            host: imap.endpoint.host().clone(),
            port: 143,
        });
        let Ok(draft) = AccountDraft::new(name(), address(), imap.clone(), smtp, &consent) else {
            panic!("consentement ignoré");
        };
        // Le compte enregistré porte le consentement, et seulement celui-là.
        let policy = draft.plaintext_policy();
        assert!(policy.authorize(&imap.endpoint).is_ok());
        let other = settings(Protocol::Imap, "other.example.com", 143, Security::Plain);
        assert!(policy.authorize(&other.endpoint).is_err());
    }

    #[test]
    fn restore_accepts_stored_plaintext() {
        let imap = settings(Protocol::Imap, "imap.example.com", 143, Security::Plain);
        let smtp = settings(Protocol::Smtp, "smtp.example.com", 25, Security::Plain);
        assert!(AccountDraft::restore(name(), address(), imap, smtp).is_ok());
    }

    #[test]
    fn secret_keys_differ_per_protocol() {
        let imap = settings(Protocol::Imap, "mail.example.com", 993, Security::Tls);
        let smtp = settings(Protocol::Smtp, "mail.example.com", 465, Security::Tls);
        let Ok(draft) =
            AccountDraft::new(name(), address(), imap, smtp, &PlaintextPolicy::deny_all())
        else {
            panic!("compte valide refusé");
        };
        let Ok(id) = AccountId::new(7) else {
            panic!("id");
        };
        let account = draft.with_id(id);
        assert_eq!(account.id().get(), 7);
        assert_eq!(
            account.secret_key(Protocol::Imap).as_str(),
            "account:7:imap"
        );
        assert_eq!(
            account.secret_key(Protocol::Smtp).as_str(),
            "account:7:smtp"
        );
    }

    #[test]
    fn oauth_subject_is_kept_by_the_draft() {
        let imap = settings(Protocol::Imap, "mail.example.com", 993, Security::Tls);
        let smtp = settings(Protocol::Smtp, "mail.example.com", 465, Security::Tls);
        let Ok(draft) = AccountDraft::restore(name(), address(), imap, smtp) else {
            panic!("compte valide refusé");
        };
        assert_eq!(draft.oauth_subject(), None);
        let Some(oid) = OAuthSubject::parse("oid-a") else {
            panic!("identifiant valide refusé");
        };
        // Compte à mot de passe ou Google : aucun identifiant stable.
        for refused in [
            draft.clone(),
            draft.clone().with_oauth(OAuthProvider::Google),
        ] {
            assert_eq!(
                refused
                    .with_oauth_subject(oid.clone())
                    .map(|d| d.oauth_subject().cloned()),
                Err(AccountError::OAuthSubjectNotSupported)
            );
        }
        let Ok(draft) = draft
            .with_oauth(OAuthProvider::Microsoft)
            .with_oauth_subject(oid.clone())
        else {
            panic!("identifiant refusé sur un compte Microsoft");
        };
        assert_eq!(draft.oauth_subject(), Some(&oid));
        assert_eq!(draft.oauth_provider(), Some(OAuthProvider::Microsoft));
    }
}
