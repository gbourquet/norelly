//! Non-régression de l'export : le fichier écrit par pages
//! ([`ContactCodec::encoder`]) est, octet pour octet, celui des jeux
//! d'essai `tests/data/export`, produits par l'écriture d'un seul tenant
//! ([`ContactCodec::encode`]) avant le passage aux pages.

use norelly_contacts_io::ContactsIo;
use norelly_domain::contact_export::{ExportCard, ExportFormat, ExportList, export_card};
use norelly_domain::ports::ContactCodec;
use norelly_test_support::contact_import_contracts::{
    encode_by_pages, sample_export_drafts, sample_export_lists, sample_export_photo,
};

const FORMATS: [(ExportFormat, &str); 4] = [
    (ExportFormat::VCard4, "carnet-4.vcf"),
    (ExportFormat::VCard3, "carnet-3.vcf"),
    (ExportFormat::GoogleCsv, "carnet.csv"),
    (ExportFormat::Ldif, "carnet.ldif"),
];

fn golden(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/data/export/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path} : {e}"))
}

/// Fiches de l'essai : les fiches d'exemple deux fois, photo sur la
/// première et la dernière ; listes d'exemple.
fn sample(format: ExportFormat) -> (Vec<ExportCard>, Vec<ExportList>) {
    let drafts = sample_export_drafts();
    let mut cards: Vec<ExportCard> = drafts
        .iter()
        .chain(&drafts)
        .map(|d| export_card(d, format))
        .collect();
    if format.keeps_photos() {
        cards[0].photo = Some(sample_export_photo());
        if let Some(last) = cards.last_mut() {
            last.photo = Some(sample_export_photo());
        }
    }
    let lists = sample_export_lists(cards.len());
    (cards, lists)
}

#[test]
fn whole_file_matches_the_golden_files() {
    for (format, name) in FORMATS {
        let (cards, lists) = sample(format);
        let bytes = ContactsIo
            .encode(&cards, &lists, format)
            .unwrap_or_else(|e| panic!("{format:?} : {e}"));
        assert!(bytes == golden(name), "{format:?} : fichier différent");
    }
}

#[test]
fn a_file_written_by_pages_matches_the_golden_files_byte_for_byte() {
    for (format, name) in FORMATS {
        let (cards, lists) = sample(format);
        for page in 1..=cards.len() {
            let bytes = encode_by_pages(&ContactsIo, &cards, &lists, format, page)
                .unwrap_or_else(|e| panic!("{format:?}, pages de {page} : {e}"));
            assert!(
                bytes == golden(name),
                "{format:?}, pages de {page} : fichier différent"
            );
        }
    }
}

#[test]
fn an_empty_file_written_by_pages_is_the_one_written_at_once() {
    for (format, _) in FORMATS {
        assert_eq!(
            encode_by_pages(&ContactsIo, &[], &[], format, 1),
            ContactsIo.encode(&[], &[], format),
            "{format:?}"
        );
    }
}
