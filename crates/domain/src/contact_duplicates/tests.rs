use super::*;
use crate::contact_details::MAX_POSTAL_ADDRESSES;
use crate::contacts::ContactInput;

/// Normalisation de test : compose les seuls « e » suivis d'un accent aigu
/// combinant (U+0301) et la syllabe hangûl « 가 » ; ne décompose que « é »,
/// « É », « à » et « 가 ».
struct FakeNfc;

impl TextNormalizer for FakeNfc {
    fn nfc(&self, text: &str) -> String {
        text.replace("e\u{301}", "é")
            .replace("\u{1100}\u{1161}", "\u{AC00}")
    }

    fn nfd(&self, text: &str) -> String {
        text.replace('é', "e\u{301}")
            .replace('É', "E\u{301}")
            .replace('à', "a\u{300}")
            .replace('\u{AC00}', "\u{1100}\u{1161}")
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

/// Fiche telle que relue du carnet (un nom imitant une adresse,
/// enregistré avant que la saisie ne le refuse, y est retiré).
fn stored(id: i64, input: ContactInput) -> Contact {
    let id = ContactId::new(id).unwrap_or_else(|| panic!("identifiant {id}"));
    ContactDraft::from_stored(input)
        .unwrap_or_else(|e| panic!("fiche refusée : {e}"))
        .with_id(id)
}

fn id(raw: i64) -> ContactId {
    ContactId::new(raw).unwrap_or_else(|| panic!("identifiant {raw}"))
}

fn ids(group: &DuplicateGroup) -> Vec<i64> {
    group.contacts.iter().map(|c| c.get()).collect()
}

fn groups(contacts: &[Contact]) -> Vec<(Vec<i64>, Vec<DuplicateReason>)> {
    find_duplicates(contacts, &[], &FakeNfc)
        .iter()
        .map(|g| (ids(g), g.reasons.clone()))
        .collect()
}

fn pair(a: i64, b: i64) -> DismissedPair {
    DismissedPair::new(id(a), id(b)).unwrap_or_else(|| panic!("paire {a}, {b}"))
}

fn groups_without(
    contacts: &[Contact],
    dismissed: &[DismissedPair],
) -> Vec<(Vec<i64>, Vec<DuplicateReason>)> {
    find_duplicates(contacts, dismissed, &FakeNfc)
        .iter()
        .map(|g| (ids(g), g.reasons.clone()))
        .collect()
}

fn with_phone(name: &str, address: &str, number: &str) -> ContactInput {
    ContactInput {
        phones: vec![phone(number)],
        ..named(name, address)
    }
}

fn akey(raw: &str) -> AddressKey {
    AddressKey::parse(raw).unwrap_or_else(|e| panic!("{raw} : {e}"))
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
    // Seul « é » est décomposé par le faux : « è » composé n'est pas
    // ramené à « e », la décomposition est bien celle du port.
    assert_ne!(name_key(&a, &FakeNfc), name_key(&b, &FakeNfc));
}

#[test]
fn name_key_ignores_accents() {
    let a = draft(named("Élodie Cézanne", "a@example.com"));
    let b = draft(named("Elodie Cezanne", "b@example.com"));
    let c = draft(named("e\u{301}lodie CE\u{301}ZANNE", "c@example.com"));
    assert_eq!(name_key(&a, &FakeNfc).as_deref(), Some("cezanne elodie"));
    assert_eq!(name_key(&a, &FakeNfc), name_key(&b, &FakeNfc));
    assert_eq!(name_key(&a, &FakeNfc), name_key(&c, &FakeNfc));
}

#[test]
fn name_key_keeps_marks_that_are_not_accents() {
    // Le virama (U+094D) n'est pas un accent : sans lui, ce n'est plus la
    // même syllabe.
    let a = draft(named("\u{915}\u{94D}\u{937}", "a@example.com"));
    let b = draft(named("\u{915}\u{937}", "b@example.com"));
    assert_ne!(name_key(&a, &FakeNfc), name_key(&b, &FakeNfc));
    // Le résultat est recomposé : un accent hors du bloc des diacritiques
    // latins n'est pas retiré.
    let hebrew = draft(named("\u{5D1}\u{5BC}", "h@example.com"));
    assert_eq!(
        name_key(&hebrew, &FakeNfc).as_deref(),
        Some("\u{5D1}\u{5BC}")
    );
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
    let imitation = ContactDraft::from_stored(named("bob@example.com", "b@example.com"))
        .unwrap_or_else(|e| panic!("fiche refusée : {e}"));
    assert_eq!(name_key(&nameless, &FakeNfc), None);
    assert_eq!(name_key(&imitation, &FakeNfc), None);
}

#[test]
fn name_key_is_recomposed_after_removing_accents() {
    // Une syllabe hangûl décomposée en NFD redevient une syllabe.
    let a = draft(named("\u{AC00}", "a@example.com"));
    assert_eq!(name_key(&a, &FakeNfc).as_deref(), Some("\u{AC00}"));
}

#[test]
fn a_name_imitating_an_address_once_accents_are_removed_has_no_name_key() {
    let imitation = stored(1, named("support (àt) banque.fr", "s@example.com"));
    assert_eq!(name_key(imitation.draft(), &FakeNfc), None);
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
fn contacts_whose_names_differ_only_by_accents_form_a_group() {
    let book = [
        stored(1, named("Célia Dupré", "celia@perso.example")),
        stored(2, named("Celia Dupre", "celia@travail.example")),
    ];
    assert_eq!(
        groups(&book),
        [(vec![1, 2], vec![DuplicateReason::SameName])]
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
        ..ContactInput::default()
    });
    assert_eq!(completeness(&bare), 1);
    assert_eq!(completeness(&full), 6 + 2 + 3);
    let detailed = draft(ContactInput {
        title: "Directrice".into(),
        birthday: "--04-12".into(),
        postal_addresses: vec![postal("1 rue des Lilas"), postal("2 rue des Lilas")],
        websites: vec![site("https://example.com")],
        ..named("", "a@example.com")
    });
    assert_eq!(completeness(&detailed), 1 + 2 + 2 + 1);
}

fn postal(street: &str) -> crate::contacts::PostalAddressInput {
    crate::contacts::PostalAddressInput {
        street: street.into(),
        ..Default::default()
    }
}

fn site(url: &str) -> crate::contacts::WebsiteInput {
    crate::contacts::WebsiteInput {
        url: url.into(),
        label: String::new(),
    }
}

#[test]
fn merge_gathers_details_once_without_overwriting() {
    let keep = draft(ContactInput {
        birthday: "1985-04-12".into(),
        postal_addresses: vec![postal("1 rue des Lilas")],
        websites: vec![site("https://example.com")],
        ..named("Alice", "a@example.com")
    });
    let other = draft(ContactInput {
        title: "Directrice".into(),
        birthday: "--01-01".into(),
        postal_addresses: vec![postal("1 RUE DES LILAS"), postal("2 rue des Lilas")],
        websites: vec![site("HTTPS://EXAMPLE.COM"), site("https://example.org")],
        ..named("Alice", "b@example.com")
    });
    let merged = merge_contacts(&keep, &[&other]).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(merged.title(), Some("Directrice"));
    assert_eq!(merged.birthday(), keep.birthday());
    let streets: Vec<_> = merged
        .postal_addresses()
        .iter()
        .map(|p| p.street())
        .collect();
    assert_eq!(streets, [Some("1 rue des Lilas"), Some("2 rue des Lilas")]);
    let urls: Vec<_> = merged.websites().iter().map(|w| w.url()).collect();
    assert_eq!(urls, ["https://example.com", "https://example.org"]);
    // Trop d'adresses postales réunies : la fusion est refusée.
    let crowded = |first: usize| {
        draft(ContactInput {
            postal_addresses: (first..first + MAX_POSTAL_ADDRESSES)
                .map(|n| postal(&format!("{n} rue")))
                .collect(),
            ..named("Alice", &format!("x{first}@example.com"))
        })
    };
    assert_eq!(
        merge_contacts(&crowded(0), &[&crowded(100)]),
        Err(ContactError::TooManyPostalAddresses)
    );
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

// --- Bornes ------------------------------------------------------------------

#[test]
fn a_group_keeps_at_most_the_most_complete_cards_and_says_so() {
    let n = MAX_DUPLICATE_GROUP_SIZE + 5;
    let book: Vec<Contact> = (1..=n)
        .map(|i| {
            let i = i64::try_from(i).unwrap_or(0);
            // Les cinq premières fiches sont les moins complètes.
            let organization = if i > 5 { "ACME" } else { "" };
            stored(
                i,
                ContactInput {
                    organization: organization.into(),
                    ..named("Support", &format!("s{i}@example.com"))
                },
            )
        })
        .collect();
    let found = find_duplicates(&book, &[], &FakeNfc);
    assert_eq!(found.len(), 1);
    assert!(found[0].truncated);
    assert_eq!(found[0].contacts.len(), MAX_DUPLICATE_GROUP_SIZE);
    assert!(found[0].contacts.iter().all(|c| c.get() > 5));
    // Fiche gardée, puis les autres par identifiant.
    let rest: Vec<i64> = found[0].contacts[1..].iter().map(|c| c.get()).collect();
    let mut sorted = rest.clone();
    sorted.sort_unstable();
    assert_eq!(rest, sorted);

    let small = [
        stored(1, named("Alice", "a@example.com")),
        stored(2, named("Alice", "b@example.com")),
    ];
    assert!(!find_duplicates(&small, &[], &FakeNfc)[0].truncated);
}

#[test]
fn a_search_returns_at_most_a_bounded_number_of_groups() {
    let book: Vec<Contact> = (0..MAX_DUPLICATE_GROUPS + 3)
        .flat_map(|g| {
            let base = i64::try_from(2 * g).unwrap_or(0);
            let name = format!("Nom{g:05}");
            [
                stored(base + 1, named(&name, &format!("a{g}@example.com"))),
                stored(base + 2, named(&name, &format!("b{g}@example.com"))),
            ]
        })
        .collect();
    let found = find_duplicates(&book, &[], &FakeNfc);
    assert_eq!(found.len(), MAX_DUPLICATE_GROUPS);
    // Les premiers dans l'ordre des noms.
    assert_eq!(found[0].contacts[0].get(), 1);
}

// --- Adresses Gmail ----------------------------------------------------------

#[test]
fn gmail_key_ignores_dots_and_the_googlemail_domain() {
    assert_eq!(
        gmail_key(&akey("Jean.Dupont@gmail.com")).as_deref(),
        Some("jeandupont@gmail.com")
    );
    assert_eq!(
        gmail_key(&akey("j.e.a.n.dupont@googlemail.com")).as_deref(),
        Some("jeandupont@gmail.com")
    );
    assert_eq!(
        gmail_key(&akey("jeandupont@GoogleMail.com")).as_deref(),
        Some("jeandupont@gmail.com")
    );
}

#[test]
fn gmail_key_leaves_other_providers_aside() {
    assert_eq!(gmail_key(&akey("jean.dupont@example.com")), None);
    assert_eq!(gmail_key(&akey("jean.dupont@gmail.com.example")), None);
    assert_eq!(gmail_key(&akey("jean.dupont@mail.gmail.com")), None);
}

#[test]
fn gmail_addresses_of_the_same_mailbox_form_a_group() {
    let book = [
        stored(1, named("Jean", "jean.dupont@gmail.com")),
        stored(2, named("JD", "jeandupont@googlemail.com")),
        stored(3, named("Autre", "jean.dupont@example.com")),
        stored(4, named("Encore", "jeandupont@example.com")),
    ];
    assert_eq!(
        groups(&book),
        [(vec![1, 2], vec![DuplicateReason::SameGmailAddress])]
    );
}

#[test]
fn a_card_with_two_spellings_of_its_gmail_address_is_not_its_own_duplicate() {
    let book = [stored(
        1,
        ContactInput {
            emails: vec![
                email("jean.dupont@gmail.com"),
                email("jeandupont@gmail.com"),
            ],
            ..named("Jean", "x@example.com")
        },
    )];
    assert!(groups(&book).is_empty());
}

// --- Fiche proposée d'office -------------------------------------------------

#[test]
fn a_card_is_suggested_as_kept_only_for_a_group_of_the_same_name() {
    let book = [
        stored(1, with_phone("Alice", "a1@example.com", "06 12 34 56 78")),
        stored(2, with_phone("Alice", "a2@example.com", "06 12 34 56 78")),
        stored(3, with_phone("Bob", "bob@example.com", "01 23 45 67 89")),
        stored(
            4,
            with_phone("Robert", "robert@example.com", "01 23 45 67 89"),
        ),
        stored(5, named("Carole", "carole.x@gmail.com")),
        stored(6, named("Caro", "carolex@gmail.com")),
    ];
    let found: Vec<(Vec<i64>, bool)> = find_duplicates(&book, &[], &FakeNfc)
        .iter()
        .map(|g| (ids(g), g.suggest_keep))
        .collect();
    assert_eq!(
        found,
        [(vec![1, 2], true), (vec![3, 4], false), (vec![5, 6], false)]
    );
}

#[test]
fn no_card_is_suggested_for_names_that_differ_only_by_accents() {
    // « й » et « и » ne sont pas la même lettre : rapprochées pour que
    // l'utilisateur en juge, mais aucune fiche n'est cochée d'office.
    let book = [
        stored(1, named("Célia Dupré", "celia@perso.example")),
        stored(
            2,
            with_phone("Celia Dupre", "celia@import.example", "06 12 34 56 78"),
        ),
    ];
    let found: Vec<(Vec<i64>, bool)> = find_duplicates(&book, &[], &FakeNfc)
        .iter()
        .map(|g| (ids(g), g.suggest_keep))
        .collect();
    assert_eq!(found, [(vec![2, 1], false)]);
}

#[test]
fn a_card_with_an_exact_namesake_is_suggested_among_accent_variants() {
    let book = [
        stored(1, named("Célia Dupré", "celia@perso.example")),
        stored(
            2,
            with_phone("Célia Dupré", "celia@travail.example", "06 12 34 56 78"),
        ),
        stored(3, named("Celia Dupre", "celia@import.example")),
    ];
    let suggested = |dismissed: &[DismissedPair]| -> Vec<(Vec<i64>, bool)> {
        find_duplicates(&book, dismissed, &FakeNfc)
            .iter()
            .map(|g| (ids(g), g.suggest_keep))
            .collect()
    };
    assert_eq!(suggested(&[]), [(vec![2, 1, 3], true)]);
    // Le seul homonyme exact écarté : la fiche n'est plus rapprochée que
    // par les accents.
    assert_eq!(suggested(&[pair(1, 2)]), [(vec![2, 1, 3], false)]);
}

// --- Groupes écartés ---------------------------------------------------------

#[test]
fn a_dismissed_group_is_no_longer_proposed() {
    let book = [
        stored(1, with_phone("Alice", "a1@example.com", "06 12 34 56 78")),
        stored(2, with_phone("Alice", "a2@example.com", "06 12 34 56 78")),
        stored(3, named("Bob", "b1@example.com")),
        stored(4, named("Bob", "b2@example.com")),
    ];
    assert_eq!(
        groups_without(&book, &[pair(2, 1)]),
        [(vec![3, 4], vec![DuplicateReason::SameName])]
    );
}

#[test]
fn only_the_dismissed_links_are_removed_before_forming_groups() {
    let book = [
        stored(1, named("Alice", "a1@example.com")),
        stored(2, named("Alice", "a2@example.com")),
        stored(3, named("Alice", "a3@example.com")),
    ];
    let same_name = vec![DuplicateReason::SameName];
    // 1 et 2 restent réunies par 3, qui n'a été écartée de personne.
    assert_eq!(
        groups_without(&book, &[pair(1, 2)]),
        [(vec![1, 2, 3], same_name.clone())]
    );
    assert_eq!(
        groups_without(&book, &[pair(1, 2), pair(1, 3)]),
        [(vec![2, 3], same_name)]
    );
    assert!(groups_without(&book, &[pair(1, 2), pair(1, 3), pair(2, 3)]).is_empty());
}

#[test]
fn the_reasons_of_a_group_come_from_its_remaining_links() {
    let book = [
        stored(1, named("Alice", "a1@example.com")),
        stored(2, with_phone("Alice", "a2@example.com", "06 12 34 56 78")),
        stored(3, with_phone("Bob", "b@example.com", "06 12 34 56 78")),
    ];
    let found = find_duplicates(&book, &[pair(1, 2)], &FakeNfc);
    assert_eq!(found.len(), 1);
    assert_eq!(ids(&found[0]), [2, 3]);
    assert_eq!(found[0].reasons, [DuplicateReason::SamePhone]);
    assert!(!found[0].suggest_keep);
}

#[test]
fn dismissed_links_are_removed_whatever_their_reason() {
    let book = [
        stored(1, with_phone("Alice", "a1@example.com", "06 12 34 56 78")),
        stored(
            2,
            with_phone("Alice", "alice.x@gmail.com", "06 12 34 56 78"),
        ),
        stored(3, named("Lili", "alicex@gmail.com")),
    ];
    assert!(groups_without(&book, &[pair(1, 2), pair(1, 3), pair(2, 3)]).is_empty());
    assert_eq!(
        groups_without(&book, &[pair(1, 2)]),
        [(vec![2, 3], vec![DuplicateReason::SameGmailAddress])]
    );
}

#[test]
fn dismissed_pairs_of_unknown_cards_are_ignored() {
    let book = [
        stored(1, named("Alice", "a1@example.com")),
        stored(2, named("Alice", "a2@example.com")),
    ];
    assert_eq!(groups_without(&book, &[pair(1, 9), pair(7, 8)]).len(), 1);
}

#[test]
fn a_dismissed_pair_is_ordered_and_joins_two_distinct_cards() {
    assert_eq!(pair(3, 1).low(), id(1));
    assert_eq!(pair(3, 1).high(), id(3));
    assert_eq!(pair(3, 1), pair(1, 3));
    assert_eq!(DismissedPair::new(id(2), id(2)), None);
}

#[test]
fn a_dismissal_holds_every_pair_of_the_group_once() {
    let dismissal = Dismissal::new(&[3_u32, 1, 2, 1]).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(dismissal.pairs(), [pair(1, 2), pair(1, 3), pair(2, 3)]);
}

#[test]
fn a_dismissal_is_refused_when_invalid() {
    let size = u32::try_from(MAX_DUPLICATE_GROUP_SIZE).unwrap_or(u32::MAX);
    assert_eq!(Dismissal::new(&[0_u32, 1]), Err(DismissalError::InvalidId));
    assert_eq!(Dismissal::new(&[1_u32]), Err(DismissalError::TooFew));
    assert_eq!(Dismissal::new(&[1_u32, 1]), Err(DismissalError::TooFew));
    let over: Vec<u32> = (1..=size + 1).collect();
    assert_eq!(Dismissal::new(&over), Err(DismissalError::TooMany));
    let repeated = vec![1_u32; 3 * MAX_DUPLICATE_GROUP_SIZE];
    assert_eq!(Dismissal::new(&repeated), Err(DismissalError::TooMany));
    let full: Vec<u32> = (1..=size).collect();
    assert_eq!(
        Dismissal::new(&full).map(|d| d.pairs().len()),
        Ok(MAX_DUPLICATE_GROUP_SIZE * (MAX_DUPLICATE_GROUP_SIZE - 1) / 2)
    );
}

#[test]
fn a_card_joined_without_its_name_is_never_suggested_as_kept() {
    // Alice (1) et Alice (2) portent le même nom ; la fiche 3, la plus
    // remplie, ne les rejoint que par le téléphone de 2 : elle n'est pas
    // proposée d'office.
    let hostile = ContactInput {
        organization: "Banque".into(),
        notes: "Rappeler".into(),
        emails: vec![email("x0@example.com"), email("x1@example.com")],
        phones: vec![phone("06 12 34 56 78"), phone("01 99 99 99 99")],
        ..named("Banque", "x0@example.com")
    };
    let book = [
        stored(1, named("Alice", "a1@example.com")),
        stored(2, with_phone("Alice", "a2@example.com", "06 12 34 56 78")),
        stored(3, hostile),
    ];
    let found = find_duplicates(&book, &[], &FakeNfc);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].contacts[0], id(3), "la plus complète d'abord");
    assert!(
        !found[0].suggest_keep,
        "fiche rapprochée sans le nom proposée"
    );
}

#[test]
fn many_dismissed_pairs_in_one_bucket_still_form_the_right_groups() {
    // Dix fiches de même nom : les paires entre pairs et impairs sont
    // écartées, il reste deux groupes (pairs, impairs).
    let book: Vec<Contact> = (1..=10)
        .map(|i| stored(i, named("Alice", &format!("a{i}@example.com"))))
        .collect();
    let mut dismissed = Vec::new();
    for a in 1..=10 {
        for b in (a + 1)..=10 {
            if (a + b) % 2 == 1 {
                dismissed.push(pair(a, b));
            }
        }
    }
    let mut found: Vec<Vec<i64>> = groups_without(&book, &dismissed)
        .into_iter()
        .map(|(mut ids, _)| {
            ids.sort_unstable();
            ids
        })
        .collect();
    found.sort();
    assert_eq!(found, [vec![1, 3, 5, 7, 9], vec![2, 4, 6, 8, 10]]);
}
