//! Lecture des jeux d'essai (`tests/data`) : exports de Google Contacts
//! (CSV, vCard), Outlook (CSV anglais et français, vCard 2.1), Apple
//! Contacts (vCard 3.0) et Thunderbird (CSV, LDIF), aux données fictives.

use norelly_contacts_io::{ContactsIo, IcuNfc};
use norelly_domain::contact_import::{
    ContactFormat, DecodedContacts, DuplicatePolicy, ImportedCard, RejectReason, plan_import,
};
use norelly_domain::contacts::{ContactError, ContactInput, EmailInput, PhoneInput};
use norelly_domain::ports::ContactCodec;
use norelly_test_support::contact_import_contracts::{
    SAMPLE_VCARD, contact_codec_contract, contact_encoding_contract, sample_export_drafts,
    text_normalizer_contract,
};

fn data(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path} : {e}"))
}

fn decode(name: &str) -> DecodedContacts {
    ContactsIo
        .decode(&data(name))
        .unwrap_or_else(|e| panic!("{name} : {e}"))
}

fn contacts(decoded: &DecodedContacts) -> Vec<&ContactInput> {
    decoded
        .cards
        .iter()
        .filter_map(|c| match c {
            ImportedCard::Contact(input) => Some(input),
            ImportedCard::List | ImportedCard::Unreadable => None,
        })
        .collect()
}

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

#[test]
fn real_codec_passes_contract() {
    let expected = DecodedContacts {
        format: ContactFormat::VCard,
        cards: vec![ImportedCard::Contact(ContactInput {
            display_name: "Alice Martin".into(),
            given_name: "Alice".into(),
            family_name: "Martin".into(),
            emails: vec![email("alice@example.com", "", false)],
            ..ContactInput::default()
        })],
    };
    contact_codec_contract(&ContactsIo, SAMPLE_VCARD, &expected);
}

#[test]
fn real_codec_passes_encoding_contract() {
    contact_encoding_contract(&ContactsIo, &sample_export_drafts());
}

#[test]
fn real_normalizer_passes_contract() {
    text_normalizer_contract(&IcuNfc);
}

#[test]
fn gmail_csv() {
    let d = decode("gmail.csv");
    assert_eq!(d.format, ContactFormat::GoogleCsv);
    assert_eq!(d.cards.len(), 4);
    let c = contacts(&d);
    assert_eq!(
        *c[0],
        ContactInput {
            given_name: "Élodie".into(),
            family_name: "Cézanne".into(),
            nickname: "Lolo".into(),
            organization: "Atelier Sud".into(),
            notes: "Rencontrée à Lyon, France.\r\nPréfère le téléphone.".into(),
            emails: vec![
                email("elodie.cezanne@example.com", "Work", true),
                email("elodie@example.org", "Home", false),
            ],
            phones: vec![
                phone("+33 6 12 34 56 78", "Mobile"),
                phone("+33 4 72 00 00 00", "Work"),
            ],
            ..ContactInput::default()
        }
    );
    // Plusieurs adresses dans une cellule, séparées par « ::: ».
    assert_eq!(
        c[1].emails,
        [
            email("marc.durand@example.net", "", false),
            email("m.durand@example.net", "", false)
        ]
    );
    assert_eq!(c[2].organization, "Plomberie Express");
    assert!(c[3].emails.is_empty());
    assert_eq!(c[3].phones, [phone("06 00 00 00 01", "Mobile")]);
}

#[test]
fn gmail_vcard() {
    let d = decode("gmail.vcf");
    assert_eq!(d.format, ContactFormat::VCard);
    let c = contacts(&d);
    assert_eq!(c.len(), 2);
    assert_eq!(
        *c[0],
        ContactInput {
            display_name: "Élodie Cézanne".into(),
            given_name: "Élodie".into(),
            family_name: "Cézanne".into(),
            nickname: "Lolo".into(),
            organization: "Atelier Sud".into(),
            notes: "Rencontrée à Lyon, France.\nPréfère le téléphone. Une note assez longue \
                    pour être repliée sur deux lignes par l'export."
                .into(),
            emails: vec![
                email("elodie.cezanne@example.com", "work", false),
                email("elodie@example.org", "home", false),
            ],
            phones: vec![
                phone("+33 6 12 34 56 78", "cell"),
                phone("+33 4 72 00 00 00", "work"),
            ],
            ..ContactInput::default()
        }
    );
    assert_eq!(c[1].emails.len(), 2);
}

#[test]
fn apple_vcard() {
    let d = decode("apple.vcf");
    assert_eq!(d.format, ContactFormat::VCard);
    assert_eq!(d.cards.len(), 3);
    assert_eq!(d.cards[1], ImportedCard::List);
    let c = contacts(&d);
    assert_eq!(
        *c[0],
        ContactInput {
            display_name: "Alice Martin".into(),
            given_name: "Alice".into(),
            family_name: "Martin".into(),
            nickname: "Lili".into(),
            organization: "ACME".into(),
            notes: "Collègue, bureau 4".into(),
            emails: vec![
                email("alice.martin@example.com", "", true),
                email("alice@example.org", "home", false),
            ],
            phones: vec![
                phone("+33 6 98 76 54 32", "cell"),
                phone("01 02 03 04 05", "")
            ],
            ..ContactInput::default()
        }
    );
    assert_eq!(c[1].display_name, "service@paypal.com");
}

