//! Cas d'usage de l'export du carnet (ADR-012), par pages : jamais tout
//! le carnet en mémoire.
//!
//! [`PrepareContactExport`] repère d'abord les contacts visés, page par
//! page ([`EXPORT_PAGE_SIZE`]) : il décide ce qui s'exporte (aucun
//! contact, trop de contacts et de listes) avant que l'utilisateur ne
//! choisisse le fichier, et laisse le format fixer ses colonnes (CSV).
//! [`ContactExport::next_chunk`] relit ensuite les mêmes contacts, page par
//! page, et rend les octets du fichier morceau par morceau, préparés par
//! les règles du domaine ([`export_card`], [`export_lists`]) et mis en
//! forme par le port [`ContactCodec`]. L'appelant écrit chaque morceau
//! (disque), hors des fils asynchrones.
//!
//! Le carnet peut changer entre le repérage et l'écriture : une fiche
//! supprimée entre-temps est omise (ses listes aussi l'omettent), une
//! liste supprimée n'est pas écrite, une fiche modifiée est écrite telle
//! qu'elle est relue ; en CSV, une fiche qui ne tient plus dans les
//! colonnes repérées arrête l'export (jamais tronquée). Une fiche ou une
//! liste créée, un membre ajouté, attendent l'export suivant.
//!
//! La photo d'un contact, réencodée à l'import, est lue dans le
//! [`ContactPhotoStore`] pour les formats qui la gardent
//! ([`ExportFormat::keeps_photos`]) ; une photo absente, illisible ou qui
//! n'est pas un JPEG est omise, sans empêcher l'export, comme toutes les
//! photos au-delà du budget de l'export
//! ([`norelly_domain::contact_export::MAX_EXPORT_PHOTO_BYTES`]).

use std::collections::HashSet;

use norelly_domain::contact_export::{
    ContactSelection, EXPORT_PAGE_SIZE, ExportCard, ExportFormat, ExportMember,
    MAX_EXPORT_CONTACTS, PhotoBudget, export_card, export_lists, exports_lists, listed_contacts,
};
use norelly_domain::contact_lists::{ContactList, ContactListId};
use norelly_domain::contact_photo::is_kept_photo;
use norelly_domain::contacts::{Contact, ContactId};
use norelly_domain::ports::{
    ContactCodec, ContactCodecError, ContactDirectory, ContactEncoder, ContactListDirectory,
    ContactPhotoStore, StoreError,
};

/// Échec d'un export.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExportFailure {
    /// Aucun contact à exporter (carnet vide, sélection supprimée).
    #[error("aucun contact à exporter")]
    Empty,
    /// Plus de [`MAX_EXPORT_CONTACTS`] contacts et listes.
    #[error("trop de contacts à exporter")]
    TooMany,
    /// Fichier refusé par sa mise en forme (trop gros, fiche modifiée
    /// depuis le repérage).
    #[error(transparent)]
    Codec(#[from] ContactCodecError),
    /// Mise en forme interrompue (panique) : le fichier est à abandonner.
    #[error("mise en forme interrompue")]
    Interrupted,
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Cas d'usage : repérer les contacts visés (et les listes avec tout le
/// carnet) pour un export au format `format`.
#[derive(Debug)]
pub struct PrepareContactExport<'a, D, S, C> {
    pub directory: &'a D,
    pub photos: &'a S,
    pub codec: &'a C,
}

impl<'a, D, S, C> PrepareContactExport<'a, D, S, C>
where
    D: ContactDirectory + ContactListDirectory,
    S: ContactPhotoStore,
    C: ContactCodec,
{
    /// Export repéré, prêt à écrire ; ensemble au plus
    /// [`MAX_EXPORT_CONTACTS`] fiches et listes.
    pub async fn execute(
        &self,
        selection: &ContactSelection,
        format: ExportFormat,
    ) -> Result<ContactExport<'a, D, S, C::Encoder>, ExportFailure> {
        // Fiches invalides comprises : de quoi trouver une fiche valide de
        // plus que la borne même derrière autant de fiches invalides.
        let scan = 2 * (MAX_EXPORT_CONTACTS + 1);
        let ids = self.directory.contact_ids(selection, scan).await?;
        let mut encoder = self.codec.encoder(format);
        // Repérage, page par page : seules les fiches valides comptent.
        let mut contacts = 0;
        for page in ids.chunks(EXPORT_PAGE_SIZE) {
            let page = read_page(self.directory, page).await?;
            contacts += page.len();
            if contacts > MAX_EXPORT_CONTACTS {
                return Err(ExportFailure::TooMany);
            }
            let cards: Vec<ExportCard> = page
                .iter()
                .map(|c| export_card(c.draft(), format))
                .collect();
            encoder.survey(&cards);
        }
        if contacts == 0 {
            return Err(ExportFailure::Empty);
        }
        // Autant de fiches invalides que de place : la suite est inconnue,
        // refusée plutôt que tronquée.
        if ids.len() >= scan {
            return Err(ExportFailure::TooMany);
        }
        let lists = if exports_lists(selection, format) {
            // Une liste de plus que la place restante : savoir qu'elle est
            // dépassée.
            let room = MAX_EXPORT_CONTACTS - contacts;
            let lists = self.directory.all_lists(room + 1).await?;
            if lists.len() > room {
                return Err(ExportFailure::TooMany);
            }
            lists
        } else {
            Vec::new()
        };
        Ok(ContactExport {
            directory: self.directory,
            photos: self.photos,
            format,
            encoder: Some(encoder),
            ids,
            next: 0,
            listed: listed_contacts(&lists),
            lists: lists.iter().map(ContactList::id).collect(),
            members: Vec::new(),
            member_cards: Vec::new(),
            budget: PhotoBudget::default(),
            exported: 0,
            done: false,
        })
    }
}

