//! Oracle du fuzzing de [`crate::sanitize::sanitize_html`] : cherche dans
//! un HTML assaini ce qui ne doit jamais en sortir.

use std::fmt;

/// Contenu actif trouvé dans un HTML assaini.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    /// Balise interdite (nom en minuscules).
    Tag(String),
    /// Gestionnaire d'événement (`on…`).
    EventHandler { tag: String, attribute: String },
    /// Adresse active dans un attribut d'adresse : `javascript:`,
    /// `vbscript:`, ou `data:` autre qu'une image matricielle en base64
    /// dans `src`.
    ActiveUrl { tag: String, attribute: String },
    /// Adresse hors de celles attendues ([`find_unexpected_address`]).
    UnexpectedAddress { tag: String, attribute: String },
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tag(tag) => write!(f, "balise <{tag}>"),
            Self::EventHandler { tag, attribute } => {
                write!(f, "gestionnaire « {attribute} » sur <{tag}>")
            }
            Self::ActiveUrl { tag, attribute } => {
                write!(f, "adresse active dans « {attribute} » sur <{tag}>")
            }
            Self::UnexpectedAddress { tag, attribute } => {
                write!(f, "adresse inattendue dans « {attribute} » sur <{tag}>")
            }
        }
    }
}

/// Balises qui exécutent du code, chargent un document, un objet ou une
/// feuille de style, changent l'adresse de base ou envoient des données.
/// Le pistage par les images distantes n'est pas du ressort de l'oracle
/// (vecteurs hostiles, `tests/hostile_html.rs`).
const ACTIVE_TAGS: &[&str] = &[
    "script", "iframe", "frame", "frameset", "object", "embed", "applet", "base", "meta", "link",
    "form",
];

/// Balises que l'analyse de l'oracle ne modélise pas, signalées dès
/// qu'elles apparaissent :
/// - contenu étranger (SVG, MathML : ni texte brut ni mêmes règles) ;
/// - `<noscript>`, `<noembed>`, `<noframes>` : texte brut ou HTML selon
///   le réglage du moteur (scripts, greffons, cadres) ;
/// - les éléments de texte brut autres que `<style>` (`<xmp>`,
///   `<textarea>`, `<title>`, `<plaintext>`) ;
/// - `<select>` et `<template>`, dans lesquels le navigateur peut ignorer
///   `<style>` (par exemple après `<template><col>`) et lire son contenu
///   comme du balisage.
const UNMODELED_TAGS: &[&str] = &[
    "svg",
    "math",
    "noscript",
    "noembed",
    "noframes",
    "xmp",
    "textarea",
    "title",
    "plaintext",
    "select",
    "template",
];

/// Attributs qui portent une adresse.
const URL_ATTRIBUTES: &[&str] = &[
    "href",
    "src",
    "action",
    "formaction",
    "background",
    "cite",
    "data",
    "poster",
    "codebase",
    "longdesc",
    "lowsrc",
    "dynsrc",
    "ping",
    "xlink:href",
];

/// Schémas d'adresse qui exécutent du code.
const SCRIPT_SCHEMES: &[&str] = &["javascript:", "vbscript:"];

/// Types d'image en ligne (`data:image/…;base64,`) admis dans `src` : les
/// formats matriciels, sans SVG.
const RASTER_IMAGES: &[&str] = &["png", "gif", "jpeg", "jpg", "webp", "bmp"];

/// Premier contenu actif trouvé dans `html`, ou `None`.
///
/// `html` est analysé comme le tokeniseur HTML du navigateur (voir
/// [`scan`]) ; les contextes que le navigateur analyse autrement (contenu
/// étranger, `<select>`, `<template>`, `<noscript>`…) sont signalés
/// d'office.
#[must_use]
pub fn find_active_content(html: &str) -> Option<Finding> {
    scan(html, &mut |name, attributes| {
        if ACTIVE_TAGS.contains(&name) || UNMODELED_TAGS.contains(&name) {
            return Some(Finding::Tag(name.to_owned()));
        }
        for (attribute, value) in attributes {
            if attribute.starts_with("on") {
                return Some(Finding::EventHandler {
                    tag: name.to_owned(),
                    attribute: attribute.clone(),
                });
            }
            if URL_ATTRIBUTES.contains(&attribute.as_str()) && is_active_url(attribute, value) {
                return Some(Finding::ActiveUrl {
                    tag: name.to_owned(),
                    attribute: attribute.clone(),
                });
            }
        }
        None
    })
}

