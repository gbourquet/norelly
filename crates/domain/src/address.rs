//! Adresse électronique (`partie-locale@domaine`), validée.
//!
//! Validation volontairement stricte pour une adresse saisie par
//! l'utilisateur à l'ajout d'un compte : forme `dot-atom` de la RFC 5322
//! (caractères UTF-8 admis dans la partie locale, RFC 6531), domaine validé
//! comme un nom d'hôte DNS. Les parties locales entre guillemets et les
//! domaines en adresse IP littérale sont refusés.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::endpoint::Host;

/// Longueur maximale d'une adresse (RFC 5321, chemin moins les chevrons).
const MAX_ADDRESS_LEN: usize = 254;
/// Longueur maximale de la partie locale (RFC 5321).
const MAX_LOCAL_LEN: usize = 64;

/// Erreur de validation d'une adresse.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EmailAddressError {
    #[error("l'adresse est vide")]
    Empty,
    #[error("l'adresse dépasse {MAX_ADDRESS_LEN} caractères")]
    TooLong,
    #[error("l'adresse doit contenir un seul « @ » entre la partie locale et le domaine")]
    MissingAt,
    #[error("la partie avant « @ » est vide ou dépasse {MAX_LOCAL_LEN} caractères")]
    InvalidLocalLength,
    #[error("la partie avant « @ » contient un caractère non autorisé")]
    InvalidLocalCharacter,
    #[error("la partie avant « @ » contient un point en début, en fin ou doublé")]
    InvalidLocalDot,
    #[error("le domaine de l'adresse est invalide")]
    InvalidDomain,
}

/// Adresse électronique validée. Le domaine est normalisé en minuscules ;
/// la partie locale est conservée telle quelle (elle peut être sensible à
/// la casse, RFC 5321).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct EmailAddress {
    local: String,
    domain: Host,
}

/// Caractères spéciaux admis dans un `atext` (RFC 5322 § 3.2.3).
const ATEXT_SPECIALS: &[u8] = b"!#$%&'*+-/=?^_`{|}~";

fn is_local_char(c: char) -> bool {
    if c.is_ascii() {
        c.is_ascii_alphanumeric() || c == '.' || ATEXT_SPECIALS.contains(&(c as u8))
    } else {
        // UTF-8 (RFC 6531), hors caractères de contrôle, espaces et
        // caractères de mise en forme invisibles.
        !c.is_control() && !c.is_whitespace() && !crate::connection::is_format_char(c)
    }
}

impl EmailAddress {
    /// Valide une adresse. Les espaces de début et de fin sont retirés.
    pub fn parse(raw: &str) -> Result<Self, EmailAddressError> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(EmailAddressError::Empty);
        }
        if raw.len() > MAX_ADDRESS_LEN {
            return Err(EmailAddressError::TooLong);
        }
        let Some((local, domain)) = raw.split_once('@') else {
            return Err(EmailAddressError::MissingAt);
        };
        if domain.contains('@') {
            return Err(EmailAddressError::MissingAt);
        }
        if local.is_empty() || local.len() > MAX_LOCAL_LEN {
            return Err(EmailAddressError::InvalidLocalLength);
        }
        if !local.chars().all(is_local_char) {
            return Err(EmailAddressError::InvalidLocalCharacter);
        }
        if local.starts_with('.') || local.ends_with('.') || local.contains("..") {
            return Err(EmailAddressError::InvalidLocalDot);
        }
        // Le point final d'un FQDN est refusé ici : il n'a pas sa place dans
        // une adresse.
        if domain.ends_with('.') || domain.starts_with('[') {
            return Err(EmailAddressError::InvalidDomain);
        }
        let domain = Host::parse(domain).map_err(|_| EmailAddressError::InvalidDomain)?;
        if domain.is_ip_literal() {
            return Err(EmailAddressError::InvalidDomain);
        }
        Ok(Self {
            local: local.to_owned(),
            domain,
        })
    }

    /// Partie avant `@`.
    #[must_use]
    pub fn local_part(&self) -> &str {
        &self.local
    }

    /// Domaine, normalisé en minuscules.
    #[must_use]
    pub fn domain(&self) -> &Host {
        &self.domain
    }
}

