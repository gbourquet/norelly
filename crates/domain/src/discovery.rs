//! Stratégie d'autodécouverte des serveurs d'un compte, à partir de
//! l'adresse.
//!
//! Ordre des sources (on s'arrête dès que IMAP et SMTP sont trouvés) :
//! 1. autoconfig du fournisseur (`autoconfig.<domaine>` puis
//!    `<domaine>/.well-known/autoconfig`), en HTTPS uniquement ;
//! 2. ISPDB de Mozilla (HTTPS, seul le domaine est transmis) ;
//! 3. enregistrements SRV (RFC 6186) ;
//! 4. sondage des ports usuels sur `imap.`, `smtp.`, `mail.` et le domaine
//!    lui-même, en TLS implicite ou STARTTLS uniquement ;
//! 5. saisie manuelle.
//!
//! Pour chaque protocole, on retient le premier serveur de la première
//! source qui en fournit, en préférant TLS implicite, puis STARTTLS, puis
//! le clair (qui exigera encore un consentement explicite).
//!
//! Sécurité : une réponse autoconfig ou DNS est une donnée non fiable. Le
//! HTTP en clair n'est jamais utilisé (une réponse falsifiée enverrait le
//! mot de passe à un serveur choisi par l'attaquant). Le DNS n'étant pas
//! authentifié, un serveur SRV hors du domaine de l'adresse doit être
//! confirmé par l'utilisateur.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::address::EmailAddress;
use crate::auth::AuthError;
use crate::credentials::{CredentialsError, Username};
use crate::endpoint::{Host, Protocol, Security, ServerEndpoint};

/// Origine de réglages découverts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DiscoverySource {
    /// Fichier autoconfig publié par le fournisseur.
    ProviderAutoconfig,
    /// Base ISPDB de Mozilla.
    MozillaIspdb,
    /// Enregistrements DNS SRV (RFC 6186).
    DnsSrv,
    /// Sondage des ports usuels.
    Probe,
    /// Saisie par l'utilisateur.
    Manual,
}

impl DiscoverySource {
    /// Sources automatiques, dans l'ordre où elles sont essayées.
    pub const AUTOMATIC: [Self; 4] = [
        Self::ProviderAutoconfig,
        Self::MozillaIspdb,
        Self::DnsSrv,
        Self::Probe,
    ];
}

impl fmt::Display for DiscoverySource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ProviderAutoconfig => "configuration publiée par le fournisseur",
            Self::MozillaIspdb => "base de fournisseurs de Mozilla",
            Self::DnsSrv => "enregistrements DNS SRV",
            Self::Probe => "sondage des ports usuels",
            Self::Manual => "saisie manuelle",
        })
    }
}

/// Rang de préférence d'un mode de sécurité (0 = meilleur).
#[must_use]
pub const fn security_rank(security: Security) -> u8 {
    match security {
        Security::Tls => 0,
        Security::StartTls => 1,
        Security::Plain => 2,
    }
}

/// Mode de sécurité d'un `socketType` autoconfig (`SSL`, `STARTTLS`,
/// `plain`), insensible à la casse.
#[must_use]
pub fn socket_type(raw: &str) -> Option<Security> {
    match raw.trim().to_ascii_uppercase().as_str() {
        "SSL" | "TLS" => Some(Security::Tls),
        "STARTTLS" => Some(Security::StartTls),
        "PLAIN" => Some(Security::Plain),
        _ => None,
    }
}

/// Vrai si une méthode d'authentification autoconfig est utilisable avec
/// un mot de passe (Norelly n'a pas encore d'OAuth2).
#[must_use]
pub fn is_password_authentication(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "password-cleartext" | "plain" | "password-encrypted" | "secure"
    )
}

/// Erreur de modèle de nom d'utilisateur.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UsernameTemplateError {
    #[error("modèle de nom d'utilisateur trop long")]
    TooLong,
    #[error("variable inconnue dans le modèle de nom d'utilisateur")]
    UnknownPlaceholder,
    #[error(transparent)]
    Invalid(#[from] CredentialsError),
}

