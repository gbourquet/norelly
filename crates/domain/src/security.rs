//! Politiques de sécurité du transport : confiance dans les certificats
//! (avec exceptions par hôte et empreinte), consentement au clair et
//! décision STARTTLS.
//!
//! Ces règles sont pures : l'adaptateur TLS fournit le verdict de la
//! vérification de la plateforme et l'empreinte du certificat feuille, le
//! domaine décide.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::endpoint::{Host, Security, ServerEndpoint};

/// Empreinte SHA-256 du certificat feuille (DER).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CertificateFingerprint([u8; 32]);

/// Erreur de lecture d'une empreinte hexadécimale.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("empreinte SHA-256 invalide (64 chiffres hexadécimaux attendus)")]
pub struct FingerprintError;

impl CertificateFingerprint {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Lit une empreinte hexadécimale, avec ou sans séparateurs `:`.
    pub fn parse_hex(raw: &str) -> Result<Self, FingerprintError> {
        let digits: Vec<u8> = raw.bytes().filter(|&b| b != b':').collect();
        if digits.len() != 64 {
            return Err(FingerprintError);
        }
        let mut out = [0u8; 32];
        for (i, pair) in digits.as_chunks::<2>().0.iter().enumerate() {
            let [hi, lo] = *pair;
            let hi = hex_value(hi).ok_or(FingerprintError)?;
            let lo = hex_value(lo).ok_or(FingerprintError)?;
            out[i] = (hi << 4) | lo;
        }
        Ok(Self(out))
    }
}

fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Forme usuelle : `AB:CD:…` (majuscules, séparateurs `:`).
impl fmt::Display for CertificateFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, b) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(":")?;
            }
            write!(f, "{b:02X}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for CertificateFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CertificateFingerprint({self})")
    }
}

impl TryFrom<String> for CertificateFingerprint {
    type Error = FingerprintError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse_hex(&value)
    }
}

impl From<CertificateFingerprint> for String {
    fn from(fp: CertificateFingerprint) -> Self {
        fp.to_string()
    }
}

/// Raison du rejet d'un certificat par la vérification de la plateforme.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CertificateIssue {
    /// Autorité inconnue : certificat autosigné ou CA non reconnue.
    UnknownIssuer,
    Expired,
    NotYetValid,
    /// Le certificat ne couvre pas le nom d'hôte demandé.
    HostnameMismatch,
    /// Révoqué : aucune exception ne peut l'accepter.
    Revoked,
    /// Signature invalide ou algorithme non pris en charge.
    InvalidSignature,
    /// Certificat illisible.
    Malformed,
    /// Autre raison, décrite par l'adaptateur.
    Other(String),
}

impl fmt::Display for CertificateIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownIssuer => {
                f.write_str("autorité de certification inconnue (certificat autosigné ?)")
            }
            Self::Expired => f.write_str("certificat expiré"),
            Self::NotYetValid => f.write_str("certificat pas encore valide"),
            Self::HostnameMismatch => {
                f.write_str("le certificat ne correspond pas au nom du serveur")
            }
            Self::Revoked => f.write_str("certificat révoqué"),
            Self::InvalidSignature => f.write_str("signature du certificat invalide"),
            Self::Malformed => f.write_str("certificat illisible"),
            Self::Other(detail) => write!(f, "certificat refusé : {detail}"),
        }
    }
}

/// Exception mémorisée : l'utilisateur fait confiance à **ce** certificat
/// (empreinte) pour **cet** hôte, et à rien d'autre.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CertificateException {
    pub host: Host,
    pub fingerprint: CertificateFingerprint,
}

/// Rejet détaillé d'un certificat, de quoi proposer une exception à
/// l'utilisateur en connaissance de cause.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificateRejection {
    pub host: Host,
    pub fingerprint: CertificateFingerprint,
    pub issue: CertificateIssue,
    /// Une exception existait pour cet hôte avec une **autre** empreinte :
    /// le certificat a changé (renouvellement, ou attaque de l'homme du
    /// milieu). À signaler clairement.
    pub fingerprint_changed: bool,
}

