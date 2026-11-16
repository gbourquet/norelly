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
fn folder_label_rejects_names_interpreted_by_servers() {
    // Répertoire personnel, espace de noms, segments relatifs, et accolade
    // finale prise pour l'annonce d'un littéral IMAP (`{3}`).
    for label in [
        "~",
        "~alice",
        " ~Perso",
        "#news",
        "#shared",
        ".",
        "..",
        " .. ",
        "a/..",
        "./a",
        "a/ ..",
        "a/.. ",
        "a/ ~root",
        "a\\..",
        "a\\~b",
        "x\\#y",
        "a/\u{a0}#b",
        "~a/b",
        "a/#b",
        "Notes{3}",
        "Notes{3+}",
        "x}",
        "{}",
    ] {
        assert_eq!(
            folder_label(label),
            Err(NewFolderError::Reserved),
            "{label}"
        );
    }
    for label in [
        "Notes~",
        "C#",
        "a.b",
        "...",
        "a..b",
        ".cache",
        "{3} notes",
        "Projet {x",
        "a~b",
    ] {
        assert_eq!(folder_label(label), Ok(label), "{label}");
    }
}

#[test]
fn new_and_renamed_folders_reject_reserved_names_without_delimiter() {
    let flat = vec![remote("INBOX", None), remote("Bla", None)];
    for label in ["~alice", "#news", "..", "Notes{3}"] {
        assert_eq!(
            new_folder_name(label, None, &flat),
            Err(NewFolderError::Reserved),
            "{label}"
        );
        assert_eq!(
            renamed_folder_name(&name("Bla"), label, &flat),
            Err(FolderChangeError::Name(NewFolderError::Reserved)),
            "{label}"
        );
    }
}

#[test]
fn moved_folder_rejects_a_reserved_kept_name() {
    // Le nom conservé vient du serveur (dossier créé par un autre client).
    for leaf in ["~alice", "#news", "..", ".", " ..", "Notes{3}"] {
        let folder = format!("Projets/{leaf}");
        let mut existing = plain_account();
        existing.push(remote(&folder, Some('/')));
        assert_eq!(
            moved_folder_name(&name(&folder), None, &existing),
            Err(FolderChangeError::ReservedName),
            "{leaf}"
        );
        // Déjà à cet endroit : rien n'est envoyé, rien à refuser.
        assert_eq!(
            moved_folder_name(&name(&folder), Some(&name("Projets")), &existing),
            Ok(name(&folder)),
            "{leaf}"
        );
    }
    // Serveur sans séparateur : le nom entier est conservé, et il est déjà
    // au premier niveau.
    let flat = vec![remote("INBOX", None), remote("Notes{3}", None)];
    assert_eq!(
        moved_folder_name(&name("Notes{3}"), None, &flat),
        Ok(name("Notes{3}"))
    );
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
    assert_eq!(
        deletable_folder(&name("Projets/2026"), &existing),
        Ok(Some('/'))
    );
    let mut hidden = remote("Vieux", Some('/'));
    hidden.selectable = false;
    assert_eq!(
        deletable_folder(&hidden.name.clone(), &[hidden]),
        Ok(Some('/'))
    );
    // Serveur sans hiérarchie : aucun sous-dossier à relire.
    let flat = remote("Vieux", None);
    assert_eq!(deletable_folder(&flat.name.clone(), &[flat]), Ok(None));
}

#[test]
fn folder_with_subfolders_cannot_be_deleted() {
    let existing = plain_account();
    assert_eq!(
        deletable_folder(&name("Projets"), &existing),
        Err(FolderChangeError::HasSubfolders)
    );
    // À la casse près, comme au déplacement ; un homonyme n'en est pas un.
    let existing = vec![
        remote("Projets", Some('/')),
        remote("PROJETS", Some('/')),
        remote("projets/2026", Some('/')),
    ];
    assert_eq!(
        deletable_folder(&name("Projets"), &existing),
        Err(FolderChangeError::HasSubfolders)
    );
    assert_eq!(
        deletable_folder(&name("PROJETS"), &existing[..2]),
        Ok(Some('/'))
    );
    // Un nom qui commence pareil n'est pas un sous-dossier.
    let existing = vec![
        remote("Projets", Some('/')),
        remote("Projets 2026", Some('/')),
    ];
    assert_eq!(deletable_folder(&name("Projets"), &existing), Ok(Some('/')));
}

