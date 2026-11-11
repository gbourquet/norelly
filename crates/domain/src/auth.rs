//! Authentification : choix de la méthode selon les capacités et
//! mécanismes SASL (PLAIN, LOGIN, XOAUTH2).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::capabilities::ServerCapabilities;
use crate::credentials::{CredentialKind, Credentials, SecretBytes};
use crate::oauth::xoauth2_initial_response;
use crate::ports::Authenticator;

/// Nombre maximal d'échanges défi/réponse acceptés dans un échange SASL.
pub const MAX_SASL_STEPS: usize = 4;

/// Mécanisme SASL.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AuthMechanism {
    Plain,
    Login,
    XOAuth2,
    /// Mécanisme annoncé mais inconnu de Norelly (nom en majuscules).
    Other(String),
}

impl AuthMechanism {
    /// Nom d'un mécanisme, insensible à la casse.
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        match raw.to_ascii_uppercase().as_str() {
            "PLAIN" => Self::Plain,
            "LOGIN" => Self::Login,
            "XOAUTH2" => Self::XOAuth2,
            other => Self::Other(other.to_owned()),
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Plain => "PLAIN",
            Self::Login => "LOGIN",
            Self::XOAuth2 => "XOAUTH2",
            Self::Other(name) => name,
        }
    }
}

impl fmt::Display for AuthMechanism {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Méthode d'authentification retenue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMethod {
    /// Échange SASL (IMAP `AUTHENTICATE`, SMTP `AUTH`).
    Sasl(AuthMechanism),
    /// Commande IMAP `LOGIN` (interdite si `LOGINDISABLED`).
    ImapLogin,
}

/// Erreur d'authentification.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum AuthError {
    #[error("mécanisme d'authentification non pris en charge : {0}")]
    Unsupported(AuthMechanism),
    #[error(
        "le serveur interdit LOGIN (LOGINDISABLED) et ne propose aucun mécanisme pris en charge"
    )]
    LoginDisabled,
    #[error("aucun mécanisme d'authentification commun (proposés : {})", offered.join(", "))]
    NoSupportedMechanism { offered: Vec<String> },
    #[error("défi d'authentification inattendu à l'étape {step}")]
    UnexpectedChallenge { step: usize },
    #[error("identifiants refusés par le serveur : {message}")]
    Rejected { message: String },
    /// Jeton OAuth destiné à un serveur qui n'est pas celui du fournisseur,
    /// ou à une connexion en clair : il n'est pas envoyé.
    #[error("jeton refusé pour {host} (serveur hors du fournisseur ou connexion en clair)")]
    TokenDestinationRefused { host: String },
    /// Compte OAuth dont le jeton ne peut plus être rafraîchi.
    #[error("reconnexion nécessaire")]
    ReauthRequired,
}

/// Choisit la méthode d'authentification IMAP.
///
/// Ordre : `AUTHENTICATE PLAIN` si annoncé ; sinon `LOGIN` sauf
/// `LOGINDISABLED` ; sinon `AUTHENTICATE LOGIN` si annoncé. XOAUTH2 seul
/// donne [`AuthError::Unsupported`].
/// Avec un jeton OAuth, seul `AUTHENTICATE XOAUTH2` convient.
pub fn select_imap_auth_method(
    caps: &ServerCapabilities,
    kind: CredentialKind,
) -> Result<AuthMethod, AuthError> {
    if kind == CredentialKind::OAuth2 {
        return xoauth2_offered(caps).map(AuthMethod::Sasl);
    }
    if caps.supports_auth(&AuthMechanism::Plain) {
        return Ok(AuthMethod::Sasl(AuthMechanism::Plain));
    }
    if !caps.login_disabled() {
        return Ok(AuthMethod::ImapLogin);
    }
    if caps.supports_auth(&AuthMechanism::Login) {
        return Ok(AuthMethod::Sasl(AuthMechanism::Login));
    }
    if caps.supports_auth(&AuthMechanism::XOAuth2) {
        return Err(AuthError::Unsupported(AuthMechanism::XOAuth2));
    }
    Err(AuthError::LoginDisabled)
}

