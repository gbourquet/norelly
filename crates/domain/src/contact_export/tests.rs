use proptest::prelude::*;

use super::*;
use crate::contacts::{ContactInput, EmailInput, PhoneInput};

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

/// Fiche complète : adresse préférée en deuxième, étiquettes variées.
fn full() -> ContactDraft {
    draft(ContactInput {
        display_name: "Élodie C.".into(),
        given_name: "Élodie".into(),
        family_name: "Cézanne".into(),
        nickname: "Lolo".into(),
        organization: "Atelier Sud".into(),
        notes: "Première ligne\nseconde ligne".into(),
        emails: vec![
            email("elodie@example.org", "Maison", false),
            email("elodie.cezanne@example.com", "Travail", true),
            email("lolo@example.net", "", false),
        ],
        phones: vec![
            phone("+33 6 12 34 56 78", "Portable"),
            phone("04 72 00 00 00", ""),
        ],
        favorite: true,
    })
}

fn addresses(card: &ExportCard) -> Vec<(&str, bool)> {
    card.emails
        .iter()
        .map(|e| (e.address.as_str(), e.preferred))
        .collect()
}

fn email_labels(card: &ExportCard) -> Vec<Option<&str>> {
    card.emails.iter().map(|e| e.label.as_deref()).collect()
}

fn phone_labels(card: &ExportCard) -> Vec<Option<&str>> {
    card.phones.iter().map(|p| p.label.as_deref()).collect()
}

const ALL_FORMATS: [ExportFormat; 4] = [
    ExportFormat::VCard4,
    ExportFormat::VCard3,
    ExportFormat::GoogleCsv,
    ExportFormat::Ldif,
];

#[test]
fn extensions_follow_the_format() {
    assert_eq!(ExportFormat::VCard4.extension(), "vcf");
    assert_eq!(ExportFormat::VCard3.extension(), "vcf");
    assert_eq!(ExportFormat::GoogleCsv.extension(), "csv");
    assert_eq!(ExportFormat::Ldif.extension(), "ldif");
}

#[test]
fn selection_is_validated_and_deduplicated() {
    let id = |n| ContactId::new(n).unwrap_or_else(|| panic!("id"));
    assert_eq!(
        ContactSelection::of(&[3_i64, 1, 3]),
        Ok(ContactSelection::Contacts(vec![id(3), id(1)]))
    );
    assert_eq!(ContactSelection::of::<i64>(&[]), Err(SelectionError::Empty));
    assert_eq!(
        ContactSelection::of(&[1_i64, 0]),
        Err(SelectionError::InvalidId)
    );
    let too_many: Vec<i64> = (1..=i64::try_from(MAX_CONTACT_SELECTION).unwrap_or(0) + 1).collect();
    assert_eq!(
        ContactSelection::of(&too_many),
        Err(SelectionError::TooMany)
    );
    let max: Vec<i64> = (1..=i64::try_from(MAX_CONTACT_SELECTION).unwrap_or(0)).collect();
    assert!(ContactSelection::of(&max).is_ok());
}

#[test]
fn the_preferred_address_comes_first_in_every_format() {
    for format in ALL_FORMATS {
        let card = export_card(&full(), format);
        assert_eq!(
            addresses(&card),
            [
                ("elodie.cezanne@example.com", true),
                ("elodie@example.org", false),
                ("lolo@example.net", false),
            ],
            "{format:?}"
        );
    }
}

#[test]
fn names_organization_and_notes_are_kept_in_every_format() {
    for format in ALL_FORMATS {
        let card = export_card(&full(), format);
        assert_eq!(card.full_name.as_deref(), Some("Élodie C."), "{format:?}");
        assert_eq!(card.given_name.as_deref(), Some("Élodie"));
        assert_eq!(card.family_name.as_deref(), Some("Cézanne"));
        assert_eq!(card.nickname.as_deref(), Some("Lolo"));
        assert_eq!(card.organization.as_deref(), Some("Atelier Sud"));
        assert_eq!(card.notes.as_deref(), Some("Première ligne\nseconde ligne"));
        let numbers: Vec<&str> = card.phones.iter().map(|p| p.number.as_str()).collect();
        assert_eq!(numbers, ["+33 6 12 34 56 78", "04 72 00 00 00"]);
    }
}

