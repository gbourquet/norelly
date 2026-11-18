//! Cas d'usage de la synchronisation CardDAV du carnet (ADR-014) :
//! découverte des carnets ([`DiscoverAddressBooks`]), enregistrement et
//! retrait du compte ([`ConfigureCardDav`], [`RemoveCardDav`]),
//! synchronisation dans les deux sens ([`SyncCardDav`]).
//!
//! Les décisions (que tirer, que pousser, qui gagne un conflit) sont
//! celles du domaine ([`plan_sync`]) ; ce module lit les deux carnets,
//! applique le plan et enregistre son bilan.

use std::collections::HashMap;

use norelly_domain::carddav::{
    AddressBook, CardDavAccount, CardVersion, DavCredentials, DavUrl, ETag, FetchedCard, LinkState,
    LocalCard, MAX_MULTIGET_HREFS, RemoteEntry, SyncAction, SyncFailure, SyncLink, SyncReport,
    card_version, hrefs_to_fetch, merge_card, new_card, new_card_href, plan_sync, remote_draft,
};
use norelly_domain::contact_export::{
    ContactSelection, ExportFormat, MAX_EXPORT_CONTACTS, export_card,
};
use norelly_domain::contact_import::{ContactCompletion, import_keys};
use norelly_domain::contacts::{Contact, ContactDraft};
use norelly_domain::credentials::Secret;
use norelly_domain::ports::{
    CardDavClient, CardDavError, CardDavStore, CardDavSyncRecord, ContactBook, ContactCodec,
    ContactDirectory, ContactStoreError, SecretKey, SecretStore, SecretStoreError, StoreError,
    TextNormalizer,
};

#[cfg(test)]
mod tests;

/// Clé du mot de passe du compte CardDAV dans le trousseau (un seul
/// compte).
pub const SECRET_KEY: &str = "carddav:password";

/// ETag retenue quand le serveur n'en rend pas après une écriture : elle
/// ne correspond à aucune version, la fiche sera relue à la
/// synchronisation suivante.
const UNKNOWN_ETAG: &str = "\"norelly-inconnue\"";

/// Clé du mot de passe dans le trousseau.
#[must_use]
pub fn secret_key() -> SecretKey {
    // Clé constante, toujours valide.
    SecretKey::parse(SECRET_KEY).unwrap_or_else(|_| unreachable!("clé constante"))
}

fn unknown_etag() -> ETag {
    ETag::parse(UNKNOWN_ETAG).unwrap_or_else(|| unreachable!("ETag constante"))
}

/// Échec de l'enregistrement ou du retrait du compte.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CardDavSetupError {
    #[error(transparent)]
    Keyring(#[from] SecretStoreError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Échec d'une synchronisation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncError {
    /// Aucun compte CardDAV.
    NotConfigured,
    /// Échec enregistré comme dernière synchronisation.
    Failed(SyncFailure),
}

/// Code d'échec d'une erreur du serveur.
#[must_use]
pub fn failure_of(error: &CardDavError) -> SyncFailure {
    match error {
        CardDavError::Unauthorized => SyncFailure::Unauthorized,
        CardDavError::Network(_) | CardDavError::Timeout => SyncFailure::Network,
        CardDavError::Certificate(_) => SyncFailure::Certificate,
        CardDavError::NotFound | CardDavError::NoAddressBook => SyncFailure::NotFound,
        CardDavError::TooLarge => SyncFailure::TooLarge,
        CardDavError::Server(_) => SyncFailure::Server,
        CardDavError::Protocol(_) | CardDavError::PreconditionFailed => SyncFailure::Protocol,
    }
}

/// Cas d'usage : carnets proposés par le serveur.
#[derive(Debug)]
pub struct DiscoverAddressBooks<'a, C> {
    pub client: &'a C,
}

impl<C: CardDavClient> DiscoverAddressBooks<'_, C> {
    pub async fn execute(
        &self,
        server: &DavUrl,
        credentials: &DavCredentials,
    ) -> Result<Vec<AddressBook>, CardDavError> {
        self.client.address_books(server, credentials).await
    }
}

/// Cas d'usage : enregistrer le compte (mot de passe dans le trousseau,
/// d'abord : un compte n'est jamais enregistré sans lui).
#[derive(Debug)]
pub struct ConfigureCardDav<'a, S, K> {
    pub store: &'a S,
    pub secrets: &'a K,
}

