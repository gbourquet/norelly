//! Assainissement du CSS d'un mail (balises `<style>` et attributs
//! `style`), en défense en profondeur de la CSP (qui interdit déjà tout
//! chargement réseau depuis le CSS) :
//! - commentaires et échappements CSS résolus, puis vérifications faites
//!   sur le texte final, celui que lira le moteur de rendu ;
//! - règles `@import`, `@charset`, `@namespace` et `@page` supprimées ;
//! - requêtes de média alignées sur l'écran (`print` jamais vrai,
//!   `screen` toujours vrai) : l'impression montre ce que l'écran montre ;
//! - `url(…)` réécrites : `cid:` vers `mailpart://`, `data:image/…`
//!   conservées, tout le reste remplacé par `none` ;
//! - fonctions qui chargent ou exécutent (`image-set`, `expression`, `src`,
//!   `element`…) et propriétés dangereuses (`behavior`, `-moz-binding`)
//!   neutralisées ;
//! - `<` remplacé (le texte ne peut pas fermer la balise `<style>`).

/// Longueur maximale du CSS conservé par bloc, en octets.
pub const MAX_CSS_LEN: usize = 512 * 1024;

/// Fonctions CSS neutralisées (elles chargent une ressource ou exécutent
/// du code dans certains moteurs).
const BLOCKED_FUNCTIONS: &[&str] = &[
    "image-set",
    "-webkit-image-set",
    "image",
    "cross-fade",
    "-webkit-cross-fade",
    "element",
    "-moz-element",
    "expression",
    "src",
    "paint",
    "attr",
];

/// Mots neutralisés partout (propriétés ou valeurs exécutables).
/// `color-adjust` : l'impression des fonds est fixée par Norelly.
const BLOCKED_WORDS: &[&str] = &[
    "behavior",
    "-moz-binding",
    "javascript",
    "vbscript",
    "color-adjust",
];

/// Règles retirées en entier.
/// `@custom-media` nommerait une requête de média hors de la réécriture.
const BLOCKED_AT_RULES: &[&str] = &["@import", "@charset", "@namespace", "@custom-media"];

/// Résout les échappements et retire les commentaires.
fn normalize(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut chars = css.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = '\0';
                for c in chars.by_ref() {
                    if prev == '*' && c == '/' {
                        break;
                    }
                    prev = c;
                }
                out.push(' ');
            }
            '\\' => {
                let mut hex = String::new();
                while hex.len() < 6 {
                    match chars.peek() {
                        Some(h) if h.is_ascii_hexdigit() => {
                            hex.push(*h);
                            chars.next();
                        }
                        _ => break,
                    }
                }
                if hex.is_empty() {
                    // `\` suivi d'un autre caractère : ce caractère.
                    match chars.next() {
                        Some('\n') | None => {}
                        Some(other) => out.push(other),
                    }
                } else {
                    if chars.peek().is_some_and(|c| c.is_whitespace()) {
                        chars.next();
                    }
                    let decoded = u32::from_str_radix(&hex, 16)
                        .ok()
                        .and_then(char::from_u32)
                        .filter(|c| *c != '\0')
                        .unwrap_or('\u{fffd}');
                    out.push(decoded);
                }
            }
            '\0' => out.push('\u{fffd}'),
            _ => out.push(c),
        }
    }
    // Plus aucun échappement possible dans le résultat.
    out.replace('\\', " ")
}

/// Retire les règles `@import`, `@charset`, `@namespace` (jusqu'au `;`),
/// en un seul passage ; recommence tant qu'il en reste (une règle peut en
/// former une autre une fois retirée), chaque tour raccourcissant le texte.
fn remove_at_rules(css: String) -> String {
    let mut current = css;
    loop {
        let lower = current.to_ascii_lowercase();
        let mut out = String::with_capacity(current.len());
        let mut i = 0;
        let mut removed = false;
        while i < current.len() {
            let rest = lower.get(i..).unwrap_or_default();
            if rest.starts_with('@') && BLOCKED_AT_RULES.iter().any(|r| rest.starts_with(r)) {
                let end = rest.find(';').map_or(current.len(), |j| i + j + 1);
                out.push(' ');
                i = end;
                removed = true;
                continue;
            }
            let Some(c) = current.get(i..).and_then(|r| r.chars().next()) else {
                break;
            };
            out.push(c);
            i += c.len_utf8();
        }
        if !removed {
            return out;
        }
        current = out;
    }
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '-' || c == '_'
}

