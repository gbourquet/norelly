//! Analyse des fichiers autoconfig (`config-v1.1.xml`, format Thunderbird).
//!
//! Le fichier est une donnée hostile : taille bornée, profondeur bornée,
//! nombre de serveurs borné, DTD refusée (donc aucune entité externe ni
//! expansion d'entités), seules les références de caractère et les cinq
//! entités prédéfinies sont admises. Un serveur aux valeurs invalides est
//! ignoré ; un document mal formé est refusé.

use norelly_domain::discovery::{
    DiscoveredServers, ServerCandidate, UsernameTemplate, is_password_authentication, socket_type,
};
use norelly_domain::endpoint::{Host, Protocol, ServerEndpoint};
use norelly_domain::ports::DiscoveryError;
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

/// Taille maximale d'un fichier autoconfig (les vrais font quelques Ko).
pub const MAX_XML_BYTES: usize = 128 * 1024;
/// Profondeur maximale d'imbrication.
const MAX_DEPTH: usize = 16;
/// Nombre maximal de serveurs retenus par protocole.
const MAX_SERVERS: usize = 16;
/// Nombre maximal de méthodes d'authentification par serveur.
const MAX_AUTH_METHODS: usize = 8;
/// Longueur maximale d'une valeur de champ.
const MAX_FIELD_LEN: usize = 512;

fn invalid(msg: impl Into<String>) -> DiscoveryError {
    DiscoveryError::Invalid(msg.into())
}

/// Champ de serveur en cours de lecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Hostname,
    Port,
    SocketType,
    Username,
    Authentication,
}

impl Field {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "hostname" => Self::Hostname,
            "port" => Self::Port,
            "socketType" => Self::SocketType,
            "username" => Self::Username,
            "authentication" => Self::Authentication,
            _ => return None,
        })
    }
}

/// Serveur en cours de lecture.
#[derive(Debug, Default)]
struct RawServer {
    protocol: Option<Protocol>,
    hostname: String,
    port: String,
    socket_type: String,
    username: Option<String>,
    authentication: Vec<String>,
}

impl RawServer {
    /// Ouvre un champ : une valeur par élément (la dernière l'emporte, sauf
    /// pour `authentication`, qui est répétable).
    fn open(&mut self, field: Field) {
        match field {
            Field::Hostname => self.hostname.clear(),
            Field::Port => self.port.clear(),
            Field::SocketType => self.socket_type.clear(),
            Field::Username => self.username = Some(String::new()),
            Field::Authentication => self.authentication.push(String::new()),
        }
    }

    /// Valeur du champ ouvert.
    fn value_mut(&mut self, field: Field) -> Option<&mut String> {
        match field {
            Field::Hostname => Some(&mut self.hostname),
            Field::Port => Some(&mut self.port),
            Field::SocketType => Some(&mut self.socket_type),
            Field::Username => self.username.as_mut(),
            Field::Authentication => self.authentication.last_mut(),
        }
    }

    /// Candidat si toutes les valeurs sont valides et utilisables.
    fn into_candidate(self, domain: &Host) -> Option<ServerCandidate> {
        let protocol = self.protocol?;
        let hostname = self
            .hostname
            .trim()
            .replace("%EMAILDOMAIN%", domain.as_str());
        let host = Host::parse(&hostname).ok()?;
        let port: u16 = self.port.trim().parse().ok()?;
        let security = socket_type(&self.socket_type)?;
        let endpoint = ServerEndpoint::new(host, port, security).ok()?;
        if !self.authentication.is_empty()
            && !self
                .authentication
                .iter()
                .any(|a| is_password_authentication(a))
        {
            return None;
        }
        let username = match self.username {
            Some(raw) => UsernameTemplate::parse(&raw).ok()?,
            None => UsernameTemplate::email_address(),
        };
        Some(ServerCandidate {
            protocol,
            endpoint,
            username,
        })
    }
}

