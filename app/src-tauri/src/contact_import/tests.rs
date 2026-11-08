use std::io::Write;

use norelly_application::contact_import::ImportPreview;
use norelly_domain::contact_import::{
    ContactFormat, ImportPlan, ImportRejection, MAX_IMPORT_FILE_BYTES, RejectReason,
};
use norelly_domain::contacts::{ContactError, ContactField, ContactId};
use norelly_domain::ports::{ContactCodecError, ContactStoreError, StoreError};
use norelly_test_support::contacts_contracts::sample_contact;

use super::*;
use crate::ipc::{ContactImportFormatDto, ErrorCodeDto, ImportCountsDto, ImportRejectReasonDto};

fn rejection(card: usize, reason: RejectReason) -> ImportRejection {
    ImportRejection {
        card,
        hint: format!("fiche {card}"),
        reason,
    }
}

fn preview(rejected: Vec<ImportRejection>) -> ImportPreview {
    let alice = sample_contact("Alice", &["alice@example.com"]);
    let id = ContactId::new(3).unwrap_or_else(|| panic!("id"));
    ImportPreview {
        format: ContactFormat::GoogleCsv,
        cards: Vec::new(),
        skip: ImportPlan {
            create: vec![alice.clone(), alice.clone()],
            duplicates: 4,
            renamed: 1,
            rejected: rejected.clone(),
            ..ImportPlan::default()
        },
        complete: ImportPlan {
            create: vec![alice.clone()],
            complete: vec![alice.with_id(id)],
            duplicates: 2,
            renamed: 1,
            rejected,
        },
    }
}

#[test]
fn preview_counts_both_policies_and_lists_reasons() -> Result<(), IpcError> {
    let invalid = |e| RejectReason::Invalid(e);
    let reasons = [
        (RejectReason::Unreadable, ImportRejectReasonDto::Unreadable),
        (RejectReason::List, ImportRejectReasonDto::List),
        (
            invalid(ContactError::NoAddress),
            ImportRejectReasonDto::NoAddress,
        ),
        (
            invalid(ContactError::DuplicateAddress { index: 1 }),
            ImportRejectReasonDto::InvalidAddress,
        ),
        (
            invalid(ContactError::SeveralPreferred),
            ImportRejectReasonDto::InvalidAddress,
        ),
        (
            invalid(ContactError::TooLong(ContactField::Notes)),
            ImportRejectReasonDto::TooLong,
        ),
        (
            invalid(ContactError::InvalidCharacter(ContactField::Nickname)),
            ImportRejectReasonDto::InvalidField,
        ),
        (
            invalid(ContactError::EmptyPhone { index: 0 }),
            ImportRejectReasonDto::InvalidField,
        ),
        (
            invalid(ContactError::TooManyAddresses),
            ImportRejectReasonDto::TooManyValues,
        ),
        (
            invalid(ContactError::TooManyPhones),
            ImportRejectReasonDto::TooManyValues,
        ),
    ];
    let rejected = reasons
        .iter()
        .enumerate()
        .map(|(i, (r, _))| rejection(i + 1, r.clone()))
        .collect();
    let dto = preview_dto(7, "contacts.csv", &preview(rejected))?;
    assert_eq!(dto.token, 7);
    assert_eq!(dto.file_name, "contacts.csv");
    assert_eq!(dto.format, ContactImportFormatDto::GoogleCsv);
    assert_eq!(
        dto.skip,
        ImportCountsDto {
            create: 2,
            complete: 0,
            duplicates: 4
        }
    );
    assert_eq!(
        dto.complete,
        ImportCountsDto {
            create: 1,
            complete: 1,
            duplicates: 2
        }
    );
    assert_eq!(dto.renamed, 1);
    assert_eq!(dto.rejected_total, 10);
    let got: Vec<_> = dto.rejected.iter().map(|r| (r.card, r.reason)).collect();
    let expected: Vec<_> = (1..).zip(reasons.iter().map(|(_, d)| *d)).collect();
    assert_eq!(got, expected);
    assert_eq!(dto.rejected[0].hint, "fiche 1");
    Ok(())
}