/// Choisit le mécanisme SMTP : PLAIN, puis LOGIN.
/// Avec un jeton OAuth, seul XOAUTH2 convient.
pub fn select_smtp_auth_mechanism(
    caps: &ServerCapabilities,
    kind: CredentialKind,
) -> Result<AuthMechanism, AuthError> {
    if kind == CredentialKind::OAuth2 {
        return xoauth2_offered(caps);
    }
    for mechanism in [AuthMechanism::Plain, AuthMechanism::Login] {
        if caps.supports_auth(&mechanism) {
            return Ok(mechanism);
        }
    }
    if caps.supports_auth(&AuthMechanism::XOAuth2) {
        return Err(AuthError::Unsupported(AuthMechanism::XOAuth2));
    }
    Err(AuthError::NoSupportedMechanism {
        offered: caps.auth_mechanisms().map(|m| m.to_string()).collect(),
    })
}

fn xoauth2_offered(caps: &ServerCapabilities) -> Result<AuthMechanism, AuthError> {
    if caps.supports_auth(&AuthMechanism::XOAuth2) {
        Ok(AuthMechanism::XOAuth2)
    } else {
        Err(AuthError::NoSupportedMechanism {
            offered: caps.auth_mechanisms().map(|m| m.to_string()).collect(),
        })
    }
}

/// SASL PLAIN (RFC 4616) : une seule réponse `\0utilisateur\0secret`.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlainAuthenticator;

impl Authenticator for PlainAuthenticator {
    fn mechanism(&self) -> AuthMechanism {
        AuthMechanism::Plain
    }

    fn supports_initial_response(&self) -> bool {
        true
    }

    fn respond(
        &self,
        credentials: &Credentials,
        step: usize,
        _challenge: &[u8],
    ) -> Result<SecretBytes, AuthError> {
        if step != 0 {
            return Err(AuthError::UnexpectedChallenge { step });
        }
        let user = credentials.username().as_str().as_bytes();
        let secret = credentials.secret().expose().as_bytes();
        let mut out = Vec::with_capacity(user.len() + secret.len() + 2);
        out.push(0);
        out.extend_from_slice(user);
        out.push(0);
        out.extend_from_slice(secret);
        Ok(SecretBytes::new(out))
    }
}

/// SASL LOGIN (historique) : utilisateur au premier défi, secret au second.
#[derive(Debug, Clone, Copy, Default)]
pub struct LoginAuthenticator;

impl Authenticator for LoginAuthenticator {
    fn mechanism(&self) -> AuthMechanism {
        AuthMechanism::Login
    }

    fn supports_initial_response(&self) -> bool {
        false
    }

    fn respond(
        &self,
        credentials: &Credentials,
        step: usize,
        _challenge: &[u8],
    ) -> Result<SecretBytes, AuthError> {
        match step {
            0 => Ok(SecretBytes::new(
                credentials.username().as_str().as_bytes().to_vec(),
            )),
            1 => Ok(SecretBytes::new(
                credentials.secret().expose().as_bytes().to_vec(),
            )),
            _ => Err(AuthError::UnexpectedChallenge { step }),
        }
    }
}

/// XOAUTH2 (comptes Microsoft) : réponse initiale avec le jeton d'accès ;
/// un défi ensuite porte l'erreur du serveur (JSON), à laquelle on répond
/// par une réponse vide pour obtenir le refus final.
#[derive(Debug, Clone, Copy, Default)]
pub struct XOAuth2Authenticator;

impl Authenticator for XOAuth2Authenticator {
    fn mechanism(&self) -> AuthMechanism {
        AuthMechanism::XOAuth2
    }

    fn supports_initial_response(&self) -> bool {
        true
    }

    fn respond(
        &self,
        credentials: &Credentials,
        step: usize,
        _challenge: &[u8],
    ) -> Result<SecretBytes, AuthError> {
        if credentials.kind() != CredentialKind::OAuth2 {
            return Err(AuthError::Unsupported(AuthMechanism::XOAuth2));
        }
        match step {
            0 => Ok(xoauth2_initial_response(
                credentials.username().as_str(),
                credentials.secret(),
            )),
            1 => Ok(SecretBytes::default()),
            _ => Err(AuthError::UnexpectedChallenge { step }),
        }
    }
}

