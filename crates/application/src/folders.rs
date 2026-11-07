//! Cas d'usage des dossiers sur le serveur : création ([`CreateFolder`]),
//! renommage ([`RenameFolder`]), déplacement ([`MoveFolder`]) et
//! suppression ([`DeleteFolder`]).
//!
//! Les règles (nom, préfixe, séparateur, doublons, dossiers protégés,
//! sous-dossiers) sont dans `norelly_domain::mailbox` (`new_folder_name`,
//! `renamed_folder_name`, `moved_folder_name`, `deletable_folder`) ; ce
//! module ne fait qu'orchestrer la session, d'après la liste relue sur le
//! serveur.

use norelly_domain::connection::ConnectionError;
use norelly_domain::mailbox::{
    FolderChangeError, MailboxName, NewFolderError, classify_mailboxes, deletable_folder,
    moved_folder_name, new_folder_name, renamed_folder_name,
};
use norelly_domain::ports::RemoteMailboxSession;

/// Échec de la création d'un dossier.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FolderFailure {
    /// Nom refusé par le domaine : rien n'a été envoyé au serveur.
    #[error(transparent)]
    Invalid(#[from] NewFolderError),
    /// Dossier qui ne peut être ni renommé ni supprimé : rien n'a été
    /// envoyé au serveur.
    #[error(transparent)]
    Change(FolderChangeError),
    /// Refus du serveur (`NO`) ou connexion perdue.
    #[error(transparent)]
    Connection(#[from] ConnectionError),
}

impl From<FolderChangeError> for FolderFailure {
    fn from(e: FolderChangeError) -> Self {
        match e {
            FolderChangeError::Name(e) => Self::Invalid(e),
            e => Self::Change(e),
        }
    }
}

/// Cas d'usage : créer le dossier `label`, sous `parent` ou au premier
/// niveau. La liste des dossiers est relue sur le serveur juste avant, pour
/// décider du nom d'après son état présent.
#[derive(Debug)]
pub struct CreateFolder<'a> {
    pub label: &'a str,
    pub parent: Option<&'a MailboxName>,
}

impl CreateFolder<'_> {
    /// Renvoie le nom complet du dossier créé.
    pub async fn execute<S: RemoteMailboxSession>(
        &self,
        session: &mut S,
    ) -> Result<MailboxName, FolderFailure> {
        let existing = classify_mailboxes(session.list_mailboxes().await?);
        let full = new_folder_name(self.label, self.parent, &existing)?;
        session.create_mailbox(&full).await?;
        Ok(full)
    }
}

/// Cas d'usage : renommer le dossier `folder` en `label`, à la même place.
#[derive(Debug)]
pub struct RenameFolder<'a> {
    pub folder: &'a MailboxName,
    pub label: &'a str,
}

impl RenameFolder<'_> {
    /// Renvoie le nouveau nom complet du dossier ; nom inchangé : rien
    /// n'est envoyé.
    pub async fn execute<S: RemoteMailboxSession>(
        &self,
        session: &mut S,
    ) -> Result<MailboxName, FolderFailure> {
        let existing = classify_mailboxes(session.list_mailboxes().await?);
        let renamed = renamed_folder_name(self.folder, self.label, &existing)?;
        if renamed != *self.folder {
            session.rename_mailbox(self.folder, &renamed).await?;
        }
        Ok(renamed)
    }
}

/// Cas d'usage : déplacer le dossier `folder` sous `parent` (ou au premier
/// niveau), avec son nom et ses sous-dossiers.
#[derive(Debug)]
pub struct MoveFolder<'a> {
    pub folder: &'a MailboxName,
    pub parent: Option<&'a MailboxName>,
}

impl MoveFolder<'_> {
    /// Renvoie le nouveau nom complet du dossier ; déjà à cet endroit :
    /// rien n'est envoyé.
    pub async fn execute<S: RemoteMailboxSession>(
        &self,
        session: &mut S,
    ) -> Result<MailboxName, FolderFailure> {
        let existing = classify_mailboxes(session.list_mailboxes().await?);
        let moved = moved_folder_name(self.folder, self.parent, &existing)?;
        if moved != *self.folder {
            session.rename_mailbox(self.folder, &moved).await?;
        }
        Ok(moved)
    }
}

/// Cas d'usage : supprimer le dossier `folder` et ses messages.
#[derive(Debug)]
pub struct DeleteFolder<'a> {
    pub folder: &'a MailboxName,
}

impl DeleteFolder<'_> {
    pub async fn execute<S: RemoteMailboxSession>(
        &self,
        session: &mut S,
    ) -> Result<(), FolderFailure> {
        let existing = classify_mailboxes(session.list_mailboxes().await?);
        deletable_folder(self.folder, &existing)?;
        session.delete_mailbox(self.folder).await?;
        Ok(())
    }
}

/// Modification d'un dossier demandée par l'utilisateur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderChange {
    Create {
        label: String,
        parent: Option<MailboxName>,
    },
    Rename {
        folder: MailboxName,
        label: String,
    },
    Move {
        folder: MailboxName,
        parent: Option<MailboxName>,
    },
    Delete {
        folder: MailboxName,
    },
}

impl FolderChange {
    /// Applique la modification ; renvoie le nom du dossier créé, renommé
    /// ou déplacé (nouveau nom), ou supprimé.
    pub async fn execute<S: RemoteMailboxSession>(
        &self,
        session: &mut S,
    ) -> Result<MailboxName, FolderFailure> {
        match self {
            Self::Create { label, parent } => {
                CreateFolder {
                    label,
                    parent: parent.as_ref(),
                }
                .execute(session)
                .await
            }
            Self::Rename { folder, label } => RenameFolder { folder, label }.execute(session).await,
            Self::Move { folder, parent } => {
                MoveFolder {
                    folder,
                    parent: parent.as_ref(),
                }
                .execute(session)
                .await
            }
            Self::Delete { folder } => {
                DeleteFolder { folder }.execute(session).await?;
                Ok(folder.clone())
            }
        }
    }

    /// Nom de l'opération, pour les journaux.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Create { .. } => "dossier créé",
            Self::Rename { .. } => "dossier renommé",
            Self::Move { .. } => "dossier déplacé",
            Self::Delete { .. } => "dossier supprimé",
        }
    }
}

#[cfg(test)]
mod tests;
