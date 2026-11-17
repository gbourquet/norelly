use proptest::prelude::*;

use super::*;
use crate::conversation::ConversationScope;
use crate::mailbox::{MailboxId, MailboxRole};

fn kw(raw: &str) -> Keyword {
    Keyword::parse(raw).unwrap_or_else(|| panic!("mot-clé {raw:?}"))
}

fn name(raw: &str) -> LabelName {
    LabelName::parse(raw).unwrap_or_else(|e| panic!("nom {raw:?} refusé : {e}"))
}

fn color(raw: &str) -> LabelColor {
    LabelColor::parse(raw).unwrap_or_else(|| panic!("couleur {raw:?}"))
}

fn flags(keywords: &[&str]) -> Flags {
    Flags::from_tokens(keywords.iter().copied())
}

// ---------------------------------------------------------------------------
// Noms et encodage
// ---------------------------------------------------------------------------

#[test]
fn ascii_names_are_their_own_keyword() {
    assert_eq!(name("Projet").keyword(), kw("Projet"));
    assert_eq!(name("  client-42  ").as_str(), "client-42");
    assert_eq!(name("client-42").keyword(), kw("client-42"));
}

#[test]
fn non_ascii_and_atom_specials_are_encoded_reversibly() {
    for raw in [
        "À faire",
        "Été 2026",
        "a&b",
        "(urgent)",
        "100 %",
        "dos\\sier",
        "\"cité\"",
        "x]y",
        "*",
        "{z}",
        "日本語",
        "😀 joie",
        "$Junk",
        "$label1",
    ] {
        let k = name(raw).keyword();
        assert!(
            Keyword::parse(k.as_str()).is_some(),
            "{raw:?} → {k:?} n'est pas un atom"
        );
        assert_eq!(decode_label(&k).as_deref(), Some(raw), "{raw:?} → {k:?}");
    }
    assert_eq!(name("À faire").keyword(), kw("&AMAAIA-faire"));
    assert_eq!(name("a&b").keyword(), kw("a&-b"));
}

#[test]
fn user_names_never_produce_dollar_keywords() {
    // Les mots-clés `$…` appartiennent au protocole et à Thunderbird.
    assert!(!name("$Junk").keyword().as_str().starts_with('$'));
    assert!(!name("$label1").keyword().as_str().starts_with('$'));
    assert!(name("prix en $").keyword().as_str().starts_with("prix"));
}

#[test]
fn names_are_validated() {
    assert_eq!(LabelName::parse("   "), Err(LabelError::Empty));
    assert_eq!(LabelName::parse("a\u{7}b"), Err(LabelError::InvalidChar));
    assert_eq!(LabelName::parse("a\nb"), Err(LabelError::InvalidChar));
    assert_eq!(LabelName::parse("a\u{200B}b"), Err(LabelError::InvalidChar));
    assert_eq!(LabelName::parse("a\u{202E}b"), Err(LabelError::InvalidChar));
    assert_eq!(
        LabelName::parse(&"x".repeat(MAX_LABEL_NAME_CHARS + 1)),
        Err(LabelError::TooLong)
    );
    assert!(LabelName::parse(&"x".repeat(MAX_LABEL_NAME_CHARS)).is_ok());
    // Trop long une fois encodé (mot-clé de 64 octets au plus).
    assert_eq!(
        LabelName::parse(&"é".repeat(MAX_LABEL_NAME_CHARS)),
        Err(LabelError::TooLong)
    );
}

#[test]
fn non_canonical_or_invalid_encodings_are_unreadable() {
    // `&` sans fin, base64 invalide, ASCII direct encodé inutilement.
    for raw in ["a&b", "&AMA", "&!!-", "&AGE-"] {
        assert_eq!(decode_label(&kw(raw)), None, "{raw:?}");
    }
}

#[test]
fn display_of_keywords() {
    assert_eq!(display(&kw("$label1")), LabelDisplay::Builtin(1));
    assert_eq!(display(&kw("$Label5")), LabelDisplay::Builtin(5));
    assert_eq!(
        display(&kw("$label6")),
        LabelDisplay::Raw("$label6".to_owned())
    );
    assert_eq!(
        display(&kw("&AMAAIA-faire")),
        LabelDisplay::Named("À faire".to_owned())
    );
    assert_eq!(display(&kw("a&b")), LabelDisplay::Raw("a&b".to_owned()));
}

