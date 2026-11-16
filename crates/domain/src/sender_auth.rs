//! Authentification de l'expéditeur d'un message reçu, d'après l'en-tête
//! `Authentication-Results` (RFC 8601) posé par le serveur du compte.
//!
//! L'adresse `From` est falsifiable : seul le verdict d'un serveur de
//! confiance (DMARC, ou DKIM aligné sur le domaine du `From`) permet de
//! cautionner l'expéditeur, par exemple par la pastille « Dans vos
//! contacts ». Un `Authentication-Results` posé ailleurs (par l'expéditeur
//! lui-même, ou par un serveur intermédiaire) est ignoré.
//!
//! Serveur de confiance : celui dont l'`authserv-id` appartient au domaine
//! de l'hôte IMAP du compte (`imap.exemple.fr` → `exemple.fr` et
//! `*.exemple.fr`), ou à la table d'alias des grands fournisseurs dont
//! l'`authserv-id` est ailleurs ([`PROVIDER_ALIASES`]).

use serde::{Deserialize, Serialize};

use crate::address::EmailAddress;
use crate::endpoint::Host;
use crate::mailbox::Correspondent;

/// Verdict sur l'expéditeur d'un message.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SenderAuth {
    /// Le serveur de confiance atteste l'expéditeur : DMARC réussi, ou
    /// signature DKIM valide d'un domaine aligné sur celui du `From`.
    Pass,
    /// Le serveur de confiance a constaté un échec (DMARC, ou DKIM aligné).
    Fail,
    /// Aucun verdict exploitable (pas d'en-tête de confiance, plusieurs
    /// `From`, adresse invalide, message enregistré avant la vérification).
    #[default]
    Unknown,
}

impl SenderAuth {
    /// Code stable, pour le stockage.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Unknown => "unknown",
        }
    }

    /// Verdict d'un code de [`SenderAuth::code`].
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "pass" => Some(Self::Pass),
            "fail" => Some(Self::Fail),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }

    /// Verdict pour un message d'en-têtes `Authentication-Results`
    /// `headers` (valeurs brutes, dans l'ordre du message, du plus récent
    /// au plus ancien) et d'expéditeurs `from`.
    ///
    /// Seul le premier en-tête posé par un serveur de confiance (`trust`)
    /// compte : c'est le dernier ajouté, par le serveur qui a reçu le
    /// message ; ceux qui le suivent ont pu être écrits par l'expéditeur.
    /// Sans `From` unique et valide : [`SenderAuth::Unknown`].
    #[must_use]
    pub fn evaluate<'a>(
        trust: &AuthServTrust,
        headers: impl IntoIterator<Item = &'a str>,
        from: &[Correspondent],
    ) -> Self {
        let [from] = from else {
            return Self::Unknown;
        };
        let Ok(address) = EmailAddress::parse(&from.address) else {
            return Self::Unknown;
        };
        headers
            .into_iter()
            .take(MAX_EXAMINED_HEADERS)
            .filter_map(AuthenticationResults::parse)
            .find(|results| trust.trusts(&results.authserv_id))
            .map_or(Self::Unknown, |results| {
                results.verdict(address.domain().as_str())
            })
    }
}

/// Nombre maximal d'en-têtes `Authentication-Results` examinés par
/// message.
pub const MAX_EXAMINED_HEADERS: usize = 32;

/// Longueur maximale (octets) d'une valeur d'en-tête analysée ; au-delà,
/// l'en-tête est ignoré.
pub const MAX_HEADER_LEN: usize = 16 * 1024;

/// Nombre maximal de résultats retenus par en-tête.
pub const MAX_RESULTS: usize = 64;

/// Alias des grands fournisseurs : domaine de l'hôte IMAP, et
/// `authserv-id` de confiance hors de ce domaine (`*.` : le domaine et
/// tous ses sous-domaines).
pub const PROVIDER_ALIASES: [(&str, &[&str]); 3] = [
    ("gmail.com", &["mx.google.com"]),
    ("googlemail.com", &["mx.google.com"]),
    (
        "office365.com",
        &["*.outlook.com", "*.protection.outlook.com"],
    ),
];

/// Deuxièmes niveaux d'usage courant sous un domaine de pays
/// (`co.uk`, `com.au`) : `exemple.co.uk` est un domaine complet, pas un
/// sous-domaine de `co.uk`.
const SECOND_LEVELS: [&str; 10] = [
    "co", "com", "org", "net", "gov", "gouv", "ac", "edu", "ne", "or",
];

