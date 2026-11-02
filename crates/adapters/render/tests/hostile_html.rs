//! Suite de vecteurs hostiles (`fixtures/hostile-html/`, ADR-004) : chaque
//! fichier est assaini comme le corps d'un mail, images distantes
//! bloquées puis autorisées, et la sortie est auditée balise par balise :
//! - balises et attributs dans la liste blanche, aucun gestionnaire
//!   d'événement ;
//! - images servies par `mailpart://`, `remote-img://` ou en ligne
//!   (`data:image/…` en base64, sans SVG) ;
//! - liens `http`, `https` ou `mailto` seulement ;
//! - CSS sans `@import`, sans `url()` externe, sans `expression` ;
//! - aucune trace de l'hôte canari dans un attribut ou une feuille de
//!   style.

use std::collections::HashMap;
use std::path::PathBuf;

use norelly_domain::fetch_policy::PartPath;
use norelly_domain::reading::RemoteContent;
use norelly_render::Protocols;
use norelly_render::sanitize::{SanitizeOptions, sanitize_html};

const MIN_VECTORS: usize = 40;

const ALLOWED_TAGS: &[&str] = &[
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

const ALLOWED_ATTRIBUTES: &[&str] = &[
    "align",
    "bgcolor",
    "class",
    "dir",
    "height",
    "lang",
    "style",
    "title",
    "valign",
    "width",
    "href",
    "name",
    "rel",
    "src",
    "alt",
    "border",
    "hspace",
    "vspace",
    "color",
    "face",
    "size",
    "cellpadding",
    "cellspacing",
    "frame",
    "rules",
    "summary",
    "colspan",
    "rowspan",
    "nowrap",
    "abbr",
    "headers",
    "scope",
    "span",
    "start",
    "type",
    "reversed",
    "value",
    "noshade",
    "datetime",
    "open",
];

fn fixtures() -> Vec<(String, String)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/hostile-html");
    let mut out: Vec<(String, String)> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "html"))
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let html =
                std::fs::read_to_string(e.path()).unwrap_or_else(|err| panic!("{name}: {err}"));
            (name, html)
        })
        .collect();
    out.sort();
    out
}

fn options(remote: RemoteContent) -> SanitizeOptions {
    let mut content_ids = HashMap::new();
    if let Some(p) = PartPath::parse("2") {
        content_ids.insert("logo@exemple".to_owned(), p);
    }
    SanitizeOptions {
        message: 9,
        remote,
        content_ids,
        protocols: Protocols::custom_schemes(),
    }
}

fn decode_entities(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&#39;", "'")
        .replace("&nbsp;", "\u{a0}")
        .replace("&amp;", "&")
}

fn safe_src(value: &str) -> bool {
    value.starts_with("mailpart://localhost/9/")
        || value.starts_with("remote-img://localhost/9/")
        || norelly_render::sanitize::safe_data_image(value)
}

fn safe_href(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
}

fn audit_css(name: &str, css: &str) {
    let lower = css.to_lowercase();
    for forbidden in [
        "@import",
        "expression",
        "javascript",
        "behavior",
        "binding",
        "image-set",
        "canari",
        "\\",
        "<",
    ] {
        assert!(
            !lower.contains(forbidden),
            "{name} : CSS « {forbidden} » : {css}"
        );
    }
    let mut rest = lower.as_str();
    while let Some(i) = rest.find("url(") {
        let after = rest.get(i + 4..).unwrap_or_default();
        assert!(
            after.starts_with("\"mailpart://localhost/9/") || after.starts_with("\"data:image/"),
            "{name} : url() externe : {css}"
        );
        rest = after;
    }
}

