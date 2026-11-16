//! Cas d'usage des dossiers sur le serveur : création ([`CreateFolder`]),
//! renommage ([`RenameFolder`]), déplacement ([`MoveFolder`]) et
//! suppression ([`DeleteFolder`]).
//!
//! Les règles (nom, préfixe, séparateur, doublons, dossiers protégés,
//! sous-dossiers) sont dans `norelly_domain::mailbox` (`new_folder_name`,
//! `renamed_folder_name`, `moved_folder_name`, `deletable_folder`,
//! `no_listed_subfolder`) ; ce module ne fait qu'orchestrer la session,
//! d'après la liste relue sur le serveur (et, avant une suppression, les
//! sous-dossiers relus sous le dossier).

use norelly_domain::connection::ConnectionError;
use norelly_domain::mailbox::{
    FolderChangeError, MailboxName, NewFolderError, classify_mailboxes, deletable_folder,
    moved_folder_name, new_folder_name, no_listed_subfolder, renamed_folder_name,
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
    /// Connexion perdue (ou délai du serveur dépassé) sur la commande qui
    /// modifie le dossier : le serveur a pu l'appliquer. La connexion est
    /// à rouvrir, puis la liste des dossiers est relue. `target` : nom du
    /// dossier visé (créé, renommé ou déplacé, ou supprimé).
    #[error("résultat inconnu : {cause}")]
    Interrupted {
        cause: ConnectionError,
        target: MailboxName,
    },
    /// Délai dépassé alors que la modification était déjà en cours sur la
    /// connexion de commandes : elle a pu aboutir. Elle se poursuit, et la
    /// liste des dossiers est relue à son terme.
    #[error("résultat inconnu : délai dépassé pendant la modification")]
    OutcomeUnknown,
    /// Demande abandonnée par l'appelant avant l'envoi de la commande qui
    /// modifie le dossier : rien n'a été modifié.
    #[error("demande abandonnée avant l'envoi")]
    Abandoned,
}

impl From<FolderChangeError> for FolderFailure {
    fn from(e: FolderChangeError) -> Self {
        match e {
            FolderChangeError::Name(e) => Self::Invalid(e),
            e => Self::Change(e),
        }
    }
}

/// Échec de la commande qui modifie le dossier : refus (`NO`), dossier tel
/// quel ; sinon, résultat inconnu
/// ([`ConnectionError::leaves_command_unapplied`]) sur `target`.
fn sent(target: &MailboxName) -> impl FnOnce(ConnectionError) -> FolderFailure + '_ {
    move |cause| {
        if cause.leaves_command_unapplied() {
            FolderFailure::Connection(cause)
        } else {
            FolderFailure::Interrupted {
                cause,
                target: target.clone(),
            }
        }
    }
}