/// `authserv-id` de confiance pour un compte.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthServTrust {
    /// Domaine de l'hôte IMAP (absent pour une adresse IP).
    domain: Option<String>,
    /// Alias du fournisseur.
    aliases: &'static [&'static str],
}

impl AuthServTrust {
    /// Confiance pour un compte dont le serveur IMAP est `host` : son
    /// domaine (l'hôte sans sa première étiquette s'il en a au moins
    /// trois, `imap.exemple.fr` → `exemple.fr`) et ses sous-domaines,
    /// plus les alias du fournisseur. Une adresse IP n'a pas de domaine :
    /// rien n'est de confiance, hors alias (aucun).
    #[must_use]
    pub fn for_imap_host(host: &Host) -> Self {
        if host.is_ip_literal() {
            return Self::default();
        }
        let domain = imap_domain(host.as_str());
        let aliases = PROVIDER_ALIASES
            .iter()
            .find(|(provider, _)| within(&domain, provider))
            .map_or(&[][..], |(_, aliases)| *aliases);
        Self {
            domain: Some(domain),
            aliases,
        }
    }

    /// Vrai si `authserv_id` est celui d'un serveur de confiance.
    #[must_use]
    pub fn trusts(&self, authserv_id: &str) -> bool {
        let id = authserv_id.trim_end_matches('.').to_ascii_lowercase();
        if id.is_empty() {
            return false;
        }
        self.domain.as_deref().is_some_and(|d| within(&id, d))
            || self
                .aliases
                .iter()
                .any(|alias| match alias.strip_prefix("*.") {
                    Some(domain) => within(&id, domain),
                    None => id == *alias,
                })
    }
}

/// Domaine d'un hôte IMAP (en minuscules) : sans sa première étiquette
/// s'il en a au moins trois, sauf si le reste est un deuxième niveau de
/// pays (`exemple.co.uk` reste entier).
fn imap_domain(host: &str) -> String {
    let labels: Vec<&str> = host.split('.').collect();
    let rest = labels.get(1..).unwrap_or_default();
    let country_second_level = rest.len() == 2
        && rest.first().is_some_and(|l| SECOND_LEVELS.contains(l))
        && rest.get(1).is_some_and(|tld| tld.len() == 2);
    if labels.len() >= 3 && !country_second_level {
        rest.join(".")
    } else {
        host.to_owned()
    }
}

/// Vrai si `name` est `domain` ou l'un de ses sous-domaines.
fn within(name: &str, domain: &str) -> bool {
    name == domain
        || name
            .strip_suffix(domain)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

/// Un résultat d'authentification (`dkim=pass header.d=exemple.fr`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodResult {
    /// Méthode, en minuscules, sans version (`dkim`, `dmarc`, `spf`).
    pub method: String,
    /// Résultat, en minuscules (`pass`, `fail`, `none`…).
    pub result: String,
    /// Propriétés (`header.d`, `header.from`…) : nom en minuscules,
    /// valeur telle quelle.
    pub properties: Vec<(String, String)>,
}

impl MethodResult {
    fn property(&self, name: &str) -> Option<&str> {
        self.properties
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Contenu d'un en-tête `Authentication-Results`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticationResults {
    /// Serveur qui a posé l'en-tête, en minuscules.
    pub authserv_id: String,
    pub results: Vec<MethodResult>,
}

impl AuthenticationResults {
    /// Analyse la valeur brute (éventuellement repliée) d'un en-tête ;
    /// `None` si elle est vide, trop longue ou sans `authserv-id`.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        if value.len() > MAX_HEADER_LEN {
            return None;
        }
        let tokens = tokens(value);
        let mut segments = tokens.split(|t| *t == Token::Semicolon);
        let authserv_id = match segments.next()?.first()? {
            Token::Word(id) => id.trim_end_matches('.').to_ascii_lowercase(),
            _ => return None,
        };
        if authserv_id.is_empty() {
            return None;
        }
        let results = segments
            .filter_map(method_result)
            .take(MAX_RESULTS)
            .collect();
        Some(Self {
            authserv_id,
            results,
        })
    }

    /// Verdict de cet en-tête pour un `From` du domaine `from_domain` :
    /// un résultat DMARC sur ce domaine fait foi (réussi ou échoué) ;
    /// sinon une signature DKIM valide d'un domaine aligné atteste
    /// l'expéditeur, et à défaut une signature alignée invalide le met en
    /// échec.
    fn verdict(&self, from_domain: &str) -> SenderAuth {
        let dmarc = self
            .results
            .iter()
            .filter(|r| r.method == "dmarc")
            .find(|r| {
                r.property("header.from")
                    .is_none_or(|d| d.eq_ignore_ascii_case(from_domain))
                    && matches!(r.result.as_str(), "pass" | "fail")
            });
        match dmarc.map(|r| r.result.as_str()) {
            Some("pass") => return SenderAuth::Pass,
            Some("fail") => return SenderAuth::Fail,
            _ => {}
        }
        let aligned: Vec<&str> = self
            .results
            .iter()
            .filter(|r| r.method == "dkim")
            .filter(|r| dkim_domain(r).is_some_and(|d| aligned(&d, from_domain)))
            .map(|r| r.result.as_str())
            .collect();
        if aligned.contains(&"pass") {
            SenderAuth::Pass
        } else if aligned.contains(&"fail") {
            SenderAuth::Fail
        } else {
            SenderAuth::Unknown
        }
    }
}

