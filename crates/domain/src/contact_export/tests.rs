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
fn the_favorite_is_kept_in_vcard_and_google_csv_but_not_in_ldif() {
    for format in [
        ExportFormat::VCard4,
        ExportFormat::VCard3,
        ExportFormat::GoogleCsv,
    ] {
        assert!(export_card(&full(), format).favorite, "{format:?}");
    }
    // Thunderbird n'a pas de favori.
    assert!(!export_card(&full(), ExportFormat::Ldif).favorite);
    let mut plain = full().to_input();
    plain.favorite = false;
    for format in ALL_FORMATS {
        assert!(
            !export_card(&draft(plain.clone()), format).favorite,
            "{format:?}"
        );
    }
}

#[test]
fn the_favorite_category_is_recognized_whatever_its_case() {
    assert!(is_favorite_category(VCARD_FAVORITE_CATEGORY));
    for category in ["starred", "Starred", " STARRED "] {
        assert!(is_favorite_category(category), "{category:?}");
    }
    for category in [
        "",
        "myContacts",
        "starred2",
        "* starred",
        "star red",
        "favoris",
    ] {
        assert!(!is_favorite_category(category), "{category:?}");
    }
}

#[test]
fn the_favorite_google_label_is_the_starred_system_group() {
    assert!(is_favorite_google_label(GOOGLE_FAVORITE_LABEL));
    // Ancien export de Google : « * Starred » (colonne « Group Membership »).
    for label in ["* starred", "* Starred", "  *starred ", "*  STARRED"] {
        assert!(is_favorite_google_label(label), "{label:?}");
    }
    // Sans « * », c'est une étiquette créée par l'utilisateur.
    for label in [
        "",
        "starred",
        "Starred",
        "* myContacts",
        "** starred",
        "* starred x",
    ] {
        assert!(!is_favorite_google_label(label), "{label:?}");
    }
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
        // Petites formes et signe moins, neutralisés à titre préventif.
        ("\u{FE62}1", "'\u{FE62}1"),
        ("\u{FE63}x", "'\u{FE63}x"),
        ("\u{FE66}1", "'\u{FE66}1"),
        ("\u{FE6B}x", "'\u{FE6B}x"),
        ("\u{2212}1", "'\u{2212}1"),
        ("'\u{2212}1", "''\u{2212}1"),
        // Exposants et indices de `+`, `=`, `-`, et le plus hébreu, qui se
        // ramènent aux mêmes signes par normalisation de compatibilité.
        ("\u{207A}1", "'\u{207A}1"),
        ("\u{208A}1", "'\u{208A}1"),
        ("\u{FB29}1", "'\u{FB29}1"),
        ("\u{207C}1", "'\u{207C}1"),
        ("\u{208C}1", "'\u{208C}1"),
        ("\u{207B}x", "'\u{207B}x"),
        ("\u{208B}x", "'\u{208B}x"),
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
        "1\u{2212}2",
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

fn temporary_of(target: &str, candidate: &str) -> bool {
    is_export_temporary_of(target.as_bytes(), candidate.as_bytes())
}

#[test]
fn export_temporaries_of_the_target_are_recognized() {
    for name in [
        ".contacts.vcf.1234-0.tmp",
        ".contacts.vcf.1-99.tmp",
        ".contacts.vcf.4294967295-0.tmp",
    ] {
        assert!(temporary_of("contacts.vcf", name), "{name:?}");
    }
    assert!(temporary_of("a b.csv", ".a b.csv.7-3.tmp"));
    // Nom qui n'est pas de l'UTF-8 : comparé octet par octet.
    assert!(is_export_temporary_of(b"c\xff.vcf", b".c\xff.vcf.5-0.tmp"));
    assert!(!is_export_temporary_of(b"c\xff.vcf", b".c\xfe.vcf.5-0.tmp"));
}

#[test]
fn the_case_of_the_name_is_ignored() {
    // Un système de fichiers insensible à la casse (Windows, clé USB) voit
    // « Contacts.vcf » et « contacts.vcf » comme un seul fichier.
    assert!(temporary_of("Contacts.vcf", ".contacts.VCF.3-0.tmp"));
    assert!(temporary_of("Carnet-É.vcf", ".carnet-é.vcf.3-0.tmp"));
    assert!(is_export_temporary_of(b"C\xff.vcf", b".c\xff.VCF.5-0.tmp"));
    // La forme reste exacte : ni « .TMP » ni autre lettre.
    assert!(!temporary_of("Contacts.vcf", ".Contacts.vcf.3-0.TMP"));
    assert!(!temporary_of("Carnet-É.vcf", ".carnet-e.vcf.3-0.tmp"));
}

#[test]
fn only_old_unlocked_temporaries_are_stale() {
    let old = Some(STALE_EXPORT_TEMPORARY_AGE);
    assert!(is_stale_export_temporary(TemporaryLock::Free, old));
    assert!(is_stale_export_temporary(
        TemporaryLock::Free,
        Some(STALE_EXPORT_TEMPORARY_AGE * 100)
    ));
    // Export peut-être en cours : verrou tenu ou impossible à vérifier,
    // temporaire récent ou d'âge inconnu.
    assert!(!is_stale_export_temporary(TemporaryLock::Held, old));
    assert!(!is_stale_export_temporary(TemporaryLock::Unknown, old));
    assert!(!is_stale_export_temporary(
        TemporaryLock::Free,
        Some(STALE_EXPORT_TEMPORARY_AGE - Duration::from_secs(1))
    ));
    assert!(!is_stale_export_temporary(TemporaryLock::Free, None));
}

#[test]
fn other_names_are_not_export_temporaries() {
    for name in [
        "contacts.vcf",
        ".contacts.vcf",
        "contacts.vcf.1234-0.tmp",
        ".contacts.vcf.1234-0.tmp.bak",
        ".contacts.vcf.1234-0",
        ".contacts.vcf.1234.tmp",
        ".contacts.vcf.-0.tmp",
        ".contacts.vcf.1234-.tmp",
        ".contacts.vcf.12a4-0.tmp",
        ".contacts.vcf.+1-0.tmp",
        ".contacts.vcf.1-2-3.tmp",
        ".contacts.vcf..1-0.tmp",
        ".contacts.vcf.1-0.tmp.2-0.tmp",
        ".autre.vcf.1234-0.tmp",
        ".contacts.vcf2.1234-0.tmp",
        ".contacts.vcf.12345678901-0.tmp",
        ".contacts.vcf.1-12345678901.tmp",
        ".contacts.vcf.4294967296-0.tmp",
        ".contacts.vcf.1-4294967296.tmp",
        ".contacts.vcf.\u{661}-0.tmp",
        "..1-0.tmp",
    ] {
        assert!(!temporary_of("contacts.vcf", name), "{name:?}");
    }
    // Sans nom de cible, rien n'est un temporaire.
    assert!(!temporary_of("", "..1-0.tmp"));
}

/// Débuts qu'aucune cellule CSV exportée ne présente : signes de formule,
/// leurs formes de compatibilité (pleine chasse, petites, exposants,
/// indices, plus hébreu), signe moins, tabulation, retour chariot.
const FORMULA_LIKE: [char; 22] = [
    '=', '+', '-', '@', '\t', '\r', '\u{FF1D}', '\u{FF0B}', '\u{FF0D}', '\u{FF20}', '\u{FE62}',
    '\u{FE63}', '\u{FE66}', '\u{FE6B}', '\u{2212}', '\u{207A}', '\u{208A}', '\u{FB29}', '\u{207C}',
    '\u{208C}', '\u{207B}', '\u{208B}',
];

proptest! {
    #[test]
    fn written_temporaries_are_recognized(target in "[^/\\\\\\x00]{1,12}", pid: u32, n in 0_u32..100) {
        let name = format!(".{target}.{pid}-{n}.tmp");
        prop_assert!(temporary_of(&target, &name));
    }

    #[test]
    fn restore_undoes_neutralize(
        cell in "['=+@\t\r a-z\u{FF1D}\u{FF0B}\u{FF0D}\u{FF20}\u{FE62}\u{FE63}\u{FE66}\u{FE6B}\u{2212}\u{207A}\u{208A}\u{FB29}\u{207C}\u{208C}\u{207B}\u{208B}-]{0,8}"
    ) {
        let written = neutralize_csv_cell(&cell);
        prop_assert_eq!(restore_csv_cell(&written), cell.as_str());
    }

    #[test]
    fn neutralized_cells_never_start_with_a_formula(
        cell in "['=+@\u{FF1D}\u{FF0B}\u{FF0D}\u{FF20}\u{FE62}\u{FE63}\u{FE66}\u{FE6B}\u{2212}\u{207A}\u{208A}\u{FB29}\u{207C}\u{208C}\u{207B}\u{208B}-]?\\PC{0,7}"
    ) {
        let out = neutralize_csv_cell(&cell);
        prop_assert!(!out.starts_with(FORMULA_LIKE));
    }
}

#[test]
fn the_proposed_file_name_is_the_translated_stem_with_the_format_extension() {
    assert_eq!(
        export_file_name("carnet", ExportFormat::VCard4),
        "carnet.vcf"
    );
    assert_eq!(
        export_file_name("Mes contacts", ExportFormat::VCard3),
        "Mes contacts.vcf"
    );
    assert_eq!(
        export_file_name("Kontakte", ExportFormat::GoogleCsv),
        "Kontakte.csv"
    );
    assert_eq!(
        export_file_name("Adressbuch", ExportFormat::Ldif),
        "Adressbuch.ldif"
    );
}

#[test]
fn an_empty_or_unusable_stem_falls_back_to_contacts() {
    for raw in ["", "   ", "...", " . ", "\u{3000}.."] {
        assert_eq!(
            export_file_name(raw, ExportFormat::VCard4),
            "contacts.vcf",
            "{raw:?}"
        );
    }
    assert_eq!(export_file_name("", ExportFormat::Ldif), "contacts.ldif");
    assert_eq!(
        export_file_name(" .. ", ExportFormat::GoogleCsv),
        "contacts.csv"
    );
}

#[test]
fn the_proposed_file_name_never_leaves_the_chosen_folder() {
    assert_eq!(
        export_file_name("../../.ssh/authorized_keys", ExportFormat::VCard4),
        "_____ssh_authorized_keys.vcf"
    );
    assert_eq!(
        export_file_name("C:\\Windows\\carnet", ExportFormat::Ldif),
        "C__Windows_carnet.ldif"
    );
    // Nom réservé de Windows : préfixé.
    assert_eq!(export_file_name("CON", ExportFormat::VCard4), "_CON.vcf");
    // Le format décide de l'extension, pas l'interface : aucune extension
    // intermédiaire (« carnet.exe » affiché quand les extensions sont
    // masquées).
    assert_eq!(
        export_file_name("carnet.exe", ExportFormat::GoogleCsv),
        "carnet_exe.csv"
    );
    assert_eq!(
        export_file_name("con.vcf", ExportFormat::VCard4),
        "_con_vcf.vcf"
    );
}

proptest! {
    #[test]
    fn the_proposed_file_name_is_safe_bounded_and_keeps_the_extension(stem in "\\PC{0,400}") {
        for format in [
            ExportFormat::VCard4,
            ExportFormat::VCard3,
            ExportFormat::GoogleCsv,
            ExportFormat::Ldif,
        ] {
            let name = export_file_name(&stem, format);
            prop_assert!(name.len() <= crate::reading::MAX_FILE_NAME_BYTES, "{name:?}");
            let suffix = format!(".{}", format.extension());
            prop_assert!(name.ends_with(&suffix), "{name:?}");
            prop_assert!(name.len() > suffix.len(), "{name:?}");
            prop_assert_eq!(crate::reading::safe_file_name(&name), name.clone());
        }
    }
}

#[test]
fn an_export_label_is_bounded_before_any_processing() {
    assert_eq!(export_label("Enregistrer"), "Enregistrer");
    let long = "é".repeat(MAX_EXPORT_LABEL_BYTES);
    let bounded = export_label(&long);
    assert!(bounded.len() <= MAX_EXPORT_LABEL_BYTES, "{}", bounded.len());
    assert!(
        bounded.len() >= MAX_EXPORT_LABEL_BYTES - 1,
        "{}",
        bounded.len()
    );
    assert!(long.starts_with(bounded));
    let ascii = "a".repeat(MAX_EXPORT_LABEL_BYTES + 10);
    assert_eq!(export_label(&ascii).len(), MAX_EXPORT_LABEL_BYTES);
}
