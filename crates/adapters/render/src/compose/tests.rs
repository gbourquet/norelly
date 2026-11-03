use mail_parser::{MessageParser, MimeHeaders};
use norelly_domain::account::AccountId;
use norelly_domain::compose::Threading;

use super::*;

fn address(raw: &str) -> EmailAddress {
    EmailAddress::parse(raw).unwrap_or_else(|_| panic!("{raw}"))
}

fn recipient(name: &str, raw: &str) -> Recipient {
    Recipient::new(name, address(raw))
}

fn draft() -> Draft {
    let mut d = Draft::empty(AccountId::new(1).unwrap_or_else(|_| panic!("compte")));
    d.to = vec![recipient("Bob Dupont", "bob@example.com")];
    d.cc = vec![recipient("", "carole@example.com")];
    d.bcc = vec![recipient("Caché", "cache@example.com")];
    d.subject = "Réunion de rentrée".into();
    d.text = "Bonjour Bob".into();
    d.threading = Threading {
        in_reply_to: Some("m3@example.com".into()),
        references: vec!["m1@example.com".into(), "m3@example.com".into()],
    };
    d
}

fn me() -> Recipient {
    recipient("Élodie Martin", "elodie@example.com")
}

fn composition<'a>(
    from: &'a Recipient,
    d: &'a Draft,
    attachments: &'a [OutgoingAttachment],
) -> Composition<'a> {
    Composition {
        from,
        draft: d,
        attachments,
        date: 1_793_613_600,
        message_id: "abc123@example.com",
    }
}

fn built(c: &Composition<'_>, bcc: BccHeader) -> Vec<u8> {
    build_message(c, bcc).unwrap_or_else(|e| panic!("{e}"))
}

// --- Réassainissement ---

#[test]
fn composed_html_keeps_only_the_editor_formatting() {
    let html = sanitize_composed_html(
        "<p>Un <strong>gras</strong>, <em>italique</em>, <b>b</b>, <i>i</i>, \
         <a href=\"https://example.com/x\">lien</a></p>\
         <ul><li>un</li></ul><ol><li>deux</li></ol>\
         <blockquote><p>cité</p></blockquote><p>ligne<br>suite</p>",
    );
    for kept in [
        "<strong>gras</strong>",
        "<em>italique</em>",
        "<b>b</b>",
        "<i>i</i>",
        "href=\"https://example.com/x\"",
        "<ul><li>un</li></ul>",
        "<ol><li>deux</li></ol>",
        "<blockquote><p>cité</p></blockquote>",
        "<br>",
    ] {
        assert!(html.contains(kept), "{kept} absent de {html}");
    }
}

#[test]
fn composed_html_drops_anything_else() {
    let html = sanitize_composed_html(
        "<p onclick=\"x()\" style=\"color:red\" class=\"c\">a</p>\
         <script>alert(1)</script><style>p{}</style>\
         <img src=\"https://traceur.example/p.gif\">\
         <a href=\"javascript:alert(1)\">j</a><a href=\"data:text/html,x\">d</a>\
         <a href=\"mailto:bob@example.com\">m</a>\
         <iframe src=\"https://x\"></iframe><form><input></form>\
         <h1>titre</h1><table><tr><td>t</td></tr></table>",
    );
    for gone in [
        "onclick",
        "style",
        "class",
        "script",
        "alert",
        "<img",
        "traceur",
        "javascript:",
        "data:",
        "iframe",
        "form",
        "input",
        "<h1",
        "<table",
    ] {
        assert!(!html.contains(gone), "{gone} présent dans {html}");
    }
    assert!(html.contains("href=\"mailto:bob@example.com\""), "{html}");
    assert!(
        html.contains("titre") && html.contains('t'),
        "texte conservé : {html}"
    );
}

// --- Version texte ---

