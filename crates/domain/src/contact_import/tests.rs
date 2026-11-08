use super::*;
use crate::contacts::ContactId;

/// Normalisation de test : compose les seuls « e » et « E » suivis d'un
/// accent aigu combinant (U+0301).
struct FakeNfc;

impl TextNormalizer for FakeNfc {
    fn nfc(&self, text: &str) -> String {
        text.replace("e\u{301}", "é").replace("E\u{301}", "É")
    }
}

fn email(address: &str) -> EmailInput {
    EmailInput {
        address: address.into(),
        ..EmailInput::default()
    }
}

fn card(name: &str, addresses: &[&str]) -> ImportedCard {
    ImportedCard::Contact(ContactInput {
        display_name: name.into(),
        emails: addresses.iter().map(|a| email(a)).collect(),
        ..ContactInput::default()
    })
}

fn draft(name: &str, addresses: &[&str]) -> ContactDraft {
    let ImportedCard::Contact(input) = card(name, addresses) else {
        unreachable!()
    };
    ContactDraft::new(input).unwrap_or_else(|e| panic!("{e}"))
}

fn stored(id: i64, draft: ContactDraft) -> Contact {
    draft.with_id(ContactId::new(id).unwrap_or_else(|| panic!("id")))
}

fn names(drafts: &[ContactDraft]) -> Vec<String> {
    drafts.iter().map(ContactDraft::display_name).collect()
}

fn plan(existing: &[Contact], cards: &[ImportedCard], policy: DuplicatePolicy) -> ImportPlan {
    plan_import(existing, cards, policy, &FakeNfc)
}

#[test]
fn new_cards_are_created_in_file_order() {
    let p = plan(
        &[],
        &[
            card("Alice", &["alice@example.com"]),
            card("Bob", &["bob@example.net"]),
        ],
        DuplicatePolicy::Skip,
    );
    assert_eq!(names(&p.create), ["Alice", "Bob"]);
    assert!(p.complete.is_empty());
    assert_eq!((p.duplicates, p.renamed), (0, 0));
    assert!(p.rejected.is_empty());
}

#[test]
fn unreadable_lists_and_invalid_cards_are_rejected_with_their_reason() {
    let p = plan(
        &[],
        &[
            ImportedCard::Unreadable,
            card("Alice", &["alice@example.com"]),
            ImportedCard::List,
            card("Sans adresse", &[]),
            card("Mauvaise", &["pas-une-adresse"]),
        ],
        DuplicatePolicy::Skip,
    );
    assert_eq!(names(&p.create), ["Alice"]);
    let reasons: Vec<_> = p
        .rejected
        .iter()
        .map(|r| (r.card, r.hint.as_str(), r.reason.clone()))
        .collect();
    assert_eq!(reasons.len(), 4);
    assert_eq!(reasons[0], (1, "", RejectReason::Unreadable));
    assert_eq!(reasons[1], (3, "", RejectReason::List));
    assert_eq!(
        reasons[2],
        (
            4,
            "Sans adresse",
            RejectReason::Invalid(ContactError::NoAddress)
        )
    );
    assert_eq!(reasons[3].0, 5);
    assert_eq!(reasons[3].1, "Mauvaise");
    assert!(matches!(
        reasons[3].2,
        RejectReason::Invalid(ContactError::InvalidAddress { index: 0, .. })
    ));
}

#[test]
fn rejection_hint_falls_back_to_the_address_and_is_cleaned_and_bounded() {
    let long = format!("{}\u{7}fin", "x".repeat(200));
    let cards = [
        ImportedCard::Contact(ContactInput {
            notes: "\u{0}".into(),
            emails: vec![email("alice@example.com")],
            ..ContactInput::default()
        }),
        ImportedCard::Contact(ContactInput {
            given_name: "Ali".into(),
            family_name: "Baba".into(),
            notes: "\u{0}".into(),
            emails: vec![email("ali@example.com")],
            ..ContactInput::default()
        }),
        card(&long, &["x@example.com"]),
    ];
    let p = plan(&[], &cards, DuplicatePolicy::Skip);
    let hints: Vec<_> = p.rejected.iter().map(|r| r.hint.clone()).collect();
    assert_eq!(hints[0], "alice@example.com");
    assert_eq!(hints[1], "Ali Baba");
    assert_eq!(hints[2].chars().count(), MAX_IMPORT_HINT_CHARS);
    assert!(!hints[2].contains('\u{7}'));
}