#[test]
fn vcard_full_name_falls_back_to_the_composed_name() {
    let composed = draft(ContactInput {
        given_name: "Anne".into(),
        family_name: "Martin".into(),
        emails: vec![email("anne@example.com", "", false)],
        ..ContactInput::default()
    });
    for format in [ExportFormat::VCard4, ExportFormat::VCard3] {
        assert_eq!(
            export_card(&composed, format).full_name.as_deref(),
            Some("Anne Martin")
        );
    }
    // Les autres formats ont une colonne pour chaque nom : le nom affiché
    // absent le reste.
    for format in [ExportFormat::GoogleCsv, ExportFormat::Ldif] {
        assert_eq!(export_card(&composed, format).full_name, None);
    }

    let nickname_only = draft(ContactInput {
        nickname: "Lolo".into(),
        emails: vec![email("lolo@example.com", "", false)],
        ..ContactInput::default()
    });
    assert_eq!(
        export_card(&nickname_only, ExportFormat::VCard4)
            .full_name
            .as_deref(),
        Some("Lolo")
    );
}

#[test]
fn vcard_full_name_is_empty_rather_than_an_address_or_too_long() {
    // Sans nom, le nom affiché serait l'adresse : l'import le retirerait.
    let anonymous = draft(ContactInput {
        emails: vec![email("anonyme@example.com", "", false)],
        ..ContactInput::default()
    });
    assert_eq!(
        export_card(&anonymous, ExportFormat::VCard4).full_name,
        None
    );

    // Prénom et nom de longueur maximale : le nom composé dépasserait la
    // longueur d'un nom, l'import refuserait la fiche.
    let long = "a".repeat(crate::contacts::MAX_CONTACT_NAME_CHARS);
    let long_names = draft(ContactInput {
        given_name: long.clone(),
        family_name: long,
        emails: vec![email("long@example.com", "", false)],
        ..ContactInput::default()
    });
    assert_eq!(
        export_card(&long_names, ExportFormat::VCard3).full_name,
        None
    );
}

#[test]
fn vcard_labels_are_lowercase_types() {
    let card = export_card(&full(), ExportFormat::VCard4);
    assert_eq!(email_labels(&card), [Some("travail"), Some("maison"), None]);
    assert_eq!(phone_labels(&card), [Some("portable"), None]);
}

#[test]
fn vcard_labels_drop_reserved_words_and_unsafe_characters() {
    let labeled = |label: &str| {
        let d = draft(ContactInput {
            emails: vec![email("a@example.com", label, false)],
            ..ContactInput::default()
        });
        export_card(&d, ExportFormat::VCard3).emails[0]
            .label
            .clone()
    };
    // Mots qui changeraient la lecture de la carte (préférence, encodage,
    // types techniques) : pas d'étiquette.
    for reserved in [
        "PREF",
        "internet",
        "x400",
        "voice",
        "text",
        "x-perso",
        "Quoted-Printable",
        "base64",
        "b",
        "8bit",
        "7bit",
    ] {
        assert_eq!(labeled(reserved), None, "{reserved}");
    }
    // Séparateurs et guillemets de la syntaxe vCard remplacés par une
    // espace, espaces resserrées.
    assert_eq!(
        labeled("Bureau, \"Lyon\"; 2:e").as_deref(),
        Some("bureau lyon 2 e")
    );
    assert_eq!(labeled("Maison à l'île").as_deref(), Some("maison à l'île"));
    assert_eq!(labeled(",;").as_deref(), None);
    // Caret : échappement des paramètres en vCard 4.0 (RFC 6868).
    assert_eq!(labeled("a^nb").as_deref(), Some("a nb"));
}