/// Modèle de nom d'utilisateur des fichiers autoconfig : texte libre avec
/// les variables `%EMAILADDRESS%`, `%EMAILLOCALPART%` et `%EMAILDOMAIN%`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UsernameTemplate(String);

/// Longueur maximale d'un modèle (borne contre un fichier hostile).
const MAX_TEMPLATE_LEN: usize = 256;

impl UsernameTemplate {
    /// L'adresse complète : choix par défaut (SRV, sondage, RFC 6186).
    #[must_use]
    pub fn email_address() -> Self {
        Self("%EMAILADDRESS%".to_owned())
    }

    pub fn parse(raw: &str) -> Result<Self, UsernameTemplateError> {
        let raw = raw.trim();
        if raw.len() > MAX_TEMPLATE_LEN {
            return Err(UsernameTemplateError::TooLong);
        }
        if raw.chars().any(char::is_control) {
            return Err(CredentialsError::UsernameControlCharacter.into());
        }
        Ok(Self(raw.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Nom d'utilisateur pour `address`. Toute autre variable `%…%` est
    /// refusée ; le résultat est validé comme un [`Username`].
    pub fn resolve(&self, address: &EmailAddress) -> Result<Username, UsernameTemplateError> {
        let resolved = self
            .0
            .replace("%EMAILADDRESS%", &address.to_string())
            .replace("%EMAILLOCALPART%", address.local_part())
            .replace("%EMAILDOMAIN%", address.domain().as_str());
        // Un « % » restant (dans le modèle, pas dans l'adresse) trahit une
        // variable non prise en charge.
        let template_without_known = self
            .0
            .replace("%EMAILADDRESS%", "")
            .replace("%EMAILLOCALPART%", "")
            .replace("%EMAILDOMAIN%", "");
        if template_without_known.contains('%') {
            return Err(UsernameTemplateError::UnknownPlaceholder);
        }
        Ok(Username::parse(&resolved)?)
    }
}

/// Un serveur proposé par une source.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ServerCandidate {
    pub protocol: Protocol,
    pub endpoint: ServerEndpoint,
    pub username: UsernameTemplate,
}

/// Serveurs proposés par une source, dans l'ordre de la source.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredServers {
    pub imap: Vec<ServerCandidate>,
    pub smtp: Vec<ServerCandidate>,
}

impl DiscoveredServers {
    /// Range un candidat selon son protocole.
    pub fn push(&mut self, candidate: ServerCandidate) {
        match candidate.protocol {
            Protocol::Imap => self.imap.push(candidate),
            Protocol::Smtp => self.smtp.push(candidate),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.imap.is_empty() && self.smtp.is_empty()
    }

    #[must_use]
    pub fn candidates(&self, protocol: Protocol) -> &[ServerCandidate] {
        match protocol {
            Protocol::Imap => &self.imap,
            Protocol::Smtp => &self.smtp,
        }
    }
}

/// Meilleur candidat : sécurité la plus forte, puis ordre de la source.
#[must_use]
pub fn best_candidate(candidates: &[ServerCandidate]) -> Option<&ServerCandidate> {
    candidates
        .iter()
        .enumerate()
        .min_by_key(|(i, c)| (security_rank(c.endpoint.security()), *i))
        .map(|(_, c)| c)
}

/// Vrai si `host` est le domaine lui-même ou l'un de ses sous-domaines.
#[must_use]
pub fn host_in_domain(host: &Host, domain: &Host) -> bool {
    let (h, d) = (host.as_str(), domain.as_str());
    h == d
        || h.strip_suffix(d)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

/// Encodage d'URL (RFC 3986) : tout sauf les caractères non réservés.
fn percent_encode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Requête autoconfig : URL HTTPS à interroger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoconfigRequest {
    pub source: DiscoverySource,
    pub url: String,
    /// Domaine de l'adresse : remplace `%EMAILDOMAIN%` dans les noms
    /// d'hôte du fichier.
    pub domain: Host,
}

/// Base de l'ISPDB de Mozilla.
pub const MOZILLA_ISPDB_BASE: &str = "https://autoconfig.thunderbird.net/v1.1/";

/// URL autoconfig à essayer, dans l'ordre, pour une source.
///
/// L'adresse complète n'est envoyée qu'au serveur du fournisseur (qui la
/// connaît déjà) ; l'ISPDB ne reçoit que le domaine.
#[must_use]
pub fn autoconfig_requests(
    source: DiscoverySource,
    address: &EmailAddress,
) -> Vec<AutoconfigRequest> {
    let domain = address.domain().as_str();
    let urls = match source {
        DiscoverySource::ProviderAutoconfig => vec![
            format!(
                "https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress={}",
                percent_encode(&address.to_string())
            ),
            format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml"),
        ],
        DiscoverySource::MozillaIspdb => vec![format!("{MOZILLA_ISPDB_BASE}{domain}")],
        DiscoverySource::DnsSrv | DiscoverySource::Probe | DiscoverySource::Manual => vec![],
    };
    urls.into_iter()
        .map(|url| AutoconfigRequest {
            source,
            url,
            domain: address.domain().clone(),
        })
        .collect()
}

/// Service SRV de la RFC 6186 (et RFC 8314 pour la soumission en TLS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SrvService {
    /// `_imaps._tcp` : IMAP en TLS implicite.
    Imaps,
    /// `_imap._tcp` : IMAP avec STARTTLS (le clair n'est jamais déduit du
    /// DNS).
    Imap,
    /// `_submissions._tcp` : soumission en TLS implicite.
    Submissions,
    /// `_submission._tcp` : soumission avec STARTTLS.
    Submission,
}

impl SrvService {
    /// Services interrogés, par ordre de préférence.
    pub const ALL: [Self; 4] = [Self::Imaps, Self::Imap, Self::Submissions, Self::Submission];

