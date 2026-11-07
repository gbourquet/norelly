//! Commandes IPC des dossiers : création d'un dossier sur le serveur.
//!
//! L'interface envoie le nom saisi et, pour un sous-dossier, le dossier
//! parent ; le nom complet est décidé par le domaine
//! (`new_folder_name`), d'après la liste relue sur le serveur.

use norelly_application::folders::FolderFailure;
use norelly_domain::account::AccountId;
use norelly_domain::connection::ConnectionError;
use norelly_domain::mailbox::{MailboxId, NewFolderError, folder_label};
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

impl From<FolderFailure> for IpcError {
    fn from(e: FolderFailure) -> Self {
        match e {
            FolderFailure::Invalid(e) => e.into(),
            // Réponse du serveur citée telle quelle ; sinon, la cause
            // technique (hors ligne, délai…).
            FolderFailure::Connection(ConnectionError::Rejected(reason)) => {
                Self::new(ErrorCodeDto::FolderCreateFailed).with_detail(reason)
            }
            FolderFailure::Connection(e) => {
                Self::new(ErrorCodeDto::FolderCreateFailed).with_detail(e)
            }
        }
    }
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
    let account =
        AccountId::new(i64::from(account)).map_err(|_| IpcError::invalid(FieldDto::Account))?;
    let parent = parent
        .map(|m| MailboxId::new(i64::from(m)).ok_or_else(|| IpcError::invalid(FieldDto::Mailbox)))
        .transpose()?;
    let label = folder_label(&name)?;
    // Le parent doit être un dossier du compte (sélectionnable ou non :
    // un dossier « \Noselect » peut contenir des sous-dossiers).
    let parent = match parent {
        None => None,
        Some(id) => {
            let folders = state
                .mail
                .store
                .action_folders(account)
                .await
                .map_err(IpcError::storage)?;
            let folder = folders
                .into_iter()
                .find(|f| f.id == id)
                .ok_or_else(|| IpcError::invalid(FieldDto::Mailbox))?;
            Some(folder.name)
        }
    };
    state
        .mail
        .create_folder(account, label.to_owned(), parent)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(e: FolderFailure) -> (ErrorCodeDto, Option<String>) {
        let e = IpcError::from(e);
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
}