#[test]
fn protocol_keywords_are_never_labels() {
    for raw in [
        "$Junk",
        "$NotJunk",
        "$Forwarded",
        "$MDNSent",
        "$Phishing",
        "$SubmitPending",
        "NonJunk",
        "Junk",
        "notjunk",
        "$HasAttachment",
    ] {
        assert!(is_protocol_keyword(&kw(raw)), "{raw}");
    }
    for raw in ["$label1", "$label5", "Projet", "&AMAAIA-faire"] {
        assert!(!is_protocol_keyword(&kw(raw)), "{raw}");
    }
}

#[test]
fn visible_labels_skip_protocol_and_hidden_keywords() {
    let hidden = BTreeSet::from([kw("Ancien")]);
    assert_eq!(
        visible_labels(
            &flags(&["\\Seen", "$Junk", "$label2", "Ancien", "Projet"]),
            &hidden
        ),
        [kw("$label2"), kw("Projet")]
    );
}

// ---------------------------------------------------------------------------
// Couleurs
// ---------------------------------------------------------------------------

#[test]
fn colors_are_strict_hex() {
    assert_eq!(color("#A0b1C2").to_hex(), "#a0b1c2");
    for raw in [
        "a0b1c2",
        "#abc",
        "#a0b1c2ff",
        "#g0b1c2",
        "red",
        "#a0b1c2;x",
        " #a0b1c2",
        "",
        "#ａ0b1c2",
    ] {
        assert_eq!(LabelColor::parse(raw), None, "{raw:?}");
    }
}

#[test]
fn text_tone_follows_the_background() {
    assert_eq!(color("#ffffff").text(), TextTone::Dark);
    assert_eq!(color("#ffeb3b").text(), TextTone::Dark);
    assert_eq!(color("#000000").text(), TextTone::Light);
    assert_eq!(color("#1565c0").text(), TextTone::Light);
}

proptest! {
    /// Contraste du texte d'une pastille : au moins 4,5:1 (WCAG AA) pour
    /// toute couleur, quel que soit le thème (la pastille porte son propre
    /// fond).
    #[test]
    fn chosen_text_always_reaches_aa_contrast(r: u8, g: u8, b: u8) {
        let c = LabelColor::new(r, g, b);
        prop_assert!(contrast_ratio(c, c.text().color()) >= 4.5);
    }

    #[test]
    fn any_valid_name_roundtrips(raw in "\\PC{1,30}") {
        if let Ok(n) = LabelName::parse(&raw) {
            let k = n.keyword();
            prop_assert!(Keyword::parse(k.as_str()).is_some());
            let decoded = decode_label(&k);
            prop_assert_eq!(decoded.as_deref(), Some(n.as_str()));
        }
    }

    #[test]
    fn decoding_never_panics(raw in "[!-~]{1,64}") {
        if let Some(k) = Keyword::parse(&raw) {
            let _ = display(&k);
        }
    }
}

// ---------------------------------------------------------------------------
// Mots-clés permanents (PERMANENTFLAGS)
// ---------------------------------------------------------------------------

#[test]
fn permanent_flags_decide_what_can_be_stored() {
    let any = KeywordPermission::from_tokens(["\\Seen", "\\*"]);
    assert_eq!(any, KeywordPermission::Any);
    assert!(any.can_create() && any.can_store(&kw("Projet")));

    let listed = KeywordPermission::from_tokens(["\\Seen", "$label1", "Projet"]);
    assert!(!listed.can_create());
    assert!(listed.can_store(&kw("Projet")));
    assert!(listed.can_store(&kw("$Label1")));
    assert!(!listed.can_store(&kw("Autre")));

    let none = KeywordPermission::from_tokens(["\\Seen", "\\Flagged"]);
    assert!(!none.can_create() && !none.can_store(&kw("Projet")));

    // Réponse absente : tout est permanent (RFC 3501, 7.1).
    assert!(KeywordPermission::Unknown.can_create());
    assert!(KeywordPermission::Unknown.can_store(&kw("Projet")));
}

#[test]
fn permanent_flags_roundtrip_through_storage() {
    for p in [
        KeywordPermission::Unknown,
        KeywordPermission::Any,
        KeywordPermission::from_tokens(["Projet", "$label1"]),
        KeywordPermission::from_tokens(Vec::<&str>::new()),
    ] {
        assert_eq!(
            KeywordPermission::from_storage(p.to_storage().as_deref()),
            p
        );
    }
    // Valeur stockée illisible : prudence, rien de nouveau.
    assert_eq!(
        KeywordPermission::from_storage(Some("((")),
        KeywordPermission::Listed(BTreeSet::new())
    );
}

// ---------------------------------------------------------------------------
// Liste d'un compte
// ---------------------------------------------------------------------------

fn stored(keyword: &str, color: Option<&str>, hidden: bool) -> StoredLabel {
    StoredLabel {
        keyword: kw(keyword),
        color: color.map(|c| LabelColor::parse(c).unwrap_or_else(|| panic!("couleur"))),
        hidden,
    }
}