#[test]
fn names_are_normalized_to_nfc() {
    let cards = [ImportedCard::Contact(ContactInput {
        display_name: "E\u{301}lodie Ce\u{301}zanne".into(),
        given_name: "E\u{301}lodie".into(),
        organization: "Socie\u{301}te\u{301}".into(),
        notes: "Note\u{301}".into(),
        emails: vec![EmailInput {
            address: "elodie@example.com".into(),
            label: "Pre\u{301}fe\u{301}re\u{301}e".into(),
            preferred: false,
        }],
        ..ContactInput::default()
    })];
    let p = plan(&[], &cards, DuplicatePolicy::Skip);
    let d = &p.create[0];
    assert_eq!(d.display_name_field(), Some("Élodie Cézanne"));
    assert_eq!(d.given_name(), Some("Élodie"));
    assert_eq!(d.organization(), Some("Société"));
    assert_eq!(d.notes(), Some("Noté"));
    assert_eq!(d.emails()[0].label(), Some("Préférée"));
}

#[test]
fn a_name_imitating_an_address_is_dropped_and_the_card_kept() {
    let cards = [
        ImportedCard::Contact(ContactInput {
            display_name: "paypal@paypal.com".into(),
            nickname: "Service <securite@paypal.com>".into(),
            given_name: "Jean".into(),
            emails: vec![email("x@evil.test")],
            ..ContactInput::default()
        }),
        card("bob@example.net", &["bob@example.net"]),
        card("Carole", &["carole@example.org"]),
    ];
    let p = plan(&[], &cards, DuplicatePolicy::Skip);
    assert_eq!(p.renamed, 2);
    assert_eq!(p.create[0].display_name_field(), None);
    assert_eq!(p.create[0].nickname(), None);
    assert_eq!(p.create[0].given_name(), Some("Jean"));
    assert_eq!(p.create[1].display_name(), "bob@example.net");
    assert_eq!(p.create[1].display_name_field(), None);
    assert_eq!(p.create[2].display_name(), "Carole");
}

#[test]
fn the_imitation_check_sees_the_normalized_name() {
    // U+FE6B (petit arobase) n'est pas décomposé par NFC : le nom reste
    // refusé après normalisation.
    let p = plan(
        &[],
        &[card("chef\u{FE6B}corp.com", &["x@evil.test"])],
        DuplicatePolicy::Skip,
    );
    assert_eq!(p.renamed, 1);
    assert_eq!(p.create[0].display_name_field(), None);
}

#[test]
fn blank_and_repeated_addresses_of_a_card_are_ignored() {
    let cards = [ImportedCard::Contact(ContactInput {
        display_name: "Alice".into(),
        emails: vec![
            email("  "),
            EmailInput {
                address: "alice@example.com".into(),
                label: "travail".into(),
                preferred: true,
            },
            EmailInput {
                address: "ALICE@example.com".into(),
                label: String::new(),
                preferred: true,
            },
            EmailInput {
                address: "alice@example.org".into(),
                label: String::new(),
                preferred: true,
            },
        ],
        ..ContactInput::default()
    })];
    let p = plan(&[], &cards, DuplicatePolicy::Skip);
    assert!(p.rejected.is_empty(), "{:?}", p.rejected);
    let d = &p.create[0];
    let addresses: Vec<_> = d
        .emails()
        .iter()
        .map(|e| (e.address().to_string(), e.preferred()))
        .collect();
    assert_eq!(
        addresses,
        [
            ("alice@example.com".to_owned(), true),
            ("alice@example.org".to_owned(), false)
        ]
    );
}