impl<S: CardDavStore, K: SecretStore> ConfigureCardDav<'_, S, K> {
    pub async fn execute(
        &self,
        account: &CardDavAccount,
        password: &Secret,
    ) -> Result<(), CardDavSetupError> {
        self.secrets.set(&secret_key(), password).await?;
        self.store.save_account(account).await?;
        Ok(())
    }
}

/// Cas d'usage : retirer le compte (les contacts restent).
#[derive(Debug)]
pub struct RemoveCardDav<'a, S, K> {
    pub store: &'a S,
    pub secrets: &'a K,
}

impl<S: CardDavStore, K: SecretStore> RemoveCardDav<'_, S, K> {
    pub async fn execute(&self) -> Result<(), CardDavSetupError> {
        self.store.remove_account().await?;
        self.secrets.delete(&secret_key()).await?;
        Ok(())
    }
}

/// Cas d'usage : synchroniser le carnet avec le carnet distant.
#[derive(Debug)]
pub struct SyncCardDav<'a, C, S, B, K, X, N> {
    pub client: &'a C,
    pub store: &'a S,
    pub book: &'a B,
    pub secrets: &'a K,
    pub codec: &'a X,
    pub normalizer: &'a N,
}

/// Issue d'une écriture du plan.
enum Step {
    Done,
    /// Remise à la prochaine synchronisation (fiche changée entre-temps,
    /// adresse prise…).
    Skipped,
}

/// Ordre d'application : suppressions ici d'abord (elles libèrent des
/// adresses), puis reprises, puis écritures sur le serveur.
fn rank(action: &SyncAction) -> u8 {
    match action {
        SyncAction::DeleteLocal { .. } => 0,
        SyncAction::PullUpdate { .. } => 1,
        SyncAction::PullCreate { .. } => 2,
        SyncAction::Link { .. }
        | SyncAction::Refresh { .. }
        | SyncAction::Ignore { .. }
        | SyncAction::Forget { .. }
        | SyncAction::Defer { .. } => 3,
        SyncAction::PushUpdate { .. } => 4,
        SyncAction::PushCreate { .. } => 5,
        SyncAction::DeleteRemote { .. } => 6,
    }
}

/// Erreur du serveur pendant une écriture : une fiche changée ou
/// disparue entre-temps, ou une écriture refusée (carnet en lecture
/// seule : 403, 405…) est remise à plus tard ; le reste arrête tout. Les
/// identifiants, eux, ont été acceptés par la lecture du carnet.
fn remote_step(error: CardDavError) -> Result<Step, SyncFailure> {
    match error {
        CardDavError::PreconditionFailed
        | CardDavError::NotFound
        | CardDavError::Unauthorized => Ok(Step::Skipped),
        CardDavError::Server(code) if (400..500).contains(&code) => Ok(Step::Skipped),
        other => Err(failure_of(&other)),
    }
}

/// Erreur du carnet pendant une écriture : une fiche changée ou une
/// adresse prise entre-temps est remise à plus tard.
fn local_step(error: ContactStoreError) -> Result<Step, SyncFailure> {
    match error {
        ContactStoreError::Conflict
        | ContactStoreError::NotFound
        | ContactStoreError::DuplicateAddress(_) => Ok(Step::Skipped),
        ContactStoreError::Corrupted(_) | ContactStoreError::Store(_) => Err(SyncFailure::Storage),
    }
}

