use norelly_domain::address::EmailAddress;
use norelly_domain::contacts::{ContactError, ContactField};
use norelly_domain::ports::{ContactStoreError, StoreError};

use super::*;
use crate::ipc::{ErrorCodeDto, FieldDto};

fn code(e: impl Into<ContactFailure>) -> (ErrorCodeDto, Option<FieldDto>, Option<String>) {
    let e = contact_error(e.into(), &[]);
    (e.code, e.field, e.detail)
}

fn address(raw: &str) -> EmailAddress {
    EmailAddress::parse(raw).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn refused_cards_become_codes_with_their_field() {
    assert_eq!(
        code(ContactError::NoAddress),
        (ErrorCodeDto::ContactNoAddress, None, None)
    );
    for (field, expected) in [
        (ContactField::DisplayName, FieldDto::DisplayName),
        (ContactField::GivenName, FieldDto::GivenName),
        (ContactField::FamilyName, FieldDto::FamilyName),
        (ContactField::Nickname, FieldDto::Nickname),
        (ContactField::Organization, FieldDto::Organization),
        (ContactField::Notes, FieldDto::Notes),
        (ContactField::EmailLabel(2), FieldDto::ContactEmail),
        (ContactField::Phone(0), FieldDto::ContactPhone),
        (ContactField::PhoneLabel(1), FieldDto::ContactPhone),
    ] {
        assert_eq!(
            code(ContactError::TooLong(field)),
            (ErrorCodeDto::ContactTooLong, Some(expected), None),
            "{field:?}"
        );
        assert_eq!(
            code(ContactError::InvalidCharacter(field)),
            (ErrorCodeDto::ContactInvalid, Some(expected), None),
            "{field:?}"
        );
    }
    for (error, field) in [
        (ContactError::TooManyAddresses, FieldDto::ContactEmail),
        (ContactError::SeveralPreferred, FieldDto::ContactEmail),
        (ContactError::TooManyPhones, FieldDto::ContactPhone),
        (
            ContactError::EmptyPhone { index: 0 },
            FieldDto::ContactPhone,
        ),
    ] {
        assert_eq!(
            code(error.clone()),
            (ErrorCodeDto::ContactInvalid, Some(field), None),
            "{error:?}"
        );
    }
}

#[test]
fn store_refusals_become_codes() {
    assert_eq!(
        code(ContactStoreError::DuplicateAddress(address(
            "Bob@Example.com"
        ))),
        (
            ErrorCodeDto::ContactDuplicateAddress,
            None,
            Some("Bob@example.com".to_owned())
        )
    );
    assert_eq!(
        code(ContactStoreError::NotFound),
        (ErrorCodeDto::ContactNotFound, None, None)
    );
    assert_eq!(
        code(ContactStoreError::Corrupted("x".into())).0,
        ErrorCodeDto::Storage
    );
    assert_eq!(
        code(ContactStoreError::Store(StoreError("x".into()))),
        (ErrorCodeDto::Storage, None, None)
    );
}

#[test]
fn refused_addresses_are_quoted_from_the_input() {
    let typed = ["a@example.com".to_owned(), "pas une adresse".to_owned()];
    let invalid = ContactDraftCheck::invalid_address(1);
    let e = contact_error(invalid.into(), &typed);
    assert_eq!(
        (e.code, e.field, e.detail.as_deref()),
        (
            ErrorCodeDto::ContactInvalid,
            Some(FieldDto::ContactEmail),
            Some("pas une adresse")
        )
    );
    let e = contact_error(ContactError::DuplicateAddress { index: 0 }.into(), &typed);
    assert_eq!(
        (e.code, e.detail.as_deref()),
        (ErrorCodeDto::ContactRepeatedAddress, Some("a@example.com"))
    );
    // Position hors de la saisie : pas de détail.
    let e = contact_error(ContactError::DuplicateAddress { index: 9 }.into(), &typed);
    assert_eq!(
        (e.code, e.detail),
        (ErrorCodeDto::ContactRepeatedAddress, None)
    );
}

/// Erreur d'adresse invalide telle que la produit le domaine.
struct ContactDraftCheck;

impl ContactDraftCheck {
    fn invalid_address(index: usize) -> ContactError {
        let error = EmailAddress::parse("pas une adresse")
            .err()
            .unwrap_or_else(|| panic!("adresse refusée attendue"));
        ContactError::InvalidAddress { index, error }
    }
}
