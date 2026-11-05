//! MIME vers vue (`mail-parser`) : choix du corps (HTML ou texte, meilleure
//! alternative), charsets décodés, pièces jointes listées avec leur chemin
//! de partie IMAP (`1.2`), qui sert d'identifiant stable que le message
//! soit stocké complet ou par parties.
//!
//! Un message partiel est stocké en en-têtes + entités MIME téléchargées
//! (en-têtes de partie + contenu, voir l'adaptateur IMAP) ; les parties
//! différées sont listées d'après la structure annoncée par le serveur.

use std::collections::HashMap;

use mail_parser::{Message, MessageParser, MessagePart, MimeHeaders, PartType};
use norelly_domain::fetch_policy::{
    MAX_MIME_DEPTH, MAX_MIME_PARTS, MimePart, PartPath, attachment_name, mime_token,
};

/// Taille maximale d'un message analysé (au-delà : seuls les en-têtes
/// sont exploités).
pub const MAX_PARSE_LEN: usize = 64 * 1024 * 1024;
/// Taille maximale du corps HTML ou texte conservé, en octets.
pub const MAX_BODY_LEN: usize = 8 * 1024 * 1024;

/// Message tel que stocké localement.
#[derive(Debug, Clone)]
pub enum Source<'a> {
    /// Message brut complet.
    Full(&'a [u8]),
    /// En-têtes et entités MIME téléchargées ; parties différées.
    Partial {
        header: &'a [u8],
        parts: Vec<(PartPath, &'a [u8])>,
        deferred: Vec<PartPath>,
    },
}

/// Pièce jointe listée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    /// Chemin de partie IMAP (identifiant).
    pub path: PartPath,
    /// Nom annoncé, assaini pour l'affichage.
    pub name: String,
    /// Type MIME assaini (`application/pdf`).
    pub media_type: String,
    /// Taille décodée (ou annoncée pour une partie différée), en octets.
    pub size: u64,
    /// `Content-ID` normalisé (sans chevrons, minuscules).
    pub content_id: Option<String>,
    /// Image en ligne référencée par le HTML (`cid:`) : affichée dans le
    /// corps, pas dans la liste.
    pub inline: bool,
    /// Faux pour une partie différée (à télécharger).
    pub available: bool,
}

/// Corps et pièces jointes d'un message.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedBody {
    pub html: Option<String>,
    pub text: Option<String>,
    pub attachments: Vec<Attachment>,
}

impl ParsedBody {
    /// `Content-ID` → partie, pour la réécriture des `cid:`.
    #[must_use]
    pub fn content_ids(&self) -> HashMap<String, PartPath> {
        self.attachments
            .iter()
            .filter_map(|a| a.content_id.clone().map(|c| (c, a.path.clone())))
            .collect()
    }
}

/// Contenu décodé d'une partie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartContent {
    pub bytes: Vec<u8>,
    pub media_type: String,
    pub name: String,
}

fn parse(raw: &[u8]) -> Option<Message<'_>> {
    MessageParser::default().parse(raw.get(..raw.len().min(MAX_PARSE_LEN))?)
}

fn normalize_cid(raw: &str) -> Option<String> {
    let cid = raw
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .to_lowercase();
    (!cid.is_empty() && cid.len() <= 256 && !cid.chars().any(char::is_control)).then_some(cid)
}

pub(crate) fn media_type(part: &MessagePart<'_>) -> String {
    match part.content_type() {
        Some(ct) => format!(
            "{}/{}",
            mime_token(ct.ctype()),
            mime_token(ct.subtype().unwrap_or("octet-stream"))
        ),
        None if part.is_text() => "text/plain".to_owned(),
        None => "application/octet-stream".to_owned(),
    }
}

fn truncate(text: &str) -> String {
    let mut end = text.len().min(MAX_BODY_LEN);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).unwrap_or_default().to_owned()
}

