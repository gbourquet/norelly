//! Cas d'usage des dossiers : création d'un dossier sur le serveur
//! ([`CreateFolder`]).
//!
//! La règle du nom (validation, préfixe, séparateur, doublons) est dans
//! `norelly_domain::mailbox::new_folder_name` ; ce module ne fait
//! qu'orchestrer la session.

use norelly_domain::connection::ConnectionError;
use norelly_domain::mailbox::{MailboxName, NewFolderError, classify_mailboxes, new_folder_name};
use norelly_domain::ports::RemoteMailboxSession;

/// Échec de la création d'un dossier.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FolderFailure {
    /// Nom refusé par le domaine : rien n'a été envoyé au serveur.
    #[error(transparent)]
    Invalid(#[from] NewFolderError),
    /// Refus du serveur (`NO`) ou connexion perdue.
    #[error(transparent)]
    Connection(#[from] ConnectionError),
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

#[cfg(test)]
mod tests;
