//! Adaptateur de rendu (ADR-004, « A+ ») : MIME vers vue, assainissement
//! du HTML, document servi par `mailview://` avec sa CSP et l'agent.
//!
//! - [`mime`] : corps (meilleure alternative, charsets), pièces jointes ;
//! - [`sanitize`] : liste blanche `ammonia`, réécriture des images ;
//! - [`css`] : CSS des mails ;
//! - [`render`] : document complet, prêt à servir ;
//! - [`content_security_policy`] : CSP du document, qui n'autorise que
//!   l'agent ([`AGENT_JS`]) par son empreinte SHA-256, calculée à la
//!   compilation ([`AGENT_SHA256`]).
//!
//! Ce crate est une zone de relecture prioritaire : tout ce qu'il reçoit
//! est hostile.

pub mod css;
pub mod mime;
pub mod sanitize;

use norelly_domain::fetch_policy::{MimePart, PartPath};
use norelly_domain::reading::{ExternalLink, RemoteContent};

pub use mime::{Attachment, ParsedBody, PartContent, Source, parse_body, part_content};
use sanitize::{SanitizeOptions, sanitize_html};

include!(concat!(env!("OUT_DIR"), "/agent_hash.rs"));

/// Agent : seul script autorisé dans un document `mailview://`.
pub const AGENT_JS: &str = include_str!("agent.js");

/// Adresses des protocoles personnalisés, selon la plateforme : les
/// webviews Linux (WebKitGTK) les servent sous `scheme://localhost/`,
/// Windows (WebView2) sous `http://scheme.localhost/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Protocols {
    mailpart: String,
    remote_img: String,
    sources: String,
}

impl Protocols {
    /// `mailpart://localhost/`, `remote-img://localhost/`.
    #[must_use]
    pub fn custom_schemes() -> Self {
        Self {
            mailpart: "mailpart://localhost/".to_owned(),
            remote_img: "remote-img://localhost/".to_owned(),
            sources: "mailpart: remote-img:".to_owned(),
        }
    }

    /// `http://mailpart.localhost/`, `http://remote-img.localhost/`.
    #[must_use]
    pub fn localhost_http() -> Self {
        Self {
            mailpart: "http://mailpart.localhost/".to_owned(),
            remote_img: "http://remote-img.localhost/".to_owned(),
            sources: "http://mailpart.localhost http://remote-img.localhost".to_owned(),
        }
    }

    /// Forme de la plateforme courante.
    #[must_use]
    pub fn for_platform() -> Self {
        if cfg!(windows) {
            Self::localhost_http()
        } else {
            Self::custom_schemes()
        }
    }

    /// Adresse d'une partie d'un message.
    #[must_use]
    pub fn mailpart_url(&self, message: i64, path: &PartPath) -> String {
        format!("{}{message}/{path}", self.mailpart)
    }

    /// Adresse d'une image distante (index dans la liste du message).
    #[must_use]
    pub fn remote_image_url(&self, message: i64, index: usize) -> String {
        format!("{}{message}/{index}", self.remote_img)
    }
}

/// CSP du document d'un mail (en-tête HTTP de `mailview://`) : rien n'est
/// autorisé, sauf l'agent (empreinte), les styles en ligne et les images
/// servies par nos protocoles ou en ligne (`data:`).
#[must_use]
pub fn content_security_policy(protocols: &Protocols) -> String {
    format!(
        "default-src 'none'; script-src 'sha256-{AGENT_SHA256}'; style-src 'unsafe-inline'; \
         img-src {} data:; font-src 'none'; connect-src 'none'; media-src 'none'; \
         object-src 'none'; frame-src 'none'; child-src 'none'; worker-src 'none'; \
         manifest-src 'none'; base-uri 'none'; form-action 'none'",
        protocols.sources
    )
}

/// Échappe un texte pour le HTML.
#[must_use]
pub fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Texte brut vers HTML : échappé, retours à la ligne conservés, adresses
/// `http(s)` rendues cliquables (validées par le domaine).
#[must_use]
pub fn text_to_html(text: &str) -> String {
    let mut out = String::from("<div class=\"norelly-text\">");
    for (i, line) in text.lines().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        for (j, word) in line.split(' ').enumerate() {
            if j > 0 {
                out.push(' ');
            }
            let lower = word.to_ascii_lowercase();
            let link = (lower.starts_with("http://") || lower.starts_with("https://"))
                .then(|| word.trim_end_matches(['.', ',', ';', ')', '>', '"', '\'']))
                .and_then(|w| ExternalLink::parse(w).ok().map(|l| (w.len(), l)));
            match link {
                Some((len, link)) => {
                    out.push_str("<a href=\"");
                    out.push_str(&escape_html(link.as_str()));
                    out.push_str("\">");
                    out.push_str(&escape_html(word.get(..len).unwrap_or(word)));
                    out.push_str("</a>");
                    out.push_str(&escape_html(word.get(len..).unwrap_or_default()));
                }
                None => out.push_str(&escape_html(word)),
            }
        }
    }
    out.push_str("</div>");
    out
}

/// Styles de base du document (avant ceux du mail).
const BASE_STYLE: &str = "html{color-scheme:light}\
body{margin:0;padding:16px;font-family:system-ui,-apple-system,'Segoe UI',sans-serif;\
font-size:14px;line-height:1.5;color:#1f2328;background:#fff;overflow-wrap:anywhere}\
img{max-width:100%}\
.norelly-text{white-space:pre-wrap;font-family:inherit}\
blockquote{margin:0 0 0 8px;padding-left:8px;border-left:2px solid #d0d7de}";