/// Protocole d'un élément `incomingServer`/`outgoingServer`, `None` pour
/// les types ignorés (POP3, Exchange…).
fn server_protocol(element: &BytesStart<'_>) -> Result<Option<Protocol>, DiscoveryError> {
    let incoming = match element.local_name().as_ref() {
        "incomingServer" => true,
        "outgoingServer" => false,
        _ => return Ok(None),
    };
    let kind = element
        .try_get_attribute("type")
        .map_err(|e| invalid(e.to_string()))?
        .map(|a| a.value.into_owned())
        .unwrap_or_default();
    Ok(match (incoming, kind.as_str()) {
        (true, "imap") => Some(Protocol::Imap),
        (false, "smtp") => Some(Protocol::Smtp),
        _ => None,
    })
}

/// Analyse un fichier autoconfig. `domain` remplace `%EMAILDOMAIN%` dans
/// les noms d'hôte.
pub fn parse(xml: &[u8], domain: &Host) -> Result<DiscoveredServers, DiscoveryError> {
    if xml.len() > MAX_XML_BYTES {
        return Err(invalid("fichier trop volumineux"));
    }
    let text = std::str::from_utf8(xml).map_err(|_| invalid("encodage non UTF-8"))?;
    let mut reader = Reader::from_str(text);
    reader.config_mut().check_end_names = true;

    let mut out = DiscoveredServers::default();
    let mut path: Vec<String> = Vec::new();
    let mut seen_root = false;
    let mut server: Option<RawServer> = None;
    let mut field: Option<Field> = None;

    loop {
        let event = reader.read_event().map_err(|e| invalid(e.to_string()))?;
        match event {
            Event::DocType(_) => return Err(invalid("DTD refusée")),
            Event::Start(e) => {
                let name = e.local_name().as_ref().to_owned();
                if path.is_empty() {
                    if seen_root || name != "clientConfig" {
                        return Err(invalid("élément racine inattendu"));
                    }
                    seen_root = true;
                }
                path.push(name);
                if path.len() > MAX_DEPTH {
                    return Err(invalid("imbrication trop profonde"));
                }
                let in_provider = path.len() == 3
                    && path.first().is_some_and(|r| r == "clientConfig")
                    && path.get(1).is_some_and(|p| p == "emailProvider");
                if in_provider {
                    if let Some(protocol) = server_protocol(&e)? {
                        server = Some(RawServer {
                            protocol: Some(protocol),
                            ..RawServer::default()
                        });
                    }
                } else if path.len() == 4 && server.is_some() {
                    field = path.last().and_then(|n| Field::from_name(n));
                    if let (Some(f), Some(s)) = (field, server.as_mut()) {
                        if f == Field::Authentication && s.authentication.len() >= MAX_AUTH_METHODS
                        {
                            return Err(invalid("trop de méthodes d'authentification"));
                        }
                        s.open(f);
                    }
                }
            }
            Event::Empty(e) => {
                if path.is_empty() {
                    return Err(invalid("document vide"));
                }
                // `<incomingServer type="imap"/>` : serveur sans valeur, ignoré.
                let _ = e;
            }
            Event::Text(t) => append(&mut server, field, &t.xml10_content())?,
            Event::CData(t) => {
                append(&mut server, field, &t.into_inner())?;
            }
            Event::GeneralRef(r) => {
                let resolved = if r.is_char_ref() {
                    r.resolve_char_ref()
                        .map_err(|e| invalid(e.to_string()))?
                        .ok_or_else(|| invalid("référence de caractère invalide"))?
                } else {
                    match r.xml10_content().as_ref() {
                        "amp" => '&',
                        "lt" => '<',
                        "gt" => '>',
                        "quot" => '"',
                        "apos" => '\'',
                        _ => return Err(invalid("entité inconnue")),
                    }
                };
                append(&mut server, field, resolved.encode_utf8(&mut [0u8; 4]))?;
            }
            Event::End(_) => {
                let depth = path.len();
                path.pop();
                if depth == 4 {
                    field = None;
                } else if depth == 3
                    && let Some(s) = server.take()
                    && let Some(protocol) = s.protocol
                    && out.candidates(protocol).len() < MAX_SERVERS
                    && let Some(candidate) = s.into_candidate(domain)
                {
                    out.push(candidate);
                }
            }
            Event::Eof => break,
            Event::Decl(_) | Event::PI(_) | Event::Comment(_) => {}
        }
    }
    if !path.is_empty() || !seen_root {
        return Err(invalid("document incomplet"));
    }
    Ok(out)
}

