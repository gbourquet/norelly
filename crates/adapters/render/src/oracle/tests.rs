use std::collections::HashMap;
use std::path::PathBuf;

use norelly_domain::reading::RemoteContent;

use super::*;
use crate::Protocols;
use crate::sanitize::{SanitizeOptions, sanitize_html};

fn tag(name: &str) -> Option<Finding> {
    Some(Finding::Tag(name.to_owned()))
}

fn handler(tag: &str, attribute: &str) -> Option<Finding> {
    Some(Finding::EventHandler {
        tag: tag.to_owned(),
        attribute: attribute.to_owned(),
    })
}

fn active_url(tag: &str, attribute: &str) -> Option<Finding> {
    Some(Finding::ActiveUrl {
        tag: tag.to_owned(),
        attribute: attribute.to_owned(),
    })
}

#[test]
fn forbidden_tags_are_found_whatever_their_case() {
    for (html, name) in [
        ("<script>alert(1)</script>", "script"),
        ("<ScRiPt src=x>", "script"),
        ("<p>a</p><IFRAME src=x>", "iframe"),
        ("<base href=\"https://x.test/\">", "base"),
        ("<meta http-equiv=refresh content=0>", "meta"),
        ("<form action=x><p>a</p></form>", "form"),
        ("<object data=x>", "object"),
        ("<embed src=x>", "embed"),
        ("<link rel=stylesheet href=x>", "link"),
        ("<frameset><frame src=x>", "frameset"),
        ("<svg><style>x</style></svg>", "svg"),
        ("<math><mi>x</mi></math>", "math"),
        ("<noscript><p>x</p></noscript>", "noscript"),
        // Dans un `<select>`, le navigateur ignore `<style>` ou
        // `<plaintext>` et lit la suite comme du balisage.
        ("<select><style></select><img onerror=x></style>", "select"),
        ("<script/src=x>", "script"),
        ("<script\n>", "script"),
        // Balise non fermée en fin de sortie : le document qui l'englobe
        // pourrait la compléter.
        ("<p>a</p><script", "script"),
    ] {
        assert_eq!(find_active_content(html), tag(name), "{html}");
    }
}

#[test]
fn event_handlers_are_found_in_every_attribute_syntax() {
    for (html, t, a) in [
        ("<img src=x onerror=alert(1)>", "img", "onerror"),
        ("<p ONCLICK=\"x\">", "p", "onclick"),
        ("<p title='a' onmouseover='x'>", "p", "onmouseover"),
        ("<p/onclick=x>", "p", "onclick"),
        ("<p\tonfocus\n=\nx>", "p", "onfocus"),
        ("<details open ontoggle>", "details", "ontoggle"),
        // Tout attribut en `on…`, même inconnu aujourd'hui.
        ("<p one=1>", "p", "one"),
        ("<p title=\"a\"onclick=\"x\">", "p", "onclick"),
        ("<img src=x onerror=alert(1)", "img", "onerror"),
        // Les attributs d'une balise fermante sont ignorés par le
        // navigateur, mais leurs guillemets délimitent toujours la balise.
        ("</p title=\">\"><img onerror=x>", "img", "onerror"),
    ] {
        assert_eq!(find_active_content(html), handler(t, a), "{html}");
    }
}

#[test]
fn active_urls_are_found_after_decoding_and_url_cleanup() {
    for (html, t, a) in [
        ("<a href=\"javascript:alert(1)\">x</a>", "a", "href"),
        ("<a href=\"JavaScript:alert(1)\">x</a>", "a", "href"),
        ("<a href='  javascript:x'>", "a", "href"),
        ("<a href=java&#9;script:x>", "a", "href"),
        (
            "<a href=\"&#106;&#97;vascript&#58;alert(1)\">x</a>",
            "a",
            "href",
        ),
        ("<a href=\"&#x6A;avascript&colon;x\">", "a", "href"),
        ("<a href=\"&#X6a&#x61vascript:x\">", "a", "href"),
        ("<a href=\"java&Tab;scr&NewLine;ipt:x\">", "a", "href"),
        ("<a href=\"\u{1}\u{1f} javascript:x\">", "a", "href"),
        ("<a href=\"java\rscript:x\">", "a", "href"),
        ("<img src=\"vbscript:x\">", "img", "src"),
        ("<td background=\"javascript:x\">", "td", "background"),
        ("<blockquote CITE=\"javascript:x\">", "blockquote", "cite"),
        // Document en ligne, image SVG (qui peut contenir des scripts).
        (
            "<a href=\"data:text/html;base64,PHNjcmlwdD4=\">",
            "a",
            "href",
        ),
        ("<a href=\" DATA:image/png;base64,iVBOR=\">", "a", "href"),
        (
            "<img src=\"data:image/svg+xml;base64,PHN2Zz4=\">",
            "img",
            "src",
        ),
        ("<img src=\"data:image/png,<svg>\">", "img", "src"),
        ("<img src=\"data:text/html;base64,PHN2Zz4=\">", "img", "src"),
    ] {
        assert_eq!(find_active_content(html), active_url(t, a), "{html}");
    }
}

