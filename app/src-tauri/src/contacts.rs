//! Commandes IPC du carnet d'adresses (ADR-012) : lister, lire, créer,
//! modifier et supprimer des fiches.
//!
//! La saisie reçue est revalidée par le domaine (`ContactDraft::new`, dans
//! les cas d'usage) ; les identifiants, la sélection et le curseur aussi
//! (`ContactId::new`, `contact_selection`, `ContactCursor::parse`). Les
//! refus deviennent des codes ; aucun texte destiné à l'utilisateur.

use norelly_application::contacts::{
    ContactFailure, CreateContact, DeleteContacts, GetContact, ListContacts, UpdateContact,
};
use norelly_domain::contacts::{
    Contact, ContactError, ContactField, ContactId, ContactInput, EmailInput, MAX_CONTACT_EMAILS,
    PhoneInput, contact_selection,
};
use norelly_domain::ports::{ContactCursor, ContactListItem, ContactStoreError};
use tauri::State;

use crate::ipc::{
    ContactCursorDto, ContactDto, ContactEmailDto, ContactFieldsDto, ContactItemDto,
    ContactPageDto, ContactPhoneDto, ErrorCodeDto, FieldDto, IpcError, id32,
};
use crate::state::AppState;

fn field_dto(field: ContactField) -> FieldDto {
    match field {
        ContactField::DisplayName => FieldDto::DisplayName,
        ContactField::GivenName => FieldDto::GivenName,
        ContactField::FamilyName => FieldDto::FamilyName,
        ContactField::Nickname => FieldDto::Nickname,
        ContactField::Organization => FieldDto::Organization,
        ContactField::Notes => FieldDto::Notes,
        ContactField::EmailLabel(_) => FieldDto::ContactEmail,
        ContactField::Phone(_) | ContactField::PhoneLabel(_) => FieldDto::ContactPhone,
    }
}

/// Échec d'un cas d'usage du carnet, en code ; une adresse refusée est
/// citée d'après la saisie `typed` (adresses dans l'ordre reçu).
fn contact_error(e: ContactFailure, typed: &[String]) -> IpcError {
    let quoted = |code: ErrorCodeDto, index: usize| {
        let e = IpcError::new(code);
        match typed.get(index) {
            Some(address) => e.with_detail(address),
            None => e,
        }
    };
    match e {
        ContactFailure::Invalid(e) => match e {
            ContactError::NoAddress => IpcError::new(ErrorCodeDto::ContactNoAddress),
            ContactError::InvalidAddress { index, .. } => IpcError {
                field: Some(FieldDto::ContactEmail),
                ..quoted(ErrorCodeDto::ContactInvalid, index)
            },
            ContactError::DuplicateAddress { index } => {
                quoted(ErrorCodeDto::ContactRepeatedAddress, index)
            }
            ContactError::TooManyAddresses | ContactError::SeveralPreferred => {
                invalid_field(ErrorCodeDto::ContactInvalid, FieldDto::ContactEmail)
            }
            ContactError::TooManyPhones | ContactError::EmptyPhone { .. } => {
                invalid_field(ErrorCodeDto::ContactInvalid, FieldDto::ContactPhone)
            }
            ContactError::TooLong(field) => {
                invalid_field(ErrorCodeDto::ContactTooLong, field_dto(field))
            }
            ContactError::InvalidCharacter(field) => {
                invalid_field(ErrorCodeDto::ContactInvalid, field_dto(field))
            }
        },
        ContactFailure::Store(e) => match e {
            ContactStoreError::DuplicateAddress(address) => {
                IpcError::new(ErrorCodeDto::ContactDuplicateAddress).with_detail(address)
            }
            ContactStoreError::NotFound => IpcError::new(ErrorCodeDto::ContactNotFound),
            ContactStoreError::Corrupted(e) => IpcError::storage(e),
            ContactStoreError::Store(e) => IpcError::storage(e),
        },
    }
}

fn invalid_field(code: ErrorCodeDto, field: FieldDto) -> IpcError {
    IpcError {
        field: Some(field),
        ..IpcError::new(code)
    }
}

fn contact_id(raw: u32) -> Result<ContactId, IpcError> {
    ContactId::new(i64::from(raw)).ok_or_else(|| IpcError::invalid(FieldDto::Contact))
}

/// Saisie de l'interface, à valider par le domaine.
fn input_of(dto: ContactFieldsDto) -> ContactInput {
    ContactInput {
        display_name: dto.display_name,
        given_name: dto.given_name,
        family_name: dto.family_name,
        nickname: dto.nickname,
        organization: dto.organization,
        notes: dto.notes,
        emails: dto
            .emails
            .into_iter()
            .map(|e| EmailInput {
                address: e.address,
                label: e.label,
                preferred: e.preferred,
            })
            .collect(),
        phones: dto
            .phones
            .into_iter()
            .map(|p| PhoneInput {
                number: p.number,
                label: p.label,
            })
            .collect(),
        favorite: dto.favorite,
    }
}