/// Contenu de `url(` … `)` (à partir de l'indice suivant la parenthèse) :
/// renvoie la valeur sans guillemets et l'indice de fin (après `)`).
fn url_argument(css: &str, from: usize) -> (String, usize) {
    let rest = css.get(from..).unwrap_or_default();
    let trimmed = rest.trim_start();
    let offset = rest.len() - trimmed.len();
    let mut chars = trimmed.char_indices();
    let quote = match trimmed.chars().next() {
        Some(q @ ('"' | '\'')) => {
            chars.next();
            Some(q)
        }
        _ => None,
    };
    let mut value = String::new();
    let mut end = trimmed.len();
    let mut closed_quote = quote.is_none();
    for (i, c) in chars {
        if !closed_quote {
            if Some(c) == quote {
                closed_quote = true;
            } else {
                value.push(c);
            }
            continue;
        }
        if c == ')' {
            end = i + 1;
            break;
        }
        if quote.is_none() {
            value.push(c);
        }
    }
    (value.trim().to_owned(), from + offset + end)
}

/// Fin d'un bloc délimité par `open` et `close` qui commence à `from` :
/// indice après le `close` correspondant (blocs imbriqués et chaînes
/// compris), ou après un `;` rencontré hors de tout bloc si `semicolon`,
/// ou fin du texte.
fn block_end(css: &str, from: usize, open: char, close: char, semicolon: bool) -> usize {
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    for (j, c) in css.get(from..).unwrap_or_default().char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, ';') if semicolon && depth == 0 => return from + j + 1,
            (None, c) if c == open => depth += 1,
            (None, c) if c == close => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return from + j + 1;
                }
            }
            _ => {}
        }
    }
    css.len()
}

/// Fin d'un appel de fonction dont la parenthèse ouvrante est à `open`.
fn call_end(css: &str, open: usize) -> usize {
    block_end(css, open, '(', ')', false)
}

/// Fin d'une règle `@…` commençant à `from` : après son `;` si elle n'a
/// pas de bloc, sinon après l'accolade fermante de son bloc.
fn at_rule_end(css: &str, from: usize) -> usize {
    block_end(css, from, '{', '}', true)
}

/// Caractéristiques de média dont la valeur change sur le papier.
const PRINT_DEPENDENT_FEATURES: &[&str] = &[
    "update",
    "scan",
    "grid",
    "color",
    "min-color",
    "max-color",
    "color-index",
    "min-color-index",
    "max-color-index",
    "monochrome",
    "min-monochrome",
    "max-monochrome",
    "prefers-color-scheme",
    "display-mode",
];

/// Mot d'une requête de média réécrit pour que l'impression montre ce que
/// montre l'écran : `print` et les caractéristiques qui changent sur le
/// papier (rafraîchissement, appareil, résolution, couleurs) deviennent
/// inconnus (jamais vrais), `screen` devient `all` (vrai aussi sur le
/// papier).
fn media_word(lower: &str) -> Option<&'static str> {
    match lower {
        "screen" => Some("all"),
        "print" => Some("x-bloque"),
        _ if PRINT_DEPENDENT_FEATURES.contains(&lower)
            || lower.contains("device-")
            || lower.ends_with("resolution") =>
        {
            Some("x-bloque")
        }
        _ => None,
    }
}

/// Réécriture d'une ressource référencée par `url()`.
fn rewrite_url(value: &str, resolve_cid: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    if let Some(cid) = lower.strip_prefix("cid:") {
        let original = value.get(4..).unwrap_or(cid);
        return resolve_cid(original);
    }
    crate::sanitize::safe_data_image(value).then(|| value.to_owned())
}

