//! Commandes IPC des listes de diffusion (ADR-012) : lister, lire, créer,
//! modifier, supprimer une liste, y ajouter des contacts, la déplier en
//! destinataires, la proposer pendant la saisie.
//!
//! La saisie reçue est revalidée par le domaine (`ContactListDraft::new`,
//! dans les cas d'usage) ; les identifiants, la sélection et le curseur
//! aussi. Les refus deviennent des codes ; aucun texte destiné à
//! l'utilisateur.

use norelly_application::contact_lists::{
    AddToContactList, ContactListFailure, ContactListView, CreateContactList, DeleteContactLists,
    ExpandContactList, GetContactList, ListContactLists, SuggestContactLists, UpdateContactList,
};
use norelly_domain::contact_lists::{
    ContactListError, ContactListField, ContactListId, ContactListInput, MAX_LIST_SUGGESTIONS,
};
use norelly_domain::contacts::{Contact, contact_selection};
use norelly_domain::ports::{
    ContactListCursor, ContactListItem, ContactListStoreError, ContactListSummary,
};
use tauri::State;

use crate::contacts::item_dto as contact_item_dto;
use crate::ipc::{
    ContactItemDto, ContactListCursorDto, ContactListDto, ContactListFieldsDto, ContactListItemDto,
    ContactListPageDto, ErrorCodeDto, ExpandedListDto, FieldDto, IpcError, RecipientDto, id32,
};
use crate::state::AppState;

/// Échec d'un cas d'usage des listes, en code.
fn list_error(e: ContactListFailure) -> IpcError {
    let field = |code: ErrorCodeDto, field: FieldDto| IpcError {
        field: Some(field),
        ..IpcError::new(code)
    };
    let field_of = |f: ContactListField| match f {
        ContactListField::Name => FieldDto::ListName,
        ContactListField::Description => FieldDto::ListDescription,
    };
    match e {
        ContactListFailure::Invalid(e) => match e {
            ContactListError::NoName => field(ErrorCodeDto::ContactListNoName, FieldDto::ListName),
            ContactListError::TooLong(f) => field(ErrorCodeDto::ContactListTooLong, field_of(f)),
            ContactListError::InvalidCharacter(f) => {
                field(ErrorCodeDto::ContactListInvalid, field_of(f))
            }
            ContactListError::InvalidMember => IpcError::invalid(FieldDto::Contact),
            ContactListError::TooManyMembers => {
                IpcError::new(ErrorCodeDto::ContactListTooManyMembers)
            }
            ContactListError::Empty => IpcError::new(ErrorCodeDto::ContactListEmpty),
        },
        ContactListFailure::Store(e) => match e {
            ContactListStoreError::DuplicateName(name) => {
                field(ErrorCodeDto::ContactListDuplicateName, FieldDto::ListName).with_detail(name)
            }
            ContactListStoreError::UnknownContact(_) => {
                IpcError::new(ErrorCodeDto::ContactNotFound)
            }
            ContactListStoreError::NotFound => IpcError::new(ErrorCodeDto::ContactListNotFound),
            ContactListStoreError::Store(e) => IpcError::storage(e),
        },
    }
}

fn list_id(raw: u32) -> Result<ContactListId, IpcError> {
    ContactListId::new(i64::from(raw)).ok_or_else(|| IpcError::invalid(FieldDto::ContactList))
}

/// Saisie de l'interface, à valider par le domaine.
fn input_of(dto: ContactListFieldsDto) -> ContactListInput {
    ContactListInput {
        name: dto.name,
        description: dto.description,
        members: dto.members.into_iter().map(i64::from).collect(),
        bcc: dto.bcc,
    }
}

fn item_dto(item: &ContactListSummary) -> Result<ContactListItemDto, IpcError> {
    Ok(ContactListItemDto {
        id: id32(item.id.get())?,
        name: item.name.clone(),
        members: u32::try_from(item.members).map_err(|_| IpcError::internal("membres"))?,
        bcc: item.bcc,
    })
}

fn list_dto(view: &ContactListView) -> Result<ContactListDto, IpcError> {
    let draft = view.list.draft();
    let input = draft.to_input();
    Ok(ContactListDto {
        id: id32(view.list.id().get())?,
        fields: ContactListFieldsDto {
            name: input.name,
            description: input.description,
            members: draft
                .members()
                .iter()
                .map(|m| id32(m.get()))
                .collect::<Result<_, _>>()?,
            bcc: input.bcc,
        },
        members: view
            .members
            .iter()
            .map(item_dto_of_contact)
            .collect::<Result<_, _>>()?,
    })
}

fn item_dto_of_contact(contact: &Contact) -> Result<ContactItemDto, IpcError> {
    contact_item_dto(ContactListItem::from(contact))
}