/// Feuilles d'un message analysé : identifiant de partie `mail-parser` →
/// chemin IMAP. Un message encapsulé est une feuille.
fn leaf_paths(message: &Message<'_>) -> Vec<(u32, PartPath)> {
    fn walk(message: &Message<'_>, id: u32, path: Vec<u32>, out: &mut Vec<(u32, PartPath)>) {
        if out.len() >= MAX_MIME_PARTS || path.len() > MAX_MIME_DEPTH {
            return;
        }
        let Some(part) = message.part(id) else {
            return;
        };
        match &part.body {
            PartType::Multipart(children) => {
                for (i, child) in children.iter().enumerate() {
                    let mut p = path.clone();
                    p.push(u32::try_from(i + 1).unwrap_or(u32::MAX));
                    walk(message, *child, p, out);
                }
            }
            _ => {
                let path = if path.is_empty() { vec![1] } else { path };
                if let Some(p) = PartPath::new(path) {
                    out.push((id, p));
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(message, 0, Vec::new(), &mut out);
    out
}

fn attachment_of(part: &MessagePart<'_>, path: PartPath) -> Attachment {
    Attachment {
        path,
        name: part
            .attachment_name()
            .map(attachment_name)
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| norelly_domain::reading::DEFAULT_FILE_NAME.to_owned()),
        media_type: media_type(part),
        size: u64::try_from(part.len()).unwrap_or(u64::MAX),
        content_id: part.content_id().and_then(normalize_cid),
        inline: false,
        available: true,
    }
}

fn mark_inline(body: &mut ParsedBody) {
    let html = body.html.as_deref().unwrap_or_default().to_lowercase();
    for a in &mut body.attachments {
        a.inline = a.content_id.as_ref().is_some_and(|cid| {
            a.media_type.starts_with("image/") && html.contains(&format!("cid:{cid}"))
        });
    }
}

fn parse_full(raw: &[u8]) -> ParsedBody {
    let Some(message) = parse(raw) else {
        return ParsedBody::default();
    };
    let paths: HashMap<u32, PartPath> = leaf_paths(&message).into_iter().collect();
    let has_html = message
        .html_body
        .iter()
        .any(|id| message.part(*id).is_some_and(MessagePart::is_text_html));
    let html = has_html.then(|| {
        let mut out = String::new();
        for id in &message.html_body {
            match message.part(*id).map(|p| &p.body) {
                Some(PartType::Html(h)) => out.push_str(h),
                Some(PartType::Text(t)) => out.push_str(&crate::text_to_html(t)),
                _ => {}
            }
        }
        truncate(&out)
    });
    let text = (0..message.text_body_count())
        .filter_map(|i| message.body_text(i))
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut body = ParsedBody {
        html,
        text: (!text.is_empty()).then(|| truncate(&text)),
        attachments: message
            .attachments
            .iter()
            .filter_map(|id| {
                let part = message.part(*id)?;
                Some(attachment_of(part, paths.get(id)?.clone()))
            })
            .collect(),
    };
    mark_inline(&mut body);
    body
}

fn deferred_attachment(path: &PartPath, structure: Option<&MimePart>) -> Attachment {
    let leaf = structure.and_then(|s| {
        s.leaves()
            .into_iter()
            .find(|l| &l.path == path)
            .map(|l| l.part.clone())
    });
    let (media_type, size, name) = match leaf {
        Some(MimePart::Leaf {
            media_type,
            subtype,
            size,
            filename,
            ..
        }) => (format!("{media_type}/{subtype}"), size, filename),
        Some(MimePart::Message { size }) => ("message/rfc822".to_owned(), size, None),
        _ => ("application/octet-stream".to_owned(), 0, None),
    };
    Attachment {
        path: path.clone(),
        name: name
            .as_deref()
            .map(attachment_name)
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| norelly_domain::reading::DEFAULT_FILE_NAME.to_owned()),
        media_type,
        size: u64::from(size),
        content_id: None,
        inline: false,
        available: false,
    }
}

/// Entité MIME téléchargée seule : en-têtes vides (partie unique d'un
/// message non multipart) remplacés par ceux du message.
fn entity_bytes(header: &[u8], entity: &[u8]) -> Vec<u8> {
    let blank = entity.starts_with(b"\r\n") || entity.starts_with(b"\n");
    if blank {
        let mut out = header.to_vec();
        out.extend_from_slice(
            entity
                .strip_prefix(b"\r\n")
                .or_else(|| entity.strip_prefix(b"\n"))
                .unwrap_or(entity),
        );
        out
    } else {
        entity.to_vec()
    }
}

fn parse_partial(
    header: &[u8],
    parts: &[(PartPath, &[u8])],
    deferred: &[PartPath],
    structure: Option<&MimePart>,
) -> ParsedBody {
    let leaf_of = |path: &PartPath| {
        structure.and_then(|s| {
            s.leaves()
                .into_iter()
                .find(|l| &l.path == path)
                .map(|l| l.part.is_body_text())
        })
    };
    let mut body = ParsedBody::default();
    let mut html = String::new();
    let mut text = String::new();
    for (path, entity) in parts {
        let bytes = entity_bytes(header, entity);
        let Some(message) = parse(&bytes) else {
            continue;
        };
        let Some(root) = message.part(0) else {
            continue;
        };
        let is_body = leaf_of(path).unwrap_or_else(|| {
            root.is_text()
                && !root
                    .content_disposition()
                    .is_some_and(|d| d.ctype().eq_ignore_ascii_case("attachment"))
        });
        match (&root.body, is_body) {
            (PartType::Html(h), true) => html.push_str(h),
            (PartType::Text(t), true) => {
                if !text.is_empty() {
                    text.push_str("\n\n");
                }
                text.push_str(t);
            }
            _ => body.attachments.push(attachment_of(root, path.clone())),
        }
    }
    body.attachments
        .extend(deferred.iter().map(|p| deferred_attachment(p, structure)));
    body.html = (!html.is_empty()).then(|| truncate(&html));
    body.text = (!text.is_empty()).then(|| truncate(&text));
    mark_inline(&mut body);
    body
}

/// Corps et pièces jointes d'un message stocké.
#[must_use]
pub fn parse_body(source: &Source<'_>, structure: Option<&MimePart>) -> ParsedBody {
    match source {
        Source::Full(raw) => parse_full(raw),
        Source::Partial {
            header,
            parts,
            deferred,
        } => parse_partial(header, parts, deferred, structure),
    }
}

/// Contenu décodé d'une partie, `None` si elle n'existe pas (ou n'est pas
/// téléchargée).
#[must_use]
pub fn part_content(source: &Source<'_>, path: &PartPath) -> Option<PartContent> {
    let content = |part: &MessagePart<'_>| PartContent {
        bytes: match &part.body {
            PartType::Message(m) => m.raw_message().to_vec(),
            _ => part.contents().to_vec(),
        },
        media_type: media_type(part),
        name: part
            .attachment_name()
            .map(attachment_name)
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| norelly_domain::reading::DEFAULT_FILE_NAME.to_owned()),
    };
    match source {
        Source::Full(raw) => {
            let message = parse(raw)?;
            let (id, _) = leaf_paths(&message).into_iter().find(|(_, p)| p == path)?;
            message.part(id).map(content)
        }
        Source::Partial { header, parts, .. } => {
            let (_, entity) = parts.iter().find(|(p, _)| p == path)?;
            let bytes = entity_bytes(header, entity);
            let message = parse(&bytes)?;
            message.part(0).map(content)
        }
    }
}