#[test]
fn outlook_vcard_2_1_in_windows_1252_with_quoted_printable() {
    let d = decode("outlook.vcf");
    assert_eq!(d.format, ContactFormat::VCard);
    let c = contacts(&d);
    assert_eq!(
        *c[0],
        ContactInput {
            display_name: "Hélène Lefèvre".into(),
            given_name: "Hélène".into(),
            family_name: "Lefèvre".into(),
            organization: "Mairie de Saint-Étienne".into(),
            notes: "Première ligne\r\nDeuxième ligne".into(),
            emails: vec![
                email("helene.lefevre@example.fr", "", true),
                email("helene@example.org", "", false),
            ],
            phones: vec![
                phone("04 77 00 00 00", "work"),
                phone("06 11 22 33 44", "cell")
            ],
            ..ContactInput::default()
        }
    );
    assert_eq!(c[1].display_name, "Émilie Bernard");
    assert_eq!(c[1].given_name, "Émilie");
    assert_eq!(
        c[1].emails,
        [email("emilie.bernard@example.com", "home", false)]
    );
}

#[test]
fn outlook_csv() {
    let d = decode("outlook.csv");
    assert_eq!(d.format, ContactFormat::OutlookCsv);
    // La ligne vide est omise.
    assert_eq!(d.cards.len(), 2);
    let c = contacts(&d);
    assert_eq!(
        *c[0],
        ContactInput {
            given_name: "Paul".into(),
            family_name: "Petit".into(),
            organization: "Petit & Fils".into(),
            notes: "Client depuis 2019".into(),
            emails: vec![
                email("paul.petit@example.com", "", false),
                email("paul@example.org", "", false),
            ],
            phones: vec![
                phone("01 40 00 00 00", "work"),
                phone("06 22 33 44 55", "cell")
            ],
            ..ContactInput::default()
        }
    );
    assert_eq!(c[1].given_name, "Zoé");
}

#[test]
fn french_outlook_csv_with_semicolons_in_windows_1252() {
    let d = decode("outlook-fr.csv");
    assert_eq!(d.format, ContactFormat::OutlookCsv);
    let c = contacts(&d);
    assert_eq!(
        *c[0],
        ContactInput {
            given_name: "François".into(),
            family_name: "Lemaître".into(),
            organization: "Boulangerie Lemaître".into(),
            notes: "Livraison le mardi; pas le lundi".into(),
            emails: vec![email("francois.lemaitre@example.fr", "", false)],
            phones: vec![phone("06 99 88 77 66", "cell")],
            ..ContactInput::default()
        }
    );
}

#[test]
fn thunderbird_csv() {
    let d = decode("thunderbird.csv");
    assert_eq!(d.format, ContactFormat::ThunderbirdCsv);
    let c = contacts(&d);
    assert_eq!(
        *c[0],
        ContactInput {
            display_name: "Hélène Lefèvre".into(),
            given_name: "Hélène".into(),
            family_name: "Lefèvre".into(),
            nickname: "Lène".into(),
            organization: "Mairie".into(),
            notes: "Une note\r\nsur deux lignes".into(),
            emails: vec![
                email("helene.lefevre@example.fr", "", true),
                email("helene@example.org", "", false),
            ],
            phones: vec![
                phone("04 77 00 00 00", "work"),
                phone("06 11 22 33 44", "cell")
            ],
            ..ContactInput::default()
        }
    );
    assert_eq!(c[1].display_name, "Liste Amis <amis@example.com>");
}

#[test]
fn thunderbird_ldif() {
    let d = decode("thunderbird.ldif");
    assert_eq!(d.format, ContactFormat::Ldif);
    assert_eq!(d.cards.len(), 3);
    assert_eq!(d.cards[2], ImportedCard::List);
    let c = contacts(&d);
    assert_eq!(
        *c[0],
        ContactInput {
            display_name: "Hélène Lefèvre".into(),
            given_name: "Hélène".into(),
            family_name: "Lefèvre".into(),
            nickname: "Lène".into(),
            organization: "Mairie".into(),
            notes: "Une note\nsur deux lignes".into(),
            emails: vec![
                email("helene.lefevre@example.fr", "", true),
                email("helene@example.org", "", false),
            ],
            phones: vec![
                phone("04 77 00 00 00", "work"),
                phone("06 11 22 33 44", "cell")
            ],
            ..ContactInput::default()
        }
    );
    assert_eq!(c[1].notes, "Une description repliée sur deux lignes");
}

#[test]
fn every_fixture_plans_without_surprise() {
    // Bout à bout : chaque export donne des fiches que le domaine accepte,
    // sauf celles que le jeu d'essai rend invalides exprès.
    for (name, created, rejected) in [
        ("gmail.csv", 3, 1),
        ("gmail.vcf", 2, 0),
        ("apple.vcf", 2, 1),
        ("outlook.vcf", 2, 0),
        ("outlook.csv", 2, 0),
        ("outlook-fr.csv", 1, 0),
        ("thunderbird.csv", 2, 0),
        ("thunderbird.ldif", 2, 1),
    ] {
        let d = decode(name);
        let plan = plan_import(&[], &d.cards, DuplicatePolicy::Skip, &IcuNfc);
        assert_eq!(plan.create.len(), created, "{name} : {:?}", plan.rejected);
        assert_eq!(
            plan.rejected.len(),
            rejected,
            "{name} : {:?}",
            plan.rejected
        );
    }
    let plan = plan_import(
        &[],
        &decode("gmail.csv").cards,
        DuplicatePolicy::Skip,
        &IcuNfc,
    );
    assert_eq!(
        plan.rejected[0].reason,
        RejectReason::Invalid(ContactError::NoAddress)
    );
    // Nom imitant une adresse retiré (Apple, Thunderbird).
    let plan = plan_import(
        &[],
        &decode("apple.vcf").cards,
        DuplicatePolicy::Skip,
        &IcuNfc,
    );
    assert_eq!(plan.renamed, 1);
    assert_eq!(plan.rejected[0].reason, RejectReason::List);
}