impl FolderFailure {
    /// Connexion perdue à rouvrir (refus du serveur exclu).
    #[must_use]
    pub fn lost_connection(&self) -> Option<&ConnectionError> {
        match self {
            Self::Connection(ConnectionError::Rejected(_)) => None,
            Self::Connection(e) | Self::Interrupted { cause: e, .. } => Some(e),
            Self::Invalid(_) | Self::Change(_) | Self::OutcomeUnknown | Self::Abandoned => None,
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

/// Point de non-retour : `proceed` refuse, la demande est abandonnée avant
/// l'envoi de la commande qui modifie le dossier.
fn checkpoint(proceed: impl FnOnce() -> bool) -> Result<(), FolderFailure> {
    if proceed() {
        Ok(())
    } else {
        Err(FolderFailure::Abandoned)
    }
}

impl CreateFolder<'_> {
    /// Renvoie le nom complet du dossier créé.
    pub async fn execute<S: RemoteMailboxSession>(
        &self,
        session: &mut S,
    ) -> Result<MailboxName, FolderFailure> {
        self.execute_if(session, || true).await
    }

    /// Comme [`Self::execute`], si `proceed` l'autorise juste avant le
    /// `CREATE`.
    pub async fn execute_if<S: RemoteMailboxSession>(
        &self,
        session: &mut S,
        proceed: impl FnOnce() -> bool + Send,
    ) -> Result<MailboxName, FolderFailure> {
        let existing = classify_mailboxes(session.list_mailboxes().await?);
        let full = new_folder_name(self.label, self.parent, &existing)?;
        checkpoint(proceed)?;
        session.create_mailbox(&full).await.map_err(sent(&full))?;
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
        self.execute_if(session, || true).await
    }

    /// Comme [`Self::execute`], si `proceed` l'autorise juste avant le
    /// `RENAME`.
    pub async fn execute_if<S: RemoteMailboxSession>(
        &self,
        session: &mut S,
        proceed: impl FnOnce() -> bool + Send,
    ) -> Result<MailboxName, FolderFailure> {
        let existing = classify_mailboxes(session.list_mailboxes().await?);
        let renamed = renamed_folder_name(self.folder, self.label, &existing)?;
        if renamed != *self.folder {
            checkpoint(proceed)?;
            session
                .rename_mailbox(self.folder, &renamed)
                .await
                .map_err(sent(&renamed))?;
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
        self.execute_if(session, || true).await
    }

    /// Comme [`Self::execute`], si `proceed` l'autorise juste avant le
    /// `RENAME`.
    pub async fn execute_if<S: RemoteMailboxSession>(
        &self,
        session: &mut S,
        proceed: impl FnOnce() -> bool + Send,
    ) -> Result<MailboxName, FolderFailure> {
        let existing = classify_mailboxes(session.list_mailboxes().await?);
        let moved = moved_folder_name(self.folder, self.parent, &existing)?;
        if moved != *self.folder {
            checkpoint(proceed)?;
            session
                .rename_mailbox(self.folder, &moved)
                .await
                .map_err(sent(&moved))?;
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
        self.execute_if(session, || true).await
    }

    /// Comme [`Self::execute`], si `proceed` l'autorise juste avant le
    /// `DELETE`.
    pub async fn execute_if<S: RemoteMailboxSession>(
        &self,
        session: &mut S,
        proceed: impl FnOnce() -> bool + Send,
    ) -> Result<(), FolderFailure> {
        let existing = classify_mailboxes(session.list_mailboxes().await?);
        if let Some(delimiter) = deletable_folder(self.folder, &existing)? {
            no_listed_subfolder(session.count_subfolders(self.folder, delimiter).await?)?;
        }
        // Après les contrôles (sous-dossiers relus compris) : dernier point
        // d'abandon.
        checkpoint(proceed)?;
        session
            .delete_mailbox(self.folder)
            .await
            .map_err(sent(self.folder))?;
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
    /// Comme [`Self::execute`], si `proceed` l'autorise juste avant
    /// l'envoi de la commande qui modifie le dossier.
    pub async fn execute_if<S: RemoteMailboxSession>(
        &self,
        session: &mut S,
        proceed: impl FnOnce() -> bool + Send,
    ) -> Result<MailboxName, FolderFailure> {
        match self {
            Self::Create { label, parent } => {
                CreateFolder {
                    label,
                    parent: parent.as_ref(),
                }
                .execute_if(session, proceed)
                .await
            }
            Self::Rename { folder, label } => {
                RenameFolder { folder, label }
                    .execute_if(session, proceed)
                    .await
            }
            Self::Move { folder, parent } => {
                MoveFolder {
                    folder,
                    parent: parent.as_ref(),
                }
                .execute_if(session, proceed)
                .await
            }
            Self::Delete { folder } => {
                DeleteFolder { folder }.execute_if(session, proceed).await?;
                Ok(folder.clone())
            }
        }
    }

    /// Applique la modification ; renvoie le nom du dossier créé, renommé
    /// ou déplacé (nouveau nom), ou supprimé.
    pub async fn execute<S: RemoteMailboxSession>(
        &self,
        session: &mut S,
    ) -> Result<MailboxName, FolderFailure> {
        self.execute_if(session, || true).await
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