/// Première adresse inattendue de `html` : un `src` qui n'est ni une
/// image matricielle en ligne en base64 ni sous l'un des préfixes
/// `sources`, un `href` qui n'est ni `http://`, ni `https://`, ni
/// `mailto:`. Analyse comme [`find_active_content`].
#[must_use]
pub fn find_unexpected_address(html: &str, sources: &[&str]) -> Option<Finding> {
    scan(html, &mut |name, attributes| {
        attributes.iter().find_map(|(attribute, value)| {
            let lower = value.to_ascii_lowercase();
            let expected = match attribute.as_str() {
                "src" => {
                    lower.strip_prefix("data:").is_some_and(is_raster_base64)
                        || sources.iter().any(|s| value.starts_with(s))
                }
                "href" => ["http://", "https://", "mailto:"]
                    .iter()
                    .any(|s| lower.starts_with(s)),
                _ => true,
            };
            (!expected).then(|| Finding::UnexpectedAddress {
                tag: name.to_owned(),
                attribute: attribute.clone(),
            })
        })
    })
}

/// Attribut d'une balise : nom en minuscules, valeur décodée.
type Attribute = (String, String);

/// Analyse `html` comme le tokeniseur HTML du navigateur et passe chaque
/// balise ouvrante (nom en minuscules, attributs aux valeurs décodées) à
/// `visit`, jusqu'au premier constat.
///
/// Noms de balises et d'attributs insensibles à la casse, valeurs entre
/// guillemets doubles, simples ou sans guillemets, références de
/// caractères décodées, contenu de `<style>` arrêté à sa balise fermante.
/// Le texte échappé (`&lt;script&gt;`) et le contenu des valeurs
/// d'attribut ne sont pas des balises. Dans le doute, l'analyse penche
/// vers le constat : une balise non fermée en fin de texte est analysée
/// comme si elle l'était (le document qui englobe le HTML assaini
/// pourrait la fermer), un `<!…>` s'arrête au premier `>`.
fn scan(
    html: &str,
    visit: &mut dyn FnMut(&str, &[Attribute]) -> Option<Finding>,
) -> Option<Finding> {
    let mut rest = html;
    while let Some(lt) = rest.find('<') {
        let after = rest.get(lt + 1..).unwrap_or_default();
        let closing = after.starts_with('/');
        let name_at = usize::from(closing);
        let starts_name = after
            .get(name_at..)
            .and_then(|s| s.chars().next())
            .is_some_and(|c| c.is_ascii_alphabetic());
        if !starts_name {
            rest = if after.starts_with(['!', '?'])
                || (closing && !after.get(1..).unwrap_or_default().starts_with('>'))
            {
                // Commentaire, doctype, instruction ou fermante invalide :
                // ignorés jusqu'au premier `>`.
                after
                    .find('>')
                    .map_or("", |i| after.get(i + 1..).unwrap_or_default())
            } else {
                after
            };
            continue;
        }
        let tag_text = after.get(name_at..).unwrap_or_default();
        let name_len = tag_text.find(is_tag_name_end).unwrap_or(tag_text.len());
        let name = tag_text
            .get(..name_len)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let (attributes, tail) = parse_attributes(tag_text.get(name_len..).unwrap_or_default());
        rest = tail;
        if closing {
            continue;
        }
        if let Some(finding) = visit(&name, &attributes) {
            return Some(finding);
        }
        if name == "style" {
            rest = skip_raw_text(rest, &name);
        }
    }
    None
}

fn is_html_space(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\x0c' | '\r' | ' ')
}

fn is_tag_name_end(c: char) -> bool {
    is_html_space(c) || c == '/' || c == '>'
}

/// Attributs (noms en minuscules, valeurs décodées) d'une balise dont le
/// nom vient d'être lu, et le texte qui suit son `>`.
fn parse_attributes(text: &str) -> (Vec<(String, String)>, &str) {
    let mut attributes = Vec::new();
    let mut rest = text;
    loop {
        rest = rest.trim_start_matches(|c| is_html_space(c) || c == '/');
        if rest.is_empty() {
            return (attributes, "");
        }
        if let Some(tail) = rest.strip_prefix('>') {
            return (attributes, tail);
        }
        // Un `=` en tête fait partie du nom.
        let first = rest.chars().next().map_or(0, char::len_utf8);
        let name_len = first
            + rest
                .get(first..)
                .unwrap_or_default()
                .find(|c| is_tag_name_end(c) || c == '=')
                .unwrap_or(rest.len() - first);
        let name = rest
            .get(..name_len)
            .unwrap_or_default()
            .to_ascii_lowercase();
        rest = rest.get(name_len..).unwrap_or_default();
        let after_name = rest.trim_start_matches(is_html_space);
        let mut value = String::new();
        if let Some(v) = after_name.strip_prefix('=') {
            let v = v.trim_start_matches(is_html_space);
            let (raw, tail) = if let Some(q) = v.strip_prefix('"') {
                split_quoted(q, '"')
            } else if let Some(q) = v.strip_prefix('\'') {
                split_quoted(q, '\'')
            } else {
                let end = v.find(|c| is_html_space(c) || c == '>').unwrap_or(v.len());
                (
                    v.get(..end).unwrap_or_default(),
                    v.get(end..).unwrap_or_default(),
                )
            };
            value = decode_character_references(raw);
            rest = tail;
        } else {
            rest = after_name;
        }
        attributes.push((name, value));
    }
}