fn contact_dto(contact: &Contact) -> Result<ContactDto, IpcError> {
    let draft = contact.draft();
    let input = draft.to_input();
    Ok(ContactDto {
        id: id32(contact.id().get())?,
        name: draft.display_name(),
        fields: ContactFieldsDto {
            display_name: input.display_name,
            given_name: input.given_name,
            family_name: input.family_name,
            nickname: input.nickname,
            organization: input.organization,
            notes: input.notes,
            emails: input
                .emails
                .into_iter()
                .map(|e| ContactEmailDto {
                    address: e.address,
                    label: e.label,
                    preferred: e.preferred,
                })
                .collect(),
            phones: input
                .phones
                .into_iter()
                .map(|p| ContactPhoneDto {
                    number: p.number,
                    label: p.label,
                })
                .collect(),
            favorite: input.favorite,
        },
    })
}

pub(crate) fn item_dto(item: ContactListItem) -> Result<ContactItemDto, IpcError> {
    Ok(ContactItemDto {
        id: id32(item.id.get())?,
        name: item.name,
        address: item.address.to_string(),
        organization: item.organization,
        favorite: item.favorite,
    })
}

/// Adresses saisies, pour citer celle qui est refusée : les seules que le
/// domaine peut désigner (une fiche plus longue est refusée en bloc).
fn typed_addresses(dto: &ContactFieldsDto) -> Vec<String> {
    dto.emails
        .iter()
        .take(MAX_CONTACT_EMAILS)
        .map(|e| e.address.clone())
        .collect()
}

/// Une page de contacts filtrée sur `text` (vide : tous) et, au besoin,
/// les seuls favoris, après `cursor`.
#[tauri::command]
#[specta::specta]
pub async fn list_contacts(
    state: State<'_, AppState>,
    text: String,
    favorites_only: bool,
    cursor: Option<ContactCursorDto>,
    limit: u32,
) -> Result<ContactPageDto, IpcError> {
    let cursor = cursor
        .map(|c| {
            ContactCursor::parse(c.sort_key, i64::from(c.id))
                .ok_or_else(|| IpcError::invalid(FieldDto::Cursor))
        })
        .transpose()?;
    let page = ListContacts {
        directory: &state.contacts,
    }
    .execute(&text, favorites_only, cursor, limit)
    .await
    .map_err(|e| contact_error(e, &[]))?;
    Ok(ContactPageDto {
        items: page
            .items
            .into_iter()
            .map(item_dto)
            .collect::<Result<_, _>>()?,
        next: page
            .next
            .map(|c| {
                Ok::<_, IpcError>(ContactCursorDto {
                    sort_key: c.sort_key,
                    id: id32(c.id.get())?,
                })
            })
            .transpose()?,
    })
}

/// Fiche complète `id`.
#[tauri::command]
#[specta::specta]
pub async fn get_contact(state: State<'_, AppState>, id: u32) -> Result<ContactDto, IpcError> {
    let id = contact_id(id)?;
    let contact = GetContact {
        directory: &state.contacts,
    }
    .execute(id)
    .await
    .map_err(|e| contact_error(e, &[]))?;
    contact_dto(&contact)
}

/// Enregistre une nouvelle fiche.
#[tauri::command]
#[specta::specta]
pub async fn create_contact(
    state: State<'_, AppState>,
    contact: ContactFieldsDto,
) -> Result<ContactDto, IpcError> {
    let typed = typed_addresses(&contact);
    let created = CreateContact {
        book: &state.contacts,
    }
    .execute(input_of(contact))
    .await
    .map_err(|e| contact_error(e, &typed))?;
    contact_dto(&created)
}

/// Remplace le contenu de la fiche `id`.
#[tauri::command]
#[specta::specta]
pub async fn update_contact(
    state: State<'_, AppState>,
    id: u32,
    contact: ContactFieldsDto,
) -> Result<ContactDto, IpcError> {
    let id = contact_id(id)?;
    let typed = typed_addresses(&contact);
    let updated = UpdateContact {
        book: &state.contacts,
    }
    .execute(id, input_of(contact))
    .await
    .map_err(|e| contact_error(e, &typed))?;
    contact_dto(&updated)
}

/// Supprime des fiches (chaque identifiant une fois) ; renvoie le nombre
/// de fiches supprimées.
#[tauri::command]
#[specta::specta]
pub async fn delete_contacts(state: State<'_, AppState>, ids: Vec<u32>) -> Result<u32, IpcError> {
    let ids = contact_selection(&ids).map_err(|_| IpcError::invalid(FieldDto::Contact))?;
    let deleted = DeleteContacts {
        book: &state.contacts,
    }
    .execute(&ids)
    .await
    .map_err(|e| contact_error(e, &[]))?;
    u32::try_from(deleted).map_err(|_| IpcError::internal("nombre de contacts supprimés"))
}

#[cfg(test)]
mod tests;
