//! Export d'un carnet : les fiches validées d'un fichier quelconque
//! (lecture puis plan d'import) s'écrivent dans chaque format sans
//! panique, dans les bornes, et se relisent à l'identique.
#![no_main]

use libfuzzer_sys::fuzz_target;
use norelly_contacts_io::{ContactsIo, IcuNfc};
use norelly_domain::contact_export::{
    ExportCard, ExportFormat, MAX_EXPORT_FILE_BYTES, export_card,
};
use norelly_domain::contact_import::{DuplicatePolicy, ImportedCard, plan_import};
use norelly_domain::contacts::{ContactInput, EmailInput, PhoneInput};
use norelly_domain::ports::ContactCodec;

const FORMATS: [ExportFormat; 4] = [
    ExportFormat::VCard4,
    ExportFormat::VCard3,
    ExportFormat::GoogleCsv,
    ExportFormat::Ldif,
];

/// Fiche que la relecture de l'export de `card` doit rendre.
fn imported(card: &ExportCard) -> ImportedCard {
    let owned = |v: &Option<String>| v.clone().unwrap_or_default();
    ImportedCard::Contact(ContactInput {
        display_name: owned(&card.full_name),
        given_name: owned(&card.given_name),
        family_name: owned(&card.family_name),
        nickname: owned(&card.nickname),
        organization: owned(&card.organization),
        notes: owned(&card.notes),
        emails: card
            .emails
            .iter()
            .map(|e| EmailInput {
                address: e.address.clone(),
                label: owned(&e.label),
                preferred: e.preferred,
            })
            .collect(),
        phones: card
            .phones
            .iter()
            .map(|p| PhoneInput {
                number: p.number.clone(),
                label: owned(&p.label),
            })
            .collect(),
        favorite: false,
    })
}

fuzz_target!(|bytes: &[u8]| {
    let Ok(decoded) = ContactsIo.decode(bytes) else {
        return;
    };
    let plan = plan_import(&[], &decoded.cards, DuplicatePolicy::Skip, &IcuNfc);
    if plan.create.is_empty() {
        return;
    }
    for format in FORMATS {
        let cards: Vec<ExportCard> = plan.create.iter().map(|d| export_card(d, format)).collect();
        let Ok(written) = ContactsIo.encode(&cards, format) else {
            continue;
        };
        assert!(written.len() <= MAX_EXPORT_FILE_BYTES);
        let read = ContactsIo
            .decode(&written)
            .unwrap_or_else(|e| panic!("{format:?} : export illisible : {e}"));
        let expected: Vec<ImportedCard> = cards.iter().map(imported).collect();
        assert_eq!(read.cards, expected, "{format:?}");
    }
});