#[test]
fn subfolder_whatever_the_case() {
    for (child, parent) in [
        ("Projets/a", "Projets"),
        ("PROJETS/a/b", "Projets"),
        ("été/x", "Été"),
    ] {
        assert!(is_subfolder(&name(child), &name(parent), '/'), "{child}");
    }
    for (other, parent) in [
        ("Projets", "Projets"),
        ("50x/b", "50%"),
        ("Projets.a", "Projets"),
    ] {
        assert!(!is_subfolder(&name(other), &name(parent), '/'), "{other}");
    }
}

#[test]
fn folder_with_a_subfolder_listed_by_the_server_cannot_be_deleted() {
    // Sous-dossiers relus juste avant la suppression, noms illisibles ou
    // absents de la liste du compte compris.
    assert_eq!(no_listed_subfolder(0), Ok(()));
    for listed in [1, 2, MAX_MAILBOXES + 1] {
        assert_eq!(
            no_listed_subfolder(listed),
            Err(FolderChangeError::HasSubfolders),
            "{listed}"
        );
    }
}

#[test]
fn deleted_folder_must_exist() {
    assert_eq!(
        deletable_folder(&name("Absent"), &plain_account()),
        Err(FolderChangeError::UnknownFolder)
    );
}

// ---------------------------------------------------------------------------
// Déplacer un dossier
// ---------------------------------------------------------------------------

#[test]
fn moved_folder_goes_under_its_new_parent() {
    let existing = vec![
        remote("INBOX", Some('/')),
        remote("Bla", Some('/')),
        remote("Bli", Some('/')),
    ];
    assert_eq!(
        moved_folder_name(&name("Bli"), Some(&name("Bla")), &existing),
        Ok(name("Bla/Bli"))
    );
    // Sous la boîte de réception, comme un nouveau dossier.
    assert_eq!(
        moved_folder_name(&name("Bli"), Some(&name("INBOX")), &existing),
        Ok(name("INBOX/Bli"))
    );
}

#[test]
fn moved_folder_back_to_top_level() {
    let existing = plain_account();
    assert_eq!(
        moved_folder_name(&name("Projets/2026"), None, &existing),
        Ok(name("2026"))
    );
    // Serveur qui range tout sous la boîte de réception.
    let existing = vec![
        remote("INBOX", Some('.')),
        remote("INBOX.Projets", Some('.')),
        remote("INBOX.Projets.2026", Some('.')),
    ];
    assert_eq!(
        moved_folder_name(&name("INBOX.Projets.2026"), None, &existing),
        Ok(name("INBOX.2026"))
    );
}

#[test]
fn moved_folder_already_there_keeps_its_name() {
    let existing = plain_account();
    assert_eq!(
        moved_folder_name(&name("Projets/2026"), Some(&name("Projets")), &existing),
        Ok(name("Projets/2026"))
    );
    assert_eq!(
        moved_folder_name(&name("Projets"), None, &existing),
        Ok(name("Projets"))
    );
}

#[test]
fn moved_folder_cannot_go_into_itself() {
    let mut existing = plain_account();
    existing.push(remote("Projets/2026/Mars", Some('/')));
    for parent in ["Projets", "Projets/2026", "Projets/2026/Mars"] {
        assert_eq!(
            moved_folder_name(&name("Projets"), Some(&name(parent)), &existing),
            Err(FolderChangeError::IntoItself),
            "{parent}"
        );
    }
    // Un nom qui commence pareil n'est pas un sous-dossier.
    existing.push(remote("Projets 2026", Some('/')));
    assert_eq!(
        moved_folder_name(&name("Projets"), Some(&name("Projets 2026")), &existing),
        Ok(name("Projets 2026/Projets"))
    );
}

