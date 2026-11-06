use crate::account::AccountId;
use proptest::prelude::*;

use super::*;

fn who(name: &str, address: &str) -> Correspondent {
    Correspondent::new(name, address)
}

fn me() -> Vec<EmailAddress> {
    ["moi@example.com", "alias@exemple.fr"]
        .iter()
        .map(|a| EmailAddress::parse(a).unwrap_or_else(|_| panic!("adresse {a}")))
        .collect()
}

fn addresses(list: &[Correspondent]) -> Vec<&str> {
    list.iter().map(|c| c.address.as_str()).collect()
}

// --- Sujets ---

#[test]
fn reply_and_forward_subjects_do_not_stack_prefixes() {
    assert_eq!(reply_subject("Réunion"), "Re: Réunion");
    assert_eq!(reply_subject("Re: Réunion"), "Re: Réunion");
    assert_eq!(reply_subject("RE: Re: re: Réunion"), "Re: Réunion");
    assert_eq!(reply_subject("Fwd: Réunion"), "Re: Réunion");
    assert_eq!(forward_subject("Re: Réunion"), "Fwd: Réunion");
    assert_eq!(forward_subject("Fwd: Fw: Réunion"), "Fwd: Réunion");
    assert_eq!(reply_subject(""), "Re: ");
    assert_eq!(reply_subject("   "), "Re: ");
}

#[test]
fn foreign_prefixes_are_recognized_and_replaced() {
    for subject in [
        "TR: Réunion",
        "AW: Réunion",
        "WG: Réunion",
        "SV: Réunion",
        "VS: Réunion",
        "Antw: Réunion",
        "RIF: Réunion",
        "R: Réunion",
        "RE : Réunion",
        "Re[2]: Réunion",
        "Re：Réunion",
        "Réf. Réunion",
    ] {
        let expected = if subject.starts_with("Réf.") {
            // Pas un préfixe (pas de deux-points) : conservé.
            "Re: Réf. Réunion"
        } else {
            "Re: Réunion"
        };
        assert_eq!(reply_subject(subject), expected, "{subject}");
    }
}

#[test]
fn list_tags_and_inner_words_are_kept() {
    assert_eq!(
        reply_subject("[liste] Re: Réunion"),
        "Re: [liste] Re: Réunion"
    );
    assert_eq!(reply_subject("Retour: suite"), "Re: Retour: suite");
    assert_eq!(strip_reply_prefixes("Re: Re: x"), "x");
}

fn prefix() -> impl Strategy<Value = String> {
    (
        prop::sample::select(vec![
            "re", "RE", "Re", "fwd", "Fwd", "FW", "tr", "TR", "aw", "AW", "wg", "sv", "vs", "antw",
            "Antw", "rif", "RIF", "r",
        ]),
        prop::sample::select(vec!["", " "]),
        prop::sample::select(vec![":", "："]),
    )
        .prop_map(|(word, space, colon)| format!("{word}{space}{colon} "))
}

proptest! {
    /// Quel que soit l'empilement de préfixes reconnus, un seul `Re:`.
    #[test]
    fn stacked_known_prefixes_collapse(
        prefixes in prop::collection::vec(prefix(), 0..6),
        base in "[a-zA-Zéè0-9 ]{1,30}",
    ) {
        let base = strip_reply_prefixes(base.trim()).to_owned();
        let subject = format!("{}{base}", prefixes.concat());
        prop_assert_eq!(reply_subject(&subject), format!("Re: {base}"));
        prop_assert_eq!(forward_subject(&subject), format!("Fwd: {base}"));
    }

    /// Répondre à une réponse ne change plus le sujet ; jamais de panique.
    #[test]
    fn reply_subject_is_idempotent(subject in "\\PC{0,80}") {
        let once = reply_subject(&subject);
        prop_assert!(once.starts_with(REPLY_PREFIX));
        prop_assert_eq!(reply_subject(&once), once.clone());
        let fwd = forward_subject(&subject);
        prop_assert!(fwd.starts_with(FORWARD_PREFIX));
        prop_assert_eq!(forward_subject(&fwd), fwd.clone());
    }
}

// --- Destinataires ---

fn source() -> ReplySource {
    ReplySource {
        subject: "Réunion".into(),
        from: vec![who("Alice", "alice@example.com")],
        to: vec![
            who("Moi", "MOI@Example.com"),
            who("Bob", "bob@example.com"),
            who("", "pas une adresse"),
        ],
        cc: vec![
            who("Carole", "carole@example.com"),
            who("Bob bis", "Bob@EXAMPLE.com"),
            who("Alias", "alias@exemple.fr"),
        ],
        reply_to: vec![],
        message_id: Some("m3@example.com".into()),
        in_reply_to: vec!["m2@example.com".into()],
        references: vec!["m1@example.com".into(), "m2@example.com".into()],
        body: String::new(),
    }
}

#[test]
fn reply_goes_to_the_sender_only() {
    let r = reply_recipients(&source(), ReplyKind::Reply, &me());
    assert_eq!(addresses(&r.to), ["alice@example.com"]);
    assert!(r.cc.is_empty());
    assert_eq!(r.to[0].name, "Alice");
}