#[test]
fn a_card_whose_address_is_in_the_book_is_skipped() {
    let existing = [stored(7, draft("Alice", &["alice@example.com"]))];
    let p = plan(
        &existing,
        &[
            card("Alice M.", &["Alice@Example.com"]),
            card("Bob", &["bob@example.net"]),
        ],
        DuplicatePolicy::Skip,
    );
    assert_eq!(names(&p.create), ["Bob"]);
    assert!(p.complete.is_empty());
    assert_eq!(p.duplicates, 1);
}

#[test]
fn a_card_repeating_an_earlier_card_of_the_file_is_skipped() {
    let p = plan(
        &[],
        &[
            card("Alice", &["alice@example.com"]),
            card("Alice bis", &["other@example.com", "ALICE@example.com"]),
        ],
        DuplicatePolicy::Skip,
    );
    assert_eq!(names(&p.create), ["Alice"]);
    assert_eq!(p.create[0].emails().len(), 1);
    assert_eq!(p.duplicates, 1);
}

#[test]
fn complete_fills_empty_fields_and_adds_missing_addresses_and_phones() {
    let mut base = draft("Alice", &["alice@example.com"]).to_input();
    base.phones = vec![PhoneInput {
        number: "01 23 45 67 89".into(),
        label: String::new(),
    }];
    base.favorite = true;
    let existing = [stored(
        7,
        ContactDraft::new(base).unwrap_or_else(|e| panic!("{e}")),
    )];
    let cards = [ImportedCard::Contact(ContactInput {
        display_name: "Alice Martin".into(),
        family_name: "Martin".into(),
        organization: "ACME".into(),
        emails: vec![
            EmailInput {
                address: "alice@example.org".into(),
                label: "maison".into(),
                preferred: true,
            },
            email("ALICE@example.com"),
        ],
        phones: vec![
            PhoneInput {
                number: "0123456789".into(),
                label: "bureau".into(),
            },
            PhoneInput {
                number: "06 00 00 00 00".into(),
                label: "mobile".into(),
            },
        ],
        ..ContactInput::default()
    })];
    let p = plan(&existing, &cards, DuplicatePolicy::Complete);
    assert!(p.create.is_empty());
    assert_eq!(p.duplicates, 0);
    assert_eq!(p.complete.len(), 1);
    let c = &p.complete[0];
    assert_eq!(c.id().get(), 7);
    let d = c.draft();
    // Rien n'est écrasé : nom affiché, adresse préférée, favori gardés.
    assert_eq!(d.display_name_field(), Some("Alice"));
    assert_eq!(d.family_name(), Some("Martin"));
    assert_eq!(d.organization(), Some("ACME"));
    assert!(d.favorite());
    let emails: Vec<_> = d
        .emails()
        .iter()
        .map(|e| (e.address().to_string(), e.label(), e.preferred()))
        .collect();
    assert_eq!(
        emails,
        [
            ("alice@example.com".to_owned(), None, true),
            ("alice@example.org".to_owned(), Some("maison"), false)
        ]
    );
    // Même numéro aux espaces près : pas ajouté.
    let phones: Vec<_> = d.phones().iter().map(|p| p.number()).collect();
    assert_eq!(phones, ["01 23 45 67 89", "06 00 00 00 00"]);
}

#[test]
fn complete_counts_a_card_that_brings_nothing_as_a_duplicate() {
    let existing = [stored(7, draft("Alice", &["alice@example.com"]))];
    let p = plan(
        &existing,
        &[card("Autre nom", &["alice@example.com"])],
        DuplicatePolicy::Complete,
    );
    assert!(p.complete.is_empty());
    assert_eq!(p.duplicates, 1);
}

