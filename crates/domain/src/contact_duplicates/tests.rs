use super::*;
use crate::contacts::ContactInput;

/// Normalisation de test : compose les seuls « e » suivis d'un accent aigu
/// combinant (U+0301).
struct FakeNfc;

impl TextNormalizer for FakeNfc {
    fn nfc(&self, text: &str) -> String {
        text.replace("e\u{301}", "é")
    }
}

fn email(address: &str) -> EmailInput {
    EmailInput {
        address: address.into(),
        ..EmailInput::default()
    }
}

fn phone(number: &str) -> PhoneInput {
    PhoneInput {
        number: number.into(),
        ..PhoneInput::default()
    }
}

fn draft(input: ContactInput) -> ContactDraft {
    ContactDraft::new(input).unwrap_or_else(|e| panic!("fiche refusée : {e}"))
}

fn named(name: &str, address: &str) -> ContactInput {
    ContactInput {
        display_name: name.into(),
        emails: vec![email(address)],
        ..ContactInput::default()
    }
}

fn stored(id: i64, input: ContactInput) -> Contact {
    let id = ContactId::new(id).unwrap_or_else(|| panic!("identifiant {id}"));
    draft(input).with_id(id)
}

fn id(raw: i64) -> ContactId {
    ContactId::new(raw).unwrap_or_else(|| panic!("identifiant {raw}"))
}

fn ids(group: &DuplicateGroup) -> Vec<i64> {
    group.contacts.iter().map(|c| c.get()).collect()
}

fn groups(contacts: &[Contact]) -> Vec<(Vec<i64>, Vec<DuplicateReason>)> {
    find_duplicates(contacts, &FakeNfc)
        .iter()
        .map(|g| (ids(g), g.reasons.clone()))
        .collect()
}

// --- Clé de nom --------------------------------------------------------------

#[test]
fn name_key_ignores_case_word_order_and_spacing() {
    let a = draft(named("Alice  Martin", "a@example.com"));
    let b = draft(named("martin ALICE", "b@example.com"));
    assert_eq!(name_key(&a, &FakeNfc).as_deref(), Some("alice martin"));
    assert_eq!(name_key(&a, &FakeNfc), name_key(&b, &FakeNfc));
}

#[test]
fn name_key_compares_composed_and_decomposed_forms() {
    let a = draft(named("Hélène", "a@example.com"));
    let b = draft(named("He\u{301}le\u{300}ne", "b@example.com"));
    let c = draft(named("He\u{301}lène", "c@example.com"));
    assert_eq!(name_key(&a, &FakeNfc), name_key(&c, &FakeNfc));
    // Seul « e » + aigu est composé par le faux : la normalisation est bien
    // celle du port.
    assert_ne!(name_key(&a, &FakeNfc), name_key(&b, &FakeNfc));
}

#[test]
fn name_key_uses_the_composed_name_without_display_name() {
    let a = draft(ContactInput {
        given_name: "Alice".into(),
        family_name: "Martin".into(),
        emails: vec![email("a@example.com")],
        ..ContactInput::default()
    });
    let b = draft(ContactInput {
        nickname: "Lili".into(),
        emails: vec![email("b@example.com")],
        ..ContactInput::default()
    });
    assert_eq!(name_key(&a, &FakeNfc).as_deref(), Some("alice martin"));
    assert_eq!(name_key(&b, &FakeNfc).as_deref(), Some("lili"));
}

#[test]
fn a_card_without_name_or_with_an_address_like_name_has_no_name_key() {
    let nameless = draft(ContactInput {
        organization: "ACME".into(),
        emails: vec![email("a@example.com")],
        ..ContactInput::default()
    });
    let imitation = draft(named("bob@example.com", "b@example.com"));
    assert_eq!(name_key(&nameless, &FakeNfc), None);
    assert_eq!(name_key(&imitation, &FakeNfc), None);
}

// --- Détection ---------------------------------------------------------------