#[cfg(test)]
mod tests {
    use norelly_domain::fetch_policy::Disposition;

    use super::*;

    const MIXED: &[u8] = b"From: Alice <alice@example.com>\r\n\
Subject: =?ISO-8859-1?Q?R=E9union?=\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"ext\"\r\n\
\r\n\
--ext\r\n\
Content-Type: multipart/related; boundary=\"rel\"\r\n\
\r\n\
--rel\r\n\
Content-Type: multipart/alternative; boundary=\"alt\"\r\n\
\r\n\
--alt\r\n\
Content-Type: text/plain; charset=iso-8859-1\r\n\
Content-Transfer-Encoding: quoted-printable\r\n\
\r\n\
Caf=E9 cr=E8me\r\n\
--alt\r\n\
Content-Type: text/html; charset=windows-1252\r\n\
Content-Transfer-Encoding: quoted-printable\r\n\
\r\n\
<p>Caf=E9 =80 <img src=3D\"cid:logo@x\"></p>\r\n\
--alt--\r\n\
--rel\r\n\
Content-Type: image/png\r\n\
Content-ID: <logo@x>\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
iVBORw0KGgo=\r\n\
--rel--\r\n\
--ext\r\n\
Content-Type: application/pdf; name=\"../../rapport.pdf\"\r\n\
Content-Disposition: attachment; filename=\"../../rapport.pdf\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
JVBERi0xLjQK\r\n\
--ext--\r\n";