#[test]
fn complete_never_takes_an_address_of_another_contact() {
    let existing = [
        stored(7, draft("Alice", &["alice@example.com"])),
        stored(8, draft("Bob", &["bob@example.net"])),
    ];
    let p = plan(
        &existing,
        &[card(
            "",
            &["alice@example.com", "bob@example.net", "new@example.com"],
        )],
        DuplicatePolicy::Complete,
    );
    assert_eq!(p.complete.len(), 1);
    let d = p.complete[0].draft();
    assert_eq!(p.complete[0].id().get(), 7);
    let keys: Vec<_> = d.emails().iter().map(|e| e.key().as_str()).collect();
    assert_eq!(keys, ["alice@example.com", "new@example.com"]);
}

#[test]
fn complete_merges_cards_of_the_file_and_completes_a_contact_once() {
    let existing = [stored(7, draft("Alice", &["alice@example.com"]))];
    let cards = [
        card("Bob", &["bob@example.net"]),
        card("", &["bob@example.net", "bob@work.example"]),
        card("", &["alice@example.com", "a2@example.com"]),
        card("", &["a2@example.com", "a3@example.com"]),
    ];
    let p = plan(&existing, &cards, DuplicatePolicy::Complete);
    assert_eq!(p.create.len(), 1);
    let bob: Vec<_> = p.create[0]
        .emails()
        .iter()
        .map(|e| e.key().as_str())
        .collect();
    assert_eq!(bob, ["bob@example.net", "bob@work.example"]);
    // Une fiche du fichier versée dans une autre compte comme doublon.
    assert_eq!(p.duplicates, 1);
    assert_eq!(p.complete.len(), 1);
    let alice: Vec<_> = p.complete[0]
        .draft()
        .emails()
        .iter()
        .map(|e| e.key().as_str())
        .collect();
    assert_eq!(
        alice,
        ["alice@example.com", "a2@example.com", "a3@example.com"]
    );
}

#[test]
fn complete_stays_within_the_limits_of_a_card() {
    let many: Vec<String> = (0..MAX_CONTACT_EMAILS)
        .map(|i| format!("a{i}@example.com"))
        .collect();
    let refs: Vec<&str> = many.iter().map(String::as_str).collect();
    let existing = [stored(7, draft("Alice", &refs))];
    let p = plan(
        &existing,
        &[card("", &["a0@example.com", "extra@example.com"])],
        DuplicatePolicy::Complete,
    );
    assert!(p.complete.is_empty());
    assert_eq!(p.duplicates, 1);
    let phones = (0..=MAX_CONTACT_PHONES)
        .map(|i| PhoneInput {
            number: format!("0{i}"),
            label: String::new(),
        })
        .collect();
    let extra = ContactDraft::new(ContactInput {
        phones,
        emails: vec![email("b@example.com")],
        ..ContactInput::default()
    });
    assert_eq!(extra, Err(ContactError::TooManyPhones));
    let base = draft("Bob", &["b@example.com"]);
    let mut input = base.to_input();
    input.phones = (0..MAX_CONTACT_PHONES)
        .map(|i| PhoneInput {
            number: format!("0{i}"),
            label: String::new(),
        })
        .collect();
    let extra = ContactDraft::new(input).unwrap_or_else(|e| panic!("{e}"));
    let full = complete_contact(&base, &extra, |_| false).unwrap_or_else(|| panic!("complété"));
    assert_eq!(full.phones().len(), MAX_CONTACT_PHONES);
    let mut input = extra.to_input();
    input.phones[0].number = "99".into();
    let more = ContactDraft::new(input).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(complete_contact(&full, &more, |_| false), None);
}

#[test]
fn import_keys_lists_each_valid_address_once_in_file_order() {
    let keys = import_keys(&[
        card("A", &["b@example.com", "pas-une-adresse", "A@example.com"]),
        ImportedCard::List,
        card("B", &["a@EXAMPLE.com", "c@example.com"]),
    ]);
    let keys: Vec<_> = keys.iter().map(AddressKey::as_str).collect();
    assert_eq!(keys, ["b@example.com", "a@example.com", "c@example.com"]);
}