impl CertificateRejection {
    /// Vrai si l'utilisateur peut accepter ce certificat par une exception.
    #[must_use]
    pub fn can_be_excepted(&self) -> bool {
        self.issue != CertificateIssue::Revoked
    }
}

/// Décision de confiance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustDecision {
    /// Certificat valide pour la plateforme.
    Trusted,
    /// Certificat invalide pour la plateforme, mais exception mémorisée
    /// pour le même hôte et la même empreinte.
    TrustedByException,
    Rejected(CertificateRejection),
}

/// Politique de confiance TLS.
#[derive(Debug, Clone, Copy, Default)]
pub struct TlsTrustPolicy;

impl TlsTrustPolicy {
    /// Décide de la confiance accordée au certificat présenté par `host`.
    ///
    /// - Certificat valide pour la plateforme : accepté.
    /// - Sinon, accepté seulement s'il existe une exception pour cet hôte
    ///   **et** cette empreinte, et qu'il n'est pas révoqué.
    /// - Sinon rejeté, en signalant un éventuel changement d'empreinte.
    #[must_use]
    pub fn evaluate(
        host: &Host,
        fingerprint: &CertificateFingerprint,
        platform_verdict: Result<(), CertificateIssue>,
        exceptions: &[CertificateException],
    ) -> TrustDecision {
        let issue = match platform_verdict {
            Ok(()) => return TrustDecision::Trusted,
            Err(issue) => issue,
        };
        let for_host = || exceptions.iter().filter(|e| &e.host == host);
        if issue != CertificateIssue::Revoked && for_host().any(|e| &e.fingerprint == fingerprint) {
            return TrustDecision::TrustedByException;
        }
        TrustDecision::Rejected(CertificateRejection {
            host: host.clone(),
            fingerprint: *fingerprint,
            fingerprint_changed: for_host().any(|e| &e.fingerprint != fingerprint),
            issue,
        })
    }
}

/// Consentement explicite à une connexion en clair vers un serveur précis.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PlaintextConsent {
    pub host: Host,
    pub port: u16,
}

/// Refus d'une connexion en clair.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[error(
    "connexion en clair refusée vers {host}:{port} : aucun consentement explicite pour ce serveur"
)]
pub struct PlaintextRefused {
    pub host: Host,
    pub port: u16,
}

/// Politique de connexion en clair : refus par défaut, consentement par
/// serveur (hôte et port).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlaintextPolicy {
    consents: BTreeSet<PlaintextConsent>,
}

impl PlaintextPolicy {
    /// Aucun consentement : tout clair est refusé.
    #[must_use]
    pub fn deny_all() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_consent(mut self, consent: PlaintextConsent) -> Self {
        self.consents.insert(consent);
        self
    }

    /// Autorise le point de connexion : toujours pour TLS et STARTTLS, sur
    /// consentement pour le clair.
    pub fn authorize(&self, endpoint: &ServerEndpoint) -> Result<(), PlaintextRefused> {
        if endpoint.security() != Security::Plain {
            return Ok(());
        }
        let consent = PlaintextConsent {
            host: endpoint.host().clone(),
            port: endpoint.port(),
        };
        if self.consents.contains(&consent) {
            Ok(())
        } else {
            Err(PlaintextRefused {
                host: consent.host,
                port: consent.port,
            })
        }
    }
}

/// Décision prise sur un canal en clair, après lecture des capacités.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartTlsDecision {
    /// Le canal est déjà chiffré (TLS implicite) : rien à faire.
    AlreadyEncrypted,
    /// Passer en TLS par STARTTLS avant toute authentification.
    Upgrade,
    /// Rester en clair (mode clair consenti, STARTTLS non annoncé).
    StayPlain,
}

