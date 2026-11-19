use super::*;
use crate::contacts::ContactId;

/// Normalisation de test : compose les seuls « e » et « E » suivis d'un
/// accent aigu combinant (U+0301), et ne décompose qu'eux.
struct FakeNfc;

impl TextNormalizer for FakeNfc {
    fn nfc(&self, text: &str) -> String {
        text.replace("e\u{301}", "é").replace("E\u{301}", "É")
    }

    fn nfd(&self, text: &str) -> String {
        text.replace('é', "e\u{301}").replace('É', "E\u{301}")
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
    plan_import(existing, &[], cards, policy, &FakeNfc)
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
            ImportedCard::List(ImportedList::default()),
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
    assert_eq!(
        reasons[1],
        (3, "", RejectReason::InvalidList(ContactListError::NoName))
    );
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
    // La fiche lue accompagne le complément, pour que l'import refuse
    // d'écraser une modification faite depuis.
    assert_eq!(c.read(), &existing[0]);
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
fn a_favorite_card_of_the_file_makes_its_new_contact_a_favorite() {
    let favorite_card = |name: &str, addresses: &[&str]| {
        let mut input = draft(name, addresses).to_input();
        input.favorite = true;
        ImportedCard::Contact(input)
    };
    // Deux fiches du fichier pour un même nouveau contact : il est favori
    // si l'une d'elles l'est, quel que soit leur ordre.
    for cards in [
        [
            card("Anne", &["anne@example.com"]),
            favorite_card("Anne", &["anne@example.com", "anne@example.org"]),
        ],
        [
            card("Anne", &["anne@example.com"]),
            favorite_card("Anne", &["anne@example.com"]),
        ],
        [
            favorite_card("Anne", &["anne@example.com"]),
            card("Anne", &["anne@example.com", "anne@example.org"]),
        ],
    ] {
        let p = plan(&[], &cards, DuplicatePolicy::Complete);
        assert_eq!(p.create.len(), 1);
        assert!(p.create[0].favorite(), "{cards:?}");
        assert_eq!(p.duplicates, 1);
    }
}

#[test]
fn a_favorite_card_never_changes_the_favorite_of_a_contact_of_the_book() {
    // Le carnet décide du favori de ses contacts : un fichier ne le
    // change pas, même en complétant.
    let existing = [stored(7, draft("Alice", &["alice@example.com"]))];
    let mut input = draft("Alice", &["alice@example.com", "alice@example.org"]).to_input();
    input.favorite = true;
    let p = plan(
        &existing,
        &[ImportedCard::Contact(input)],
        DuplicatePolicy::Complete,
    );
    assert_eq!(p.complete.len(), 1);
    assert!(!p.complete[0].draft().favorite());
}

#[test]
fn import_keys_lists_each_valid_address_once_in_file_order() {
    let keys = import_keys(
        &[
            card("A", &["b@example.com", "pas-une-adresse", "A@example.com"]),
            ImportedCard::List(ImportedList {
                name: "L".into(),
                members: vec![
                    ImportedMember::Address(" d@example.org ".into()),
                    ImportedMember::Card(0),
                    ImportedMember::Address("c@example.com".into()),
                ],
                ..ImportedList::default()
            }),
            card("B", &["a@EXAMPLE.com", "c@example.com"]),
        ],
        &FakeNfc,
    );
    let keys: Vec<_> = keys.iter().map(AddressKey::as_str).collect();
    assert_eq!(
        keys,
        [
            "b@example.com",
            "a@example.com",
            "d@example.org",
            "c@example.com"
        ]
    );
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

#[test]
fn only_a_book_changed_since_the_read_outdates_the_plan() {
    use crate::ports::{ContactStoreError, StoreError};
    let taken = crate::address::EmailAddress::parse("a@example.com")
        .unwrap_or_else(|e| panic!("adresse : {e}"));
    assert!(plan_outdated(&ContactStoreError::Conflict));
    assert!(plan_outdated(&ContactStoreError::NotFound));
    assert!(plan_outdated(&ContactStoreError::DuplicateAddress(taken)));
    assert!(!plan_outdated(&ContactStoreError::Corrupted("x".into())));
    assert!(!plan_outdated(&ContactStoreError::Store(StoreError(
        "disque plein".into()
    ))));
}

fn details_card(address: &str) -> ContactInput {
    use crate::contacts::{PostalAddressInput, WebsiteInput};
    ContactInput {
        display_name: "Élodie".into(),
        title: "Re\u{301}gisseuse".into(),
        birthday: "--0412".into(),
        emails: vec![email(address)],
        postal_addresses: vec![
            PostalAddressInput {
                street: "1 rue de l'E\u{301}glise".into(),
                locality: "Lyon".into(),
                label: "home".into(),
                ..PostalAddressInput::default()
            },
            // Vide : écartée.
            PostalAddressInput {
                label: "work".into(),
                ..PostalAddressInput::default()
            },
        ],
        websites: vec![
            WebsiteInput {
                url: "www.example.com".into(),
                label: "work".into(),
            },
            // Schéma refusé : écarté, la fiche est gardée.
            WebsiteInput {
                url: "javascript:alert(1)".into(),
                label: String::new(),
            },
        ],
        ..ContactInput::default()
    }
}

#[test]
fn imported_details_are_normalized_and_unusable_ones_dropped() {
    let mut odd = details_card("b@example.com");
    // Date illisible : écartée, la fiche est gardée.
    odd.birthday = "12/04/1985".into();
    let p = plan(
        &[],
        &[
            ImportedCard::Contact(details_card("a@example.com")),
            ImportedCard::Contact(odd),
        ],
        DuplicatePolicy::Skip,
    );
    assert!(p.rejected.is_empty(), "{:?}", p.rejected);
    let [a, b] = &p.create[..] else {
        panic!("{:?}", p.create)
    };
    assert_eq!(a.title(), Some("Régisseuse"));
    assert_eq!(a.birthday().map(|d| d.iso()), Some("--04-12".into()));
    let streets: Vec<_> = a.postal_addresses().iter().map(|p| p.street()).collect();
    assert_eq!(streets, [Some("1 rue de l'Église")]);
    let urls: Vec<_> = a.websites().iter().map(|w| w.url()).collect();
    assert_eq!(urls, ["https://www.example.com"]);
    assert_eq!(b.birthday(), None);
}

#[test]
fn invisible_characters_in_details_reject_an_imported_card() {
    use crate::contacts::ContactField;
    let mut card = details_card("a@example.com");
    card.postal_addresses[0].locality = "Ly\u{2800}on".into();
    let p = plan(&[], &[ImportedCard::Contact(card)], DuplicatePolicy::Skip);
    assert_eq!(
        p.rejected.first().map(|r| &r.reason),
        Some(&RejectReason::Invalid(ContactError::InvalidCharacter(
            ContactField::PostalAddress(0)
        )))
    );
    let mut card = details_card("a@example.com");
    card.title = "\u{FEFF}Chef".into();
    let p = plan(&[], &[ImportedCard::Contact(card)], DuplicatePolicy::Skip);
    assert_eq!(
        p.rejected.first().map(|r| &r.reason),
        Some(&RejectReason::Invalid(ContactError::InvalidCharacter(
            ContactField::Title
        )))
    );
}

const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0];

fn photos(cards: &[(usize, &[u8])]) -> CardPhotos {
    cards
        .iter()
        .map(|(card, bytes)| {
            (
                *card,
                crate::contact_photo::EmbeddedPhoto {
                    declared: None,
                    bytes: bytes.to_vec(),
                },
            )
        })
        .collect()
}

fn photo_id(n: u8) -> BlobId {
    BlobId::parse(&format!("{n:02x}").repeat(32)).unwrap_or_else(|| panic!("id"))
}

fn plan_photos(
    existing: &[Contact],
    cards: &[ImportedCard],
    photos: &CardPhotos,
    policy: DuplicatePolicy,
) -> ImportPlan {
    plan_import_with_photos(existing, &[], cards, photos, policy, &FakeNfc)
}

#[test]
fn new_cards_receive_their_embedded_photo() {
    let cards = [
        card("Alice", &["alice@example.com"]),
        card("Bob", &["bob@example.net"]),
        card("Carole", &["carole@example.org"]),
    ];
    let p = plan_photos(
        &[],
        &cards,
        &photos(&[(0, JPEG), (2, b"GIF89a")]),
        DuplicatePolicy::Skip,
    );
    assert_eq!(
        p.photos,
        [PhotoPlacement {
            card: 0,
            format: PhotoFormat::Jpeg,
            target: PhotoTarget::Created(0),
        }]
    );
}

#[test]
fn photos_of_rejected_or_skipped_cards_are_ignored() {
    let existing = [stored(1, draft("Alice", &["alice@example.com"]))];
    let cards = [
        card("Sans adresse", &[]),
        card("Alice bis", &["alice@example.com"]),
    ];
    let p = plan_photos(
        &existing,
        &cards,
        &photos(&[(0, JPEG), (1, JPEG)]),
        DuplicatePolicy::Skip,
    );
    assert!(p.photos.is_empty(), "{:?}", p.photos);
    assert!(p.complete.is_empty());
}

#[test]
fn a_completed_contact_receives_a_photo_only_if_it_has_none() {
    let existing = [
        stored(1, draft("Alice", &["alice@example.com"])),
        stored(2, draft("Bob", &["bob@example.net"])).with_photo(Some(photo_id(7))),
    ];
    // Fiches qui n'apportent rien d'autre que la photo.
    let cards = [
        card("Alice", &["alice@example.com"]),
        card("Bob", &["bob@example.net"]),
    ];
    let p = plan_photos(
        &existing,
        &cards,
        &photos(&[(0, JPEG), (1, JPEG)]),
        DuplicatePolicy::Complete,
    );
    let id = |n| ContactId::new(n).unwrap_or_else(|| panic!("id"));
    assert_eq!(
        p.photos,
        [PhotoPlacement {
            card: 0,
            format: PhotoFormat::Jpeg,
            target: PhotoTarget::Existing(id(1)),
        }]
    );
    // Alice est complétée (sa photo seule), sans rien changer d'autre ;
    // Bob, qui a déjà une photo, n'est pas touché.
    assert_eq!(p.complete.len(), 1);
    assert_eq!(p.complete[0].id(), id(1));
    assert_eq!(p.complete[0].draft(), existing[0].draft());
    assert_eq!(p.duplicates, 1);
}

#[test]
fn the_first_photo_merged_into_a_new_card_wins() {
    let cards = [
        card("Alice", &["alice@example.com"]),
        card("Alice", &["alice@example.com", "alice@example.org"]),
        card("Alice", &["alice@example.org"]),
    ];
    let p = plan_photos(
        &[],
        &cards,
        &photos(&[(1, JPEG), (2, JPEG)]),
        DuplicatePolicy::Complete,
    );
    assert_eq!(p.create.len(), 1);
    assert_eq!(
        p.photos,
        [PhotoPlacement {
            card: 1,
            format: PhotoFormat::Jpeg,
            target: PhotoTarget::Created(0),
        }]
    );
}

#[test]
fn completing_a_contact_fills_details_without_overwriting() {
    let base = draft("Alice", &["a@example.com"]);
    let mut extra = details_card("a@example.com");
    extra.websites.pop();
    extra.postal_addresses.pop();
    let extra = ContactDraft::new(extra).unwrap_or_else(|e| panic!("{e}"));
    let full = complete_contact(&base, &extra, |_| false).unwrap_or_else(|| panic!("complété"));
    assert_eq!(full.display_name(), "Alice");
    assert_eq!(full.title(), Some("Re\u{301}gisseuse"));
    assert_eq!(full.birthday(), extra.birthday());
    assert_eq!(full.postal_addresses(), extra.postal_addresses());
    assert_eq!(full.websites(), extra.websites());
    // Déjà là (casse près pour l'adresse et le site) : rien n'est ajouté.
    let mut again = details_card("a@example.com");
    again.websites.pop();
    again.postal_addresses.pop();
    again.title = "Autre".into();
    again.birthday = "1990-01-01".into();
    again.postal_addresses[0].street = "1 RUE DE L'E\u{301}GLISE".into();
    again.websites[0].url = "HTTPS://WWW.EXAMPLE.COM".into();
    let again = ContactDraft::new(again).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(complete_contact(&full, &again, |_| false), None);
}

#[test]
fn repeated_or_surplus_details_of_an_imported_card_are_dropped_not_the_card() {
    use crate::contact_details::{MAX_CONTACT_WEBSITES, MAX_POSTAL_ADDRESSES};
    use crate::contacts::{PostalAddressInput, WebsiteInput};
    let place = |street: String| PostalAddressInput {
        street,
        ..PostalAddressInput::default()
    };
    let site = |url: String| WebsiteInput {
        url,
        label: String::new(),
    };
    let mut card = details_card("a@example.com");
    card.postal_addresses = std::iter::once(place("1 RUE A".into()))
        .chain((0..MAX_POSTAL_ADDRESSES + 5).map(|n| {
            place(if n == 0 {
                "1 rue a".into()
            } else {
                format!("{n} rue")
            })
        }))
        .collect();
    card.websites = std::iter::once(site("www.example.com".into()))
        .chain((0..MAX_CONTACT_WEBSITES + 5).map(|n| {
            site(if n == 0 {
                "HTTPS://WWW.EXAMPLE.COM".into()
            } else {
                format!("https://s{n}.example")
            })
        }))
        .collect();
    let p = plan(&[], &[ImportedCard::Contact(card)], DuplicatePolicy::Skip);
    assert!(p.rejected.is_empty(), "{:?}", p.rejected);
    let [d] = &p.create[..] else {
        panic!("{:?}", p.create)
    };
    let streets: Vec<_> = d
        .postal_addresses()
        .iter()
        .filter_map(|a| a.street())
        .collect();
    assert_eq!(streets.len(), MAX_POSTAL_ADDRESSES);
    assert_eq!(streets[..2], ["1 RUE A", "1 rue"]);
    let urls: Vec<_> = d.websites().iter().map(|w| w.url()).collect();
    assert_eq!(urls.len(), MAX_CONTACT_WEBSITES);
    assert_eq!(urls[..2], ["https://www.example.com", "https://s1.example"]);
}

// ---------------------------------------------------------------------------
// Listes de diffusion
// ---------------------------------------------------------------------------

fn address(a: &str) -> ImportedMember {
    ImportedMember::Address(a.to_owned())
}

fn list(name: &str, members: &[&str]) -> ImportedCard {
    ImportedCard::List(ImportedList {
        name: name.into(),
        members: members.iter().map(|&m| address(m)).collect(),
        ..ImportedList::default()
    })
}

fn cid(raw: i64) -> ContactId {
    ContactId::new(raw).unwrap_or_else(|| panic!("id"))
}

fn stored_list(id: i64, name: &str, members: &[i64]) -> ContactList {
    use crate::contact_lists::ContactListId;
    ContactListDraft::new(
        ContactListInput {
            name: name.into(),
            members: members.to_vec(),
            ..ContactListInput::default()
        },
        &FakeNfc,
    )
    .unwrap_or_else(|e| panic!("{e}"))
    .with_id(ContactListId::new(id).unwrap_or_else(|| panic!("id")))
}

fn plan_lists(
    existing: &[Contact],
    lists: &[ContactList],
    cards: &[ImportedCard],
    policy: DuplicatePolicy,
) -> ImportPlan {
    plan_import(existing, lists, cards, policy, &FakeNfc)
}

#[test]
fn a_list_gathers_book_contacts_and_new_cards() {
    let alice = stored(7, draft("Alice", &["alice@example.com"]));
    let p = plan_lists(
        &[alice],
        &[],
        &[
            // Liste avant ses membres : retrouvés quand même.
            ImportedCard::List(ImportedList {
                name: " Club E\u{301}te\u{301} ".into(),
                description: "Le jeudi\n  20 h\n".into(),
                members: vec![
                    ImportedMember::Card(1),
                    address("ALICE@example.com"),
                    address("Bob@Example.net"),
                    address("inconnu@example.org"),
                    address("pas-une-adresse"),
                    // Une liste, ou au-delà du fichier : introuvable.
                    ImportedMember::Card(0),
                    ImportedMember::Card(99),
                ],
                unresolved: 2,
                bcc: true,
            }),
            card("Bob", &["bob@example.net"]),
        ],
        DuplicatePolicy::Skip,
    );
    assert_eq!(names(&p.create), ["Bob"]);
    let [imported] = &p.lists[..] else {
        panic!("{:?}", p.lists)
    };
    assert_eq!(imported.draft().name(), "Club Été");
    // Description de plusieurs lignes ramenée à une ligne.
    assert_eq!(imported.draft().description(), Some("Le jeudi 20 h"));
    assert!(imported.draft().bcc());
    assert_eq!(
        imported.members(),
        [ListMember::New(0), ListMember::Existing(cid(7))]
    );
    // Deux introuvables dans le fichier, quatre ici.
    assert_eq!(p.missing_members, 6);
    let written = imported
        .resolve(&[cid(12)])
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(written.members(), [cid(12), cid(7)]);
    assert_eq!(
        imported.resolve(&[]),
        Err(ContactListError::InvalidMember),
        "fiche créée inconnue"
    );
    assert!(p.rejected.is_empty(), "{:?}", p.rejected);
}

#[test]
fn a_member_card_is_found_whatever_its_first_address() {
    // Première adresse invalide (écartée par la fiche) ou fiche en double :
    // le membre est le contact où la fiche a fini.
    let book = stored(4, draft("Alice", &["alice@example.com"]));
    let cards = [
        card("Bob", &["   ", "bob@example.net"]),
        card("Alice bis", &["alice@example.com"]),
        card("Rejetée", &["pas-une-adresse"]),
        ImportedCard::List(ImportedList {
            name: "Club".into(),
            members: vec![
                ImportedMember::Card(0),
                ImportedMember::Card(1),
                ImportedMember::Card(2),
            ],
            ..ImportedList::default()
        }),
    ];
    for policy in [DuplicatePolicy::Skip, DuplicatePolicy::Complete] {
        let p = plan_lists(std::slice::from_ref(&book), &[], &cards, policy);
        assert_eq!(
            p.lists[0].members(),
            [ListMember::New(0), ListMember::Existing(cid(4))],
            "{policy:?}"
        );
        assert_eq!(p.missing_members, 1, "{policy:?}");
    }
}

#[test]
fn invalid_lists_are_rejected_like_a_typed_list() {
    let many: Vec<String> = (0..=MAX_CONTACT_LIST_MEMBERS)
        .map(|i| format!("m{i}@example.com"))
        .collect();
    let mut cards: Vec<ImportedCard> = many.iter().map(|a| card("", &[a.as_str()])).collect();
    let refs: Vec<&str> = many.iter().map(String::as_str).collect();
    cards.push(list("Trop", &refs));
    cards.push(list("   ", &[]));
    cards.push(list("Fa\u{200B}mille", &[]));
    cards.push(list("patron@entreprise.fr", &[]));
    cards.push(list(&"n".repeat(500), &[]));
    let p = plan(&[], &cards, DuplicatePolicy::Skip);
    assert!(p.lists.is_empty(), "{:?}", p.lists);
    let reasons: Vec<_> = p
        .rejected
        .iter()
        .map(|r| (r.card, r.hint.clone(), r.reason.clone()))
        .collect();
    let first = MAX_CONTACT_LIST_MEMBERS + 2;
    assert_eq!(
        reasons,
        [
            (
                first,
                "Trop".to_owned(),
                RejectReason::InvalidList(ContactListError::TooManyMembers)
            ),
            (
                first + 1,
                String::new(),
                RejectReason::InvalidList(ContactListError::NoName)
            ),
            (
                first + 2,
                "Famille".to_owned(),
                RejectReason::InvalidList(ContactListError::InvalidCharacter(
                    ContactListField::Name
                ))
            ),
            (
                first + 3,
                "patron@entreprise.fr".to_owned(),
                RejectReason::ListNameLooksLikeAddress
            ),
            (
                first + 4,
                "n".repeat(MAX_IMPORT_HINT_CHARS),
                RejectReason::InvalidList(ContactListError::TooLong(ContactListField::Name))
            ),
        ]
    );
}

#[test]
fn a_list_named_like_a_book_list_is_always_left_aside() {
    // Le fichier ne fait jamais entrer quelqu'un dans une liste existante,
    // même en complétant : un nom se devine.
    let alice = stored(1, draft("Alice", &["alice@example.com"]));
    let book_list = stored_list(5, "Famille", &[1]);
    let cards = [
        card("Espion", &["espion@evil.example"]),
        list("FAMILLE", &["espion@evil.example", "alice@example.com"]),
    ];
    for policy in [DuplicatePolicy::Skip, DuplicatePolicy::Complete] {
        let p = plan_lists(
            std::slice::from_ref(&alice),
            std::slice::from_ref(&book_list),
            &cards,
            policy,
        );
        assert!(p.lists.is_empty(), "{policy:?}");
        assert_eq!(p.duplicates, 1, "{policy:?}");
        assert_eq!(p.missing_members, 0, "{policy:?}");
        assert!(p.rejected.is_empty(), "{policy:?}");
    }
}

#[test]
fn a_book_duplicate_is_never_rejected_for_its_size() {
    let many: Vec<String> = (0..=MAX_CONTACT_LIST_MEMBERS)
        .map(|i| format!("m{i}@example.com"))
        .collect();
    let mut cards: Vec<ImportedCard> = many.iter().map(|a| card("", &[a.as_str()])).collect();
    let refs: Vec<&str> = many.iter().map(String::as_str).collect();
    cards.push(list("Famille", &refs));
    let p = plan_lists(
        &[],
        &[stored_list(5, "famille", &[])],
        &cards,
        DuplicatePolicy::Skip,
    );
    assert!(p.rejected.is_empty(), "{:?}", p.rejected);
    assert_eq!(p.duplicates, 1);
}

#[test]
fn lists_of_the_same_name_in_the_file_are_merged_or_skipped() {
    let cards = [
        card("Alice", &["alice@example.com"]),
        card("Bob", &["bob@example.net"]),
        list("Club", &["alice@example.com"]),
        list(
            "CLUB",
            &["bob@example.net", "alice@example.com", "x@inconnu.example"],
        ),
    ];
    let skip = plan(&[], &cards, DuplicatePolicy::Skip);
    assert_eq!(skip.lists.len(), 1);
    assert_eq!(skip.lists[0].members(), [ListMember::New(0)]);
    assert_eq!((skip.duplicates, skip.missing_members), (1, 0));
    let complete = plan(&[], &cards, DuplicatePolicy::Complete);
    assert_eq!(complete.lists.len(), 1);
    assert_eq!(complete.lists[0].draft().name(), "Club");
    assert_eq!(
        complete.lists[0].members(),
        [ListMember::New(0), ListMember::New(1)]
    );
    assert_eq!((complete.duplicates, complete.missing_members), (1, 1));
}

#[test]
fn merging_lists_of_the_file_never_exceeds_the_bound() {
    let many: Vec<String> = (0..=MAX_CONTACT_LIST_MEMBERS)
        .map(|i| format!("m{i}@example.com"))
        .collect();
    let mut cards: Vec<ImportedCard> = many.iter().map(|a| card("", &[a.as_str()])).collect();
    let (head, tail) = many.split_at(MAX_CONTACT_LIST_MEMBERS);
    let head: Vec<&str> = head.iter().map(String::as_str).collect();
    let tail: Vec<&str> = tail.iter().map(String::as_str).collect();
    cards.push(list("Grande", &head));
    cards.push(list("grande", &tail));
    let p = plan(&[], &cards, DuplicatePolicy::Complete);
    assert_eq!(p.lists[0].members().len(), MAX_CONTACT_LIST_MEMBERS);
    assert_eq!(p.rejected.len(), 1);
    assert_eq!(p.rejected[0].reason, RejectReason::ListMergeTooLarge);
    assert_eq!(p.rejected[0].card, cards.len());
}

#[test]
fn import_list_keys_gives_each_valid_name_once() {
    let keys = import_list_keys(
        &[
            list("Club E\u{301}te\u{301}", &[]),
            card("A", &["a@example.com"]),
            list("club été", &[]),
            list("  ", &[]),
            list("Famille", &[]),
        ],
        &FakeNfc,
    );
    assert_eq!(keys, ["club été", "famille"]);
}

#[test]
fn plan_import_alone_places_no_photo() {
    let p = plan(
        &[],
        &[card("Alice", &["alice@example.com"])],
        DuplicatePolicy::Skip,
    );
    assert!(p.photos.is_empty());
}

#[test]
fn photos_beyond_the_import_bound_are_left_out() {
    use crate::contact_photo::MAX_IMPORT_PHOTOS;
    let cards: Vec<ImportedCard> = (0..=MAX_IMPORT_PHOTOS)
        .map(|i| card(&format!("c{i}"), &[&format!("c{i}@example.com")]))
        .collect();
    let all: Vec<(usize, &[u8])> = (0..cards.len()).map(|i| (i, JPEG)).collect();
    let p = plan_photos(&[], &cards, &photos(&all), DuplicatePolicy::Skip);
    assert_eq!(p.create.len(), MAX_IMPORT_PHOTOS + 1);
    assert_eq!(p.photos.len(), MAX_IMPORT_PHOTOS);
    assert!(p.photos.iter().all(|ph| ph.card < MAX_IMPORT_PHOTOS));
}