    fn path(raw: &str) -> PartPath {
        PartPath::parse(raw).unwrap_or_else(|| panic!("{raw}"))
    }

    #[test]
    fn best_alternative_and_charsets() {
        let body = parse_body(&Source::Full(MIXED), None);
        let html = body.html.clone().unwrap_or_default();
        assert!(html.contains("Café €"), "{html}");
        assert_eq!(body.text.as_deref().map(str::trim), Some("Café crème"));
        let list: Vec<(String, &str, bool)> = body
            .attachments
            .iter()
            .map(|a| (a.path.to_string(), a.media_type.as_str(), a.inline))
            .collect();
        assert_eq!(
            list,
            [
                ("1.2".to_owned(), "image/png", true),
                ("2".to_owned(), "application/pdf", false)
            ]
        );
        assert_eq!(
            body.attachments[1].name, "../../rapport.pdf",
            "affichage seulement"
        );
        assert_eq!(body.content_ids().get("logo@x"), Some(&path("1.2")));
    }

    #[test]
    fn part_content_is_decoded() {
        let pdf = part_content(&Source::Full(MIXED), &path("2"));
        assert_eq!(pdf.map(|p| p.bytes), Some(b"%PDF-1.4\n".to_vec()));
        assert!(part_content(&Source::Full(MIXED), &path("9")).is_none());
    }

    #[test]
    fn single_part_and_garbage() {
        let plain = b"Subject: x\r\n\r\nBonjour\r\n";
        let body = parse_body(&Source::Full(plain), None);
        assert!(body.html.is_none());
        assert_eq!(body.text.as_deref().map(str::trim), Some("Bonjour"));
        let garbage: Vec<u8> = (0..=255u8).cycle().take(10_000).collect();
        let _ = parse_body(&Source::Full(&garbage), None);
        let _ = parse_body(&Source::Full(b""), None);
    }

    #[test]
    fn partial_message_uses_entities_and_lists_deferred_parts() {
        let header = b"From: a@b.c\r\nContent-Type: multipart/mixed; boundary=x\r\n\r\n";
        let text = b"Content-Type: text/plain; charset=utf-8\r\n\r\nCorps \xc3\xa9t\xc3\xa9\r\n";
        let structure = MimePart::Multipart {
            subtype: "mixed".into(),
            parts: vec![
                MimePart::Leaf {
                    media_type: "text".into(),
                    subtype: "plain".into(),
                    size: 10,
                    disposition: Disposition::Unspecified,
                    filename: None,
                },
                MimePart::Leaf {
                    media_type: "video".into(),
                    subtype: "mp4".into(),
                    size: 90_000_000,
                    disposition: Disposition::Attachment,
                    filename: Some("film.mp4".into()),
                },
            ],
        };
        let source = Source::Partial {
            header,
            parts: vec![(path("1"), text.as_slice())],
            deferred: vec![path("2")],
        };
        let body = parse_body(&source, Some(&structure));
        assert_eq!(body.text.as_deref().map(str::trim), Some("Corps été"));
        assert_eq!(body.attachments.len(), 1);
        let film = &body.attachments[0];
        assert_eq!(
            (film.name.as_str(), film.size, film.available),
            ("film.mp4", 90_000_000, false)
        );
        assert!(part_content(&source, &path("2")).is_none());
    }
}
