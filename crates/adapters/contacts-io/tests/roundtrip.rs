//! Aller-retour de l'export : un carnet exporté puis réimporté (dans un
//! carnet vide) redonne le même carnet, à ce que chaque format retient
//! près (fixé par le domaine, `export_card`) : adresse préférée en tête,
//! étiquettes vCard en minuscules, pas d'étiquette d'adresse en LDIF,
//! téléphones LDIF rangés dans les attributs de Thunderbird, nom complet
//! d'une vCard toujours écrit. Le favori n'est porté par aucun format.

use norelly_contacts_io::{ContactsIo, IcuNfc};
use norelly_domain::contact_export::{ExportCard, ExportFormat, export_card};
use norelly_domain::contact_import::{DuplicatePolicy, ImportPlan, plan_import};
use norelly_domain::contacts::{ContactDraft, ContactInput, EmailInput, PhoneInput};
use norelly_domain::ports::ContactCodec;
use norelly_test_support::contact_import_contracts::{EXPORT_FORMATS, imported};
use proptest::prelude::*;

fn email(address: &str, label: &str, preferred: bool) -> EmailInput {
    EmailInput {
        address: address.into(),
        label: label.into(),
        preferred,
    }
}

fn phone(number: &str, label: &str) -> PhoneInput {
    PhoneInput {
        number: number.into(),
        label: label.into(),
    }
}

fn draft(input: ContactInput) -> ContactDraft {
    ContactDraft::new(input).unwrap_or_else(|e| panic!("{e}"))
}

/// Exporte `book` dans `format` et réimporte le fichier dans un carnet
/// vide.
fn round_trip(book: &[ContactDraft], format: ExportFormat) -> ImportPlan {
    let cards: Vec<ExportCard> = book.iter().map(|d| export_card(d, format)).collect();
    let bytes = ContactsIo
        .encode(&cards, format)
        .unwrap_or_else(|e| panic!("{format:?} : {e}"));
    let decoded = ContactsIo
        .decode(&bytes)
        .unwrap_or_else(|e| panic!("{format:?} : {e}"));
    plan_import(&[], &decoded.cards, DuplicatePolicy::Skip, &IcuNfc)
}

/// Carnet d'essai : accents, notes sur plusieurs lignes, plusieurs
/// adresses dont la préférée (en deuxième), étiquettes, téléphone
/// commençant par « + », caractères des syntaxes de chaque format.
fn book() -> Vec<ContactDraft> {
    vec![
        draft(ContactInput {
            display_name: "Hélène Lefèvre".into(),
            given_name: "Hélène".into(),
            family_name: "Lefèvre".into(),
            nickname: "Lène".into(),
            organization: "Mairie de Saint-Étienne".into(),
            notes: "Rencontrée à Lyon, France.\nPréfère le téléphone ; « merci ».\n\tFin.".into(),
            emails: vec![
                email("helene@example.org", "Maison", false),
                email("helene.lefevre@example.fr", "Travail", true),
                email("lene@example.net", "", false),
            ],
            phones: vec![
                phone("+33 6 11 22 33 44", "Portable"),
                phone("04 77 00 00 00", "Bureau"),
            ],
            favorite: false,
        }),
        draft(ContactInput {
            given_name: "Seán".into(),
            family_name: "O'Brien".into(),
            organization: "=Cie; \"Jo\" & fils, Lyon".into(),
            emails: vec![email("sean@example.ie", "", false)],
            ..ContactInput::default()
        }),
        draft(ContactInput {
            emails: vec![email("anonyme@example.com", "", false)],
            ..ContactInput::default()
        }),
    ]
}

/// Le carnet attendu après l'aller-retour : `book` transformé fiche par
/// fiche par `change`.
fn expected(change: impl Fn(&mut ContactInput)) -> Vec<ContactDraft> {
    book()
        .iter()
        .map(|d| {
            let mut input = d.to_input();
            // Adresse préférée en tête, partout.
            input.emails.sort_by_key(|e| !e.preferred);
            change(&mut input);
            draft(input)
        })
        .collect()
}

fn assert_clean(plan: &ImportPlan) {
    assert!(plan.rejected.is_empty(), "{:?}", plan.rejected);
    assert_eq!((plan.duplicates, plan.renamed), (0, 0));
    assert!(plan.complete.is_empty());
}

#[test]
fn vcard_round_trip_keeps_the_book() {
    for format in [ExportFormat::VCard4, ExportFormat::VCard3] {
        let plan = round_trip(&book(), format);
        assert_clean(&plan);
        let want = expected(|c| {
            for e in &mut c.emails {
                e.label = e.label.to_lowercase();
            }
            for p in &mut c.phones {
                p.label = p.label.to_lowercase();
            }
            // Nom complet obligatoire : le nom composé s'écrit.
            if c.display_name.is_empty() && !c.given_name.is_empty() {
                c.display_name = format!("{} {}", c.given_name, c.family_name);
            }
        });
        assert_eq!(plan.create, want, "{format:?}");
    }
}

