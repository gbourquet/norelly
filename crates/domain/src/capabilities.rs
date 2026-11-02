//! Capacités annoncées par un serveur (IMAP `CAPABILITY`, SMTP `EHLO`).
//!
//! Les capacités lues avant TLS sont hostiles par nature : elles ne servent
//! qu'à décider de STARTTLS, puis sont relues une fois le canal chiffré.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::auth::AuthMechanism;

/// Nombre maximal de capacités retenues (borne contre un serveur hostile).
const MAX_TOKENS: usize = 256;
/// Longueur maximale d'une capacité retenue.
const MAX_TOKEN_LEN: usize = 128;

/// Ensemble de capacités normalisées en majuscules.
///
/// Les mécanismes d'authentification sont représentés sous la forme
/// `AUTH=<MÉCANISME>` quel que soit le protocole.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerCapabilities {
    tokens: BTreeSet<String>,
}

impl ServerCapabilities {
    /// Capacités IMAP, telles que renvoyées par `CAPABILITY` (par exemple
    /// `IMAP4rev1`, `STARTTLS`, `AUTH=PLAIN`, `LOGINDISABLED`).
    pub fn from_imap<I, S>(tokens: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut caps = Self::default();
        for token in tokens {
            caps.insert(token.as_ref());
        }
        caps
    }

    /// Capacités SMTP, à partir des lignes de la réponse `EHLO` hors
    /// première ligne (par exemple `STARTTLS`, `AUTH PLAIN LOGIN`,
    /// `SIZE 35882577`). Seul le mot-clé est retenu, sauf pour `AUTH` dont
    /// chaque mécanisme devient `AUTH=<MÉCANISME>`.
    pub fn from_smtp_ehlo<I, S>(lines: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut caps = Self::default();
        for line in lines {
            let mut words = line.as_ref().split_ascii_whitespace();
            let Some(keyword) = words.next() else {
                continue;
            };
            if keyword.eq_ignore_ascii_case("AUTH") {
                for mechanism in words {
                    caps.insert(&format!("AUTH={mechanism}"));
                }
            } else {
                caps.insert(keyword);
            }
        }
        caps
    }

    fn insert(&mut self, token: &str) {
        let token = token.trim();
        let valid = !token.is_empty()
            && token.len() <= MAX_TOKEN_LEN
            && token.bytes().all(|b| b.is_ascii_graphic());
        if valid && self.tokens.len() < MAX_TOKENS {
            self.tokens.insert(token.to_ascii_uppercase());
        }
    }

    /// Vrai si la capacité est annoncée (comparaison insensible à la casse).
    #[must_use]
    pub fn has(&self, token: &str) -> bool {
        self.tokens.contains(&token.to_ascii_uppercase())
    }

    /// Mêmes capacités sans celles de `hidden` (capacités masquées :
    /// serveur à l'implémentation défaillante, tests des replis).
    #[must_use]
    pub fn without(&self, hidden: &[String]) -> Self {
        Self {
            tokens: self
                .tokens
                .iter()
                .filter(|t| !hidden.iter().any(|h| h.eq_ignore_ascii_case(t)))
                .cloned()
                .collect(),
        }
    }

    /// STARTTLS annoncé.
    #[must_use]
    pub fn starttls(&self) -> bool {
        self.has("STARTTLS")
    }

    /// IMAP `LOGINDISABLED` : la commande LOGIN est interdite.
    #[must_use]
    pub fn login_disabled(&self) -> bool {
        self.has("LOGINDISABLED")
    }

    /// IMAP `SASL-IR` : réponse initiale admise dans `AUTHENTICATE`.
    #[must_use]
    pub fn sasl_initial_response(&self) -> bool {
        self.has("SASL-IR")
    }

    /// Mécanismes d'authentification annoncés.
    pub fn auth_mechanisms(&self) -> impl Iterator<Item = AuthMechanism> + '_ {
        self.tokens
            .iter()
            .filter_map(|t| t.strip_prefix("AUTH="))
            .map(AuthMechanism::parse)
    }

    /// Vrai si le mécanisme est annoncé.
    #[must_use]
    pub fn supports_auth(&self, mechanism: &AuthMechanism) -> bool {
        self.has(&format!("AUTH={}", mechanism.as_str()))
    }

    /// Capacités de `required` absentes de cet ensemble.
    #[must_use]
    pub fn missing<'a>(&self, required: &[&'a str]) -> Vec<&'a str> {
        required.iter().copied().filter(|r| !self.has(r)).collect()
    }

    /// Toutes les capacités, triées.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.tokens.iter().map(String::as_str)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imap_tokens_are_case_insensitive() {
        let caps = ServerCapabilities::from_imap(["imap4rev1", "StartTLS", "AUTH=plain"]);
        assert!(caps.starttls());
        assert!(caps.has("IMAP4REV1"));
        assert!(caps.supports_auth(&AuthMechanism::Plain));
        assert!(!caps.login_disabled());
        assert_eq!(
            caps.auth_mechanisms().collect::<Vec<_>>(),
            [AuthMechanism::Plain]
        );
    }

    #[test]
    fn smtp_ehlo_lines_are_parsed() {
        let caps = ServerCapabilities::from_smtp_ehlo([
            "PIPELINING",
            "SIZE 35882577",
            "AUTH PLAIN LOGIN XOAUTH2",
            "STARTTLS",
            "",
        ]);
        assert!(caps.starttls());
        assert!(caps.has("SIZE"));
        assert!(caps.supports_auth(&AuthMechanism::Plain));
        assert!(caps.supports_auth(&AuthMechanism::Login));
        assert!(caps.supports_auth(&AuthMechanism::XOAuth2));
        assert_eq!(caps.iter().count(), 6);
    }

    #[test]
    fn hostile_tokens_are_bounded() {
        let many = (0..10_000).map(|i| format!("X{i}"));
        assert_eq!(
            ServerCapabilities::from_imap(many).iter().count(),
            MAX_TOKENS
        );
        let caps = ServerCapabilities::from_imap(["A\r\nB", &"Y".repeat(1000), "é"]);
        assert!(caps.is_empty());
    }

    #[test]
    fn missing_lists_absent_capabilities() {
        let caps = ServerCapabilities::from_imap(["IDLE", "MOVE"]);
        assert_eq!(caps.missing(&["IDLE", "QRESYNC", "move"]), ["QRESYNC"]);
    }
}
