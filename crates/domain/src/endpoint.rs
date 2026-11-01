//! Point de connexion à un serveur : hôte, port et mode de sécurité du
//! transport.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Longueur maximale d'un nom d'hôte DNS (RFC 1035), sans le point final.
const MAX_HOST_LEN: usize = 253;
/// Longueur maximale d'un label DNS.
const MAX_LABEL_LEN: usize = 63;

/// Protocole d'un serveur de messagerie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    /// Réception (IMAP4rev1/IMAP4rev2).
    Imap,
    /// Envoi (soumission SMTP).
    Smtp,
}

impl Protocol {
    /// Nom court, stable, utilisé dans les clés de stockage.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Imap => "imap",
            Self::Smtp => "smtp",
        }
    }
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Imap => "IMAP",
            Self::Smtp => "SMTP",
        })
    }
}

/// Erreur de validation d'un hôte.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HostError {
    #[error("le nom d'hôte est vide")]
    Empty,
    #[error("le nom d'hôte dépasse {MAX_HOST_LEN} caractères")]
    TooLong,
    #[error(
        "le nom d'hôte contient un caractère non autorisé (les noms internationalisés doivent être saisis en punycode)"
    )]
    InvalidCharacter,
    #[error("le nom d'hôte contient un label vide ou trop long")]
    InvalidLabel,
    #[error("adresse IPv6 invalide")]
    InvalidIpv6,
}

/// Nom d'hôte DNS ou adresse IP littérale, validé et normalisé en minuscules.
///
/// Les crochets d'une adresse IPv6 et un point final éventuel sont retirés.
/// Les noms internationalisés doivent être fournis sous forme punycode
/// (`xn--…`) : la conversion relève de l'interface.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Host(String);

impl Host {
    /// Valide et normalise un hôte.
    pub fn parse(raw: &str) -> Result<Self, HostError> {
        if raw.is_empty() {
            return Err(HostError::Empty);
        }
        if let Some(inner) = raw.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            return Self::parse_ipv6(inner);
        }
        if raw.contains(':') {
            return Self::parse_ipv6(raw);
        }
        let name = raw.strip_suffix('.').unwrap_or(raw);
        if name.is_empty() {
            return Err(HostError::Empty);
        }
        if name.len() > MAX_HOST_LEN {
            return Err(HostError::TooLong);
        }
        if !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_'))
        {
            return Err(HostError::InvalidCharacter);
        }
        for label in name.split('.') {
            if label.is_empty()
                || label.len() > MAX_LABEL_LEN
                || label.starts_with('-')
                || label.ends_with('-')
            {
                return Err(HostError::InvalidLabel);
            }
        }
        Ok(Self(name.to_ascii_lowercase()))
    }

    fn parse_ipv6(raw: &str) -> Result<Self, HostError> {
        let valid_chars = raw
            .bytes()
            .all(|b| b.is_ascii_hexdigit() || matches!(b, b':' | b'.'));
        if raw.is_empty()
            || !valid_chars
            || raw.len() > 45
            || !raw.contains("::") && raw.matches(':').count() < 2
        {
            return Err(HostError::InvalidIpv6);
        }
        Ok(Self(raw.to_ascii_lowercase()))
    }

    /// L'hôte normalisé.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Vrai pour une adresse IP littérale (v4 ou v6).
    #[must_use]
    pub fn is_ip_literal(&self) -> bool {
        self.0.contains(':')
            || (self.0.split('.').count() == 4
                && self
                    .0
                    .split('.')
                    .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())))
    }
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.contains(':') {
            write!(f, "[{}]", self.0)
        } else {
            f.write_str(&self.0)
        }
    }
}

impl TryFrom<String> for Host {
    type Error = HostError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<Host> for String {
    fn from(host: Host) -> Self {
        host.0
    }
}

/// Sécurité du transport demandée pour un serveur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    /// TLS dès la connexion (IMAP 993, SMTP 465). Recommandé (RFC 8314).
    Tls,
    /// Connexion en clair puis passage obligatoire en TLS par STARTTLS
    /// (IMAP 143, SMTP 587). Aucun repli en clair.
    StartTls,
    /// Clair. Interdit sans consentement explicite pour ce serveur
    /// (voir [`crate::security::PlaintextPolicy`]). STARTTLS est tout de même
    /// utilisé s'il est annoncé.
    Plain,
}