/// Mode d'affichage effectivement employé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyMode {
    Html,
    Text,
    Empty,
}

/// Demande de rendu.
#[derive(Debug, Clone)]
pub struct RenderRequest<'a> {
    /// Identifiant local du message.
    pub message: i64,
    pub source: Source<'a>,
    /// Structure annoncée par le serveur (parties différées).
    pub structure: Option<&'a MimePart>,
    pub remote: RemoteContent,
    pub prefer_plain_text: bool,
    pub protocols: &'a Protocols,
}

/// Message rendu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    /// Document HTML complet (à servir avec [`content_security_policy`]).
    pub document: String,
    pub mode: BodyMode,
    /// Vrai si le message a une version HTML (choix « texte brut »
    /// pertinent).
    pub has_html: bool,
    pub remote_images: Vec<String>,
    pub blocked_images: usize,
    pub attachments: Vec<Attachment>,
}

/// Document complet autour d'un corps déjà assaini.
#[must_use]
pub fn document(body: &str) -> String {
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
         <meta name=\"referrer\" content=\"no-referrer\">\
         <style>{BASE_STYLE}</style><script>{AGENT_JS}</script></head>\
         <body>{body}</body></html>"
    )
}

/// Rend un message stocké.
#[must_use]
pub fn render(request: &RenderRequest<'_>) -> Rendered {
    let parsed = parse_body(&request.source, request.structure);
    let has_html = parsed.html.is_some();
    let (raw, mode) = match (&parsed.html, &parsed.text, request.prefer_plain_text) {
        (Some(html), None, _) | (Some(html), Some(_), false) => (html.clone(), BodyMode::Html),
        (_, Some(text), _) => (text_to_html(text), BodyMode::Text),
        (None, None, _) => (String::new(), BodyMode::Empty),
    };
    let sanitized = sanitize_html(
        &raw,
        &SanitizeOptions {
            message: request.message,
            remote: request.remote,
            content_ids: parsed.content_ids(),
            protocols: request.protocols.clone(),
        },
    );
    Rendered {
        document: document(&sanitized.html),
        mode,
        has_html,
        remote_images: sanitized.remote_images,
        blocked_images: sanitized.blocked_images,
        attachments: parsed.attachments,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_hash_matches_the_embedded_agent() {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(AGENT_JS.as_bytes());
        const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut expected = String::new();
        for chunk in digest.chunks(3) {
            let n = chunk.iter().fold(0u32, |acc, b| (acc << 8) | u32::from(*b))
                << (8 * (3 - chunk.len()));
            for i in 0..=chunk.len() {
                expected.push(char::from(B64[((n >> (18 - 6 * i)) & 0x3f) as usize]));
            }
            for _ in chunk.len()..3 {
                expected.push('=');
            }
        }
        assert_eq!(AGENT_SHA256, expected);
    }

    #[test]
    fn csp_allows_only_the_agent() {
        let csp = content_security_policy(&Protocols::custom_schemes());
        assert!(csp.starts_with("default-src 'none';"));
        assert!(csp.contains(&format!("script-src 'sha256-{AGENT_SHA256}';")));
        let script = csp
            .split(';')
            .find(|d| d.trim().starts_with("script-src"))
            .unwrap_or_default();
        assert!(!script.contains("unsafe"), "{script}");
        assert!(csp.contains("img-src mailpart: remote-img: data:;"));
        assert!(csp.contains("form-action 'none'"));
        assert!(csp.contains("base-uri 'none'"));
        let windows = content_security_policy(&Protocols::localhost_http());
        assert!(
            windows
                .contains("img-src http://mailpart.localhost http://remote-img.localhost data:;")
        );
    }

    #[test]
    fn document_embeds_exactly_the_hashed_agent() {
        let doc = document("<p>x</p>");
        assert_eq!(doc.matches("<script>").count(), 1);
        assert!(doc.contains(&format!("<script>{AGENT_JS}</script>")));
    }

    #[test]
    fn plain_text_is_escaped_and_linkified() {
        let html = text_to_html("<b>a</b> voir https://exemple.fr/x. et javascript:alert(1)");
        assert_eq!(
            html,
            "<div class=\"norelly-text\">&lt;b&gt;a&lt;/b&gt; voir \
             <a href=\"https://exemple.fr/x\">https://exemple.fr/x</a>. et javascript:alert(1)</div>"
        );
    }

    #[test]
    fn plain_text_preference_and_fallbacks() {
        let raw = b"Content-Type: multipart/alternative; boundary=b\r\n\r\n--b\r\n\
Content-Type: text/plain\r\n\r\ntexte\r\n--b\r\nContent-Type: text/html\r\n\r\n<p>html</p>\r\n--b--\r\n";
        let protocols = Protocols::custom_schemes();
        let mut request = RenderRequest {
            message: 1,
            source: Source::Full(raw),
            structure: None,
            remote: RemoteContent::Blocked,
            prefer_plain_text: false,
            protocols: &protocols,
        };
        let html = render(&request);
        assert_eq!((html.mode, html.has_html), (BodyMode::Html, true));
        assert!(html.document.contains("<p>html</p>"));
        request.prefer_plain_text = true;
        let text = render(&request);
        assert_eq!(text.mode, BodyMode::Text);
        assert!(text.document.contains("texte"));
        assert!(!text.document.contains("<p>html</p>"));
        request.source = Source::Full(b"");
        assert_eq!(render(&request).mode, BodyMode::Empty);
    }
}
