//! Commandes IPC des doublons du carnet (ADR-012) : chercher les groupes
//! de doublons, prévoir et faire la fusion de fiches.
//!
//! La fusion est recalculée en Rust d'après les fiches enregistrées :
//! l'interface ne désigne que la fiche gardée et les fiches absorbées
//! (identifiants revalidés par `MergeSelection::new`). Les refus deviennent
//! des codes ; aucun texte destiné à l'utilisateur.

use norelly_application::contact_duplicates::{
    DuplicateCandidates, DuplicateScanFailure, FindDuplicates, MergeContacts, PreviewMerge,
};
use norelly_application::contacts::ContactFailure;
use norelly_domain::contact_duplicates::{DuplicateReason, MergeSelection};
use norelly_domain::contacts::{Contact, ContactError, ContactField};
use tauri::State;

use crate::contacts::{contact_dto, contact_error};
use crate::ipc::{
    ContactDto, ContactDuplicateGroupDto, ContactEmailDto, ContactMergePreviewDto,
    DuplicateCardDto, DuplicateReasonDto, ErrorCodeDto, FieldDto, IpcError, id32,
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

/// Fiche d'un groupe, réduite à ce que la fenêtre affiche (les notes, qui
/// peuvent être longues, ne quittent pas Rust).
fn card_dto(contact: &Contact) -> Result<DuplicateCardDto, IpcError> {
    let draft = contact.draft();
    Ok(DuplicateCardDto {
        id: id32(contact.id().get())?,
        name: draft.display_name(),
        emails: draft
            .emails()
            .iter()
            .map(|e| ContactEmailDto {
                address: e.address().to_string(),
                label: e.label().unwrap_or_default().to_owned(),
                preferred: e.preferred(),
            })
            .collect(),
        phones: draft
            .phones()
            .iter()
            .map(|p| p.number().to_owned())
            .collect(),
        organization: draft.organization().unwrap_or_default().to_owned(),
    })
}

fn group_dto(group: &DuplicateCandidates) -> Result<ContactDuplicateGroupDto, IpcError> {
    Ok(ContactDuplicateGroupDto {
        contacts: group
            .contacts
            .iter()
            .map(card_dto)
            .collect::<Result<_, _>>()?,
        truncated: group.truncated,
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

/// Ce que garderait la fiche `keep` si les fiches `others` y étaient
/// fusionnées (nom, adresse préférée), sans rien écrire.
#[tauri::command]
#[specta::specta]
pub async fn preview_contact_merge(
    state: State<'_, AppState>,
    keep: u32,
    others: Vec<u32>,
) -> Result<ContactMergePreviewDto, IpcError> {
    let selection =
        MergeSelection::new(keep, &others).map_err(|_| IpcError::invalid(FieldDto::Contact))?;
    let merged = PreviewMerge {
        directory: &state.contacts,
    }
    .execute(&selection)
    .await
    .map_err(merge_error)?;
    Ok(ContactMergePreviewDto {
        name: merged.display_name(),
        preferred_address: merged.preferred_email().address().to_string(),
    })
}

/// Fusionne les fiches `others` dans la fiche `keep` ; renvoie la fiche
/// gardée. Refusée si une fiche a changé depuis la recherche.
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