/// STARTTLS exigé mais non annoncé : on refuse, sans repli en clair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("le serveur n'annonce pas STARTTLS : connexion interrompue (aucun repli en clair)")]
pub struct StartTlsUnavailable;

/// Décide de STARTTLS : obligatoire en mode STARTTLS (erreur s'il n'est pas
/// annoncé) et utilisé dès qu'il est annoncé, même en mode clair consenti.
pub fn decide_starttls(
    security: Security,
    starttls_offered: bool,
) -> Result<StartTlsDecision, StartTlsUnavailable> {
    match (security, starttls_offered) {
        (Security::Tls, _) => Ok(StartTlsDecision::AlreadyEncrypted),
        (Security::StartTls | Security::Plain, true) => Ok(StartTlsDecision::Upgrade),
        (Security::StartTls, false) => Err(StartTlsUnavailable),
        (Security::Plain, false) => Ok(StartTlsDecision::StayPlain),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(name: &str) -> Host {
        let Ok(h) = Host::parse(name) else {
            panic!("hôte valide refusé : {name}");
        };
        h
    }

    const FP_A: CertificateFingerprint = CertificateFingerprint::from_bytes([0xAA; 32]);
    const FP_B: CertificateFingerprint = CertificateFingerprint::from_bytes([0xBB; 32]);

    fn exception(h: &str, fp: CertificateFingerprint) -> CertificateException {
        CertificateException {
            host: host(h),
            fingerprint: fp,
        }
    }

    const ALL_ISSUES: [CertificateIssue; 7] = [
        CertificateIssue::UnknownIssuer,
        CertificateIssue::Expired,
        CertificateIssue::NotYetValid,
        CertificateIssue::HostnameMismatch,
        CertificateIssue::Revoked,
        CertificateIssue::InvalidSignature,
        CertificateIssue::Malformed,
    ];

    #[test]
    fn valid_certificate_is_trusted_even_with_stale_exception() {
        let h = host("imap.example.com");
        for exceptions in [vec![], vec![exception("imap.example.com", FP_B)]] {
            assert_eq!(
                TlsTrustPolicy::evaluate(&h, &FP_A, Ok(()), &exceptions),
                TrustDecision::Trusted
            );
        }
    }

    #[test]
    fn every_issue_is_rejected_without_exception() {
        let h = host("imap.example.com");
        for issue in ALL_ISSUES
            .into_iter()
            .chain([CertificateIssue::Other("x".into())])
        {
            let decision = TlsTrustPolicy::evaluate(&h, &FP_A, Err(issue.clone()), &[]);
            assert_eq!(
                decision,
                TrustDecision::Rejected(CertificateRejection {
                    host: h.clone(),
                    fingerprint: FP_A,
                    issue: issue.clone(),
                    fingerprint_changed: false,
                })
            );
        }
    }

    #[test]
    fn exception_accepts_same_host_and_same_fingerprint_only() {
        let h = host("imap.example.com");
        for issue in ALL_ISSUES {
            let revoked = issue == CertificateIssue::Revoked;
            let decision = TlsTrustPolicy::evaluate(
                &h,
                &FP_A,
                Err(issue),
                &[exception("imap.example.com", FP_A)],
            );
            if revoked {
                assert!(matches!(decision, TrustDecision::Rejected(ref r) if !r.can_be_excepted()));
            } else {
                assert_eq!(decision, TrustDecision::TrustedByException);
            }
        }
    }

    #[test]
    fn exception_for_another_host_is_ignored() {
        let decision = TlsTrustPolicy::evaluate(
            &host("imap.example.com"),
            &FP_A,
            Err(CertificateIssue::UnknownIssuer),
            &[exception("smtp.example.com", FP_A)],
        );
        assert!(matches!(
            decision,
            TrustDecision::Rejected(CertificateRejection {
                fingerprint_changed: false,
                ..
            })
        ));
    }

    #[test]
    fn changed_fingerprint_is_rejected_and_flagged() {
        let decision = TlsTrustPolicy::evaluate(
            &host("imap.example.com"),
            &FP_A,
            Err(CertificateIssue::UnknownIssuer),
            &[exception("imap.example.com", FP_B)],
        );
        let TrustDecision::Rejected(rejection) = decision else {
            panic!("empreinte changée acceptée");
        };
        assert!(rejection.fingerprint_changed);
        assert!(rejection.can_be_excepted());
        assert_eq!(rejection.issue, CertificateIssue::UnknownIssuer);
    }

    #[test]
    fn host_comparison_is_normalized() {
        let decision = TlsTrustPolicy::evaluate(
            &host("IMAP.example.com."),
            &FP_A,
            Err(CertificateIssue::UnknownIssuer),
            &[exception("imap.EXAMPLE.com", FP_A)],
        );
        assert_eq!(decision, TrustDecision::TrustedByException);
    }

    #[test]
    fn fingerprint_hex_roundtrip() {
        let text = FP_A.to_string();
        assert_eq!(text.len(), 32 * 3 - 1);
        assert!(text.starts_with("AA:AA"));
        assert_eq!(CertificateFingerprint::parse_hex(&text), Ok(FP_A));
        assert_eq!(
            CertificateFingerprint::parse_hex(&"bb".repeat(32)),
            Ok(FP_B)
        );
        assert_eq!(
            CertificateFingerprint::parse_hex("zz"),
            Err(FingerprintError)
        );
        assert_eq!(
            CertificateFingerprint::parse_hex(&"g0".repeat(32)),
            Err(FingerprintError)
        );
        assert_eq!(String::from(FP_B), FP_B.to_string());
    }

    fn endpoint(h: &str, port: u16, security: Security) -> ServerEndpoint {
        let Ok(e) = ServerEndpoint::new(host(h), port, security) else {
            panic!("point de connexion valide refusé");
        };
        e
    }

    #[test]
    fn plaintext_is_refused_without_consent() {
        let policy = PlaintextPolicy::deny_all();
        assert_eq!(
            policy.authorize(&endpoint("mail.lan", 143, Security::Plain)),
            Err(PlaintextRefused {
                host: host("mail.lan"),
                port: 143
            })
        );
        assert_eq!(
            policy.authorize(&endpoint("mail.lan", 993, Security::Tls)),
            Ok(())
        );
        assert_eq!(
            policy.authorize(&endpoint("mail.lan", 143, Security::StartTls)),
            Ok(())
        );
    }

    #[test]
    fn plaintext_consent_is_per_host_and_port() {
        let policy = PlaintextPolicy::deny_all().with_consent(PlaintextConsent {
            host: host("mail.lan"),
            port: 143,
        });
        assert_eq!(
            policy.authorize(&endpoint("MAIL.lan", 143, Security::Plain)),
            Ok(())
        );
        assert!(
            policy
                .authorize(&endpoint("mail.lan", 25, Security::Plain))
                .is_err()
        );
        assert!(
            policy
                .authorize(&endpoint("other.lan", 143, Security::Plain))
                .is_err()
        );
    }

    #[test]
    fn starttls_decision_table() {
        assert_eq!(
            decide_starttls(Security::Tls, false),
            Ok(StartTlsDecision::AlreadyEncrypted)
        );
        assert_eq!(
            decide_starttls(Security::Tls, true),
            Ok(StartTlsDecision::AlreadyEncrypted)
        );
        assert_eq!(
            decide_starttls(Security::StartTls, true),
            Ok(StartTlsDecision::Upgrade)
        );
        assert_eq!(
            decide_starttls(Security::StartTls, false),
            Err(StartTlsUnavailable)
        );
        assert_eq!(
            decide_starttls(Security::Plain, true),
            Ok(StartTlsDecision::Upgrade)
        );
        assert_eq!(
            decide_starttls(Security::Plain, false),
            Ok(StartTlsDecision::StayPlain)
        );
    }
}