fn keywords(labels: &[Label]) -> Vec<&str> {
    labels.iter().map(|l| l.keyword.as_str()).collect()
}

#[test]
fn account_labels_merge_stored_and_seen_keywords() {
    let labels = account_labels(
        &[
            stored("Zebre", None, false),
            stored("Projet", Some("#1565c0"), false),
            stored("Ancien", None, true),
        ],
        &BTreeSet::from([
            kw("$label3"),
            kw("Ancien"),
            kw("$Junk"),
            kw("alpha"),
            kw("Projet"),
            kw("$label1"),
        ]),
        &[],
    );
    assert_eq!(
        keywords(&labels),
        ["$label1", "$label3", "alpha", "Projet", "Zebre"]
    );
    let projet = labels.iter().find(|l| l.keyword == kw("Projet"));
    assert_eq!(projet.and_then(|l| l.color), Some(color("#1565c0")));
}

#[test]
fn rewritten_keywords_leave_the_list() {
    let rewrite = LabelRewrite {
        from: kw("Vieux"),
        to: Some(kw("Neuf")),
    };
    let labels = account_labels(
        &[stored("Neuf", None, false)],
        &BTreeSet::from([kw("Vieux"), kw("Neuf")]),
        std::slice::from_ref(&rewrite),
    );
    assert_eq!(keywords(&labels), ["Neuf"]);
    assert_eq!(
        hidden_keywords(&[stored("Masque", None, true)], &[rewrite]),
        BTreeSet::from([kw("Masque"), kw("Vieux")])
    );
}

#[test]
fn creating_a_label() {
    let existing = account_labels(&[stored("Projet", None, false)], &BTreeSet::new(), &[]);
    assert_eq!(plan_create(&name("Client"), &existing), Ok(kw("Client")));
    assert_eq!(
        plan_create(&name("Projet"), &existing),
        Err(LabelError::AlreadyExists)
    );
    // Même nom à la casse et aux accents près : déjà là.
    assert_eq!(
        plan_create(&name("projet"), &existing),
        Err(LabelError::AlreadyExists)
    );
}

#[test]
fn renaming_a_label_plans_a_rewrite() {
    let existing = account_labels(
        &[stored("Projet", None, false), stored("Client", None, false)],
        &BTreeSet::from([kw("$label2")]),
        &[],
    );
    assert_eq!(
        plan_rename(&kw("Projet"), &name("Projets"), &existing),
        Ok(LabelRewrite {
            from: kw("Projet"),
            to: Some(kw("Projets")),
        })
    );
    assert_eq!(
        plan_rename(&kw("$label2"), &name("Bureau"), &existing),
        Ok(LabelRewrite {
            from: kw("$label2"),
            to: Some(kw("Bureau")),
        })
    );
    assert_eq!(
        plan_rename(&kw("Projet"), &name("client"), &existing),
        Err(LabelError::AlreadyExists)
    );
    assert_eq!(
        plan_rename(&kw("Absent"), &name("X"), &existing),
        Err(LabelError::Unknown)
    );
    assert_eq!(
        plan_rename(&kw("Projet"), &name("Projet"), &existing),
        Err(LabelError::Unchanged)
    );
    // Changer seulement la casse est un vrai renommage.
    assert!(plan_rename(&kw("Projet"), &name("PROJET"), &existing).is_ok());
}

#[test]
fn rewrite_of_a_mailbox() {
    let rename = LabelRewrite {
        from: kw("Vieux"),
        to: Some(kw("Neuf")),
    };
    let remove = LabelRewrite {
        from: kw("Vieux"),
        to: None,
    };
    let refuses = KeywordPermission::from_tokens(["Vieux"]);
    assert!(rename.applies_to(&KeywordPermission::Any));
    assert!(rename.applies_to(&KeywordPermission::Unknown));
    assert!(!rename.applies_to(&refuses));
    assert!(remove.applies_to(&refuses));
}

#[test]
fn rewrite_batches_are_bounded() {
    let uids: BTreeSet<Uid> = (1..=1201).filter_map(Uid::new).collect();
    let batches = rewrite_batches(&uids);
    assert_eq!(
        batches.iter().map(Vec::len).collect::<Vec<_>>(),
        [REWRITE_BATCH, REWRITE_BATCH, 1201 - 2 * REWRITE_BATCH]
    );
    assert!(rewrite_batches(&BTreeSet::new()).is_empty());
}

