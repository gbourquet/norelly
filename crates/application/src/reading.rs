//! Cas d'usage de lecture :
//! - [`ReadMessage`] : emplacement, corps stocké (blobs), décision sur les
//!   images distantes (autorisation par expéditeur, domaine) et préférence
//!   « texte brut » ; le rendu lui-même est fait par l'adaptateur
//!   `norelly-render` au point d'assemblage ;
//! - [`allow_remote_images`] : autorise l'expéditeur d'un message ;
//! - [`DownloadPart`] : télécharge une partie différée (pièce jointe
//!   volumineuse) par la connexion IMAP du compte, puis l'enregistre.

use std::sync::Arc;

use norelly_domain::fetch_policy::{BodyFetch, FetchPolicy, PartPath};
use norelly_domain::ports::{
    BlobStore, FetchedBody, MailStore, MessageId, MessageLocation, MessageLocator,
    ReadingPreferences, RemoteBodyFetcher, SearchTextExtractor, StoreError, StoredBody,
};
use norelly_domain::reading::{RemoteContent, remote_content, sender_address};

use crate::sync::store_fetched;

/// Échec d'un cas d'usage de lecture.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("message introuvable")]
    NotFound,
    #[error("expéditeur sans adresse valide : autorisation impossible")]
    NoValidSender,
    #[error("partie introuvable")]
    PartNotFound,
    #[error("téléchargement impossible : {0}")]
    Download(String),
}

/// Corps d'un message, chargé depuis les blobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadedBody {
    /// Pas encore téléchargé (ou contenu local perdu).
    Missing,
    Full(Vec<u8>),
    Partial {
        header: Vec<u8>,
        parts: Vec<(PartPath, Vec<u8>)>,
        deferred: Vec<PartPath>,
    },
}

/// Message prêt à rendre.
#[derive(Debug, Clone)]
pub struct LoadedMessage {
    pub location: MessageLocation,
    pub body: LoadedBody,
    pub remote: RemoteContent,
    pub prefer_plain_text: bool,
}

/// Cas d'usage : charger un message pour le lire.
#[derive(Debug)]
pub struct ReadMessage<'a, L, B, P> {
    pub locator: &'a L,
    pub blobs: &'a B,
    pub prefs: &'a P,
}

impl<L: MessageLocator, B: BlobStore, P: ReadingPreferences> ReadMessage<'_, L, B, P> {
    /// Corps stocké d'un message localisé. Un blob manquant (données
    /// locales effacées) donne un corps absent.
    pub async fn body(&self, location: &MessageLocation) -> Result<LoadedBody, StoreError> {
        let account = location.account;
        Ok(match &location.body {
            StoredBody::Missing => LoadedBody::Missing,
            StoredBody::Full(id) => match self.blobs.get(account, id).await? {
                Some(raw) => LoadedBody::Full(raw),
                None => LoadedBody::Missing,
            },
            StoredBody::Partial {
                header,
                parts,
                deferred,
            } => {
                let Some(header) = self.blobs.get(account, header).await? else {
                    return Ok(LoadedBody::Missing);
                };
                let mut loaded = Vec::with_capacity(parts.len());
                let mut deferred = deferred.clone();
                for (path, id) in parts {
                    match self.blobs.get(account, id).await? {
                        Some(bytes) => loaded.push((path.clone(), bytes)),
                        None => deferred.push(path.clone()),
                    }
                }
                LoadedBody::Partial {
                    header,
                    parts: loaded,
                    deferred,
                }
            }
        })
    }

    /// Message, corps et décisions de lecture ; `None` s'il n'existe pas.
    pub async fn load(&self, id: MessageId) -> Result<Option<LoadedMessage>, ReadError> {
        let Some(location) = self.locator.locate(id).await? else {
            return Ok(None);
        };
        let allowed = match sender_address(location.from.as_ref()) {
            Some(sender) => self.prefs.remote_images_allowed(&sender).await?,
            None => false,
        };
        let body = self.body(&location).await?;
        Ok(Some(LoadedMessage {
            body,
            remote: remote_content(allowed),
            prefer_plain_text: self.prefs.prefer_plain_text().await?,
            location,
        }))
    }
}