/// Domaine signataire d'un résultat DKIM, en minuscules : `header.d`, à
/// défaut le domaine de `header.i`.
fn dkim_domain(result: &MethodResult) -> Option<String> {
    result
        .property("header.d")
        .or_else(|| {
            result
                .property("header.i")
                .and_then(|i| i.rsplit_once('@'))
                .map(|(_, d)| d)
        })
        .map(|d| d.trim_end_matches('.').to_ascii_lowercase())
}

/// Vrai si le domaine signataire `signer` est aligné sur `from_domain`
/// (alignement relâché) : le même domaine, ou un domaine parent du `From`
/// (`banque.fr` pour `news.banque.fr`), jamais un domaine de premier
/// niveau seul.
fn aligned(signer: &str, from_domain: &str) -> bool {
    signer.contains('.') && within(&from_domain.to_ascii_lowercase(), signer)
}

/// Résultat d'un segment (`méthode=résultat propriété=valeur…`) ; `None`
/// sans couple `méthode=résultat` en tête (`none`).
fn method_result(segment: &[Token]) -> Option<MethodResult> {
    let mut pairs = Vec::new();
    let mut rest = segment;
    while let Some((first, tail)) = rest.split_first() {
        match (first, tail) {
            (Token::Word(key), [Token::Equals, Token::Word(value), after @ ..]) => {
                pairs.push((key.to_ascii_lowercase(), value.clone()));
                rest = after;
            }
            _ => rest = tail,
        }
        if pairs.len() > MAX_RESULTS {
            break;
        }
    }
    let mut pairs = pairs.into_iter();
    let (method, result) = pairs.next()?;
    let method = method
        .split_once('/')
        .map_or(method.as_str(), |(m, _)| m)
        .to_owned();
    Some(MethodResult {
        method,
        result: result.to_ascii_lowercase(),
        properties: pairs.collect(),
    })
}

/// Élément lexical d'un en-tête `Authentication-Results`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Word(String),
    Equals,
    Semicolon,
}