    /// Préfixe DNS (`_imaps._tcp`).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Imaps => "_imaps._tcp",
            Self::Imap => "_imap._tcp",
            Self::Submissions => "_submissions._tcp",
            Self::Submission => "_submission._tcp",
        }
    }

    #[must_use]
    pub const fn protocol(self) -> Protocol {
        match self {
            Self::Imaps | Self::Imap => Protocol::Imap,
            Self::Submissions | Self::Submission => Protocol::Smtp,
        }
    }

    #[must_use]
    pub const fn security(self) -> Security {
        match self {
            Self::Imaps | Self::Submissions => Security::Tls,
            Self::Imap | Self::Submission => Security::StartTls,
        }
    }
}

/// Enregistrement SRV. `target` vaut `None` pour la cible « . » (service
/// explicitement absent, RFC 2782).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SrvRecord {
    pub priority: u16,
    pub weight: u16,
    pub port: u16,
    pub target: Option<Host>,
}

/// Candidats d'un service SRV : priorité croissante, puis poids
/// décroissant. Les cibles « . » et les ports nuls sont ignorés.
#[must_use]
pub fn srv_candidates(service: SrvService, records: &[SrvRecord]) -> Vec<ServerCandidate> {
    let mut sorted: Vec<&SrvRecord> = records.iter().collect();
    sorted.sort_by_key(|r| (r.priority, std::cmp::Reverse(r.weight)));
    sorted
        .into_iter()
        .filter_map(|r| {
            let target = r.target.clone()?;
            let endpoint = ServerEndpoint::new(target, r.port, service.security()).ok()?;
            Some(ServerCandidate {
                protocol: service.protocol(),
                endpoint,
                username: UsernameTemplate::email_address(),
            })
        })
        .collect()
}