/// Autorise les images distantes de l'expéditeur du message `id`.
pub async fn allow_remote_images<L: MessageLocator, P: ReadingPreferences>(
    locator: &L,
    prefs: &P,
    id: MessageId,
) -> Result<(), ReadError> {
    let location = locator.locate(id).await?.ok_or(ReadError::NotFound)?;
    let sender = sender_address(location.from.as_ref()).ok_or(ReadError::NoValidSender)?;
    prefs.allow_remote_images(&sender).await?;
    Ok(())
}

/// Cas d'usage : télécharger une partie différée et l'enregistrer.
#[derive(Debug)]
pub struct DownloadPart<'a, L, B, M, F> {
    pub locator: &'a L,
    pub blobs: &'a B,
    pub store: &'a M,
    pub fetcher: &'a F,
}

impl<L, B, M, F> DownloadPart<'_, L, B, M, F>
where
    L: MessageLocator,
    B: BlobStore,
    M: MailStore,
    F: RemoteBodyFetcher,
{
    /// Télécharge la partie `path` du message `id` si elle est différée ;
    /// sans effet si elle est déjà présente.
    pub async fn execute(&self, id: MessageId, path: &PartPath) -> Result<(), ReadError> {
        let location = self.locator.locate(id).await?.ok_or(ReadError::NotFound)?;
        let StoredBody::Partial {
            header,
            parts,
            deferred,
        } = &location.body
        else {
            return Ok(());
        };
        if parts.iter().any(|(p, _)| p == path) {
            return Ok(());
        }
        if !deferred.contains(path) {
            return Err(ReadError::PartNotFound);
        }
        let fetch = BodyFetch::Partial {
            parts: vec![path.clone()],
            deferred: vec![],
        };
        let fetched = self
            .fetcher
            .fetch(
                location.account,
                &location.mailbox_name,
                location.uid,
                &fetch,
            )
            .await
            .map_err(|e| ReadError::Download(e.to_string()))?;
        let Some(FetchedBody::Partial { parts: got, .. }) = fetched else {
            return Err(ReadError::Download("message disparu du serveur".into()));
        };
        let bytes = got
            .into_iter()
            .find(|(p, _)| p == path)
            .map(|(_, b)| b)
            .ok_or_else(|| ReadError::Download("partie absente de la réponse".into()))?;
        let blob = self.blobs.put(location.account, bytes).await?;
        let mut parts = parts.clone();
        parts.push((path.clone(), blob));
        let body = StoredBody::Partial {
            header: header.clone(),
            parts,
            deferred: deferred.iter().filter(|p| *p != path).cloned().collect(),
        };
        self.store
            .set_body(location.mailbox, location.uid, &body, None)
            .await?;
        Ok(())
    }
}

/// Cas d'usage : télécharger le corps d'un message qui n'en a pas encore
/// (ouverture avant que la synchronisation ne l'ait rapatrié), selon la
/// politique de rapatriement.
#[derive(Debug)]
pub struct DownloadBody<'a, L, B, M, F> {
    pub locator: &'a L,
    pub blobs: &'a B,
    pub store: &'a M,
    pub fetcher: &'a F,
    pub policy: FetchPolicy,
    /// Texte cherchable du corps téléchargé (extrait hors du fil
    /// asynchrone, en temps borné).
    pub text: Arc<dyn SearchTextExtractor>,
}

impl<L, B, M, F> DownloadBody<'_, L, B, M, F>
where
    L: MessageLocator,
    B: BlobStore,
    M: MailStore,
    F: RemoteBodyFetcher,
{
    /// Télécharge le corps s'il manque ; renvoie vrai si un corps a été
    /// enregistré.
    pub async fn execute(&self, id: MessageId) -> Result<bool, ReadError> {
        let location = self.locator.locate(id).await?.ok_or(ReadError::NotFound)?;
        if location.body != StoredBody::Missing {
            return Ok(false);
        }
        let fetch = self
            .policy
            .decide(location.size, location.structure.as_ref());
        let fetched = self
            .fetcher
            .fetch(
                location.account,
                &location.mailbox_name,
                location.uid,
                &fetch,
            )
            .await
            .map_err(|e| ReadError::Download(e.to_string()))?
            .ok_or_else(|| ReadError::Download("message disparu du serveur".into()))?;
        let text = crate::sync::extract_text(
            &self.text,
            &fetched,
            location.structure.as_ref(),
            crate::sync::TEXT_EXTRACTION_TIMEOUT,
        )
        .await;
        let body = store_fetched(self.blobs, location.account, fetched, fetch).await?;
        self.store
            .set_body(location.mailbox, location.uid, &body, Some(&text))
            .await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests;