fn split_quoted(text: &str, quote: char) -> (&str, &str) {
    match text.find(quote) {
        Some(i) => (
            text.get(..i).unwrap_or_default(),
            text.get(i + 1..).unwrap_or_default(),
        ),
        None => (text, ""),
    }
}

/// Texte qui suit la balise fermante `</name` (insensible à la casse,
/// suivie d'une espace, de `/` ou de `>`) du texte brut `text`.
fn skip_raw_text<'a>(text: &'a str, name: &str) -> &'a str {
    let mut from = 0;
    while let Some(i) = text.get(from..).and_then(|t| t.find("</")) {
        let at = from + i;
        let candidate = text.get(at + 2..).unwrap_or_default();
        let matches_name = candidate
            .get(..name.len())
            .is_some_and(|n| n.eq_ignore_ascii_case(name));
        let ends = candidate
            .get(name.len()..)
            .and_then(|t| t.chars().next())
            .is_none_or(is_tag_name_end);
        if matches_name && ends {
            return text.get(at..).unwrap_or_default();
        }
        from = at + 2;
    }
    ""
}

/// Valeur d'attribut décodée comme le fait le navigateur : références
/// numériques (avec ou sans `;`) et références nommées qui peuvent
/// masquer un schéma (`&colon;`, `&Tab;`, `&NewLine;`, …). Une référence
/// inconnue reste telle quelle.
fn decode_character_references(raw: &str) -> String {
    const NAMED: &[(&str, char)] = &[
        ("amp;", '&'),
        ("lt;", '<'),
        ("gt;", '>'),
        ("quot;", '"'),
        ("apos;", '\''),
        ("nbsp;", '\u{a0}'),
        ("colon;", ':'),
        ("Tab;", '\t'),
        ("NewLine;", '\n'),
    ];
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(amp) = rest.find('&') {
        out.push_str(rest.get(..amp).unwrap_or_default());
        let after = rest.get(amp + 1..).unwrap_or_default();
        if let Some(numeric) = after.strip_prefix('#') {
            let (hex, digits) = match numeric.strip_prefix(['x', 'X']) {
                Some(h) => (true, h),
                None => (false, numeric),
            };
            let len = digits
                .find(|c: char| {
                    if hex {
                        !c.is_ascii_hexdigit()
                    } else {
                        !c.is_ascii_digit()
                    }
                })
                .unwrap_or(digits.len());
            if len > 0 {
                let code = u32::from_str_radix(
                    digits.get(..len).unwrap_or_default(),
                    if hex { 16 } else { 10 },
                )
                .unwrap_or(u32::MAX);
                out.push(
                    char::from_u32(code)
                        .filter(|c| *c != '\0')
                        .unwrap_or('\u{fffd}'),
                );
                let tail = digits.get(len..).unwrap_or_default();
                rest = tail.strip_prefix(';').unwrap_or(tail);
                continue;
            }
        } else if let Some((name, c)) = NAMED.iter().find(|(n, _)| after.starts_with(n)) {
            out.push(*c);
            rest = after.get(name.len()..).unwrap_or_default();
            continue;
        }
        out.push('&');
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Vrai si l'adresse, nettoyée comme le fait l'analyseur d'URL (espaces
/// et contrôles C0 en tête et en queue retirés, tabulations et fins de
/// ligne supprimées partout), commence par un schéma de script, ou par
/// `data:` hors d'une image matricielle en base64 dans `src`.
fn is_active_url(attribute: &str, value: &str) -> bool {
    let cleaned: String = value
        .trim_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect::<String>()
        .to_ascii_lowercase();
    if SCRIPT_SCHEMES.iter().any(|s| cleaned.starts_with(s)) {
        return true;
    }
    let Some(data) = cleaned.strip_prefix("data:") else {
        return false;
    };
    !(attribute == "src" && is_raster_base64(data))
}

/// Vrai pour la suite d'une adresse `data:` (en minuscules) d'image
/// matricielle en base64.
fn is_raster_base64(data: &str) -> bool {
    data.strip_prefix("image/")
        .and_then(|d| d.split_once(';'))
        .is_some_and(|(kind, rest)| RASTER_IMAGES.contains(&kind) && rest.starts_with("base64,"))
}

#[cfg(test)]
mod tests;