impl<C, S, B, K, X, N> SyncCardDav<'_, C, S, B, K, X, N>
where
    C: CardDavClient,
    S: CardDavStore,
    B: ContactBook + ContactDirectory,
    K: SecretStore,
    X: ContactCodec,
    N: TextNormalizer,
{
    /// Synchronise à l'heure `now` ; le bilan ou l'échec est enregistré.
    pub async fn execute(&self, now: i64) -> Result<SyncReport, SyncError> {
        let account = self
            .store
            .account()
            .await
            .map_err(|_| SyncError::Failed(SyncFailure::Storage))?
            .ok_or(SyncError::NotConfigured)?;
        let outcome = self.run(&account, now).await;
        let record = CardDavSyncRecord { at: now, outcome };
        if let Err(e) = self.store.record_sync(&record).await {
            tracing::warn!(error = %e, "bilan de synchronisation CardDAV non enregistré");
        }
        outcome.map_err(SyncError::Failed)
    }

    /// Fiche vCard du contact dans la version `version`.
    fn encode(&self, draft: &ContactDraft, version: CardVersion) -> Result<String, SyncFailure> {
        let format = match version {
            CardVersion::V4 => ExportFormat::VCard4,
            CardVersion::V3 => ExportFormat::VCard3,
        };
        let bytes = self
            .codec
            .encode(&[export_card(draft, format)], format)
            .map_err(|_| SyncFailure::Storage)?;
        String::from_utf8(bytes).map_err(|_| SyncFailure::Storage)
    }

    /// Fiche du contact à comparer d'une synchronisation à l'autre.
    fn snapshot(&self, draft: &ContactDraft) -> Result<String, SyncFailure> {
        self.encode(draft, CardVersion::V4)
    }

    async fn run(&self, account: &CardDavAccount, now: i64) -> Result<SyncReport, SyncFailure> {
        let password = self
            .secrets
            .get(&secret_key())
            .await
            .map_err(|_| SyncFailure::Keyring)?
            .ok_or(SyncFailure::Keyring)?;
        let credentials = DavCredentials {
            username: account.username().clone(),
            password,
        };
        let book = account.book().url();
        let remote = self
            .client
            .entries(book, &credentials)
            .await
            .map_err(|e| failure_of(&e))?;
        let links = self.store.links().await.map_err(|_| SyncFailure::Storage)?;
        let local = self.local_cards().await?;
        let fetched = self.fetch(book, &credentials, &links, &remote).await?;
        let mut plan = plan_sync(&links, &local, &remote, &fetched, now);
        plan.sort_by_key(rank);
        let mut report = SyncReport::default();
        for action in &plan {
            match self.apply(book, &credentials, action, now).await? {
                Step::Done => report.count(action),
                Step::Skipped => report.skipped += 1,
            }
        }
        Ok(report)
    }

    /// Contacts du carnet, avec leur fiche et leur date de modification.
    async fn local_cards(&self) -> Result<Vec<LocalCard>, SyncFailure> {
        let contacts = self
            .book
            .contacts(&ContactSelection::All, MAX_EXPORT_CONTACTS + 1)
            .await
            .map_err(|_| SyncFailure::Storage)?;
        if contacts.len() > MAX_EXPORT_CONTACTS {
            return Err(SyncFailure::TooLarge);
        }
        let times: HashMap<_, _> = self
            .store
            .modification_times()
            .await
            .map_err(|_| SyncFailure::Storage)?
            .into_iter()
            .collect();
        contacts
            .into_iter()
            .map(|contact| {
                Ok(LocalCard {
                    snapshot: self.snapshot(contact.draft())?,
                    modified_at: times.get(&contact.id()).copied().unwrap_or(0),
                    contact,
                })
            })
            .collect()
    }

    /// Fiches distantes nouvelles ou changées, téléchargées et lues.
    async fn fetch(
        &self,
        book: &DavUrl,
        credentials: &DavCredentials,
        links: &[SyncLink],
        remote: &[RemoteEntry],
    ) -> Result<Vec<FetchedCard>, SyncFailure> {
        let wanted = hrefs_to_fetch(links, remote);
        let mut fetched = Vec::with_capacity(wanted.len());
        for chunk in wanted.chunks(MAX_MULTIGET_HREFS) {
            let cards = self
                .client
                .cards(book, credentials, chunk)
                .await
                .map_err(|e| failure_of(&e))?;
            for card in cards {
                let decoded = self.codec.decode(card.data.as_bytes()).ok();
                let draft = decoded
                    .as_ref()
                    .and_then(|decoded| remote_draft(decoded, self.normalizer));
                let keys = decoded
                    .as_ref()
                    .map(|decoded| import_keys(&decoded.cards))
                    .unwrap_or_default();
                fetched.push(FetchedCard {
                    entry: RemoteEntry {
                        href: card.href,
                        etag: card.etag,
                    },
                    raw: card.data,
                    draft,
                    keys,
                });
            }
        }
        Ok(fetched)
    }

    async fn save_link(&self, link: SyncLink) -> Result<Step, SyncFailure> {
        self.store
            .save_link(&link)
            .await
            .map_err(|_| SyncFailure::Storage)?;
        Ok(Step::Done)
    }

    /// Ressource reliée au contact `contact` tel qu'écrit.
    async fn link_to(&self, card: &FetchedCard, contact: &Contact) -> Result<Step, SyncFailure> {
        self.save_link(SyncLink {
            href: card.entry.href.clone(),
            etag: card.entry.etag.clone(),
            state: LinkState::Linked(contact.id()),
            synced: self.snapshot(contact.draft())?,
            remote: card.raw.clone(),
        })
        .await
    }

    async fn apply(
        &self,
        book: &DavUrl,
        credentials: &DavCredentials,
        action: &SyncAction,
        now: i64,
    ) -> Result<Step, SyncFailure> {
        match action {
            SyncAction::PullCreate { card } => {
                let Some(draft) = &card.draft else {
                    return Ok(Step::Skipped);
                };
                match self.book.create(draft).await {
                    Ok(contact) => self.link_to(card, &contact).await,
                    Err(e) => local_step(e),
                }
            }
            SyncAction::PullUpdate {
                card,
                contact,
                draft,
            } => {
                // Écrit seulement si la fiche n'a pas changé depuis sa
                // lecture : une saisie faite pendant la synchronisation
                // n'est jamais écrasée.
                let completion = ContactCompletion::new(contact.clone(), draft.clone());
                if let Err(e) = self.book.import(&[], &[completion]).await {
                    return local_step(e);
                }
                // Fiche relue telle qu'enregistrée : c'est elle que la
                // synchronisation suivante comparera.
                let Some(written) = self
                    .book
                    .get(contact.id())
                    .await
                    .map_err(|_| SyncFailure::Storage)?
                else {
                    return Ok(Step::Skipped);
                };
                self.link_to(card, &written).await
            }
            SyncAction::Link { card, contact } | SyncAction::Refresh { card, contact } => {
                self.link_to(card, contact).await
            }
            SyncAction::Ignore { card } => {
                self.save_link(SyncLink {
                    href: card.entry.href.clone(),
                    etag: card.entry.etag.clone(),
                    state: LinkState::Ignored,
                    synced: String::new(),
                    remote: card.raw.clone(),
                })
                .await
            }
            SyncAction::Defer { .. } => Ok(Step::Done),
            SyncAction::Forget { href } => {
                self.store
                    .forget_link(href)
                    .await
                    .map_err(|_| SyncFailure::Storage)?;
                Ok(Step::Done)
            }
            SyncAction::DeleteLocal { href, contact } => {
                // Modifiée depuis la lecture : gardée, revue la prochaine
                // fois.
                let current = self
                    .book
                    .get(contact.id())
                    .await
                    .map_err(|_| SyncFailure::Storage)?;
                if current.as_ref() != Some(contact) {
                    return Ok(Step::Skipped);
                }
                if let Err(e) = self.book.delete(contact.id()).await {
                    return local_step(e);
                }
                self.store
                    .forget_link(href)
                    .await
                    .map_err(|_| SyncFailure::Storage)?;
                Ok(Step::Done)
            }
            SyncAction::PushCreate { contact } => {
                let uid = self.client.new_uid();
                let href = new_card_href(book, &uid);
                let card = new_card(&self.encode(contact.draft(), CardVersion::V3)?, &uid, now);
                match self.client.create(book, credentials, &href, &card).await {
                    Ok(etag) => {
                        self.save_link(SyncLink {
                            href,
                            etag: etag.unwrap_or_else(unknown_etag),
                            state: LinkState::Linked(contact.id()),
                            synced: self.snapshot(contact.draft())?,
                            remote: card,
                        })
                        .await
                    }
                    Err(e) => remote_step(e),
                }
            }
            SyncAction::PushUpdate {
                href,
                etag,
                base,
                contact,
            } => {
                let ours = self.encode(contact.draft(), card_version(base))?;
                let card = merge_card(base, &ours, now);
                match self
                    .client
                    .update(book, credentials, href, etag, &card)
                    .await
                {
                    Ok(etag) => {
                        self.save_link(SyncLink {
                            href: href.clone(),
                            etag: etag.unwrap_or_else(unknown_etag),
                            state: LinkState::Linked(contact.id()),
                            synced: self.snapshot(contact.draft())?,
                            remote: card,
                        })
                        .await
                    }
                    Err(e) => remote_step(e),
                }
            }
            SyncAction::DeleteRemote { href, etag } => {
                match self.client.delete(book, credentials, href, etag).await {
                    Ok(()) => {
                        self.store
                            .forget_link(href)
                            .await
                            .map_err(|_| SyncFailure::Storage)?;
                        Ok(Step::Done)
                    }
                    Err(e) => remote_step(e),
                }
            }
        }
    }
}