/// Fiches complètes des contacts `page` (au plus [`EXPORT_PAGE_SIZE`]),
/// dans l'ordre de l'export ; invalides ou supprimées entre-temps omises.
async fn read_page<D: ContactDirectory>(
    directory: &D,
    page: &[ContactId],
) -> Result<Vec<Contact>, StoreError> {
    directory
        .contacts(&ContactSelection::Contacts(page.to_vec()), page.len())
        .await
}

/// Export repéré, écrit morceau par morceau ([`Self::next_chunk`]).
pub struct ContactExport<'a, D, S, E> {
    directory: &'a D,
    photos: &'a S,
    format: ExportFormat,
    /// Absent pendant une mise en forme (hors des fils asynchrones), ou
    /// après qu'elle a été interrompue.
    encoder: Option<E>,
    /// Contacts visés, dans l'ordre de l'export (fiches invalides
    /// comprises, écartées à la lecture de leur page).
    ids: Vec<ContactId>,
    /// Prochain contact à lire dans `ids`.
    next: usize,
    /// Listes repérées, relues après les fiches (une liste supprimée
    /// entre-temps n'est pas écrite).
    lists: Vec<ContactListId>,
    /// Contacts membres d'une liste : seul ce que le format en écrit est
    /// gardé ([`ExportMember`]).
    listed: HashSet<ContactId>,
    members: Vec<ContactId>,
    member_cards: Vec<ExportMember>,
    budget: PhotoBudget,
    exported: usize,
    done: bool,
}

impl<D, S, E> std::fmt::Debug for ContactExport<'_, D, S, E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Ni fiche ni liste dans le journal.
        f.debug_struct("ContactExport")
            .field("format", &self.format)
            .field("contacts", &self.ids.len())
            .field("next", &self.next)
            .field("exported", &self.exported)
            .finish_non_exhaustive()
    }
}

impl<D, S, E> ContactExport<'_, D, S, E>
where
    D: ContactDirectory + ContactListDirectory,
    S: ContactPhotoStore,
    E: ContactEncoder + 'static,
{
    /// Fiches écrites jusqu'ici.
    #[must_use]
    pub fn exported(&self) -> usize {
        self.exported
    }

    /// Morceau suivant du fichier ; `None` quand il est complet. Après une
    /// erreur, le fichier est à abandonner.
    pub async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, ExportFailure> {
        if self.done {
            return Ok(None);
        }
        if self.next < self.ids.len() {
            let end = self.ids.len().min(self.next + EXPORT_PAGE_SIZE);
            let page = read_page(self.directory, &self.ids[self.next..end]).await?;
            self.next = end;
            let mut cards = Vec::with_capacity(page.len());
            for contact in &page {
                let card = self.card(contact).await;
                if self.listed.contains(&contact.id()) {
                    self.members.push(contact.id());
                    self.member_cards.push(ExportMember::of(&card));
                }
                cards.push(card);
            }
            let written = cards.len();
            let bytes = self.encode(move |e| e.cards(&cards)).await?;
            self.exported += written;
            return Ok(Some(bytes));
        }
        // Contacts tous supprimés depuis le repérage : rien à exporter.
        if self.exported == 0 {
            return Err(ExportFailure::Empty);
        }
        // Listes relues : leur contenu actuel, sans celles supprimées
        // entre-temps ; un membre ajouté depuis le repérage n'est pas gardé.
        let mut current = Vec::with_capacity(self.lists.len());
        for &id in &self.lists {
            current.extend(self.directory.get_list(id).await?);
        }
        let lists = export_lists(&current, &self.members);
        let members = std::mem::take(&mut self.member_cards);
        let bytes = self
            .encode(move |e| {
                let mut bytes = e.lists(&lists, &members)?;
                bytes.extend(e.finish()?);
                Ok(bytes)
            })
            .await?;
        self.done = true;
        Ok(Some(bytes))
    }

    /// Mise en forme `write` hors des fils asynchrones : une page peut
    /// porter tout le budget des photos.
    async fn encode<T: Send + 'static>(
        &mut self,
        write: impl FnOnce(&mut E) -> Result<T, ContactCodecError> + Send + 'static,
    ) -> Result<T, ExportFailure> {
        let mut encoder = self.encoder.take().ok_or(ExportFailure::Interrupted)?;
        let (encoder, written) = tokio::task::spawn_blocking(move || {
            let written = write(&mut encoder);
            (encoder, written)
        })
        .await
        .map_err(|_| ExportFailure::Interrupted)?;
        self.encoder = Some(encoder);
        Ok(written?)
    }

    /// Fiche de `contact` pour le format de l'export, avec sa photo pour
    /// les formats qui la gardent.
    async fn card(&mut self, contact: &Contact) -> ExportCard {
        let mut card = export_card(contact.draft(), self.format);
        if self.format.keeps_photos()
            && !self.budget.exhausted()
            && let Some(photo) = contact.photo()
        {
            card.photo = match self.photos.get(photo).await {
                // Seule une photo gardée (JPEG) est écrite, dans la limite
                // du budget des photos.
                Ok(Some(bytes)) if is_kept_photo(&bytes) && self.budget.take(bytes.len()) => {
                    Some(bytes)
                }
                Ok(_) => None,
                Err(e) => {
                    // Ni le contenu ni la fiche dans le journal.
                    tracing::warn!(error = %e, "photo de contact illisible, omise de l'export");
                    None
                }
            };
        }
        card
    }
}

#[cfg(test)]
mod tests;