impl fmt::Display for EmailAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.local, self.domain)
    }
}

impl TryFrom<String> for EmailAddress {
    type Error = EmailAddressError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<EmailAddress> for String {
    fn from(address: EmailAddress) -> Self {
        address.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_addresses_are_accepted_and_domain_lowercased() {
        for (raw, expected) in [
            ("alice@example.com", "alice@example.com"),
            ("  Alice.Martin@Example.COM ", "Alice.Martin@example.com"),
            ("a+tag@sub.example.org", "a+tag@sub.example.org"),
            ("o'brien@example.ie", "o'brien@example.ie"),
            ("prénom@exemple.fr", "prénom@exemple.fr"),
            ("x@localhost", "x@localhost"),
            (
                "{weird}!#$%&*=?^_`|~-/@example.com",
                "{weird}!#$%&*=?^_`|~-/@example.com",
            ),
        ] {
            assert_eq!(
                EmailAddress::parse(raw).map(|a| a.to_string()),
                Ok(expected.to_owned()),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn parts_are_exposed() {
        let Ok(a) = EmailAddress::parse("bob@Mail.Example.com") else {
            panic!("adresse valide refusée");
        };
        assert_eq!(a.local_part(), "bob");
        assert_eq!(a.domain().as_str(), "mail.example.com");
    }

    #[test]
    fn structural_errors() {
        use EmailAddressError as E;
        for (raw, err) in [
            ("", E::Empty),
            ("   ", E::Empty),
            ("alice", E::MissingAt),
            ("a@b@c.com", E::MissingAt),
            ("@example.com", E::InvalidLocalLength),
            (".a@example.com", E::InvalidLocalDot),
            ("a.@example.com", E::InvalidLocalDot),
            ("a..b@example.com", E::InvalidLocalDot),
            ("alice@", E::InvalidDomain),
            ("alice@example.com.", E::InvalidDomain),
            ("alice@[127.0.0.1]", E::InvalidDomain),
            ("alice@127.0.0.1", E::InvalidDomain),
            ("alice@exa mple.com", E::InvalidDomain),
            ("alice@-example.com", E::InvalidDomain),
            ("alice@exemple.fr\r\nRCPT", E::InvalidDomain),
        ] {
            assert_eq!(EmailAddress::parse(raw), Err(err), "{raw:?}");
        }
    }

    #[test]
    fn injection_and_invisible_characters_are_rejected() {
        for raw in [
            "a b@example.com",
            "a\r\nb@example.com",
            "a\0@example.com",
            "\"quoted\"@example.com",
            "a<b@example.com",
            "a>b@example.com",
            "a,b@example.com",
            "a;b@example.com",
            "a:b@example.com",
            "a(b)@example.com",
            "a\\b@example.com",
            "a\u{202E}b@example.com",
            "a\u{200B}b@example.com",
            "a\u{00A0}b@example.com",
        ] {
            assert_eq!(
                EmailAddress::parse(raw),
                Err(EmailAddressError::InvalidLocalCharacter),
                "accepté : {raw:?}"
            );
        }
    }

    #[test]
    fn lengths_are_bounded() {
        let local = "a".repeat(65);
        assert_eq!(
            EmailAddress::parse(&format!("{local}@example.com")),
            Err(EmailAddressError::InvalidLocalLength)
        );
        assert!(EmailAddress::parse(&format!("{}@example.com", "a".repeat(64))).is_ok());
        let domain = vec!["a".repeat(50); 5].join(".");
        assert_eq!(
            EmailAddress::parse(&format!("abc@{domain}")),
            Err(EmailAddressError::TooLong)
        );
    }

    #[test]
    fn serde_round_trip_validates() {
        assert_eq!(
            EmailAddress::try_from("X@Y.org".to_owned()).map(String::from),
            Ok("X@y.org".to_owned())
        );
        assert!(EmailAddress::try_from("pas une adresse".to_owned()).is_err());
    }
}