/// Le contenu de `<style>` s'arrête à sa balise fermante, même au milieu
/// d'une chaîne CSS : ce qui suit est analysé comme du HTML, comme le fait
/// le navigateur.
#[test]
fn style_ends_where_the_browser_ends_it() {
    assert_eq!(
        find_active_content("<style>p{content:'</style><img src=x onerror=alert(1)>'}</style>"),
        handler("img", "onerror")
    );
    assert_eq!(
        find_active_content("<style>a</STYLE\n><p onclick=x>"),
        handler("p", "onclick")
    );
    assert_eq!(
        find_active_content("<style>a</style/><a href=javascript:x>"),
        active_url("a", "href")
    );
    assert_eq!(
        find_active_content("<style>a</styles><p onclick=x></style>"),
        None
    );
}

/// Les autres éléments de texte brut, et ceux dans lesquels le navigateur
/// cesse de traiter `<style>` comme du texte brut, sont signalés d'office.
#[test]
fn other_raw_text_and_template_contexts_are_found() {
    for (html, name) in [
        (
            "<textarea><p title=\"</TEXTAREA ><script>\"></textarea>",
            "textarea",
        ),
        ("<title><p title=\"</title/><img onerror=x>\">", "title"),
        (
            "<xmp><a title=\"</xmp><a href=javascript:x>\"></xmp>",
            "xmp",
        ),
        ("<plaintext><script>alert(1)</script>", "plaintext"),
        ("<noembed><img src=x onerror=alert(1)></noembed>", "noembed"),
        (
            "<noframes><img src=x onerror=alert(1)></noframes>",
            "noframes",
        ),
        (
            "<template><col><style></template><img src=x onerror=alert(1)></style>",
            "template",
        ),
        (
            "<template><col><plaintext></template><img onerror=x>",
            "template",
        ),
    ] {
        assert_eq!(find_active_content(html), tag(name), "{html}");
    }
}

#[test]
fn escaped_or_inert_text_is_not_a_finding() {
    for html in [
        "",
        "texte simple",
        "&lt;script&gt;alert(1)&lt;/script&gt;",
        "<p>&lt;iframe src=x&gt; &lt;img onerror=x&gt;</p>",
        "<p>javascript:alert(1) onclick=x</p>",
        "<p title=\"&lt;script&gt; onclick=x javascript:x\">a</p>",
        "<p title=\"<script> <img onerror=x>\">a</p>",
        "<p title='<script>'>a</p>",
        "<a href=\"https://exemple.fr/?q=javascript:x\" rel=\"noopener noreferrer\">a</a>",
        "<a href=\"https://exemple.fr\" title=\"javascript:x\">a</a>",
        "<img src=\"data:image/png;base64,iVBOR=\" alt=\"onerror\">",
        "<img src=\"DATA:image/JPEG;base64,/9j/4A==\">",
        "<p title=\"data:text/html,x\">a</p>",
        "<p class=\"onclick\">a</p>",
        "<style>p{color:red}</style><p>a</p>",
        "<style>p::before{content:'<script>'}</style>",
        "<!-- <script> --><p>a</p>",
        "<!doctype html><p>a</p>",
        "<? <script> ?>",
        "a < script > b",
        "a <3 b",
        "</ script>",
        "<scripts>",
        // Le nom d'une balise s'arrête à une espace, `/` ou `>` : `<` en
        // fait partie, comme pour le navigateur.
        "<scr<script>ipt>",
        "<iframes>",
        "<formulaire>",
        "<metadata>",
        "<baseline>",
        "<\u{17f}cript>",
        "<p data-on=\"x\">a</p>",
    ] {
        assert_eq!(find_active_content(html), None, "{html}");
    }
}

#[test]
fn findings_are_described() {
    assert_eq!(
        tag("script").map(|f| f.to_string()).as_deref(),
        Some("balise <script>")
    );
    assert_eq!(
        handler("img", "onerror").map(|f| f.to_string()).as_deref(),
        Some("gestionnaire « onerror » sur <img>")
    );
    assert_eq!(
        active_url("a", "href").map(|f| f.to_string()).as_deref(),
        Some("adresse active dans « href » sur <a>")
    );
}

