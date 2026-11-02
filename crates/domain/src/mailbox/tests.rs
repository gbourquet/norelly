use super::*;

fn name(raw: &str) -> MailboxName {
    MailboxName::parse(raw).unwrap_or_else(|e| panic!("{raw} : {e}"))
}

fn listed(raw: &str, delimiter: Option<char>, attributes: &[&str]) -> ListedMailbox {
    ListedMailbox {
        name: name(raw),
        delimiter,
        attributes: attributes.iter().map(|a| (*a).to_owned()).collect(),
    }
}

#[test]
fn identifiers_reject_zero() {
    assert!(Uid::new(0).is_none());
    assert_eq!(Uid::new(7).map(Uid::get), Some(7));
    assert!(UidValidity::new(0).is_none());
    assert!(ModSeq::new(0).is_none());
    assert!(ModSeq::new(ModSeq::MAX + 1).is_none());
    assert!(ModSeq::new(ModSeq::MAX).is_some());
    assert_eq!(Uid::new(u32::MAX).and_then(Uid::next), None);
}

#[test]
fn flags_from_tokens() {
    let f = Flags::from_tokens([
        "\\SEEN",
        "\\Flagged",
        "\\Recent",
        "\\Inconnu",
        "$Forwarded",
        "bad(kw",
    ]);
    assert!(f.seen && f.flagged && !f.answered && !f.deleted && !f.draft);
    assert_eq!(
        f.keywords().iter().map(Keyword::as_str).collect::<Vec<_>>(),
        ["$Forwarded"]
    );
    assert_eq!(f.to_tokens(), ["\\Seen", "\\Flagged", "$Forwarded"]);
}

#[test]
fn flags_bits_roundtrip() {
    let f = Flags::from_tokens(["\\Seen", "\\Answered", "\\Deleted", "\\Draft", "Travail"]);
    let back = Flags::from_parts(f.system_bits(), f.keywords().iter().map(Keyword::as_str));
    assert_eq!(back, f);
    // Bits inconnus et mots-clés invalides ignorés.
    let g = Flags::from_parts(0xff, ["ok", "", "a b", "\\Seen"]);
    assert_eq!(g.system_bits(), 0x1f);
    assert_eq!(g.keywords().len(), 1);
}

#[test]
fn keywords_are_bounded() {
    let many: Vec<String> = (0..1000).map(|i| format!("k{i}")).collect();
    assert_eq!(Flags::from_tokens(&many).keywords().len(), MAX_KEYWORDS);
    assert!(Keyword::parse(&"a".repeat(MAX_KEYWORD_LEN + 1)).is_none());
    assert!(Keyword::parse("é").is_none());
}

#[test]
fn mailbox_name_validation() {
    assert_eq!(name("inbox"), MailboxName::inbox());
    assert!(name("InBoX").is_inbox());
    assert_eq!(MailboxName::parse(""), Err(MailboxNameError::Empty));
    assert_eq!(
        MailboxName::parse("a\r\nb"),
        Err(MailboxNameError::InvalidCharacter)
    );
    assert_eq!(
        MailboxName::parse("a\u{202E}b"),
        Err(MailboxNameError::InvalidCharacter)
    );
    assert_eq!(
        MailboxName::parse(&"x".repeat(MAX_MAILBOX_NAME_LEN + 1)),
        Err(MailboxNameError::TooLong)
    );
    assert_eq!(name("Travail/Projets/2026").leaf(Some('/')), "2026");
    assert_eq!(name("Travail/Projets").leaf(None), "Travail/Projets");
}

#[test]
fn role_detection_prefers_special_use() {
    assert_eq!(
        MailboxRole::detect(&name("INBOX"), Some('/'), ["\\Sent"]),
        MailboxRole::Inbox
    );
    assert_eq!(
        MailboxRole::detect(
            &name("Mes envois"),
            Some('/'),
            ["\\HasNoChildren", "\\Sent"]
        ),
        MailboxRole::Sent
    );
    assert_eq!(
        MailboxRole::detect(&name("[Gmail]/Tous les messages"), Some('/'), ["\\All"]),
        MailboxRole::All
    );
}

