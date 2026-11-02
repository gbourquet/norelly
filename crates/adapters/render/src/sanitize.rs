//! Assainissement du HTML d'un mail par liste blanche (`ammonia`,
//! ADR-004) :
//! - balises et attributs de mise en forme seulement : ni script, ni
//!   formulaire, ni cadre, ni objet, ni SVG, ni `<base>`, `<meta>`,
//!   `<link>` ;
//! - liens : `http`, `https`, `mailto` seulement, `rel="noopener
//!   noreferrer"`, sans cible ;
//! - images : `cid:` réécrites vers `mailpart://`, distantes vers
//!   `remote-img://` (index dans la liste des adresses du message) si
//!   l'expéditeur est autorisé, sinon remplacées par une image vide et
//!   comptées ; `data:image/…` en base64 conservées ;
//! - CSS des attributs `style` et des balises `<style>` assaini
//!   ([`crate::css`]).

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use norelly_domain::fetch_policy::PartPath;
use norelly_domain::reading::{ExternalLink, RemoteContent};

use crate::Protocols;
use crate::css::sanitize_css;

/// Nombre maximal d'images distantes servies par message.
pub const MAX_REMOTE_IMAGES: usize = 200;
/// Longueur maximale d'une adresse d'image distante.
pub const MAX_REMOTE_URL_LEN: usize = 2048;

/// Image vide (GIF transparent 1×1) qui remplace une image bloquée.
pub const BLOCKED_IMAGE: &str =
    "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";

const TAGS: &[&str] = &[
    "a",
    "abbr",
    "acronym",
    "address",
    "b",
    "bdi",
    "bdo",
    "big",
    "blockquote",
    "br",
    "caption",
    "center",
    "cite",
    "code",
    "col",
    "colgroup",
    "dd",
    "del",
    "details",
    "dfn",
    "div",
    "dl",
    "dt",
    "em",
    "figcaption",
    "figure",
    "font",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "i",
    "img",
    "ins",
    "kbd",
    "li",
    "mark",
    "ol",
    "p",
    "pre",
    "q",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "small",
    "span",
    "strike",
    "strong",
    "style",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "tt",
    "u",
    "ul",
    "var",
    "wbr",
];

const GENERIC_ATTRIBUTES: &[&str] = &[
    "align", "bgcolor", "class", "dir", "height", "lang", "style", "title", "valign", "width",
];

const TAG_ATTRIBUTES: &[(&str, &[&str])] = &[
    ("a", &["href", "name"]),
    ("img", &["src", "alt", "border", "hspace", "vspace"]),
    ("font", &["color", "face", "size"]),
    (
        "table",
        &[
            "border",
            "cellpadding",
            "cellspacing",
            "frame",
            "rules",
            "summary",
        ],
    ),
    (
        "td",
        &["colspan", "rowspan", "nowrap", "abbr", "headers", "scope"],
    ),
    (
        "th",
        &["colspan", "rowspan", "nowrap", "abbr", "headers", "scope"],
    ),
    ("col", &["span"]),
    ("colgroup", &["span"]),
    ("ol", &["start", "type", "reversed"]),
    ("ul", &["type"]),
    ("li", &["value"]),
    ("hr", &["noshade", "size"]),
    ("blockquote", &["cite"]),
    ("q", &["cite"]),
    ("del", &["cite", "datetime"]),
    ("ins", &["cite", "datetime"]),
    ("details", &["open"]),
];

/// Vrai pour une image en ligne inoffensive : `data:image/(png|gif|jpeg|
/// webp|bmp)` encodée en base64 (le SVG, qui peut contenir des scripts
/// ou des références externes, est exclu).
#[must_use]
pub fn safe_data_image(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    ["png", "gif", "jpeg", "jpg", "webp", "bmp"]
        .iter()
        .any(|t| {
            lower
                .strip_prefix("data:image/")
                .and_then(|r| r.strip_prefix(t))
                .is_some_and(|r| r.starts_with(";base64,"))
        })
        && value
            .bytes()
            .skip(value.find(',').map_or(0, |i| i + 1))
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
}