#[test]
fn reply_to_header_takes_precedence() {
    let mut s = source();
    s.reply_to = vec![who("Liste", "liste@example.org")];
    let r = reply_recipients(&s, ReplyKind::Reply, &me());
    assert_eq!(addresses(&r.to), ["liste@example.org"]);
    let all = reply_recipients(&s, ReplyKind::ReplyAll, &me());
    assert_eq!(
        addresses(&all.to),
        ["liste@example.org", "bob@example.com"],
        "Reply-To remplace l'expéditeur"
    );
}

#[test]
fn reply_all_excludes_own_addresses_and_duplicates() {
    let r = reply_recipients(&source(), ReplyKind::ReplyAll, &me());
    assert_eq!(addresses(&r.to), ["alice@example.com", "bob@example.com"]);
    // Bob déjà en « À » (casse différente), alias exclu, invalide ignoré.
    assert_eq!(addresses(&r.cc), ["carole@example.com"]);
}

#[test]
fn replying_to_own_message_goes_to_its_recipients() {
    let mut s = source();
    s.from = vec![who("Moi", "moi@example.com")];
    s.to = vec![who("Bob", "bob@example.com")];
    let r = reply_recipients(&s, ReplyKind::Reply, &me());
    assert_eq!(addresses(&r.to), ["bob@example.com"]);
    let all = reply_recipients(&s, ReplyKind::ReplyAll, &me());
    assert_eq!(addresses(&all.to), ["bob@example.com"]);
    assert_eq!(addresses(&all.cc), ["carole@example.com"]);
}

#[test]
fn forward_has_no_recipients() {
    assert_eq!(
        reply_recipients(&source(), ReplyKind::Forward, &me()),
        Recipients::default()
    );
}

// --- Fil ---

#[test]
fn threading_appends_the_message_id_to_references() {
    let t = reply_threading(&source());
    assert_eq!(t.in_reply_to.as_deref(), Some("m3@example.com"));
    assert_eq!(
        t.references,
        ["m1@example.com", "m2@example.com", "m3@example.com"]
    );
}

#[test]
fn threading_falls_back_to_in_reply_to_and_is_bounded() {
    let mut s = source();
    s.references.clear();
    let t = reply_threading(&s);
    assert_eq!(t.references, ["m2@example.com", "m3@example.com"]);

    s.references = (0..100).map(|n| format!("r{n}@x")).collect();
    let t = reply_threading(&s);
    assert_eq!(t.references.len(), MAX_REFERENCES);
    assert_eq!(
        t.references.first().map(String::as_str),
        Some("r0@x"),
        "racine gardée"
    );
    assert_eq!(
        t.references.last().map(String::as_str),
        Some("m3@example.com")
    );
    assert_eq!(t.references.get(1).map(String::as_str), Some("r82@x"));

    s.message_id = None;
    s.references = vec!["a@x".into()];
    let t = reply_threading(&s);
    assert_eq!(t.in_reply_to, None);
    assert_eq!(t.references, ["a@x"]);
}

#[test]
fn threading_does_not_repeat_identifiers() {
    let mut s = source();
    s.references = vec!["m1@x".into(), "m1@x".into(), "m3@example.com".into()];
    let t = reply_threading(&s);
    assert_eq!(t.references, ["m1@x", "m3@example.com"]);
}

// --- Citation ---

#[test]
fn quote_prefixes_every_line_under_the_attribution() {
    let quoted = quote(
        "Bonjour,\r\n\nÀ demain\n> déjà cité",
        "Le 2 nov. 2026, Alice a écrit :",
    );
    assert_eq!(
        quoted,
        "Le 2 nov. 2026, Alice a écrit :\n> Bonjour,\n>\n> À demain\n>> déjà cité\n"
    );
    assert_eq!(quote("", "X :"), "X :\n>\n");
}

/// Un identifiant reçu non conforme (non
/// ASCII, trop long) est ignoré, il n'empêche pas de répondre.
#[test]
fn invalid_received_identifiers_are_dropped_from_threading() {
    let mut s = source();
    s.message_id = Some("é@x".into());
    s.references = vec!["ok@x".into(), "y".repeat(300), "b<c@x".into()];
    let t = reply_threading(&s);
    assert_eq!(t.in_reply_to, None);
    assert_eq!(t.references, ["ok@x"]);
    let mut d = super::draft::Draft::empty(AccountId::new(1).unwrap_or_else(|_| panic!("compte")));
    d.threading = t;
    assert_eq!(d.check_storable(), Ok(()));
}

#[test]
fn a_resumed_draft_keeps_its_own_threading() {
    let mut s = source();
    s.in_reply_to = vec!["pas conforme".into(), "parent@example.com".into()];
    s.references = (0..30).map(|i| format!("r{i}@example.com")).collect();
    let t = resumed_threading(&s);
    assert_eq!(t.in_reply_to.as_deref(), Some("parent@example.com"));
    assert_eq!(t.references.len(), MAX_REFERENCES);
    assert_eq!(
        t.references.first().map(String::as_str),
        Some("r0@example.com")
    );
    assert_eq!(
        t.references.last().map(String::as_str),
        Some("r29@example.com")
    );
    assert_eq!(
        resumed_threading(&ReplySource::default()),
        Threading::default()
    );
}
