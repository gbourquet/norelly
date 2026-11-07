//! Commandes IPC des dossiers : création, renommage, déplacement et
//! suppression d'un dossier sur le serveur.
//!
//! L'interface envoie le nom saisi et l'identifiant local du dossier visé
//! (parent d'un nouveau dossier, dossier à renommer, à déplacer et sa
//! destination, dossier à supprimer) ; l'identifiant est rapporté à un
//! dossier du compte, puis le domaine décide (`new_folder_name`,
//! `renamed_folder_name`, `moved_folder_name`, `deletable_folder`) d'après
//! la liste relue sur le serveur.

use norelly_application::folders::{FolderChange, FolderFailure};
use norelly_domain::account::AccountId;
use norelly_domain::connection::ConnectionError;
use norelly_domain::mailbox::{
    FolderChangeError, MailboxId, MailboxName, NewFolderError, folder_label,
};
use norelly_domain::ports::PendingOpStore;
use tauri::State;

use crate::ipc::{ErrorCodeDto, FieldDto, IpcError};
use crate::state::AppState;

impl From<NewFolderError> for IpcError {
    fn from(e: NewFolderError) -> Self {
        match e {
            NewFolderError::Empty => Self::new(ErrorCodeDto::FolderNameEmpty),
            NewFolderError::TooLong => Self::new(ErrorCodeDto::FolderNameTooLong),
            NewFolderError::InvalidCharacter => Self::new(ErrorCodeDto::FolderNameInvalid),
            NewFolderError::ContainsDelimiter(d) => {
                Self::new(ErrorCodeDto::FolderNameDelimiter).with_detail(d)
            }
            NewFolderError::AlreadyExists => Self::new(ErrorCodeDto::FolderExists),
            NewFolderError::NoHierarchy => Self::new(ErrorCodeDto::NoSubfolders),
            NewFolderError::UnknownParent | NewFolderError::VirtualParent => {
                Self::invalid(FieldDto::Mailbox)
            }
        }
    }
}

impl From<FolderChangeError> for IpcError {
    fn from(e: FolderChangeError) -> Self {
        match e {
            FolderChangeError::UnknownFolder => Self::invalid(FieldDto::Mailbox),
            FolderChangeError::Protected => Self::new(ErrorCodeDto::FolderProtected),
            FolderChangeError::HasSubfolders => Self::new(ErrorCodeDto::FolderHasSubfolders),
            FolderChangeError::IntoItself => Self::new(ErrorCodeDto::FolderIntoItself),
            FolderChangeError::Name(e) => e.into(),
        }
    }
}

/// Échec d'une modification de dossier ; refus du serveur ou connexion
/// perdue : code `failed`, propre à l'opération.
fn folder_error(e: FolderFailure, failed: ErrorCodeDto) -> IpcError {
    match e {
        FolderFailure::Invalid(e) => e.into(),
        FolderFailure::Change(e) => e.into(),
        // Réponse du serveur citée telle quelle ; sinon, la cause
        // technique (hors ligne, délai…).
        FolderFailure::Connection(ConnectionError::Rejected(reason)) => {
            IpcError::new(failed).with_detail(reason)
        }
        FolderFailure::Connection(e) => IpcError::new(failed).with_detail(e),
    }
}

fn account_id(account: u32) -> Result<AccountId, IpcError> {
    AccountId::new(i64::from(account)).map_err(|_| IpcError::invalid(FieldDto::Account))
}

fn mailbox_id(mailbox: u32) -> Result<MailboxId, IpcError> {
    MailboxId::new(i64::from(mailbox)).ok_or_else(|| IpcError::invalid(FieldDto::Mailbox))
}

/// Nom du dossier `id` du compte `account` (sélectionnable ou non) ;
/// dossier d'un autre compte ou inconnu : entrée invalide.
async fn folder_of(
    state: &AppState,
    account: AccountId,
    id: MailboxId,
) -> Result<MailboxName, IpcError> {
    state
        .mail
        .store
        .action_folders(account)
        .await
        .map_err(IpcError::storage)?
        .into_iter()
        .find(|f| f.id == id)
        .map(|f| f.name)
        .ok_or_else(|| IpcError::invalid(FieldDto::Mailbox))
}

/// Crée le dossier `name` dans le compte `account`, sous `parent` ou au
/// premier niveau ; la liste des dossiers est ensuite relue (événement
/// `folders_changed`).
#[tauri::command]
#[specta::specta]
pub async fn create_folder(
    state: State<'_, AppState>,
    account: u32,
    name: String,
    parent: Option<u32>,
) -> Result<(), IpcError> {
    let account = account_id(account)?;
    let parent = parent.map(mailbox_id).transpose()?;
    let label = folder_label(&name)?;
    // Le parent doit être un dossier du compte (sélectionnable ou non :
    // un dossier « \Noselect » peut contenir des sous-dossiers).
    let parent = match parent {
        None => None,
        Some(id) => Some(folder_of(&state, account, id).await?),
    };
    let change = FolderChange::Create {
        label: label.to_owned(),
        parent,
    };
    state
        .mail
        .change_folder(account, change)
        .await
        .map_err(|e| folder_error(e, ErrorCodeDto::FolderCreateFailed))?;
    Ok(())
}

/// Renomme le dossier `mailbox` du compte `account` en `name`, à la même
/// place ; renvoie l'identifiant local du dossier sous son nouveau nom
/// (`null` si la liste n'a pas encore été relue).
#[tauri::command]
#[specta::specta]
pub async fn rename_folder(
    state: State<'_, AppState>,
    account: u32,
    mailbox: u32,
    name: String,
) -> Result<Option<u32>, IpcError> {
    let account = account_id(account)?;
    let id = mailbox_id(mailbox)?;
    let label = folder_label(&name)?;
    let folder = folder_of(&state, account, id).await?;
    let change = FolderChange::Rename {
        folder,
        label: label.to_owned(),
    };
    let renamed = state
        .mail
        .change_folder(account, change)
        .await
        .map_err(|e| folder_error(e, ErrorCodeDto::FolderRenameFailed))?;
    local_id(&state, account, &renamed).await
}

