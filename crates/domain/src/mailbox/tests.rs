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

// ---------------------------------------------------------------------------
// Nouveau dossier
// ---------------------------------------------------------------------------

fn remote(raw: &str, delimiter: Option<char>) -> RemoteMailbox {
    RemoteMailbox {
        name: name(raw),
        delimiter,
        role: MailboxRole::Normal,
        selectable: true,
    }
}

fn plain_account() -> Vec<RemoteMailbox> {
    vec![
        remote("INBOX", Some('/')),
        remote("Sent", Some('/')),
        remote("Projets", Some('/')),
        remote("Projets/2026", Some('/')),
    ]
}

#[test]
fn new_folder_at_top_level() {
    let existing = plain_account();
    assert_eq!(
        new_folder_name("Factures", None, &existing),
        Ok(name("Factures"))
    );
}

#[test]
fn new_folder_label_is_trimmed() {
    let existing = plain_account();
    assert_eq!(
        new_folder_name("  Voyages été  ", None, &existing),
        Ok(name("Voyages été"))
    );
}

#[test]
fn new_folder_under_parent_uses_its_delimiter() {
    let existing = vec![remote("INBOX", Some('.')), remote("Projets", Some('.'))];
    assert_eq!(
        new_folder_name("2027", Some(&name("Projets")), &existing),
        Ok(name("Projets.2027"))
    );
}

#[test]
fn new_folder_follows_inbox_prefix_of_the_server() {
    // Courier, Cyrus : tous les dossiers personnels sont sous « INBOX. ».
    let existing = vec![
        remote("INBOX", Some('.')),
        remote("INBOX.Sent", Some('.')),
        remote("INBOX.Trash", Some('.')),
    ];
    assert_eq!(
        new_folder_name("Factures", None, &existing),
        Ok(name("INBOX.Factures"))
    );
}

#[test]
fn new_folder_without_prefix_when_only_inbox_exists() {
    let existing = vec![remote("INBOX", Some('/'))];
    assert_eq!(
        new_folder_name("Factures", None, &existing),
        Ok(name("Factures"))
    );
}

#[test]
fn new_folder_rejects_empty_label() {
    let existing = plain_account();
    assert_eq!(
        new_folder_name("   ", None, &existing),
        Err(NewFolderError::Empty)
    );
}

#[test]
fn new_folder_rejects_too_long_label() {
    let existing = plain_account();
    let long = "é".repeat(MAX_FOLDER_LABEL_CHARS + 1);
    assert_eq!(
        new_folder_name(&long, None, &existing),
        Err(NewFolderError::TooLong)
    );
    let max = "é".repeat(MAX_FOLDER_LABEL_CHARS);
    assert!(new_folder_name(&max, None, &existing).is_ok());
}

#[test]
fn new_folder_rejects_forbidden_characters() {
    let existing = plain_account();
    for label in ["a*b", "a%b", "a\tb", "a\u{202e}b", "a\u{0}b"] {
        assert_eq!(
            new_folder_name(label, None, &existing),
            Err(NewFolderError::InvalidCharacter),
            "{label:?}"
        );
    }
}

#[test]
fn new_folder_rejects_the_hierarchy_delimiter() {
    let existing = plain_account();
    assert_eq!(
        new_folder_name("Clients/Dupont", None, &existing),
        Err(NewFolderError::ContainsDelimiter('/'))
    );
}

#[test]
fn new_folder_rejects_existing_name_ignoring_case() {
    let existing = plain_account();
    for label in ["Projets", "projets", "inbox", "INBOX"] {
        assert_eq!(
            new_folder_name(label, None, &existing),
            Err(NewFolderError::AlreadyExists),
            "{label}"
        );
    }
    assert_eq!(
        new_folder_name("2026", Some(&name("Projets")), &existing),
        Err(NewFolderError::AlreadyExists)
    );
}

#[test]
fn new_folder_rejects_unknown_parent() {
    let existing = plain_account();
    assert_eq!(
        new_folder_name("x", Some(&name("Absent")), &existing),
        Err(NewFolderError::UnknownParent)
    );
}

#[test]
fn new_folder_under_parent_needs_a_hierarchy() {
    let existing = vec![remote("INBOX", None), remote("Projets", None)];
    assert_eq!(
        new_folder_name("x", Some(&name("Projets")), &existing),
        Err(NewFolderError::NoHierarchy)
    );
    // Au premier niveau, un serveur sans hiérarchie convient.
    assert_eq!(new_folder_name("x", None, &existing), Ok(name("x")));
}

#[test]
fn folder_label_is_checked_alone() {
    assert_eq!(folder_label("  Factures "), Ok("Factures"));
    assert_eq!(folder_label(""), Err(NewFolderError::Empty));
    assert_eq!(
        folder_label(&"x".repeat(MAX_FOLDER_LABEL_CHARS + 1)),
        Err(NewFolderError::TooLong)
    );
    assert_eq!(folder_label("a%b"), Err(NewFolderError::InvalidCharacter));
}