#[test]
fn moved_folder_cannot_go_into_itself_whatever_the_case() {
    // Serveur insensible à la casse : « projets/2026 » est sous « Projets ».
    let existing = vec![
        remote("INBOX", Some('/')),
        remote("Projets", Some('/')),
        remote("projets", Some('/')),
        remote("projets/2026", Some('/')),
        remote("INBOX/Projets", Some('/')),
        remote("inbox/projets/Mars", Some('/')),
        remote("Ⱥ", Some('/')),
        remote("ⱥ/x", Some('/')),
    ];
    for (folder, parent) in [
        ("Projets", "projets"),
        ("Projets", "projets/2026"),
        ("INBOX/Projets", "inbox/projets/Mars"),
        // Majuscule et minuscule de longueurs différentes en UTF-8.
        ("Ⱥ", "ⱥ/x"),
    ] {
        assert_eq!(
            moved_folder_name(&name(folder), Some(&name(parent)), &existing),
            Err(FolderChangeError::IntoItself),
            "{folder} -> {parent}"
        );
    }
}

#[test]
fn moved_folder_cannot_go_into_itself_with_the_parent_delimiter() {
    // Séparateur annoncé différemment pour le dossier et pour le parent.
    let existing = vec![
        remote("INBOX", Some('.')),
        remote("Projets", None),
        remote("Projets.2026", Some('.')),
        remote("Projets 2026", Some('.')),
    ];
    assert_eq!(
        moved_folder_name(&name("Projets"), Some(&name("Projets.2026")), &existing),
        Err(FolderChangeError::IntoItself)
    );
    assert_eq!(
        moved_folder_name(&name("Projets"), Some(&name("Projets 2026")), &existing),
        Ok(name("Projets 2026.Projets"))
    );
}

#[test]
fn folder_holding_a_special_folder_cannot_be_renamed_or_moved() {
    let existing = vec![
        remote("INBOX", Some('/')),
        remote("Perso", Some('/')),
        remote("Perso/Notes", Some('/')),
        special("Perso/Envoyés", MailboxRole::Sent),
        RemoteMailbox {
            selectable: false,
            ..remote("[Gmail]", Some('/'))
        },
        special("[Gmail]/Corbeille", MailboxRole::Trash),
        remote("Classeur", Some('/')),
        special("classeur/Profond/Archives", MailboxRole::Archive),
        remote("Bla", Some('/')),
    ];
    for folder in ["Perso", "[Gmail]", "Classeur"] {
        assert_eq!(
            renamed_folder_name(&name(folder), "Autre", &existing),
            Err(FolderChangeError::ContainsSpecial),
            "{folder}"
        );
        assert_eq!(
            moved_folder_name(&name(folder), Some(&name("Bla")), &existing),
            Err(FolderChangeError::ContainsSpecial),
            "{folder}"
        );
    }
    // Un dossier spécial de même nom à la casse près n'est pas un
    // sous-dossier (serveur sensible à la casse).
    let siblings = vec![
        remote("INBOX", Some('/')),
        remote("Notes", Some('/')),
        special("NOTES", MailboxRole::Archive),
    ];
    assert_eq!(
        renamed_folder_name(&name("Notes"), "Idées", &siblings),
        Ok(name("Idées"))
    );
    // Le sous-dossier ordinaire et le dossier voisin restent libres.
    assert_eq!(
        renamed_folder_name(&name("Perso/Notes"), "Idées", &existing),
        Ok(name("Perso/Idées"))
    );
    assert_eq!(
        moved_folder_name(&name("Bla"), Some(&name("Perso")), &existing),
        Ok(name("Perso/Bla"))
    );
}

#[test]
fn moved_folder_rejects_a_name_taken_at_destination() {
    let mut existing = plain_account();
    existing.push(remote("Archives perso", Some('/')));
    existing.push(remote("Archives perso/projets", Some('/')));
    assert_eq!(
        moved_folder_name(&name("Projets"), Some(&name("Archives perso")), &existing),
        Err(FolderChangeError::Name(NewFolderError::AlreadyExists))
    );
}