/// Plan de sondage d'un protocole : préfixes d'hôte et ports.
type ProbePlan = (Protocol, [&'static str; 3], [(u16, Security); 2]);

/// Candidats du sondage, dans l'ordre d'essai. Jamais de clair.
#[must_use]
pub fn probe_candidates(address: &EmailAddress) -> Vec<ServerCandidate> {
    let domain = address.domain().as_str();
    let plan: [ProbePlan; 2] = [
        (
            Protocol::Imap,
            ["imap", "mail", ""],
            [(993, Security::Tls), (143, Security::StartTls)],
        ),
        (
            Protocol::Smtp,
            ["smtp", "mail", ""],
            [(465, Security::Tls), (587, Security::StartTls)],
        ),
    ];
    let mut out = Vec::new();
    for (protocol, prefixes, ports) in plan {
        for (port, security) in ports {
            for prefix in prefixes {
                let name = if prefix.is_empty() {
                    domain.to_owned()
                } else {
                    format!("{prefix}.{domain}")
                };
                let Ok(host) = Host::parse(&name) else {
                    continue;
                };
                let Ok(endpoint) = ServerEndpoint::new(host, port, security) else {
                    continue;
                };
                out.push(ServerCandidate {
                    protocol,
                    endpoint,
                    username: UsernameTemplate::email_address(),
                });
            }
        }
    }
    out
}

/// Serveur retenu pour un protocole.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredServer {
    pub source: DiscoverySource,
    pub candidate: ServerCandidate,
    /// L'utilisateur doit confirmer ce serveur (SRV hors du domaine de
    /// l'adresse : le DNS n'est pas authentifié).
    pub requires_confirmation: bool,
}

/// Accumulation des résultats des sources successives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovery {
    address: EmailAddress,
    imap: Option<DiscoveredServer>,
    smtp: Option<DiscoveredServer>,
}

impl Discovery {
    #[must_use]
    pub fn new(address: EmailAddress) -> Self {
        Self {
            address,
            imap: None,
            smtp: None,
        }
    }

    #[must_use]
    pub fn address(&self) -> &EmailAddress {
        &self.address
    }

    /// Prend en compte les serveurs fournis par `source`. Un protocole déjà
    /// trouvé par une source précédente n'est pas remplacé. Les candidats
    /// dont le nom d'utilisateur ne peut pas être déduit sont ignorés.
    pub fn record(&mut self, source: DiscoverySource, servers: &DiscoveredServers) {
        for protocol in [Protocol::Imap, Protocol::Smtp] {
            let slot = match protocol {
                Protocol::Imap => &mut self.imap,
                Protocol::Smtp => &mut self.smtp,
            };
            if slot.is_some() {
                continue;
            }
            let usable: Vec<ServerCandidate> = servers
                .candidates(protocol)
                .iter()
                .filter(|c| c.protocol == protocol && c.username.resolve(&self.address).is_ok())
                .cloned()
                .collect();
            if let Some(best) = best_candidate(&usable) {
                let requires_confirmation = source == DiscoverySource::DnsSrv
                    && !host_in_domain(best.endpoint.host(), self.address.domain());
                *slot = Some(DiscoveredServer {
                    source,
                    candidate: best.clone(),
                    requires_confirmation,
                });
            }
        }
    }

    /// IMAP et SMTP trouvés : inutile d'essayer les sources suivantes.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.imap.is_some() && self.smtp.is_some()
    }

    #[must_use]
    pub fn imap(&self) -> Option<&DiscoveredServer> {
        self.imap.as_ref()
    }

    #[must_use]
    pub fn smtp(&self) -> Option<&DiscoveredServer> {
        self.smtp.as_ref()
    }
}

/// Conseil à afficher quand un serveur refuse les identifiants d'un compte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AuthenticationAdvice {
    /// Google refuse le mot de passe du compte en IMAP et en SMTP : il faut
    /// activer la validation en deux étapes, créer un mot de passe
    /// d'application et le saisir à la place.
    GoogleAppPassword,
    /// Compte Microsoft dont l'envoi authentifié (SMTP) est désactivé :
    /// la lecture fonctionne, l'envoi doit être activé dans les réglages
    /// du compte Outlook.com.
    MicrosoftSmtpDisabled,
}