impl fmt::Display for Security {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Tls => "TLS",
            Self::StartTls => "STARTTLS",
            Self::Plain => "clair",
        })
    }
}

/// Erreur de construction d'un [`ServerEndpoint`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EndpointError {
    #[error("le port 0 n'est pas valide")]
    ZeroPort,
}

/// Adresse d'un serveur et sécurité du transport.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ServerEndpoint {
    host: Host,
    port: u16,
    security: Security,
}

impl ServerEndpoint {
    /// Construit un point de connexion ; le port doit être non nul.
    pub fn new(host: Host, port: u16, security: Security) -> Result<Self, EndpointError> {
        if port == 0 {
            return Err(EndpointError::ZeroPort);
        }
        Ok(Self {
            host,
            port,
            security,
        })
    }

    #[must_use]
    pub fn host(&self) -> &Host {
        &self.host
    }

    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    #[must_use]
    pub fn security(&self) -> Security {
        self.security
    }
}

impl fmt::Display for ServerEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{} ({})", self.host, self.port, self.security)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_is_normalized_to_lowercase_without_trailing_dot() {
        assert_eq!(
            Host::parse("IMAP.Example.COM.").map(|h| h.as_str().to_owned()),
            Ok("imap.example.com".to_owned())
        );
    }

    #[test]
    fn host_rejects_injection_and_invalid_characters() {
        for raw in [
            "imap.example.com\r\nLOGIN",
            "a b",
            "exa\0mple",
            "hôte.fr",
            "a/b",
            "user@host",
        ] {
            assert_eq!(
                Host::parse(raw),
                Err(HostError::InvalidCharacter),
                "accepté : {raw:?}"
            );
        }
    }

    #[test]
    fn host_rejects_bad_labels_and_lengths() {
        assert_eq!(Host::parse(""), Err(HostError::Empty));
        assert_eq!(Host::parse("."), Err(HostError::Empty));
        assert_eq!(Host::parse("a..b"), Err(HostError::InvalidLabel));
        assert_eq!(Host::parse("-a.b"), Err(HostError::InvalidLabel));
        assert_eq!(Host::parse("a-.b"), Err(HostError::InvalidLabel));
        assert_eq!(Host::parse(&"a".repeat(64)), Err(HostError::InvalidLabel));
        let long = vec!["a".repeat(60); 5].join(".");
        assert_eq!(Host::parse(&long), Err(HostError::TooLong));
    }

    #[test]
    fn host_accepts_ip_literals() {
        let v4 = Host::parse("127.0.0.1");
        assert!(v4.as_ref().is_ok_and(Host::is_ip_literal));
        let v6 = Host::parse("[::1]");
        assert_eq!(v6.as_ref().map(Host::as_str), Ok("::1"));
        assert!(v6.as_ref().is_ok_and(Host::is_ip_literal));
        assert_eq!(v6.map(|h| h.to_string()), Ok("[::1]".to_owned()));
        assert_eq!(Host::parse("[::g]"), Err(HostError::InvalidIpv6));
        assert_eq!(Host::parse("a:b"), Err(HostError::InvalidIpv6));
        assert!(Host::parse("imap.example.com").is_ok_and(|h| !h.is_ip_literal()));
    }

    /// `TryFrom<String>` est le chemin emprunté par la désérialisation serde
    /// (`#[serde(try_from = "String")]`) : il doit valider.
    #[test]
    fn host_try_from_string_validates() {
        assert_eq!(
            Host::try_from("Mail.Example.org".to_owned()).map(String::from),
            Ok("mail.example.org".to_owned())
        );
        assert!(Host::try_from("bad host".to_owned()).is_err());
    }

    #[test]
    fn endpoint_rejects_port_zero_and_displays() {
        let Ok(host) = Host::parse("imap.example.com") else {
            panic!("hôte valide refusé");
        };
        assert_eq!(
            ServerEndpoint::new(host.clone(), 0, Security::Tls),
            Err(EndpointError::ZeroPort)
        );
        let endpoint = ServerEndpoint::new(host, 993, Security::Tls);
        assert_eq!(
            endpoint.map(|e| e.to_string()),
            Ok("imap.example.com:993 (TLS)".to_owned())
        );
    }
}