#[test]
fn contacts_of_the_same_name_form_a_group() {
    let book = [
        stored(1, named("Alice Martin", "alice@perso.example")),
        stored(2, named("Bob", "bob@example.com")),
        stored(3, named("martin alice", "alice@travail.example")),
    ];
    assert_eq!(
        groups(&book),
        [(vec![1, 3], vec![DuplicateReason::SameName])]
    );
}

#[test]
fn a_single_word_name_is_enough() {
    let book = [
        stored(1, named("Support", "support@a.example")),
        stored(2, named("support", "support@b.example")),
    ];
    assert_eq!(
        groups(&book),
        [(vec![1, 2], vec![DuplicateReason::SameName])]
    );
}

#[test]
fn nameless_contacts_are_never_grouped_by_name() {
    let nameless = |address: &str| ContactInput {
        emails: vec![email(address)],
        ..ContactInput::default()
    };
    let book = [
        stored(1, nameless("a@example.com")),
        stored(2, nameless("b@example.com")),
        stored(3, named("x@example.com", "c@example.com")),
        stored(4, named("x@example.com", "d@example.com")),
    ];
    assert!(groups(&book).is_empty());
}

#[test]
fn contacts_sharing_a_phone_number_form_a_group() {
    let with_phone = |name: &str, address: &str, number: &str| ContactInput {
        phones: vec![phone(number)],
        ..named(name, address)
    };
    let book = [
        stored(1, with_phone("Alice", "a@example.com", "06 12 34 56 78")),
        stored(2, with_phone("A. Martin", "b@example.com", "0612345678")),
        stored(3, with_phone("Carole", "c@example.com", "07 00 00 00 00")),
    ];
    assert_eq!(
        groups(&book),
        [(vec![1, 2], vec![DuplicateReason::SamePhone])]
    );
}

#[test]
fn a_short_number_does_not_group_contacts() {
    let with_phone = |name: &str, address: &str, number: &str| ContactInput {
        phones: vec![phone(number)],
        ..named(name, address)
    };
    let book = [
        stored(1, with_phone("Alice", "a@example.com", "3615 1")),
        stored(2, with_phone("Bob", "b@example.com", "36151")),
        stored(3, with_phone("Carole", "c@example.com", "standard")),
        stored(4, with_phone("Denis", "d@example.com", "Standard")),
    ];
    assert!(groups(&book).is_empty());
}

#[test]
fn groups_are_transitive_and_list_every_reason() {
    let book = [
        stored(
            1,
            ContactInput {
                phones: vec![phone("+33 6 12 34 56 78")],
                ..named("Alice", "a@example.com")
            },
        ),
        stored(2, named("alice", "b@example.com")),
        stored(
            3,
            ContactInput {
                phones: vec![phone("+33612345678")],
                ..named("A. Martin", "c@example.com")
            },
        ),
        stored(4, named("Bob", "d@example.com")),
    ];
    assert_eq!(
        groups(&book),
        [(
            vec![1, 2, 3],
            vec![DuplicateReason::SameName, DuplicateReason::SamePhone]
        )]
    );
}

#[test]
fn the_most_complete_card_comes_first_then_the_others_by_id() {
    let book = [
        stored(5, named("Alice", "a@example.com")),
        stored(
            9,
            ContactInput {
                organization: "ACME".into(),
                emails: vec![email("b@example.com"), email("c@example.com")],
                ..named("Alice", "b@example.com")
            },
        ),
        stored(2, named("Alice", "d@example.com")),
    ];
    assert_eq!(groups(&book)[0].0, [9, 2, 5]);
}

#[test]
fn completeness_ties_go_to_the_favorite_then_the_oldest_card() {
    let book = [
        stored(4, named("Alice", "a@example.com")),
        stored(
            7,
            ContactInput {
                favorite: true,
                ..named("Alice", "b@example.com")
            },
        ),
        stored(3, named("Alice", "c@example.com")),
    ];
    assert_eq!(groups(&book)[0].0, [7, 3, 4]);
    let book = [
        stored(4, named("Alice", "a@example.com")),
        stored(3, named("Alice", "c@example.com")),
    ];
    assert_eq!(groups(&book)[0].0, [3, 4]);
}