#[test]
fn moved_folder_destination_rules() {
    let mut existing = plain_account();
    existing.push(special("[Gmail]/Tous", MailboxRole::All));
    existing.push(special("[Gmail]/Suivis", MailboxRole::Flagged));
    assert_eq!(
        moved_folder_name(&name("Projets"), Some(&name("Absent")), &existing),
        Err(FolderChangeError::Name(NewFolderError::UnknownParent))
    );
    for parent in ["[Gmail]/Tous", "[Gmail]/Suivis"] {
        assert_eq!(
            moved_folder_name(&name("Projets"), Some(&name(parent)), &existing),
            Err(FolderChangeError::Name(NewFolderError::VirtualParent)),
            "{parent}"
        );
    }
    // Serveur sans hiérarchie.
    let flat = vec![
        remote("INBOX", None),
        remote("Bla", None),
        remote("Bli", None),
    ];
    assert_eq!(
        moved_folder_name(&name("Bli"), Some(&name("Bla")), &flat),
        Err(FolderChangeError::Name(NewFolderError::NoHierarchy))
    );
}

#[test]
fn special_and_unknown_folders_cannot_be_moved() {
    let existing = vec![
        remote("INBOX", Some('/')),
        special("Envoyés", MailboxRole::Sent),
        special("Corbeille", MailboxRole::Trash),
        remote("Bla", Some('/')),
    ];
    for folder in ["INBOX", "Envoyés", "Corbeille"] {
        assert_eq!(
            moved_folder_name(&name(folder), Some(&name("Bla")), &existing),
            Err(FolderChangeError::Protected),
            "{folder}"
        );
    }
    assert_eq!(
        moved_folder_name(&name("Absent"), None, &existing),
        Err(FolderChangeError::UnknownFolder)
    );
}

fn node(id: i64, raw: &str, role: MailboxRole) -> FolderNode {
    FolderNode {
        id: MailboxId::new(id).unwrap_or_else(|| panic!("identifiant")),
        name: name(raw),
        delimiter: Some('/'),
        role,
        selectable: true,
        total: 0,
        unread: 0,
    }
}

fn display_order(folders: Vec<FolderNode>) -> Vec<String> {
    folder_display_order(folders)
        .into_iter()
        .map(|f| f.name.to_string())
        .collect()
}

#[test]
fn subfolders_follow_their_parent() {
    use MailboxRole::{Drafts, Inbox, Normal, Trash};
    let order = display_order(vec![
        node(1, "Projets/2026", Normal),
        node(2, "Trash/Vieux", Normal),
        node(3, "Projets", Normal),
        node(4, "INBOX/Factures", Normal),
        node(5, "Trash", Trash),
        node(6, "Archives perso", Normal),
        node(7, "INBOX", Inbox),
        node(8, "Drafts", Drafts),
        node(9, "Projets/2025", Normal),
        node(10, "INBOX/Factures/EDF", Normal),
    ]);
    assert_eq!(
        order,
        [
            "INBOX",
            "INBOX/Factures",
            "INBOX/Factures/EDF",
            "Drafts",
            "Trash",
            "Trash/Vieux",
            "Archives perso",
            "Projets",
            "Projets/2025",
            "Projets/2026",
        ]
    );
}

#[test]
fn special_folders_stay_first_even_when_nested() {
    use MailboxRole::{Inbox, Normal, Sent, Trash};
    // Gmail range ses dossiers spéciaux sous `[Gmail]` ; certains serveurs
    // les rangent sous la boîte de réception (`INBOX/Sent`).
    let order = display_order(vec![
        node(1, "[Gmail]", Normal),
        node(2, "[Gmail]/Important", Normal),
        node(3, "[Gmail]/Corbeille", Trash),
        node(4, "INBOX/Sent", Sent),
        node(5, "INBOX/Sent/2025", Normal),
        node(6, "INBOX", Inbox),
        node(7, "Achats", Normal),
    ]);
    assert_eq!(
        order,
        [
            "INBOX",
            "INBOX/Sent",
            "INBOX/Sent/2025",
            "[Gmail]/Corbeille",
            "Achats",
            "[Gmail]",
            "[Gmail]/Important",
        ]
    );
}