/// Domaines des adresses Gmail grand public.
const GOOGLE_MAIL_DOMAINS: [&str; 2] = ["gmail.com", "googlemail.com"];

/// Serveurs IMAP et SMTP de Google, aussi utilisés par les comptes Google
/// Workspace sous un autre domaine.
const GOOGLE_MAIL_SERVERS: [&str; 2] = ["imap.gmail.com", "smtp.gmail.com"];

/// Conseil pour un refus d'authentification de `server` sur le compte
/// `address`.
///
/// Un compte Google est reconnu à son adresse (`gmail.com`,
/// `googlemail.com`) ou à son serveur (`imap.gmail.com`, `smtp.gmail.com`),
/// par égalité exacte : un nom qui ne fait que ressembler n'est pas
/// reconnu. Seul un refus des identifiants appelle le conseil ; un
/// mécanisme d'authentification impossible est un autre problème.
#[must_use]
pub fn authentication_advice(
    address: &EmailAddress,
    server: &Host,
    error: &AuthError,
) -> Option<AuthenticationAdvice> {
    if !matches!(error, AuthError::Rejected { .. }) {
        return None;
    }
    let google = GOOGLE_MAIL_DOMAINS.contains(&address.domain().as_str())
        || GOOGLE_MAIL_SERVERS.contains(&server.as_str());
    google.then_some(AuthenticationAdvice::GoogleAppPassword)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(raw: &str) -> EmailAddress {
        EmailAddress::parse(raw).unwrap_or_else(|_| panic!("adresse : {raw}"))
    }

    fn host(raw: &str) -> Host {
        Host::parse(raw).unwrap_or_else(|_| panic!("hôte : {raw}"))
    }

    fn candidate(protocol: Protocol, h: &str, port: u16, security: Security) -> ServerCandidate {
        ServerCandidate {
            protocol,
            endpoint: ServerEndpoint::new(host(h), port, security)
                .unwrap_or_else(|_| panic!("point")),
            username: UsernameTemplate::email_address(),
        }
    }

    #[test]
    fn automatic_order_is_provider_ispdb_srv_probe() {
        assert_eq!(
            DiscoverySource::AUTOMATIC,
            [
                DiscoverySource::ProviderAutoconfig,
                DiscoverySource::MozillaIspdb,
                DiscoverySource::DnsSrv,
                DiscoverySource::Probe
            ]
        );
    }

    #[test]
    fn security_preference_tls_then_starttls_then_plain() {
        assert!(security_rank(Security::Tls) < security_rank(Security::StartTls));
        assert!(security_rank(Security::StartTls) < security_rank(Security::Plain));
        let list = vec![
            candidate(Protocol::Imap, "a.example.com", 143, Security::Plain),
            candidate(Protocol::Imap, "b.example.com", 143, Security::StartTls),
            candidate(Protocol::Imap, "c.example.com", 993, Security::Tls),
            candidate(Protocol::Imap, "d.example.com", 993, Security::Tls),
        ];
        assert_eq!(
            best_candidate(&list).map(|c| c.endpoint.host().as_str()),
            Some("c.example.com")
        );
        assert_eq!(
            best_candidate(&list[..2]).map(|c| c.endpoint.port()),
            Some(143)
        );
        assert_eq!(
            best_candidate(&list[..2]).map(|c| c.endpoint.security()),
            Some(Security::StartTls)
        );
        assert!(best_candidate(&[]).is_none());
    }

    #[test]
    fn socket_types_and_auth_methods() {
        assert_eq!(socket_type("SSL"), Some(Security::Tls));
        assert_eq!(socket_type(" starttls "), Some(Security::StartTls));
        assert_eq!(socket_type("plain"), Some(Security::Plain));
        assert_eq!(socket_type("TLS1.3"), None);
        assert!(is_password_authentication("password-cleartext"));
        assert!(is_password_authentication("password-encrypted"));
        assert!(!is_password_authentication("OAuth2"));
        assert!(!is_password_authentication("client-IP-address"));
    }

    #[test]
    fn username_templates() {
        let a = address("alice.m@Example.org");
        let resolve = |t: &str| {
            UsernameTemplate::parse(t)
                .ok()
                .and_then(|t| t.resolve(&a).ok())
                .map(|u| u.to_string())
        };
        assert_eq!(
            resolve("%EMAILADDRESS%"),
            Some("alice.m@example.org".into())
        );
        assert_eq!(resolve("%EMAILLOCALPART%"), Some("alice.m".into()));
        assert_eq!(
            resolve("%EMAILLOCALPART%+imap@%EMAILDOMAIN%"),
            Some("alice.m+imap@example.org".into())
        );
        assert_eq!(resolve("fixed-user"), Some("fixed-user".into()));
        assert_eq!(resolve("%REALNAME%"), None);
        assert_eq!(resolve("%EMAILADDRESS"), None);
        assert_eq!(resolve(""), None);
        assert_eq!(resolve("a\r\nb"), None);
        assert_eq!(
            UsernameTemplate::parse("a\nb"),
            Err(UsernameTemplateError::Invalid(
                CredentialsError::UsernameControlCharacter
            ))
        );
        assert_eq!(
            UsernameTemplate::parse(&"x".repeat(257)),
            Err(UsernameTemplateError::TooLong)
        );
    }

    #[test]
    fn percent_sign_in_address_is_not_a_placeholder() {
        let a = address("100%real@example.org");
        assert_eq!(
            UsernameTemplate::email_address()
                .resolve(&a)
                .map(|u| u.to_string()),
            Ok("100%real@example.org".to_owned())
        );
    }

    #[test]
    fn host_in_domain_checks_label_boundaries() {
        let d = host("example.com");
        assert!(host_in_domain(&host("example.com"), &d));
        assert!(host_in_domain(&host("imap.example.com"), &d));
        assert!(host_in_domain(&host("a.b.example.com"), &d));
        assert!(!host_in_domain(&host("badexample.com"), &d));
        assert!(!host_in_domain(&host("example.com.evil.net"), &d));
        assert!(!host_in_domain(&host("com"), &d));
    }

    #[test]
    fn autoconfig_urls_are_https_and_encode_the_address() {
        let a = address("a+b&c=d@Example.org");
        let provider = autoconfig_requests(DiscoverySource::ProviderAutoconfig, &a);
        assert_eq!(
            provider.iter().map(|r| r.url.as_str()).collect::<Vec<_>>(),
            [
                "https://autoconfig.example.org/mail/config-v1.1.xml?emailaddress=a%2Bb%26c%3Dd%40example.org",
                "https://example.org/.well-known/autoconfig/mail/config-v1.1.xml",
            ]
        );
        let ispdb = autoconfig_requests(DiscoverySource::MozillaIspdb, &a);
        assert_eq!(ispdb.len(), 1);
        assert_eq!(
            ispdb.first().map(|r| r.url.as_str()),
            Some("https://autoconfig.thunderbird.net/v1.1/example.org")
        );
        // L'ISPDB ne reçoit jamais la partie locale.
        assert!(ispdb.iter().all(|r| !r.url.contains("a%2Bb")));
        for r in provider.iter().chain(&ispdb) {
            assert!(r.url.starts_with("https://"));
        }
        assert!(autoconfig_requests(DiscoverySource::DnsSrv, &a).is_empty());
    }

    #[test]
    fn utf8_address_is_percent_encoded() {
        let a = address("é@example.org");
        let urls = autoconfig_requests(DiscoverySource::ProviderAutoconfig, &a);
        assert!(
            urls.first()
                .is_some_and(|r| r.url.ends_with("=%C3%A9%40example.org"))
        );
    }

    #[test]
    fn srv_services() {
        assert_eq!(SrvService::Imaps.label(), "_imaps._tcp");
        assert_eq!(SrvService::Submission.label(), "_submission._tcp");
        assert_eq!(SrvService::Imap.security(), Security::StartTls);
        assert_eq!(SrvService::Submissions.protocol(), Protocol::Smtp);
    }

    #[test]
    fn srv_records_are_sorted_and_filtered() {
        let rec = |priority, weight, port, target: Option<&str>| SrvRecord {
            priority,
            weight,
            port,
            target: target.map(host),
        };
        let out = srv_candidates(
            SrvService::Imaps,
            &[
                rec(20, 0, 993, Some("backup.example.com")),
                rec(10, 1, 993, Some("light.example.com")),
                rec(10, 50, 993, Some("heavy.example.com")),
                rec(0, 0, 0, Some("zero-port.example.com")),
                rec(0, 0, 993, None),
            ],
        );
        assert_eq!(
            out.iter()
                .map(|c| c.endpoint.host().as_str())
                .collect::<Vec<_>>(),
            [
                "heavy.example.com",
                "light.example.com",
                "backup.example.com"
            ]
        );
        assert!(out.iter().all(|c| c.endpoint.security() == Security::Tls));
        assert!(out.iter().all(|c| c.protocol == Protocol::Imap));
    }

    #[test]
    fn probe_candidates_never_plaintext() {
        let list = probe_candidates(&address("x@example.net"));
        assert_eq!(list.len(), 12);
        assert!(
            list.iter()
                .all(|c| c.endpoint.security() != Security::Plain)
        );
        let first_imap = list.iter().find(|c| c.protocol == Protocol::Imap);
        assert_eq!(
            first_imap.map(|c| c.endpoint.to_string()),
            Some("imap.example.net:993 (TLS)".to_owned())
        );
        let first_smtp = list.iter().find(|c| c.protocol == Protocol::Smtp);
        assert_eq!(
            first_smtp.map(|c| c.endpoint.to_string()),
            Some("smtp.example.net:465 (TLS)".to_owned())
        );
        assert!(
            list.iter()
                .all(|c| host_in_domain(c.endpoint.host(), &host("example.net")))
        );
    }

    #[test]
    fn discovery_takes_each_protocol_from_the_first_source() {
        let mut d = Discovery::new(address("a@example.com"));
        let mut only_imap = DiscoveredServers::default();
        only_imap.push(candidate(
            Protocol::Imap,
            "imap.example.com",
            143,
            Security::StartTls,
        ));
        only_imap.push(candidate(
            Protocol::Imap,
            "imap.example.com",
            993,
            Security::Tls,
        ));
        d.record(DiscoverySource::ProviderAutoconfig, &only_imap);
        assert!(!d.is_complete());

        let mut both = DiscoveredServers::default();
        both.push(candidate(
            Protocol::Imap,
            "other.example.com",
            993,
            Security::Tls,
        ));
        both.push(candidate(
            Protocol::Smtp,
            "smtp.example.com",
            587,
            Security::StartTls,
        ));
        d.record(DiscoverySource::MozillaIspdb, &both);
        assert!(d.is_complete());

        let imap = d.imap().unwrap_or_else(|| panic!("imap"));
        assert_eq!(imap.source, DiscoverySource::ProviderAutoconfig);
        assert_eq!(imap.candidate.endpoint.port(), 993);
        let smtp = d.smtp().unwrap_or_else(|| panic!("smtp"));
        assert_eq!(smtp.source, DiscoverySource::MozillaIspdb);
        assert!(!smtp.requires_confirmation);
    }

    #[test]
    fn misfiled_or_unusable_candidates_are_ignored() {
        let mut d = Discovery::new(address("a@example.com"));
        let mut servers = DiscoveredServers::default();
        // SMTP rangé dans la liste IMAP par une source fautive.
        servers.imap.push(candidate(
            Protocol::Smtp,
            "smtp.example.com",
            465,
            Security::Tls,
        ));
        let mut bad_user = candidate(Protocol::Imap, "imap.example.com", 993, Security::Tls);
        bad_user.username = UsernameTemplate("%UNKNOWN%".into());
        servers.imap.push(bad_user);
        d.record(DiscoverySource::ProviderAutoconfig, &servers);
        assert!(d.imap().is_none());
        assert!(d.smtp().is_none());
    }

    #[test]
    fn srv_outside_the_domain_requires_confirmation() {
        let mut d = Discovery::new(address("a@example.com"));
        let mut servers = DiscoveredServers::default();
        servers.push(candidate(
            Protocol::Imap,
            "imap.attacker.net",
            993,
            Security::Tls,
        ));
        servers.push(candidate(
            Protocol::Smtp,
            "smtp.example.com",
            465,
            Security::Tls,
        ));
        d.record(DiscoverySource::DnsSrv, &servers);
        assert_eq!(d.imap().map(|s| s.requires_confirmation), Some(true));
        assert_eq!(d.smtp().map(|s| s.requires_confirmation), Some(false));
    }

    #[test]
    fn ispdb_hosts_outside_the_domain_are_trusted() {
        // L'ISPDB (HTTPS, base de référence) renvoie souvent un autre domaine
        // (googlemail.com → imap.gmail.com).
        let mut d = Discovery::new(address("a@googlemail.com"));
        let mut servers = DiscoveredServers::default();
        servers.push(candidate(
            Protocol::Imap,
            "imap.gmail.com",
            993,
            Security::Tls,
        ));
        d.record(DiscoverySource::MozillaIspdb, &servers);
        assert_eq!(d.imap().map(|s| s.requires_confirmation), Some(false));
    }

    fn rejected() -> AuthError {
        AuthError::Rejected {
            message: "[AUTHENTICATIONFAILED] Invalid credentials".into(),
        }
    }

    #[test]
    fn gmail_addresses_are_guided_to_an_app_password() {
        for (a, server) in [
            ("alice@gmail.com", "imap.gmail.com"),
            ("alice@GMail.com", "smtp.gmail.com"),
            ("alice@googlemail.com", "imap.gmail.com"),
            // Réglages saisis à la main : l'adresse suffit.
            ("alice@gmail.com", "imap.example.net"),
        ] {
            assert_eq!(
                authentication_advice(&address(a), &host(server), &rejected()),
                Some(AuthenticationAdvice::GoogleAppPassword),
                "{a} sur {server}"
            );
        }
    }

    #[test]
    fn google_workspace_is_recognized_by_its_servers() {
        for server in ["imap.gmail.com", "smtp.gmail.com", "IMAP.GMAIL.COM."] {
            assert_eq!(
                authentication_advice(&address("alice@exemple.fr"), &host(server), &rejected()),
                Some(AuthenticationAdvice::GoogleAppPassword),
                "{server}"
            );
        }
    }

    #[test]
    fn other_providers_and_lookalikes_get_no_advice() {
        for (a, server) in [
            ("alice@example.com", "imap.example.com"),
            ("alice@notgmail.com", "imap.notgmail.com"),
            ("alice@gmail.com.example.net", "imap.example.net"),
            ("alice@example.com", "imap.gmail.com.example.net"),
            ("alice@example.com", "evilgmail.com"),
        ] {
            assert_eq!(
                authentication_advice(&address(a), &host(server), &rejected()),
                None,
                "{a} sur {server}"
            );
        }
    }

    #[test]
    fn only_rejected_credentials_call_for_an_app_password() {
        let gmail = address("alice@gmail.com");
        let server = host("imap.gmail.com");
        for error in [
            AuthError::LoginDisabled,
            AuthError::NoSupportedMechanism { offered: vec![] },
            AuthError::UnexpectedChallenge { step: 1 },
            AuthError::Unsupported(crate::auth::AuthMechanism::XOAuth2),
        ] {
            assert_eq!(
                authentication_advice(&gmail, &server, &error),
                None,
                "{error}"
            );
        }
    }
}
