//! Commandes IPC des doublons du carnet (ADR-012) : chercher les groupes
//! de doublons, fusionner des fiches.
//!
//! La fusion est recalculée en Rust d'après les fiches enregistrées :
//! l'interface ne désigne que la fiche gardée et les fiches absorbées
//! (identifiants revalidés par `MergeSelection::new`). Les refus deviennent
//! des codes ; aucun texte destiné à l'utilisateur.

use norelly_application::contact_duplicates::{
    DuplicateCandidates, DuplicateScanFailure, FindDuplicates, MergeContacts,
};
use norelly_application::contacts::ContactFailure;
use norelly_domain::contact_duplicates::{DuplicateReason, MergeSelection};
use norelly_domain::contacts::{ContactError, ContactField};
use tauri::State;

use crate::contacts::{contact_dto, contact_error};
use crate::ipc::{
    ContactDto, ContactDuplicateGroupDto, DuplicateReasonDto, ErrorCodeDto, FieldDto, IpcError,
};
use crate::state::AppState;

/// Échec d'une recherche de doublons, en code.
pub(crate) fn scan_error(e: DuplicateScanFailure) -> IpcError {
    match e {
        DuplicateScanFailure::TooMany => IpcError::new(ErrorCodeDto::ContactDuplicatesTooMany),
        DuplicateScanFailure::Store(e) => IpcError::storage(e),
    }
}

/// Échec d'une fusion, en code.
pub(crate) fn merge_error(e: ContactFailure) -> IpcError {
    match e {
        ContactFailure::Invalid(
            ContactError::TooManyAddresses
            | ContactError::TooManyPhones
            | ContactError::TooLong(ContactField::Notes),
        ) => IpcError::new(ErrorCodeDto::ContactMergeTooLarge),
        e => contact_error(e, &[]),
    }
}

fn group_dto(group: &DuplicateCandidates) -> Result<ContactDuplicateGroupDto, IpcError> {
    Ok(ContactDuplicateGroupDto {
        contacts: group
            .contacts
            .iter()
            .map(contact_dto)
            .collect::<Result<_, _>>()?,
        reasons: group
            .reasons
            .iter()
            .map(|r| match r {
                DuplicateReason::SameName => DuplicateReasonDto::SameName,
                DuplicateReason::SamePhone => DuplicateReasonDto::SamePhone,
            })
            .collect(),
    })
}

/// Groupes de doublons du carnet, dans l'ordre du nom de leur première
/// fiche.
#[tauri::command]
#[specta::specta]
pub async fn find_contact_duplicates(
    state: State<'_, AppState>,
) -> Result<Vec<ContactDuplicateGroupDto>, IpcError> {
    let groups = FindDuplicates {
        directory: &state.contacts,
        normalizer: &state.imports.nfc,
    }
    .execute()
    .await
    .map_err(scan_error)?;
    groups.iter().map(group_dto).collect()
}

/// Fusionne les fiches `others` dans la fiche `keep` ; renvoie la fiche
/// gardée.
#[tauri::command]
#[specta::specta]
pub async fn merge_contacts(
    state: State<'_, AppState>,
    keep: u32,
    others: Vec<u32>,
) -> Result<ContactDto, IpcError> {
    let selection =
        MergeSelection::new(keep, &others).map_err(|_| IpcError::invalid(FieldDto::Contact))?;
    let kept = MergeContacts {
        book: &state.contacts,
    }
    .execute(&selection)
    .await
    .map_err(merge_error)?;
    contact_dto(&kept)
}

#[cfg(test)]
mod tests;