#[test]
fn local_rewrite_of_flags() {
    let mut f = flags(&["\\Seen", "Vieux", "Autre"]);
    rewrite_flags(&mut f, &kw("Vieux"), Some(&kw("Neuf")));
    assert_eq!(f, flags(&["\\Seen", "Neuf", "Autre"]));
    rewrite_flags(&mut f, &kw("Neuf"), None);
    assert_eq!(f, flags(&["\\Seen", "Autre"]));
    // Sans l'ancien mot-clé : rien ne change.
    rewrite_flags(&mut f, &kw("Absent"), Some(&kw("X")));
    assert_eq!(f, flags(&["\\Seen", "Autre"]));
}

// ---------------------------------------------------------------------------
// Menu « Étiqueter… »
// ---------------------------------------------------------------------------

fn member(id: i64, mailbox: i64, keywords: &[&str]) -> Member {
    Member {
        id: crate::ports::MessageId::new(id).unwrap_or_else(|| panic!("id")),
        mailbox: MailboxId::new(mailbox).unwrap_or_else(|| panic!("id")),
        role: if mailbox == 1 {
            MailboxRole::Inbox
        } else {
            MailboxRole::Sent
        },
        flags: flags(keywords),
        date: id,
        message_id: None,
    }
}

fn inbox() -> ConversationScope {
    ConversationScope::Mailbox(MailboxId::new(1).unwrap_or_else(|| panic!("id")))
}

#[test]
fn a_label_is_checked_when_every_targeted_message_has_it() {
    let labels = [kw("Projet"), kw("Client")];
    let permissions = |_: MailboxId| KeywordPermission::Any;
    let menu = label_menu(
        &[
            vec![
                member(1, 1, &["Projet", "Client"]),
                member(2, 1, &["Projet"]),
                // Hors portée (« Envoyés ») : ne compte pas.
                member(3, 2, &[]),
            ],
            vec![member(4, 1, &["Projet"])],
        ],
        &inbox(),
        &labels,
        permissions,
    );
    assert!(menu.can_create);
    assert_eq!(
        menu.choices,
        [
            LabelChoice {
                keyword: kw("Projet"),
                checked: true,
                allowed: true,
            },
            LabelChoice {
                keyword: kw("Client"),
                checked: false,
                allowed: true,
            },
        ]
    );
}

#[test]
fn label_menu_follows_permanent_flags_of_the_targeted_folders() {
    let labels = [kw("Projet"), kw("Client")];
    let permissions = |_: MailboxId| KeywordPermission::from_tokens(["Projet"]);
    let menu = label_menu(&[vec![member(1, 1, &[])]], &inbox(), &labels, permissions);
    assert!(!menu.can_create);
    assert_eq!(
        menu.choices
            .iter()
            .map(|c| (c.keyword.as_str(), c.allowed))
            .collect::<Vec<_>>(),
        [("Projet", true), ("Client", false)]
    );
    assert!(menu.is_available());
    let refused = label_menu(&[vec![member(1, 1, &[])]], &inbox(), &labels, |_| {
        KeywordPermission::from_tokens(Vec::<&str>::new())
    });
    assert!(!refused.is_available());
    // Une étiquette déjà posée reste retirable.
    let removable = label_menu(
        &[vec![member(1, 1, &["Client"])]],
        &inbox(),
        &labels,
        |_| KeywordPermission::from_tokens(Vec::<&str>::new()),
    );
    assert!(removable.is_available());
    assert!(removable.choices.iter().any(|c| c.checked && c.allowed));
}

#[test]
fn empty_selection_checks_nothing() {
    let menu = label_menu(&[], &inbox(), &[kw("Projet")], |_| KeywordPermission::Any);
    assert!(menu.choices.iter().all(|c| !c.checked));
}

// ---------------------------------------------------------------------------
// Recherche
// ---------------------------------------------------------------------------

#[test]
fn label_search_matches_names_case_and_accent_insensitively() {
    let known = [kw("Projet"), kw("&AMAAIA-faire"), kw("$label2"), kw("a&b")];
    assert_eq!(
        label_keywords("projet", &known),
        [kw("projet"), kw("Projet")]
    );
    assert_eq!(
        label_keywords("a faire", &known),
        [kw("a&ACA-faire"), kw("$label4"), kw("&AMAAIA-faire")]
    );
    // Noms de Thunderbird, en français comme en anglais.
    assert_eq!(
        label_keywords("travail", &known),
        [kw("travail"), kw("$label2")]
    );
    assert_eq!(label_keywords("Work", &known), [kw("Work"), kw("$label2")]);
    // Mot-clé illisible : cherché tel quel.
    assert_eq!(label_keywords("a&b", &known), [kw("a&-b"), kw("a&b")]);
    assert!(label_keywords("   ", &known).is_empty());
}