#[test]
fn subfolder_without_listed_parent_follows_its_nearest_ancestor() {
    use MailboxRole::Normal;
    let mut lone = node(4, "Perso.Voyages", Normal);
    lone.delimiter = None;
    let order = display_order(vec![
        node(1, "Projets/A/B", Normal),
        node(2, "Zeta/x", Normal),
        node(3, "Projets", Normal),
        lone,
        node(5, "Projets/C", Normal),
    ]);
    assert_eq!(
        order,
        [
            "Perso.Voyages",
            "Projets",
            "Projets/A/B",
            "Projets/C",
            "Zeta/x"
        ]
    );
}

#[test]
fn inbox_children_are_found_whatever_the_case_of_inbox() {
    use MailboxRole::{Inbox, Normal};
    let order = display_order(vec![
        node(1, "Achats", Normal),
        node(2, "inbox/Factures", Normal),
        node(3, "INBOX", Inbox),
    ]);
    assert_eq!(order, ["INBOX", "inbox/Factures", "Achats"]);
}

/// Pire cas annoncé par un serveur hostile : `MAX_MAILBOXES` noms de
/// `MAX_MAILBOX_NAME_LEN` octets presque faits de séparateurs, sans aucun
/// ancêtre listé. L'ordre doit rester rapide (relu à chaque changement) ;
/// seuil large pour la couverture instrumentée, l'ordre quadratique
/// prenait plus de 30 s.
#[test]
fn display_order_stays_fast_with_deep_hostile_names() {
    let folders: Vec<FolderNode> = (1..=i64::try_from(MAX_MAILBOXES).unwrap_or(i64::MAX))
        .map(|i| {
            let suffix = format!("{i}");
            let raw = format!(
                "{}{suffix}",
                "/".repeat(MAX_MAILBOX_NAME_LEN - suffix.len())
            );
            node(i, &raw, MailboxRole::Normal)
        })
        .collect();
    // Chemin complet de la liste des dossiers : ordre (port de requête),
    // puis placement.
    let started = std::time::Instant::now();
    let tree = place_in_display_tree(folder_display_order(folders));
    let elapsed = started.elapsed();
    assert_eq!(tree.len(), MAX_MAILBOXES);
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "ordre trop lent : {elapsed:?}"
    );
}

/// Entrées hostiles : la sortie est une permutation de l'entrée.
#[test]
fn display_order_keeps_every_folder_once() {
    use MailboxRole::{Inbox, Normal, Sent, Trash};
    let mut euro = node(9, "Perso€Banque", Normal);
    euro.delimiter = Some('€');
    let mut euro_parent = node(10, "Perso", Normal);
    euro_parent.delimiter = Some('€');
    let folders = vec![
        node(1, "/", Normal),
        node(2, "//", Normal),
        node(3, "a//b", Normal),
        node(4, "inbox/x", Normal),
        node(5, "INBOX", Normal),
        node(6, "INBOX/Sent", Sent),
        node(7, "Trash/Trash", Trash),
        node(8, "a", Inbox),
        euro,
        euro_parent,
    ];
    let mut ids: Vec<i64> = folder_display_order(folders.clone())
        .iter()
        .map(|f| f.id.get())
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, (1..=10).collect::<Vec<_>>());
    let names = display_order(folders);
    let after = |child: &str, parent: &str| {
        let p = names.iter().position(|n| n == parent);
        let c = names.iter().position(|n| n == child);
        assert_eq!(
            c,
            p.map(|p| p + 1),
            "{child} juste après {parent} : {names:?}"
        );
    };
    after("Perso€Banque", "Perso");
    after("inbox/x", "INBOX");
}

fn display_tree(folders: Vec<FolderNode>) -> Vec<(String, usize, String)> {
    place_in_display_tree(folder_display_order(folders))
        .into_iter()
        .map(|d| (d.folder.name.to_string(), d.indent, d.label))
        .collect()
}

fn row(name: &str, depth: usize, label: &str) -> (String, usize, String) {
    (name.to_owned(), depth, label.to_owned())
}