/// Ce que l'assainissement a relevé.
#[derive(Debug, Default)]
struct Collected {
    remote: Vec<String>,
    blocked: usize,
}

/// Paramètres d'un assainissement.
#[derive(Debug, Clone)]
pub struct SanitizeOptions {
    /// Identifiant local du message (adresses `mailpart://` et
    /// `remote-img://`).
    pub message: i64,
    pub remote: RemoteContent,
    /// `Content-ID` (sans chevrons, en minuscules) → partie du message.
    pub content_ids: HashMap<String, PartPath>,
    pub protocols: Protocols,
}

/// Résultat de l'assainissement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sanitized {
    pub html: String,
    /// Adresses des images distantes autorisées, dans l'ordre de leur
    /// index `remote-img://<message>/<index>`.
    pub remote_images: Vec<String>,
    /// Images distantes bloquées (bannière).
    pub blocked_images: usize,
}

fn cid_resolver(options: &SanitizeOptions) -> impl Fn(&str) -> Option<String> + use<> {
    let content_ids = options.content_ids.clone();
    let protocols = options.protocols.clone();
    let message = options.message;
    move |cid: &str| {
        let key = cid
            .trim()
            .trim_start_matches('<')
            .trim_end_matches('>')
            .to_lowercase();
        content_ids
            .get(&key)
            .map(|path| protocols.mailpart_url(message, path))
    }
}

fn rewrite_image(
    value: &str,
    options: &SanitizeOptions,
    collected: &Mutex<Collected>,
    resolve_cid: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("cid:") {
        return resolve_cid(trimmed.get(4..).unwrap_or_default());
    }
    if safe_data_image(trimmed) {
        return Some(trimmed.to_owned());
    }
    let remote = (lower.starts_with("http://") || lower.starts_with("https://"))
        && trimmed.len() <= MAX_REMOTE_URL_LEN
        && ExternalLink::parse(trimmed).is_ok();
    if !remote {
        return None;
    }
    let mut c = collected.lock().unwrap_or_else(PoisonError::into_inner);
    if options.remote == RemoteContent::Allowed && c.remote.len() < MAX_REMOTE_IMAGES {
        let index = match c.remote.iter().position(|u| u == trimmed) {
            Some(i) => i,
            None => {
                c.remote.push(trimmed.to_owned());
                c.remote.len() - 1
            }
        };
        Some(options.protocols.remote_image_url(options.message, index))
    } else {
        c.blocked += 1;
        Some(BLOCKED_IMAGE.to_owned())
    }
}

/// Remplace le contenu de chaque `<style>` de la sortie d'`ammonia` (sans
/// attribut, texte brut non échappé) par sa version assainie.
fn sanitize_style_blocks(html: &str, resolve_cid: &dyn Fn(&str) -> Option<String>) -> String {
    const CLOSE: &str = "</style>";
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    // Toute balise ouvrante `<style` (avec ou sans attribut, par défense en
    // profondeur : l'attribute_filter les retire déjà).
    while let Some(start) = rest.find("<style") {
        let after_name = rest.get(start + 6..).unwrap_or_default();
        if !after_name.starts_with(['>', ' ', '\t', '\n', '/']) {
            let (before, after) = rest.split_at(start + 6);
            out.push_str(before);
            rest = after;
            continue;
        }
        let Some(gt) = after_name.find('>') else {
            break;
        };
        let (before, after) = rest.split_at(start + 6 + gt + 1);
        out.push_str(before);
        let end = after.find(CLOSE).unwrap_or(after.len());
        out.push_str(&sanitize_css(
            after.get(..end).unwrap_or_default(),
            resolve_cid,
        ));
        rest = after.get(end..).unwrap_or_default();
    }
    out.push_str(rest);
    out
}

/// Taille maximale du HTML assaini, en octets (au-delà : tronqué).
pub const MAX_HTML_LEN: usize = 2 * 1024 * 1024;
/// Profondeur d'imbrication maximale acceptée (au-delà : le HTML n'est pas
/// rendu, l'analyse deviendrait quadratique).
pub const MAX_NESTING: usize = 512;