/// Ajoute du texte au champ courant, en bornant sa longueur.
fn append(
    server: &mut Option<RawServer>,
    field: Option<Field>,
    text: &str,
) -> Result<(), DiscoveryError> {
    let (Some(s), Some(f)) = (server.as_mut(), field) else {
        return Ok(());
    };
    if let Some(value) = s.value_mut(f) {
        if value.len() + text.len() > MAX_FIELD_LEN {
            return Err(invalid("valeur trop longue"));
        }
        value.push_str(text);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use norelly_domain::endpoint::Security;

    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../fixtures/ispdb")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn domain(raw: &str) -> Host {
        Host::parse(raw).unwrap_or_else(|_| panic!("domaine"))
    }

    fn summary(servers: &[ServerCandidate]) -> Vec<String> {
        servers
            .iter()
            .map(|c| format!("{} {}", c.endpoint, c.username.as_str()))
            .collect()
    }

    #[test]
    fn provider_file_keeps_imap_and_smtp_only() {
        let Ok(out) = parse(&fixture("provider-full.xml"), &domain("example.com")) else {
            panic!("fichier valide refusé");
        };
        assert_eq!(
            summary(&out.imap),
            [
                "imap.example.com:143 (STARTTLS) %EMAILADDRESS%",
                "imap.example.com:993 (TLS) %EMAILADDRESS%"
            ]
        );
        assert_eq!(
            summary(&out.smtp),
            [
                "smtp.example.com:587 (STARTTLS) %EMAILADDRESS%",
                "smtp.example.com:465 (TLS) %EMAILADDRESS%"
            ]
        );
    }

    #[test]
    fn oauth_only_servers_are_skipped() {
        let Ok(out) = parse(
            &fixture("ispdb-oauth-and-password.xml"),
            &domain("bigmail.example"),
        ) else {
            panic!("fichier valide refusé");
        };
        assert_eq!(
            summary(&out.imap),
            ["imap.bigmail.example:993 (TLS) %EMAILADDRESS%"]
        );
        // Le serveur 587 n'accepte qu'OAuth2.
        assert_eq!(
            summary(&out.smtp),
            ["smtp.bigmail.example:465 (TLS) %EMAILADDRESS%"]
        );
        let Ok(none) = parse(&fixture("oauth-only.xml"), &domain("oauth.example")) else {
            panic!("fichier valide refusé");
        };
        assert!(none.is_empty());
    }

    #[test]
    fn domain_placeholder_in_hostname() {
        let Ok(out) = parse(
            &fixture("ispdb-domain-placeholder.xml"),
            &domain("client.fr"),
        ) else {
            panic!("fichier valide refusé");
        };
        assert_eq!(
            summary(&out.imap),
            ["mail.client.fr:993 (TLS) %EMAILLOCALPART%"]
        );
        assert_eq!(
            summary(&out.smtp),
            ["mail.client.fr:587 (STARTTLS) %EMAILLOCALPART%"]
        );
    }

    #[test]
    fn plaintext_servers_are_reported_as_plain() {
        let Ok(out) = parse(&fixture("plain-only.xml"), &domain("legacy.example")) else {
            panic!("fichier valide refusé");
        };
        assert!(
            out.imap
                .iter()
                .chain(&out.smtp)
                .all(|c| c.endpoint.security() == Security::Plain)
        );
        assert_eq!(out.imap.len(), 1);
    }

    #[test]
    fn hostile_values_produce_no_candidate() {
        let Ok(out) = parse(
            &fixture("hostile/injection-values.xml"),
            &domain("example.com"),
        ) else {
            panic!("document bien formé refusé");
        };
        assert!(out.is_empty(), "candidats : {:?}", summary(&out.imap));
    }

    #[test]
    fn hostile_documents_are_rejected_without_panic() {
        for name in [
            "hostile/billion-laughs.xml",
            "hostile/external-entity.xml",
            "hostile/truncated.xml",
            "hostile/mismatched-tags.xml",
            "hostile/binary-garbage.bin",
            "hostile/html-page.xml",
            "hostile/deep-nesting.xml",
            "hostile/oversized.xml",
        ] {
            let result = parse(&fixture(name), &domain("example.com"));
            assert!(
                matches!(result, Err(DiscoveryError::Invalid(_))),
                "{name} : {result:?}"
            );
        }
    }

    #[test]
    fn number_of_servers_is_bounded() {
        let Ok(out) = parse(&fixture("hostile/many-servers.xml"), &domain("example.com")) else {
            panic!("document bien formé refusé");
        };
        assert_eq!(out.imap.len(), MAX_SERVERS);
    }

    #[test]
    fn entities_and_cdata_are_decoded() {
        let xml = br#"<clientConfig><emailProvider><incomingServer type="imap"><hostname>imap.&#101;xample.com</hostname><port><![CDATA[993]]></port><socketType>SSL</socketType><username>a&amp;b</username></incomingServer></emailProvider></clientConfig>"#;
        let Ok(out) = parse(xml, &domain("example.com")) else {
            panic!("document valide refusé");
        };
        assert_eq!(summary(&out.imap), ["imap.example.com:993 (TLS) a&b"]);
    }

    #[test]
    fn servers_outside_email_provider_are_ignored() {
        let xml = br#"<clientConfig><incomingServer type="imap"><hostname>imap.example.com</hostname><port>993</port><socketType>SSL</socketType></incomingServer><emailProvider/></clientConfig>"#;
        assert!(parse(xml, &domain("example.com")).is_ok_and(|o| o.is_empty()));
    }

    #[test]
    fn missing_username_defaults_to_address() {
        let xml = br#"<clientConfig><emailProvider><outgoingServer type="smtp"><hostname>smtp.example.com</hostname><port>465</port><socketType>SSL</socketType></outgoingServer></emailProvider></clientConfig>"#;
        let Ok(out) = parse(xml, &domain("example.com")) else {
            panic!("document valide refusé");
        };
        assert_eq!(
            summary(&out.smtp),
            ["smtp.example.com:465 (TLS) %EMAILADDRESS%"]
        );
    }

    #[test]
    fn duplicated_fields_keep_the_last_value() {
        let xml = br#"<clientConfig><emailProvider><incomingServer type="imap"><hostname>a.example.com</hostname><hostname>b.example.com</hostname><port>993</port><socketType>SSL</socketType></incomingServer></emailProvider></clientConfig>"#;
        let Ok(out) = parse(xml, &domain("example.com")) else {
            panic!("document valide refusé");
        };
        assert_eq!(
            summary(&out.imap),
            ["b.example.com:993 (TLS) %EMAILADDRESS%"]
        );
    }

    #[test]
    fn arbitrary_bytes_never_panic() {
        // Petit fuzzing déterministe : préfixes et mutations d'un fichier
        // valide.
        let base = fixture("provider-full.xml");
        let d = domain("example.com");
        for cut in (0..base.len()).step_by(7) {
            let _ = parse(base.get(..cut).unwrap_or_default(), &d);
        }
        let mut state: u32 = 0x1234_5678;
        for _ in 0..500 {
            let mut doc = base.clone();
            for _ in 0..8 {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let len = doc.len().max(1);
                if let Some(b) = doc.get_mut(state as usize % len) {
                    *b = (state >> 8) as u8;
                }
            }
            let _ = parse(&doc, &d);
        }
    }
}
