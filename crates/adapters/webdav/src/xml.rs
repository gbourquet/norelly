//! Corps XML des requêtes WebDAV et CardDAV, et lecture des réponses
//! `207 Multi-Status` (RFC 4918, RFC 6352).
//!
//! Une réponse est une donnée hostile : taille bornée avant lecture (par
//! l'appelant), profondeur bornée ([`MAX_XML_DEPTH`]), nombre de réponses
//! borné, DTD refusée (donc aucune entité externe ni expansion
//! d'entités), seules les références de caractère et les cinq entités
//! prédéfinies sont admises. Les éléments sont reconnus par leur nom
//! local, dans les espaces `DAV:` et CardDAV comme ailleurs.

use norelly_domain::carddav::{Href, MAX_XML_DEPTH};
use norelly_domain::ports::CardDavError;
use quick_xml::Reader;
use quick_xml::events::Event;

/// Découverte : principal, dossier des carnets, type et nom.
pub const DISCOVERY_PROPFIND: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
<d:propfind xmlns:d=\"DAV:\" xmlns:card=\"urn:ietf:params:xml:ns:carddav\"><d:prop>\
<d:current-user-principal/><card:addressbook-home-set/><d:resourcetype/><d:displayname/>\
</d:prop></d:propfind>";

/// Liste d'un carnet : type et ETag de chaque ressource.
pub const ENTRIES_PROPFIND: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
<d:propfind xmlns:d=\"DAV:\"><d:prop><d:resourcetype/><d:getetag/></d:prop></d:propfind>";

/// Échappement XML d'un texte.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `REPORT addressbook-multiget` des fiches `hrefs`.
#[must_use]
pub fn multiget(hrefs: &[Href]) -> String {
    let mut body = String::from(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
<card:addressbook-multiget xmlns:d=\"DAV:\" xmlns:card=\"urn:ietf:params:xml:ns:carddav\">\
<d:prop><d:getetag/><card:address-data/></d:prop>",
    );
    for href in hrefs {
        body.push_str("<d:href>");
        body.push_str(&escape(href.as_str()));
        body.push_str("</d:href>");
    }
    body.push_str("</card:addressbook-multiget>");
    body
}

/// Une réponse d'un `207 Multi-Status`, propriétés des `propstat` en
/// succès seulement.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DavResponse {
    pub href: String,
    /// Statut de la réponse elle-même (`404` d'une fiche absente), s'il
    /// est donné.
    pub status: Option<u16>,
    pub collection: bool,
    pub addressbook: bool,
    pub displayname: Option<String>,
    pub etag: Option<String>,
    pub address_data: Option<String>,
    pub current_user_principal: Option<String>,
    pub addressbook_home_set: Option<String>,
}