#[test]
fn text_version_follows_the_html_structure() {
    let text = html_to_text(
        "<p>Bonjour <strong>Bob</strong>,</p><p>ligne<br>suite</p>\
         <ul><li>un</li><li>deux</li></ul><ol><li>premier</li><li>second</li></ol>\
         <p><a href=\"https://example.com\">site</a> et \
         <a href=\"https://example.com/y\">https://example.com/y</a></p>\
         <blockquote><p>cité</p><blockquote><p>deux fois</p></blockquote></blockquote>\
         <p>&lt;pas une balise&gt; &amp;</p>",
    );
    assert_eq!(
        text,
        "Bonjour Bob,\n\nligne\nsuite\n\n- un\n- deux\n\n1. premier\n2. second\n\n\
         site <https://example.com> et https://example.com/y\n\n\
         > cité\n>\n>> deux fois\n\n<pas une balise> &\n"
    );
}

// --- Construction MIME ---

#[test]
fn plain_message_has_all_headers() {
    let from = me();
    let mut d = draft();
    d.html = None;
    let raw = built(&composition(&from, &d, &[]), BccHeader::Omit);
    let m = MessageParser::default()
        .parse(&raw)
        .unwrap_or_else(|| panic!("illisible"));
    assert_eq!(m.subject(), Some("Réunion de rentrée"));
    let first = |a: Option<&mail_parser::Address<'_>>| {
        a.and_then(|a| a.first()).map(|a| {
            (
                a.name().unwrap_or_default().to_owned(),
                a.address().unwrap_or_default().to_owned(),
            )
        })
    };
    assert_eq!(
        first(m.from()),
        Some(("Élodie Martin".into(), "elodie@example.com".into()))
    );
    assert_eq!(
        first(m.to()),
        Some(("Bob Dupont".into(), "bob@example.com".into()))
    );
    assert_eq!(
        first(m.cc()).map(|a| a.1),
        Some("carole@example.com".into())
    );
    assert!(m.bcc().is_none(), "Bcc jamais transmis");
    assert!(!String::from_utf8_lossy(&raw).contains("cache@example.com"));
    assert_eq!(m.message_id(), Some("abc123@example.com"));
    assert_eq!(m.in_reply_to().as_text(), Some("m3@example.com"));
    assert_eq!(
        m.references()
            .as_text_list()
            .map(|l| l.iter().map(|r| r.as_ref()).collect::<Vec<&str>>()),
        Some(vec!["m1@example.com", "m3@example.com"])
    );
    assert_eq!(
        m.date().map(mail_parser::DateTime::to_timestamp),
        Some(1_793_613_600)
    );
    assert_eq!(
        m.body_text(0).as_deref().map(str::trim_end),
        Some("Bonjour Bob")
    );
    assert!(
        m.body_html(0).is_none_or(|h| !h.contains("<p")),
        "texte seul"
    );
    let ct = m
        .content_type()
        .map(|c| (c.ctype().to_owned(), c.subtype().map(str::to_owned)));
    assert_eq!(ct, Some(("text".into(), Some("plain".into()))));
    // Ni nom de machine ni logiciel divulgués.
    let head = String::from_utf8_lossy(&raw).to_lowercase();
    assert!(!head.contains("user-agent") && !head.contains("x-mailer"));
}

#[test]
fn local_copies_keep_bcc() {
    let from = me();
    let d = draft();
    let raw = built(&composition(&from, &d, &[]), BccHeader::Keep);
    let m = MessageParser::default()
        .parse(&raw)
        .unwrap_or_else(|| panic!("illisible"));
    assert_eq!(
        m.bcc().and_then(|a| a.first()).and_then(|a| a.address()),
        Some("cache@example.com")
    );
}

#[test]
fn html_message_is_alternative_with_sanitized_html_and_derived_text() {
    let from = me();
    let mut d = draft();
    d.html = Some("<p>Bonjour <strong>Bob</strong></p><script>alert(1)</script>".into());
    d.text = "texte ignoré".into();
    let raw = built(&composition(&from, &d, &[]), BccHeader::Omit);
    let m = MessageParser::default()
        .parse(&raw)
        .unwrap_or_else(|| panic!("illisible"));
    let ct = m
        .content_type()
        .map(|c| (c.ctype().to_owned(), c.subtype().map(str::to_owned)));
    assert_eq!(ct, Some(("multipart".into(), Some("alternative".into()))));
    let html = m.body_html(0).unwrap_or_default();
    assert!(html.contains("<strong>Bob</strong>"), "{html}");
    assert!(
        !html.contains("script") && !html.contains("alert"),
        "{html}"
    );
    assert_eq!(
        m.body_text(0).as_deref().map(str::trim_end),
        Some("Bonjour Bob")
    );
}