/// Identifiant local du dossier `name` du compte, d'après la liste relue
/// par l'acteur après la modification (`None` si elle ne l'est pas encore).
async fn local_id(
    state: &AppState,
    account: AccountId,
    name: &MailboxName,
) -> Result<Option<u32>, IpcError> {
    let folders = state
        .mail
        .store
        .action_folders(account)
        .await
        .map_err(IpcError::storage)?;
    Ok(folders
        .into_iter()
        .find(|f| f.name == *name)
        .and_then(|f| u32::try_from(f.id.get()).ok()))
}

/// Déplace le dossier `mailbox` du compte `account` sous `parent` (ou au
/// premier niveau), avec ses sous-dossiers ; renvoie l'identifiant local du
/// dossier à sa nouvelle place (`null` si la liste n'a pas encore été
/// relue).
#[tauri::command]
#[specta::specta]
pub async fn move_folder(
    state: State<'_, AppState>,
    account: u32,
    mailbox: u32,
    parent: Option<u32>,
) -> Result<Option<u32>, IpcError> {
    let account = account_id(account)?;
    let id = mailbox_id(mailbox)?;
    let parent = parent.map(mailbox_id).transpose()?;
    let folder = folder_of(&state, account, id).await?;
    let parent = match parent {
        None => None,
        Some(id) => Some(folder_of(&state, account, id).await?),
    };
    let moved = state
        .mail
        .change_folder(account, FolderChange::Move { folder, parent })
        .await
        .map_err(|e| folder_error(e, ErrorCodeDto::FolderMoveFailed))?;
    local_id(&state, account, &moved).await
}

/// Supprime le dossier `mailbox` du compte `account` et ses messages, sur
/// le serveur ; la liste des dossiers est ensuite relue.
#[tauri::command]
#[specta::specta]
pub async fn delete_folder(
    state: State<'_, AppState>,
    account: u32,
    mailbox: u32,
) -> Result<(), IpcError> {
    let account = account_id(account)?;
    let id = mailbox_id(mailbox)?;
    let folder = folder_of(&state, account, id).await?;
    state
        .mail
        .change_folder(account, FolderChange::Delete { folder })
        .await
        .map_err(|e| folder_error(e, ErrorCodeDto::FolderDeleteFailed))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(e: FolderFailure) -> (ErrorCodeDto, Option<String>) {
        let e = folder_error(e, ErrorCodeDto::FolderCreateFailed);
        (e.code, e.detail)
    }

    #[test]
    fn folder_failures_become_codes() {
        for (failure, expected) in [
            (NewFolderError::Empty, ErrorCodeDto::FolderNameEmpty),
            (NewFolderError::TooLong, ErrorCodeDto::FolderNameTooLong),
            (
                NewFolderError::InvalidCharacter,
                ErrorCodeDto::FolderNameInvalid,
            ),
            (NewFolderError::AlreadyExists, ErrorCodeDto::FolderExists),
            (NewFolderError::NoHierarchy, ErrorCodeDto::NoSubfolders),
        ] {
            assert_eq!(code(failure.into()), (expected, None), "{failure:?}");
        }
        for failure in [NewFolderError::UnknownParent, NewFolderError::VirtualParent] {
            assert_eq!(code(failure.into()).0, ErrorCodeDto::InvalidInput);
        }
        assert_eq!(
            code(NewFolderError::ContainsDelimiter('/').into()),
            (ErrorCodeDto::FolderNameDelimiter, Some("/".to_owned()))
        );
        assert_eq!(
            code(ConnectionError::Rejected("[NOPERM] interdit".into()).into()),
            (
                ErrorCodeDto::FolderCreateFailed,
                Some("[NOPERM] interdit".to_owned())
            )
        );
        assert_eq!(
            code(ConnectionError::Closed.into()).0,
            ErrorCodeDto::FolderCreateFailed
        );
    }

    #[test]
    fn folder_change_failures_become_codes() {
        let change = |e: FolderChangeError| {
            let e = folder_error(e.into(), ErrorCodeDto::FolderRenameFailed);
            (e.code, e.detail)
        };
        assert_eq!(
            change(FolderChangeError::Protected),
            (ErrorCodeDto::FolderProtected, None)
        );
        assert_eq!(
            change(FolderChangeError::HasSubfolders),
            (ErrorCodeDto::FolderHasSubfolders, None)
        );
        assert_eq!(
            change(FolderChangeError::IntoItself),
            (ErrorCodeDto::FolderIntoItself, None)
        );
        assert_eq!(
            change(FolderChangeError::UnknownFolder).0,
            ErrorCodeDto::InvalidInput
        );
        assert_eq!(
            change(FolderChangeError::Name(NewFolderError::AlreadyExists)),
            (ErrorCodeDto::FolderExists, None)
        );
        // Refus du serveur : code propre à l'opération, réponse citée.
        for failed in [
            ErrorCodeDto::FolderRenameFailed,
            ErrorCodeDto::FolderMoveFailed,
            ErrorCodeDto::FolderDeleteFailed,
        ] {
            let e = folder_error(
                ConnectionError::Rejected("[NOPERM] interdit".into()).into(),
                failed,
            );
            assert_eq!(
                (e.code, e.detail),
                (failed, Some("[NOPERM] interdit".to_owned()))
            );
        }
    }
}