/// Authentificateur correspondant à un mécanisme.
pub fn authenticator_for(mechanism: &AuthMechanism) -> Result<Box<dyn Authenticator>, AuthError> {
    match mechanism {
        AuthMechanism::Plain => Ok(Box::new(PlainAuthenticator)),
        AuthMechanism::Login => Ok(Box::new(LoginAuthenticator)),
        AuthMechanism::XOAuth2 => Ok(Box::new(XOAuth2Authenticator)),
        AuthMechanism::Other(_) => Err(AuthError::Unsupported(mechanism.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::{Secret, Username};

    fn creds() -> Credentials {
        let (Ok(u), s) = (Username::parse("alice"), Secret::new("pw".to_owned())) else {
            panic!("utilisateur valide refusé");
        };
        let Ok(c) = Credentials::new(u, s) else {
            panic!("identifiants valides refusés");
        };
        c
    }

    fn select_imap_auth_method_pw(caps: &ServerCapabilities) -> Result<AuthMethod, AuthError> {
        select_imap_auth_method(caps, CredentialKind::Password)
    }

    fn select_smtp_auth_mechanism_pw(
        caps: &ServerCapabilities,
    ) -> Result<AuthMechanism, AuthError> {
        select_smtp_auth_mechanism(caps, CredentialKind::Password)
    }

    fn oauth_creds() -> Credentials {
        let (Ok(u), t) = (
            Username::parse("alice@outlook.com"),
            Secret::new("tok".to_owned()),
        ) else {
            panic!("utilisateur valide refusé");
        };
        let Ok(c) = Credentials::oauth2(u, t, crate::oauth::OAuthProvider::Microsoft) else {
            panic!("jeton valide refusé");
        };
        c
    }

    fn imap(tokens: &[&str]) -> ServerCapabilities {
        ServerCapabilities::from_imap(tokens)
    }

    #[test]
    fn imap_prefers_authenticate_plain() {
        assert_eq!(
            select_imap_auth_method_pw(&imap(&["IMAP4rev1", "AUTH=PLAIN", "AUTH=LOGIN"])),
            Ok(AuthMethod::Sasl(AuthMechanism::Plain))
        );
    }

    #[test]
    fn imap_falls_back_to_login_command() {
        assert_eq!(
            select_imap_auth_method_pw(&imap(&["IMAP4rev1"])),
            Ok(AuthMethod::ImapLogin)
        );
    }

    #[test]
    fn imap_respects_logindisabled() {
        assert_eq!(
            select_imap_auth_method_pw(&imap(&["IMAP4rev1", "LOGINDISABLED"])),
            Err(AuthError::LoginDisabled)
        );
        assert_eq!(
            select_imap_auth_method_pw(&imap(&["LOGINDISABLED", "AUTH=LOGIN"])),
            Ok(AuthMethod::Sasl(AuthMechanism::Login))
        );
        assert_eq!(
            select_imap_auth_method_pw(&imap(&["LOGINDISABLED", "AUTH=XOAUTH2"])),
            Err(AuthError::Unsupported(AuthMechanism::XOAuth2))
        );
    }

    #[test]
    fn smtp_selection() {
        let smtp = |l: &[&str]| ServerCapabilities::from_smtp_ehlo(l);
        assert_eq!(
            select_smtp_auth_mechanism_pw(&smtp(&["AUTH LOGIN PLAIN"])),
            Ok(AuthMechanism::Plain)
        );
        assert_eq!(
            select_smtp_auth_mechanism_pw(&smtp(&["AUTH LOGIN"])),
            Ok(AuthMechanism::Login)
        );
        assert_eq!(
            select_smtp_auth_mechanism_pw(&smtp(&["AUTH XOAUTH2"])),
            Err(AuthError::Unsupported(AuthMechanism::XOAuth2))
        );
        assert_eq!(
            select_smtp_auth_mechanism_pw(&smtp(&["AUTH CRAM-MD5"])),
            Err(AuthError::NoSupportedMechanism {
                offered: vec!["CRAM-MD5".to_owned()]
            })
        );
        assert_eq!(
            select_smtp_auth_mechanism_pw(&smtp(&["PIPELINING"])),
            Err(AuthError::NoSupportedMechanism { offered: vec![] })
        );
    }

    #[test]
    fn plain_builds_single_response() {
        let r = PlainAuthenticator.respond(&creds(), 0, b"");
        assert_eq!(r.as_ref().map(SecretBytes::expose), Ok(&b"\0alice\0pw"[..]));
        assert_eq!(
            PlainAuthenticator.respond(&creds(), 1, b""),
            Err(AuthError::UnexpectedChallenge { step: 1 })
        );
        assert!(PlainAuthenticator.supports_initial_response());
    }

    #[test]
    fn login_answers_two_challenges() {
        let a = LoginAuthenticator;
        assert_eq!(
            a.respond(&creds(), 0, b"Username:")
                .map(|s| s.expose().to_vec()),
            Ok(b"alice".to_vec())
        );
        assert_eq!(
            a.respond(&creds(), 1, b"Password:")
                .map(|s| s.expose().to_vec()),
            Ok(b"pw".to_vec())
        );
        assert_eq!(
            a.respond(&creds(), 2, b""),
            Err(AuthError::UnexpectedChallenge { step: 2 })
        );
        assert!(!a.supports_initial_response());
    }

    #[test]
    fn oauth_credentials_require_xoauth2() {
        let kind = oauth_creds().kind();
        assert_eq!(kind, CredentialKind::OAuth2);
        assert_eq!(
            select_imap_auth_method(&imap(&["IMAP4rev1", "AUTH=PLAIN", "AUTH=XOAUTH2"]), kind),
            Ok(AuthMethod::Sasl(AuthMechanism::XOAuth2))
        );
        assert_eq!(
            select_imap_auth_method(&imap(&["IMAP4rev1", "AUTH=PLAIN"]), kind),
            Err(AuthError::NoSupportedMechanism {
                offered: vec!["PLAIN".to_owned()]
            })
        );
        let smtp = |l: &[&str]| ServerCapabilities::from_smtp_ehlo(l);
        assert_eq!(
            select_smtp_auth_mechanism(&smtp(&["AUTH LOGIN XOAUTH2"]), kind),
            Ok(AuthMechanism::XOAuth2)
        );
        assert!(select_smtp_auth_mechanism(&smtp(&["AUTH LOGIN"]), kind).is_err());
    }

    #[test]
    fn xoauth2_answers_with_bearer_then_empty() {
        let a = XOAuth2Authenticator;
        assert_eq!(
            a.respond(&oauth_creds(), 0, b"")
                .map(|s| s.expose().to_vec()),
            Ok(b"user=alice@outlook.com\x01auth=Bearer tok\x01\x01".to_vec())
        );
        assert_eq!(
            a.respond(&oauth_creds(), 1, b"{\"status\":\"401\"}")
                .map(|s| s.expose().to_vec()),
            Ok(Vec::new())
        );
        assert_eq!(
            a.respond(&oauth_creds(), 2, b""),
            Err(AuthError::UnexpectedChallenge { step: 2 })
        );
        assert_eq!(
            a.respond(&creds(), 0, b""),
            Err(AuthError::Unsupported(AuthMechanism::XOAuth2)),
            "un mot de passe n'est jamais envoyé comme jeton"
        );
        assert!(
            authenticator_for(&AuthMechanism::XOAuth2)
                .is_ok_and(|a| a.mechanism() == AuthMechanism::XOAuth2)
        );
        assert!(authenticator_for(&AuthMechanism::Other("CRAM-MD5".into())).is_err());
    }

    #[test]
    fn mechanism_parse_roundtrip() {
        for m in ["plain", "LOGIN", "XOAuth2", "SCRAM-SHA-256"] {
            assert_eq!(AuthMechanism::parse(m).as_str(), m.to_ascii_uppercase());
        }
    }
}