#[test]
fn groups_are_ordered_by_the_name_of_their_first_card() {
    let book = [
        stored(1, named("Zoé", "z1@example.com")),
        stored(2, named("Zoé", "z2@example.com")),
        stored(3, named("alice", "a1@example.com")),
        stored(4, named("Alice", "a2@example.com")),
    ];
    let order: Vec<Vec<i64>> = groups(&book).into_iter().map(|g| g.0).collect();
    assert_eq!(order, [vec![3, 4], vec![1, 2]]);
}

#[test]
fn completeness_counts_filled_fields_addresses_and_phones() {
    let bare = draft(ContactInput {
        emails: vec![email("a@example.com")],
        ..ContactInput::default()
    });
    let full = draft(ContactInput {
        display_name: "Alice".into(),
        given_name: "Alice".into(),
        family_name: "Martin".into(),
        nickname: "Lili".into(),
        organization: "ACME".into(),
        notes: "note".into(),
        emails: vec![email("a@example.com"), email("b@example.com")],
        phones: vec![phone("01"), phone("02"), phone("03")],
        favorite: true,
    });
    assert_eq!(completeness(&bare), 1);
    assert_eq!(completeness(&full), 6 + 2 + 3);
}

// --- Fusion ------------------------------------------------------------------

#[test]
fn merge_fills_empty_fields_without_overwriting() {
    let keep = draft(ContactInput {
        given_name: "Alice".into(),
        ..named("Alice Martin", "a@example.com")
    });
    let other = draft(ContactInput {
        given_name: "Alicia".into(),
        family_name: "Martin".into(),
        nickname: "Lili".into(),
        ..named("A. Martin", "b@example.com")
    });
    let third = draft(ContactInput {
        nickname: "Lisette".into(),
        organization: "ACME".into(),
        ..named("Alice", "c@example.com")
    });
    let merged = merge_contacts(&keep, &[&other, &third]).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(merged.display_name_field(), Some("Alice Martin"));
    assert_eq!(merged.given_name(), Some("Alice"));
    assert_eq!(merged.family_name(), Some("Martin"));
    assert_eq!(merged.nickname(), Some("Lili"));
    assert_eq!(merged.organization(), Some("ACME"));
}

#[test]
fn merge_gathers_addresses_and_keeps_the_preferred_one() {
    let keep = draft(ContactInput {
        emails: vec![
            email("a@example.com"),
            EmailInput {
                preferred: true,
                ..email("b@example.com")
            },
        ],
        ..ContactInput::default()
    });
    let other = draft(ContactInput {
        emails: vec![
            EmailInput {
                label: "travail".into(),
                preferred: true,
                ..email("c@example.com")
            },
            email("B@example.com"),
        ],
        ..ContactInput::default()
    });
    let merged = merge_contacts(&keep, &[&other]).unwrap_or_else(|e| panic!("{e}"));
    let addresses: Vec<_> = merged
        .emails()
        .iter()
        .map(|e| (e.address().to_string(), e.label(), e.preferred()))
        .collect();
    assert_eq!(
        addresses,
        [
            ("a@example.com".to_owned(), None, false),
            ("b@example.com".to_owned(), None, true),
            ("c@example.com".to_owned(), Some("travail"), false),
        ]
    );
}

#[test]
fn merge_gathers_phones_once_per_number() {
    let keep = draft(ContactInput {
        phones: vec![phone("06 12 34 56 78")],
        ..named("Alice", "a@example.com")
    });
    let other = draft(ContactInput {
        phones: vec![
            phone("0612345678"),
            PhoneInput {
                number: "01 02 03 04 05".into(),
                label: "maison".into(),
            },
        ],
        ..named("Alice", "b@example.com")
    });
    let merged = merge_contacts(&keep, &[&other]).unwrap_or_else(|e| panic!("{e}"));
    let phones: Vec<_> = merged
        .phones()
        .iter()
        .map(|p| (p.number(), p.label()))
        .collect();
    assert_eq!(
        phones,
        [("06 12 34 56 78", None), ("01 02 03 04 05", Some("maison"))]
    );
}