#[test]
fn attachments_make_a_mixed_message() {
    let from = me();
    let mut d = draft();
    d.html = Some("<p>Ci-joint</p>".into());
    let files = [
        OutgoingAttachment {
            file_name: "../../etc/passwd\r\nX-Injecté: 1".into(),
            content_type: "application/pdf".into(),
            bytes: vec![0, 159, 146, 150, 255],
        },
        OutgoingAttachment {
            file_name: "notes été.txt".into(),
            content_type: "pas un type".into(),
            bytes: b"bonjour".to_vec(),
        },
    ];
    let raw = built(&composition(&from, &d, &files), BccHeader::Omit);
    let m = MessageParser::default()
        .parse(&raw)
        .unwrap_or_else(|| panic!("illisible"));
    let ct = m
        .content_type()
        .map(|c| (c.ctype().to_owned(), c.subtype().map(str::to_owned)));
    assert_eq!(ct, Some(("multipart".into(), Some("mixed".into()))));
    assert!(
        m.header_raw("X-Injecté").is_none(),
        "pas d'injection d'en-tête"
    );
    let parts: Vec<_> = m.attachments().collect();
    assert_eq!(parts.len(), 2);
    let name = parts[0].attachment_name().unwrap_or_default();
    assert!(!name.contains(['/', '\\', '\r', '\n']), "{name:?}");
    assert_eq!(parts[0].contents(), &[0, 159, 146, 150, 255]);
    assert_eq!(
        parts[0]
            .content_type()
            .map(|c| c.subtype().unwrap_or_default().to_owned()),
        Some("pdf".into())
    );
    assert_eq!(parts[1].attachment_name(), Some("notes été.txt"));
    assert_eq!(parts[1].contents(), b"bonjour");
    assert_eq!(
        parts[1].content_type().map(|c| c.ctype().to_owned()),
        Some("application".into()),
        "type invalide : application/octet-stream"
    );
    assert_eq!(
        m.body_text(0).as_deref().map(str::trim_end),
        Some("Ci-joint")
    );
}

#[test]
fn envelope_lists_every_recipient_but_the_message_hides_bcc() {
    let from = me();
    let d = draft();
    let out = outgoing(&composition(&from, &d, &[])).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(out.from, "elodie@example.com");
    assert_eq!(
        out.recipients,
        ["bob@example.com", "carole@example.com", "cache@example.com"]
    );
    assert!(!String::from_utf8_lossy(&out.raw).contains("cache@example.com"));
}

#[test]
fn message_ids_are_random_and_in_the_sender_domain() {
    let a = new_message_id(&address("x@example.com")).unwrap_or_else(|e| panic!("{e}"));
    let b = new_message_id(&address("x@example.com")).unwrap_or_else(|e| panic!("{e}"));
    assert_ne!(a, b);
    let (left, right) = a.split_once('@').unwrap_or_else(|| panic!("{a}"));
    assert_eq!(right, "example.com");
    assert_eq!(left.len(), 32);
    assert!(left.bytes().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn deeply_nested_html_does_not_overflow_the_stack() {
    let depth = 200_000;
    let html = format!(
        "{}profond{}",
        "<blockquote><ul><li>".repeat(depth),
        "</li></ul></blockquote>".repeat(depth)
    );
    // Exécuté sur un fil à petite pile : une récursion non bornée y
    // déborderait.
    let text = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || html_to_text(&html))
        .unwrap_or_else(|e| panic!("{e}"))
        .join()
        .unwrap_or_else(|_| panic!("débordement de pile"));
    assert!(text.contains("profond"), "texte conservé");
}

proptest::proptest! {
    #[test]
    fn text_version_never_panics(html in "(<(/)?(p|a|ul|ol|li|br|blockquote|strong)( href=\"[a-z:/.]{0,12}\")?>|[a-z &;<>\"]{0,6}){0,60}") {
        let text = html_to_text(&html);
        proptest::prop_assert!(text.ends_with('\n'));
    }
}