/// Lit un `207 Multi-Status` : au plus `max_responses` réponses
/// (`TooLarge` au-delà).
pub fn parse_multistatus(
    body: &[u8],
    max_responses: usize,
) -> Result<Vec<DavResponse>, CardDavError> {
    let text = std::str::from_utf8(body).map_err(|_| invalid("encodage non UTF-8"))?;
    let mut reader = Reader::from_str(text);
    reader.config_mut().check_end_names = true;
    let mut path: Vec<String> = Vec::new();
    let mut out = Vec::new();
    let mut seen_root = false;
    let mut response: Option<DavResponse> = None;
    // Propriétés et statut du `propstat` en cours.
    let mut propstat: Option<(DavResponse, Option<u16>)> = None;
    let mut text_buf = String::new();
    loop {
        let event = reader.read_event().map_err(|e| invalid(&e.to_string()))?;
        match event {
            Event::DocType(_) => return Err(invalid("DTD refusée")),
            Event::Start(e) => {
                let name = e.local_name().as_ref().to_owned();
                if path.is_empty() {
                    if seen_root {
                        return Err(invalid("plusieurs éléments racines"));
                    }
                    seen_root = true;
                }
                open(
                    &name,
                    &path,
                    &mut response,
                    &mut propstat,
                    max_responses,
                    &out,
                )?;
                path.push(name);
                if path.len() > MAX_XML_DEPTH {
                    return Err(invalid("imbrication trop profonde"));
                }
                text_buf.clear();
            }
            Event::Empty(e) => {
                let name = e.local_name().as_ref().to_owned();
                if path.is_empty() {
                    return Err(invalid("document vide"));
                }
                if path.len() + 1 > MAX_XML_DEPTH {
                    return Err(invalid("imbrication trop profonde"));
                }
                if path.last().is_some_and(|p| p == "resourcetype")
                    && let Some((props, _)) = propstat.as_mut()
                {
                    match name.as_str() {
                        "collection" => props.collection = true,
                        "addressbook" => props.addressbook = true,
                        _ => {}
                    }
                }
            }
            Event::Text(t) => {
                let content = t.xml10_content();
                text_buf.push_str(&content);
            }
            Event::CData(t) => {
                let content = t.into_inner();
                text_buf.push_str(&content);
            }
            Event::GeneralRef(r) => {
                let resolved = if r.is_char_ref() {
                    r.resolve_char_ref()
                        .map_err(|e| invalid(&e.to_string()))?
                        .ok_or_else(|| invalid("référence de caractère invalide"))?
                } else {
                    let name = r.xml10_content();
                    match name.as_ref() {
                        "amp" => '&',
                        "lt" => '<',
                        "gt" => '>',
                        "quot" => '"',
                        "apos" => '\'',
                        _ => return Err(invalid("entité inconnue")),
                    }
                };
                text_buf.push(resolved);
            }
            Event::End(_) => {
                let Some(name) = path.pop() else {
                    return Err(invalid("fin d'élément inattendue"));
                };
                close(
                    &name,
                    &path,
                    &text_buf,
                    &mut response,
                    &mut propstat,
                    &mut out,
                );
                text_buf.clear();
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

fn invalid(message: &str) -> CardDavError {
    CardDavError::Protocol(message.to_owned())
}

/// Début d'un élément sous `path`.
fn open(
    name: &str,
    path: &[String],
    response: &mut Option<DavResponse>,
    propstat: &mut Option<(DavResponse, Option<u16>)>,
    max_responses: usize,
    out: &[DavResponse],
) -> Result<(), CardDavError> {
    match name {
        "response" if path.len() == 1 => {
            if out.len() >= max_responses {
                return Err(CardDavError::TooLarge);
            }
            *response = Some(DavResponse::default());
        }
        "propstat" if response.is_some() => {
            *propstat = Some((DavResponse::default(), None));
        }
        _ => {}
    }
    Ok(())
}

/// Code d'une ligne de statut (`HTTP/1.1 200 OK`).
fn status_code(line: &str) -> Option<u16> {
    line.split_whitespace().nth(1)?.parse().ok()
}

/// Fin de l'élément `name`, dont le parent est le dernier de `path`.
fn close(
    name: &str,
    path: &[String],
    text: &str,
    response: &mut Option<DavResponse>,
    propstat: &mut Option<(DavResponse, Option<u16>)>,
    out: &mut Vec<DavResponse>,
) {
    let parent = path.last().map(String::as_str);
    let text = text.trim();
    if let Some((props, status)) = propstat.as_mut() {
        match (name, parent) {
            ("status", Some("propstat")) => *status = status_code(text),
            ("displayname", _) => props.displayname = Some(text.to_owned()),
            ("getetag", _) => props.etag = Some(text.to_owned()),
            ("address-data", _) => props.address_data = Some(text.to_owned()),
            ("href", Some("current-user-principal")) => {
                props.current_user_principal = Some(text.to_owned());
            }
            ("href", Some("addressbook-home-set")) => {
                props.addressbook_home_set = Some(text.to_owned());
            }
            ("propstat", _) => {
                if let (Some((props, status)), Some(r)) = (propstat.take(), response.as_mut())
                    && status.is_none_or(|s| (200..300).contains(&s))
                {
                    merge(r, props);
                }
            }
            _ => {}
        }
        return;
    }
    let Some(r) = response.as_mut() else {
        return;
    };
    match (name, parent) {
        ("href", Some("response")) => r.href = text.to_owned(),
        ("status", Some("response")) => r.status = status_code(text),
        ("response", _) => {
            if let Some(r) = response.take() {
                out.push(r);
            }
        }
        _ => {}
    }
}

/// Propriétés d'un `propstat` en succès versées dans la réponse.
fn merge(response: &mut DavResponse, props: DavResponse) {
    response.collection |= props.collection;
    response.addressbook |= props.addressbook;
    for (target, value) in [
        (&mut response.displayname, props.displayname),
        (&mut response.etag, props.etag),
        (&mut response.address_data, props.address_data),
        (
            &mut response.current_user_principal,
            props.current_user_principal,
        ),
        (
            &mut response.addressbook_home_set,
            props.addressbook_home_set,
        ),
    ] {
        if value.is_some() {
            *target = value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(xml: &str) -> Result<Vec<DavResponse>, CardDavError> {
        parse_multistatus(xml.as_bytes(), 100)
    }

    const DISCOVERY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<multistatus xmlns="DAV:" xmlns:C="urn:ietf:params:xml:ns:carddav">
  <response>
    <href>/dav/</href>
    <propstat>
      <prop>
        <current-user-principal><href>/dav/principals/alice/</href></current-user-principal>
        <resourcetype><collection/></resourcetype>
      </prop>
      <status>HTTP/1.1 200 OK</status>
    </propstat>
    <propstat>
      <prop><C:addressbook-home-set/><displayname/></prop>
      <status>HTTP/1.1 404 Not Found</status>
    </propstat>
  </response>
</multistatus>"#;

    #[test]
    fn discovery_properties_are_read() {
        let responses = parse(DISCOVERY).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            responses,
            [DavResponse {
                href: "/dav/".into(),
                collection: true,
                current_user_principal: Some("/dav/principals/alice/".into()),
                ..DavResponse::default()
            }]
        );
    }

    #[test]
    fn books_cards_and_missing_resources_are_read() {
        let xml = r#"<d:multistatus xmlns:d="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav">
<d:response><d:href>/b/contacts/</d:href><d:propstat><d:prop>
<d:resourcetype><d:collection/><card:addressbook/></d:resourcetype>
<d:displayname>Mes &amp; contacts &#x263A;</d:displayname></d:prop>
<d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
<d:response><d:href>/b/contacts/a.vcf</d:href><d:propstat><d:prop>
<d:getetag>"1"</d:getetag><card:address-data><![CDATA[BEGIN:VCARD
END:VCARD]]></card:address-data></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
<d:response><d:href>/b/contacts/x.vcf</d:href><d:status>HTTP/1.1 404 Not Found</d:status></d:response>
</d:multistatus>"#;
        let responses = parse(xml).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(responses.len(), 3);
        assert!(responses[0].addressbook && responses[0].collection);
        assert_eq!(
            responses[0].displayname.as_deref(),
            Some("Mes & contacts ☺")
        );
        assert_eq!(responses[1].etag.as_deref(), Some("\"1\""));
        assert_eq!(
            responses[1].address_data.as_deref(),
            Some("BEGIN:VCARD\nEND:VCARD")
        );
        assert_eq!(responses[2].status, Some(404));
    }

    #[test]
    fn hostile_documents_are_refused() {
        // Entité externe (XXE) : la DTD est refusée.
        let xxe = r#"<?xml version="1.0"?><!DOCTYPE m [<!ENTITY x SYSTEM "file:///etc/passwd">]>
<multistatus xmlns="DAV:"><response><href>&x;</href></response></multistatus>"#;
        assert!(parse(xxe).is_err());
        // Entité inconnue.
        assert!(
            parse(
                r#"<multistatus xmlns="DAV:"><response><href>&x;</href></response></multistatus>"#
            )
            .is_err()
        );
        // Profondeur.
        let deep = format!(
            "<multistatus xmlns=\"DAV:\">{}{}</multistatus>",
            "<a>".repeat(MAX_XML_DEPTH),
            "</a>".repeat(MAX_XML_DEPTH)
        );
        assert!(parse(&deep).is_err());
        // Mal formé.
        assert!(parse("<multistatus><response></multistatus>").is_err());
        assert!(parse("pas du xml").is_err());
        assert!(parse_multistatus(b"\xff\xfe", 10).is_err());
    }

    #[test]
    fn the_number_of_responses_is_bounded() {
        let many = format!(
            "<multistatus xmlns=\"DAV:\">{}</multistatus>",
            "<response><href>/a</href></response>".repeat(3)
        );
        assert_eq!(
            parse_multistatus(many.as_bytes(), 3).map(|r| r.len()),
            Ok(3)
        );
        assert_eq!(
            parse_multistatus(many.as_bytes(), 2),
            Err(CardDavError::TooLarge)
        );
    }

    #[test]
    fn multiget_hrefs_are_escaped() {
        let href = Href::parse("/b/a&b.vcf").unwrap_or_else(|e| panic!("{e}"));
        let body = multiget(&[href]);
        assert!(body.contains("<d:href>/b/a&amp;b.vcf</d:href>"));
        assert!(body.contains("addressbook-multiget"));
    }
}
