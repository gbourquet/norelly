use norelly_domain::contacts::ContactId;
use norelly_domain::ports::StoreError;

use super::*;

fn code(e: impl Into<ContactListFailure>) -> (ErrorCodeDto, Option<FieldDto>, Option<String>) {
    let e = list_error(e.into());
    (e.code, e.field, e.detail)
}

#[test]
fn refused_lists_become_codes_with_their_field() {
    assert_eq!(
        code(ContactListError::NoName),
        (
            ErrorCodeDto::ContactListNoName,
            Some(FieldDto::ListName),
            None
        )
    );
    for (field, expected) in [
        (ContactListField::Name, FieldDto::ListName),
        (ContactListField::Description, FieldDto::ListDescription),
    ] {
        assert_eq!(
            code(ContactListError::TooLong(field)),
            (ErrorCodeDto::ContactListTooLong, Some(expected), None)
        );
        assert_eq!(
            code(ContactListError::InvalidCharacter(field)),
            (ErrorCodeDto::ContactListInvalid, Some(expected), None)
        );
    }
    assert_eq!(
        code(ContactListError::InvalidMember),
        (ErrorCodeDto::InvalidInput, Some(FieldDto::Contact), None)
    );
    assert_eq!(
        code(ContactListError::TooManyMembers),
        (ErrorCodeDto::ContactListTooManyMembers, None, None)
    );
    assert_eq!(
        code(ContactListError::Empty),
        (ErrorCodeDto::ContactListEmpty, None, None)
    );
}

#[test]
fn store_refusals_become_codes() {
    assert_eq!(
        code(ContactListStoreError::DuplicateName("Famille".into())),
        (
            ErrorCodeDto::ContactListDuplicateName,
            Some(FieldDto::ListName),
            Some("Famille".into())
        )
    );
    let ghost = ContactId::new(4).unwrap_or_else(|| panic!("id"));
    assert_eq!(
        code(ContactListStoreError::UnknownContact(ghost)),
        (ErrorCodeDto::ContactNotFound, None, None)
    );
    assert_eq!(
        code(ContactListStoreError::NotFound),
        (ErrorCodeDto::ContactListNotFound, None, None)
    );
    // Détail du stockage jamais transmis.
    assert_eq!(
        code(ContactListStoreError::Store(StoreError(
            "disque plein".into()
        ))),
        (ErrorCodeDto::Storage, None, None)
    );
}

#[test]
fn list_ids_are_validated() {
    assert_eq!(
        list_id(0).map_err(|e| (e.code, e.field)),
        Err((ErrorCodeDto::InvalidInput, Some(FieldDto::ContactList)))
    );
    assert_eq!(list_id(3).map(ContactListId::get).ok(), Some(3));
}