/// Audit de la sortie d'`ammonia` (HTML sérialisé : attributs entre
/// guillemets doubles, texte échappé).
fn audit(name: &str, html: &str) {
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        let after = rest.get(start + 1..).unwrap_or_default();
        assert!(
            !after.starts_with('!'),
            "{name} : commentaire ou doctype : {html}"
        );
        let closing = after.starts_with('/');
        let tag_start = if closing { 1 } else { 0 };
        let tag_len = after
            .get(tag_start..)
            .unwrap_or_default()
            .find(|c: char| !c.is_ascii_alphanumeric())
            .unwrap_or(0);
        let tag = after
            .get(tag_start..tag_start + tag_len)
            .unwrap_or_default();
        assert!(
            ALLOWED_TAGS.contains(&tag),
            "{name} : balise « {tag} » : {html}"
        );
        // Attributs jusqu'au `>` (hors guillemets).
        let mut i = tag_start + tag_len;
        let bytes = after.as_bytes();
        let mut in_quotes = false;
        let mut end = after.len();
        while i < bytes.len() {
            match bytes[i] {
                b'"' => in_quotes = !in_quotes,
                b'>' if !in_quotes => {
                    end = i;
                    break;
                }
                _ => {}
            }
            i += 1;
        }
        let attrs = after.get(tag_start + tag_len..end).unwrap_or_default();
        let mut a = attrs.trim();
        while !a.is_empty() {
            let name_end = a.find(['=', ' ', '/']).unwrap_or(a.len());
            let attr = a.get(..name_end).unwrap_or_default();
            if attr.is_empty() {
                a = a.get(1..).unwrap_or_default().trim_start();
                continue;
            }
            assert!(
                !closing,
                "{name} : attribut sur une balise fermante : {html}"
            );
            assert!(
                ALLOWED_ATTRIBUTES.contains(&attr) && !attr.starts_with("on"),
                "{name} : attribut « {attr} » sur <{tag}> : {html}"
            );
            let mut value = String::new();
            a = a.get(name_end..).unwrap_or_default();
            if let Some(v) = a.strip_prefix("=\"") {
                let close = v.find('"').unwrap_or(v.len());
                value = decode_entities(v.get(..close).unwrap_or_default());
                a = v.get(close + 1..).unwrap_or_default();
            }
            a = a.trim_start();
            match attr {
                "src" => assert!(safe_src(&value), "{name} : src « {value} »"),
                "href" => assert!(safe_href(&value), "{name} : href « {value} »"),
                "style" => audit_css(name, &value),
                _ => assert!(
                    !value.to_lowercase().contains("canari"),
                    "{name} : canari dans {attr} : {value}"
                ),
            }
        }
        rest = after.get(end + 1..).unwrap_or_default();
        if tag == "style" && !closing {
            let close = rest.find("</style>").unwrap_or(rest.len());
            audit_css(name, rest.get(..close).unwrap_or_default());
            rest = rest.get(close..).unwrap_or_default();
        }
    }
}

#[test]
fn hostile_vectors_are_neutralized() {
    let vectors = fixtures();
    assert!(
        vectors.len() >= MIN_VECTORS,
        "au moins {MIN_VECTORS} vecteurs attendus, {} trouvés",
        vectors.len()
    );
    for (name, html) in &vectors {
        for remote in [RemoteContent::Blocked, RemoteContent::Allowed] {
            let out = sanitize_html(html, &options(remote));
            audit(name, &out.html);
            // Images distantes : bloquées ou servies par le proxy, jamais
            // demandées directement.
            if remote == RemoteContent::Blocked {
                assert!(out.remote_images.is_empty(), "{name}");
            }
            for url in &out.remote_images {
                assert!(url.starts_with("http"), "{name} : {url}");
            }
            // Toute sortie est stable par réassainissement.
            let again = sanitize_html(&out.html, &options(remote));
            audit(name, &again.html);
        }
    }
}

#[test]
fn tracking_pixel_is_counted_as_blocked() {
    let (_, html) = fixtures()
        .into_iter()
        .find(|(n, _)| n.contains("pixel-espion"))
        .unwrap_or_else(|| panic!("vecteur pixel-espion absent"));
    let blocked = sanitize_html(&html, &options(RemoteContent::Blocked));
    assert_eq!(blocked.blocked_images, 1);
    let allowed = sanitize_html(&html, &options(RemoteContent::Allowed));
    assert_eq!(allowed.remote_images, ["http://canari.test/pixel.gif"]);
    assert!(allowed.html.contains("remote-img://localhost/9/0"));
}

/// L'audit lui-même détecte chaque vecteur non assaini (sinon il ne
/// prouverait rien).
#[test]
fn audit_rejects_every_raw_vector() {
    for (name, html) in fixtures() {
        let raw = html.trim().to_owned();
        let caught = std::panic::catch_unwind(|| audit(&name, &raw)).is_err();
        assert!(caught, "{name} : l'audit ne détecte pas le vecteur brut");
    }
}