#[test]
fn google_csv_round_trip_keeps_the_book() {
    let plan = round_trip(&book(), ExportFormat::GoogleCsv);
    assert_clean(&plan);
    assert_eq!(plan.create, expected(|_| {}));
}

#[test]
fn ldif_round_trip_keeps_the_book() {
    let plan = round_trip(&book(), ExportFormat::Ldif);
    assert_clean(&plan);
    let want = expected(|c| {
        for e in &mut c.emails {
            e.label.clear();
        }
        for p in &mut c.phones {
            p.label = match p.label.as_str() {
                "Portable" => "cell",
                _ => "work",
            }
            .into();
        }
    });
    assert_eq!(plan.create, want);
}

#[test]
fn favorites_are_not_exported() {
    let favorite = draft(ContactInput {
        display_name: "Zoé".into(),
        emails: vec![email("zoe@example.com", "", false)],
        favorite: true,
        ..ContactInput::default()
    });
    for format in EXPORT_FORMATS {
        let plan = round_trip(std::slice::from_ref(&favorite), format);
        assert!(!plan.create[0].favorite(), "{format:?}");
    }
}

/// Nom : lettres accentuées, ponctuation des syntaxes vCard, CSV et LDIF,
/// débuts de formule, emoji (sans `@`, `<` ni `>`, qu'un nom ne garde
/// pas à l'import).
fn name_text() -> impl Strategy<Value = String> {
    r#"[a-zA-Zéèçà ŒœÉ😀,;:\\"'=+*#.-]{0,30}"#
}

/// Texte d'une ligne quelconque (organisation).
fn line_text() -> impl Strategy<Value = String> {
    r#"[a-zA-Zéèçà ŒœÉ😀,;:\\"'=+@*#<>.-]{0,30}"#
}

fn notes_text() -> impl Strategy<Value = String> {
    "[a-zé ,;:\\\\\"'=+@\n\t😀-]{0,200}"
}

fn address() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9]{0,6}([._+-][a-z0-9]{1,4})?@[a-z]{1,8}\\.(fr|com|org)"
}

fn label() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        Just("Travail".to_owned()),
        Just("home".to_owned()),
        Just("Portable".to_owned()),
        "[a-zA-Zé ,;\"*=-]{0,12}",
    ]
}

fn contact() -> impl Strategy<Value = ContactDraft> {
    let emails = prop::collection::vec((address(), label()), 1..5);
    let phones = prop::collection::vec(("[+0-9(-][0-9 ().-]{0,19}", label()), 0..4);
    (
        [name_text(), name_text(), name_text(), name_text()],
        line_text(),
        notes_text(),
        emails,
        any::<prop::sample::Index>(),
        phones,
    )
        .prop_filter_map(
            "fiche refusée par le domaine",
            |(names, organization, notes, emails, preferred, phones)| {
                let [display_name, given_name, family_name, nickname] = names;
                let preferred = preferred.index(emails.len());
                ContactDraft::new(ContactInput {
                    display_name,
                    given_name,
                    family_name,
                    nickname,
                    organization,
                    notes,
                    emails: emails
                        .into_iter()
                        .enumerate()
                        .map(|(i, (address, label))| EmailInput {
                            address,
                            label,
                            preferred: i == preferred,
                        })
                        .collect(),
                    phones: phones
                        .into_iter()
                        .map(|(number, label)| PhoneInput { number, label })
                        .collect(),
                    favorite: false,
                })
                .ok()
            },
        )
}

fn format() -> impl Strategy<Value = ExportFormat> {
    prop::sample::select(EXPORT_FORMATS.to_vec())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Ce qui est écrit se relit à l'identique.
    #[test]
    fn decoding_an_export_gives_back_the_cards(
        book in prop::collection::vec(contact(), 1..6),
        format in format(),
    ) {
        let cards: Vec<ExportCard> = book.iter().map(|d| export_card(d, format)).collect();
        let bytes = ContactsIo.encode(&cards, format).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let decoded = ContactsIo.decode(&bytes).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let want: Vec<_> = cards.iter().map(imported).collect();
        prop_assert_eq!(decoded.cards, want);
    }

    /// Un carnet réimporté se réexporte à l'identique : l'aller-retour
    /// n'enlève ou ne change rien de plus que la première fois.
    #[test]
    fn reexporting_a_reimported_book_is_stable(
        book in prop::collection::vec(contact(), 1..6),
        format in format(),
    ) {
        let first = round_trip(&book, format);
        prop_assert!(first.rejected.is_empty(), "{:?}", first.rejected);
        // Un nom qui imite une adresse est retiré à l'import (règle du
        // domaine) : hors du propos de l'aller-retour.
        prop_assume!(first.renamed == 0);
        prop_assert_eq!(first.create.len(), book.len());
        let again = round_trip(&first.create, format);
        prop_assert_eq!(again.create, first.create);
    }
}