fn hostile_vectors() -> Vec<(String, String)> {
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

/// L'oracle accepte toute sortie de l'assainissement (pas de faux
/// positif) et repère les vecteurs bruts de son périmètre.
#[test]
fn oracle_agrees_with_the_hostile_vectors() {
    let vectors = hostile_vectors();
    assert!(vectors.len() >= 40, "{}", vectors.len());
    for (name, html) in &vectors {
        for remote in [RemoteContent::Blocked, RemoteContent::Allowed] {
            let options = SanitizeOptions {
                message: 1,
                remote,
                content_ids: HashMap::new(),
                protocols: Protocols::custom_schemes(),
            };
            let out = sanitize_html(html, &options).html;
            assert_eq!(find_active_content(&out), None, "{name} : {out}");
        }
    }
    for prefix in [
        "01", "02", "04", "05", "06", "07", "08", "09", "10", "11", "12", "13", "14", "15", "18",
        "19", "20", "21", "31", "38", "39", "40", "41", "42", "43", "44", "46", "47", "48", "49",
        "50", "51", "54", "57",
    ] {
        let (name, html) = vectors
            .iter()
            .find(|(n, _)| n.starts_with(prefix))
            .unwrap_or_else(|| panic!("vecteur {prefix} absent"));
        assert!(
            find_active_content(html).is_some(),
            "{name} : vecteur brut non repéré"
        );
    }
}

fn unexpected(tag: &str, attribute: &str) -> Option<Finding> {
    Some(Finding::UnexpectedAddress {
        tag: tag.to_owned(),
        attribute: attribute.to_owned(),
    })
}

const SOURCES: &[&str] = &["mailpart://localhost/1/", "remote-img://localhost/1/"];

#[test]
fn addresses_outside_the_expected_ones_are_found() {
    for (html, t, a) in [
        ("<img src=\"https://canari.test/p.gif\">", "img", "src"),
        ("<img SRC=\"//canari.test/p.gif\">", "img", "src"),
        ("<img src=\"mailpart://localhost/2/1\">", "img", "src"),
        ("<img src=\"remote-img://localhost/1\">", "img", "src"),
        (
            "<img src=\"data:image/svg+xml;base64,PHN2Zz4=\">",
            "img",
            "src",
        ),
        ("<img src=\"logo.png\">", "img", "src"),
        ("<img src>", "img", "src"),
        ("<a href=\"ftp://x.test/\">x</a>", "a", "href"),
        ("<a href=\"/relatif\">x</a>", "a", "href"),
        ("<a href=\"\">x</a>", "a", "href"),
    ] {
        assert_eq!(
            find_unexpected_address(html, SOURCES),
            unexpected(t, a),
            "{html}"
        );
    }
    assert_eq!(
        find_unexpected_address("<img src=\"remote-img://localhost/1/0\">", &SOURCES[..1]),
        unexpected("img", "src")
    );
}

#[test]
fn expected_addresses_are_accepted() {
    for html in [
        "<img src=\"mailpart://localhost/1/2\"><img src=\"remote-img://localhost/1/0\">",
        "<img src=\"data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7\">",
        "<a href=\"https://exemple.fr/?a=1&amp;b=2\" rel=\"noopener noreferrer\">a</a>",
        "<a href=\"HTTP://exemple.fr\">a</a><a href=\"mailto:a@exemple.fr\">m</a>",
        "<a rel=\"noopener noreferrer\">sans adresse</a>",
        "<p title=\"src=https://canari.test\">&lt;img src=https://canari.test&gt;</p>",
        "<style>p{color:red}</style>",
    ] {
        assert_eq!(find_unexpected_address(html, SOURCES), None, "{html}");
    }
    assert_eq!(
        unexpected("img", "src").map(|f| f.to_string()).as_deref(),
        Some("adresse inattendue dans « src » sur <img>")
    );
}

/// Toute sortie de l'assainissement ne contient que des adresses
/// attendues ; images distantes bloquées : aucune `remote-img://`.
#[test]
fn sanitized_hostile_vectors_only_hold_expected_addresses() {
    for (name, html) in hostile_vectors() {
        for (remote, sources) in [
            (RemoteContent::Blocked, &SOURCES[..1]),
            (RemoteContent::Allowed, SOURCES),
        ] {
            let options = SanitizeOptions {
                message: 1,
                remote,
                content_ids: HashMap::new(),
                protocols: Protocols::custom_schemes(),
            };
            let out = sanitize_html(&html, &options).html;
            assert_eq!(
                find_unexpected_address(&out, sources),
                None,
                "{name} : {out}"
            );
        }
    }
}