#[test]
fn invisible_characters_reject_an_imported_card() {
    use crate::contacts::{ContactField, PhoneInput};
    let base = || ContactInput {
        emails: vec![email("a@example.com")],
        ..ContactInput::default()
    };
    let cases: Vec<(ContactInput, ContactField)> = vec![
        (
            ContactInput {
                display_name: "Ali\u{2028}ce".into(),
                ..base()
            },
            ContactField::DisplayName,
        ),
        (
            ContactInput {
                given_name: "Bob\u{E0041}".into(),
                ..base()
            },
            ContactField::GivenName,
        ),
        (
            ContactInput {
                family_name: "\u{3164}".into(),
                ..base()
            },
            ContactField::FamilyName,
        ),
        (
            ContactInput {
                nickname: "x\u{FE0F}".into(),
                ..base()
            },
            ContactField::Nickname,
        ),
        (
            ContactInput {
                organization: "\u{206A}ACME".into(),
                ..base()
            },
            ContactField::Organization,
        ),
        (
            ContactInput {
                notes: "ligne\n\u{2800}".into(),
                ..base()
            },
            ContactField::Notes,
        ),
        (
            ContactInput {
                emails: vec![EmailInput {
                    address: "a@example.com".into(),
                    label: "travail\u{FFF9}".into(),
                    preferred: false,
                }],
                ..base()
            },
            ContactField::EmailLabel(0),
        ),
        (
            ContactInput {
                phones: vec![PhoneInput {
                    number: "\u{FEFF}06".into(),
                    label: String::new(),
                }],
                ..base()
            },
            ContactField::Phone(0),
        ),
        (
            ContactInput {
                phones: vec![PhoneInput {
                    number: "06".into(),
                    label: "\u{E0100}".into(),
                }],
                ..base()
            },
            ContactField::PhoneLabel(0),
        ),
    ];
    for (input, field) in cases {
        let p = plan(&[], &[ImportedCard::Contact(input)], DuplicatePolicy::Skip);
        assert!(p.create.is_empty(), "{field:?} accepté");
        assert_eq!(
            p.rejected[0].reason,
            RejectReason::Invalid(ContactError::InvalidCharacter(field))
        );
    }
    // Lettres, accents combinants et emoji sans sélecteur restent admis.
    let p = plan(
        &[],
        &[card("Zoe\u{301} 😀 한국", &["z@example.com"])],
        DuplicatePolicy::Skip,
    );
    assert_eq!(p.create.len(), 1, "{:?}", p.rejected);
}

#[test]
fn imported_addresses_are_normalized_and_free_of_invisible_characters() {
    use crate::address::EmailAddressError;
    // Partie locale avec un remplisseur invisible : rejetée.
    let p = plan(
        &[],
        &[card("Patron", &["patron\u{3164}@example.com"])],
        DuplicatePolicy::Skip,
    );
    assert_eq!(
        p.rejected[0].reason,
        RejectReason::Invalid(ContactError::InvalidAddress {
            index: 0,
            error: EmailAddressError::InvalidLocalCharacter
        })
    );
    // Forme décomposée : la même adresse que la forme composée du carnet.
    let existing = [stored(7, draft("Pétra", &["pétra@example.com"]))];
    let p = plan(
        &existing,
        &[card("Pe\u{301}tra", &["pe\u{301}tra@example.com"])],
        DuplicatePolicy::Skip,
    );
    assert_eq!((p.create.len(), p.duplicates), (0, 1));
    // Le repère d'une fiche rejetée n'en garde aucun.
    let p = plan(
        &[],
        &[card("Ali\u{3164}ce\u{2800}", &[])],
        DuplicatePolicy::Skip,
    );
    assert_eq!(p.rejected[0].hint, "Alice");
}