#[test]
fn new_folder_rejects_virtual_parents() {
    let mut all = remote("[Gmail]/Tous", Some('/'));
    all.role = MailboxRole::All;
    let mut flagged = remote("[Gmail]/Suivis", Some('/'));
    flagged.role = MailboxRole::Flagged;
    let existing = vec![remote("INBOX", Some('/')), all, flagged];
    for parent in ["[Gmail]/Tous", "[Gmail]/Suivis"] {
        assert_eq!(
            new_folder_name("x", Some(&name(parent)), &existing),
            Err(NewFolderError::VirtualParent),
            "{parent}"
        );
    }
}

// ---------------------------------------------------------------------------
// Renommer et supprimer un dossier
// ---------------------------------------------------------------------------

fn special(raw: &str, role: MailboxRole) -> RemoteMailbox {
    RemoteMailbox {
        role,
        ..remote(raw, Some('/'))
    }
}

#[test]
fn renamed_folder_keeps_its_place() {
    let existing = plain_account();
    assert_eq!(
        renamed_folder_name(&name("Projets"), " Chantiers ", &existing),
        Ok(name("Chantiers"))
    );
    assert_eq!(
        renamed_folder_name(&name("Projets/2026"), "Année 2026", &existing),
        Ok(name("Projets/Année 2026"))
    );
}

#[test]
fn renamed_folder_under_inbox_prefix() {
    let existing = vec![
        remote("INBOX", Some('.')),
        remote("INBOX.Sent", Some('.')),
        remote("INBOX.Projets", Some('.')),
    ];
    assert_eq!(
        renamed_folder_name(&name("INBOX.Projets"), "Chantiers", &existing),
        Ok(name("INBOX.Chantiers"))
    );
}

#[test]
fn renamed_folder_may_change_only_its_case() {
    let existing = plain_account();
    assert_eq!(
        renamed_folder_name(&name("Projets"), "PROJETS", &existing),
        Ok(name("PROJETS"))
    );
    assert_eq!(
        renamed_folder_name(&name("Projets"), "Projets", &existing),
        Ok(name("Projets"))
    );
}

#[test]
fn renamed_folder_follows_the_name_rules() {
    let existing = plain_account();
    let folder = name("Projets");
    for (label, expected) in [
        ("  ", NewFolderError::Empty),
        ("a*b", NewFolderError::InvalidCharacter),
        ("a/b", NewFolderError::ContainsDelimiter('/')),
        ("sent", NewFolderError::AlreadyExists),
    ] {
        assert_eq!(
            renamed_folder_name(&folder, label, &existing),
            Err(FolderChangeError::Name(expected)),
            "{label:?}"
        );
    }
    assert_eq!(
        renamed_folder_name(&folder, &"x".repeat(MAX_FOLDER_LABEL_CHARS + 1), &existing),
        Err(FolderChangeError::Name(NewFolderError::TooLong))
    );
}

#[test]
fn renamed_folder_must_exist() {
    assert_eq!(
        renamed_folder_name(&name("Absent"), "x", &plain_account()),
        Err(FolderChangeError::UnknownFolder)
    );
}

#[test]
fn special_folders_cannot_be_renamed_or_deleted() {
    let mut existing = vec![
        remote("INBOX", Some('/')),
        special("Envoyés", MailboxRole::Sent),
        special("Brouillons", MailboxRole::Drafts),
        special("Corbeille", MailboxRole::Trash),
        special("Indésirables", MailboxRole::Junk),
        special("Archives", MailboxRole::Archive),
        special("[Gmail]/Tous", MailboxRole::All),
        special("[Gmail]/Suivis", MailboxRole::Flagged),
    ];
    // La boîte de réception est protégée même sans rôle reconnu.
    existing[0].role = MailboxRole::Normal;
    for folder in &existing {
        assert_eq!(
            renamed_folder_name(&folder.name, "x", &existing),
            Err(FolderChangeError::Protected),
            "{}",
            folder.name
        );
        assert_eq!(
            deletable_folder(&folder.name, &existing),
            Err(FolderChangeError::Protected),
            "{}",
            folder.name
        );
    }
}

#[test]
fn folder_without_subfolders_can_be_deleted() {
    let existing = plain_account();
    assert_eq!(deletable_folder(&name("Projets/2026"), &existing), Ok(()));
    let mut hidden = remote("Vieux", Some('/'));
    hidden.selectable = false;
    assert_eq!(deletable_folder(&hidden.name.clone(), &[hidden]), Ok(()));
}

#[test]
fn folder_with_subfolders_cannot_be_deleted() {
    let existing = plain_account();
    assert_eq!(
        deletable_folder(&name("Projets"), &existing),
        Err(FolderChangeError::HasSubfolders)
    );
    // Un nom qui commence pareil n'est pas un sous-dossier.
    let existing = vec![
        remote("Projets", Some('/')),
        remote("Projets 2026", Some('/')),
    ];
    assert_eq!(deletable_folder(&name("Projets"), &existing), Ok(()));
}

#[test]
fn deleted_folder_must_exist() {
    assert_eq!(
        deletable_folder(&name("Absent"), &plain_account()),
        Err(FolderChangeError::UnknownFolder)
    );
}