#[test]
fn merge_puts_distinct_notes_end_to_end() {
    let with_notes = |address: &str, notes: &str| {
        draft(ContactInput {
            notes: notes.into(),
            ..named("Alice", address)
        })
    };
    let keep = with_notes("a@example.com", "Rencontrée à Lyon");
    let same = with_notes("b@example.com", "Rencontrée à Lyon");
    let none = with_notes("c@example.com", "");
    let other = with_notes("d@example.com", "Aime le thé\nvert");
    let merged = merge_contacts(&keep, &[&same, &none, &other]).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        merged.notes(),
        Some("Rencontrée à Lyon\n\nAime le thé\nvert")
    );
    let merged = merge_contacts(&none, &[&other]).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(merged.notes(), Some("Aime le thé\nvert"));
}

#[test]
fn merge_keeps_the_favorite_if_any_card_is_one() {
    let plain = draft(named("Alice", "a@example.com"));
    let favorite = draft(ContactInput {
        favorite: true,
        ..named("Alice", "b@example.com")
    });
    let merged = merge_contacts(&plain, &[&favorite]).unwrap_or_else(|e| panic!("{e}"));
    assert!(merged.favorite());
    let merged = merge_contacts(&plain, &[&plain.clone()]);
    assert!(!merged.unwrap_or_else(|e| panic!("{e}")).favorite());
}

#[test]
fn a_merge_beyond_the_bounds_of_a_card_is_refused() {
    let many = |prefix: &str, n: usize| {
        draft(ContactInput {
            emails: (0..n)
                .map(|i| email(&format!("{prefix}{i}@example.com")))
                .collect(),
            ..ContactInput::default()
        })
    };
    let keep = many("a", 15);
    let other = many("b", 6);
    assert_eq!(
        merge_contacts(&keep, &[&other]),
        Err(ContactError::TooManyAddresses)
    );

    let phones = |address: &str, start: usize| {
        draft(ContactInput {
            phones: (start..start + 11)
                .map(|i| phone(&format!("01 00 00 00 {i:02}")))
                .collect(),
            ..named("Alice", address)
        })
    };
    assert_eq!(
        merge_contacts(&phones("a@example.com", 0), &[&phones("b@example.com", 50)]),
        Err(ContactError::TooManyPhones)
    );

    let notes = |address: &str, c: &str| {
        draft(ContactInput {
            notes: c.repeat(6_000),
            ..named("Alice", address)
        })
    };
    assert_eq!(
        merge_contacts(
            &notes("a@example.com", "x"),
            &[&notes("b@example.com", "y")]
        ),
        Err(ContactError::TooLong(crate::contacts::ContactField::Notes))
    );
}

// --- Demande de fusion -------------------------------------------------------

#[test]
fn a_merge_selection_keeps_distinct_absorbed_cards_in_order() {
    let selection = MergeSelection::new(1_i64, &[4, 2, 4]).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(selection.keep(), id(1));
    assert_eq!(selection.others(), [id(4), id(2)]);
}

#[test]
fn a_merge_selection_is_refused_when_invalid() {
    assert_eq!(
        MergeSelection::new(0_i64, &[2]),
        Err(MergeSelectionError::InvalidId)
    );
    assert_eq!(
        MergeSelection::new(1_i64, &[-2]),
        Err(MergeSelectionError::InvalidId)
    );
    assert_eq!(
        MergeSelection::new(1_i64, &[]),
        Err(MergeSelectionError::Empty)
    );
    assert_eq!(
        MergeSelection::new(1_i64, &[2, 1]),
        Err(MergeSelectionError::KeepAbsorbed)
    );
    let too_many: Vec<i64> = (2..=i64::try_from(MAX_MERGED_CONTACTS).unwrap_or(0) + 2).collect();
    assert_eq!(
        MergeSelection::new(1_i64, &too_many),
        Err(MergeSelectionError::TooMany)
    );
    let at_bound: Vec<i64> = (2..=i64::try_from(MAX_MERGED_CONTACTS).unwrap_or(0) + 1).collect();
    assert!(MergeSelection::new(1_i64, &at_bound).is_ok());
}