/// Découpe une valeur d'en-tête : commentaires `(…)` (imbriqués) et
/// blancs (repliement compris) ignorés, chaînes entre guillemets
/// décodées, `=` et `;` isolés.
fn tokens(value: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '(' => {
                let mut depth = 1_usize;
                while depth > 0 {
                    match chars.next() {
                        Some('\\') => {
                            chars.next();
                        }
                        Some('(') => depth += 1,
                        Some(')') => depth -= 1,
                        Some(_) => {}
                        None => break,
                    }
                }
            }
            '"' => {
                let mut word = String::new();
                loop {
                    match chars.next() {
                        Some('\\') => word.extend(chars.next()),
                        Some('"') | None => break,
                        Some(c) => word.push(c),
                    }
                }
                out.push(Token::Word(word));
            }
            '=' => out.push(Token::Equals),
            ';' => out.push(Token::Semicolon),
            c if c.is_whitespace() => {}
            c => {
                let mut word = String::from(c);
                while let Some(&next) = chars.peek() {
                    if next.is_whitespace() || matches!(next, '(' | '"' | '=' | ';') {
                        break;
                    }
                    word.push(next);
                    chars.next();
                }
                out.push(Token::Word(word));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(raw: &str) -> Host {
        Host::parse(raw).unwrap_or_else(|e| panic!("hôte {raw} : {e}"))
    }

    fn trust(raw: &str) -> AuthServTrust {
        AuthServTrust::for_imap_host(&host(raw))
    }

    fn from(address: &str) -> Vec<Correspondent> {
        vec![Correspondent::new("Alice", address)]
    }

    fn verdict(imap: &str, headers: &[&str], sender: &str) -> SenderAuth {
        SenderAuth::evaluate(&trust(imap), headers.iter().copied(), &from(sender))
    }

    #[test]
    fn codes_round_trip() {
        for auth in [SenderAuth::Pass, SenderAuth::Fail, SenderAuth::Unknown] {
            assert_eq!(SenderAuth::from_code(auth.code()), Some(auth));
        }
        assert_eq!(SenderAuth::from_code("PASS"), None);
        assert_eq!(SenderAuth::default(), SenderAuth::Unknown);
    }

    #[test]
    fn the_imap_host_domain_and_its_subdomains_are_trusted() {
        let t = trust("imap.exemple.fr");
        for id in [
            "exemple.fr",
            "mx.exemple.fr",
            "MX1.Exemple.FR",
            "a.b.exemple.fr",
            "mx.exemple.fr.",
        ] {
            assert!(t.trusts(id), "{id}");
        }
        for id in [
            "exemple.com",
            "pirate-exemple.fr",
            "exemple.fr.pirate.net",
            "fr",
            "",
            "mx.google.com",
        ] {
            assert!(!t.trusts(id), "{id}");
        }
    }

    #[test]
    fn a_two_label_host_is_its_own_domain() {
        let t = trust("exemple.fr");
        assert!(t.trusts("mx.exemple.fr"));
        assert!(!t.trusts("autre.fr"));
        let single = trust("localhost");
        assert!(single.trusts("localhost"));
        assert!(single.trusts("mx.localhost"));
        assert!(!single.trusts("example.com"));
    }

    #[test]
    fn a_country_second_level_is_never_the_trusted_domain() {
        let t = trust("exemple.co.uk");
        assert!(t.trusts("mx.exemple.co.uk"));
        assert!(!t.trusts("pirate.co.uk"));
        let t = trust("imap.exemple.co.uk");
        assert!(t.trusts("mx.exemple.co.uk"));
        assert!(!t.trusts("pirate.co.uk"));
    }

    #[test]
    fn an_ip_host_trusts_nothing() {
        let t = trust("192.0.2.1");
        assert!(!t.trusts("192.0.2.1"));
        assert!(!t.trusts("0.2.1"));
        assert!(!trust("::1").trusts("::1"));
        assert!(!AuthServTrust::default().trusts("mx.exemple.fr"));
    }

    #[test]
    fn provider_aliases_are_trusted_only_for_their_provider() {
        let gmail = trust("imap.gmail.com");
        assert!(gmail.trusts("mx.google.com"));
        assert!(!gmail.trusts("pirate.google.com"));
        let outlook = trust("outlook.office365.com");
        assert!(outlook.trusts("mail.protection.outlook.com"));
        assert!(outlook.trusts("am0pr01mb1234.eurprd01.prod.outlook.com"));
        assert!(!outlook.trusts("mx.google.com"));
        let hotmail = trust("imap-mail.outlook.com");
        assert!(hotmail.trusts("bl2nam02.prod.protection.outlook.com"));
        // Les alias d'un fournisseur ne valent pas pour un autre compte.
        assert!(!trust("imap.exemple.fr").trusts("mx.google.com"));
        assert!(!trust("imap.exemple.fr").trusts("mail.protection.outlook.com"));
    }

    #[test]
    fn parses_a_folded_header_with_comments_and_versions() {
        let raw = "mx.google.com;\r\n       dkim=pass header.i=@exemple.fr header.s=s1 header.b=AbC;\r\n       spf=pass (google.com: domain of a@exemple.fr designates 192.0.2.1 as permitted sender) smtp.mailfrom=a@exemple.fr;\r\n       dmarc=pass (p=NONE sp=NONE dis=NONE) header.from=exemple.fr";
        let parsed = AuthenticationResults::parse(raw);
        let parsed = parsed.unwrap_or_else(|| panic!("en-tête refusé"));
        assert_eq!(parsed.authserv_id, "mx.google.com");
        let methods: Vec<(&str, &str)> = parsed
            .results
            .iter()
            .map(|r| (r.method.as_str(), r.result.as_str()))
            .collect();
        assert_eq!(
            methods,
            [("dkim", "pass"), ("spf", "pass"), ("dmarc", "pass")]
        );
        assert_eq!(
            parsed.results.first().and_then(|r| r.property("header.i")),
            Some("@exemple.fr")
        );
        assert_eq!(
            parsed
                .results
                .get(2)
                .and_then(|r| r.property("header.from")),
            Some("exemple.fr")
        );
    }

    #[test]
    fn parses_versions_spaces_quotes_and_none() {
        let parsed = AuthenticationResults::parse(
            "MX.Exemple.FR 1; DKIM/1 = Pass (ok) header.d = \"exemple.fr\"; reason=\"x;y\"",
        );
        let parsed = parsed.unwrap_or_else(|| panic!("en-tête refusé"));
        assert_eq!(parsed.authserv_id, "mx.exemple.fr");
        let dkim = parsed.results.first();
        assert_eq!(dkim.map(|r| r.method.as_str()), Some("dkim"));
        assert_eq!(dkim.map(|r| r.result.as_str()), Some("pass"));
        assert_eq!(
            dkim.and_then(|r| r.property("header.d")),
            Some("exemple.fr")
        );
        let none = AuthenticationResults::parse("mx.exemple.fr; none");
        assert_eq!(
            none.map(|n| (n.authserv_id, n.results.len())),
            Some(("mx.exemple.fr".to_owned(), 0))
        );
    }

    #[test]
    fn rejects_empty_or_oversized_headers() {
        assert_eq!(AuthenticationResults::parse(""), None);
        assert_eq!(
            AuthenticationResults::parse("  (commentaire) ; dkim=pass"),
            None
        );
        assert_eq!(AuthenticationResults::parse("; dkim=pass"), None);
        let long = format!(
            "mx.exemple.fr; dkim=pass header.d={}",
            "a".repeat(MAX_HEADER_LEN)
        );
        assert_eq!(AuthenticationResults::parse(&long), None);
        let many = format!("mx.exemple.fr{}", "; dkim=pass".repeat(MAX_RESULTS + 10));
        assert_eq!(
            AuthenticationResults::parse(&many).map(|r| r.results.len()),
            Some(MAX_RESULTS)
        );
    }

    #[test]
    fn dmarc_pass_from_the_trusted_server_passes() {
        assert_eq!(
            verdict(
                "imap.exemple.fr",
                &["mx.exemple.fr; spf=fail; dmarc=pass header.from=banque.fr"],
                "alice@banque.fr"
            ),
            SenderAuth::Pass
        );
        // `header.from` absent : le verdict porte sur le `From` du message.
        assert_eq!(
            verdict(
                "imap.exemple.fr",
                &["mx.exemple.fr; dmarc=pass"],
                "a@banque.fr"
            ),
            SenderAuth::Pass
        );
    }

    #[test]
    fn dmarc_fail_from_the_trusted_server_fails() {
        assert_eq!(
            verdict(
                "imap.exemple.fr",
                &["mx.exemple.fr; dkim=pass header.d=banque.fr; dmarc=fail header.from=banque.fr"],
                "alice@banque.fr"
            ),
            SenderAuth::Fail
        );
    }

    #[test]
    fn a_dmarc_result_about_another_domain_is_ignored() {
        assert_eq!(
            verdict(
                "imap.exemple.fr",
                &["mx.exemple.fr; dmarc=pass header.from=pirate.net"],
                "alice@banque.fr"
            ),
            SenderAuth::Unknown
        );
    }

    #[test]
    fn aligned_dkim_passes_and_unaligned_dkim_does_not() {
        let at = |header: &str, sender: &str| verdict("imap.exemple.fr", &[header], sender);
        assert_eq!(
            at("mx.exemple.fr; dkim=pass header.d=banque.fr", "a@banque.fr"),
            SenderAuth::Pass
        );
        assert_eq!(
            at(
                "mx.exemple.fr; dkim=pass header.d=Banque.FR",
                "a@news.banque.fr"
            ),
            SenderAuth::Pass
        );
        assert_eq!(
            at(
                "mx.exemple.fr; dkim=pass header.i=@banque.fr",
                "a@banque.fr"
            ),
            SenderAuth::Pass
        );
        assert_eq!(
            at(
                "mx.exemple.fr; dkim=pass header.d=pirate.net",
                "a@banque.fr"
            ),
            SenderAuth::Unknown
        );
        // Un sous-domaine du `From` n'atteste pas le domaine parent.
        assert_eq!(
            at(
                "mx.exemple.fr; dkim=pass header.d=x.banque.fr",
                "a@banque.fr"
            ),
            SenderAuth::Unknown
        );
        assert_eq!(
            at("mx.exemple.fr; dkim=pass header.d=fr", "a@banque.fr"),
            SenderAuth::Unknown
        );
        assert_eq!(
            at(
                "mx.exemple.fr; dkim=pass header.d=notbanque.fr",
                "a@banque.fr"
            ),
            SenderAuth::Unknown
        );
        assert_eq!(
            at("mx.exemple.fr; dkim=fail header.d=banque.fr", "a@banque.fr"),
            SenderAuth::Fail
        );
        assert_eq!(
            at(
                "mx.exemple.fr; dkim=fail header.d=banque.fr; dkim=pass header.d=banque.fr",
                "a@banque.fr"
            ),
            SenderAuth::Pass
        );
        assert_eq!(
            at(
                "mx.exemple.fr; dkim=fail header.d=pirate.net",
                "a@banque.fr"
            ),
            SenderAuth::Unknown
        );
        assert_eq!(
            at(
                "mx.exemple.fr; spf=pass smtp.mailfrom=a@banque.fr",
                "a@banque.fr"
            ),
            SenderAuth::Unknown
        );
    }

    #[test]
    fn untrusted_headers_are_ignored() {
        assert_eq!(
            verdict(
                "imap.exemple.fr",
                &["mx.pirate.net; dmarc=pass header.from=banque.fr"],
                "a@banque.fr"
            ),
            SenderAuth::Unknown
        );
        assert_eq!(
            verdict(
                "imap.exemple.fr",
                &["mx.google.com; dmarc=pass header.from=banque.fr"],
                "a@banque.fr"
            ),
            SenderAuth::Unknown
        );
        assert_eq!(
            verdict(
                "imap.gmail.com",
                &[
                    "mx.pirate.net; dmarc=pass",
                    "mx.google.com; dmarc=fail header.from=banque.fr"
                ],
                "a@banque.fr"
            ),
            SenderAuth::Fail
        );
    }

    #[test]
    fn only_the_topmost_trusted_header_counts() {
        // Le second a pu être écrit par l'expéditeur.
        assert_eq!(
            verdict(
                "imap.exemple.fr",
                &[
                    "mx.exemple.fr; dmarc=fail header.from=banque.fr",
                    "mx.exemple.fr; dmarc=pass header.from=banque.fr",
                ],
                "a@banque.fr"
            ),
            SenderAuth::Fail
        );
        assert_eq!(
            verdict(
                "imap.exemple.fr",
                &[
                    "mx.exemple.fr; none",
                    "mx.exemple.fr; dmarc=pass header.from=banque.fr",
                ],
                "a@banque.fr"
            ),
            SenderAuth::Unknown
        );
    }

    #[test]
    fn without_a_single_valid_from_the_sender_is_unknown() {
        let pass = ["mx.exemple.fr; dmarc=pass"];
        let t = trust("imap.exemple.fr");
        assert_eq!(
            SenderAuth::evaluate(&t, pass.iter().copied(), &[]),
            SenderAuth::Unknown
        );
        let two = [
            Correspondent::new("A", "a@banque.fr"),
            Correspondent::new("B", "b@banque.fr"),
        ];
        assert_eq!(
            SenderAuth::evaluate(&t, pass.iter().copied(), &two),
            SenderAuth::Unknown
        );
        assert_eq!(
            SenderAuth::evaluate(&t, pass.iter().copied(), &from("pas une adresse")),
            SenderAuth::Unknown
        );
        assert_eq!(
            SenderAuth::evaluate(&t, std::iter::empty(), &from("a@banque.fr")),
            SenderAuth::Unknown
        );
    }

    #[test]
    fn neutral_results_give_no_verdict() {
        for result in ["none", "neutral", "temperror", "permerror", "policy"] {
            assert_eq!(
                verdict(
                    "imap.exemple.fr",
                    &[&format!(
                        "mx.exemple.fr; dmarc={result}; dkim={result} header.d=banque.fr"
                    )],
                    "a@banque.fr"
                ),
                SenderAuth::Unknown,
                "{result}"
            );
        }
    }

    proptest::proptest! {
        #[test]
        fn parsing_never_panics(raw in "\\PC{0,400}") {
            let _ = AuthenticationResults::parse(&raw);
        }

        #[test]
        fn parsing_structured_noise_never_panics(
            raw in "[a-z.;=() \"\\\\/@\r\n\t]{0,400}"
        ) {
            let _ = AuthenticationResults::parse(&raw);
        }
    }
}