/// Le placement ne trie pas : il garde l'ordre reçu, même faux, sans
/// perdre ni dupliquer de dossier.
#[test]
fn placing_keeps_every_folder_in_the_given_order() {
    use MailboxRole::{Inbox, Normal, Trash};
    let folders = vec![
        node(1, "Projets/2026", Normal),
        node(2, "INBOX", Inbox),
        node(3, "Projets", Normal),
        node(4, "Trash", Trash),
        node(5, "/", Normal),
        node(6, "//", Normal),
    ];
    let ids: Vec<i64> = place_in_display_tree(folders)
        .iter()
        .map(|d| d.folder.id.get())
        .collect();
    assert_eq!(ids, [1, 2, 3, 4, 5, 6]);
}

#[test]
fn subfolders_are_indented_under_their_parent() {
    use MailboxRole::{Inbox, Normal};
    assert_eq!(
        display_tree(vec![
            node(1, "INBOX", Inbox),
            node(2, "INBOX/Factures", Normal),
            node(3, "INBOX/Factures/EDF", Normal),
            node(4, "Projets", Normal),
        ]),
        [
            row("INBOX", 0, "INBOX"),
            row("INBOX/Factures", 1, "Factures"),
            row("INBOX/Factures/EDF", 2, "EDF"),
            row("Projets", 0, "Projets"),
        ]
    );
}

#[test]
fn nested_special_folders_are_shown_at_the_first_level() {
    use MailboxRole::{Inbox, Normal, Sent, Trash};
    assert_eq!(
        display_tree(vec![
            node(1, "INBOX", Inbox),
            node(2, "INBOX/Sent", Sent),
            node(3, "INBOX/Sent/2025", Normal),
            node(4, "[Gmail]", Normal),
            node(5, "[Gmail]/Corbeille", Trash),
            node(6, "[Gmail]/Important", Normal),
        ]),
        [
            row("INBOX", 0, "INBOX"),
            row("INBOX/Sent", 0, "Sent"),
            row("INBOX/Sent/2025", 1, "2025"),
            row("[Gmail]/Corbeille", 0, "Corbeille"),
            row("[Gmail]", 0, "[Gmail]"),
            row("[Gmail]/Important", 1, "Important"),
        ]
    );
}

#[test]
fn orphan_folders_show_the_missing_path() {
    use MailboxRole::{Inbox, Normal};
    let mut dotted = node(5, "Perso.Voyages.Italie", Normal);
    dotted.delimiter = Some('.');
    let mut dotted_parent = node(6, "Perso", Normal);
    dotted_parent.delimiter = Some('.');
    assert_eq!(
        display_tree(vec![
            node(1, "INBOX", Inbox),
            node(2, "INBOX/Perso/Banque", Normal),
            node(3, "INBOX/Perso/Banque/2025", Normal),
            node(4, "Zeta/x/y", Normal),
            dotted,
            dotted_parent,
        ]),
        [
            row("INBOX", 0, "INBOX"),
            row("INBOX/Perso/Banque", 1, "Perso/Banque"),
            row("INBOX/Perso/Banque/2025", 2, "2025"),
            row("Perso", 0, "Perso"),
            row("Perso.Voyages.Italie", 1, "Voyages.Italie"),
            row("Zeta/x/y", 0, "Zeta/x/y"),
        ]
    );
}

#[test]
fn orphan_label_ignores_the_case_of_inbox() {
    use MailboxRole::{Inbox, Normal};
    let mut euro = node(3, "Perso€A€B", Normal);
    euro.delimiter = Some('€');
    let mut euro_parent = node(4, "Perso", Normal);
    euro_parent.delimiter = Some('€');
    assert_eq!(
        display_tree(vec![
            node(1, "INBOX", Inbox),
            node(2, "inbox/a/b", Normal),
            euro,
            euro_parent,
        ]),
        [
            row("INBOX", 0, "INBOX"),
            row("inbox/a/b", 1, "a/b"),
            row("Perso", 0, "Perso"),
            row("Perso€A€B", 1, "A€B"),
        ]
    );
}

