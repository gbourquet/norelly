use norelly_domain::fetch_policy::PartPath;
use norelly_domain::ports::MAX_INDEXED_TEXT_CHARS;

use super::*;

fn text_of(raw: &str) -> String {
    MimeTextExtractor.searchable_text(&FetchedBody::Full(raw.as_bytes().to_vec()), None)
}

fn path(raw: &str) -> PartPath {
    PartPath::parse(raw).unwrap_or_else(|| panic!("chemin {raw}"))
}

#[test]
fn plain_text_message() {
    let text = text_of(
        "Subject: Bonjour\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nLes cerises sont mûres.\r\n",
    );
    assert!(text.contains("Les cerises sont mûres."), "{text:?}");
}

#[test]
fn quoted_printable_and_charset_are_decoded() {
    let text = text_of(
        "Content-Type: text/plain; charset=iso-8859-1\r\n\
         Content-Transfer-Encoding: quoted-printable\r\n\r\n\
         D=E9j=E0 vu\r\n",
    );
    assert!(text.contains("Déjà vu"), "{text:?}");
}

#[test]
fn html_only_message_is_reduced_to_text_without_scripts_or_styles() {
    let text = text_of(
        "Content-Type: text/html; charset=utf-8\r\n\r\n\
         <html><head><style>.x { color: rouge }</style></head>\
         <body><script>alert('piege')</script><p>Bonjour <b>monde</b></p>\
         <p>R&eacute;union &amp; d&eacute;jeuner</p></body></html>",
    );
    assert!(text.contains("Bonjour"), "{text:?}");
    assert!(text.contains("monde"), "{text:?}");
    assert!(text.contains("Réunion & déjeuner"), "{text:?}");
    assert!(!text.contains("piege"), "{text:?}");
    assert!(!text.contains("rouge"), "{text:?}");
    assert!(!text.contains('<'), "{text:?}");
}

#[test]
fn alternative_keeps_the_text_part_once() {
    let text = text_of(
        "Content-Type: multipart/alternative; boundary=b\r\n\r\n\
         --b\r\nContent-Type: text/plain\r\n\r\nversion texte\r\n\
         --b\r\nContent-Type: text/html\r\n\r\n<p>version html</p>\r\n\
         --b--\r\n",
    );
    assert!(text.contains("version texte"), "{text:?}");
    assert!(!text.contains("version html"), "{text:?}");
}

#[test]
fn attachments_are_not_indexed() {
    let text = text_of(
        "Content-Type: multipart/mixed; boundary=b\r\n\r\n\
         --b\r\nContent-Type: text/plain\r\n\r\ncorps visible\r\n\
         --b\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=n.txt\r\n\r\n\
         contenu joint\r\n\
         --b--\r\n",
    );
    assert!(text.contains("corps visible"), "{text:?}");
    assert!(!text.contains("contenu joint"), "{text:?}");
}

#[test]
fn partial_body_uses_the_downloaded_text_part() {
    let header = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n".to_vec();
    let entity = b"Content-Type: text/plain; charset=utf-8\r\n\r\ntexte partiel\r\n".to_vec();
    let body = FetchedBody::Partial {
        header,
        parts: vec![(path("1"), entity)],
    };
    let text = MimeTextExtractor.searchable_text(&body, None);
    assert!(text.contains("texte partiel"), "{text:?}");
}

#[test]
fn text_is_bounded() {
    let raw = format!(
        "Content-Type: text/plain\r\n\r\n{}",
        "mot ".repeat(MAX_INDEXED_TEXT_CHARS)
    );
    let text = text_of(&raw);
    assert!(text.starts_with("mot mot"));
    assert!(text.chars().count() <= MAX_INDEXED_TEXT_CHARS);
}

#[test]
fn arbitrary_bytes_do_not_panic() {
    for raw in [
        &b""[..],
        b"\xff\xfe\x00",
        b"Content-Type: text/html\r\n\r\n<<<<",
        b"\r\n\r\n",
    ] {
        let _ = MimeTextExtractor.searchable_text(&FetchedBody::Full(raw.to_vec()), None);
    }
}

/// Un HTML dense de 2 Mio est réduit au
/// texte en temps raisonnable (le délai maximal du cas d'usage reste le
/// filet de sécurité).
#[test]
fn hostile_html_stays_fast() {
    let started = std::time::Instant::now();
    let html = "<p><b>mot</b> <i>autre</i></p></q>".repeat(60_000);
    let raw = format!("Content-Type: text/html\r\n\r\n{html}");
    let text = text_of(&raw);
    assert!(text.chars().count() <= MAX_INDEXED_TEXT_CHARS);
    let elapsed = started.elapsed();
    eprintln!("HTML hostile : {elapsed:?}");
    assert!(elapsed < std::time::Duration::from_secs(5), "{elapsed:?}");
}