#[test]
fn preview_lists_only_the_first_rejections() -> Result<(), IpcError> {
    let rejected = (1..=MAX_LISTED_REJECTIONS + 5)
        .map(|i| rejection(i, RejectReason::Unreadable))
        .collect();
    let dto = preview_dto(1, "x.vcf", &preview(rejected))?;
    assert_eq!(dto.rejected.len(), MAX_LISTED_REJECTIONS);
    assert_eq!(dto.rejected_total as usize, MAX_LISTED_REJECTIONS + 5);
    Ok(())
}

#[test]
fn file_name_is_cleaned() -> Result<(), IpcError> {
    let dto = preview_dto(1, "a\u{202E}b\u{7}.vcf", &preview(Vec::new()))?;
    assert!(!dto.file_name.contains('\u{202E}') && !dto.file_name.contains('\u{7}'));
    Ok(())
}

#[test]
fn failures_become_codes() {
    let code = |e: ImportFailure| import_error(e).code;
    assert_eq!(
        code(ContactCodecError::TooLarge.into()),
        ErrorCodeDto::ContactImportTooLarge
    );
    assert_eq!(
        code(ContactCodecError::TooManyCards.into()),
        ErrorCodeDto::ContactImportTooManyCards
    );
    assert_eq!(
        code(ContactCodecError::UnknownFormat.into()),
        ErrorCodeDto::ContactImportUnknownFormat
    );
    let address = norelly_domain::address::EmailAddress::parse("a@example.com")
        .unwrap_or_else(|e| panic!("{e}"));
    let e = import_error(ContactStoreError::DuplicateAddress(address).into());
    assert_eq!(e.code, ErrorCodeDto::ContactDuplicateAddress);
    assert_eq!(e.detail.as_deref(), Some("a@example.com"));
    assert_eq!(
        code(ContactStoreError::NotFound.into()),
        ErrorCodeDto::ContactNotFound
    );
    assert_eq!(
        code(StoreError("disque plein".into()).into()),
        ErrorCodeDto::Storage
    );
}

#[test]
fn pending_import_is_taken_once_with_its_token() {
    let pending = PendingImport::default();
    let first = pending.remember(vec![ImportedCard::List]);
    let second = pending.remember(vec![ImportedCard::Unreadable]);
    assert_ne!(first, second);
    // L'aperçu remplacé est périmé.
    assert_eq!(pending.take(first), None);
    assert_eq!(pending.take(second), Some(vec![ImportedCard::Unreadable]));
    assert_eq!(pending.take(second), None);
    // Rendu après un échec : de nouveau disponible, sauf si un autre
    // aperçu l'a remplacé.
    pending.restore(second, vec![ImportedCard::List]);
    assert_eq!(pending.take(second), Some(vec![ImportedCard::List]));
    let third = pending.remember(vec![ImportedCard::Unreadable]);
    pending.restore(second, vec![ImportedCard::List]);
    assert_eq!(pending.take(second), None);
    assert_eq!(pending.take(third), Some(vec![ImportedCard::Unreadable]));
    assert_ne!(third, 0);
}

#[test]
fn contact_file_is_read_within_bounds() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("norelly-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let small = dir.join("petit.vcf");
    std::fs::write(&small, b"BEGIN:VCARD")?;
    assert_eq!(
        read_contact_file(&small).map_err(|e| e.to_string())?,
        b"BEGIN:VCARD"
    );

    let big = dir.join("gros.vcf");
    let mut file = std::fs::File::create(&big)?;
    file.write_all(&vec![b'a'; MAX_IMPORT_FILE_BYTES + 1])?;
    drop(file);
    assert_eq!(
        read_contact_file(&big).map(|_| ()).map_err(|e| e.code),
        Err(ErrorCodeDto::ContactImportTooLarge)
    );
    let missing = read_contact_file(&dir.join("absent.vcf")).map(|_| ());
    assert_eq!(
        missing.map_err(|e| e.code),
        Err(ErrorCodeDto::ContactImportReadFailed)
    );
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}