/// Assainit un bloc CSS. `resolve_cid` donne l'adresse `mailpart://` d'un
/// `Content-ID` du message (ou `None` s'il est inconnu).
#[must_use]
pub fn sanitize_css(css: &str, resolve_cid: &dyn Fn(&str) -> Option<String>) -> String {
    let truncated = match css.char_indices().nth(MAX_CSS_LEN) {
        Some((i, _)) => css.get(..i).unwrap_or_default(),
        None => css,
    };
    let css = remove_at_rules(normalize(truncated));
    let mut out = String::with_capacity(css.len());
    let mut i = 0;
    // Entre `@media` et son `{` : requête de média.
    let mut media_query = false;
    // Chaîne en cours (les échappements sont déjà résolus) : `@`, `{` et
    // `;` n'y sont que du texte.
    let mut quote: Option<char> = None;
    while i < css.len() {
        let Some(c) = css.get(i..).and_then(|r| r.chars().next()) else {
            break;
        };
        if is_ident_char(c) {
            let start = i;
            let mut end = i;
            for (j, ch) in css.get(i..).unwrap_or_default().char_indices() {
                if !is_ident_char(ch) {
                    break;
                }
                end = i + j + ch.len_utf8();
            }
            let word = css.get(start..end).unwrap_or_default();
            let lower = word.to_lowercase();
            if css.get(end..).is_some_and(|r| r.starts_with('(')) {
                if lower == "url" {
                    let (value, after) = url_argument(&css, end + 1);
                    match rewrite_url(&value, resolve_cid) {
                        Some(url) => {
                            out.push_str("url(\"");
                            out.push_str(&url.replace(['"', '\n', '\r'], ""));
                            out.push_str("\")");
                        }
                        None => out.push_str("none"),
                    }
                    i = after;
                    continue;
                }
                if BLOCKED_FUNCTIONS.contains(&lower.as_str()) {
                    out.push_str("none");
                    i = call_end(&css, end);
                    continue;
                }
            }
            if BLOCKED_WORDS.iter().any(|w| lower.contains(w)) {
                out.push_str("x-bloque");
            } else if let Some(media) = (media_query && quote.is_none())
                .then(|| media_word(&lower))
                .flatten()
            {
                out.push_str(media);
            } else {
                out.push_str(word);
            }
            i = end;
            continue;
        }
        match c {
            '<' => out.push(' '),
            '"' | '\'' => {
                match quote {
                    None => quote = Some(c),
                    Some(q) if q == c => quote = None,
                    Some(_) => {}
                }
                out.push(c);
            }
            '@' | '{' | ';' if quote.is_some() => out.push(c),
            '@' => {
                // Mot-clé lu sur quelques caractères seulement (coût borné
                // même pour un texte truffé de `@`).
                let keyword: String = css
                    .get(i + 1..)
                    .unwrap_or_default()
                    .chars()
                    .take_while(|ch| is_ident_char(*ch))
                    .take(8)
                    .collect::<String>()
                    .to_ascii_lowercase();
                if keyword == "page" {
                    // Mise en page de l'impression (marges, textes des
                    // marges) : supprimée, le papier reprend l'écran.
                    out.push(' ');
                    i = at_rule_end(&css, i);
                    continue;
                }
                // At-rule restante (média, keyframes…) : conservée.
                media_query = keyword == "media";
                out.push('@');
            }
            '{' | ';' => {
                media_query = false;
                out.push(c);
            }
            _ => out.push(c),
        }
        i += c.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(css: &str) -> String {
        sanitize_css(css, &|cid| {
            (cid == "logo@x").then(|| "mailpart://localhost/7/2".to_owned())
        })
    }

    #[test]
    fn imports_and_remote_urls_are_removed() {
        for hostile in [
            "@import url(http://canari.test/a.css); p { color: red }",
            "@import 'http://canari.test/a.css'; p{}",
            "@\\69mport url(http://canari.test/x.css);",
            "@IMPORT url(http://canari.test/x.css);",
            "@imp/**/ort url(http://canari.test/x.css);",
            "@imp@importort url(http://canari.test/x.css);",
            "body { background: url(http://canari.test/fond.png) }",
            "body { background: url( 'https://canari.test/fond.png' ) }",
            "body { background: u\\72l(http://canari.test/f.png) }",
            "body { background: URL(\"//canari.test/f.png\") }",
            "body { background-image: image-set('http://canari.test/a.png' 1x) }",
            "body { background-image: -webkit-image-set(url(http://canari.test/a.png) 1x) }",
            "@font-face { font-family: x; src: url(http://canari.test/f.woff) }",
            "li { list-style: url(http://canari.test/puce.png) }",
            "div { cursor: url(http://canari.test/c.cur), auto }",
            "div { width: expression(alert(1)) }",
            "div { behavior: url(x.htc) }",
            "div { -moz-binding: url(http://canari.test/x.xml#x) }",
            "div { background: url(javascript:alert(1)) }",
            "a { content: '</style><script>alert(1)</script>' }",
            "@media print { body { background: url(http://canari.test/impression.png) } }",
            "@page { @top-center { content: url(http://canari.test/marge.png) } }",
            "@supports (display: grid) { @media print { p { background: url(//canari.test/x) } } }",
        ] {
            let out = clean(hostile);
            let lower = out.to_lowercase();
            for forbidden in [
                "@import",
                "canari",
                "expression(",
                "behavior",
                "binding",
                "javascript",
                "image-set(",
                "<",
                "\\",
            ] {
                assert!(
                    !lower.contains(forbidden),
                    "{hostile:?} → {out:?} ({forbidden})"
                );
            }
        }
    }

    #[test]
    fn cid_and_inline_images_are_kept() {
        assert_eq!(
            clean("p{background:url(cid:logo@x)}"),
            "p{background:url(\"mailpart://localhost/7/2\")}"
        );
        assert_eq!(
            clean("p{background:url(cid:inconnu)}"),
            "p{background:none}"
        );
        let data = "p{background:url(data:image/png;base64,iVBORw0KGgo=)}";
        assert_eq!(
            clean(data),
            "p{background:url(\"data:image/png;base64,iVBORw0KGgo=\")}"
        );
        assert_eq!(
            clean("p{background:url(data:text/html;base64,PHNjcmlwdD4=)}"),
            "p{background:none}"
        );
    }

    #[test]
    fn ordinary_css_is_untouched() {
        let css = "@media (max-width: 600px) { .col { width: 100% !important; } } \
                   td { font-family: 'Segoe UI', sans-serif; color: #333; }";
        assert_eq!(clean(css), css);
    }

    #[test]
    fn print_media_matches_the_screen() {
        // L'impression montre ce que l'écran montre : une règle réservée
        // à l'impression ne s'applique jamais, une règle réservée à
        // l'écran s'applique aussi sur le papier.
        assert_eq!(
            clean("@media print { .iban { display: block } } p{}"),
            "@media x-bloque { .iban { display: block } } p{}"
        );
        assert_eq!(
            clean("@MEDIA Print, (max-width: 1px) { a{} }"),
            "@MEDIA x-bloque, (max-width: 1px) { a{} }"
        );
        assert_eq!(
            clean("@media only screen and (max-width: 600px) { .col { width: 100% } }"),
            "@media only all and (max-width: 600px) { .col { width: 100% } }"
        );
        assert_eq!(
            clean("@supports (display: grid) { @media not screen { a { color: red } } }"),
            "@supports (display: grid) { @media not all { a { color: red } } }"
        );
        // Une chaîne ne termine pas la requête.
        assert_eq!(
            clean("@media \"{\", print { a{} }"),
            "@media \"{\", x-bloque { a{} }"
        );
        assert_eq!(
            clean("@media ';', print { a{} }"),
            "@media ';', x-bloque { a{} }"
        );
        // Caractéristiques qui changent sur le papier : jamais vraies.
        for feature in [
            "update: none",
            "min-device-width: 1px",
            "-webkit-min-device-pixel-ratio: 2",
            "min-resolution: 2dppx",
            "monochrome",
            "color",
            "prefers-color-scheme: dark",
        ] {
            let out = clean(&format!("@media ({feature}) {{ a{{}} }}"));
            assert!(out.starts_with("@media (x-bloque"), "{feature:?} → {out:?}");
        }
        // Ailleurs que dans une requête de média, les mots restent.
        let ordinary = ".print, .screen, .update { color: red; width: 1px }";
        assert_eq!(clean(ordinary), ordinary);
        // Dans une chaîne, `@media` et `@page` ne sont que du texte.
        let quoted = "p::after{content:\"@media screen print\"} a{content:'@page {'} b{color:red}";
        assert_eq!(clean(quoted), quoted);
    }

    #[test]
    fn page_rules_are_removed() {
        for css in [
            "@page { margin: 0; @top-center { content: 'Payez sur FR76' } } p{color:red}",
            "@PAGE :first { size: A4 } p{color:red}",
            "@pa\\67 e { margin: 0 } p{color:red}",
        ] {
            let out = clean(css);
            assert!(!out.to_lowercase().contains("@page"), "{css:?} → {out:?}");
            assert!(!out.contains("Payez"), "{css:?} → {out:?}");
            assert!(
                out.trim_start().starts_with("p{color:red}"),
                "{css:?} → {out:?}"
            );
        }
        // Règle non fermée : supprimée jusqu'à la fin.
        assert_eq!(clean("p{} @page { margin: 0").trim(), "p{}");
        // Dans une chaîne, `@page` n'est que du texte.
        let quoted = "a{content:\"info@page.fr\"} b{color:red}";
        assert_eq!(clean(quoted), quoted);
        // Requête de média nommée : supprimée (elle échapperait à la
        // réécriture).
        let out = clean("@custom-media --p print; @media (--p) { a{} }");
        assert!(
            !out.contains("custom-media") && !out.contains("print"),
            "{out:?}"
        );
    }

    #[test]
    fn print_color_adjustment_is_left_to_norelly() {
        // Le mail ne choisit pas l'impression des fonds : un texte masqué
        // par son fond à l'écran le reste sur le papier.
        for css in [
            "p { print-color-adjust: economy }",
            "p { -webkit-print-color-adjust: economy }",
            "p { color-adjust: economy }",
        ] {
            let out = clean(css);
            assert!(!out.contains("color-adjust"), "{css:?} → {out:?}");
        }
    }

    #[test]
    fn many_at_rules_are_removed_in_linear_time() {
        let css = "@import;".repeat(60_000);
        let started = std::time::Instant::now();
        assert!(!clean(&css).contains("@import"));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn many_media_rules_stay_fast() {
        let css = "@m".repeat(200_000) + &"@page{".repeat(50_000);
        let started = std::time::Instant::now();
        let _ = clean(&css);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn huge_css_is_truncated() {
        let css = "a{}".repeat(MAX_CSS_LEN);
        assert!(clean(&css).len() <= MAX_CSS_LEN);
    }
}