/// Liste `id` relue avec ses membres.
async fn view(state: &AppState, id: ContactListId) -> Result<ContactListDto, IpcError> {
    let view = GetContactList {
        lists: &state.contacts,
        contacts: &state.contacts,
    }
    .execute(id)
    .await
    .map_err(list_error)?;
    list_dto(&view)
}

/// Une page des listes filtrée sur le nom (`text` vide : toutes), après
/// `cursor`.
#[tauri::command]
#[specta::specta]
pub async fn list_contact_lists(
    state: State<'_, AppState>,
    text: String,
    cursor: Option<ContactListCursorDto>,
    limit: u32,
) -> Result<ContactListPageDto, IpcError> {
    let cursor = cursor
        .map(|c| {
            ContactListCursor::parse(c.sort_key, i64::from(c.id))
                .ok_or_else(|| IpcError::invalid(FieldDto::Cursor))
        })
        .transpose()?;
    let page = ListContactLists {
        directory: &state.contacts,
    }
    .execute(&text, cursor, limit)
    .await
    .map_err(list_error)?;
    Ok(ContactListPageDto {
        items: page.items.iter().map(item_dto).collect::<Result<_, _>>()?,
        next: page
            .next
            .map(|c| {
                Ok::<_, IpcError>(ContactListCursorDto {
                    sort_key: c.sort_key,
                    id: id32(c.id.get())?,
                })
            })
            .transpose()?,
    })
}

/// Liste `id` et ses membres.
#[tauri::command]
#[specta::specta]
pub async fn get_contact_list(
    state: State<'_, AppState>,
    id: u32,
) -> Result<ContactListDto, IpcError> {
    view(&state, list_id(id)?).await
}

/// Enregistre une nouvelle liste.
#[tauri::command]
#[specta::specta]
pub async fn create_contact_list(
    state: State<'_, AppState>,
    list: ContactListFieldsDto,
) -> Result<ContactListDto, IpcError> {
    let created = CreateContactList {
        book: &state.contacts,
    }
    .execute(input_of(list))
    .await
    .map_err(list_error)?;
    view(&state, created.id()).await
}

/// Remplace le contenu de la liste `id`.
#[tauri::command]
#[specta::specta]
pub async fn update_contact_list(
    state: State<'_, AppState>,
    id: u32,
    list: ContactListFieldsDto,
) -> Result<ContactListDto, IpcError> {
    let id = list_id(id)?;
    UpdateContactList {
        book: &state.contacts,
    }
    .execute(id, input_of(list))
    .await
    .map_err(list_error)?;
    view(&state, id).await
}

/// Supprime la liste `id` (ses contacts restent) ; `false` si elle
/// n'existait plus.
#[tauri::command]
#[specta::specta]
pub async fn delete_contact_list(state: State<'_, AppState>, id: u32) -> Result<bool, IpcError> {
    let id = list_id(id)?;
    let deleted = DeleteContactLists {
        book: &state.contacts,
    }
    .execute(&[id])
    .await
    .map_err(list_error)?;
    Ok(deleted == 1)
}

/// Ajoute des contacts à la liste `id` (déjà membres : ignorés).
#[tauri::command]
#[specta::specta]
pub async fn add_to_contact_list(
    state: State<'_, AppState>,
    id: u32,
    contacts: Vec<u32>,
) -> Result<ContactListDto, IpcError> {
    let id = list_id(id)?;
    let contacts =
        contact_selection(&contacts).map_err(|_| IpcError::invalid(FieldDto::Contact))?;
    AddToContactList {
        book: &state.contacts,
    }
    .execute(id, &contacts)
    .await
    .map_err(list_error)?;
    view(&state, id).await
}

/// Destinataires de la liste `id` : chaque membre à son adresse préférée,
/// nom lu par le domaine.
#[tauri::command]
#[specta::specta]
pub async fn expand_contact_list(
    state: State<'_, AppState>,
    id: u32,
) -> Result<ExpandedListDto, IpcError> {
    let id = list_id(id)?;
    let expansion = ExpandContactList {
        lists: &state.contacts,
        contacts: &state.contacts,
    }
    .execute(id)
    .await
    .map_err(list_error)?;
    Ok(ExpandedListDto {
        recipients: expansion
            .recipients
            .into_iter()
            .map(|r| RecipientDto {
                name: r.name,
                address: r.address.to_string(),
            })
            .collect(),
        bcc: expansion.bcc,
    })
}

/// Listes proposées pendant la saisie d'un destinataire.
#[tauri::command]
#[specta::specta]
pub async fn suggest_contact_lists(
    state: State<'_, AppState>,
    text: String,
) -> Result<Vec<ContactListItemDto>, IpcError> {
    let found = SuggestContactLists {
        directory: &state.contacts,
    }
    .execute(&text, MAX_LIST_SUGGESTIONS)
    .await
    .map_err(IpcError::storage)?;
    found.iter().map(item_dto).collect()
}

#[cfg(test)]
mod tests;