#[test]
fn role_detection_by_name() {
    for (raw, role) in [
        ("Sent", MailboxRole::Sent),
        ("Éléments envoyés", MailboxRole::Sent),
        ("Brouillons", MailboxRole::Drafts),
        ("Corbeille", MailboxRole::Trash),
        ("Deleted Items", MailboxRole::Trash),
        ("Spam", MailboxRole::Junk),
        ("Courrier indésirable", MailboxRole::Junk),
        ("Archives", MailboxRole::Archive),
        ("INBOX.Sent", MailboxRole::Sent),
        ("Factures", MailboxRole::Normal),
    ] {
        assert_eq!(
            MailboxRole::detect(&name(raw), Some('.'), Vec::<String>::new()),
            role,
            "{raw}"
        );
    }
    // Trop profond pour l'heuristique : un sous-dossier « Sent » d'un
    // projet n'est pas la boîte des envoyés.
    assert_eq!(
        MailboxRole::detect(
            &name("Projets/Client/Sent"),
            Some('/'),
            Vec::<String>::new()
        ),
        MailboxRole::Normal
    );
}

#[test]
fn role_strings_roundtrip() {
    for role in [
        MailboxRole::Inbox,
        MailboxRole::Sent,
        MailboxRole::Drafts,
        MailboxRole::Trash,
        MailboxRole::Junk,
        MailboxRole::Archive,
        MailboxRole::All,
        MailboxRole::Flagged,
        MailboxRole::Normal,
    ] {
        assert_eq!(MailboxRole::parse(role.as_str()), Some(role));
    }
    assert_eq!(MailboxRole::parse("autre"), None);
}

#[test]
fn classify_dedupes_names_and_roles() {
    let out = classify_mailboxes(vec![
        listed("INBOX", Some('/'), &[]),
        listed("Sent", Some('/'), &[]),
        listed("Sent Items", Some('/'), &["\\Sent"]),
        listed("Sent Items", Some('/'), &["\\Sent"]),
        listed("Trash", Some('/'), &["\\Trash"]),
        listed("Corbeille", Some('/'), &["\\Trash"]),
        listed("[Gmail]", Some('/'), &["\\Noselect"]),
        listed("Fantôme", Some('/'), &["\\NonExistent"]),
    ]);
    let summary: Vec<(&str, MailboxRole, bool)> = out
        .iter()
        .map(|m| (m.name.as_str(), m.role, m.selectable))
        .collect();
    assert_eq!(
        summary,
        [
            ("INBOX", MailboxRole::Inbox, true),
            ("Sent", MailboxRole::Normal, true),
            ("Sent Items", MailboxRole::Sent, true),
            ("Trash", MailboxRole::Trash, true),
            ("Corbeille", MailboxRole::Normal, true),
            ("[Gmail]", MailboxRole::Normal, false),
            ("Fantôme", MailboxRole::Normal, false),
        ]
    );
}

#[test]
fn header_text_is_sanitized_and_bounded() {
    assert_eq!(header_text("  a\r\n\tb\u{202E}c  ", 100), "a b c");
    assert_eq!(header_text(&"é".repeat(2000), 10).chars().count(), 10);
    assert_eq!(header_text("", 10), "");
}

#[test]
fn headers_sanitize() {
    let raw = RawHeaders {
        subject: "Réunion\r\nBcc: victime@example.com",
        from: vec![("Alice \u{202E}", "alice@example.com")],
        to: vec![("", "bob @example.com"); 500],
        message_id: Some(" <abc@x> "),
        in_reply_to: vec!["<p@x>", "<>"],
        references: vec!["<r1@x>", "<r2@x>"],
        ..RawHeaders::default()
    };
    let h = MessageHeaders::sanitize(&raw);
    assert_eq!(h.subject, "Réunion Bcc: victime@example.com");
    assert_eq!(h.from.first().map(|c| c.name.as_str()), Some("Alice"));
    assert_eq!(h.to.len(), MAX_ADDRESSES);
    assert_eq!(
        h.to.first().map(|c| c.address.as_str()),
        Some("bob@example.com")
    );
    assert_eq!(h.message_id.as_deref(), Some("abc@x"));
    assert_eq!(h.in_reply_to, ["p@x"]);
    assert_eq!(h.references, ["r1@x", "r2@x"]);
}

#[test]
fn sort_date_rejects_future_header_dates() {
    let mut meta = MessageMeta {
        uid: Uid::new(1).unwrap_or_else(|| panic!("uid")),
        flags: Flags::default(),
        modseq: None,
        internal_date: 1_000_000,
        size: 10,
        headers: MessageHeaders::default(),
        structure: None,
    };
    assert_eq!(meta.sort_date(), 1_000_000);
    meta.headers.date = Some(999_000);
    assert_eq!(meta.sort_date(), 999_000);
    meta.headers.date = Some(5_000_000_000);
    assert_eq!(meta.sort_date(), 1_000_000, "date d'en-tête dans le futur");
    meta.headers.date = Some(-5);
    assert_eq!(meta.sort_date(), 1_000_000);
}
