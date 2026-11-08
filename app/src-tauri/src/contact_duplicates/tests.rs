use norelly_domain::contacts::ContactError;
use norelly_domain::ports::{ContactStoreError, StoreError};

use super::*;

#[test]
fn a_merge_beyond_the_bounds_of_a_card_has_its_own_code() {
    for error in [
        ContactError::TooManyAddresses,
        ContactError::TooManyPhones,
        ContactError::TooLong(ContactField::Notes),
    ] {
        let e = merge_error(ContactFailure::Invalid(error.clone()));
        assert_eq!(
            (e.code, e.field),
            (ErrorCodeDto::ContactMergeTooLarge, None),
            "{error:?}"
        );
    }
    // Les autres refus gardent les codes du carnet.
    let e = merge_error(ContactFailure::Store(ContactStoreError::NotFound));
    assert_eq!(e.code, ErrorCodeDto::ContactNotFound);
}

#[test]
fn a_book_too_large_to_scan_has_its_own_code() {
    assert_eq!(
        scan_error(DuplicateScanFailure::TooMany).code,
        ErrorCodeDto::ContactDuplicatesTooMany
    );
    assert_eq!(
        scan_error(DuplicateScanFailure::Store(StoreError("x".into()))).code,
        ErrorCodeDto::Storage
    );
}

#[test]
fn a_card_changed_since_the_search_has_its_own_code() {
    let e = merge_error(ContactFailure::Store(ContactStoreError::Conflict));
    assert_eq!(e.code, ErrorCodeDto::ContactMergeConflict);
}