/// Estimation bon marché de la profondeur d'imbrication des balises
/// (balises ouvrantes moins fermantes, éléments vides ignorés).
#[must_use]
pub fn nesting_depth(html: &str) -> usize {
    const VOID: &[&str] = &[
        "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source",
        "track", "wbr", "p", "li", "td", "tr", "th", "option",
    ];
    let bytes = html.as_bytes();
    let (mut depth, mut max) = (0usize, 0usize);
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'<' {
            let closing = bytes[i + 1] == b'/';
            let start = i + 1 + usize::from(closing);
            let mut end = start;
            while end < bytes.len() && bytes[end].is_ascii_alphanumeric() {
                end += 1;
            }
            if end > start {
                let name = html
                    .get(start..end)
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if closing {
                    depth = depth.saturating_sub(1);
                } else if !VOID.contains(&name.as_str()) {
                    depth += 1;
                    max = max.max(depth);
                }
            }
            i = end.max(i + 1);
        } else {
            i += 1;
        }
    }
    max
}

/// Assainit le HTML d'un mail.
#[must_use]
pub fn sanitize_html(html: &str, options: &SanitizeOptions) -> Sanitized {
    let mut cut = html.len().min(MAX_HTML_LEN);
    while !html.is_char_boundary(cut) {
        cut -= 1;
    }
    let html = html.get(..cut).unwrap_or_default();
    if nesting_depth(html) > MAX_NESTING {
        return Sanitized {
            html: "<p>Ce message est trop complexe pour être affiché en HTML.</p>".to_owned(),
            remote_images: Vec::new(),
            blocked_images: 0,
        };
    }
    let collected = Arc::new(Mutex::new(Collected::default()));
    let resolve = Arc::new(cid_resolver(options));
    let tag_attributes: HashMap<&str, HashSet<&str>> = TAG_ATTRIBUTES
        .iter()
        .map(|(t, a)| (*t, a.iter().copied().collect()))
        .collect();
    let mut builder = ammonia::Builder::empty();
    builder
        .tags(TAGS.iter().copied().collect())
        .clean_content_tags(
            [
                "script", "noscript", "template", "iframe", "noembed", "noframes", "object",
                "title",
            ]
            .into_iter()
            .collect(),
        )
        .generic_attributes(GENERIC_ATTRIBUTES.iter().copied().collect())
        .tag_attributes(tag_attributes)
        .url_schemes(
            ["http", "https", "mailto", "cid", "data"]
                .into_iter()
                .collect(),
        )
        .url_relative(ammonia::UrlRelative::Deny)
        .link_rel(Some("noopener noreferrer"))
        .strip_comments(true);
    {
        let options = options.clone();
        let collected = Arc::clone(&collected);
        let resolve = Arc::clone(&resolve);
        builder.attribute_filter(move |element, attribute, value| {
            let rewritten = match (element, attribute) {
                // `<style>` toujours sans attribut : son contenu est repéré
                // et assaini par sanitize_style_blocks.
                ("style", _) => None,
                ("img", "src") => rewrite_image(value, &options, &collected, &*resolve),
                (_, "href") => ExternalLink::parse(value).ok().map(|l| l.to_string()),
                // Citation : adresse non suivie, simple texte.
                (_, "cite") => None,
                (_, "style") => Some(sanitize_css(value, &*resolve)),
                _ => return Some(Cow::Borrowed(value)),
            };
            rewritten.map(Cow::Owned)
        });
    }
    let cleaned = builder.clean(html).to_string();
    let html = sanitize_style_blocks(&cleaned, &*resolve);
    let c = collected.lock().unwrap_or_else(PoisonError::into_inner);
    Sanitized {
        html,
        remote_images: c.remote.clone(),
        blocked_images: c.blocked,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(remote: RemoteContent) -> SanitizeOptions {
        let mut content_ids = HashMap::new();
        if let Some(p) = PartPath::parse("2") {
            content_ids.insert("logo@exemple".to_owned(), p);
        }
        SanitizeOptions {
            message: 42,
            remote,
            content_ids,
            protocols: Protocols::custom_schemes(),
        }
    }

    #[test]
    fn scripts_and_handlers_are_removed() {
        let out = sanitize_html(
            "<p onclick=\"alert(1)\">a<script>alert(2)</script><img src=x onerror=alert(3)></p>",
            &options(RemoteContent::Blocked),
        );
        assert_eq!(out.html, "<p>a<img></p>");
    }

    #[test]
    fn images_are_rewritten() {
        let html = "<img src=\"cid:logo@exemple\"><img src=\"cid:<LOGO@exemple>\">\
                    <img src=\"https://img.test/a.png\"><img src=\"https://img.test/a.png\">\
                    <img src=\"http://img.test/b.png\"><img src=\"ftp://x/y\">";
        let blocked = sanitize_html(html, &options(RemoteContent::Blocked));
        assert_eq!(blocked.blocked_images, 3);
        assert!(blocked.remote_images.is_empty());
        assert!(!blocked.html.contains("img.test"));
        assert_eq!(blocked.html.matches("mailpart://localhost/42/2").count(), 2);
        let allowed = sanitize_html(html, &options(RemoteContent::Allowed));
        assert_eq!(allowed.blocked_images, 0);
        assert_eq!(
            allowed.remote_images,
            ["https://img.test/a.png", "http://img.test/b.png"]
        );
        assert!(allowed.html.contains("remote-img://localhost/42/0"));
        assert!(allowed.html.contains("remote-img://localhost/42/1"));
        assert!(!allowed.html.contains("img.test"));
    }

    #[test]
    fn links_are_restricted_and_hardened() {
        let out = sanitize_html(
            "<a href=\"https://exemple.fr\" target=\"_top\">ok</a>\
             <a href=\"javascript:alert(1)\">x</a><a href=\"/relatif\">r</a>\
             <a href=\"https://banque.fr@attaquant.net\">p</a>",
            &options(RemoteContent::Blocked),
        );
        assert_eq!(
            out.html,
            "<a href=\"https://exemple.fr\" rel=\"noopener noreferrer\">ok</a>\
             <a rel=\"noopener noreferrer\">x</a><a rel=\"noopener noreferrer\">r</a>\
             <a rel=\"noopener noreferrer\">p</a>"
        );
    }

    #[test]
    fn style_blocks_are_sanitized() {
        let out = sanitize_html(
            "<style>@import url(http://canari.test/x.css); p{color:red;background:url(cid:logo@exemple)}</style><p>t</p>",
            &options(RemoteContent::Blocked),
        );
        assert!(!out.html.contains("canari"), "{}", out.html);
        assert!(!out.html.contains("@import"));
        assert!(
            out.html
                .contains("p{color:red;background:url(\"mailpart://localhost/42/2\")}")
        );
    }

    #[test]
    fn style_with_attributes_is_sanitized_too() {
        let out = sanitize_html(
            "<style class=\"x\" media=all>@import url(http://canari.test/i.css); \
             p{background:url(mailpart://localhost/1/2)}</style>",
            &options(RemoteContent::Blocked),
        );
        assert!(out.html.starts_with("<style>"), "{}", out.html);
        assert!(!out.html.contains("canari") && !out.html.contains("localhost/1/2"));
    }

    #[test]
    fn pathological_nesting_is_not_parsed() {
        let deep = "<div>".repeat(80_000);
        let started = std::time::Instant::now();
        let out = sanitize_html(&deep, &options(RemoteContent::Blocked));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        assert!(out.html.contains("trop complexe"));
        let fine = "<div>".repeat(200) + "x";
        assert!(
            sanitize_html(&fine, &options(RemoteContent::Blocked))
                .html
                .contains('x')
        );
    }

    #[test]
    fn data_images_are_limited_to_raster_base64() {
        assert!(safe_data_image("data:image/png;base64,iVBOR="));
        assert!(!safe_data_image("data:image/svg+xml;base64,PHN2Zz4="));
        assert!(!safe_data_image("data:image/png,<svg>"));
        assert!(!safe_data_image("data:text/html;base64,PHNjcmlwdD4="));
        assert!(!safe_data_image("data:image/png;base64,abc\"onerror=x"));
    }
}