#[test]
fn received_headers_carry_the_sender_authentication() {
    use crate::endpoint::Host;
    use crate::sender_auth::{AuthServTrust, SenderAuth};
    let host = Host::parse("imap.exemple.fr").unwrap_or_else(|e| panic!("{e}"));
    let trust = AuthServTrust::for_imap_host(&host);
    let raw = |results: Vec<&'static str>| RawHeaders {
        from: vec![("Banque", "contact@banque.fr")],
        from_fields: 1,
        authentication_results: results,
        ..RawHeaders::default()
    };
    let pass = raw(vec!["mx.exemple.fr; dmarc=pass header.from=banque.fr"]);
    assert_eq!(
        MessageHeaders::sanitize_received(&pass, &trust).sender_auth,
        SenderAuth::Pass
    );
    let fail = raw(vec!["mx.exemple.fr; dmarc=fail header.from=banque.fr"]);
    assert_eq!(
        MessageHeaders::sanitize_received(&fail, &trust).sender_auth,
        SenderAuth::Fail
    );
    let foreign = raw(vec!["mx.pirate.net; dmarc=pass header.from=banque.fr"]);
    assert_eq!(
        MessageHeaders::sanitize_received(&foreign, &trust).sender_auth,
        SenderAuth::Unknown
    );
    // Plusieurs champs `From` : celui qui s'affiche n'est peut-être pas
    // celui qui a été vérifié.
    let two_from = RawHeaders {
        from_fields: 2,
        ..raw(vec!["mx.exemple.fr; dmarc=pass header.from=banque.fr"])
    };
    assert_eq!(
        MessageHeaders::sanitize_received(&two_from, &trust).sender_auth,
        SenderAuth::Unknown
    );
    // Sans confiance (rédaction, en-têtes locaux) : aucun verdict.
    assert_eq!(
        MessageHeaders::sanitize(&pass).sender_auth,
        SenderAuth::Unknown
    );
    let received = MessageHeaders::sanitize_received(&pass, &trust);
    assert_eq!(
        MessageHeaders {
            sender_auth: SenderAuth::Unknown,
            ..received
        },
        MessageHeaders::sanitize(&pass)
    );
}

#[test]
fn a_relocated_name_never_becomes_the_inbox() {
    // Séparateur hostile « b » : « Pbox » sous « P » deviendrait « Inbox »,
    // c'est-à-dire INBOX, si « P » était renommé en « In ».
    assert_eq!(
        name("Pbox").relocated(&name("P"), &name("In"), Some('b')),
        None
    );
    assert_eq!(
        name("P").relocated(&name("P"), &name("INBOX"), Some('/')),
        None
    );
    assert_eq!(
        name("P/x").relocated(&name("P"), &name("Q"), Some('/')),
        Some(name("Q/x"))
    );
}

#[test]
fn interrupted_rename_is_confirmed_only_by_the_new_name_without_the_old() {
    let (from, to) = (name("Bli"), name("Blo"));
    let d = Some('/');
    // Appliqué : nouveau nom listé, ancien absent.
    assert!(relocation_confirmed(
        &from,
        &to,
        &[listed("INBOX", d, &[]), listed("Blo", d, &[])]
    ));
    // Dossier qui ne sert qu'à en contenir d'autres : renommé aussi.
    assert!(relocation_confirmed(
        &from,
        &to,
        &[listed("Blo", d, &["\\Noselect"])]
    ));
    // Ancien nom seulement listé comme inexistant : il a bien disparu.
    assert!(relocation_confirmed(
        &from,
        &to,
        &[listed("Bli", d, &["\\NonExistent"]), listed("Blo", d, &[])]
    ));
    // Pas appliqué : ancien nom encore là, avec ou sans le nouveau.
    assert!(!relocation_confirmed(&from, &to, &[listed("Bli", d, &[])]));
    assert!(!relocation_confirmed(
        &from,
        &to,
        &[listed("Bli", d, &[]), listed("Blo", d, &[])]
    ));
    // Nouveau nom absent, ou seulement inexistant.
    assert!(!relocation_confirmed(
        &from,
        &to,
        &[listed("INBOX", d, &[])]
    ));
    assert!(!relocation_confirmed(
        &from,
        &to,
        &[listed("Blo", d, &["\\NonExistent"])]
    ));
}