#[test]
fn google_labels_keep_their_case_without_a_leading_star() {
    let card = export_card(&full(), ExportFormat::GoogleCsv);
    assert_eq!(email_labels(&card), [Some("Travail"), Some("Maison"), None]);
    assert_eq!(phone_labels(&card), [Some("Portable"), None]);

    // « * » en tête marque l'adresse principale chez Google.
    let starred = draft(ContactInput {
        emails: vec![
            email("a@example.com", "", true),
            email("b@example.com", "* Perso", false),
            email("c@example.com", "* * Pro", false),
        ],
        phones: vec![phone("1", "**"), phone("2", "* *")],
        ..ContactInput::default()
    });
    let card = export_card(&starred, ExportFormat::GoogleCsv);
    assert_eq!(email_labels(&card), [None, Some("Perso"), Some("Pro")]);
    assert_eq!(phone_labels(&card), [None, None]);
}

#[test]
fn ldif_has_no_address_labels_and_maps_phones_to_thunderbird_attributes() {
    let d = draft(ContactInput {
        emails: vec![email("a@example.com", "Travail", false)],
        phones: vec![
            phone("1", "Travail"),
            phone("2", "Maison"),
            phone("3", "Portable"),
            phone("4", "Fax"),
            phone("5", "Pager"),
            phone("6", ""),
            phone("7", "Chalet"),
            phone("8", "MOBILE"),
            phone("9", "domicile"),
            phone("10", "cell"),
            phone("11", "bipeur"),
            phone("12", "Télécopie"),
            phone("13", "home"),
            phone("14", "work"),
            phone("15", "bureau"),
        ],
        ..ContactInput::default()
    });
    let card = export_card(&d, ExportFormat::Ldif);
    assert_eq!(email_labels(&card), [None]);
    assert_eq!(
        phone_labels(&card),
        [
            Some("work"),
            Some("home"),
            Some("cell"),
            Some("fax"),
            Some("pager"),
            Some("work"),
            Some("work"),
            Some("cell"),
            Some("home"),
            Some("cell"),
            Some("pager"),
            Some("fax"),
            Some("home"),
            Some("work"),
            Some("work"),
        ]
    );
}

#[test]
fn csv_cells_that_look_like_formulas_are_neutralized() {
    for (raw, written) in [
        ("=1+1", "'=1+1"),
        ("+33 6 12", "'+33 6 12"),
        ("-x", "'-x"),
        ("@SUM(A1)", "'@SUM(A1)"),
        ("\tx", "'\tx"),
        ("\rx", "'\rx"),
        // Signes pleine chasse, que certains tableurs assimilent.
        ("＝1", "'＝1"),
        ("＋33", "'＋33"),
        ("－x", "'－x"),
        ("＠x", "'＠x"),
        // Une apostrophe déjà là serait retirée à la relecture : doublée.
        ("'=x", "''=x"),
        ("''+x", "'''+x"),
    ] {
        assert_eq!(neutralize_csv_cell(raw), written, "{raw:?}");
        assert_eq!(restore_csv_cell(written), raw, "{written:?}");
    }
    for plain in [
        "",
        "Anne",
        "'apostrophe",
        "a=b",
        "06 12",
        "''",
        "x@example.com",
    ] {
        assert!(
            matches!(neutralize_csv_cell(plain), Cow::Borrowed(_)),
            "{plain:?}"
        );
        assert_eq!(restore_csv_cell(plain), plain, "{plain:?}");
    }
}

#[test]
fn restoring_removes_the_apostrophe_of_other_spreadsheets() {
    assert_eq!(restore_csv_cell("'+33 6"), "+33 6");
    assert_eq!(restore_csv_cell("'Anne"), "'Anne");
    assert_eq!(restore_csv_cell("'"), "'");
}

proptest! {
    #[test]
    fn restore_undoes_neutralize(cell in "['=+@\t\r a-z-]{0,8}") {
        let written = neutralize_csv_cell(&cell);
        prop_assert_eq!(restore_csv_cell(&written), cell.as_str());
    }

    #[test]
    fn neutralized_cells_never_start_with_a_formula(cell in "\\PC{0,8}") {
        let out = neutralize_csv_cell(&cell);
        prop_assert!(!out.starts_with(['=', '+', '-', '@', '\t', '\r']));
    }
}
